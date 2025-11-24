use flexi_logger::{FileSpec, detailed_format};
use pan::Pan;
use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout, Margin, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Bar, BarChart, BarGroup, Block, Paragraph},
};
use std::{
    env,
    fs::File,
    io::{Read, Write},
    time::{Duration, Instant},
};
use std::{io, path::Path};
use tui_input::Input;
use tui_input::backend::crossterm::EventHandler;
use usb::StripKind;

use crate::midi_control::{GlobalControl, StripTarget};

mod midi;
mod midi_control;
mod pan;
mod usb;

fn main() -> io::Result<()> {
    let _logger = flexi_logger::Logger::try_with_env()
        .unwrap()
        .log_to_file(FileSpec::default().suppress_timestamp())
        .format(detailed_format)
        .append()
        .start()
        .unwrap();

    log::info!("Starting Baton");
    let mut terminal = ratatui::init();
    let app_result = App::new().run(&mut terminal);
    ratatui::restore();
    log::info!("Ending Baton");
    app_result
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum InputMode {
    #[default]
    Normal,
    Rename,
    Command,
}

pub struct App {
    exit: bool,
    active_mix_index: usize,
    active_strip_index: usize,
    first_strip_index: usize,
    strip_width: u16,
    meter_heigth: u16,
    status_line: String,
    ps: usb::PreSonusStudio1824c,
    tick_rate: Duration,
    last_tick: Instant,
    bypass: bool,
    input: Input,
    input_mode: InputMode,
    midi_input: Option<midi::MidiInput>,
    midi_mapping: midi_control::MidiMapping,
    midi_learn_state: midi_control::MidiLearnState,
}

impl App {
    fn new() -> Self {
        let midi_input = match midi::MidiInput::new() {
            Ok(m) => {
                log::info!("MIDI input initialized");
                Some(m)
            }
            Err(e) => {
                log::warn!("Failed to initialize MIDI input: {}", e);
                None
            }
        };

        // Load or create MIDI mapping
        let midi_mapping_file = match env::var("HOME") {
            Ok(h) => format!("{h}/.baton_midi_mapping.json"),
            Err(_) => ".baton_midi_mapping.json".to_string(),
        };

        let midi_mapping = if let Ok(mut file) = File::open(&midi_mapping_file) {
            let mut contents = String::new();
            file.read_to_string(&mut contents).ok();
            serde_json::from_str(&contents)
                .unwrap_or_else(|_| midi_control::MidiMapping::create_default())
        } else {
            midi_control::MidiMapping::create_default()
        };

        let mut app = App {
            exit: false,
            active_mix_index: 0,
            active_strip_index: 0,
            first_strip_index: 0,
            strip_width: 5,
            meter_heigth: 20,
            status_line: String::with_capacity(256),
            ps: usb::PreSonusStudio1824c::new().expect("Failed to open device"),
            last_tick: Instant::now(),
            tick_rate: Duration::from_millis(100),
            bypass: false,
            input: Input::default(),
            input_mode: InputMode::Normal,
            midi_input,
            midi_mapping,
            midi_learn_state: midi_control::MidiLearnState::Inactive,
        };

        app.set_active_strip(app.active_strip_index as isize);
        app
    }

    fn set_active_strip(&mut self, strip_index: isize) {
        let mix = &mut self.ps.mixes[self.active_mix_index];
        self.active_strip_index =
            strip_index.clamp(0, mix.strips.channel_strips.len() as isize) as usize;

        for s in &mut *mix.strips.channel_strips {
            s.active = false;
        }
        mix.strips.bus_strip.active = false;

        mix.strips
            .iter_mut()
            .nth(self.active_strip_index)
            .unwrap()
            .active = true;
    }

    /// runs the application's main loop until the user quits
    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        // Load config
        let config_file = match env::var("HOME") {
            Ok(h) => format!("{h}/.baton.json"),
            Err(_) => "baton.json".to_string(),
        };
        let path = Path::new(&config_file);
        let file = File::open(&config_file);

        match file {
            Err(_) => (),
            Ok(mut f) => {
                let mut serialized = String::new();
                f.read_to_string(&mut serialized).unwrap();
                self.ps.load_config(&serialized);
                self.ps.write_state();
            }
        }

        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;
            let timeout = self.tick_rate.saturating_sub(self.last_tick.elapsed());
            if event::poll(timeout)? {
                self.handle_events()?;
            }

            if self.last_tick.elapsed() >= self.tick_rate {
                self.on_tick();
                self.last_tick = Instant::now();
            }
        }

        // Save config
        let serialized = serde_json::to_string_pretty(&self.ps).unwrap();
        let mut file = File::create(path).unwrap();
        file.write_all(serialized.as_bytes()).unwrap();
        file.flush().unwrap();

        // Save MIDI mapping
        self.save_midi_mapping();

        Ok(())
    }

    fn on_tick(&mut self) {
        self.ps.poll_state();
        self.process_midi_messages();
    }

    // Add method to start learning
    fn start_midi_learn(&mut self, control: midi_control::StripControl) {
        let target = midi_control::ControlTarget::Strip(midi_control::StripTarget {
            mix_index: self.active_mix_index,
            strip_index: self.active_strip_index,
            control,
        });

        self.midi_learn_state = self.midi_mapping.start_learning(target);
        self.status_line = format!("MIDI Learn: Move a control to assign to {:?}", control);
    }

    fn process_midi_messages(&mut self) {
        let midi_input = match &self.midi_input {
            Some(m) => m,
            None => return,
        };

        let mut messages = Vec::new();
        while let Some(msg) = midi_input.try_recv() {
            messages.push(msg);
        }

        for msg in messages {
            match msg {
                midi::MidiMessage::ControlChange {
                    channel,
                    controller,
                    value,
                } => {
                    let midi_control = midi_control::MidiControl {
                        channel,
                        cc: controller,
                    };

                    // Check if we're in learn mode
                    if self.midi_learn_state != midi_control::MidiLearnState::Inactive {
                        let default_range = match &self.midi_learn_state {
                            midi_control::MidiLearnState::Learning { target } => {
                                midi_control::MidiMapping::default_range_for_control(match target {
                                    midi_control::ControlTarget::Strip(strip_target) => {
                                        &strip_target.control
                                    }
                                    _ => &midi_control::StripControl::Fader, // Default fallback
                                })
                            }
                            _ => continue,
                        };

                        if self.midi_mapping.learn_mapping(
                            &self.midi_learn_state,
                            midi_control,
                            default_range,
                        ) {
                            self.status_line = format!(
                                "MIDI Learn: Assigned channel {} CC {}",
                                channel, controller
                            );
                            self.midi_learn_state = midi_control::MidiLearnState::Inactive;

                            // Save the mapping
                            self.save_midi_mapping();
                        }
                        continue;
                    }

                    // Normal MIDI processing
                    log::debug!(
                        "MIDI CC: channel={}, controller={}, value={}",
                        channel,
                        controller,
                        value
                    );

                    if let Some(target) = self.midi_mapping.get_target(&midi_control).cloned() {
                        let transformed_value =
                            self.midi_mapping.transform_value(&midi_control, value);

                        match target {
                            midi_control::ControlTarget::Strip(strip_target) => {
                                self.handle_strip_control(&strip_target, transformed_value);
                            }
                            midi_control::ControlTarget::Global(global_control) => {
                                self.handle_global_control(&global_control, value);
                            }
                        }
                    }
                }
            }
        }
    }

    // Add method to save MIDI mapping
    fn save_midi_mapping(&mut self) {
        let midi_mapping_file = match env::var("HOME") {
            Ok(h) => format!("{h}/.baton_midi_mapping.json"),
            Err(_) => ".baton_midi_mapping.json".to_string(),
        };

        self.midi_mapping.sort_mappings();
        if let Ok(json) = serde_json::to_string_pretty(&self.midi_mapping) {
            if let Ok(mut file) = File::create(&midi_mapping_file) {
                let _ = file.write_all(json.as_bytes());
                let _ = file.flush();
            }
        }
    }

    fn handle_global_control(&mut self, control: &GlobalControl, value: u8) {
        match control {
            GlobalControl::PhantomPower => {
                if value > 63 {
                    self.toggle_phantom_power();
                }
            }
            GlobalControl::Line1_2 => {
                if value > 63 {
                    self.toggle_1_2_line();
                }
            }
            GlobalControl::MainMute => {
                if value > 63 {
                    self.toggle_main_mute();
                }
            }
            GlobalControl::MainMono => {
                if value > 63 {
                    self.toggle_main_mono();
                }
            }
            GlobalControl::ActiveMixSelect => {
                let mix_index = ((value as f64 / 127.0) * 8.0) as usize;
                self.set_active_mix(mix_index.min(8));
            }
            GlobalControl::ActiveStripSelect => {
                let strip_index = ((value as f64 / 127.0) * 10.0) as usize;
                self.set_active_strip(strip_index as isize);
            }
        }
    }

    fn handle_strip_control(&mut self, target: &StripTarget, value: f64) {
        let mix = &mut self.ps.mixes[target.mix_index];
        let strip = mix.strips.iter_mut().nth(target.strip_index).unwrap();

        match target.control {
            midi_control::StripControl::Fader => {
                strip.set_fader(value);
                self.ps
                    .write_channel_fader(target.mix_index, target.strip_index);
            }
            midi_control::StripControl::Balance => {
                strip.balance = value.clamp(-100.0, 100.0);
                self.ps
                    .write_channel_fader(target.mix_index, target.strip_index);
            }
            midi_control::StripControl::Mute => {
                if value >= 63.0 {
                    self.ps.mixes[target.mix_index]
                        .strips
                        .iter_mut()
                        .nth(target.strip_index)
                        .unwrap()
                        .mute = !strip.mute;
                }
                self.ps.write_state();
            }
            midi_control::StripControl::Solo => {
                if value >= 63.0 {
                    self.ps.mixes[target.mix_index].toggle_solo(target.strip_index);
                    self.ps.write_state();
                }
            }
        }
    }

fn draw(&mut self, frame: &mut Frame) {
    let [state_area, meters_area, pan_area, strips_area, status_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Percentage(self.meter_heigth),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(3),
    ])
    .spacing(0)
    .areas(frame.area());

    // Compose status text
    let active_strip = self.ps.mixes[self.active_mix_index]
        .strips
        .iter()
        .nth(self.active_strip_index)
        .unwrap();
    let name = if self.active_strip_index < self.ps.channel_names.len() {
        self.ps.channel_names[self.active_strip_index].as_str()
    } else {
        &self.ps.mixes[self.active_mix_index].name
    };
    let meter_value = if self.active_strip_index < self.ps.channel_meters.len() {
        self.ps.channel_meters[self.active_strip_index].value
    } else {
        self.ps.bus_meters[self.active_mix_index * 2].value
    };

    let status_line = Line::from(self.status_line.as_str()).left_aligned();

    // Autoscroll left and right
    let strips_width = strips_area.inner(Margin::new(1, 1)).width;
    let strip_display_cap = strips_width / (self.strip_width + 1) - 1;
    while self.active_strip_index < self.first_strip_index {
        self.first_strip_index -= 1;
    }
    while self.active_strip_index > self.first_strip_index + strip_display_cap as usize - 1 {
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

    let mut bypass: Span = Span::from(" Bypass ");
    if self.bypass {
        bypass = bypass.style(Style::new().bold().black().on_light_blue());
    } else {
        bypass = bypass.style(Style::new().reset());
    }

    let state_line = Line::from(vec![
        phantom,
        spacer.clone(),
        line,
        spacer.clone(),
        mute,
        spacer.clone(),
        mono,
        spacer,
        bypass,
    ]);

    frame.render_widget(state_line, state_area);
    frame.render_widget(
        self.meters_barchart(&self.ps.mixes[self.active_mix_index]),
        meters_area,
    );
    
    // Render pan widgets for each visible channel strip
    self.render_pan_widgets(frame, pan_area);
    
    frame.render_widget(
        self.faders_barchart(&self.ps.mixes[self.active_mix_index]),
        strips_area,
    );

    if self.input_mode == InputMode::Rename || self.input_mode == InputMode::Command {
        let title = format!("{:?}", self.input_mode);
        let width = status_area.width.max(3) - 3;
        let style = Style::default();
        let scroll = self.input.visual_scroll(width as usize);
        let input = Paragraph::new(self.input.value())
            .style(style)
            .scroll((0, scroll as u16))
            .block(Block::bordered().title(title));
        frame.render_widget(input, status_area);
        // Ratatui hides the cursor unless it's explicitly set. Position the  cursor past the
        // end of the input text and one line down from the border to the input line
        let x = self.input.visual_cursor().max(scroll) - scroll + 1;
        frame.set_cursor_position((status_area.x + x as u16, status_area.y + 1))
    } else {
        frame.render_widget(
            Paragraph::new(status_line).block(Block::bordered().title("Status")),
            status_area,
        );
    }
}

fn render_pan_widgets(&self, frame: &mut Frame, pan_area: Rect) {
    // Calculate the number of visible strips
    let mix = &self.ps.mixes[self.active_mix_index];
    let num_channel_strips = mix.strips.channel_strips.len();
    let visible_strips_count = (num_channel_strips + 1).min(
        self.first_strip_index + (pan_area.width / (self.strip_width + 1)) as usize
    );
    
    // Create layout constraints for each visible strip
    let mut constraints = Vec::new();
    let visible_end = visible_strips_count.min(num_channel_strips + 1);
    
    // Add 1 character offset to align with BarChart border
    constraints.push(Constraint::Length(1));
    
    for i in self.first_strip_index..visible_end {
        if i < num_channel_strips {
            // Channel strip - add pan widget
            constraints.push(Constraint::Length(self.strip_width));
            constraints.push(Constraint::Length(1)); // Spacer
        } else {
            // Bus/Main strip - add empty space (no pan widget)
            constraints.push(Constraint::Length(self.strip_width));
            constraints.push(Constraint::Length(1)); // Spacer
        }
    }
    
    // Remove the last spacer if exists
    if constraints.len() > 1 {
        constraints.pop();
    }
    
    let pan_areas = Layout::horizontal(&constraints).split(pan_area);
    
    // Render pan widgets only for channel strips
    // Start from index 1 to skip the offset area
    let mut area_idx = 1;
    for i in self.first_strip_index..visible_end {
        if i < num_channel_strips {
            // This is a channel strip - render pan widget
            let strip = &mix.strips.channel_strips[i];
            frame.render_widget(
                Pan {
                    balance: strip.balance as i64,
                },
                pan_areas[area_idx],
            );
        }
        // else: This is bus/main strip - skip rendering pan widget
        
        area_idx += 2; // Skip to next strip (widget + spacer)
    }
}

    fn handle_events(&mut self) -> io::Result<()> {
        let event = event::read()?;
        match event {
            Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                match self.input_mode {
                    InputMode::Normal => self.handle_key_event(key_event),
                    InputMode::Rename | InputMode::Command => match key_event.code {
                        KeyCode::Enter => self.push_message(),
                        KeyCode::Esc => self.stop_editing(),
                        _ => {
                            self.input.handle_event(&event);
                        }
                    },
                }
            }
            _ => {}
        };
        Ok(())
    }

    fn push_message(&mut self) {
        match self.input_mode {
            InputMode::Rename => self.execute_rename(),
            InputMode::Command => self.execute_command(),
            _ => todo!(),
        }
    }

    fn execute_rename(&mut self) {
        match self.ps.mixes[self.active_mix_index]
            .strips
            .iter()
            .nth(self.active_strip_index)
            .unwrap()
            .kind
        {
            StripKind::Channel => {
                self.ps.channel_names[self.active_strip_index] = self.input.value_and_reset();
            }
            StripKind::Bus | StripKind::Main => {
                self.ps.mixes[self.active_mix_index].name = self.input.value_and_reset();
            }
        }

        self.input_mode = InputMode::Normal;
    }

    fn execute_command(&mut self) {
        let command = self.input.value_and_reset();
        match command.as_str() {
            ":mute" => self.toggle_mute(),
            ":solo" => self.toggle_solo(),
            _ => (),
        }

        self.input_mode = InputMode::Normal;
    }

    fn stop_editing(&mut self) {
        self.input_mode = InputMode::Normal;
    }

    fn start_editing(&mut self, mode: InputMode) {
        match mode {
            InputMode::Rename => self.init_rename_channel(),
            InputMode::Command => self.init_input_command(),
            _ => todo!(),
        }
    }

    fn init_rename_channel(&mut self) {
        match self.ps.mixes[self.active_mix_index]
            .strips
            .iter()
            .nth(self.active_strip_index)
            .unwrap()
            .kind
        {
            StripKind::Channel => {
                self.input = Input::new(self.ps.channel_names[self.active_strip_index].to_string());
            }
            StripKind::Bus | StripKind::Main => {
                self.input = Input::new(self.ps.mixes[self.active_mix_index].name.to_string());
            }
        }
        self.input_mode = InputMode::Rename;
    }

    fn init_input_command(&mut self) {
        self.input = Input::new(String::from(':'));
        self.input_mode = InputMode::Command;
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
            KeyCode::Char('9') => self.set_active_mix(8),
            KeyCode::Char('m') => self.toggle_mute(),
            KeyCode::Char('s') => self.toggle_solo(),
            KeyCode::Char('b') => self.toggle_bypass(),
            KeyCode::Char(' ') => self.clear_clip_indicators(),
            KeyCode::Char('F') => {
                if key_event.modifiers == KeyModifiers::SHIFT {
                    self.start_midi_learn(midi_control::StripControl::Fader);
                }
            }
            KeyCode::Char('B') => {
                if key_event.modifiers == KeyModifiers::SHIFT {
                    self.start_midi_learn(midi_control::StripControl::Balance);
                }
            }
            KeyCode::Char('M') => {
                if key_event.modifiers == KeyModifiers::SHIFT {
                    self.start_midi_learn(midi_control::StripControl::Mute);
                }
            }
            KeyCode::Char('S') => {
                if key_event.modifiers == KeyModifiers::SHIFT {
                    self.start_midi_learn(midi_control::StripControl::Solo);
                }
            }
            KeyCode::Esc => {
                // Cancel learn mode if active
                if self.midi_learn_state != midi_control::MidiLearnState::Inactive {
                    self.midi_learn_state = midi_control::MidiLearnState::Inactive;
                    self.status_line = "MIDI Learn cancelled".to_string();
                }
            }
            KeyCode::Char('r') => self.start_editing(InputMode::Rename),
            KeyCode::Char(':') => self.start_editing(InputMode::Command),
            KeyCode::PageDown => self.increment_meter_heigth(1),
            KeyCode::PageUp => self.increment_meter_heigth(-1),
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

    fn clear_clip_indicators(&mut self) {
        for meter in &mut self.ps.channel_meters {
            meter.clip = false;
            meter.max = -f64::INFINITY;
        }
        for meter in &mut self.ps.bus_meters {
            meter.clip = false;
            meter.max = -f64::INFINITY;
        }
    }

    fn increment_meter_heigth(&mut self, delta: i16) {
        let mh = (self.meter_heigth as i16 + delta).clamp(0, 100) as u16;
        self.meter_heigth = mh;
    }

    fn increment_fader(&mut self, delta: f64) {
        let strip = &mut self.ps.mixes[self.active_mix_index]
            .strips
            .iter_mut()
            .nth(self.active_strip_index)
            .unwrap();
        let current = strip.fader;
        strip.set_fader(current + delta);
        self.write_active_fader();
    }

    fn write_active_fader(&mut self) {
        self.ps
            .write_channel_fader(self.active_mix_index, self.active_strip_index);
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
        let strip = &mut self.ps.mixes[self.active_mix_index].strips.channel_strips
            [self.active_strip_index];
        strip.balance = (strip.balance + delta).clamp(-100.0, 100.0);

        self.write_active_fader();
    }

    fn center_balance(&mut self) {
        let strip = &mut self.ps.mixes[self.active_mix_index].strips.channel_strips
            [self.active_strip_index];
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
        match self.ps.mixes[self.active_mix_index]
            .strips
            .iter()
            .nth(self.active_strip_index)
            .unwrap()
            .kind
        {
            StripKind::Channel | StripKind::Bus => {
                let muted = self.ps.mixes[self.active_mix_index]
                    .strips
                    .iter()
                    .nth(self.active_strip_index)
                    .unwrap()
                    .mute;

                self.ps.mixes[self.active_mix_index]
                    .strips
                    .iter_mut()
                    .nth(self.active_strip_index)
                    .unwrap()
                    .mute = !muted;

                self.write_active_fader();
            }
            StripKind::Main => {
                self.toggle_main_mute();
            }
        }
    }

    fn toggle_solo(&mut self) {
        self.ps.mixes[self.active_mix_index].toggle_solo(self.active_strip_index);
        self.ps.write_state();
    }

    fn toggle_bypass(&mut self) {
        self.bypass = !self.bypass;
        if self.bypass {
            self.ps.bypass_mixer();
        } else {
            self.ps.write_state();
        }
    }

    fn set_active_mix(&mut self, index: usize) {
        self.active_mix_index = index;
        self.set_active_strip(self.active_strip_index as isize);
    }

    fn faders_barchart(&self, mix: &usb::Mix) -> BarChart<'_> {
        let mut bars: Vec<Bar> = mix
            .strips
            .channel_strips
            .iter()
            .enumerate()
            .map(|(i, strip)| self.fader_bar(strip, self.ps.channel_names[i].as_str()))
            .collect();
        bars.push(self.fader_bar(&mix.strips.bus_strip, &mix.name));
        let title = self.ps.mixes[self.active_mix_index].name.as_str();
        let title = Line::from(title).centered().bold();

        BarChart::default()
            .data(BarGroup::default().bars(&bars[self.first_strip_index..]))
            .block(Block::bordered().title(title))
            .bar_width(self.strip_width)
            .max(500)
    }

    fn fader_bar(&self, strip: &usb::Strip, name: &str) -> Bar<'_> {
        let a = strip.min;
        let b = strip.max;
        let c = 20.0;
        let d = 500.0;
        let t = strip.fader;

        let value: u64 = (c + ((d - c) / (b - a)) * (t - a)) as u64;

        let mut strip_fg_color: Color;
        let mut label_fg_color = Color::White;
        let strip_bg_color = Color::DarkGray;
        let mut label_bg_color = Color::Reset;

        match strip.kind {
            usb::StripKind::Channel => {
                strip_fg_color = Color::White;
            }
            usb::StripKind::Bus => {
                strip_fg_color = Color::Yellow;
            }
            usb::StripKind::Main => {
                strip_fg_color = Color::LightBlue;
            }
        }
        if strip.active {
            strip_fg_color = Color::Green;
        }
        if strip.mute_by_solo {
            label_bg_color = Color::Reset;
            label_fg_color = Color::Red;
        }
        if strip.mute {
            label_bg_color = Color::Red;
            label_fg_color = Color::Black;
        }
        if strip.solo {
            label_bg_color = Color::Yellow;
            label_fg_color = Color::Black;
        }

        let style = Style::new().fg(strip_fg_color).bg(strip_bg_color);

        Bar::default()
            .value(value)
            .label(
                Line::from(name.to_string())
                    .fg(label_fg_color)
                    .bg(label_bg_color),
            )
            .text_value(format!("{0:>5.1}", strip.fader))
            .style(style)
    }

    // fn pan_widgets(&self, mix: &usb::Mix) -> Widget {
    //     let mut pans: Vec<Pan> = mix
    //         .channel_strips
    //         .iter()
    //         .map(|strip| Pan {
    //             balance: strip.balance,
    //         })
    //         .collect();

    //     Layout::horizontal(constraints)
    // }

    fn meters_barchart(&self, mix: &usb::Mix) -> BarChart<'_> {
        let mut bars: Vec<Bar> = self
            .ps
            .channel_meters
            .iter()
            .enumerate()
            .map(|(i, meter)| {
                self.meter_bar(
                    meter.clip,
                    self.ps.channel_names[i].as_str(),
                    meter.value,
                    meter.max,
                )
            })
            .collect();
        let bus_meter_left = &self.ps.bus_meters[self.active_mix_index * 2];
        let bus_meter_right = &self.ps.bus_meters[self.active_mix_index * 2 + 1];
        bars.push(self.meter_bar(
            bus_meter_left.clip,
            &mix.name,
            bus_meter_left.value,
            bus_meter_left.max,
        ));
        bars.push(self.meter_bar(
            bus_meter_right.clip,
            &mix.name,
            bus_meter_right.value,
            bus_meter_right.max,
        ));
        let title = "Meters";
        let title = Line::from(title).centered().bold();

        BarChart::default()
            .data(BarGroup::default().bars(&bars[self.first_strip_index..]))
            .block(Block::bordered().title(title))
            .bar_width(self.strip_width)
            .max(500)
    }

    fn meter_bar(&self, clip: bool, name: &str, meter_value: f64, meter_max_value: f64) -> Bar<'_> {
        let a = -50.0;
        let b = 0.0;
        let c = 0.0;
        let d = 500.0;
        let t = meter_value;

        let value: u64 = (c + ((d - c) / (b - a)) * (t - a)) as u64;

        let mut strip_fg_color = Color::Rgb(0, 185, 0);
        let mut label_fg_color = Color::White;
        let strip_bg_color = Color::DarkGray;
        let label_bg_color = Color::Reset;

        if clip {
            label_fg_color = Color::Red;
        }
        if meter_value > -18.0 {
            strip_fg_color = Color::Green;
        }
        if meter_value > -9.0 {
            strip_fg_color = Color::Yellow;
        }
        if meter_value > -6.0 {
            strip_fg_color = Color::Rgb(255, 165, 0);
        }
        if meter_value > -3.0 {
            strip_fg_color = Color::Red;
        }

        let style = Style::new().fg(strip_fg_color).bg(strip_bg_color);

        Bar::default()
            .value(value)
            .label(
                Line::from(name.to_string())
                    .fg(label_fg_color)
                    .bg(label_bg_color),
            )
            .text_value(format!("{0:>5.1}", meter_max_value))
            .style(style)
    }
}
