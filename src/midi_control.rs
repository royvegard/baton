use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MidiControl {
    pub channel: u8,  // 0-15
    pub cc: u8,       // 0-127
}

impl MidiControl {
    /// Convert to string key for serialization: "chan XX: cc YYY"
    pub fn to_key(&self) -> String {
        format!("chan {:02}: cc {:03}", self.channel, self.cc)
    }
    
    /// Parse from string key: "chan XX: cc YYY"
    pub fn from_key(s: &str) -> Option<Self> {
        let rest = s.strip_prefix("chan ")?;
        let (ch_str, cc_part) = rest.split_once(": cc ")?;
        let channel = ch_str.trim().parse().ok()?;
        let cc = cc_part.trim().parse().ok()?;
        Some(MidiControl { channel, cc })
    }
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

/// Complete MIDI mapping configuration
#[derive(Debug, Clone, Default)]
pub struct MidiMapping {
    /// Map MIDI CC to strip/global controls
    pub mappings: HashMap<MidiControl, ControlTarget>,
    
    /// Optional: Value transformation
    /// Maps MIDI value range (0-127) to custom min/max
    pub value_ranges: HashMap<MidiControl, ValueRange>,
}

// Custom serialization for MidiMapping
impl Serialize for MidiMapping {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeStruct;
        use std::collections::BTreeMap;
        
        let mut state = serializer.serialize_struct("MidiMapping", 2)?;
        
        // Convert HashMap to BTreeMap for sorted output
        // BTreeMap automatically sorts by key
        let mappings_str: BTreeMap<String, &ControlTarget> = self
            .mappings
            .iter()
            .map(|(k, v)| (k.to_key(), v))
            .collect();
        state.serialize_field("mappings", &mappings_str)?;
        
        let ranges_str: BTreeMap<String, &ValueRange> = self
            .value_ranges
            .iter()
            .map(|(k, v)| (k.to_key(), v))
            .collect();
        state.serialize_field("value_ranges", &ranges_str)?;
        
        state.end()
    }
}

// Custom deserialization for MidiMapping
impl<'de> Deserialize<'de> for MidiMapping {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::{self, MapAccess, Visitor};
        use std::fmt;
        
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "snake_case")]
        enum Field {
            Mappings,
            ValueRanges,
        }
        
        struct MidiMappingVisitor;
        
        impl<'de> Visitor<'de> for MidiMappingVisitor {
            type Value = MidiMapping;
            
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct MidiMapping")
            }
            
            fn visit_map<V>(self, mut map: V) -> Result<MidiMapping, V::Error>
            where
                V: MapAccess<'de>,
            {
                let mut mappings: Option<HashMap<String, ControlTarget>> = None;
                let mut value_ranges: Option<HashMap<String, ValueRange>> = None;
                
                while let Some(key) = map.next_key()? {
                    match key {
                        Field::Mappings => {
                            if mappings.is_some() {
                                return Err(de::Error::duplicate_field("mappings"));
                            }
                            mappings = Some(map.next_value()?);
                        }
                        Field::ValueRanges => {
                            if value_ranges.is_some() {
                                return Err(de::Error::duplicate_field("value_ranges"));
                            }
                            value_ranges = Some(map.next_value()?);
                        }
                    }
                }
                
                let mappings_str = mappings.unwrap_or_default();
                let ranges_str = value_ranges.unwrap_or_default();
                
                // Convert HashMap<String, _> back to HashMap<MidiControl, _>
                let mut mappings_map = HashMap::new();
                for (key, value) in mappings_str {
                    if let Some(midi_control) = MidiControl::from_key(&key) {
                        mappings_map.insert(midi_control, value);
                    } else {
                        return Err(de::Error::custom(format!("Invalid MIDI control key: {}", key)));
                    }
                }
                
                let mut ranges_map = HashMap::new();
                for (key, value) in ranges_str {
                    if let Some(midi_control) = MidiControl::from_key(&key) {
                        ranges_map.insert(midi_control, value);
                    } else {
                        return Err(de::Error::custom(format!("Invalid MIDI control key: {}", key)));
                    }
                }
                
                Ok(MidiMapping {
                    mappings: mappings_map,
                    value_ranges: ranges_map,
                })
            }
        }
        
        const FIELDS: &[&str] = &["mappings", "value_ranges"];
        deserializer.deserialize_struct("MidiMapping", FIELDS, MidiMappingVisitor)
    }
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
    pub fn map_strip(&mut self, midi: MidiControl, target: StripTarget) {
        self.mappings.insert(midi, ControlTarget::Strip(target));
    }
    
    /// Add a mapping from MIDI CC to a global control
    pub fn map_global(&mut self, midi: MidiControl, target: GlobalControl) {
        self.mappings.insert(midi, ControlTarget::Global(target));
    }
    
    /// Set a custom value range for a MIDI control
    pub fn set_value_range(&mut self, midi: MidiControl, range: ValueRange) {
        self.value_ranges.insert(midi, range);
    }
    
    /// Get the target for a MIDI control
    pub fn get_target(&self, midi: &MidiControl) -> Option<&ControlTarget> {
        self.mappings.get(midi)
    }
    
    /// Transform MIDI value (0-127) to target range
    pub fn transform_value(&self, midi: &MidiControl, midi_value: u8) -> f64 {
        if let Some(range) = self.value_ranges.get(midi) {
            range.transform(midi_value)
        } else {
            // Default: map 0-127 to 0.0-1.0
            midi_value as f64 / 127.0
        }
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
            );
            
            // Set fader range: MIDI 0-127 -> -96.0 to +10.0 dB
            mapping.set_value_range(
                MidiControl { channel: 0, cc: i + 1 },
                ValueRange {
                    midi_min: 0,
                    midi_max: 127,
                    target_min: -96.0,
                    target_max: 10.0,
                    curve: Curve::Linear,
                },
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
            );
            
            // Set balance range: MIDI 0-127 -> -100.0 to +100.0
            mapping.set_value_range(
                MidiControl { channel: 0, cc: i + 10 },
                ValueRange {
                    midi_min: 0,
                    midi_max: 127,
                    target_min: -100.0,
                    target_max: 100.0,
                    curve: Curve::Linear,
                },
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