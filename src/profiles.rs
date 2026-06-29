//! Built-in device profiles bundled with the crate, so a supported controller
//! works the moment it is plugged in — no learn step, no config. Each profile's
//! RON is embedded at compile time; [`builtin_for_port`] returns the first whose
//! `port_match` claims a given MIDI input port name.

use crate::Profile;

/// The Pioneer DDJ-FLX4 input profile RON, embedded at build time.
pub const PIONEER_DDJ_FLX4: &str = include_str!("../profiles/pioneer-ddj-flx4.ron");

/// All bundled profile sources, in match-priority order.
const BUILTINS: &[&str] = &[PIONEER_DDJ_FLX4];

/// Parse and return the first built-in profile whose `port_match` matches the
/// given MIDI input port name (case-insensitive substring). `None` if no
/// built-in claims the port. A built-in that fails to parse (e.g. it references
/// a feature-gated target absent from this build) is skipped, not matched.
pub fn builtin_for_port(port_name: &str) -> Option<Profile> {
    BUILTINS.iter().find_map(|src| {
        let p = Profile::from_ron(src).ok()?;
        p.matches_port(port_name).then_some(p)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ActionValue, Deck, MidiMessage, Target};

    /// Schema-drift guard: every bundled profile must parse in this build's
    /// feature config. (Fails the moment a profile uses a target this build
    /// doesn't have, or the RON schema diverges from `Profile`.)
    #[test]
    fn every_builtin_parses() {
        for src in BUILTINS {
            Profile::from_ron(src).expect("bundled profile must parse");
        }
    }

    #[test]
    fn flx4_matches_real_port_names_and_rejects_others() {
        assert!(builtin_for_port("DDJ-FLX4").is_some());
        assert!(builtin_for_port("Pioneer DDJ-FLX4 MIDI 1").is_some());
        assert!(builtin_for_port("ddj-flx4").is_some());
        assert!(builtin_for_port("Numark Mixtrack Pro").is_none());
        assert!(builtin_for_port("").is_none());
    }

    #[test]
    fn flx4_sends_the_enable_sysex_on_connect() {
        let p = builtin_for_port("DDJ-FLX4").unwrap();
        assert_eq!(
            p.init,
            vec![vec![
                0xF0, 0x00, 0x40, 0x05, 0x00, 0x00, 0x04, 0x05, 0x00, 0x50, 0x02, 0xF7
            ]]
        );
    }

    #[test]
    fn flx4_decodes_both_decks() {
        let p = builtin_for_port("DDJ-FLX4").unwrap();
        // Deck A play (note 0x0B on channel 0).
        let a = p
            .decode(&MidiMessage::NoteOn {
                channel: 0,
                note: 0x0B,
                velocity: 127,
            })
            .unwrap();
        assert_eq!(a.target, Target::Play(Deck::A));
        assert_eq!(a.value, ActionValue::Absolute(1.0));
        // Deck B play (note 0x0B on channel 1).
        let b = p
            .decode(&MidiMessage::NoteOn {
                channel: 1,
                note: 0x0B,
                velocity: 127,
            })
            .unwrap();
        assert_eq!(b.target, Target::Play(Deck::B));
    }

    #[test]
    fn flx4_decodes_eq_knob_and_jog_tick() {
        let p = builtin_for_port("DDJ-FLX4").unwrap();
        // EQ-high knob (CC 0x07 on channel 0) → absolute.
        let eq = p
            .decode(&MidiMessage::ControlChange {
                channel: 0,
                controller: 0x07,
                value: 127,
            })
            .unwrap();
        assert_eq!(eq.target, Target::EqHigh(Deck::A));
        assert_eq!(eq.value, ActionValue::Absolute(1.0));
        // Jog scratch (CC 0x22, centre-64 relative) → +1 tick.
        let jog = p
            .decode(&MidiMessage::ControlChange {
                channel: 0,
                controller: 0x22,
                value: 0x41,
            })
            .unwrap();
        assert_eq!(jog.target, Target::JogScratch(Deck::A));
        assert_eq!(jog.value, ActionValue::Delta(1));
    }

    #[test]
    fn flx4_decodes_stem_pads() {
        let p = builtin_for_port("DDJ-FLX4").unwrap();
        // Sampler-mode pad 0x32 on pad channel 0x97 → mute stem 2, deck A.
        let mute = p
            .decode(&MidiMessage::NoteOn {
                channel: 7,
                note: 0x32,
                velocity: 127,
            })
            .unwrap();
        assert_eq!(mute.target, Target::StemMute(Deck::A, 2));
        // Bottom-row pad 0x34 → solo stem 0, deck A.
        let solo = p
            .decode(&MidiMessage::NoteOn {
                channel: 7,
                note: 0x34,
                velocity: 127,
            })
            .unwrap();
        assert_eq!(solo.target, Target::StemSolo(Deck::A, 0));
    }
}
