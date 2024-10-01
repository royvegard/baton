use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    layout::{Constraint, Layout, Margin},
    style::{Color, Style, Stylize},
    text::Line,
    widgets::{Bar, BarChart, BarGroup, Block, Paragraph},
    DefaultTerminal, Frame,
};
use std::io;

mod usb;

fn main() -> io::Result<()> {
    let mut terminal = ratatui::init();
    let app_result = App::new().run(&mut terminal);
    ratatui::restore();
    app_result
}

pub struct App {
    exit: bool,
    active_strip_index: usize,
    first_strip_index: usize,
    strip_width: u16,
    strip_display_cap: u16,
    status_line: String,
    ps: usb::PreSonusStudio1824c,
}

impl App {
    fn new() -> Self {
        let mut app = App {
            exit: false,
            active_strip_index: 0,
            strip_width: 5,
            strip_display_cap: 1,
            first_strip_index: 0,
            status_line: String::with_capacity(256),
            ps: usb::PreSonusStudio1824c::new().expect("Failed to open device"),
        };

        app.set_active_strip(app.active_strip_index as isize);
        app
    }

    fn set_active_strip(&mut self, index: isize) {
        self.active_strip_index =
            index.clamp(0, (self.ps.main_mix.channel_strips.len() - 1) as isize) as usize;

        for s in &mut self.ps.main_mix.channel_strips {
            s.active = false;
        }
        self.ps.main_mix.channel_strips[self.active_strip_index].active = true;
    }

    /// runs the application's main loop until the user quits
    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;
            self.handle_events()?;
        }
        Ok(())
    }

    fn draw(&mut self, frame: &mut Frame) {
        let [title_area, strips_area, status_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(3),
        ])
        .spacing(0)
        .areas(frame.area());

        let strips_width = strips_area.inner(Margin::new(1, 1)).width;

        self.status_line.clear();
        self.status_line.push_str(&format!(
            "{:?} {} - {} ({:>5.1} dB)",
            self.ps.main_mix.channel_strips[self.active_strip_index].kind,
            self.ps.main_mix.channel_strips[self.active_strip_index].number,
            self.ps.main_mix.channel_strips[self.active_strip_index].name,
            self.ps.main_mix.channel_strips[self.active_strip_index].fader
        ));

        self.strip_display_cap = strips_width / (self.strip_width + 1);

        let status_line = Line::from(self.status_line.as_str()).left_aligned();

        while self.active_strip_index < self.first_strip_index {
            self.first_strip_index -= 1;
        }
        while self.active_strip_index > self.first_strip_index + self.strip_display_cap as usize - 1
        {
            self.first_strip_index += 1;
        }

        frame.render_widget("Mixer".bold().into_centered_line(), title_area);
        frame.render_widget(
            self.vertical_barchart(&self.ps.main_mix.channel_strips),
            strips_area,
        );
        frame.render_widget(
            Paragraph::new(status_line).block(Block::bordered().title("Status")),
            status_area,
        );
    }

    fn handle_events(&mut self) -> io::Result<()> {
        match event::read()? {
            Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                self.handle_key_event(key_event)
            }
            _ => {}
        };
        Ok(())
    }

    fn handle_key_event(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Char('q') => self.exit(),
            KeyCode::Down => {
                if key_event.modifiers == KeyModifiers::SHIFT {
                    self.increment_fader(-0.1);
                } else if key_event.modifiers == KeyModifiers::CONTROL {
                    self.increment_fader(-10.0);
                } else {
                    self.increment_fader(-1.0);
                }
            }
            KeyCode::Up => {
                if key_event.modifiers == KeyModifiers::SHIFT {
                    self.increment_fader(0.1);
                } else if key_event.modifiers == KeyModifiers::CONTROL {
                    self.increment_fader(10.0);
                } else {
                    self.increment_fader(1.0);
                }
            }
            KeyCode::Left => {
                if key_event.modifiers == KeyModifiers::CONTROL {
                    self.increment_strip_width(-1);
                } else {
                    self.decrement_strip();
                }
            }
            KeyCode::Right => {
                if key_event.modifiers == KeyModifiers::CONTROL {
                    self.increment_strip_width(1);
                } else {
                    self.increment_strip();
                }
            }
            _ => {}
        }
    }

    fn exit(&mut self) {
        self.exit = true;
    }

    fn increment_fader(&mut self, delta: f64) {
        let current = self.ps.main_mix.channel_strips[self.active_strip_index].fader;
        self.ps.main_mix.channel_strips[self.active_strip_index].set_fader(current + delta);

        self.ps
            .command
            .set_db(self.ps.main_mix.channel_strips[self.active_strip_index].fader);

        match self.ps.main_mix.channel_strips[self.active_strip_index].kind {
            usb::StripKind::Main => {
                self.ps.command.input_strip = 0x00;
            }
            usb::StripKind::Channel => {
                self.ps.command.input_strip = self.active_strip_index as u32;
                self.ps.command.mode = usb::MODE_CHANNEL_STRIP;

                self.ps.command.output_strip = self.ps.main_mix.destination_strip.number;
                self.ps.command.output_channel = usb::LEFT;
                self.ps.send_command();
                self.ps.command.output_channel = usb::RIGHT;
                self.ps.send_command();
            }
            usb::StripKind::Bus => {
                self.ps.command.input_strip = 0x00;
            }
        }
    }

    fn decrement_strip(&mut self) {
        self.set_active_strip(self.active_strip_index as isize - 1);
    }

    fn increment_strip(&mut self) {
        self.set_active_strip(self.active_strip_index as isize + 1);
    }

    fn increment_strip_width(&mut self, delta: i16) {
        let w = ((self.strip_width as i16 + delta).clamp(1, 15)) as u16;
        self.strip_width = w;
    }

    fn vertical_barchart(&self, strips: &[usb::Strip]) -> BarChart {
        let bars: Vec<Bar> = strips
            .iter()
            .map(|strip| self.vertical_bar(strip))
            .collect();
        let title = Line::from("Channel Strips").centered();

        BarChart::default()
            .data(BarGroup::default().bars(&bars[self.first_strip_index..]))
            .block(Block::bordered().title(title))
            .bar_width(self.strip_width)
            .max(500)
    }

    fn vertical_bar(&self, strip: &usb::Strip) -> Bar {
        let a = strip.min;
        let b = strip.max;
        let c = 4.0;
        let d = 500.0;
        let t = strip.fader;

        let value: u64 = (c + ((d - c) / (b - a)) * (t - a)) as u64;

        let mut fg_color: Color;
        let bg_color = Color::DarkGray;

        match strip.kind {
            usb::StripKind::Channel => {
                fg_color = Color::White;
            }
            usb::StripKind::Bus => {
                fg_color = Color::Yellow;
            }
            usb::StripKind::Main => {
                fg_color = Color::LightBlue;
            }
        }
        if strip.active {
            fg_color = Color::Green;
        }

        let style = Style::new().fg(fg_color).bg(bg_color);

        Bar::default()
            .value(value)
            .label(Line::from(strip.name.to_string()))
            .text_value(format!("{0:>5.1}", strip.fader))
            .style(style)
    }
}
