//! Declarative device profiles: a controller's input bindings (and, later, LED
//! feedback) described in RON and loaded at runtime, so adding a device is
//! writing a file, not code. Pure — no MIDI I/O, no engine dependency.

use crate::Target;
use serde::Deserialize;

/// How a relative (endless-encoder) CC encodes its signed delta. The DDJ-FLX4
/// alone uses both: its jog is centred at 64 (`0x41` = +1), its browse encoder
/// at 0 (`0x01` = +1, `0x7F` = −1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum RelKind {
    /// Signed offset from 64: `value - 64`.
    Centre64,
    /// Two's-complement around 0: `1..=63` = +, `0x7F..=0x41` = − (`value` or `value - 128`).
    Centre0,
}

impl RelKind {
    /// Decode a 7-bit relative value into a signed tick delta.
    pub fn delta(self, value: u8) -> i32 {
        match self {
            RelKind::Centre64 => value as i32 - 64,
            RelKind::Centre0 => {
                if value < 64 {
                    value as i32
                } else {
                    value as i32 - 128
                }
            }
        }
    }
}

/// One input control → a [`Target`]. Concrete MIDI address (no deck templating):
/// `status` is the channel-voice status byte (`0x90` note / `0xB0` CC), `data1`
/// the note or controller number.
#[derive(Debug, Clone, Deserialize)]
pub struct InputBinding {
    pub status: u8,
    pub data1: u8,
    pub target: Target,
    /// 14-bit hi-res: the LSB arrives on controller `data1 + 0x20`.
    #[serde(default)]
    pub hires: bool,
    /// Set for endless encoders (jog, browse) — how to read the signed delta.
    #[serde(default)]
    pub rel: Option<RelKind>,
}

/// A controller mapping loaded from RON.
#[derive(Debug, Clone, Deserialize)]
pub struct Profile {
    pub name: String,
    /// Case-insensitive substring matched against a MIDI input port name.
    pub port_match: String,
    /// Bytes sent once on connect to enable the device (e.g. the FLX4 enable
    /// SysEx — quiets its idle stream and unlocks its LEDs).
    #[serde(default)]
    pub init: Vec<Vec<u8>>,
    #[serde(default)]
    pub inputs: Vec<InputBinding>,
}

impl Profile {
    /// Parse a profile from RON source.
    pub fn from_ron(src: &str) -> Result<Self, ron::error::SpannedError> {
        ron::from_str(src)
    }

    /// Whether this profile should drive a MIDI port with the given name.
    pub fn matches_port(&self, port_name: &str) -> bool {
        port_name
            .to_ascii_lowercase()
            .contains(&self.port_match.to_ascii_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Deck;

    const SAMPLE: &str = r#"
        Profile(
            name: "Pioneer DDJ-FLX4",
            port_match: "ddj-flx4",
            init: [[0xF0, 0x00, 0x40, 0xF7]],
            inputs: [
                InputBinding(status: 0x90, data1: 0x0B, target: Play(A)),
                InputBinding(status: 0xB0, data1: 0x07, target: EqHigh(A), hires: true),
                InputBinding(status: 0xB0, data1: 0x22, target: Seek(A), rel: Some(Centre64)),
            ],
        )
    "#;

    #[test]
    fn parses_meta_init_and_bindings() {
        let p = Profile::from_ron(SAMPLE).expect("valid RON");
        assert_eq!(p.name, "Pioneer DDJ-FLX4");
        assert_eq!(p.init, vec![vec![0xF0, 0x00, 0x40, 0xF7]]);
        assert_eq!(p.inputs.len(), 3);
        assert_eq!(p.inputs[0].target, Target::Play(Deck::A));
        assert!(p.inputs[1].hires);
        assert_eq!(p.inputs[2].rel, Some(RelKind::Centre64));
    }

    #[test]
    fn port_match_is_case_insensitive_substring() {
        let p = Profile::from_ron(SAMPLE).unwrap();
        assert!(p.matches_port("DDJ-FLX4"));
        assert!(p.matches_port("Pioneer DDJ-FLX4 MIDI 1"));
        assert!(!p.matches_port("Numark Mixtrack"));
    }

    #[test]
    fn relative_encodings_decode_signed_deltas() {
        assert_eq!(RelKind::Centre64.delta(0x41), 1);
        assert_eq!(RelKind::Centre64.delta(0x3F), -1);
        assert_eq!(RelKind::Centre0.delta(0x01), 1);
        assert_eq!(RelKind::Centre0.delta(0x7F), -1);
    }
}
