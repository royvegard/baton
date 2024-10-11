use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    layout::{Constraint, Layout, Margin},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Bar, BarChart, BarGroup, Block, Paragraph},
    DefaultTerminal, Frame,
};
use std::io;
use std::time::{Duration, Instant};
use usb::State;

mod usb;

fn main() -> io::Result<()> {
    let mut terminal = ratatui::init();
    let app_result = App::new().run(&mut terminal);
    ratatui::restore();
    app_result
}

pub struct App {
    exit: bool,
    active_mix_index: usize,
    active_strip_index: usize,
    first_strip_index: usize,
    strip_width: u16,
    strip_display_cap: u16,
    status_line: String,
    ps: usb::PreSonusStudio1824c,
    last_tick: Instant,
}

impl App {
    fn new() -> Self {
        let mut app = App {
            exit: false,
            active_mix_index: 0,
            active_strip_index: 0,
            first_strip_index: 0,
            strip_width: 5,
            strip_display_cap: 1,
            status_line: String::with_capacity(256),
            ps: usb::PreSonusStudio1824c::new().expect("Failed to open device"),
            last_tick: Instant::now(),
        };

        app.set_active_strip(app.active_strip_index as isize);
        app
    }

    fn set_active_strip(&mut self, strip_index: isize) {
        let strips = &mut self.ps.mixes[self.active_mix_index].channel_strips;
        self.active_strip_index = strip_index.clamp(0, (strips.len() - 1) as isize) as usize;

        for s in &mut *strips {
            s.active = false;
        }
        strips[self.active_strip_index].active = true;
    }

    /// runs the application's main loop until the user quits
    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        let tick_rate = Duration::from_millis(100);
        self.last_tick = Instant::now();

        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;
            let timeout = tick_rate.saturating_sub(self.last_tick.elapsed());
            if event::poll(timeout)? {
                self.handle_events()?;
            }

            if self.last_tick.elapsed() >= tick_rate {
                self.on_tick();
                self.last_tick = Instant::now();
            }
        }
        Ok(())
    }

    fn on_tick(&mut self) {
        self.ps.poll_state();
    }

    fn draw(&mut self, frame: &mut Frame) {
        let [title_area, state_area, strips_area, status_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(3),
        ])
        .spacing(0)
        .areas(frame.area());

        // Compose status text
        self.status_line.clear();
        let active_strip =
            &self.ps.mixes[self.active_mix_index].channel_strips[self.active_strip_index];
        self.status_line.push_str(&format!(
            "{:?} {} - {} ({:>5.1} dB) balance: {}, solo: {}, mute: {}, mute_by_solo: {}, daw0: {:>.3}",
            active_strip.kind,
            active_strip.number,
            active_strip.name,
            active_strip.fader,
            active_strip.balance,
            active_strip.solo,
            active_strip.mute,
            active_strip.mute_by_solo,
            State::get_db(self.ps.state.daw[0]),
        ));
        let status_line = Line::from(self.status_line.as_str()).left_aligned();

        // Autoscroll left and right
        let strips_width = strips_area.inner(Margin::new(1, 1)).width;
        self.strip_display_cap = strips_width / (self.strip_width + 1);
        while self.active_strip_index < self.first_strip_index {
            self.first_strip_index -= 1;
        }
        while self.active_strip_index > self.first_strip_index + self.strip_display_cap as usize - 1
        {
            self.first_strip_index += 1;
        }

        // Compose state text
        let spacer = Span::from("|").reset();
        let mut phantom: Span = Span::from(" 48V ");
        if self.ps.state.phantom == 0x01 {
            phantom = phantom.style(Style::new().bold().black().on_blue());
        } else {
            phantom = phantom.style(Style::new().reset());
        }

        let mut line: Span = Span::from(" 1-2 Line ");
        if self.ps.state.line == 0x01 {
            line = line.style(Style::new().bold().black().on_blue());
        } else {
            line = line.style(Style::new().reset());
        }

        let mut mute: Span = Span::from(" Mute ");
        if self.ps.state.mute == 0x01 {
            mute = mute.style(Style::new().bold().black().on_red());
        } else {
            mute = mute.style(Style::new().reset());
        }

        let mut mono: Span = Span::from(" Mono ");
        if self.ps.state.mono == 0x01 {
            mono = mono.style(Style::new().bold().black().on_yellow());
        } else {
            mono = mono.style(Style::new().reset());
        }

        let state_line = Line::from(vec![
            phantom,
            spacer.clone(),
            line,
            spacer.clone(),
            mute,
            spacer,
            mono,
        ]);

        frame.render_widget("Mixer".bold().into_centered_line(), title_area);
        frame.render_widget(state_line, state_area);
        frame.render_widget(
            self.vertical_barchart(&self.ps.mixes[self.active_mix_index]),
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
            KeyCode::Char('l') => self.toggle_1_2_line(),
            KeyCode::Char('u') => self.toggle_main_mute(),
            KeyCode::Char('o') => self.toggle_main_mono(),
            KeyCode::Char('p') => self.toggle_phantom_power(),
            KeyCode::Char('1') => self.set_active_mix(0),
            KeyCode::Char('2') => self.set_active_mix(1),
            KeyCode::Char('3') => self.set_active_mix(2),
            KeyCode::Char('4') => self.set_active_mix(3),
            KeyCode::Char('5') => self.set_active_mix(4),
            KeyCode::Char('6') => self.set_active_mix(5),
            KeyCode::Char('7') => self.set_active_mix(6),
            KeyCode::Char('8') => self.set_active_mix(7),
            KeyCode::Char('m') => self.toggle_mute(),
            KeyCode::Char('s') => self.toggle_solo(),
            KeyCode::Char('x') => {
                if key_event.modifiers == KeyModifiers::CONTROL {
                    self.increment_balance(-10.0);
                } else {
                    self.increment_balance(-1.0);
                }
            }
            KeyCode::Char('c') => self.center_balance(),
            KeyCode::Char('v') => {
                if key_event.modifiers == KeyModifiers::CONTROL {
                    self.increment_balance(10.0);
                } else {
                    self.increment_balance(1.0);
                }
            }
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
        let strip =
            &mut self.ps.mixes[self.active_mix_index].channel_strips[self.active_strip_index];
        let current = strip.fader;
        strip.set_fader(current + delta);
        self.write_active_fader();
    }

    fn write_active_fader(&mut self) {
        let strip =
            &mut self.ps.mixes[self.active_mix_index].channel_strips[self.active_strip_index];

        let fader = strip.fader;
        let (left, right) = strip.pan_rule(usb::PanRule::Simple);
        match strip.kind {
            usb::StripKind::Main | usb::StripKind::Bus => {
                self.ps.command.input_strip = self.ps.mixes[self.active_mix_index]
                    .get_destination_strip()
                    .number;
                self.ps.command.mode = usb::MODE_BUS_STRIP;
                self.ps.command.output_strip = 0x00;

                self.ps.command.output_channel = usb::LEFT;
                self.ps.command.set_db(fader);
                self.ps.send_command();
            }
            usb::StripKind::Channel => {
                let output_strip = self.ps.mixes[self.active_mix_index].get_destination_strip();
                self.ps.command.input_strip = self.active_strip_index as u32;
                self.ps.command.mode = usb::MODE_CHANNEL_STRIP;
                self.ps.command.output_strip = output_strip.number;

                self.ps.command.output_channel = usb::LEFT;
                self.ps.command.set_db(left);
                self.ps.send_command();

                self.ps.command.output_channel = usb::RIGHT;
                self.ps.command.set_db(right);
                self.ps.send_command();

                if let usb::StripKind::Main = output_strip.kind {
                    self.ps.command.output_strip = 0;

                    self.ps.command.output_channel = usb::LEFT;
                    self.ps.command.set_db(fader);
                    self.ps.send_command();
                }
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

    fn increment_balance(&mut self, delta: f64) {
        let strip =
            &mut self.ps.mixes[self.active_mix_index].channel_strips[self.active_strip_index];
        strip.balance = (strip.balance + delta).clamp(-100.0, 100.0);

        self.write_active_fader();
    }

    fn center_balance(&mut self) {
        let strip =
            &mut self.ps.mixes[self.active_mix_index].channel_strips[self.active_strip_index];
        strip.balance = 0.0;

        self.write_active_fader();
    }

    fn toggle_phantom_power(&mut self) {
        self.ps.set_phantom_power(!self.ps.phantom_power);
    }

    fn toggle_1_2_line(&mut self) {
        self.ps.set_1_2_line(!self.ps.in_1_2_line);
    }

    fn toggle_main_mute(&mut self) {
        self.ps.set_main_mute(!self.ps.main_mute);
    }

    fn toggle_main_mono(&mut self) {
        self.ps.set_main_mono(!self.ps.main_mono);
    }

    fn toggle_mute(&mut self) {
        if let usb::StripKind::Channel =
            self.ps.mixes[self.active_mix_index].channel_strips[self.active_strip_index].kind
        {
            self.ps.mixes[self.active_mix_index].channel_strips[self.active_strip_index].mute =
                !self.ps.mixes[self.active_mix_index].channel_strips[self.active_strip_index].mute;

            let dest_strip = self.ps.mixes[self.active_mix_index]
                .get_destination_strip()
                .number;
            let s =
                &mut self.ps.mixes[self.active_mix_index].channel_strips[self.active_strip_index];

            self.ps.command.input_strip = self.active_strip_index as u32;
            if s.mute | s.mute_by_solo {
                self.ps.command.value = usb::MUTED;
            } else {
                self.ps.command.set_db(s.fader);
            }
            if s.solo {
                self.ps.command.set_db(s.fader);
            }
            self.ps.command.mode = usb::MODE_CHANNEL_STRIP;
            self.ps.command.output_strip = dest_strip;
            self.ps.command.output_channel = usb::LEFT;
            self.ps.send_command();
            self.ps.command.output_channel = usb::RIGHT;
            self.ps.send_command();
        }
    }

    fn toggle_solo(&mut self) {
        if let usb::StripKind::Channel =
            self.ps.mixes[self.active_mix_index].channel_strips[self.active_strip_index].kind
        {
            self.ps.mixes[self.active_mix_index].channel_strips[self.active_strip_index].solo =
                !self.ps.mixes[self.active_mix_index].channel_strips[self.active_strip_index].solo;

            let length = self.ps.mixes[self.active_mix_index].channel_strips.len() - 1;
            let dest_strip = self.ps.mixes[self.active_mix_index]
                .get_destination_strip()
                .number;

            let mut solo_exists = false;
            for s in self.ps.mixes[self.active_mix_index]
                .channel_strips
                .iter()
                .take(length)
            {
                if s.solo {
                    solo_exists = true;
                }
            }

            if solo_exists {
                for i in 0..length {
                    let s = &mut self.ps.mixes[self.active_mix_index].channel_strips[i];
                    s.mute_by_solo = !s.solo;

                    self.ps.command.input_strip = i as u32;
                    if s.mute | s.mute_by_solo {
                        self.ps.command.value = usb::MUTED;
                    }
                    if s.solo {
                        self.ps.command.set_db(s.fader);
                    }
                    self.ps.command.mode = usb::MODE_CHANNEL_STRIP;
                    self.ps.command.output_strip = dest_strip;
                    self.ps.command.output_channel = usb::LEFT;
                    self.ps.send_command();
                    self.ps.command.output_channel = usb::RIGHT;
                    self.ps.send_command();
                }
            } else {
                for i in 0..length {
                    let s = &mut self.ps.mixes[self.active_mix_index].channel_strips[i];
                    s.mute_by_solo = false;

                    self.ps.command.input_strip = i as u32;
                    if !s.mute & !s.mute_by_solo {
                        self.ps.command.set_db(s.fader);
                    } else {
                        self.ps.command.value = usb::MUTED;
                    }
                    self.ps.command.mode = usb::MODE_CHANNEL_STRIP;
                    self.ps.command.output_strip = dest_strip;
                    self.ps.command.output_channel = usb::LEFT;
                    self.ps.send_command();
                    self.ps.command.output_channel = usb::RIGHT;
                    self.ps.send_command();
                }
            }
        }
    }

    fn set_active_mix(&mut self, index: usize) {
        self.active_mix_index = index;
        self.set_active_strip(self.active_strip_index as isize);
    }

    fn vertical_barchart(&self, mix: &usb::Mix) -> BarChart {
        let bars: Vec<Bar> = mix
            .channel_strips
            .iter()
            .map(|strip| self.vertical_bar(strip))
            .collect();
        let title = self.ps.mixes[self.active_mix_index]
            .get_destination_strip()
            .name
            .as_str();
        let title = Line::from(title).left_aligned().bold();

        BarChart::default()
            .data(BarGroup::default().bars(&bars[self.first_strip_index..]))
            .block(Block::bordered().title(title))
            .bar_width(self.strip_width)
            .max(500)
    }

    fn vertical_bar(&self, strip: &usb::Strip) -> Bar {
        let a = strip.min;
        let b = strip.max;
        let c = 20.0;
        let d = 500.0;
        let t = strip.fader;

        let value: u64 = (c + ((d - c) / (b - a)) * (t - a)) as u64;

        let mut fg_color: Color;
        let mut bg_color = Color::DarkGray;

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
        if strip.mute | strip.mute_by_solo {
            bg_color = Color::Red;
        }
        if strip.solo {
            bg_color = Color::Yellow;
        }

        let style = Style::new().fg(fg_color).bg(bg_color);

        Bar::default()
            .value(value)
            .label(Line::from(strip.name.to_string()))
            .text_value(format!("{0:>5.1}", strip.fader))
            .style(style)
    }
}
