use serde::{Deserialize, Serialize};

/// Identifies a specific control on a strip
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StripControl {
    Fader,
    Balance,
    Mute,
    Solo,
}

/// Identifies a target strip in a specific mix
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StripTarget {
    pub mix_index: usize,
    pub strip_index: usize,
    pub control: StripControl,
}

/// Identifies a MIDI control source
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MidiControl {
    pub channel: u8,  // 0-15
    pub cc: u8,       // 0-127
}

/// Global device controls (not strip-specific)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GlobalControl {
    PhantomPower,
    Line1_2,
    MainMute,
    MainMono,
    ActiveMixSelect,
    ActiveStripSelect,
}

/// What a MIDI control maps to
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ControlTarget {
    Strip(StripTarget),
    Global(GlobalControl),
}

/// A single MIDI mapping entry
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MidiMappingEntry {
    pub midi: MidiControl,
    #[serde(flatten)]
    pub target: ControlTarget,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_range: Option<ValueRange>,
}

/// Complete MIDI mapping configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MidiMapping {
    /// List of MIDI mappings
    pub mappings: Vec<MidiMappingEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ValueRange {
    pub midi_min: u8,    // typically 0
    pub midi_max: u8,    // typically 127
    pub target_min: f64,
    pub target_max: f64,
    #[serde(default)]
    pub curve: Curve,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Curve {
    #[default]
    Linear,
    Exponential,
    Logarithmic,
}

impl MidiMapping {
    /// Create a new empty mapping
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Add a mapping from MIDI CC to a strip control
    pub fn map_strip(&mut self, midi: MidiControl, target: StripTarget, value_range: Option<ValueRange>) {
        self.mappings.push(MidiMappingEntry {
            midi,
            target: ControlTarget::Strip(target),
            value_range,
        });
    }
    
    /// Add a mapping from MIDI CC to a global control
    pub fn map_global(&mut self, midi: MidiControl, target: GlobalControl) {
        self.mappings.push(MidiMappingEntry {
            midi,
            target: ControlTarget::Global(target),
            value_range: None,
        });
    }
    
    /// Get the target for a MIDI control
    pub fn get_target(&self, midi: &MidiControl) -> Option<&ControlTarget> {
        self.mappings
            .iter()
            .find(|entry| &entry.midi == midi)
            .map(|entry| &entry.target)
    }
    
    /// Transform MIDI value (0-127) to target range
    pub fn transform_value(&self, midi: &MidiControl, midi_value: u8) -> f64 {
        if let Some(entry) = self.mappings.iter().find(|e| &e.midi == midi) {
            if let Some(range) = &entry.value_range {
                return range.transform(midi_value);
            }
        }
        // Default: map 0-127 to 0.0-1.0
        midi_value as f64 / 127.0
    }
    
    /// Create a default mapping for a standard control surface
    /// (e.g., 8 faders on CC 1-8, channel 0)
    pub fn create_default() -> Self {
        let mut mapping = Self::new();
        
        // Map CC 1-8 on channel 0 to faders for mix 0, strips 0-7
        for i in 0..8 {
            mapping.map_strip(
                MidiControl { channel: 0, cc: i + 1 },
                StripTarget {
                    mix_index: 0,
                    strip_index: i as usize,
                    control: StripControl::Fader,
                },
                Some(ValueRange {
                    midi_min: 0,
                    midi_max: 127,
                    target_min: -96.0,
                    target_max: 10.0,
                    curve: Curve::Linear,
                }),
            );
        }
        
        // Map CC 10-17 on channel 0 to balance for mix 0, strips 0-7
        for i in 0..8 {
            mapping.map_strip(
                MidiControl { channel: 0, cc: i + 10 },
                StripTarget {
                    mix_index: 0,
                    strip_index: i as usize,
                    control: StripControl::Balance,
                },
                Some(ValueRange {
                    midi_min: 0,
                    midi_max: 127,
                    target_min: -100.0,
                    target_max: 100.0,
                    curve: Curve::Linear,
                }),
            );
        }
        
        // Global controls
        mapping.map_global(
            MidiControl { channel: 0, cc: 102 },
            GlobalControl::PhantomPower,
        );
        
        mapping
    }
}

impl ValueRange {
    /// Transform MIDI value to target range
    pub fn transform(&self, midi_value: u8) -> f64 {
        let midi_normalized = (midi_value as f64 - self.midi_min as f64) 
            / (self.midi_max as f64 - self.midi_min as f64);
        
        let curved = match self.curve {
            Curve::Linear => midi_normalized,
            Curve::Exponential => midi_normalized * midi_normalized,
            Curve::Logarithmic => midi_normalized.sqrt(),
        };
        
        self.target_min + curved * (self.target_max - self.target_min)
    }
}