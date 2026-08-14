# Teenage Engineering TX-6 — control protocol

Derived from the **official Teenage Engineering TX-6 MIDI reference**
(https://teenage.engineering/guides/tx-6, firmware 1.3.3) — **spec-derived, not
yet captured live**. Run `cargo run --features harness --bin ctl -- ui` against
a real unit to capture and confirm every address before relying on this map.
Assumes firmware ≥ 1.2.5 (earlier firmware used non-0–127 CC ranges).

## Device facts

- **6-channel pocket mixer** with USB-C audio + MIDI (class-compliant, no
  drivers on macOS). Per track: one fader, three knobs (upper/middle/lower),
  one button; plus a rotary encoder (turn + press) and a handful of global
  buttons (FX I/II, shift, aux, cue).
- **As a MIDI controller, everything is a CC on MIDI channel 1** (status
  `0xB0`). Faders/knobs send absolute 0–127; buttons send momentary 0/127
  (0–63 off / 64–127 on); the encoder turn is offset-64 relative.
- **No documented LED or motor-fader feedback** — the profile registers no
  `init` SysEx and no feedback rules.

## Enabling MIDI out on the device

- The TX-6 sends controller messages only when **`ctrl out` is enabled** in
  its MIDI settings — nothing arrives without it.
- For pure-controller use (moving a fader should NOT change the TX-6's own
  audio mix), turn **local control off** by sending it CC `122` value 0 on
  **channel 7** (the global channel of its *incoming* map, below).

## Outgoing controls (all CC, channel 1 / status `0xB0`)

| Control | CC | Encoding |
|---|---|---|
| Faders 1–6 | `1`–`6` | absolute 0–127 |
| Track 1–6 upper knobs | `7`–`12` | absolute 0–127 |
| Track 1–6 middle knobs | `13`–`18` | absolute 0–127 |
| Track 1–6 lower knobs | `19`–`24` | absolute 0–127 |
| Track buttons 1–6 | `25`–`30` | momentary 0/127 |
| Encoder turn | `31` | **relative, offset-64** (`65` = +1, `63` = −1) → `Centre64` |
| Encoder press | `32` | momentary 0/127 |
| FX I / FX II buttons | `33` / `34` | momentary 0/127 |
| Shift / aux buttons | `35` / `36` | momentary 0/127 |
| Cue button | `37` | momentary 0/127 |

## Default AURUM mapping (`profiles/te-tx-6.ron`)

| CC | Control | Target |
|---|---|---|
| 1 / 2 | faders 1–2 | Deck A / B channel volume |
| 3 | fader 3 | Crossfader |
| 4 | fader 4 | Cue mix (phones master↔cue blend) |
| 5 | fader 5 | Headphone level |
| 6 | fader 6 | Master volume |
| 7 / 13 / 19 | track-1 knobs | Deck A EQ hi / mid / low |
| 8 / 14 / 20 | track-2 knobs | Deck B EQ hi / mid / low |
| 9 / 15 / 21 | track-3 knobs | Deck A trim / tempo / pan |
| 10 / 16 / 22 | track-4 knobs | Deck B trim / tempo / pan |
| 11 / 17 / 23 · 12 / 18 / 24 | track-5/6 knobs | unmapped (spare) |
| 25 / 26 | track buttons 1–2 | Cue (PFL) A / B toggle |
| 27 / 28 | track buttons 3–4 | Play/pause A / B |
| 29 / 30 | track buttons 5–6 | SYNC A / B (fires per press; the app flips engine state) |
| 31 | encoder turn | Library scroll |
| 32 | encoder press | Library open |
| 33 / 34 | FX I / II | Hot cue 1 on deck A / B (the engine FX rack is Pro-only, so no FX target is a sensible free default) |
| 35 / 36 | shift / aux | unbound (reserved) |
| 37 | cue button | Master-cue toggle |

Mapping notes:

- The buttons' momentary 0/127 CCs work with the decoder's natural press-edge
  semantics (Toggle for play/PFL/master-cue, per-press Trigger for SYNC and
  hot cues) — no `mode` overrides needed, unlike hardware-latching CC buttons.
- Track-3/4 middle and lower knobs are **substitutions**: the suggested
  "gain + extras" have no dedicated free-tier targets beyond `Trim`, so
  `Tempo` and `Pan` fill those rows.

## Verification status

**Spec-derived — pending hardware capture.** Every address above comes from
TE's published reference, not a live capture. Before trusting the map:
`cargo run --features harness --bin ctl -- ui`, move every control, and
confirm the CC numbers, the button 0/127 behaviour, and the encoder's
offset-64 encoding (a fast spin should show magnitudes >1, e.g. `67` = +3).

## Incoming CC map (future feedback / remote-control potential)

The TX-6 also *listens* on MIDI — one channel per track (1–6) plus a global
channel 7. Not used by the profile today, but it means a host could remotely
drive the TX-6's own mix engine later:

| Parameter | CC | Channel |
|---|---|---|
| Track volume | `7` | 1–6 (per track) |
| Track pan | `8` | 1–6 |
| Track gain | `9` | 1–6 |
| Track EQ low / mid / high | `85` / `86` / `87` | 1–6 |
| Track filter | `74` | 1–6 |
| Track FX send | `91` | 1–6 |
| Track aux send | `92` | 1–6 |
| Main volume | `7` | 7 (global) |
| Cue volume | `15` | 7 |
| Local control on/off | `122` | 7 |
