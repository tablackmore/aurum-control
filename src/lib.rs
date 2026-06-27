//! MIDI handling. Stub — implemented in the Phase 1 plan.

mod highres;
pub use highres::HighResDecoder;

mod relative;
pub use relative::{decode_relative, RelEncoding};

mod message;
pub use message::{parse, MidiMessage};

mod mapping;
pub use mapping::{Action, Binding, ControlId, Deck, Kind, MidiMap, Mode, Options, Target};

pub fn midi_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_nonempty() {
        assert!(!super::midi_version().is_empty());
    }
}
