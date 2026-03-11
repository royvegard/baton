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
    pub channel: u8, // 0-15
    pub cc: u8,      // 0-127
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    pub midi_min: u8, // typically 0
    pub midi_max: u8, // typically 127
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

/// MIDI learn state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiLearnState {
    /// Not learning
    Inactive,
    /// Learning for a specific target
    Learning { target: ControlTarget },
}

impl MidiMapping {
    /// Create a new empty mapping
    pub fn new() -> Self {
        Self::default()
    }

    /// Sort mappings by MIDI channel and CC number
    pub fn sort_mappings(&mut self) {
        self.mappings.sort_by(|a, b| {
            // First compare by channel
            match a.midi.channel.cmp(&b.midi.channel) {
                std::cmp::Ordering::Equal => {
                    // If channels are equal, compare by CC number
                    a.midi.cc.cmp(&b.midi.cc)
                }
                other => other,
            }
        });
    }

    /// Add a mapping from MIDI CC to a strip control
    pub fn map_strip(
        &mut self,
        midi: MidiControl,
        target: StripTarget,
        value_range: Option<ValueRange>,
    ) {
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
        // Default: map 0-127
        midi_value as f64
    }

    /// Create a default mapping for a standard control surface
    /// (e.g., 8 faders on CC 1-8, channel 0)
    pub fn create_default() -> Self {
        let mut mapping = Self::new();

        // Map CC 1-8 on channel 0 to faders for mix 0, strips 0-7
        for i in 0..8 {
            mapping.map_strip(
                MidiControl {
                    channel: 0,
                    cc: i + 1,
                },
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
                MidiControl {
                    channel: 0,
                    cc: i + 10,
                },
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
            MidiControl {
                channel: 0,
                cc: 102,
            },
            GlobalControl::PhantomPower,
        );

        mapping
    }

    /// Start learning mode for a specific target
    /// Returns the current learn state
    pub fn start_learning(&self, target: ControlTarget) -> MidiLearnState {
        MidiLearnState::Learning { target }
    }

    /// Attempt to learn a MIDI mapping
    /// If in learning mode, maps the MIDI control to the target
    /// Returns true if learning was successful
    pub fn learn_mapping(
        &mut self,
        learn_state: &MidiLearnState,
        midi: MidiControl,
        default_range: Option<ValueRange>,
    ) -> bool {
        match learn_state {
            MidiLearnState::Learning { target } => {
                // Remove any existing mapping for this MIDI control
                self.mappings.retain(|entry| entry.midi != midi);

                // Add the new mapping
                match target {
                    ControlTarget::Strip(_) => {
                        if let ControlTarget::Strip(strip_target) = target {
                            self.map_strip(midi, *strip_target, default_range);
                        }
                    }
                    ControlTarget::Global(_) => {
                        if let ControlTarget::Global(global_control) = target {
                            self.map_global(midi, *global_control);
                        }
                    }
                }

                true
            }
            MidiLearnState::Inactive => false,
        }
    }

    /// Remove a mapping for a specific MIDI control
    pub fn remove_mapping(&mut self, midi: &MidiControl) -> bool {
        let len_before = self.mappings.len();
        self.mappings.retain(|entry| &entry.midi != midi);
        self.mappings.len() < len_before
    }

    /// Get default value range for a control type
    pub fn default_range_for_control(control: &StripControl) -> Option<ValueRange> {
        match control {
            StripControl::Fader => Some(ValueRange {
                midi_min: 0,
                midi_max: 127,
                target_min: -50.0,
                target_max: 10.0,
                curve: Curve::Linear,
            }),
            StripControl::Balance => Some(ValueRange {
                midi_min: 0,
                midi_max: 127,
                target_min: -100.0,
                target_max: 100.0,
                curve: Curve::Linear,
            }),
            StripControl::Mute | StripControl::Solo => None,
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_mapping_is_empty() {
        let mapping = MidiMapping::new();
        assert_eq!(mapping.mappings.len(), 0);
    }

    #[test]
    fn test_map_strip() {
        let mut mapping = MidiMapping::new();
        let midi = MidiControl { channel: 0, cc: 1 };
        let target = StripTarget {
            mix_index: 0,
            strip_index: 0,
            control: StripControl::Fader,
        };

        mapping.map_strip(midi, target, None);

        assert_eq!(mapping.mappings.len(), 1);
        assert_eq!(mapping.mappings[0].midi, midi);
        match &mapping.mappings[0].target {
            ControlTarget::Strip(t) => assert_eq!(*t, target),
            _ => panic!("Expected Strip target"),
        }
    }

    #[test]
    fn test_map_global() {
        let mut mapping = MidiMapping::new();
        let midi = MidiControl {
            channel: 0,
            cc: 102,
        };

        mapping.map_global(midi, GlobalControl::PhantomPower);

        assert_eq!(mapping.mappings.len(), 1);
        assert_eq!(mapping.mappings[0].midi, midi);
        match &mapping.mappings[0].target {
            ControlTarget::Global(GlobalControl::PhantomPower) => {}
            _ => panic!("Expected PhantomPower global target"),
        }
    }

    #[test]
    fn test_get_target() {
        let mut mapping = MidiMapping::new();
        let midi = MidiControl { channel: 0, cc: 1 };
        let target = StripTarget {
            mix_index: 0,
            strip_index: 0,
            control: StripControl::Fader,
        };

        mapping.map_strip(midi, target, None);

        let found = mapping.get_target(&midi);
        assert!(found.is_some());

        let not_found = mapping.get_target(&MidiControl { channel: 1, cc: 1 });
        assert!(not_found.is_none());
    }

    #[test]
    fn test_sort_mappings() {
        let mut mapping = MidiMapping::new();

        // Add in random order
        mapping.map_strip(
            MidiControl { channel: 0, cc: 10 },
            StripTarget {
                mix_index: 0,
                strip_index: 0,
                control: StripControl::Balance,
            },
            None,
        );
        mapping.map_strip(
            MidiControl { channel: 1, cc: 5 },
            StripTarget {
                mix_index: 0,
                strip_index: 1,
                control: StripControl::Fader,
            },
            None,
        );
        mapping.map_strip(
            MidiControl { channel: 0, cc: 2 },
            StripTarget {
                mix_index: 0,
                strip_index: 2,
                control: StripControl::Fader,
            },
            None,
        );
        mapping.map_strip(
            MidiControl {
                channel: 0,
                cc: 102,
            },
            StripTarget {
                mix_index: 0,
                strip_index: 3,
                control: StripControl::Fader,
            },
            None,
        );

        mapping.sort_mappings();

        // Check sorted order
        assert_eq!(mapping.mappings[0].midi, MidiControl { channel: 0, cc: 2 });
        assert_eq!(mapping.mappings[1].midi, MidiControl { channel: 0, cc: 10 });
        assert_eq!(
            mapping.mappings[2].midi,
            MidiControl {
                channel: 0,
                cc: 102
            }
        );
        assert_eq!(mapping.mappings[3].midi, MidiControl { channel: 1, cc: 5 });
    }

    #[test]
    fn test_transform_value_with_range() {
        let mut mapping = MidiMapping::new();
        let midi = MidiControl { channel: 0, cc: 1 };
        let target = StripTarget {
            mix_index: 0,
            strip_index: 0,
            control: StripControl::Fader,
        };
        let range = ValueRange {
            midi_min: 0,
            midi_max: 127,
            target_min: -96.0,
            target_max: 10.0,
            curve: Curve::Linear,
        };

        mapping.map_strip(midi, target, Some(range));

        // Test min value
        let result = mapping.transform_value(&midi, 0);
        assert_eq!(result, -96.0);

        // Test max value
        let result = mapping.transform_value(&midi, 127);
        assert_eq!(result, 10.0);

        // Test mid value (should be around -43.0)
        let result = mapping.transform_value(&midi, 64);
        assert!((result - (-43.0)).abs() < 1.0);
    }

    #[test]
    fn test_transform_value_without_range() {
        let mapping = MidiMapping::new();
        let midi = MidiControl { channel: 0, cc: 1 };

        // Should default to 0.0-1.0 mapping
        let result = mapping.transform_value(&midi, 0);
        assert_eq!(result, 0.0);

        let result = mapping.transform_value(&midi, 127);
        assert_eq!(result, 1.0);

        let result = mapping.transform_value(&midi, 64);
        assert!((result - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_value_range_linear() {
        let range = ValueRange {
            midi_min: 0,
            midi_max: 100,
            target_min: 0.0,
            target_max: 100.0,
            curve: Curve::Linear,
        };

        assert_eq!(range.transform(0), 0.0);
        assert_eq!(range.transform(50), 50.0);
        assert_eq!(range.transform(100), 100.0);
    }

    #[test]
    fn test_value_range_exponential() {
        let range = ValueRange {
            midi_min: 0,
            midi_max: 100,
            target_min: 0.0,
            target_max: 100.0,
            curve: Curve::Exponential,
        };

        // Exponential curve: slower at start, faster at end
        assert_eq!(range.transform(0), 0.0);
        assert_eq!(range.transform(50), 25.0); // 0.5^2 * 100 = 25
        assert_eq!(range.transform(100), 100.0);
    }

    #[test]
    fn test_value_range_logarithmic() {
        let range = ValueRange {
            midi_min: 0,
            midi_max: 100,
            target_min: 0.0,
            target_max: 100.0,
            curve: Curve::Logarithmic,
        };

        // Logarithmic curve: faster at start, slower at end
        assert_eq!(range.transform(0), 0.0);
        assert!((range.transform(25) - 50.0).abs() < 0.1); // sqrt(0.25) * 100 ≈ 50
        assert_eq!(range.transform(100), 100.0);
    }

    #[test]
    fn test_create_default() {
        let mapping = MidiMapping::create_default();

        // Should have 8 faders + 8 balance + 1 global = 17 mappings
        assert_eq!(mapping.mappings.len(), 17);

        // Check first fader
        let first = &mapping.mappings[0];
        assert_eq!(first.midi.channel, 0);
        assert_eq!(first.midi.cc, 1);
        match &first.target {
            ControlTarget::Strip(t) => {
                assert_eq!(t.control, StripControl::Fader);
                assert_eq!(t.strip_index, 0);
            }
            _ => panic!("Expected strip target"),
        }

        // Check phantom power global control
        let phantom = mapping.mappings.iter().find(|e| e.midi.cc == 102).unwrap();
        match &phantom.target {
            ControlTarget::Global(GlobalControl::PhantomPower) => {}
            _ => panic!("Expected PhantomPower"),
        }
    }

    #[test]
    fn test_serialization_roundtrip() {
        let mut mapping = MidiMapping::create_default();
        mapping.sort_mappings();

        // Serialize to JSON
        let json = serde_json::to_string_pretty(&mapping).unwrap();

        // Deserialize back
        let deserialized: MidiMapping = serde_json::from_str(&json).unwrap();

        // Should have same number of mappings
        assert_eq!(mapping.mappings.len(), deserialized.mappings.len());

        // Check first mapping matches
        assert_eq!(mapping.mappings[0].midi, deserialized.mappings[0].midi);
    }
}
