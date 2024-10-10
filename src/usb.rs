use futures_lite::future::block_on;
use nusb::{
    transfer::{ControlOut, ControlType, Recipient, ResponseBuffer, TransferError},
    Device,
};
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
    pub mixes: Vec<Mix>,
    // TODO: These states can be read from device
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
            mixes: vec![
                Mix::new(String::from("MAIN 1-2"), StripKind::Main, 4),
                Mix::new(String::from("MIX 3-4"), StripKind::Bus, 1),
                Mix::new(String::from("MIX 5-6"), StripKind::Bus, 2),
                Mix::new(String::from("MIX 7-8"), StripKind::Bus, 3),
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
        self.command.output_strip = 0x00;
        self.command.output_channel = BUTTON_1_2_LINE;
        self.command.value = b;

        self.send_command();
        self.in_1_2_line = on;
    }

    pub fn set_main_mute(&mut self, on: bool) {
        let b: u32 = if on { 0x01 } else { 0x00 };
        self.command.mode = MODE_BUTTON;
        self.command.input_strip = 0x00;
        self.command.output_strip = 0x00;
        self.command.output_channel = BUTTON_MAIN_MUTE;
        self.command.value = b;

        self.send_command();
        self.main_mute = on;
    }

    pub fn set_main_mono(&mut self, on: bool) {
        let b: u32 = if on { 0x01 } else { 0x00 };
        self.command.mode = MODE_BUTTON;
        self.command.input_strip = 0x00;
        self.command.output_strip = 0x00;
        self.command.output_channel = BUTTON_MAIN_MONO;
        self.command.value = b;

        self.send_command();
        self.main_mono = on;
    }

    pub fn set_phantom_power(&mut self, on: bool) {
        let b: u32 = if on { 0x01 } else { 0x00 };
        self.command.mode = MODE_BUTTON;
        self.command.input_strip = 0x00;
        self.command.output_strip = 0x00;
        self.command.output_channel = BUTTON_PHANTOM_POWER;
        self.command.value = b;

        self.send_command();
        self.phantom_power = on;
    }

    pub fn send_command(&self) {
        let _ = self.command.send_usb_command(&self.device);
    }
}

#[derive(Debug)]
pub struct Command {
    pub mode: u32,
    pub input_strip: u32,
    fix1: u32,
    fix2: u32,
    pub output_strip: u32,
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
            output_strip: 0x04,
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
        for b in self.output_strip.to_le_bytes() {
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

#[derive(Debug)]
pub enum StripKind {
    Channel,
    Bus,
    Main,
}

pub struct Strip {
    pub name: String,
    pub fader: f64,
    pub balance: f64,
    pub solo: bool,
    pub mute: bool,
    pub mute_by_solo: bool,
    pub max: f64,
    pub min: f64,
    pub active: bool,
    pub kind: StripKind,
    pub number: u32,
}

impl Strip {
    pub fn set_fader(&mut self, value: f64) {
        self.fader = value.clamp(self.min, self.max);
    }
}

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
        fader.output_strip = 0x04;
        fader.output_channel = RIGHT;
        assert_eq!(fader.as_array(), a);
    }
}
