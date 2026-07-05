# aurum-control

Device- and engine-agnostic DJ controller logic for AURUM. Pure Rust, no I/O —
the library parses and decides; the app owns the MIDI ports. Consumed by the
free and pro apps as a git dependency (`[lib] name = "midi"`, so downstream
code is `use midi::…`).

What's in the box:

- **MIDI parsing** (`message.rs`) — channel-voice parsing that never panics on
  malformed input; velocity-0 note-ons normalize to note-off.
- **Hi-res + relative CC** (`highres.rs`, `relative.rs`) — 14-bit MSB/LSB
  reassembly (pairs learned from traffic) and relative-encoder decoding
  (two's-complement, offset-64, sign-magnitude).
- **MIDI-learn mapping** (`mapping.rs`) — `MidiMap`: learnable bindings with
  continuous/toggle/momentary/trigger semantics per target, soft takeover,
  and JSON persistence with schema migration.
- **Device profiles** (`profile.rs`, `profiles.rs`, `profiles/*.ron`) —
  declarative RON profiles decoded by `ProfileDecoder`; ships a built-in
  Pioneer DDJ-FLX4 profile with hardware-captured addresses (decks, mixer,
  loop cluster, performance pads, Beat FX, headphone section).
- **Jog decoding** (`jog.rs`) — platter scratch/bend math.
- **LED/VU feedback** (`feedback.rs`) — engine state → feedback frames with
  diffing: play/cue/sync/loop/pad LEDs, VU meters.
- **Tester harness** (`--features harness`) — the `ctl` binary: a localhost
  web MIDI monitor for capturing a device's protocol live.
  `cargo run --features harness --bin ctl -- ui`

Per-device protocol notes live in `docs/devices/`.
