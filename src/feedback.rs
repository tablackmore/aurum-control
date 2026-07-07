//! Device feedback: turn engine state into the controller's LED/VU MIDI bytes,
//! and diff successive frames so only what changed is sent. Pure and
//! device-agnostic — the app fills a [`FeedbackState`] of plain numbers each
//! frame, and the profile's [`FeedbackRule`]s say which control reflects which
//! value. The MIDI I/O and the polling loop live in the app.

use crate::Deck;
use serde::Deserialize;
use std::collections::HashMap;

/// Engine state the feedback renderer reads, in device-agnostic terms. The app
/// fills this from its telemetry each frame.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FeedbackState {
    /// Per-deck output level, `0..1` (drives VU meters).
    pub deck_level: [f32; 2],
    /// Per-deck transport state (drives play LEDs).
    pub deck_playing: [bool; 2],
    /// Master output level, `0..1`.
    pub master_level: f32,
    /// Per-deck headphone-cue (PFL) state (drives cue-button LEDs).
    pub deck_cued: [bool; 2],
    /// Master headphone-cue state (drives the MASTER CUE button LED).
    pub master_cued: bool,
    /// Per-deck loop-active state (drives the IN/OUT button LEDs).
    pub loop_active: [bool; 2],
    /// Per-deck beat-sync engaged state — true on the FOLLOWER deck only
    /// (drives the BEAT SYNC button LEDs).
    pub deck_syncing: [bool; 2],
    /// Per-deck saved-loop slot occupancy (drives the Beat-Loop pad LEDs).
    pub saved_loop_present: [[bool; 8]; 2],
    /// Per-deck selected saved-loop slot (lit brighter), or `None`.
    pub saved_loop_selected: [Option<u8>; 2],
    /// Beat FX ON/OFF engaged (drives the Beat FX ON/OFF button LED).
    pub beat_fx_on: bool,
    /// Per-deck per-stem mute state (drives stem-mute button LEDs).
    /// Index `[deck][stem]`: deck 0 = A, deck 1 = B; stem 0–3.
    pub stem_muted: [[bool; 4]; 2],
    /// Per-deck per-stem solo state (drives stem-solo button LEDs).
    /// Index `[deck][stem]`: deck 0 = A, deck 1 = B; stem 0–3.
    pub stem_soloed: [[bool; 4]; 2],
}

/// Which engine value a feedback rule reflects.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub enum FeedbackSource {
    /// Continuous deck level → 7-bit (VU meter).
    DeckLevel(Deck),
    /// Continuous master level → 7-bit.
    MasterLevel,
    /// Deck transport — on → full (`0x7F`), off → zero (LED).
    DeckPlaying(Deck),
    /// Deck headphone-cue (PFL) — on → full (`0x7F`), off → zero (LED).
    DeckCued(Deck),
    /// Master headphone-cue — on → full (`0x7F`), off → zero (LED).
    MasterCued,
    /// Deck loop-active — on → full (`0x7F`), off → zero (IN/OUT button LEDs).
    LoopActive(Deck),
    /// Deck beat-sync engaged (the deck is the follower) — on → full (`0x7F`),
    /// off → zero (BEAT SYNC button LED).
    DeckSyncing(Deck),
    /// A saved-loop slot's LED (Beat-Loop pad): off when empty, dim when filled,
    /// full when it is the selected slot.
    SavedLoopSlot(Deck, u8),
    /// Beat FX ON/OFF — on → full (0x7F), off → zero (LED).
    BeatFxOn,
    /// Per-stem mute — on → full (`0x7F`), off → zero (mute button LED).
    /// `(deck, stem_index)` where stem_index is 0–3.
    StemMuted(Deck, u8),
    /// Per-stem solo — on → full (`0x7F`), off → zero (solo button LED).
    /// `(deck, stem_index)` where stem_index is 0–3.
    StemSoloed(Deck, u8),
}

/// One feedback rule: a [`FeedbackSource`] mapped to a concrete MIDI address
/// (`status` byte + `data1`). Continuous sources send `value*127`; boolean
/// sources send `0x7F`/`0x00`.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct FeedbackRule {
    pub source: FeedbackSource,
    pub status: u8,
    pub data1: u8,
}

fn idx(d: Deck) -> usize {
    match d {
        Deck::A => 0,
        Deck::B => 1,
    }
}

fn unit_to_7bit(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 127.0).round() as u8
}

/// Render one full feedback frame: the 3-byte MIDI message for every rule.
pub fn render(rules: &[FeedbackRule], state: &FeedbackState) -> Vec<[u8; 3]> {
    rules
        .iter()
        .map(|r| {
            let data2 = match r.source {
                FeedbackSource::DeckLevel(d) => unit_to_7bit(state.deck_level[idx(d)]),
                FeedbackSource::MasterLevel => unit_to_7bit(state.master_level),
                FeedbackSource::DeckPlaying(d) => {
                    if state.deck_playing[idx(d)] {
                        0x7F
                    } else {
                        0x00
                    }
                }
                FeedbackSource::DeckCued(d) => {
                    if state.deck_cued[idx(d)] {
                        0x7F
                    } else {
                        0x00
                    }
                }
                FeedbackSource::MasterCued => {
                    if state.master_cued {
                        0x7F
                    } else {
                        0x00
                    }
                }
                FeedbackSource::LoopActive(d) => {
                    if state.loop_active[idx(d)] {
                        0x7F
                    } else {
                        0x00
                    }
                }
                FeedbackSource::DeckSyncing(d) => {
                    if state.deck_syncing[idx(d)] {
                        0x7F
                    } else {
                        0x00
                    }
                }
                // Off (empty) / dim (filled) / full (selected). The dim + full
                // levels drive the pad's RGB palette; tuned live on the unit.
                FeedbackSource::SavedLoopSlot(d, s) => {
                    let (i, slot) = (idx(d), s as usize);
                    if state.saved_loop_selected[i] == Some(s) {
                        0x7F
                    } else if slot < 8 && state.saved_loop_present[i][slot] {
                        0x20
                    } else {
                        0x00
                    }
                }
                FeedbackSource::BeatFxOn => {
                    if state.beat_fx_on {
                        0x7F
                    } else {
                        0x00
                    }
                }
                FeedbackSource::StemMuted(d, s) => {
                    // Bounds guard: a malformed rule with s >= 4 returns off
                    // rather than panicking (mirrors the SavedLoopSlot slot<8 guard).
                    if (s as usize) < 4 && state.stem_muted[idx(d)][s as usize] {
                        0x7F
                    } else {
                        0x00
                    }
                }
                FeedbackSource::StemSoloed(d, s) => {
                    if (s as usize) < 4 && state.stem_soloed[idx(d)][s as usize] {
                        0x7F
                    } else {
                        0x00
                    }
                }
            };
            [r.status, r.data1, data2]
        })
        .collect()
}

/// Remembers the last value sent to each `(status, data1)` so a frame only emits
/// the messages whose value changed — VU streams while LEDs stay quiet until
/// they actually flip.
#[derive(Debug, Default)]
pub struct FeedbackDiff {
    last: HashMap<(u8, u8), u8>,
}

impl FeedbackDiff {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return only the messages in `frame` whose data byte changed since the last
    /// call (all of them on the first call — which initialises the device).
    pub fn changed(&mut self, frame: &[[u8; 3]]) -> Vec<[u8; 3]> {
        frame
            .iter()
            .copied()
            .filter(|m| self.last.insert((m[0], m[1]), m[2]) != Some(m[2]))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_loop_and_saved_loop_leds() {
        let rules = vec![
            FeedbackRule {
                source: FeedbackSource::LoopActive(Deck::A),
                status: 0x90,
                data1: 0x10,
            },
            FeedbackRule {
                source: FeedbackSource::SavedLoopSlot(Deck::A, 0),
                status: 0x97,
                data1: 0x60,
            },
            FeedbackRule {
                source: FeedbackSource::SavedLoopSlot(Deck::A, 1),
                status: 0x97,
                data1: 0x61,
            },
        ];
        let mut st = FeedbackState {
            loop_active: [true, false],
            ..Default::default()
        };
        st.saved_loop_present[0][0] = true; // slot 0 filled + selected → full
        st.saved_loop_selected[0] = Some(0);
        let frame = render(&rules, &st);
        assert_eq!(frame[0], [0x90, 0x10, 0x7F]); // loop active → IN LED full
        assert_eq!(frame[1], [0x97, 0x60, 0x7F]); // filled + selected → full
        assert_eq!(frame[2], [0x97, 0x61, 0x00]); // empty → off
                                                  // Filled but not selected → dim.
        st.saved_loop_selected[0] = None;
        assert_eq!(render(&rules, &st)[1], [0x97, 0x60, 0x20]);
    }

    #[test]
    fn renders_sync_leds_per_deck() {
        let rules = vec![
            FeedbackRule {
                source: FeedbackSource::DeckSyncing(Deck::A),
                status: 0x90,
                data1: 0x58,
            },
            FeedbackRule {
                source: FeedbackSource::DeckSyncing(Deck::B),
                status: 0x91,
                data1: 0x58,
            },
        ];
        // A is the follower → its LED full; free deck B stays dark.
        let state = FeedbackState {
            deck_syncing: [true, false],
            ..FeedbackState::default()
        };
        let frame = render(&rules, &state);
        assert_eq!(frame[0], [0x90, 0x58, 0x7F]);
        assert_eq!(frame[1], [0x91, 0x58, 0x00]);
    }

    fn rules() -> Vec<FeedbackRule> {
        vec![
            FeedbackRule {
                source: FeedbackSource::DeckLevel(Deck::A),
                status: 0xB0,
                data1: 0x02,
            },
            FeedbackRule {
                source: FeedbackSource::DeckPlaying(Deck::A),
                status: 0x90,
                data1: 0x0B,
            },
        ]
    }

    #[test]
    fn render_maps_level_to_7bit_and_playing_to_led() {
        let state = FeedbackState {
            deck_level: [1.0, 0.0],
            deck_playing: [true, false],
            ..FeedbackState::default()
        };
        let frame = render(&rules(), &state);
        assert_eq!(frame[0], [0xB0, 0x02, 127]); // full level
        assert_eq!(frame[1], [0x90, 0x0B, 0x7F]); // playing → LED on
    }

    #[test]
    fn render_maps_cued_to_led() {
        let rules = vec![
            FeedbackRule {
                source: FeedbackSource::DeckCued(Deck::A),
                status: 0x90,
                data1: 0x54,
            },
            FeedbackRule {
                source: FeedbackSource::DeckCued(Deck::B),
                status: 0x91,
                data1: 0x54,
            },
        ];
        let state = FeedbackState {
            deck_cued: [true, false],
            ..FeedbackState::default()
        };
        let frame = render(&rules, &state);
        assert_eq!(frame[0], [0x90, 0x54, 0x7F]); // cued → LED on
        assert_eq!(frame[1], [0x91, 0x54, 0x00]); // not cued → LED off
    }

    #[test]
    fn render_level_clamps_and_rounds_and_led_off() {
        let state = FeedbackState {
            deck_level: [0.5, 0.0],
            deck_playing: [false, false],
            ..FeedbackState::default()
        };
        let frame = render(&rules(), &state);
        assert_eq!(frame[0], [0xB0, 0x02, 64]); // 0.5 → 64 (rounded)
        assert_eq!(frame[1], [0x90, 0x0B, 0x00]); // not playing → LED off
    }

    #[test]
    fn diff_sends_all_first_then_only_changes() {
        let mut diff = FeedbackDiff::new();
        let off = FeedbackState::default();
        // First frame: everything is "new" → all messages emitted.
        let first = diff.changed(&render(&rules(), &off));
        assert_eq!(first.len(), 2);
        // Identical frame: nothing changed → nothing emitted.
        assert!(diff.changed(&render(&rules(), &off)).is_empty());
        // Flip just the play LED → only that one message goes out.
        let playing = FeedbackState {
            deck_playing: [true, false],
            ..off
        };
        let changed = diff.changed(&render(&rules(), &playing));
        assert_eq!(changed, vec![[0x90, 0x0B, 0x7F]]);
    }

    #[test]
    fn stem_muted_led_renders_full_or_zero() {
        let rules = [
            FeedbackRule {
                source: FeedbackSource::StemMuted(Deck::A, 0),
                status: 0x90,
                data1: 16,
            },
            FeedbackRule {
                source: FeedbackSource::StemSoloed(Deck::A, 0),
                status: 0x90,
                data1: 8,
            },
        ];
        // stem_muted[0][0] = true → note 16 full; StemSoloed false → note 8 off.
        let mut st = FeedbackState::default();
        st.stem_muted[0][0] = true;
        let frame = render(&rules, &st);
        assert_eq!(frame[0], [0x90, 16, 0x7F]);
        assert_eq!(frame[1], [0x90, 8, 0x00]);
        // Unmuted → note 16 off.
        st.stem_muted[0][0] = false;
        let frame = render(&rules, &st);
        assert_eq!(frame[0], [0x90, 16, 0x00]);
        // stem_soloed[0][0] = true → note 8 full.
        st.stem_soloed[0][0] = true;
        let frame = render(&rules, &st);
        assert_eq!(frame[1], [0x90, 8, 0x7F]);
        // Default state: both deck B stems default false.
        let b_rule = FeedbackRule {
            source: FeedbackSource::StemMuted(Deck::B, 3),
            status: 0x90,
            data1: 23,
        };
        let frame = render(&[b_rule], &FeedbackState::default());
        assert_eq!(frame[0], [0x90, 23, 0x00]);
    }

    #[test]
    fn beat_fx_on_led_renders_full_or_zero() {
        let rules = [FeedbackRule {
            source: FeedbackSource::BeatFxOn,
            status: 0x94,
            data1: 0x47,
        }];
        let on = FeedbackState {
            beat_fx_on: true,
            ..Default::default()
        };
        assert_eq!(render(&rules, &on), vec![[0x94, 0x47, 0x7F]]);
        let off = FeedbackState::default(); // beat_fx_on defaults false
        assert_eq!(render(&rules, &off), vec![[0x94, 0x47, 0x00]]);
    }
}
