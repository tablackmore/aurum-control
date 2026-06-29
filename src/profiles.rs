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
/// built-in claims the port. A built-in that fails to parse is skipped, not
/// matched.
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
    fn flx4_decodes_library_navigation() {
        let p = builtin_for_port("DDJ-FLX4").unwrap();
        // Browse encoder (CC 0x40 ch 6, centre-0 relative) → scroll delta.
        let scroll = p
            .decode(&MidiMessage::ControlChange {
                channel: 6,
                controller: 0x40,
                value: 0x01,
            })
            .unwrap();
        assert_eq!(scroll.target, Target::LibraryScroll);
        assert_eq!(scroll.value, ActionValue::Delta(1));
        // Encoder press (note 0x41 ch 6) → open panel.
        let open = p
            .decode(&MidiMessage::NoteOn {
                channel: 6,
                note: 0x41,
                velocity: 127,
            })
            .unwrap();
        assert_eq!(open.target, Target::LibraryOpen);
        // Load buttons (notes 0x46/0x47 ch 6) → load deck A / B.
        let load_a = p
            .decode(&MidiMessage::NoteOn {
                channel: 6,
                note: 0x46,
                velocity: 127,
            })
            .unwrap();
        assert_eq!(load_a.target, Target::LoadDeck(Deck::A));
        let load_b = p
            .decode(&MidiMessage::NoteOn {
                channel: 6,
                note: 0x47,
                velocity: 127,
            })
            .unwrap();
        assert_eq!(load_b.target, Target::LoadDeck(Deck::B));
    }

    #[test]
    fn flx4_feedback_renders_vu_and_play_leds() {
        use crate::FeedbackState;
        let p = builtin_for_port("DDJ-FLX4").unwrap();
        let state = FeedbackState {
            deck_level: [1.0, 0.0],
            deck_playing: [true, false],
            master_level: 0.0,
        };
        let frame = p.render_feedback(&state);
        // Deck A VU full (B0 02 7F), deck B VU silent (B0 03 00),
        // deck A play LED on (90 0B 7F), deck B play LED off (91 0B 00).
        assert_eq!(frame[0], [0xB0, 0x02, 127]);
        assert_eq!(frame[1], [0xB0, 0x03, 0]);
        assert_eq!(frame[2], [0x90, 0x0B, 0x7F]);
        assert_eq!(frame[3], [0x91, 0x0B, 0x00]);
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

    /// Checks that the FLX4 profile carries the new FX/CFX bindings, confirming
    /// `every_builtin_parses` handles extended targets in the default feature config.
    #[test]
    fn flx4_includes_fx_and_cfx_bindings() {
        let p = builtin_for_port("DDJ-FLX4").unwrap();
        // CFX knob deck A → Filter(A) (confirmed address, ch 6 CC 0x17)
        let filter_a = p
            .decode(&MidiMessage::ControlChange {
                channel: 6,
                controller: 0x17,
                value: 127,
            })
            .unwrap();
        assert_eq!(filter_a.target, Target::Filter(Deck::A));
        // Pad FX 1 pad 0x10 deck A (ch 7 = 0x97) → BeatRepeatRoll(A, 1.0)
        let roll = p
            .decode(&MidiMessage::NoteOn {
                channel: 7,
                note: 0x10,
                velocity: 127,
            })
            .unwrap();
        assert_eq!(roll.target, Target::BeatRepeatRoll(Deck::A, 1.0));
        // Pad 0x13 deck A → VinylBrake(A)
        let brake = p
            .decode(&MidiMessage::NoteOn {
                channel: 7,
                note: 0x13,
                velocity: 127,
            })
            .unwrap();
        assert_eq!(brake.target, Target::VinylBrake(Deck::A));
        // Pad 0x14 deck A → Riser(A)
        let riser = p
            .decode(&MidiMessage::NoteOn {
                channel: 7,
                note: 0x14,
                velocity: 127,
            })
            .unwrap();
        assert_eq!(riser.target, Target::Riser(Deck::A));
        // Pad 0x15 deck A → FxSlotEnable(A, 0)
        let fx1 = p
            .decode(&MidiMessage::NoteOn {
                channel: 7,
                note: 0x15,
                velocity: 127,
            })
            .unwrap();
        assert_eq!(fx1.target, Target::FxSlotEnable(Deck::A, 0));
    }
}
