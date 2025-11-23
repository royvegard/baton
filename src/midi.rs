use alsa::seq::{EventType, EvCtrl, PortCap, PortType};
use alsa::{seq, Direction};
use std::ffi::CString;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

pub enum MidiMessage {
    ControlChange {
        channel: u8,
        controller: u8,
        value: u8,
    },
}

pub struct MidiInput {
    receiver: Receiver<MidiMessage>,
}

impl MidiInput {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let (sender, receiver) = mpsc::channel();

        thread::spawn(move || {
            if let Err(e) = run_midi_loop(sender) {
                log::error!("MIDI thread error: {}", e);
            }
        });

        log::info!("ALSA MIDI sequencer port initialized");

        Ok(MidiInput { receiver })
    }

    pub fn try_recv(&self) -> Option<MidiMessage> {
        self.receiver.try_recv().ok()
    }
}

fn run_midi_loop(sender: Sender<MidiMessage>) -> Result<(), Box<dyn std::error::Error>> {
    // Open ALSA sequencer
    let seq = seq::Seq::open(None, Some(Direction::Capture), false)?;
    let client_name = CString::new("Baton")?;
    seq.set_client_name(&client_name)?;

    // Create input port
    let port_name = CString::new("baton-midi-in")?;
    let port = seq.create_simple_port(
        &port_name,
        PortCap::WRITE | PortCap::SUBS_WRITE,
        PortType::MIDI_GENERIC | PortType::APPLICATION,
    )?;

    let client_id = seq.client_id()?;
    log::info!("Created ALSA MIDI port: {}:{} (Baton:baton-midi-in)", client_id, port);
    log::info!("Connect MIDI devices using: aconnect <source-port> {}:{}", client_id, port);
    log::info!("Or use: aconnect <source-port> Baton:baton-midi-in");

    // Set up input for receiving events
    let mut input = seq.input();

    log::info!("Listening for MIDI messages...");

    loop {
        if let Ok(event) = input.event_input() {
            let event_type = event.get_type();
            
            match event_type {
                EventType::Controller => {
                    // Control Change - use EvCtrl to extract structured data
                    if let Some(ctrl_data) = event.get_data::<EvCtrl>() {
                        let _ = sender.send(MidiMessage::ControlChange {
                            channel: ctrl_data.channel,
                            controller: ctrl_data.param as u8,
                            value: ctrl_data.value as u8,
                        });
                        log::debug!("MIDI CC: ch={}, cc={}, val={}", 
                            ctrl_data.channel, ctrl_data.param, ctrl_data.value);
                    }
                }
                _ => {}
            }
        }
    }
}
