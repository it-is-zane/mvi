use std::collections::VecDeque;

use imgui::{ListClipper, Ui};

use crate::tas::{
    input::{InputPort, Joypad},
    Tas,
};

pub struct PianoRoll {
    last_selection: u32,
    screen_size: u32,

    pending_scroll_events: VecDeque<ScrollEvent>,
}

pub enum ScrollEvent {
    Up(u32),
    Down(u32),
    Top,
    Center,
    Bottom,
}

macro_rules! color {
    [$r:expr, $g:expr, $b:expr] => {
        [$r as f32 / 256., $g as f32 / 256., $b as f32 / 256., 1.]
    };
}
impl PianoRoll {
    const FRAMENO_COLOR: [f32; 4] = color![116, 128, 160];
    const SELECTED_FRAMENO_COLOR: [f32; 4] = color![0, 0, 0];
    const UNPRESSED_COLOR: [f32; 4] = color![45, 49, 55];
    const SELECT_HIGHLIGHT: [f32; 4] = color![128, 128, 128];
    const GREENZONE_HIGHLIGHT: [f32; 4] = color![16, 32, 16];

    pub fn new() -> PianoRoll {
        PianoRoll {
            last_selection: 0,
            screen_size: 30,
            pending_scroll_events: VecDeque::default(),
        }
    }

    pub fn screen_size(&self) -> u32 {
        self.screen_size
    }

    pub fn push_scroll_event(&mut self, event: ScrollEvent) {
        self.pending_scroll_events.push_back(event);
    }

    pub fn draw(&mut self, ui: &mut Ui, tas: &mut Tas) {
        ui.window("Piano Roll")
            .size([256., 768.], imgui::Condition::FirstUseEver)
            .build(|| {
                let rows = tas.movie_len();

                let _style = ui.push_style_var(imgui::StyleVar::ItemSpacing([0., 0.]));
                if tas.selected_frame() != self.last_selection {
                    ui.set_scroll_y(
                        (tas.selected_frame() as f64 * ui.text_line_height_with_spacing() as f64)
                            as f32,
                    );
                }

                let clipper = ListClipper::new(rows.try_into().unwrap()).begin(ui);

                let buttons = match tas.input_port() {
                    InputPort::Joypad(j) => j.buttons(),
                };
                let number_column_width = rows.saturating_sub(1).ilog10() + 1;
                let number_column_width = number_column_width.max(3) as usize;

                self.screen_size =
                    (ui.window_size()[1] / ui.text_line_height_with_spacing()) as u32;

                for row in clipper.iter() {
                    let (highlight, frameno_color) = if row as u32 == tas.selected_frame() {
                        (Some(Self::SELECT_HIGHLIGHT), Self::SELECTED_FRAMENO_COLOR)
                    } else if tas.greenzone().restore(row as u32).0 == row as u32 {
                        (Some(Self::GREENZONE_HIGHLIGHT), Self::FRAMENO_COLOR)
                    } else {
                        (None, Self::FRAMENO_COLOR)
                    };

                    if let Some(highlight) = highlight {
                        ui.get_window_draw_list()
                            .add_rect(
                                ui.cursor_screen_pos(),
                                [
                                    ui.cursor_screen_pos()[0] + ui.window_size()[0],
                                    ui.cursor_screen_pos()[1] + ui.text_line_height_with_spacing(),
                                ],
                                highlight,
                            )
                            .filled(true)
                            .build();
                    }

                    let marker = if row as u32 == tas.playback_frame() {
                        '>'
                    } else {
                        ' '
                    };
                    ui.text_colored(
                        frameno_color,
                        format!("{marker}{row:width$} ", width = number_column_width),
                    );

                    for text in buttons {
                        ui.same_line();
                        ui.text_colored(Self::UNPRESSED_COLOR, text);
                    }
                }
            });
        self.last_selection = tas.selected_frame();
    }
}