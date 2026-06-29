//! Declarative device profiles: a controller's input bindings (and, later, LED
//! feedback) described in RON and loaded at runtime, so adding a device is
//! writing a file, not code. Pure — no MIDI I/O, no engine dependency.

use crate::feedback::{self, FeedbackRule, FeedbackState};
use crate::{Kind, MidiMessage, Target};
use serde::Deserialize;
use std::collections::HashMap;

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
    /// LED/VU feedback rules — which engine value lights which control. Empty
    /// for input-only profiles.
    #[serde(default)]
    pub feedback: Vec<FeedbackRule>,
}

/// The decoded result of one input message: which [`Target`], and the value to
/// apply.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProfileAction {
    pub target: Target,
    pub value: ActionValue,
}

/// How a control's value should be applied. Buttons/knobs/faders carry an
/// absolute `0..1`; endless encoders (jog, browse) carry a signed tick delta.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ActionValue {
    /// Absolute parameter in `0..=1` (button on/off, knob, fader).
    Absolute(f32),
    /// Signed tick delta from a relative encoder (jog scrub, browse).
    Delta(i32),
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

    /// Render this profile's feedback rules against the given engine state into
    /// the 3-byte MIDI messages to send (one per rule). The app diffs successive
    /// frames (via [`feedback::FeedbackDiff`]) and owns the MIDI output.
    pub fn render_feedback(&self, state: &FeedbackState) -> Vec<[u8; 3]> {
        feedback::render(&self.feedback, state)
    }

    /// Resolve an incoming MIDI message to an action via this profile's bindings.
    /// Note on/off → absolute 1.0/0.0; a relative-flagged CC → a signed delta;
    /// any other CC → absolute `value/127`. `None` if nothing binds the control.
    ///
    /// (14-bit hi-res currently resolves at 7-bit from the MSB; the LSB CC simply
    /// has no binding and is ignored — true 14-bit accumulation lands next.)
    pub fn decode(&self, msg: &MidiMessage) -> Option<ProfileAction> {
        // Reconstruct the (status, data1) the binding is written against.
        let (status, data1, cc): (u8, u8, Option<u8>) = match *msg {
            MidiMessage::NoteOn { channel, note, .. } => (0x90 | channel, note, None),
            MidiMessage::NoteOff { channel, note } => (0x90 | channel, note, None),
            MidiMessage::ControlChange {
                channel,
                controller,
                value,
            } => (0xB0 | channel, controller, Some(value)),
            MidiMessage::Other => return None,
        };
        let b = self
            .inputs
            .iter()
            .find(|b| b.status == status && b.data1 == data1)?;
        let value = match (cc, b.rel) {
            // CC with a relative encoding → signed delta.
            (Some(v), Some(kind)) => ActionValue::Delta(kind.delta(v)),
            // Absolute CC → 0..1.
            (Some(v), None) => ActionValue::Absolute(v as f32 / 127.0),
            // Note: on → 1.0, off → 0.0 (the parser maps velocity 0 to NoteOff).
            (None, _) => {
                ActionValue::Absolute(matches!(msg, MidiMessage::NoteOn { .. }) as u8 as f32)
            }
        };
        Some(ProfileAction {
            target: b.target,
            value,
        })
    }
}

/// Reconstruct the `(status, data1)` address a binding is written against.
fn msg_addr(msg: &MidiMessage) -> Option<(u8, u8)> {
    match *msg {
        MidiMessage::NoteOn { channel, note, .. } | MidiMessage::NoteOff { channel, note } => {
            Some((0x90 | channel, note))
        }
        MidiMessage::ControlChange {
            channel,
            controller,
            ..
        } => Some((0xB0 | channel, controller)),
        MidiMessage::Other => None,
    }
}

/// Per-button runtime state for the stateful decoder.
#[derive(Default)]
struct ButtonState {
    /// Whether the button is currently held (for press-edge detection).
    down: bool,
    /// Latched on/off state for Toggle targets.
    toggle_on: bool,
}

/// A stateful wrapper over a [`Profile`] that applies the semantics a momentary
/// controller can't express on its own, keyed off each target's [`Kind`]:
///
/// - **Toggle** (Play, stem mute/solo, cue): flips on each *press* and ignores
///   the release — so a button tap latches instead of holding-to-engage.
/// - **Trigger** (hot cues, loops, load): fires once on the press edge.
/// - **Continuous** (knobs, faders, jog/encoders): passes straight through.
///
/// Without this, [`Profile::decode`]'s raw note-on→1.0 / note-off→0.0 makes every
/// toggle behave as hold-to-engage. One decoder per connected device.
pub struct ProfileDecoder {
    profile: Profile,
    buttons: HashMap<(u8, u8), ButtonState>,
}

impl ProfileDecoder {
    /// Wrap a profile with fresh (all-off) button state.
    pub fn new(profile: Profile) -> Self {
        Self {
            profile,
            buttons: HashMap::new(),
        }
    }

    /// The wrapped profile (for feedback rendering, `init` bytes, port match).
    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    /// Decode one message, applying Toggle/Trigger/Continuous semantics.
    pub fn decode(&mut self, msg: &MidiMessage) -> Option<ProfileAction> {
        let raw = self.profile.decode(msg)?;
        let kind = raw.target.kind();
        if kind == Kind::Continuous {
            return Some(raw); // knobs/faders/jog/encoders pass through
        }
        // Toggle / Trigger: act on the press edge only.
        let addr = msg_addr(msg)?;
        let pressed = matches!(raw.value, ActionValue::Absolute(v) if v >= 0.5);
        let st = self.buttons.entry(addr).or_default();
        let rising = pressed && !st.down;
        st.down = pressed;
        if !rising {
            return None; // release or repeat — nothing to emit
        }
        let value = if kind == Kind::Toggle {
            st.toggle_on = !st.toggle_on;
            if st.toggle_on {
                1.0
            } else {
                0.0
            }
        } else {
            1.0 // Trigger
        };
        Some(ProfileAction {
            target: raw.target,
            value: ActionValue::Absolute(value),
        })
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
    fn decode_button_press_and_release() {
        let p = Profile::from_ron(SAMPLE).unwrap();
        let on = p
            .decode(&MidiMessage::NoteOn {
                channel: 0,
                note: 0x0B,
                velocity: 127,
            })
            .unwrap();
        assert_eq!(on.target, Target::Play(Deck::A));
        assert_eq!(on.value, ActionValue::Absolute(1.0));
        let off = p
            .decode(&MidiMessage::NoteOff {
                channel: 0,
                note: 0x0B,
            })
            .unwrap();
        assert_eq!(off.value, ActionValue::Absolute(0.0));
    }

    #[test]
    fn decode_absolute_cc_to_0_1() {
        let p = Profile::from_ron(SAMPLE).unwrap();
        let a = p
            .decode(&MidiMessage::ControlChange {
                channel: 0,
                controller: 0x07,
                value: 127,
            })
            .unwrap();
        assert_eq!(a.target, Target::EqHigh(Deck::A));
        assert_eq!(a.value, ActionValue::Absolute(1.0));
    }

    #[test]
    fn decode_relative_cc_to_signed_delta() {
        let p = Profile::from_ron(SAMPLE).unwrap();
        let up = p
            .decode(&MidiMessage::ControlChange {
                channel: 0,
                controller: 0x22,
                value: 0x41,
            })
            .unwrap();
        assert_eq!(up.target, Target::Seek(Deck::A));
        assert_eq!(up.value, ActionValue::Delta(1));
        let down = p
            .decode(&MidiMessage::ControlChange {
                channel: 0,
                controller: 0x22,
                value: 0x3F,
            })
            .unwrap();
        assert_eq!(down.value, ActionValue::Delta(-1));
    }

    #[test]
    fn decode_unbound_control_is_none() {
        let p = Profile::from_ron(SAMPLE).unwrap();
        assert!(p
            .decode(&MidiMessage::ControlChange {
                channel: 0,
                controller: 0x7E,
                value: 0,
            })
            .is_none());
    }

    #[test]
    fn relative_encodings_decode_signed_deltas() {
        assert_eq!(RelKind::Centre64.delta(0x41), 1);
        assert_eq!(RelKind::Centre64.delta(0x3F), -1);
        assert_eq!(RelKind::Centre0.delta(0x01), 1);
        assert_eq!(RelKind::Centre0.delta(0x7F), -1);
    }

    const DECODER_SAMPLE: &str = r#"
        Profile(
            name: "Test",
            port_match: "test",
            inputs: [
                InputBinding(status: 0x90, data1: 0x0B, target: Play(A)),       // Toggle
                InputBinding(status: 0x97, data1: 0x00, target: HotCue(A, 0)),  // Trigger
                InputBinding(status: 0xB0, data1: 0x07, target: EqHigh(A)),     // Continuous
                InputBinding(status: 0xB0, data1: 0x22, target: Seek(A), rel: Some(Centre64)),
            ],
        )
    "#;

    fn note_on(status: u8, note: u8) -> MidiMessage {
        MidiMessage::NoteOn {
            channel: status & 0x0F,
            note,
            velocity: 127,
        }
    }
    fn note_off(status: u8, note: u8) -> MidiMessage {
        MidiMessage::NoteOff {
            channel: status & 0x0F,
            note,
        }
    }

    #[test]
    fn decoder_toggle_flips_on_press_and_ignores_release() {
        let mut d = ProfileDecoder::new(Profile::from_ron(DECODER_SAMPLE).unwrap());
        // First press → on.
        let a = d.decode(&note_on(0x90, 0x0B)).unwrap();
        assert_eq!(a.target, Target::Play(Deck::A));
        assert_eq!(a.value, ActionValue::Absolute(1.0));
        // Release → nothing.
        assert!(d.decode(&note_off(0x90, 0x0B)).is_none());
        // Second press → off.
        assert_eq!(
            d.decode(&note_on(0x90, 0x0B)).unwrap().value,
            ActionValue::Absolute(0.0)
        );
        // Its release → nothing; third press → on again.
        assert!(d.decode(&note_off(0x90, 0x0B)).is_none());
        assert_eq!(
            d.decode(&note_on(0x90, 0x0B)).unwrap().value,
            ActionValue::Absolute(1.0)
        );
    }

    #[test]
    fn decoder_trigger_fires_on_press_only() {
        let mut d = ProfileDecoder::new(Profile::from_ron(DECODER_SAMPLE).unwrap());
        let a = d.decode(&note_on(0x97, 0x00)).unwrap();
        assert_eq!(a.target, Target::HotCue(Deck::A, 0));
        assert_eq!(a.value, ActionValue::Absolute(1.0));
        // Release emits nothing; a second press fires again (1.0, not toggled off).
        assert!(d.decode(&note_off(0x97, 0x00)).is_none());
        assert_eq!(
            d.decode(&note_on(0x97, 0x00)).unwrap().value,
            ActionValue::Absolute(1.0)
        );
    }

    #[test]
    fn decoder_continuous_passes_through() {
        let mut d = ProfileDecoder::new(Profile::from_ron(DECODER_SAMPLE).unwrap());
        // Absolute knob.
        assert_eq!(
            d.decode(&MidiMessage::ControlChange {
                channel: 0,
                controller: 0x07,
                value: 127,
            })
            .unwrap()
            .value,
            ActionValue::Absolute(1.0)
        );
        // Relative encoder → signed delta, unchanged by the stateful layer.
        assert_eq!(
            d.decode(&MidiMessage::ControlChange {
                channel: 0,
                controller: 0x22,
                value: 0x41,
            })
            .unwrap()
            .value,
            ActionValue::Delta(1)
        );
    }
}
