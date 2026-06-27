//! Parsing raw MIDI bytes into channel-voice messages.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidiMessage {
    NoteOn {
        channel: u8,
        note: u8,
        velocity: u8,
    },
    NoteOff {
        channel: u8,
        note: u8,
    },
    ControlChange {
        channel: u8,
        controller: u8,
        value: u8,
    },
    /// Any message we don't act on (program change, pitch bend, sysex, …).
    Other,
}

/// Parse a single MIDI message from `bytes` (one event, as midir delivers).
/// Returns None if there is no valid status byte or the message is truncated.
pub fn parse(bytes: &[u8]) -> Option<MidiMessage> {
    let status = *bytes.first()?;
    if status < 0x80 {
        return None; // not a status byte (running status unsupported)
    }
    let kind = status & 0xF0;
    let channel = status & 0x0F;
    match kind {
        0x90 => {
            let note = *bytes.get(1)?;
            let velocity = *bytes.get(2)?;
            if velocity == 0 {
                Some(MidiMessage::NoteOff { channel, note })
            } else {
                Some(MidiMessage::NoteOn {
                    channel,
                    note,
                    velocity,
                })
            }
        }
        0x80 => {
            let note = *bytes.get(1)?;
            let _velocity = *bytes.get(2)?;
            Some(MidiMessage::NoteOff { channel, note })
        }
        0xB0 => {
            let controller = *bytes.get(1)?;
            let value = *bytes.get(2)?;
            Some(MidiMessage::ControlChange {
                channel,
                controller,
                value,
            })
        }
        _ => Some(MidiMessage::Other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_note_on() {
        assert_eq!(
            parse(&[0x92, 60, 100]),
            Some(MidiMessage::NoteOn {
                channel: 2,
                note: 60,
                velocity: 100
            })
        );
    }

    #[test]
    fn note_on_velocity_zero_is_note_off() {
        assert_eq!(
            parse(&[0x90, 60, 0]),
            Some(MidiMessage::NoteOff {
                channel: 0,
                note: 60
            })
        );
    }

    #[test]
    fn parses_note_off_and_cc() {
        assert_eq!(
            parse(&[0x81, 64, 0]),
            Some(MidiMessage::NoteOff {
                channel: 1,
                note: 64
            })
        );
        assert_eq!(
            parse(&[0xB0, 10, 77]),
            Some(MidiMessage::ControlChange {
                channel: 0,
                controller: 10,
                value: 77
            })
        );
    }

    #[test]
    fn rejects_truncated_and_non_status() {
        assert_eq!(parse(&[]), None);
        assert_eq!(parse(&[0xB0, 10]), None);
        assert_eq!(parse(&[60, 100]), None);
    }

    #[test]
    fn unknown_status_is_other() {
        assert_eq!(parse(&[0xE0, 0, 64]), Some(MidiMessage::Other));
    }
}
