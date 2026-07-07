//! Declarative device profiles: a controller's input bindings (and, later, LED
//! feedback) described in RON and loaded at runtime, so adding a device is
//! writing a file, not code. Pure — no MIDI I/O, no engine dependency.

use crate::feedback::{self, FeedbackRule, FeedbackState};
use crate::{Deck, Kind, MidiMessage, Target};
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
        // Data bytes are 7-bit; mask a stray high bit so a malformed byte
        // can't produce an outsized delta (mirrors `decode_relative`).
        let value = value & 0x7F;
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

/// How this device announces the active bank over SysEx. `prefix` is matched at
/// the start of the SysEx; the byte at `bank_index` is the (0-based) bank number.
#[derive(Debug, Clone, Deserialize)]
pub struct BankSelect {
    pub prefix: Vec<u8>,
    pub bank_index: usize,
}

/// One SysEx input binding. Two kinds:
/// - **Value** (`value_index: Some(i)`): byte at `i` is a 0–127 position →
///   `ActionValue::Absolute(byte / 127.0)`. Used for the EasyControl.9 crossfader.
/// - **Match** (`match_index: Some(i)` + `match_value: Some(v)`): if the byte at
///   `i` equals `v`, emits `ActionValue::Absolute(1.0)`. Used for MMC transport
///   buttons.
///
/// In both cases `prefix` must match the start of the incoming SysEx bytes, and
/// all indices are guarded against `bytes.len()`.
#[derive(Debug, Clone, Deserialize)]
pub struct SysExBinding {
    pub prefix: Vec<u8>,
    /// Continuous: byte at this index → `Absolute(byte / 127.0)`.
    #[serde(default)]
    pub value_index: Option<usize>,
    /// Trigger: byte at this index must equal `match_value` → `Absolute(1.0)`.
    #[serde(default)]
    pub match_index: Option<usize>,
    #[serde(default)]
    pub match_value: Option<u8>,
    pub target: Target,
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
    /// Bank this binding is active in (`None` = all banks). Devices with hardware
    /// banks (Worlde EasyControl) send a bank-select SysEx; the decoder tracks the
    /// active bank and only matches bindings whose `bank` is `None` or equal.
    #[serde(default)]
    pub bank: Option<u8>,
    /// Per-binding kind override. When set, this replaces the target's natural
    /// [`Kind`] (from [`Target::kind`]) for decoder semantics. Use `Some(Continuous)`
    /// for hardware-latching CC buttons (e.g. EasyControl.9 Banks 3/4 strip buttons)
    /// so their raw 127/0 passes straight through instead of being treated as
    /// momentary/toggle presses.
    #[serde(default)]
    pub mode: Option<Kind>,
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
    /// Match CC/note by message kind + number, ignoring the channel nibble. For
    /// devices that shift channel per bank (EasyControl.9 Bank 2 = ch 10).
    #[serde(default)]
    pub channel_agnostic: bool,
    /// Bank-select SysEx pattern (drives the decoder's current bank). None = no banks.
    #[serde(default)]
    pub bank_select: Option<BankSelect>,
    /// SysEx input bindings: crossfader (value) and MMC transport (match).
    #[serde(default)]
    pub sysex: Vec<SysExBinding>,
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

    /// Resolve an incoming MIDI message to an action via this profile's bindings,
    /// at a specific active bank. Note on/off → absolute 1.0/0.0; a relative-flagged
    /// CC → a signed delta; any other CC → absolute `value/127`. `None` if nothing
    /// binds the control at this bank.
    ///
    /// Bindings with `bank: None` match in every bank. If `channel_agnostic` is set,
    /// only the status kind (`0xF0` nibble) and `data1` must match — the channel
    /// nibble is ignored.
    ///
    /// (14-bit hi-res currently resolves at 7-bit from the MSB; the LSB CC simply
    /// has no binding and is ignored — true 14-bit accumulation lands next.)
    pub fn decode_at(&self, msg: &MidiMessage, bank: u8) -> Option<ProfileAction> {
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
        let is_match = |b: &InputBinding| {
            let addr_ok = if self.channel_agnostic {
                (b.status & 0xF0) == (status & 0xF0) && b.data1 == data1
            } else {
                b.status == status && b.data1 == data1
            };
            addr_ok && (b.bank.is_none() || b.bank == Some(bank))
        };
        let b = self.inputs.iter().find(|b| is_match(b))?;
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

    /// Resolve an incoming MIDI message at bank 0 (backwards-compatible convenience
    /// wrapper around [`decode_at`](Self::decode_at)).
    pub fn decode(&self, msg: &MidiMessage) -> Option<ProfileAction> {
        self.decode_at(msg, 0)
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

/// Which loop boundary a held IN/OUT ADJ button is adjusting.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LoopEdge {
    In,
    Out,
}

/// If `t` is a loop-adjust hold, the deck + which boundary it arms.
fn loop_adjust_edge(t: Target) -> Option<(Deck, LoopEdge)> {
    match t {
        Target::LoopInAdj(d) => Some((d, LoopEdge::In)),
        Target::LoopOutAdj(d) => Some((d, LoopEdge::Out)),
        _ => None,
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
    /// Which loop boundary each deck is jog-adjusting (IN/OUT ADJ held).
    adjust: HashMap<Deck, LoopEdge>,
    /// Jog ticks accumulated toward the next quarter-beat nudge step, per deck.
    nudge_accum: HashMap<Deck, i32>,
    /// Active bank index (0-based), updated by bank-select SysEx via `decode_bytes`.
    current_bank: u8,
    /// Last Program Change value seen (for absolute→relative conversion).
    last_program: Option<u8>,
}

impl ProfileDecoder {
    /// Wrap a profile with fresh (all-off) button state.
    pub fn new(profile: Profile) -> Self {
        Self {
            profile,
            buttons: HashMap::new(),
            adjust: HashMap::new(),
            nudge_accum: HashMap::new(),
            current_bank: 0,
            last_program: None,
        }
    }

    /// The wrapped profile (for feedback rendering, `init` bytes, port match).
    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    /// Decode raw MIDI bytes: recognise the bank-select SysEx (updates the current
    /// bank, emits nothing) and otherwise fall back to channel-voice decoding at
    /// the current bank. (A2 adds Program Change; A3 adds the other SysEx.)
    pub fn decode_bytes(&mut self, bytes: &[u8]) -> Option<ProfileAction> {
        if bytes.first() == Some(&0xF0) {
            // Check for bank-select SysEx; extract the new bank before mutating.
            let new_bank = self.profile.bank_select.as_ref().and_then(|bs| {
                if bytes.len() > bs.bank_index && bytes.starts_with(&bs.prefix) {
                    Some(bytes[bs.bank_index])
                } else {
                    None
                }
            });
            if let Some(b) = new_bank {
                self.current_bank = b;
                return None;
            }
            // SysEx input bindings: crossfader (value) and MMC transport (match).
            for s in &self.profile.sysex {
                if !bytes.starts_with(&s.prefix) {
                    continue;
                }
                if let Some(i) = s.value_index {
                    if i < bytes.len() {
                        return Some(ProfileAction {
                            target: s.target,
                            value: ActionValue::Absolute(bytes[i] as f32 / 127.0),
                        });
                    }
                } else if let (Some(i), Some(v)) = (s.match_index, s.match_value) {
                    if i < bytes.len() && bytes[i] == v {
                        return Some(ProfileAction {
                            target: s.target,
                            value: ActionValue::Absolute(1.0),
                        });
                    }
                }
            }
            return None;
        }
        if let Some(&s) = bytes.first() {
            if s & 0xF0 == 0xC0 {
                let program = bytes.get(1).copied().unwrap_or(0) & 0x7F;
                // Find a PC binding (status 0xC0) for the current bank.
                let b = self.profile.inputs.iter().find(|b| {
                    (b.status & 0xF0) == 0xC0
                        && (b.bank.is_none() || b.bank == Some(self.current_bank))
                })?;
                let target = b.target;
                let delta = match self.last_program.replace(program) {
                    None => return None, // baseline, no jump
                    Some(prev) => program as i32 - prev as i32,
                };
                if delta == 0 {
                    return None;
                }
                return Some(ProfileAction {
                    target,
                    value: ActionValue::Delta(delta),
                });
            }
        }
        let msg = crate::parse(bytes)?;
        self.decode_msg(&msg)
    }

    /// Decode one message, applying Toggle/Trigger/Continuous semantics.
    pub fn decode(&mut self, msg: &MidiMessage) -> Option<ProfileAction> {
        self.decode_msg(msg)
    }

    /// Resolve the effective [`Kind`] for a message at the given bank: returns the
    /// binding's `mode` override if set, otherwise `fallback` (the target's natural
    /// kind). Used to implement per-binding mode overrides for hardware-latching CC
    /// buttons (e.g. EasyControl.9 Banks 3/4 strip buttons with `mode: Some(Continuous)`).
    fn effective_kind(&self, msg: &MidiMessage, bank: u8, fallback: Kind) -> Kind {
        let Some((status, data1)) = msg_addr(msg) else {
            return fallback;
        };
        self.profile
            .inputs
            .iter()
            .find(|b| {
                let addr_ok = if self.profile.channel_agnostic {
                    (b.status & 0xF0) == (status & 0xF0) && b.data1 == data1
                } else {
                    b.status == status && b.data1 == data1
                };
                addr_ok && (b.bank.is_none() || b.bank == Some(bank))
            })
            .and_then(|b| b.mode)
            .unwrap_or(fallback)
    }

    /// Internal: apply Toggle/Trigger/Continuous semantics at the current bank.
    fn decode_msg(&mut self, msg: &MidiMessage) -> Option<ProfileAction> {
        let raw = self.profile.decode_at(msg, self.current_bank)?;
        // Loop-point adjust: holding IN/OUT ADJ arms this deck's jog to nudge a
        // loop boundary. Swallow the hold itself (the host never sees it).
        if let Some((deck, edge)) = loop_adjust_edge(raw.target) {
            let held = matches!(raw.value, ActionValue::Absolute(v) if v >= 0.5);
            if held {
                self.adjust.insert(deck, edge);
            } else {
                self.adjust.remove(&deck);
                self.nudge_accum.remove(&deck);
            }
            return None;
        }
        // While a deck's ADJ is held, its jog-bend becomes an accumulated
        // quarter-beat nudge of the armed loop boundary.
        if let Target::JogBend(deck) = raw.target {
            if let (Some(&edge), ActionValue::Delta(ticks)) = (self.adjust.get(&deck), raw.value) {
                return self.accumulate_nudge(deck, edge, ticks);
            }
        }
        // Effective kind: binding's mode override if set, else the target's natural kind.
        // This lets hardware-latching CC buttons (e.g. EasyControl.9 Banks 3/4) use
        // `mode: Some(Continuous)` so their 127/0 values pass through directly.
        let kind = self.effective_kind(msg, self.current_bank, raw.target.kind());
        if kind == Kind::Continuous {
            return Some(raw); // knobs/faders/jog/encoders pass through
        }
        // Toggle / Trigger act on the press edge only; Momentary acts on both.
        let addr = msg_addr(msg)?;
        let pressed = matches!(raw.value, ActionValue::Absolute(v) if v >= 0.5);
        let st = self.buttons.entry(addr).or_default();
        let rising = pressed && !st.down;
        let falling = !pressed && st.down;
        st.down = pressed;
        let value = match kind {
            // Hold-to-play: emit on press (1.0) and release (0.0), ignore repeats.
            Kind::Momentary => {
                if rising {
                    1.0
                } else if falling {
                    0.0
                } else {
                    return None;
                }
            }
            Kind::Toggle => {
                if !rising {
                    return None; // release or repeat — nothing to emit
                }
                st.toggle_on = !st.toggle_on;
                if st.toggle_on {
                    1.0
                } else {
                    0.0
                }
            }
            // Trigger (Continuous already returned above).
            _ => {
                if !rising {
                    return None;
                }
                1.0
            }
        };
        Some(ProfileAction {
            target: raw.target,
            value: ActionValue::Absolute(value),
        })
    }

    /// Fold jog ticks into whole quarter-beat steps; emit a nudge when the
    /// accumulator crosses the tick threshold, else `None`.
    fn accumulate_nudge(
        &mut self,
        deck: Deck,
        edge: LoopEdge,
        ticks: i32,
    ) -> Option<ProfileAction> {
        let accum = self.nudge_accum.entry(deck).or_default();
        *accum += ticks;
        let mut steps = 0;
        while *accum >= crate::jog::JOG_TICKS_PER_NUDGE {
            *accum -= crate::jog::JOG_TICKS_PER_NUDGE;
            steps += 1;
        }
        while *accum <= -crate::jog::JOG_TICKS_PER_NUDGE {
            *accum += crate::jog::JOG_TICKS_PER_NUDGE;
            steps -= 1;
        }
        if steps == 0 {
            return None;
        }
        let target = match edge {
            LoopEdge::In => Target::LoopNudgeIn(deck),
            LoopEdge::Out => Target::LoopNudgeOut(deck),
        };
        Some(ProfileAction {
            target,
            value: ActionValue::Delta(steps),
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
        // A stray high bit in the value byte is masked, not amplified.
        assert_eq!(RelKind::Centre64.delta(0xC1), 1);
        assert_eq!(RelKind::Centre0.delta(0xFF), -1);
    }

    #[test]
    fn decoder_roll_and_brake_are_momentary() {
        let p = Profile::from_ron(
            r#"Profile(
                name: "t", port_match: "t",
                inputs: [
                    InputBinding(status: 0x97, data1: 0x10, target: BeatRepeatRoll(A, 1.0)),
                    InputBinding(status: 0x97, data1: 0x13, target: VinylBrake(A)),
                ],
            )"#,
        )
        .unwrap();
        let mut d = ProfileDecoder::new(p);
        for note in [0x10u8, 0x13] {
            let on = MidiMessage::NoteOn {
                channel: 7,
                note,
                velocity: 127,
            };
            let off = MidiMessage::NoteOff { channel: 7, note };
            assert_eq!(d.decode(&on).unwrap().value, ActionValue::Absolute(1.0));
            // Release must stop the roll / spin the brake back up. As a Toggle
            // this edge was dropped and the effect latched until a second press.
            assert_eq!(d.decode(&off).unwrap().value, ActionValue::Absolute(0.0));
        }
    }

    const DECODER_SAMPLE: &str = r#"
        Profile(
            name: "Test",
            port_match: "test",
            inputs: [
                InputBinding(status: 0x90, data1: 0x0B, target: Play(A)),       // Toggle
                InputBinding(status: 0x97, data1: 0x00, target: HotCue(A, 0)),  // Trigger
                InputBinding(status: 0x97, data1: 0x01, target: HotCueHold(A, 1)), // Momentary
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
    fn decoder_momentary_emits_on_press_and_release() {
        let mut d = ProfileDecoder::new(Profile::from_ron(DECODER_SAMPLE).unwrap());
        // Press → 1.0 (play from cue).
        let press = d.decode(&note_on(0x97, 0x01)).unwrap();
        assert_eq!(press.target, Target::HotCueHold(Deck::A, 1));
        assert_eq!(press.value, ActionValue::Absolute(1.0));
        // Release → 0.0 (return to ghost) — unlike Trigger, which swallows it.
        let release = d.decode(&note_off(0x97, 0x01)).unwrap();
        assert_eq!(release.target, Target::HotCueHold(Deck::A, 1));
        assert_eq!(release.value, ActionValue::Absolute(0.0));
        // A repeated note-off (already up) emits nothing.
        assert!(d.decode(&note_off(0x97, 0x01)).is_none());
    }

    const NUDGE_SAMPLE: &str = r#"
        Profile(
            name: "Test", port_match: "test",
            inputs: [
                InputBinding(status: 0x90, data1: 0x4C, target: LoopInAdj(A)),
                InputBinding(status: 0x90, data1: 0x4E, target: LoopOutAdj(A)),
                InputBinding(status: 0x91, data1: 0x4C, target: LoopInAdj(B)),
                InputBinding(status: 0xB0, data1: 0x21, target: JogBend(A), rel: Some(Centre64)),
                InputBinding(status: 0xB1, data1: 0x21, target: JogBend(B), rel: Some(Centre64)),
            ],
        )
    "#;
    fn cc(status: u8, controller: u8, value: u8) -> MidiMessage {
        MidiMessage::ControlChange {
            channel: status & 0x0F,
            controller,
            value,
        }
    }

    #[test]
    fn jog_bend_passes_through_when_no_adjust_held() {
        let mut d = ProfileDecoder::new(Profile::from_ron(NUDGE_SAMPLE).unwrap());
        let a = d.decode(&cc(0xB0, 0x21, 0x41)).unwrap();
        assert_eq!(a.target, Target::JogBend(Deck::A));
        assert_eq!(a.value, ActionValue::Delta(1));
    }

    #[test]
    fn holding_in_adjust_reroutes_jog_to_accumulated_nudge() {
        use crate::jog::JOG_TICKS_PER_NUDGE;
        let mut d = ProfileDecoder::new(Profile::from_ron(NUDGE_SAMPLE).unwrap());
        assert!(
            d.decode(&note_on(0x90, 0x4C)).is_none(),
            "ADJ press swallowed"
        );
        for _ in 0..(JOG_TICKS_PER_NUDGE - 1) {
            assert!(
                d.decode(&cc(0xB0, 0x21, 0x41)).is_none(),
                "sub-threshold ticks accumulate silently"
            );
        }
        let step = d.decode(&cc(0xB0, 0x21, 0x41)).unwrap();
        assert_eq!(step.target, Target::LoopNudgeIn(Deck::A));
        assert_eq!(step.value, ActionValue::Delta(1));
        assert!(
            d.decode(&note_off(0x90, 0x4C)).is_none(),
            "ADJ release swallowed"
        );
        // Jog reverts to bending tempo once ADJ is released.
        assert_eq!(
            d.decode(&cc(0xB0, 0x21, 0x41)).unwrap().target,
            Target::JogBend(Deck::A)
        );
    }

    #[test]
    fn out_adjust_reroutes_negative_direction_and_is_deck_isolated() {
        use crate::jog::JOG_TICKS_PER_NUDGE;
        let mut d = ProfileDecoder::new(Profile::from_ron(NUDGE_SAMPLE).unwrap());
        d.decode(&note_on(0x90, 0x4E)); // hold OUT ADJ on deck A
        for _ in 0..(JOG_TICKS_PER_NUDGE - 1) {
            assert!(d.decode(&cc(0xB0, 0x21, 0x3F)).is_none());
        }
        let step = d.decode(&cc(0xB0, 0x21, 0x3F)).unwrap();
        assert_eq!(step.target, Target::LoopNudgeOut(Deck::A));
        assert_eq!(step.value, ActionValue::Delta(-1));
        // Deck B jog is unaffected — no ADJ held there.
        assert_eq!(
            d.decode(&cc(0xB1, 0x21, 0x41)).unwrap().target,
            Target::JogBend(Deck::B)
        );
    }

    // Bank-aware, channel-agnostic decode: the SAME CC resolves to a different
    // target per bank, and matches regardless of channel.
    const BANKED: &str = r#"
        Profile(
            name: "t", port_match: "t",
            channel_agnostic: true,
            bank_select: Some(BankSelect(prefix: [0xF0,0x42,0x40,0x00,0x01,0x04,0x00,0x5F,0x4F], bank_index: 9)),
            inputs: [
                InputBinding(status: 0xB0, data1: 0x00, target: ChannelVolume(A), bank: Some(0)),
                InputBinding(status: 0xB0, data1: 0x00, target: StemVolume(A, 0), bank: Some(1)),
                InputBinding(status: 0xB0, data1: 0x40, target: LoadDeck(A)),  // global (bank: None)
            ],
        )
    "#;

    #[test]
    fn bank_select_sysex_switches_resolution_channel_agnostic() {
        let mut d = ProfileDecoder::new(Profile::from_ron(BANKED).unwrap());
        // Default bank 0: CC0 (on ch 1) → ChannelVolume(A).
        assert_eq!(
            d.decode_bytes(&[0xB0, 0x00, 127]).unwrap().target,
            Target::ChannelVolume(Deck::A)
        );
        // Bank-select SysEx → bank 1; the SAME CC0, now on ch 10 (0xB9), → StemVolume(A,0).
        assert!(d
            .decode_bytes(&[0xF0, 0x42, 0x40, 0x00, 0x01, 0x04, 0x00, 0x5F, 0x4F, 0x01, 0xF7])
            .is_none());
        assert_eq!(
            d.decode_bytes(&[0xB9, 0x00, 127]).unwrap().target,
            Target::StemVolume(Deck::A, 0)
        );
        // Global binding (bank: None) resolves in any bank.
        assert_eq!(
            d.decode_bytes(&[0xB0, 0x40, 127]).unwrap().target,
            Target::LoadDeck(Deck::A)
        );
    }

    #[test]
    fn program_change_browse_becomes_relative_scroll() {
        let p = r#"Profile(name:"t", port_match:"t",
            inputs: [ InputBinding(status: 0xC0, data1: 0, target: LibraryScroll, rel: Some(Centre0)) ])"#;
        let mut d = ProfileDecoder::new(Profile::from_ron(p).unwrap());
        // First PC establishes a baseline (no jump) → None.
        assert!(d.decode_bytes(&[0xC0, 64]).is_none());
        // Next PC higher → positive scroll delta.
        assert_eq!(
            d.decode_bytes(&[0xC0, 67]).unwrap().value,
            ActionValue::Delta(3)
        );
        // Lower → negative.
        assert_eq!(
            d.decode_bytes(&[0xC0, 65]).unwrap().value,
            ActionValue::Delta(-2)
        );
    }

    #[test]
    fn sysex_value_and_match_bindings() {
        let p = r#"Profile(name:"t", port_match:"t",
            sysex: [
                SysExBinding(prefix: [0xF0,0x7F,0x7F,0x04,0x01,0x00], value_index: Some(6), target: Crossfade),
                SysExBinding(prefix: [0xF0,0x7F,0x7F,0x06], match_index: Some(4), match_value: Some(0x02), target: Play(A)),
            ])"#;
        let mut d = ProfileDecoder::new(Profile::from_ron(p).unwrap());
        // Crossfader position 0x40 → 64/127.
        let x = d
            .decode_bytes(&[0xF0, 0x7F, 0x7F, 0x04, 0x01, 0x00, 0x40, 0xF7])
            .unwrap();
        assert_eq!(x.target, Target::Crossfade);
        assert_eq!(x.value, ActionValue::Absolute(64.0 / 127.0));
        // Transport Play (MMC 0x02) → trigger 1.0.
        let play = d
            .decode_bytes(&[0xF0, 0x7F, 0x7F, 0x06, 0x02, 0xF7])
            .unwrap();
        assert_eq!(play.target, Target::Play(Deck::A));
        assert_eq!(play.value, ActionValue::Absolute(1.0));
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
