use std::time::Instant;

use anyhow::Result;

use crate::core::{self, Core};

use self::greenzone::Greenzone;

mod greenzone;
pub mod input;

pub struct Tas {
    core: Core,

    // Playback state
    /// The frame the user expects to be visible on the screen.
    /// Normally, this will lag behind next_emulator_frame by 1. However, next_emulator_frame may
    /// have a lower value if the emulator is currently behind, e.g. to needing to catch up after
    /// loading a state from the greenzone.
    playback_cursor: u32,
    /// The next frame the emulator will render.
    next_emulator_frame: u32,
    run_mode: RunMode,
    last_host_frame: Instant,
    core_frame_fraction: f32,

    // Editor state
    greenzone: Greenzone,
    selected_frame: u32,
    selection_locked: bool,

    input_port: input::InputPort,
    data: Vec<u8>,
}

pub struct Frame {}

#[derive(Clone, Debug)]
pub enum RunMode {
    Running {
        stop_at: Option<u32>,
        record_mode: RecordMode,
    },
    Paused,
}

#[derive(Clone, Debug)]
pub enum RecordMode {
    ReadOnly,
    Insert(Vec<u8>),
    Overwrite(Vec<u8>),
}

impl Tas {
    pub fn new() -> Result<Tas> {
        let mut core = unsafe {
            Core::load(
                "cores/bsnes2014_accuracy_libretro.dylib",
                "/Users/jonathan/code/sm/ntsc.sfc",
            )?
        };

        let input_port = input::InputPort::Joypad(input::Joypad::Snes);
        // Create an empty frame of input.
        let mut data = Vec::new();
        data.resize(input_port.frame_size(), 0);
        input_port.default(&mut data);

        Ok(Tas {
            playback_cursor: 0,
            next_emulator_frame: 0,
            run_mode: RunMode::Running {
                stop_at: None,
                record_mode: RecordMode::Insert(data.clone()),
            },
            last_host_frame: Instant::now(),
            core_frame_fraction: 0.,

            greenzone: Greenzone::new(core.save_state()),
            selected_frame: 0,
            selection_locked: true,

            core,

            input_port,
            data,
        })
    }

    pub fn selected_frame(&self) -> u32 {
        self.selected_frame
    }

    pub fn playback_frame(&self) -> u32 {
        self.playback_cursor
    }

    pub fn greenzone(&self) -> &Greenzone {
        &self.greenzone
    }

    pub fn run_guest_frame(&mut self) -> &core::Frame {
        self.core.run_frame();
        self.next_emulator_frame += 1;
        self.greenzone
            .save(self.next_emulator_frame, self.core.save_state());
        if self.playback_cursor < self.next_emulator_frame - 1 {
            let n = self.next_emulator_frame - self.playback_cursor - 1;
            self.playback_cursor += n;
            if self.selection_locked {
                self.selected_frame += n;
            }
        }
        &self.core.frame
    }

    pub fn run_host_frame(&mut self) -> &core::Frame {
        // Determine how many guest frames have elapsed since the last host frame
        let time = Instant::now();
        let host_frame_duration = time - self.last_host_frame;
        self.last_host_frame = time;
        self.core_frame_fraction +=
            host_frame_duration.as_secs_f32() * self.core.av_info.timing.fps as f32;

        // Don't try to skip more than one guest frame
        self.core_frame_fraction = self.core_frame_fraction.clamp(0., 2.);

        // If we're behind where playback should be, seek to catch up
        while self.playback_cursor >= self.next_emulator_frame && self.core_frame_fraction >= 1. {
            self.run_guest_frame();
            self.core_frame_fraction -= 1.;
        }

        if self.core_frame_fraction < 1. {
            return &self.core.frame;
        }

        let run_mode = std::mem::replace(&mut self.run_mode, RunMode::Paused);

        let result = match &run_mode {
            RunMode::Paused => &self.core.frame,
            RunMode::Running {
                stop_at,
                record_mode,
            } => {
                while self.core_frame_fraction >= 1. {
                    if let Some(stop) = stop_at {
                        if self.playback_cursor >= *stop {
                            self.run_mode = RunMode::Paused;
                            break;
                        }
                    }

                    match record_mode {
                        RecordMode::ReadOnly => {}
                        RecordMode::Insert(data) => {
                            assert!(data.len() == self.input_port.frame_size());
                            self.insert(self.playback_cursor + 1, data);
                        }
                        RecordMode::Overwrite(data) => {
                            assert!(data.len() == self.input_port.frame_size());
                            self.frame_mut(self.playback_cursor + 1)
                                .copy_from_slice(data);
                        }
                    }
                    assert!(self.next_emulator_frame == self.playback_cursor + 1);

                    self.run_guest_frame();
                    self.core_frame_fraction -= 1.;
                }

                &self.core.frame
            }
        };

        self.run_mode = run_mode;

        result
    }

    pub fn run_mode(&self) -> &RunMode {
        &self.run_mode
    }

    pub fn set_run_mode(&mut self, mode: RunMode) {
        self.run_mode = mode;
    }

    pub fn av_info(&self) -> libretro_ffi::retro_system_av_info {
        self.core.av_info
    }

    pub fn input_port(&self) -> &input::InputPort {
        &self.input_port
    }

    pub fn movie_len(&self) -> u32 {
        (self.data.len() / self.input_port.frame_size()) as u32
    }

    /// Invalidates the greenzone after the specified index.
    /// In other words: the savestate at the beginning of this frame is valid, but this frame's
    /// input may have changed.
    pub fn invalidate(&mut self, after: u32) {
        self.greenzone.invalidate(after);
        if self.next_emulator_frame > after {
            let (f, state) = self.greenzone.restore(after);
            self.next_emulator_frame = f;
            self.core.restore_state(state);
        }
    }

    pub fn frame(&self, idx: u32) -> &[u8] {
        let size = self.input_port.frame_size();
        &self.data[idx as usize * size..][..size]
    }

    pub fn frame_mut(&mut self, idx: u32) -> &mut [u8] {
        self.invalidate(idx);
        let size = self.input_port.frame_size();
        &mut self.data[idx as usize * size..][..size]
    }

    pub fn insert(&mut self, idx: u32, buf: &[u8]) {
        let size = self.input_port.frame_size();
        assert_eq!(buf.len() % size, 0);
        self.invalidate(idx);

        let insert_idx = idx as usize * size;
        self.data
            .splice(insert_idx..insert_idx, buf.iter().cloned());
    }

    pub fn seek_to(&mut self, frame: u32) {
        self.playback_cursor = frame;
        let (f, state) = self.greenzone.restore(frame);
        self.next_emulator_frame = f;
        self.core.restore_state(state);
    }

    pub fn toggle_playback(&mut self) {
        self.run_mode = match self.run_mode {
            RunMode::Paused => RunMode::Running {
                stop_at: None,
                record_mode: RecordMode::ReadOnly,
            },
            RunMode::Running {
                stop_at: _,
                record_mode: _,
            } => RunMode::Paused,
        }
    }

    pub fn select_next(&mut self, n: u32) {
        let n = n.min(self.movie_len().saturating_sub(self.selected_frame() + 1) as u32);
        self.selected_frame += n;
        if self.selection_locked {
            self.seek_to(self.playback_cursor + n);
        }
        self.run_mode = RunMode::Paused;
    }
    pub fn select_prev(&mut self, n: u32) {
        let n = n.min(self.selected_frame);
        self.selected_frame -= n;
        if self.selection_locked {
            self.seek_to(self.playback_cursor.saturating_sub(n));
        }
        self.run_mode = RunMode::Paused;
    }
}
