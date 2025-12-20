// src/gui_main.rs
use eframe::egui;
use flexi_logger::{FileSpec, detailed_format};
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Write},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

mod midi;
mod midi_control;
mod usb;

enum StripAction {
    None,
    FaderChanged(f64, String),
    SoloToggled,
    StartMidiLearnFader,
    StartMidiLearnPan,
    StartMidiLearnMute,
    StartMidiLearnSolo,
    NameChanged(String),
    ColorChanged(egui::Color32),
}

fn main() -> eframe::Result {
    let _logger = flexi_logger::Logger::try_with_env()
        .unwrap()
        .log_to_file(FileSpec::default().suppress_timestamp())
        .format(detailed_format)
        .append()
        .start()
        .unwrap();

    log::info!("Starting Baton GUI");

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_min_inner_size([800.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Baton Mixer",
        options,
        Box::new(|cc| Ok(Box::new(BatonApp::new(cc)))),
    )
}

struct BatonApp {
    ps: Arc<Mutex<usb::PreSonusStudio1824c>>,
    config_dir: Option<std::path::PathBuf>,
    midi_input: Option<midi::MidiInput>,
    midi_mapping: midi_control::MidiMapping,
    midi_learn_state: midi_control::MidiLearnState,
    midi_learn_start_time: Option<Instant>,
    active_mix_index: usize,
    active_strip_index: usize,
    last_tick: Instant,
    tick_rate: Duration,
    bypass: bool,
    status_message: String,
    clip_indicators: HashMap<String, Instant>, // Track clip times by meter ID
    peak_holds: HashMap<String, (f64, Instant)>, // Track peak values and times by meter ID
    meter_averages: HashMap<String, Vec<(f64, Instant)>>, // Track meter history for running average
    strip_colors: HashMap<String, egui::Color32>, // Track custom colors by strip ID (mix_index:strip_index)
}

impl BatonApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
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

        // Initialize config directory
        let mut config_dir = dirs::config_dir().map(|d| d.join("baton"));
        if let Some(ref dir) = config_dir {
            if !dir.exists() {
                if let Err(e) = std::fs::create_dir_all(dir) {
                    log::warn!("Failed to create config directory {}: {}", dir.display(), e);
                    config_dir = None;
                }
            }
        }

        let ps = Arc::new(Mutex::new(
            usb::PreSonusStudio1824c::new().expect("Failed to open device"),
        ));

        // Load config
        let mut midi_mapping = midi_control::MidiMapping::create_default();
        match config_dir {
            Some(ref dir) => {
                let config_file = dir.join("config.json");
                if let Ok(mut file) = File::open(&config_file) {
                    let mut serialized = String::new();
                    if file.read_to_string(&mut serialized).is_ok() {
                        let mut ps_lock = ps.lock().unwrap();
                        ps_lock.load_config(&serialized);
                        ps_lock.write_state();
                    }
                }

                // Load MIDI mapping
                let midi_mapping_file = dir.join("midi_mapping.json");
                if let Ok(mut file) = File::open(&midi_mapping_file) {
                    let mut contents = String::new();
                    file.read_to_string(&mut contents).ok();
                    match serde_json::from_str(&contents) {
                        Ok(mapping) => {
                            midi_mapping = mapping;
                        }
                        Err(e) => {
                            log::warn!(
                                "Failed to parse MIDI mapping from {}: {}",
                                midi_mapping_file.display(),
                                e
                            );
                        }
                    }
                }
            }
            None => (),
        }

        Self {
            ps,
            config_dir,
            midi_input,
            midi_mapping,
            midi_learn_state: midi_control::MidiLearnState::Inactive,
            midi_learn_start_time: None,
            active_mix_index: 0,
            active_strip_index: 0,
            last_tick: Instant::now(),
            tick_rate: Duration::from_millis(33),
            bypass: false,
            status_message: String::new(),
            clip_indicators: HashMap::new(),
            peak_holds: HashMap::new(),
            meter_averages: HashMap::new(),
            strip_colors: HashMap::new(),
        }
    }

    fn process_midi_messages(&mut self) {
        let midi_input = match &self.midi_input {
            Some(m) => m,
            None => return,
        };

        // Collect messages to process
        let mut messages = Vec::new();
        while let Some(msg) = midi_input.try_recv() {
            messages.push(msg);
        }

        let mut should_save = false;

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
                                    _ => &midi_control::StripControl::Fader,
                                })
                            }
                            _ => continue,
                        };

                        if self.midi_mapping.learn_mapping(
                            &self.midi_learn_state,
                            midi_control,
                            default_range,
                        ) {
                            self.status_message = format!(
                                "MIDI Learn: Assigned channel {} CC {}",
                                channel, controller
                            );
                            self.midi_learn_state = midi_control::MidiLearnState::Inactive;
                            self.midi_learn_start_time = None;
                            should_save = true;
                        }
                        continue;
                    }

                    // Normal MIDI processing
                    if let Some(target) = self.midi_mapping.get_target(&midi_control).cloned() {
                        let transformed_value =
                            self.midi_mapping.transform_value(&midi_control, value);

                        match target {
                            midi_control::ControlTarget::Strip(strip_target) => {
                                self.handle_strip_control(&strip_target, transformed_value, value);
                            }
                            midi_control::ControlTarget::Global(global_control) => {
                                self.handle_global_control(&global_control, value);
                            }
                        }
                    }
                }
            }
        }

        if should_save {
            self.save_midi_mapping();
        }
    }

    fn handle_strip_control(
        &mut self,
        target: &midi_control::StripTarget,
        value: f64,
        raw_value: u8,
    ) {
        let mut ps = self.ps.lock().unwrap();
        let mix = &mut ps.mixes[target.mix_index];
        let strip = mix.strips.iter_mut().nth(target.strip_index).unwrap();

        match target.control {
            midi_control::StripControl::Fader => {
                strip.set_fader(value);
                ps.write_channel_fader(target.mix_index, target.strip_index);
            }
            midi_control::StripControl::Balance => {
                strip.balance = value.clamp(-100.0, 100.0);
                ps.write_channel_fader(target.mix_index, target.strip_index);
            }
            midi_control::StripControl::Mute => {
                // Only toggle when MIDI value is >= 63 (button press)
                if raw_value >= 63 {
                    strip.mute = !strip.mute;
                    ps.write_state();
                }
            }
            midi_control::StripControl::Solo => {
                // Only toggle when MIDI value is >= 63 (button press)
                if raw_value >= 63 {
                    ps.mixes[target.mix_index].toggle_solo(target.strip_index);
                    ps.write_state();
                }
            }
        }
    }

    fn handle_global_control(&mut self, control: &midi_control::GlobalControl, value: u8) {
        let mut ps = self.ps.lock().unwrap();
        match control {
            midi_control::GlobalControl::PhantomPower => {
                if value > 63 {
                    let phantom_power = ps.phantom_power;
                    ps.set_phantom_power(!phantom_power);
                }
            }
            midi_control::GlobalControl::Line1_2 => {
                if value > 63 {
                    let in_1_2_line = ps.in_1_2_line;
                    ps.set_1_2_line(!in_1_2_line);
                }
            }
            midi_control::GlobalControl::MainMute => {
                if value > 63 {
                    let main_mute = ps.main_mute;
                    ps.set_main_mute(!main_mute);
                }
            }
            midi_control::GlobalControl::MainMono => {
                if value > 63 {
                    let main_mono = ps.main_mono;
                    ps.set_main_mono(!main_mono);
                }
            }
            midi_control::GlobalControl::ActiveMixSelect => {
                let mix_index = ((value as f64 / 127.0) * 8.0) as usize;
                self.active_mix_index = mix_index.min(8);
            }
            midi_control::GlobalControl::ActiveStripSelect => {
                let strip_index = ((value as f64 / 127.0) * 10.0) as usize;
                self.active_strip_index = strip_index;
            }
        }
    }

    fn save_midi_mapping(&mut self) {
        match self.config_dir {
            Some(ref dir) => {
                let midi_mapping_file = dir.join("midi_mapping.json");
                self.midi_mapping.sort_mappings();
                if let Ok(json) = serde_json::to_string_pretty(&self.midi_mapping) {
                    if let Ok(mut file) = File::create(&midi_mapping_file) {
                        let _ = file.write_all(json.as_bytes());
                        let _ = file.flush();
                    }
                }
            }
            None => (),
        }
    }

    fn draw_strip(
        ui: &mut egui::Ui,
        strip: &mut usb::Strip,
        name: &mut String,
        meter_value: f64,
        meter_value_right: Option<f64>,
        available_height: f32,
        clip_indicators: &mut HashMap<String, Instant>,
        peak_holds: &mut HashMap<String, (f64, Instant)>,
        meter_averages: &mut HashMap<String, Vec<(f64, Instant)>>,
        meter_id: &str,
        custom_color: Option<egui::Color32>,
    ) -> StripAction {
        let mut action = StripAction::None;

        // Set background color - use custom color if set, otherwise default based on strip kind
        let bg_color = custom_color.unwrap_or_else(|| match strip.kind {
            usb::StripKind::Main => egui::Color32::from_rgb(80, 80, 0), // Dark yellow
            usb::StripKind::Bus => egui::Color32::from_rgb(20, 30, 50), // Dark blue
            usb::StripKind::Channel => egui::Color32::TRANSPARENT, // No background for channels
        });

        let frame = egui::Frame::new()
            .fill(bg_color)
            .inner_margin(egui::Margin::same(3))
            .outer_margin(egui::Margin::ZERO);

        frame.show(ui, |ui| {
            ui.vertical(|ui| {
                ui.set_width(70.0);

                // Strip name (editable with right-click color picker)
                let name_response = ui.add(
                    egui::TextEdit::singleline(name)
                        .desired_width(80.0)
                        .font(egui::TextStyle::Body),
                );
                if name_response.changed() {
                    action = StripAction::NameChanged(name.clone());
                }

                // Right-click context menu for color picker
                name_response.context_menu(|ui| {
                    ui.label("Choose strip color:");
                    ui.separator();

                    let colors = [
                        ("Green", egui::Color32::from_rgb(0x00, 0x17, 0x07)), // #001707
                        ("Blue", egui::Color32::from_rgb(0x00, 0x05, 0x17)),  // #000517
                        ("Magenta", egui::Color32::from_rgb(0x17, 0x00, 0x10)), // #170010
                        ("Orange", egui::Color32::from_rgb(0x17, 0x12, 0x00)), // #171200
                    ];

                    for (label, color) in colors {
                        if ui
                            .add(
                                egui::Button::new(label)
                                    .fill(color)
                                    .min_size(egui::vec2(100.0, 25.0)),
                            )
                            .clicked()
                        {
                            action = StripAction::ColorChanged(color);
                            ui.close();
                        }
                    }

                    ui.separator();
                    if ui.button("Reset to default").clicked() {
                        action = StripAction::ColorChanged(egui::Color32::TRANSPARENT);
                        ui.close();
                    }
                });

                // Balance knob at top (only for channel strips), or blank space for alignment
                let knob_radius = 15.0;
                if matches!(strip.kind, usb::StripKind::Channel) {
                    ui.add_space(5.0);

                    ui.vertical_centered(|ui| {
                        let mut balance = strip.balance as f32;

                        // Draw a knob control
                        let (knob_rect, response) = ui.allocate_exact_size(
                            egui::vec2(knob_radius * 2.0, knob_radius * 2.0),
                            egui::Sense::click_and_drag(),
                        );

                        if response.secondary_clicked() {
                            // Right-click to start MIDI learn
                            action = StripAction::StartMidiLearnPan;
                        } else if response.double_clicked() {
                            balance = 0.0;
                            strip.balance = 0.0;
                            action = StripAction::FaderChanged(strip.fader, name.clone());
                        } else if response.dragged() {
                            let delta = response.drag_delta();
                            // Use both horizontal and vertical drag (right = positive, down = negative)
                            let combined_delta = delta.x - delta.y;
                            balance = (balance + combined_delta * 0.5).clamp(-100.0, 100.0);
                            strip.balance = balance as f64;
                            action = StripAction::FaderChanged(strip.fader, name.clone());
                        }

                        // Draw the knob
                        let painter = ui.painter();
                        let center = knob_rect.center();

                        // Outer circle
                        painter.circle_filled(center, knob_radius, egui::Color32::from_gray(60));
                        painter.circle_stroke(
                            center,
                            knob_radius,
                            egui::Stroke::new(2.0, egui::Color32::WHITE),
                        );

                        // Calculate angle from balance value (-100 to 100 -> -135° to 135°)
                        let angle = (balance / 100.0) * 2.356; // 135 degrees in radians
                        let indicator_length = knob_radius * 1.2;
                        let indicator_end = egui::pos2(
                            center.x + angle.sin() * indicator_length,
                            center.y - angle.cos() * indicator_length,
                        );

                        // Indicator line
                        painter.line_segment(
                            [center, indicator_end],
                            egui::Stroke::new(3.0, egui::Color32::WHITE),
                        );
                    });

                    ui.add_space(5.0);
                } else {
                    // Add blank space for bus/main strips to align with channel strips
                    // Match the exact structure: spacing + vertical_centered with knob size + spacing
                    ui.add_space(5.0);
                    ui.vertical_centered(|ui| {
                        ui.allocate_space(egui::vec2(knob_radius * 2.0, knob_radius * 2.0));
                    });
                    ui.add_space(5.0);
                }

                // Custom Fader
                let mut fader_value = strip.fader as f32;
                ui.add_space(10.0);

                // Calculate fader height based on available space minus fixed elements
                // Fixed elements: name label (~20), dB labels (~20), spacing (~20), pan knob (~60), buttons (~35)
                let fixed_height = 155.0;
                let fader_height = (available_height - fixed_height).max(200.0);
                let fader_width = 40.0;
                let track_width = 6.0;
                let cap_height = 20.0;
                let cap_width = 30.0;

                let (fader_rect, response) = ui.allocate_exact_size(
                    egui::vec2(fader_width, fader_height),
                    egui::Sense::click_and_drag(),
                );

                if response.secondary_clicked() {
                    // Right-click to start MIDI learn
                    action = StripAction::StartMidiLearnFader;
                } else if response.double_clicked() {
                    fader_value = 0.0;
                    strip.set_fader(0.0);
                    action = StripAction::FaderChanged(0.0, name.clone());
                } else if response.dragged() {
                    let delta_y = response.drag_delta().y;
                    // Convert pixel delta to dB range (-50 to +10)
                    // When Shift is pressed, increase sensitivity by 10x for fine control
                    let shift_pressed = ui.input(|i| i.modifiers.shift);
                    let sensitivity = if shift_pressed { 10.0 } else { 1.0 };
                    let db_per_pixel = (60.0 / fader_height) / sensitivity;
                    fader_value = (fader_value - delta_y * db_per_pixel).clamp(-50.0, 10.0);
                    strip.set_fader(fader_value as f64);
                    action = StripAction::FaderChanged(fader_value as f64, name.clone());
                }

                // Allocate meter rectangles and check for clicks (before getting painter)
                let meter_width = 12.0;
                let meter_spacing = 2.0;
                let meter_x_start = fader_rect.max.x + 25.0;

                let left_meter_rect = egui::Rect::from_min_size(
                    egui::pos2(meter_x_start, fader_rect.min.y),
                    egui::vec2(meter_width, fader_height),
                );
                let left_meter_response = ui.allocate_rect(left_meter_rect, egui::Sense::click());
                let mut meter_clicked = left_meter_response.clicked();
                let mut meter_double_clicked = left_meter_response.double_clicked();

                let right_meter_rect = if meter_value_right.is_some() {
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(
                            meter_x_start + meter_width + meter_spacing,
                            fader_rect.min.y,
                        ),
                        egui::vec2(meter_width, fader_height),
                    );
                    let response = ui.allocate_rect(rect, egui::Sense::click());
                    if response.clicked() {
                        meter_clicked = true;
                    }
                    if response.double_clicked() {
                        meter_double_clicked = true;
                    }
                    Some(rect)
                } else {
                    None
                };

                // Draw the fader
                let painter = ui.painter();

                // Calculate cap position (0dB is at 5/6 height, linear scale from -50 to +10)
                let normalized_pos = (fader_value + 50.0) / 60.0;
                let cap_y = fader_rect.max.y - (normalized_pos * fader_height);

                // Draw track background
                let track_rect = egui::Rect::from_center_size(
                    egui::pos2(fader_rect.center().x, fader_rect.center().y),
                    egui::vec2(track_width, fader_height),
                );
                painter.rect_filled(track_rect, 1.0, egui::Color32::from_gray(30));

                // Draw scale marks at 6dB intervals: +6, -6, -12, -18, -24, -30, -36, -42, -48
                let db_marks = [
                    9.0, 6.0, 3.0, -3.0, -6.0, -9.0, -12.0, -18.0, -24.0, -30.0, -36.0, -42.0,
                    -48.0,
                ];
                for &db_value in &db_marks {
                    // Calculate position for this dB value (linear scale)
                    let normalized_pos = (db_value + 50.0) / 60.0;
                    let mark_y = fader_rect.max.y - (normalized_pos * fader_height);

                    painter.line_segment(
                        [
                            egui::pos2(fader_rect.min.x, mark_y),
                            egui::pos2(fader_rect.max.x, mark_y),
                        ],
                        egui::Stroke::new(1.0, egui::Color32::from_gray(100)),
                    );

                    // Draw dB label to the right of the tick mark
                    let label_text = if db_value > 0.0 {
                        format!("+{:.0}", db_value)
                    } else {
                        format!("{:.0}", db_value)
                    };
                    painter.text(
                        egui::pos2(fader_rect.max.x + 18.0, mark_y),
                        egui::Align2::RIGHT_CENTER,
                        label_text,
                        egui::FontId::proportional(9.0),
                        egui::Color32::from_gray(150),
                    );
                }

                // Draw 0dB marker line in yellow
                let zero_db_normalized = (0.0 + 50.0) / 60.0;
                let zero_db_y = fader_rect.max.y - (zero_db_normalized * fader_height);
                painter.line_segment(
                    [
                        egui::pos2(fader_rect.min.x, zero_db_y),
                        egui::pos2(fader_rect.max.x, zero_db_y),
                    ],
                    egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 200, 0)),
                );

                // Draw 0dB label
                painter.text(
                    egui::pos2(fader_rect.max.x + 18.0, zero_db_y),
                    egui::Align2::RIGHT_CENTER,
                    "0",
                    egui::FontId::proportional(9.0),
                    egui::Color32::from_rgb(255, 200, 0),
                );

                // Check for clipping (meter value above -0.1 dB) and update peak holds
                let clip_threshold = -0.1;
                let now = Instant::now();
                let peak_hold_duration = Duration::from_millis(500);

                // Update left/mono peak hold
                let left_key = format!("{}_L", meter_id);
                let left_peak_entry = peak_holds.entry(left_key.clone()).or_insert((-50.0, now));
                // Reset if expired, otherwise update if new value is higher
                if now.duration_since(left_peak_entry.1) >= peak_hold_duration
                    || meter_value > left_peak_entry.0
                {
                    *left_peak_entry = (meter_value, now);
                }
                if meter_value > clip_threshold {
                    clip_indicators.insert(left_key.clone(), now);
                }

                // Update running average for left/mono channel
                let avg_duration = Duration::from_secs(3);
                let left_history = meter_averages.entry(left_key.clone()).or_default();
                left_history.push((meter_value, now));
                // Remove values older than 3 seconds
                left_history.retain(|(_, time)| now.duration_since(*time) < avg_duration);

                // Update right peak hold if stereo
                if let Some(right_val) = meter_value_right {
                    let right_key = format!("{}_R", meter_id);
                    let right_peak_entry =
                        peak_holds.entry(right_key.clone()).or_insert((-50.0, now));
                    // Reset if expired, otherwise update if new value is higher
                    if now.duration_since(right_peak_entry.1) >= peak_hold_duration
                        || right_val > right_peak_entry.0
                    {
                        *right_peak_entry = (right_val, now);
                    }
                    if right_val > clip_threshold {
                        clip_indicators.insert(right_key.clone(), now);
                    }

                    // Update running average for right channel
                    let right_history = meter_averages.entry(right_key.clone()).or_default();
                    right_history.push((right_val, now));
                    // Remove values older than 3 seconds
                    right_history.retain(|(_, time)| now.duration_since(*time) < avg_duration);
                }

                // Clear clip indicators if meter was clicked or double-clicked
                if meter_double_clicked {
                    // Double-click: clear ALL clip indicators
                    clip_indicators.clear();
                } else if meter_clicked {
                    // Single click: clear only this strip's clip indicators
                    clip_indicators.remove(&format!("{}_L", meter_id));
                    clip_indicators.remove(&format!("{}_R", meter_id));
                }

                // Helper function to draw a single meter
                let draw_single_meter =
                    |painter: &egui::Painter,
                     meter_rect: egui::Rect,
                     meter_val: f64,
                     channel_suffix: &str| {
                        // Meter background
                        painter.rect_filled(meter_rect, 0.0, egui::Color32::from_gray(20));

                        // Check if this meter is showing a clip indicator
                        let meter_key = format!("{}_{}", meter_id, channel_suffix);
                        let is_clipping = clip_indicators.contains_key(&meter_key);

                        // Draw meter with colored segments
                        let color_zones = [
                            (10.0, egui::Color32::RED),
                            (-3.0, egui::Color32::from_rgb(255, 165, 0)),
                            (-6.0, egui::Color32::YELLOW),
                            (-9.0, egui::Color32::GREEN),
                            (-18.0, egui::Color32::from_rgb(0, 185, 0)),
                        ];

                        // Draw each segment up to the meter value
                        for i in 0..color_zones.len() {
                            let (max_db, color) = color_zones[i];
                            let min_db = if i < color_zones.len() - 1 {
                                color_zones[i + 1].0
                            } else {
                                -50.0
                            };

                            if meter_val >= min_db {
                                let segment_max = max_db.min(meter_val);
                                let segment_min = min_db.max(-50.0);

                                if segment_max > segment_min {
                                    let top_normalized = (segment_max + 50.0) / 60.0;
                                    let bottom_normalized = (segment_min + 50.0) / 60.0;

                                    let top_y = meter_rect.max.y
                                        - (top_normalized * fader_height as f64) as f32;
                                    let bottom_y = meter_rect.max.y
                                        - (bottom_normalized * fader_height as f64) as f32;
                                    let segment_height = bottom_y - top_y;

                                    if segment_height > 0.0 {
                                        let segment_rect = egui::Rect::from_min_size(
                                            egui::pos2(meter_rect.min.x, top_y),
                                            egui::vec2(meter_width, segment_height),
                                        );
                                        painter.rect_filled(segment_rect, 0.0, color);
                                    }
                                }
                            }
                        }

                        // Draw clip indicator at the top if clipping
                        if is_clipping {
                            let clip_indicator_height = 8.0;
                            let clip_rect = egui::Rect::from_min_size(
                                egui::pos2(meter_rect.min.x, meter_rect.min.y),
                                egui::vec2(meter_width, clip_indicator_height),
                            );
                            painter.rect_filled(clip_rect, 0.0, egui::Color32::from_rgb(255, 0, 0));
                        }

                        // Draw peak hold line
                        let peak_key = format!("{}_{}", meter_id, channel_suffix);
                        if let Some(&(peak_val, _)) = peak_holds
                            .get(&peak_key)
                            .filter(|(val, _)| *val > -50.0 && *val < 10.0)
                        {
                            let peak_normalized = (peak_val + 50.0) / 60.0;
                            let peak_y =
                                meter_rect.max.y - (peak_normalized * fader_height as f64) as f32;
                            painter.line_segment(
                                [
                                    egui::pos2(meter_rect.min.x, peak_y),
                                    egui::pos2(meter_rect.max.x, peak_y),
                                ],
                                egui::Stroke::new(2.0, egui::Color32::WHITE),
                            );
                        }

                        // Draw running average line
                        let avg_key = format!("{}_{}", meter_id, channel_suffix);
                        let history = meter_averages.get(&avg_key);
                        if let Some(history) = history.filter(|h| !h.is_empty()) {
                            let avg_val = history.iter().map(|(val, _)| val).sum::<f64>()
                                / history.len() as f64;
                            if avg_val > -50.0 && avg_val < 10.0 {
                                let avg_normalized = (avg_val + 50.0) / 60.0;
                                let avg_y = meter_rect.max.y
                                    - (avg_normalized * fader_height as f64) as f32;
                                painter.line_segment(
                                    [
                                        egui::pos2(meter_rect.min.x, avg_y),
                                        egui::pos2(meter_rect.max.x, avg_y),
                                    ],
                                    egui::Stroke::new(4.0, egui::Color32::from_rgb(79, 0, 255)),
                                );
                            }
                        }
                    };

                // Draw left meter (or mono meter)
                draw_single_meter(painter, left_meter_rect, meter_value, "L");

                // Draw right meter if stereo
                if let Some(right_val) = meter_value_right {
                    if let Some(right_rect) = right_meter_rect {
                        draw_single_meter(painter, right_rect, right_val, "R");
                    }
                }

                // Draw fader cap
                let cap_rect = egui::Rect::from_center_size(
                    egui::pos2(fader_rect.center().x, cap_y),
                    egui::vec2(cap_width, cap_height),
                );

                // Cap shadow
                painter.rect_filled(
                    cap_rect.translate(egui::vec2(1.0, 2.0)),
                    3.0,
                    egui::Color32::from_black_alpha(100),
                );

                // Cap gradient effect
                painter.rect_filled(cap_rect, 3.0, egui::Color32::from_gray(180));
                painter.rect_stroke(
                    cap_rect,
                    3.0,
                    egui::Stroke::new(1.0, egui::Color32::from_gray(220)),
                    egui::StrokeKind::Middle,
                );

                // Cap highlight
                let highlight_rect = egui::Rect::from_min_size(
                    cap_rect.min,
                    egui::vec2(cap_width, cap_height / 3.0),
                );
                painter.rect_filled(highlight_rect, 3.0, egui::Color32::from_gray(220));

                // Center grip line
                painter.line_segment(
                    [
                        egui::pos2(cap_rect.center().x - 8.0, cap_y),
                        egui::pos2(cap_rect.center().x + 8.0, cap_y),
                    ],
                    egui::Stroke::new(2.0, egui::Color32::from_gray(100)),
                );

                ui.add_space(10.0);

                // Mute and Solo buttons side by side
                ui.vertical_centered(|ui| {
                    ui.horizontal(|ui| {
                        // Mute button
                        let muted = strip.mute;
                        let muted_by_solo = strip.mute_by_solo;

                        // Determine button color: if muted by solo, show red text on dark gray background
                        // If manually muted, show black text on red background
                        let (button_fill, text_color) = if muted {
                            (egui::Color32::RED, egui::Color32::BLACK)
                        } else if muted_by_solo {
                            (egui::Color32::from_rgb(40, 0, 0), egui::Color32::RED)
                        } else {
                            (egui::Color32::from_rgb(40, 0, 0), egui::Color32::LIGHT_GRAY)
                        };

                        let mute_response = ui.add(
                            egui::Button::new(egui::RichText::new("M").color(text_color))
                                .min_size(egui::vec2(25.0, 25.0))
                                .fill(button_fill)
                                .small(),
                        );

                        if mute_response.secondary_clicked() {
                            // Right-click to start MIDI learn
                            action = StripAction::StartMidiLearnMute;
                        } else if mute_response.clicked() {
                            strip.mute = !muted;
                            action = StripAction::FaderChanged(strip.fader, name.clone());
                        }

                        // Solo button (only for channel strips)
                        if matches!(strip.kind, usb::StripKind::Channel) {
                            let soloed = strip.solo;
                            let text_color = if soloed {
                                egui::Color32::BLACK
                            } else {
                                egui::Color32::LIGHT_GRAY
                            };
                            let solo_response = ui.add(
                                egui::Button::new(egui::RichText::new("S").color(text_color))
                                    .min_size(egui::vec2(25.0, 25.0))
                                    .fill(if soloed {
                                        egui::Color32::YELLOW
                                    } else {
                                        egui::Color32::from_rgb(40, 40, 0)
                                    }),
                            );

                            if solo_response.secondary_clicked() {
                                // Right-click to start MIDI learn
                                action = StripAction::StartMidiLearnSolo;
                            } else if solo_response.clicked() {
                                action = StripAction::SoloToggled;
                            }
                        }
                    });
                });

                // Add padding at bottom to prevent scrollbar from obscuring buttons
                ui.add_space(20.0);
            });
        });

        action
    }
}

impl eframe::App for BatonApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Check for MIDI learn timeout (5 seconds)
        if self.midi_learn_state != midi_control::MidiLearnState::Inactive {
            if let Some(start_time) = self.midi_learn_start_time {
                if start_time.elapsed() >= Duration::from_secs(5) {
                    self.midi_learn_state = midi_control::MidiLearnState::Inactive;
                    self.midi_learn_start_time = None;
                    self.status_message = "MIDI Learn: Timed out after 5 seconds".to_string();
                }
            }
        }

        // Poll device state periodically
        if self.last_tick.elapsed() >= self.tick_rate {
            let mut ps = self.ps.lock().unwrap();
            ps.poll_state();
            drop(ps);
            self.process_midi_messages();
            self.last_tick = Instant::now();
        }

        // Request repaint to keep UI responsive
        ctx.request_repaint_after(self.tick_rate);

        // Top panel with controls
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Baton Mixer");

                ui.separator();

                // Mix selector
                ui.label("Mix:");
                let ps = self.ps.lock().unwrap();
                let mix_names: Vec<String> = ps.mixes.iter().map(|m| m.name.clone()).collect();
                drop(ps);

                egui::ComboBox::from_id_salt("mix_selector")
                    .selected_text(&mix_names[self.active_mix_index])
                    .show_ui(ui, |ui| {
                        for (i, name) in mix_names.iter().enumerate() {
                            ui.selectable_value(&mut self.active_mix_index, i, name);
                        }
                    });

                ui.separator();

                // Global controls
                let mut ps = self.ps.lock().unwrap();

                let phantom_power = ps.phantom_power;
                if ui
                    .add(egui::Button::new("48V").fill(if phantom_power {
                        egui::Color32::BLUE
                    } else {
                        egui::Color32::DARK_GRAY
                    }))
                    .clicked()
                {
                    ps.set_phantom_power(!phantom_power);
                }

                let in_1_2_line = ps.in_1_2_line;
                if ui
                    .add(egui::Button::new("1-2 Line").fill(if in_1_2_line {
                        egui::Color32::BLUE
                    } else {
                        egui::Color32::DARK_GRAY
                    }))
                    .clicked()
                {
                    ps.set_1_2_line(!in_1_2_line);
                }

                let main_mute = ps.main_mute;
                if ui
                    .add(egui::Button::new("Mute").fill(if main_mute {
                        egui::Color32::RED
                    } else {
                        egui::Color32::DARK_GRAY
                    }))
                    .clicked()
                {
                    ps.set_main_mute(!main_mute);
                }

                let main_mono = ps.main_mono;
                if ui
                    .add(egui::Button::new("Mono").fill(if main_mono {
                        egui::Color32::YELLOW
                    } else {
                        egui::Color32::DARK_GRAY
                    }))
                    .clicked()
                {
                    ps.set_main_mono(!main_mono);
                }

                if ui
                    .add(egui::Button::new("Bypass").fill(if self.bypass {
                        egui::Color32::LIGHT_BLUE
                    } else {
                        egui::Color32::DARK_GRAY
                    }))
                    .clicked()
                {
                    self.bypass = !self.bypass;
                    if self.bypass {
                        ps.bypass_mixer();
                    } else {
                        ps.write_state();
                    }
                }

                ui.separator();

                // Reset solo button
                let solo_exists = ps.mixes[self.active_mix_index].has_solo();
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new("Reset solo").color(
                            if solo_exists {
                                egui::Color32::BLACK
                            } else {
                                egui::Color32::LIGHT_GRAY
                            },
                        ))
                        .fill(if solo_exists {
                            egui::Color32::YELLOW
                        } else {
                            egui::Color32::DARK_GRAY
                        }),
                    )
                    .clicked()
                {
                    ps.mixes[self.active_mix_index].reset_solo();
                    ps.write_state();
                }

                // Reset mute button
                let mute_exists = ps.mixes[self.active_mix_index].has_mute();
                if ui
                    .add(
                        egui::Button::new(egui::RichText::new("Reset mute").color(
                            if mute_exists {
                                egui::Color32::BLACK
                            } else {
                                egui::Color32::LIGHT_GRAY
                            },
                        ))
                        .fill(if mute_exists {
                            egui::Color32::RED
                        } else {
                            egui::Color32::DARK_GRAY
                        }),
                    )
                    .clicked()
                {
                    ps.mixes[self.active_mix_index].reset_mute();
                    ps.write_state();
                }
            });
        });

        // Status bar
        egui::TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status_message);
            });
        });

        let mut strip_actions = Vec::new();

        egui::SidePanel::right("right_panel")
            .resizable(false)
            .show(ctx, |ui| {
                // Align with channel strips
                ui.add_space(6.0);
                let available_height = ui.available_height() - 6.0;

                let ps = self.ps.lock().unwrap();
                let bus_name = ps.mixes[self.active_mix_index].name.clone();
                // Get both left and right bus meters
                let bus_meter_left = ps.bus_meters[self.active_mix_index * 2].value;
                let bus_meter_right = ps.bus_meters[self.active_mix_index * 2 + 1].value;
                drop(ps);

                let mut ps = self.ps.lock().unwrap();
                let mix = &mut ps.mixes[self.active_mix_index];

                // Draw bus strip (stereo - with left and right meters)
                let bus_strip = &mut mix.strips.bus_strip;
                let mut bus_name_mut = bus_name.clone();
                let meter_id = format!("bus_{}", self.active_mix_index);
                let bus_strip_index = mix.strips.channel_strips.len();
                let strip_id = format!("{}:{}", self.active_mix_index, bus_strip_index);
                let custom_color = self.strip_colors.get(&strip_id).copied();
                let bus_action = Self::draw_strip(
                    ui,
                    bus_strip,
                    &mut bus_name_mut,
                    bus_meter_left,
                    Some(bus_meter_right),
                    available_height,
                    &mut self.clip_indicators,
                    &mut self.peak_holds,
                    &mut self.meter_averages,
                    &meter_id,
                    custom_color,
                );
                strip_actions.push((bus_strip_index, bus_action));

                drop(ps);
            });

        // Central panel with strips
        egui::CentralPanel::default().show(ctx, |ui| {
            // Get available height for strips
            let available_height = ui.available_height();

            egui::ScrollArea::horizontal().show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Collect strip data
                    let ps = self.ps.lock().unwrap();
                    let strip_data: Vec<(String, f64)> = ps
                        .channel_names
                        .iter()
                        .zip(ps.channel_meters.iter())
                        .map(|(name, meter)| (name.clone(), meter.value))
                        .collect();
                    drop(ps);

                    let mut ps = self.ps.lock().unwrap();
                    let mix = &mut ps.mixes[self.active_mix_index];

                    // Draw channel strips (mono - no right meter)
                    for (i, strip) in mix.strips.channel_strips.iter_mut().enumerate() {
                        let (mut name, meter_value) = strip_data[i].clone();
                        let meter_id = format!("ch_{}", i);
                        let strip_id = format!("{}:{}", self.active_mix_index, i);
                        let custom_color = self.strip_colors.get(&strip_id).copied();
                        let action = Self::draw_strip(
                            ui,
                            strip,
                            &mut name,
                            meter_value,
                            None,
                            available_height,
                            &mut self.clip_indicators,
                            &mut self.peak_holds,
                            &mut self.meter_averages,
                            &meter_id,
                            custom_color,
                        );
                        strip_actions.push((i, action));
                        ui.add(egui::Separator::default().spacing(2.0));
                    }

                    drop(ps);
                });
            });
        });

        // Process actions
        let mut ps = self.ps.lock().unwrap();
        for (strip_index, action) in strip_actions {
            match action {
                StripAction::FaderChanged(fader_value, strip_name) => {
                    ps.write_channel_fader(self.active_mix_index, strip_index);
                    self.status_message = format!("{}: {:.1} dB", strip_name, fader_value);
                }
                StripAction::SoloToggled => {
                    ps.mixes[self.active_mix_index].toggle_solo(strip_index);
                    ps.write_state();
                }
                StripAction::StartMidiLearnFader => {
                    let target = midi_control::ControlTarget::Strip(midi_control::StripTarget {
                        mix_index: self.active_mix_index,
                        strip_index,
                        control: midi_control::StripControl::Fader,
                    });
                    self.midi_learn_state = self.midi_mapping.start_learning(target);
                    self.midi_learn_start_time = Some(Instant::now());
                    self.status_message = format!(
                        "Learning MIDI for strip {} fader - move a MIDI control...",
                        strip_index + 1
                    );
                }
                StripAction::StartMidiLearnPan => {
                    let target = midi_control::ControlTarget::Strip(midi_control::StripTarget {
                        mix_index: self.active_mix_index,
                        strip_index,
                        control: midi_control::StripControl::Balance,
                    });
                    self.midi_learn_state = self.midi_mapping.start_learning(target);
                    self.midi_learn_start_time = Some(Instant::now());
                    self.status_message = format!(
                        "Learning MIDI for strip {} pan - move a MIDI control...",
                        strip_index + 1
                    );
                }
                StripAction::StartMidiLearnMute => {
                    let target = midi_control::ControlTarget::Strip(midi_control::StripTarget {
                        mix_index: self.active_mix_index,
                        strip_index,
                        control: midi_control::StripControl::Mute,
                    });
                    self.midi_learn_state = self.midi_mapping.start_learning(target);
                    self.midi_learn_start_time = Some(Instant::now());
                    self.status_message = format!(
                        "Learning MIDI for strip {} mute - move a MIDI control...",
                        strip_index + 1
                    );
                }
                StripAction::StartMidiLearnSolo => {
                    let target = midi_control::ControlTarget::Strip(midi_control::StripTarget {
                        mix_index: self.active_mix_index,
                        strip_index,
                        control: midi_control::StripControl::Solo,
                    });
                    self.midi_learn_state = self.midi_mapping.start_learning(target);
                    self.midi_learn_start_time = Some(Instant::now());
                    self.status_message = format!(
                        "Learning MIDI for strip {} solo - move a MIDI control...",
                        strip_index + 1
                    );
                }
                StripAction::NameChanged(new_name) => {
                    // Update channel name or mix name
                    if strip_index < ps.channel_names.len() {
                        ps.channel_names[strip_index] = new_name;
                    } else {
                        ps.mixes[self.active_mix_index].name = new_name;
                    }
                }
                StripAction::ColorChanged(color) => {
                    let strip_id = format!("{}:{}", self.active_mix_index, strip_index);
                    if color == egui::Color32::TRANSPARENT {
                        // Reset to default - remove custom color
                        self.strip_colors.remove(&strip_id);
                    } else {
                        // Set custom color
                        self.strip_colors.insert(strip_id, color);
                    }
                }
                StripAction::None => {}
            }
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        log::info!("Saving configuration...");

        // Save config
        match self.config_dir {
            Some(ref dir) => {
                let config_file = dir.join("config.json");
                {
                    let ps = self.ps.lock().unwrap();
                    if let Ok(serialized) = serde_json::to_string_pretty(&*ps) {
                        if let Ok(mut file) = File::create(&config_file) {
                            let _ = file.write_all(serialized.as_bytes());
                            let _ = file.flush();
                        }
                    }
                }
            }
            None => (),
        };

        // Save MIDI mapping
        self.save_midi_mapping();
    }
}
