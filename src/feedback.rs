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
            master_level: 0.0,
        };
        let frame = render(&rules(), &state);
        assert_eq!(frame[0], [0xB0, 0x02, 127]); // full level
        assert_eq!(frame[1], [0x90, 0x0B, 0x7F]); // playing → LED on
    }

    #[test]
    fn render_level_clamps_and_rounds_and_led_off() {
        let state = FeedbackState {
            deck_level: [0.5, 0.0],
            deck_playing: [false, false],
            master_level: 0.0,
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
}
