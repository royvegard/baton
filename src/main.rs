use futures_lite::future::block_on;
use std::io;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use nusb::{
    transfer::{ControlOut, ControlType, Recipient},
    Device,
};
use ratatui::{
    layout::{Constraint, Layout, Margin},
    style::{Color, Style, Stylize},
    text::Line,
    widgets::{Bar, BarChart, BarGroup, Block, Paragraph},
    DefaultTerminal, Frame,
};

mod usb;

fn main() -> io::Result<()> {
    let device_info = nusb::list_devices()
        .unwrap()
        .find(|dev| dev.vendor_id() == 0x194f && dev.product_id() == 0x010d)
        .expect("device not connected");

    let device = device_info.open().expect("failed to open device");

    let mut terminal = ratatui::init();
    let mut names = vec![];

    for i in 1..=8 {
        names.push(format!("MIC {}", i));
    }
    for i in 1..=8 {
        names.push(format!("ADAT {}", i));
    }
    names.push("S/PDIF 1".to_string());
    names.push("S/PDIF 2".to_string());
    for i in 1..=16 {
        names.push(format!("DAW {}", i));
    }
    names.push("S/PDIF 1".to_string());
    names.push("S/PDIF 2".to_string());

    names.push("AUX 3-4".to_string());
    names.push("AUX 5-6".to_string());
    names.push("AUX 7-8".to_string());
    names.push("MAIN 1-2".to_string());
    names.push("AUX 9-10".to_string());
    names.push("AUX 11-12".to_string());
    names.push("AUX 13-14".to_string());
    names.push("AUX 15-16".to_string());

    let app_result = App::from_channel_names(&names, device).run(&mut terminal);
    ratatui::restore();
    app_result
}

pub struct App {
    exit: bool,
    strips: Vec<Strip>,
    active_strip_index: usize,
    first_strip_index: usize,
    strip_width: u16,
    strip_display_cap: u16,
    status_line: String,
    usb_command: usb::FaderCommand,
    usb_device: Device,
}

#[derive(Debug)]
enum StripKind {
    Channel,
    Bus(u32),
    Main(u32),
}

pub struct Strip {
    name: String,
    fader: f64,
    balance: f64,
    solo: bool,
    mute: bool,
    max: f64,
    min: f64,
    active: bool,
    kind: StripKind,
}

impl Strip {
    fn set_fader(&mut self, value: f64) {
        self.fader = value.clamp(self.min, self.max);
    }
}

impl App {
    fn from_channel_names(names: &[String], device: Device) -> Self {
        let mut strips: Vec<Strip> = vec![];
        let mut bus_number = 1;
        for n in names {
            let mut strip = Strip {
                name: n.to_string(),
                active: false,
                fader: 0.0,
                solo: false,
                mute: false,
                min: -96.0,
                max: 10.0,
                balance: 0.0,
                kind: StripKind::Channel,
            };

            if n.contains("AUX") {
                strip.kind = StripKind::Bus(bus_number);
                bus_number += 1;
            }
            if n.contains("MAIN") {
                strip.kind = StripKind::Main(bus_number);
                bus_number += 1;
            }

            strips.push(strip);
        }

        let mut app = App {
            exit: false,
            strips,
            active_strip_index: 0,
            strip_width: 5,
            strip_display_cap: 1,
            first_strip_index: 0,
            status_line: String::with_capacity(256),
            usb_command: usb::FaderCommand::new(),
            usb_device: device,
        };

        app.set_active_strip(app.active_strip_index as isize);
        app
    }

    fn set_active_strip(&mut self, index: isize) {
        self.active_strip_index = index.clamp(0, (self.strips.len() - 1) as isize) as usize;

        for s in &mut self.strips {
            s.active = false;
        }
        self.strips[self.active_strip_index].active = true;
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
            "{:?} {} ({:>5.1} dB)",
            self.strips[self.active_strip_index].kind,
            self.strips[self.active_strip_index].name,
            self.strips[self.active_strip_index].fader
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
        frame.render_widget(self.vertical_barchart(&self.strips), strips_area);
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

    fn send_usb_command(&self) {
        let fader_control: ControlOut = ControlOut {
            control_type: ControlType::Vendor,
            recipient: Recipient::Device,
            request: 160,
            value: 0x0000,
            index: 0,
            data: &self.usb_command.as_array(),
        };

        let result = block_on(self.usb_device.control_out(fader_control))
            .into_result()
            .unwrap();
    }

    fn increment_fader(&mut self, delta: f64) {
        let current = self.strips[self.active_strip_index].fader;
        self.strips[self.active_strip_index].set_fader(current + delta);

        self.usb_command
            .set_db(self.strips[self.active_strip_index].fader);

        match self.strips[self.active_strip_index].kind {
            StripKind::Main(n) => {
                self.usb_command.input_strip = 0x00;
                self.usb_command.output_strip = n;
                self.usb_command.mode = usb::MODE_BUS_STRIP;

                self.usb_command.output_channel = usb::LEFT;
                self.send_usb_command();
                self.usb_command.output_channel = usb::RIGHT;
                self.send_usb_command();
            }
            StripKind::Channel => {
                self.usb_command.input_strip = self.active_strip_index as u32;
                self.usb_command.mode = usb::MODE_CHANNEL_STRIP;

                self.usb_command.output_strip = 0x04;
                self.usb_command.output_channel = usb::LEFT;
                self.send_usb_command();
                self.usb_command.output_channel = usb::RIGHT;
                self.send_usb_command();

                self.usb_command.output_strip = 0x00;
                self.usb_command.output_channel = usb::LEFT;
                self.send_usb_command();
                self.usb_command.output_channel = usb::RIGHT;
                self.send_usb_command();
            }
            StripKind::Bus(n) => {
                self.usb_command.input_strip = 0x00;
                self.usb_command.output_strip = n;
                self.usb_command.mode = usb::MODE_BUS_STRIP;

                self.usb_command.output_channel = usb::LEFT;
                self.send_usb_command();
                self.usb_command.output_channel = usb::RIGHT;
                self.send_usb_command();
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
        let w = ((self.strip_width as i16 + delta).clamp(3, 15)) as u16;
        self.strip_width = w;
    }

    fn vertical_barchart(&self, strips: &[Strip]) -> BarChart {
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

    fn vertical_bar(&self, strip: &Strip) -> Bar {
        let a = strip.min;
        let b = strip.max;
        let c = 4.0;
        let d = 500.0;
        let t = strip.fader;

        let value: u64 = (c + ((d - c) / (b - a)) * (t - a)) as u64;

        let mut fg_color: Color;
        let bg_color = Color::DarkGray;

        match strip.kind {
            StripKind::Channel => {
                fg_color = Color::White;
            }
            StripKind::Bus(_) => {
                fg_color = Color::Yellow;
            }
            StripKind::Main(_) => {
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
