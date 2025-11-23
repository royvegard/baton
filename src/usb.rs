use baton_studio::*;
use core::time::Duration;
use nusb::{Device, MaybeFuture};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::num::NonZero;

#[derive(Clone)]
pub struct Meter {
    pub value: f64,
    pub max: f64,
    pub clip: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(default)]
pub struct PreSonusStudio1824c {
    #[serde(skip)]
    device: Device,
    #[serde(skip)]
    pub command: Command,
    #[serde(skip)]
    pub state: State,
    #[serde(skip)]
    pub channel_meters: Vec<Meter>,
    #[serde(skip)]
    pub bus_meters: Vec<Meter>,
    pub channel_names: Vec<String>,
    pub mixes: Vec<Mix>,
    #[serde(skip)]
    pub in_1_2_line: bool,
    #[serde(skip)]
    pub main_mute: bool,
    #[serde(skip)]
    pub main_mono: bool,
    #[serde(skip)]
    pub phantom_power: bool,
    #[serde(skip)]
    descriptor: Vec<String>,
}

impl Default for PreSonusStudio1824c {
    fn default() -> Self {
        PreSonusStudio1824c::new().unwrap()
    }
}

impl PreSonusStudio1824c {
    pub fn new() -> Result<Self, Box<dyn Error>> {
        let device_info = nusb::list_devices()
            .wait()?
            .find(|dev| dev.vendor_id() == 0x194f && dev.product_id() == 0x010d)
            .ok_or(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "device not found",
            ))?;

        log::info!(
            "Found Manufacturer: {}, Product: {}, Serial: {}",
            device_info.manufacturer_string().unwrap_or("unknown"),
            device_info.product_string().unwrap_or("unknown"),
            device_info.serial_number().unwrap_or("unknown"),
        );

        let interfaces = device_info.interfaces();
        for i in interfaces {
            log::debug!(
                "Interface: {} {}",
                i.interface_number(),
                i.interface_string().unwrap_or_default()
            );
        }

        let device = device_info.open().wait()?;
        log::info!("Opened device");

        let number_of_channels = 18;

        // # Read all string descriptors from device
        // Channel name descriptors start at this index
        let input_channel_name_index = 33;
        let mut channel_name: Vec<String> = vec![];
        let mut desc: Vec<String> = vec![];
        // Descriptor at index 0 is reserved for Language Table, we skip it.
        desc.push(String::from("LT"));

        let timeout = Duration::from_millis(100);
        let mut i = 1;
        while let Ok(d) = device
            .get_string_descriptor(NonZero::new(i).unwrap(), 0, timeout)
            .wait()
        {
            log::debug!("Descriptor {}: {}", i, d);
            desc.push(d);
            i += 1;
        }

        for i in 0..number_of_channels {
            let name = desc[input_channel_name_index + i].clone();
            channel_name.push(name);
        }

        for i in 1..=18 {
            channel_name.push(format!("DAW {}", i));
        }

        Ok(PreSonusStudio1824c {
            device,
            command: Command::new(),
            state: State::new(),
            mixes: vec![
                Mix::new(
                    String::from("MAIN 1-2"),
                    StripKind::Main,
                    0,
                    channel_name.len(),
                ),
                Mix::new(
                    String::from("MIX 3-4"),
                    StripKind::Bus,
                    1,
                    channel_name.len(),
                ),
                Mix::new(
                    String::from("MIX 5-6"),
                    StripKind::Bus,
                    2,
                    channel_name.len(),
                ),
                Mix::new(
                    String::from("MIX 7-8"),
                    StripKind::Bus,
                    3,
                    channel_name.len(),
                ),
                Mix::new(
                    String::from("S/PDIF"),
                    StripKind::Bus,
                    4,
                    channel_name.len(),
                ),
                Mix::new(
                    String::from("ADAT 1-2"),
                    StripKind::Bus,
                    5,
                    channel_name.len(),
                ),
                Mix::new(
                    String::from("ADAT 3-4"),
                    StripKind::Bus,
                    6,
                    channel_name.len(),
                ),
                Mix::new(
                    String::from("ADAT 5-6"),
                    StripKind::Bus,
                    7,
                    channel_name.len(),
                ),
                Mix::new(
                    String::from("ADAT 7-8"),
                    StripKind::Bus,
                    8,
                    channel_name.len(),
                ),
            ],
            channel_names: channel_name,
            channel_meters: vec![
                Meter {
                    value: -96.0,
                    max: -96.0,
                    clip: false
                };
                36
            ],
            bus_meters: vec![
                Meter {
                    value: -96.0,
                    max: -96.0,
                    clip: false
                };
                18
            ],
            in_1_2_line: false,
            main_mute: false,
            main_mono: false,
            phantom_power: false,
            descriptor: desc,
        })
    }

    pub fn set_1_2_line(&mut self, on: bool) {
        match self.command.set_button(Button::Line, on).send(&self.device) {
            Ok(_) => log::debug!("Set 1/2 line to {}", on),
            Err(e) => log::error!("Error setting 1/2 line: {}", e),
        }
    }

    pub fn set_main_mute(&mut self, on: bool) {
        match self.command.set_button(Button::Mute, on).send(&self.device) {
            Ok(_) => log::debug!("Set main mute to {}", on),
            Err(e) => log::error!("Error setting main mute: {}", e),
        }
    }

    pub fn set_main_mono(&mut self, on: bool) {
        match self.command.set_button(Button::Mono, on).send(&self.device) {
            Ok(_) => log::debug!("Set main mono to {}", on),
            Err(e) => log::error!("Error setting main mono: {}", e),
        }
    }

    pub fn set_phantom_power(&mut self, on: bool) {
        match self
            .command
            .set_button(Button::Phantom, on)
            .send(&self.device)
        {
            Ok(_) => log::debug!("Set phantom power to {}", on),
            Err(e) => log::error!("Error setting phantom power: {}", e),
        }
    }

    pub fn poll_state(&mut self) {
        match self.state.poll(&self.device) {
            Ok(_) => {
                // synch meters
                let mut channel_index = 0;
                for v in self.state.mic.iter().map(|g| gain_to_db(*g)) {
                    self.channel_meters[channel_index].value = v;
                    if v > self.channel_meters[channel_index].max {
                        self.channel_meters[channel_index].max = v;
                    }
                    if self.channel_meters[channel_index].value > -0.001 {
                        self.channel_meters[channel_index].clip = true;
                    }
                    channel_index += 1;
                }
                for v in self.state.spdif.iter().map(|g| gain_to_db(*g)) {
                    self.channel_meters[channel_index].value = v;
                    if v > self.channel_meters[channel_index].max {
                        self.channel_meters[channel_index].max = v;
                    }
                    if self.channel_meters[channel_index].value > -0.001 {
                        self.channel_meters[channel_index].clip = true;
                    }
                    channel_index += 1;
                }
                for v in self.state.adat.iter().map(|g| gain_to_db(*g)) {
                    self.channel_meters[channel_index].value = v;
                    if v > self.channel_meters[channel_index].max {
                        self.channel_meters[channel_index].max = v;
                    }
                    if self.channel_meters[channel_index].value > -0.001 {
                        self.channel_meters[channel_index].clip = true;
                    }
                    channel_index += 1;
                }
                for v in self.state.daw.iter().map(|g| gain_to_db(*g)) {
                    self.channel_meters[channel_index].value = v;
                    if v > self.channel_meters[channel_index].max {
                        self.channel_meters[channel_index].max = v;
                    }
                    if self.channel_meters[channel_index].value > -0.001 {
                        self.channel_meters[channel_index].clip = true;
                    }
                    channel_index += 1;
                }

                let mut bus_index = 0;
                for v in self.state.bus.iter().map(|g| gain_to_db(*g)) {
                    self.bus_meters[bus_index].value = v;
                    if v > self.bus_meters[bus_index].max {
                        self.bus_meters[bus_index].max = v;
                    }
                    if self.bus_meters[bus_index].value > -0.001 {
                        self.bus_meters[bus_index].clip = true;
                    }
                    bus_index += 1;
                }

                // synch button states
                self.phantom_power = self.state.phantom == 0x01;
                self.in_1_2_line = self.state.line == 0x01;
                self.main_mute = self.state.mute == 0x01;
                self.main_mono = self.state.mono == 0x01;
            }
            Err(e) => log::error!("Error polling state: {}", e),
        }
    }

    pub fn load_config(&mut self, config: &str) {
        let ps_state = serde_json::from_str::<PreSonusStudio1824c>(config).unwrap();
        self.channel_names = ps_state.channel_names;

        let mix_state = ps_state.mixes;
        for i in 0..self.mixes.len() {
            for j in 0..self.mixes[i].strips.channel_strips.len() {
                self.mixes[i].strips.channel_strips[j].fader =
                    mix_state[i].strips.channel_strips[j].fader;
                self.mixes[i].strips.channel_strips[j].balance =
                    mix_state[i].strips.channel_strips[j].balance;
                self.mixes[i].strips.channel_strips[j].solo =
                    mix_state[i].strips.channel_strips[j].solo;
                self.mixes[i].strips.channel_strips[j].mute =
                    mix_state[i].strips.channel_strips[j].mute;
                self.mixes[i].strips.channel_strips[j].mute_by_solo =
                    mix_state[i].strips.channel_strips[j].mute_by_solo;
            }

            self.mixes[i].name = mix_state[i].name.clone();
            self.mixes[i].strips.bus_strip.fader = mix_state[i].strips.bus_strip.fader;
            self.mixes[i].strips.bus_strip.mute = mix_state[i].strips.bus_strip.mute;
        }
    }

    pub fn write_state(&mut self) {
        for i in 0..self.mixes.len() {
            let mut bus_index = 0;
            for j in 0..self.mixes[i].strips.channel_strips.len() {
                self.write_channel_fader(i, j);
                bus_index = j;
            }
            self.write_channel_fader(i, bus_index + 1);
        }
    }

    pub fn write_channel_fader(&mut self, mix_index: usize, channel_index: usize) {
        let strip = self.mixes[mix_index]
            .strips
            .iter()
            .nth(channel_index)
            .unwrap();
        let muted = strip.mute | strip.mute_by_solo;
        let soloed = strip.solo;

        let fader = strip.fader;
        let (left, right) = strip.pan_rule(PanLaw::Exponential);
        match strip.kind {
            StripKind::Main | StripKind::Bus => {
                let mut value = Value::DB(fader);
                if muted {
                    value = Value::Muted;
                }
                match self
                    .command
                    .set_output_fader(self.mixes[mix_index].strips.bus_strip.number, value)
                    .send(&self.device)
                {
                    Ok(_) => {
                        log::debug!(
                            "Set output fader mix {} to {} dB",
                            self.mixes[mix_index].strips.bus_strip.number,
                            fader
                        );
                    }
                    Err(e) => log::error!("Error setting output fader: {}", e),
                }
            }
            StripKind::Channel => {
                let mut value = Value::DB(left);
                if muted & !soloed {
                    value = Value::Muted;
                }
                match self
                    .command
                    .set_input_fader(
                        channel_index as u32,
                        self.mixes[mix_index].strips.bus_strip.number,
                        Channel::Left,
                        value,
                    )
                    .send(&self.device)
                {
                    Ok(_) => {
                        log::debug!(
                            "Set input fader channel {} mix {} left to {} dB",
                            channel_index,
                            self.mixes[mix_index].strips.bus_strip.number,
                            left
                        );
                    }
                    Err(e) => log::error!("Error setting input fader: {}", e),
                }

                value = Value::DB(right);
                if muted & !soloed {
                    value = Value::Muted;
                }
                match self
                    .command
                    .set_input_fader(
                        channel_index as u32,
                        self.mixes[mix_index].strips.bus_strip.number,
                        Channel::Right,
                        value,
                    )
                    .send(&self.device)
                {
                    Ok(_) => {
                        log::debug!(
                            "Set input fader channel {} mix {} right to {} dB",
                            channel_index,
                            self.mixes[mix_index].strips.bus_strip.number,
                            right
                        );
                    }
                    Err(e) => log::error!("Error setting input fader: {}", e),
                }
            }
        }
    }

    pub fn bypass_mixer(&mut self) {
        log::debug!("Bypassing mixer...");

        // Set all stereo bus faders to unity gain
        for m in 0..9 {
            match self
                .command
                .set_output_fader(m, Value::Unity)
                .send(&self.device)
            {
                Ok(_) => {
                    log::debug!("Set output fader mix {} to unity", m);
                }
                Err(e) => log::error!("Error setting output fader: {}", e),
            }
        }

        // Set:
        // Daw 1 -> Line out 1, Daw 2 -> Line out 2
        // Daw 3 -> Line out 3, Daw 4 -> Line out 4
        // Daw 5 -> Line out 5, Daw 6 -> Line out 6
        // Daw 7 -> Line out 7, Daw 8 -> Line out 8
        // Daw 9 -> SPDIF out 1, Daw 10 -> SPDIF out 2
        // Daw 11 -> ADAT out 1, Daw 12 -> ADAT out 2
        // Daw 13 -> ADAT out 3, Daw 14 -> ADAT out 4
        // Daw 15 -> ADAT out 5, Daw 16 -> ADAT out 6
        // Daw 17 -> ADAT out 7, Daw 18 -> ADAT out 8
        // Everything else muted

        let mut daw_channel_left = 16;
        let mut daw_channel_right;

        for m in 0..9 {
            daw_channel_left += 2;
            daw_channel_right = daw_channel_left + 1;
            for c in 0..35 {
                if c == daw_channel_left {
                    match self
                        .command
                        .set_input_fader(c, m, Channel::Left, Value::Unity)
                        .send(&self.device)
                    {
                        Ok(_) => {
                            log::debug!("Set input fader channel {} mix {} left to unity", c, m);
                        }
                        Err(e) => log::error!("Error setting input fader: {}", e),
                    }
                    match self
                        .command
                        .set_input_fader(c, m, Channel::Right, Value::Muted)
                        .send(&self.device)
                    {
                        Ok(_) => {
                            log::debug!("Set input fader channel {} mix {} right to muted", c, m);
                        }
                        Err(e) => log::error!("Error setting input fader: {}", e),
                    }
                } else if c == daw_channel_right {
                    match self
                        .command
                        .set_input_fader(c, m, Channel::Left, Value::Muted)
                        .send(&self.device)
                    {
                        Ok(_) => {
                            log::debug!("Set input fader channel {} mix {} left to muted", c, m);
                        }
                        Err(e) => log::error!("Error setting input fader: {}", e),
                    }
                    match self
                        .command
                        .set_input_fader(c, m, Channel::Right, Value::Unity)
                        .send(&self.device)
                    {
                        Ok(_) => {
                            log::debug!("Set input fader channel {} mix {} right to unity", c, m);
                        }
                        Err(e) => log::error!("Error setting input fader: {}", e),
                    }
                } else {
                    match self
                        .command
                        .set_input_fader(c, m, Channel::Left, Value::Muted)
                        .send(&self.device)
                    {
                        Ok(_) => {
                            log::debug!("Set input fader channel {} mix {} left to muted", c, m);
                        }
                        Err(e) => log::error!("Error setting input fader: {}", e),
                    }
                    match self
                        .command
                        .set_input_fader(c, m, Channel::Right, Value::Muted)
                        .send(&self.device)
                    {
                        Ok(_) => {
                            log::debug!("Set input fader channel {} mix {} right to muted", c, m);
                        }
                        Err(e) => log::error!("Error setting input fader: {}", e),
                    }
                }
            }
        }
    }
}

#[derive(Default, PartialEq)]
pub enum StripKind {
    #[default]
    Channel,
    Bus,
    Main,
}

pub enum PanLaw {
    Simple,
    Exponential,
}

#[derive(Default, Deserialize, Serialize)]
pub struct Strip {
    /// Volume fader in dB.
    pub fader: f64,
    /// Left/right balance.
    /// 0 is center. -100 is left, 100 is right.
    pub balance: f64,
    pub solo: bool,
    pub mute: bool,
    pub mute_by_solo: bool,
    #[serde(skip)]
    pub max: f64,
    #[serde(skip)]
    pub min: f64,
    #[serde(skip)]
    pub active: bool,
    #[serde(skip)]
    pub kind: StripKind,
    #[serde(skip)]
    pub number: u32,
}

impl Strip {
    pub fn set_fader(&mut self, value: f64) {
        self.fader = value.clamp(self.min, self.max);
    }

    pub fn pan_rule(&self, rule: PanLaw) -> (f64, f64) {
        let mut left = self.fader;
        let mut right = self.fader;

        match rule {
            PanLaw::Simple => {
                if self.balance < 0.0 {
                    right -= self.balance.abs();
                } else if self.balance > 0.0 {
                    left -= self.balance.abs();
                }
            }
            PanLaw::Exponential => {
                let value = self.fader - (self.balance.abs().powi(2) / 96.0);

                if self.balance < 0.0 {
                    right = value;
                } else if self.balance > 0.0 {
                    left = value;
                }
            }
        }

        (left, right)
    }
}

#[derive(Deserialize, Serialize)]
pub struct MixStrips {
    pub channel_strips: Vec<Strip>,
    pub bus_strip: Strip,
}

pub struct MixStripsIterator<'a> {
    channel_strips: &'a [Strip],
    bus_strip: &'a Strip,
    index: usize,
}

pub struct MixStripsMutIterator<'a> {
    channel_strips: &'a mut [Strip],
    bus_strip: Option<&'a mut Strip>,
    index: usize,
}

impl<'a> Iterator for MixStripsIterator<'a> {
    type Item = &'a Strip;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.channel_strips.len() {
            let result = &self.channel_strips[self.index];
            self.index += 1;
            Some(result)
        } else if self.index == self.channel_strips.len() {
            self.index += 1;
            Some(self.bus_strip)
        } else {
            None
        }
    }
}

impl<'a> Iterator for MixStripsMutIterator<'a> {
    type Item = &'a mut Strip;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.channel_strips.len() {
            let result = &mut self.channel_strips[self.index] as *mut Strip;
            self.index += 1;
            // SAFETY: We ensure each element is only returned once via index tracking
            unsafe { Some(&mut *result) }
        } else if self.index == self.channel_strips.len() {
            self.index += 1;
            self.bus_strip.take()
        } else {
            None
        }
    }
}

impl MixStrips {
    pub fn iter(&self) -> MixStripsIterator<'_> {
        MixStripsIterator {
            channel_strips: &self.channel_strips,
            bus_strip: &self.bus_strip,
            index: 0,
        }
    }

    pub fn iter_mut(&mut self) -> MixStripsMutIterator<'_> {
        MixStripsMutIterator {
            channel_strips: &mut self.channel_strips,
            bus_strip: Some(&mut self.bus_strip),
            index: 0,
        }
    }
}

/// A Mix contains several channel strips,
/// and one destination or bus strip.
/// The strips are channels
/// that route to the destination.
#[derive(Deserialize, Serialize)]
pub struct Mix {
    pub name: String,
    pub strips: MixStrips,
}

impl Mix {
    pub fn new(
        mix_name: String,
        mix_kind: StripKind,
        mix_number: u32,
        number_of_channels: usize,
    ) -> Self {
        let mut channel_strips = Vec::<Strip>::new();

        for i in 0..number_of_channels {
            let strip = Strip {
                active: false,
                fader: 0.0,
                solo: false,
                mute: false,
                mute_by_solo: false,
                min: -96.0,
                max: 10.0,
                balance: 0.0,
                kind: StripKind::Channel,
                number: i as u32,
            };

            channel_strips.push(strip);
        }

        let bus_strip = Strip {
            active: false,
            fader: 0.0,
            solo: false,
            mute: false,
            mute_by_solo: false,
            min: -96.0,
            max: 10.0,
            balance: 0.0,
            kind: mix_kind,
            number: mix_number,
        };

        Mix {
            name: mix_name,
            strips: MixStrips {
                channel_strips,
                bus_strip,
            },
        }
    }

    pub fn toggle_solo(&mut self, index: usize) {
        if self.strips.iter().nth(index).unwrap().kind == StripKind::Channel {
            self.strips.channel_strips[index].solo = !self.strips.channel_strips[index].solo;

            let mut solo_exists = false;
            for s in self.strips.channel_strips.iter() {
                if s.solo {
                    solo_exists = true;
                    break;
                }
            }

            if solo_exists {
                for strip in self.strips.channel_strips.iter_mut() {
                    strip.mute_by_solo = !strip.solo;
                }
            } else {
                for strip in self.strips.channel_strips.iter_mut() {
                    strip.mute_by_solo = false;
                }
            }
        }
    }
}
