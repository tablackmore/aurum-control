//! A decoded, serialisable view of a single incoming MIDI message — the data the
//! standalone tester UI streams to the browser so you can see exactly what a
//! controller sends (which is what you need to author its profile). Pure: no I/O.

use crate::parse;
use serde::Serialize;

/// One MIDI message, decoded for display.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MonitorEvent {
    /// `"note_on" | "note_off" | "cc" | "other"`.
    pub kind: &'static str,
    /// 1-based MIDI channel (wire channel + 1), or `None` for unparseable/Other.
    pub channel: Option<u8>,
    /// First data byte in context: note number (note on/off) or controller
    /// number (cc). `None` for Other/malformed.
    pub data1: Option<u8>,
    /// Second data byte: velocity (note on) or value (cc). `None` for note off,
    /// Other, or malformed.
    pub data2: Option<u8>,
    /// The raw bytes as upper-case space-separated hex, e.g. `"B0 15 40"`.
    pub raw: String,
}

impl MonitorEvent {
    /// Decode raw MIDI bytes for display. Never fails: anything the parser can't
    /// classify (running status, sysex, truncated) becomes `kind = "other"` with
    /// the raw bytes preserved, so nothing a controller emits is silently dropped.
    pub fn from_midi(bytes: &[u8]) -> Self {
        let raw = bytes
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" ");
        match parse(bytes) {
            Some(crate::MidiMessage::NoteOn {
                channel,
                note,
                velocity,
            }) => Self {
                kind: "note_on",
                channel: Some(channel + 1),
                data1: Some(note),
                data2: Some(velocity),
                raw,
            },
            Some(crate::MidiMessage::NoteOff { channel, note }) => Self {
                kind: "note_off",
                channel: Some(channel + 1),
                data1: Some(note),
                data2: None,
                raw,
            },
            Some(crate::MidiMessage::ControlChange {
                channel,
                controller,
                value,
            }) => Self {
                kind: "cc",
                channel: Some(channel + 1),
                data1: Some(controller),
                data2: Some(value),
                raw,
            },
            // A parsed-but-unhandled status, or unparseable bytes: show it raw.
            _ => Self {
                kind: "other",
                channel: None,
                data1: None,
                data2: None,
                raw,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_note_on_with_one_based_channel_and_hex() {
        let e = MonitorEvent::from_midi(&[0x92, 60, 100]);
        assert_eq!(e.kind, "note_on");
        assert_eq!(e.channel, Some(3)); // wire channel 2 -> displayed 3
        assert_eq!(e.data1, Some(60));
        assert_eq!(e.data2, Some(100));
        assert_eq!(e.raw, "92 3C 64");
    }

    #[test]
    fn note_on_velocity_zero_is_note_off_with_no_velocity() {
        let e = MonitorEvent::from_midi(&[0x90, 60, 0]);
        assert_eq!(e.kind, "note_off");
        assert_eq!(e.data1, Some(60));
        assert_eq!(e.data2, None);
    }

    #[test]
    fn decodes_control_change() {
        let e = MonitorEvent::from_midi(&[0xB0, 21, 64]);
        assert_eq!(e.kind, "cc");
        assert_eq!(e.channel, Some(1));
        assert_eq!(e.data1, Some(21));
        assert_eq!(e.data2, Some(64));
        assert_eq!(e.raw, "B0 15 40");
    }

    #[test]
    fn truncated_or_unknown_status_is_other_but_keeps_raw() {
        let e = MonitorEvent::from_midi(&[0x90]); // note-on with no data
        assert_eq!(e.kind, "other");
        assert_eq!(e.channel, None);
        assert_eq!(e.raw, "90");
    }

    #[test]
    fn serialises_to_the_json_shape_the_ui_expects() {
        let json = serde_json::to_string(&MonitorEvent::from_midi(&[0xB0, 21, 64])).unwrap();
        assert!(json.contains("\"kind\":\"cc\""), "{json}");
        assert!(json.contains("\"data1\":21"), "{json}");
        assert!(json.contains("\"raw\":\"B0 15 40\""), "{json}");
    }
}
