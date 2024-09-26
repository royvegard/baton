use futures_lite::future::block_on;
use nusb::transfer::{ControlOut, ControlType, Recipient};
use std::{f64::consts::E, time::Duration};

#[derive(Debug)]
struct FaderCommand {
    mode: u32,
    input_strip: u32,
    fix1: u32,
    fix2: u32,
    output_strip: u32,
    output_channel: u32,
    value: u32,
}

// Modes
const MODE_BUTTON: u32 = 0x00;
const MODE_CHANNEL_STRIP: u32 = 0x64;
const MODE_BUS_STRIP: u32 = 0x65;

// Output channels
const LEFT: u32 = 0x00;
const RIGHT: u32 = 0x01;

// Fader presets
const MUTED: u32 = 0x00;
const UNITY: u32 = 0xbc000000;

impl FaderCommand {
    fn new() -> Self {
        FaderCommand {
            mode: MODE_CHANNEL_STRIP,
            input_strip: 0x00,
            fix1: 0x50617269,
            fix2: 0x14,
            output_strip: 0x04,
            output_channel: LEFT,
            value: 0x00000000,
        }
    }

    fn as_array(&self) -> [u8; 28] {
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

    fn set_db(&mut self, db: f64) {
        self.value = (11877360.0 * E.powf(db.clamp(-96.0, 10.0) / 10.0)) as u32;
    }
}

fn main() {
    let device_info = nusb::list_devices()
        .unwrap()
        .find(|dev| dev.vendor_id() == 0x194f && dev.product_id() == 0x010d)
        .expect("device not connected");

    let device = device_info.open().expect("failed to open device");
    let interface = device.claim_interface(0);

    for i in device_info.interfaces() {
        println!("{:?}", i);
    }

    for c in device.configurations() {
        println!("configuration: {:?}", c);
    }

    let desc = device
        .get_string_descriptor(0x09, 0, Duration::from_millis(100))
        .unwrap();
    println!("desc:\n{}", desc);

    let configuration = device
        .get_descriptor(0x02, 0x00, 0x0000, Duration::from_millis(100))
        .unwrap();
    println!("configuration:\n{:?}", configuration);

    let mut fader = FaderCommand::new();
    fader.mode = MODE_BUS_STRIP;
    fader.output_strip = 0x04;

    let mut line = String::with_capacity(5);
    while true {
        println!("\nEnter db (q to qiut)");
        line.clear();
        std::io::stdin()
            .read_line(&mut line)
            .expect("Failed to read line");

        if line.trim_end() == "q" {
            break;
        }

        if let Ok(db) = line.trim_end().parse::<f64>() {
            fader.set_db(db);
            fader.output_channel = LEFT;
            println!("setting volume to {}", fader.value);

            let fader_control: ControlOut = ControlOut {
                control_type: ControlType::Vendor,
                recipient: Recipient::Device,
                request: 160,
                value: 0x0000,
                index: 0,
                data: &fader.as_array(),
            };

            let result = block_on(device.control_out(fader_control))
                .into_result()
                .unwrap();

            fader.output_channel = RIGHT;
            let fader_control: ControlOut = ControlOut {
                control_type: ControlType::Vendor,
                recipient: Recipient::Device,
                request: 160,
                value: 0x0000,
                index: 0,
                data: &fader.as_array(),
            };

            let result = block_on(device.control_out(fader_control))
                .into_result()
                .unwrap();
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fadercommand_set_db() {
        let mut fader = FaderCommand::new();
        fader.set_db(-96.0);
        fader.set_db(-10.0);
        fader.set_db(0.0);
        fader.set_db(10.0);
    }

    #[test]
    fn fadercommand_set_value() {
        let mut fader = FaderCommand::new();
        fader.value = UNITY;
        fader.value = MUTED;
    }

    #[test]
    fn fadercommand_as_array() {
        let mut fader = FaderCommand::new();
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
