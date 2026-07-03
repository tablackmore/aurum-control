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
    fn flx4_decodes_beat_loop_pads() {
        let p = builtin_for_port("DDJ-FLX4").unwrap();
        // Beat-Loop pad mode, captured live. Deck A pads = ch 7 (0x97), shift =
        // ch 8 (0x98); deck B pads = ch 9 (0x99), shift = ch 10 (0x9A).
        let cases = [
            (7u8, 0x60u8, Target::LoopSlot(Deck::A, 0)),
            (7, 0x67, Target::LoopSlot(Deck::A, 7)),
            (8, 0x60, Target::LoopSlotDelete(Deck::A, 0)),
            (8, 0x67, Target::LoopSlotDelete(Deck::A, 7)),
            (9, 0x63, Target::LoopSlot(Deck::B, 3)),
            (10, 0x60, Target::LoopSlotDelete(Deck::B, 0)),
        ];
        for (channel, note, want) in cases {
            let a = p
                .decode(&MidiMessage::NoteOn {
                    channel,
                    note,
                    velocity: 127,
                })
                .unwrap_or_else(|| panic!("no binding for ch {channel} note {note:#04x}"));
            assert_eq!(a.target, want, "ch {channel} note {note:#04x}");
        }
    }

    #[test]
    fn flx4_decodes_loop_cluster_both_decks() {
        let p = builtin_for_port("DDJ-FLX4").unwrap();
        // (channel, note, expected target) — notes captured live off the unit.
        let cases = [
            (0u8, 0x10u8, Target::LoopIn(Deck::A)),
            (0, 0x11, Target::LoopOut(Deck::A)),
            (0, 0x4D, Target::LoopFourOrExit(Deck::A)),
            (0, 0x51, Target::LoopCallPrev(Deck::A)),
            (0, 0x53, Target::LoopCallNext(Deck::A)),
            (0, 0x58, Target::Sync(Deck::A)),
            (0, 0x50, Target::LoopReloop(Deck::A)),
            (0, 0x3E, Target::LoopDelete(Deck::A)),
            (0, 0x3D, Target::LoopSave(Deck::A)),
            (1, 0x10, Target::LoopIn(Deck::B)),
            (1, 0x4D, Target::LoopFourOrExit(Deck::B)),
            (1, 0x3D, Target::LoopSave(Deck::B)),
            (1, 0x58, Target::Sync(Deck::B)),
        ];
        for (channel, note, want) in cases {
            let a = p
                .decode(&MidiMessage::NoteOn {
                    channel,
                    note,
                    velocity: 127,
                })
                .unwrap_or_else(|| panic!("no binding for ch {channel} note {note:#04x}"));
            assert_eq!(a.target, want, "ch {channel} note {note:#04x}");
        }
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
            deck_cued: [true, false],
        };
        let frame = p.render_feedback(&state);
        // Deck A VU full (B0 02 7F), deck B VU silent (B1 02 00 — deck 2's meter
        // is on CHANNEL 2 like every other deck-2 control; the old B0 03 address
        // left the right meter dark on real hardware), deck A play LED on
        // (90 0B 7F), deck B play LED off (91 0B 00), headphone-cue LEDs
        // (90/91 54) mirroring the cue-monitor state.
        assert_eq!(frame[0], [0xB0, 0x02, 127]);
        assert_eq!(frame[1], [0xB1, 0x02, 0]);
        assert_eq!(frame[2], [0x90, 0x0B, 0x7F]);
        assert_eq!(frame[3], [0x91, 0x0B, 0x00]);
        assert_eq!(frame[4], [0x90, 0x54, 0x7F]);
        assert_eq!(frame[5], [0x91, 0x54, 0x00]);
    }

    #[test]
    fn flx4_decodes_headphone_cue_buttons_and_mix_knob() {
        let p = builtin_for_port("DDJ-FLX4").unwrap();
        // Headphone-cue (PFL) buttons: note 0x54 on the deck channel → CueMonitor.
        let cue_a = p
            .decode(&MidiMessage::NoteOn {
                channel: 0,
                note: 0x54,
                velocity: 127,
            })
            .unwrap();
        assert_eq!(cue_a.target, Target::CueMonitor(Deck::A));
        let cue_b = p
            .decode(&MidiMessage::NoteOn {
                channel: 1,
                note: 0x54,
                velocity: 127,
            })
            .unwrap();
        assert_eq!(cue_b.target, Target::CueMonitor(Deck::B));
        // HEADPHONES MIX knob: CC 0x0C on the mixer channel → CueMix.
        let mix = p
            .decode(&MidiMessage::ControlChange {
                channel: 6,
                controller: 0x0C,
                value: 127,
            })
            .unwrap();
        assert_eq!(mix.target, Target::CueMix);
        assert_eq!(mix.value, ActionValue::Absolute(1.0));
    }

    #[test]
    fn flx4_decodes_master_level_rotary() {
        let p = builtin_for_port("DDJ-FLX4").unwrap();
        // MASTER LEVEL rotary: CC 0x08 (MSB) on the mixer channel, captured live.
        let m = p
            .decode(&MidiMessage::ControlChange {
                channel: 6,
                controller: 0x08,
                value: 127,
            })
            .unwrap();
        assert_eq!(m.target, Target::Master);
        assert_eq!(m.value, ActionValue::Absolute(1.0));
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

    #[test]
    fn flx4_hot_cue_pads_are_momentary_hold() {
        let p = builtin_for_port("DDJ-FLX4").unwrap();
        // Hot-cue pad 1 press (note 0x00 on deck-A pad channel 0x97) → HotCueHold.
        let press = p
            .decode(&MidiMessage::NoteOn {
                channel: 7,
                note: 0x00,
                velocity: 127,
            })
            .unwrap();
        assert_eq!(press.target, Target::HotCueHold(Deck::A, 0));
        assert_eq!(press.value, ActionValue::Absolute(1.0));
        // Release (note-off) is carried through by the raw profile as 0.0 so the
        // stateful decoder can emit the "return to ghost" edge.
        let release = p
            .decode(&MidiMessage::NoteOff {
                channel: 7,
                note: 0x00,
            })
            .unwrap();
        assert_eq!(release.target, Target::HotCueHold(Deck::A, 0));
        assert_eq!(release.value, ActionValue::Absolute(0.0));
        // Deck B pad 8 (note 0x07 on pad channel 0x99).
        let b = p
            .decode(&MidiMessage::NoteOn {
                channel: 9,
                note: 0x07,
                velocity: 127,
            })
            .unwrap();
        assert_eq!(b.target, Target::HotCueHold(Deck::B, 7));
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
