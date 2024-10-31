use futures_lite::future::block_on;
use nusb::{
    transfer::{ControlIn, ControlOut, ControlType, Recipient, ResponseBuffer, TransferError},
    Device,
};
use serde::{Deserialize, Serialize};
use std::{
    f64::consts::E,
    io::{self, ErrorKind},
};

// Modes
pub(crate) const MODE_BUTTON: u32 = 0x00;
pub(crate) const MODE_CHANNEL_STRIP: u32 = 0x64;
pub(crate) const MODE_BUS_STRIP: u32 = 0x65;

// Buttons
const BUTTON_1_2_LINE: u32 = 0x00;
const BUTTON_MAIN_MUTE: u32 = 0x01;
const BUTTON_MAIN_MONO: u32 = 0x02;
const BUTTON_PHANTOM_POWER: u32 = 0x04;

// Output channels
pub(crate) const LEFT: u32 = 0x00;
pub(crate) const RIGHT: u32 = 0x01;

// Fader presets
pub(crate) const MUTED: u32 = 0x00;
pub(crate) const UNITY: u32 = 0xbc000000;

pub struct PreSonusStudio1824c {
    pub device: Device,
    pub command: Command,
    pub state: State,
    pub mixes: Vec<Mix>,
    pub in_1_2_line: bool,
    pub main_mute: bool,
    pub main_mono: bool,
    pub phantom_power: bool,
}

impl PreSonusStudio1824c {
    pub fn new() -> Result<Self, io::Error> {
        let device_info = match nusb::list_devices()?
            .find(|dev| dev.vendor_id() == 0x194f && dev.product_id() == 0x010d)
        {
            Some(d) => d,
            None => {
                return Err(io::Error::new(
                    ErrorKind::NotFound,
                    "PreSonus STUDIO1824c not found",
                ));
            }
        };

        let device = device_info.open()?;

        Ok(PreSonusStudio1824c {
            device,
            command: Command::new(),
            state: State::new(),
            mixes: vec![
                Mix::new(String::from("MAIN 1-2"), StripKind::Main, 0),
                Mix::new(String::from("MIX 3-4"), StripKind::Bus, 1),
                Mix::new(String::from("MIX 5-6"), StripKind::Bus, 2),
                Mix::new(String::from("MIX 7-8"), StripKind::Bus, 3),
                Mix::new(String::from("S/PDIF"), StripKind::Bus, 4),
                Mix::new(String::from("ADAT 1-2"), StripKind::Bus, 5),
                Mix::new(String::from("ADAT 3-4"), StripKind::Bus, 6),
                Mix::new(String::from("ADAT 5-6"), StripKind::Bus, 7),
                Mix::new(String::from("ADAT 7-8"), StripKind::Bus, 8),
            ],
            in_1_2_line: false,
            main_mute: false,
            main_mono: false,
            phantom_power: false,
        })
    }

    pub fn set_1_2_line(&mut self, on: bool) {
        let b: u32 = if on { 0x01 } else { 0x00 };
        self.command.mode = MODE_BUTTON;
        self.command.input_strip = 0x00;
        self.command.output_bus = 0x00;
        self.command.output_channel = BUTTON_1_2_LINE;
        self.command.value = b;

        self.send_command();
        self.in_1_2_line = on;
    }

    pub fn set_main_mute(&mut self, on: bool) {
        let b: u32 = if on { 0x01 } else { 0x00 };
        self.command.mode = MODE_BUTTON;
        self.command.input_strip = 0x00;
        self.command.output_bus = 0x00;
        self.command.output_channel = BUTTON_MAIN_MUTE;
        self.command.value = b;

        self.send_command();
        self.main_mute = on;
    }

    pub fn set_main_mono(&mut self, on: bool) {
        let b: u32 = if on { 0x01 } else { 0x00 };
        self.command.mode = MODE_BUTTON;
        self.command.input_strip = 0x00;
        self.command.output_bus = 0x00;
        self.command.output_channel = BUTTON_MAIN_MONO;
        self.command.value = b;

        self.send_command();
        self.main_mono = on;
    }

    pub fn set_phantom_power(&mut self, on: bool) {
        let b: u32 = if on { 0x01 } else { 0x00 };
        self.command.mode = MODE_BUTTON;
        self.command.input_strip = 0x00;
        self.command.output_bus = 0x00;
        self.command.output_channel = BUTTON_PHANTOM_POWER;
        self.command.value = b;

        self.send_command();
        self.phantom_power = on;
    }

    pub fn send_command(&self) {
        let _ = self.command.send_usb_command(&self.device);
    }

    pub fn poll_state(&mut self) {
        let dat = self.state.poll_usb_data(&self.device);

        if let Ok(d) = dat {
            self.state.parse_state(d);

            // synch meters
            let mut mic = [0.0; 8];
            let mut adat = [0.0; 8];
            let mut spdif = [0.0; 2];
            let mut daw = [0.0; 18];
            let mut bus = [0.0; 18];

            for (i, v) in mic.iter_mut().enumerate() {
                *v = State::get_db(self.state.mic[i]);
            }
            for (i, v) in adat.iter_mut().enumerate() {
                *v = State::get_db(self.state.adat[i]);
            }
            for (i, v) in spdif.iter_mut().enumerate() {
                *v = State::get_db(self.state.spdif[i]);
            }
            for (i, v) in daw.iter_mut().enumerate() {
                *v = State::get_db(self.state.daw[i]);
            }
            for (i, v) in bus.iter_mut().enumerate() {
                *v = State::get_db(self.state.bus[i]);
            }

            let mut bus_index = 0;
            for m in &mut self.mixes.iter_mut() {
                let mut channel_index = 0;

                for v in mic {
                    m.channel_strips[channel_index].meter.0 = v;
                    channel_index += 1;
                }
                for v in adat {
                    m.channel_strips[channel_index].meter.0 = v;
                    channel_index += 1;
                }
                for v in spdif {
                    m.channel_strips[channel_index].meter.0 = v;
                    channel_index += 1;
                }
                for v in daw {
                    m.channel_strips[channel_index].meter.0 = v;
                    channel_index += 1;
                }

                m.channel_strips[channel_index].meter.0 = bus[bus_index];
                bus_index += 1;
                m.channel_strips[channel_index].meter.1 = bus[bus_index];
                bus_index += 1;
            }

            // synch button states
            self.phantom_power = self.state.phantom == 0x01;
            self.in_1_2_line = self.state.line == 0x01;
            self.main_mute = self.state.mute == 0x01;
            self.main_mono = self.state.mono == 0x01;
        }
    }
}

#[derive(Debug)]
pub struct Command {
    pub mode: u32,
    pub input_strip: u32,
    fix1: u32,
    fix2: u32,
    pub output_bus: u32,
    pub output_channel: u32,
    pub value: u32,
}

impl Command {
    pub fn new() -> Self {
        Command {
            mode: MODE_CHANNEL_STRIP,
            input_strip: 0x00,
            fix1: 0x50617269,
            fix2: 0x14,
            output_bus: 0x04,
            output_channel: LEFT,
            value: 0x00000000,
        }
    }

    pub fn as_array(&self) -> [u8; 28] {
        let mut arr = [0u8; 28];
        let mut i = 0;

        for b in self.mode.to_le_bytes() {
            arr[i] = b;
            i += 1;
        }
        for b in self.input_strip.to_le_bytes() {
            arr[i] = b;
            i += 1;
        }
        for b in self.fix1.to_le_bytes() {
            arr[i] = b;
            i += 1;
        }
        for b in self.fix2.to_le_bytes() {
            arr[i] = b;
            i += 1;
        }
        for b in self.output_bus.to_le_bytes() {
            arr[i] = b;
            i += 1;
        }
        for b in self.output_channel.to_le_bytes() {
            arr[i] = b;
            i += 1;
        }
        for b in self.value.to_le_bytes() {
            arr[i] = b;
            i += 1;
        }

        arr
    }

    pub fn set_db(&mut self, db: f64) {
        self.value = (11877360.0 * E.powf(db.clamp(-96.0, 10.0) / 10.0)) as u32;
    }

    pub fn send_usb_command(&self, device: &Device) -> Result<ResponseBuffer, TransferError> {
        let fader_control: ControlOut = ControlOut {
            control_type: ControlType::Vendor,
            recipient: Recipient::Device,
            request: 160,
            value: 0x0000,
            index: 0,
            data: &self.as_array(),
        };

        block_on(device.control_out(fader_control)).into_result()
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
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
    /// Strip name.
    pub name: String,
    /// Volume fader in dB.
    pub fader: f64,
    /// Left and right meter in dBFS.
    /// Left only for mono strips.
    #[serde(skip)]
    pub meter: (f64, f64),
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

/// A Mix contains several channel strips.
/// The last strip is the destination or bus strip,
/// and the preceeding strips are channels
/// that route to the destination.
#[derive(Deserialize, Serialize)]
pub struct Mix {
    pub channel_strips: Vec<Strip>,
}

impl Mix {
    pub fn new(mix_name: String, mix_kind: StripKind, mix_number: u32) -> Self {
        let mut channel_strips = Vec::<Strip>::new();
        let mut names = vec![];

        for i in 1..=8 {
            names.push(format!("MIC {}", i));
        }
        for i in 1..=8 {
            names.push(format!("ADAT {}", i));
        }
        names.push("S/PDIF 1".to_string());
        names.push("S/PDIF 2".to_string());
        for i in 1..=18 {
            names.push(format!("DAW {}", i));
        }

        for (i, n) in names.iter().enumerate() {
            let strip = Strip {
                name: n.to_string(),
                active: false,
                fader: 0.0,
                meter: (-f64::INFINITY, -f64::INFINITY),
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

        // The last strip is the destination strip
        let destination_strip = Strip {
            name: mix_name,
            active: false,
            fader: 0.0,
            meter: (-f64::INFINITY, -f64::INFINITY),
            solo: false,
            mute: false,
            mute_by_solo: false,
            min: -96.0,
            max: 10.0,
            balance: 0.0,
            kind: mix_kind,
            number: mix_number,
        };

        channel_strips.push(destination_strip);

        Mix { channel_strips }
    }

    pub fn get_destination_strip(&self) -> &Strip {
        self.channel_strips
            .last()
            .expect("Channel strips should not be empty")
    }
}

pub struct State {
    counter: u16,
    d1: u32,
    d2: u32,
    fix1: u32,
    fix2: u32,
    /// Microphone input meters.
    mic: [u32; 8],
    /// S/PDIF input meters.
    spdif: [u32; 2],
    /// ADAT input meters.
    adat: [u32; 8],
    /// DAW input meters.
    daw: [u32; 18],
    /// Stereo busses meters.
    bus: [u32; 18],
    /// 48V phantom power.
    pub phantom: u32,
    /// Channel 1-2 line mode.
    pub line: u32,
    /// Main mix mute.
    pub mute: u32,
    /// Main mix mono.
    pub mono: u32,
    d5: u32,
}

impl State {
    fn new() -> Self {
        State {
            counter: 0x01,
            d1: 0x00,
            d2: 0x00,
            fix1: 0x64656d73,
            fix2: 0xf4,
            mic: [0x00; 8],
            spdif: [0x00; 2],
            adat: [0x00; 8],
            daw: [0x00; 18],
            bus: [0x00; 18],
            phantom: 0x00,
            line: 0x00,
            mute: 0x00,
            mono: 0x00,
            d5: 0x00,
        }
    }

    /// Reset all values to zero.
    /// This is used before requesting state from device.
    fn reset(&mut self) {
        self.counter = 0x01;
        self.d1 = 0x00;
        self.d2 = 0x00;
        self.fix1 = 0x64656d73;
        self.fix2 = 0xf4;
        self.mic = [0x00; 8];
        self.spdif = [0x00; 2];
        self.adat = [0x00; 8];
        self.daw = [0x00; 18];
        self.bus = [0x00; 18];
        self.phantom = 0x00;
        self.line = 0x00;
        self.mute = 0x00;
        self.mono = 0x00;
        self.d5 = 0x00;
    }

    /// Return the state as an array of bytes.
    pub fn as_array(&self) -> [u8; 252] {
        let mut arr = [0u8; 252];
        let mut i = 0;

        for b in self.d1.to_le_bytes() {
            arr[i] = b;
            i += 1;
        }
        for b in self.d2.to_le_bytes() {
            arr[i] = b;
            i += 1;
        }
        for b in self.fix1.to_le_bytes() {
            arr[i] = b;
            i += 1;
        }
        for b in self.fix2.to_le_bytes() {
            arr[i] = b;
            i += 1;
        }
        for m in self.mic {
            for b in m.to_le_bytes() {
                arr[i] = b;
                i += 1;
            }
        }
        for m in self.spdif {
            for b in m.to_le_bytes() {
                arr[i] = b;
                i += 1;
            }
        }
        for m in self.adat {
            for b in m.to_le_bytes() {
                arr[i] = b;
                i += 1;
            }
        }
        for m in self.daw {
            for b in m.to_le_bytes() {
                arr[i] = b;
                i += 1;
            }
        }
        for m in self.bus {
            for b in m.to_le_bytes() {
                arr[i] = b;
                i += 1;
            }
        }
        for b in self.phantom.to_le_bytes() {
            arr[i] = b;
            i += 1;
        }
        for b in self.line.to_le_bytes() {
            arr[i] = b;
            i += 1;
        }
        for b in self.mute.to_le_bytes() {
            arr[i] = b;
            i += 1;
        }
        for b in self.mono.to_le_bytes() {
            arr[i] = b;
            i += 1;
        }
        for b in self.d5.to_le_bytes() {
            arr[i] = b;
            i += 1;
        }

        arr
    }

    /// Convert a slice of 4 bytes to a u32.
    pub fn slice_to_u32(slice: &[u8]) -> u32 {
        let mut out: u32 = slice[0] as u32;
        out += slice[1] as u32 * 0x100;
        out += slice[2] as u32 * 0x100 * 0x100;
        out += slice[3] as u32 * 0x100 * 0x100 * 0x100;

        out
    }

    pub fn parse_state(&mut self, slice: Vec<u8>) {
        const MIC_INDEX: usize = 0x10;
        const ADAT_INDEX: usize = 0x38;
        const SPDIF_INDEX: usize = 0x30;
        const DAW_INDEX: usize = 0x58;
        const BUS_INDEX: usize = 0xa0;

        for i in 0..self.mic.len() {
            self.mic[i] = Self::slice_to_u32(&slice[MIC_INDEX + 4 * i..=MIC_INDEX + 4 * i + 4]);
        }
        for i in 0..self.adat.len() {
            self.adat[i] = Self::slice_to_u32(&slice[ADAT_INDEX + 4 * i..=ADAT_INDEX + 4 * i + 4]);
        }
        for i in 0..self.spdif.len() {
            self.spdif[i] =
                Self::slice_to_u32(&slice[SPDIF_INDEX + 4 * i..=SPDIF_INDEX + 4 * i + 4]);
        }
        for i in 0..self.daw.len() {
            self.daw[i] = Self::slice_to_u32(&slice[DAW_INDEX + 4 * i..=DAW_INDEX + 4 * i + 4]);
        }
        for i in 0..self.bus.len() {
            self.bus[i] = Self::slice_to_u32(&slice[BUS_INDEX + 4 * i..=BUS_INDEX + 4 * i + 4]);
        }

        self.phantom = slice[0xe8] as u32;
        self.line = slice[0xec] as u32;
        self.mute = slice[0xf0] as u32;
        self.mono = slice[0xf4] as u32;
    }

    /// Convert from integer amplitude to db amplitude.
    pub fn get_db(input: u32) -> f64 {
        const ZERO_DBFS: u32 = 0x7fffff00;
        20.0 * (input as f64 / ZERO_DBFS as f64).log10()
    }

    pub fn poll_usb_data(&mut self, device: &Device) -> Result<Vec<u8>, TransferError> {
        self.reset();

        let control: ControlOut = ControlOut {
            control_type: ControlType::Vendor,
            recipient: Recipient::Device,
            request: 161,
            value: self.counter.to_le(),
            index: 0,
            data: &self.as_array(),
        };

        let _ = block_on(device.control_out(control)).into_result();

        let control: ControlIn = ControlIn {
            control_type: ControlType::Vendor,
            recipient: Recipient::Device,
            request: 162,
            value: self.counter.to_le(),
            index: 0,
            length: self.as_array().len() as u16,
        };

        if self.counter == 0xffff {
            self.counter = 0x00;
        }
        self.counter += 1;

        block_on(device.control_in(control)).into_result()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fadercommand_set_db() {
        let mut fader = Command::new();
        fader.set_db(-96.0);
        fader.set_db(-10.0);
        fader.set_db(0.0);
        fader.set_db(10.0);
    }

    #[test]
    fn fadercommand_set_value() {
        let mut fader = Command::new();
        fader.value = UNITY;
        fader.value = MUTED;
    }

    #[test]
    fn fadercommand_as_array() {
        let mut fader = Command::new();
        let a = [
            0x65, 0x00, 0x00, 0x00, 0x22, 0x00, 0x00, 0x00, 0x69, 0x72, 0x61, 0x50, 0x14, 0x00,
            0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x26, 0x70, 0xc1, 0x00,
        ];

        fader.mode = MODE_BUS_STRIP;
        fader.value = 12677158;
        fader.set_db(0.651678);
        fader.input_strip = 0x22;
        fader.output_bus = 0x04;
        fader.output_channel = RIGHT;
        assert_eq!(fader.as_array(), a);
    }
}
