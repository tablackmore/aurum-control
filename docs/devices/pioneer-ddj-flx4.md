# Pioneer DDJ-FLX4 — control protocol

Reverse-engineered live against a real unit with the `ctl` tester harness, then
cross-checked against the Mixxx mapping and Pioneer's MIDI message list. **Two
addresses here correct bugs in the public Mixxx mapping** (the VU meters).

## Device facts

- **Class-compliant USB-MIDI + USB-audio** — no drivers; works as a plain MIDI
  device on macOS. 2-channel / 2-deck → maps 1:1 onto AURUM's two decks.
- **Decks are distinguished by MIDI channel:** deck 1 = channel 1 (status
  `0x90`/`0xB0`), deck 2 = channel 2 (`0x91`/`0xB1`). Pads use their own channels
  (deck 1 `0x97`, +shift `0x98`; deck 2 `0x99`/`0x9A`). The browser/mixer block
  (crossfader, filter, browse, load) is on channel 7 (`0x96`/`0xB6`).
- **Knobs/faders are 14-bit hi-res:** each sends an MSB on CC *n* and an LSB on
  CC *n+32*.

## Init handshake (REQUIRED)

Until the host sends this enable SysEx, the FLX4 (a) continuously streams its
deck-2 tempo-fader value (an idle "flood" of ~44 msg/s — analog LSB jitter on
CC `0x00`/`0x20` ch 2) and (b) **ignores all LED commands**. Send once on connect:

```
F0 00 40 05 00 00 04 05 00 50 02 F7
```

Confirmed live: after sending it the idle stream dropped from ~88 msgs/2s to **0**,
and LEDs began responding. (Source: Mixxx script, "reverse engineered with Wireshark".)

## Inputs (deck 1; deck 2 = same with channel +1)

| Control | Message | Encoding |
|---|---|---|
| Play / Cue | note `0x0B` / `0x0C` (`0x90`) | button |
| Hot-cue-mode pads 1–8 | notes `0x00–0x07` (status `0x97`; +shift `0x98`) | button |
| Sampler-mode pads 1–8 | notes `0x30–0x37` (status `0x97`) | button — **repurposed**: top row `0x30–0x33` → stem **mute** 0–3, bottom `0x34–0x37` → stem **solo** 0–3 |
| Pad-mode select: Hot Cue / Pad FX1 / Beat Jump / Sampler | notes `0x1B` / `0x1E` / `0x20` / `0x22` (`0x90`) | button (also LED, see below) |
| Trim · EQ Hi · EQ Mid · EQ Low | CC `0x04` · `0x07` · `0x0B` · `0x0F` (+`+0x20` LSB), `0xB0` | 14-bit |
| Channel fader | CC `0x13` (+LSB), `0xB0` | 14-bit |
| Tempo fader | CC `0x00` (+LSB), `0xB0` | 14-bit |
| Color / Filter | CC `0x17` (+LSB), `0xB6` | 14-bit |
| Crossfader | CC `0x1F` (+LSB), `0xB6` | 14-bit |
| **Jog touch** | note `0x36` (`0x90`) | button (on=touched) |
| **Jog top / scratch** | CC `0x22` (`0xB0`) | **relative, centre 64** (`0x41`=+1, `0x3F`=−1) |
| **Jog ring / pitch-bend** | CC `0x21` (`0xB0`) | relative, centre 64 |
| **Browse / select** | CC `0x40` (`0xB6`) | **relative, centre 0** (`0x01`=+1, `0x7F`=−1) |
| Browse press | note `0x41` (`0x96`) | button |
| **Load deck 1 / deck 2** | note `0x46` / `0x47` (`0x96`) | button |

Two distinct relative encodings are in play — the jog is centred at 64, the
browse encoder at 0. The profile must declare per-control encoding.

## Outputs — LEDs (only after the init SysEx)

`toggleLight` = note-on with velocity `0x7F` (on) / `0x00` (off).

| LED | Message |
|---|---|
| **VU meter — LEFT (deck 1)** | **CC `0x02`, ch 1 (`0xB0`)**, value `level×127` |
| **VU meter — RIGHT (deck 2)** | **CC `0x03`, ch 1 (`0xB0`)**, value `level×127` |
| Play / Cue LED | note `0x0B` / `0x0C` (`0x90` deck 1, `0x91` deck 2) |
| Pad-mode LEDs (Hot Cue/Pad FX1/Beat Jump/Sampler) | notes `0x1B`/`0x1E`/`0x20`/`0x22` (`0x90`/`0x91`) |
| Hot-cue pad RGB | notes `0x00–0x07` on `0x97` (deck 1) / `0x99` (deck 2); colour via velocity palette (TODO from Pioneer list) |

**⚠️ VU correction:** the public Mixxx script drives deck 2 on `B1 02`, but on
real hardware **both meters live on channel 1** — left = `B0 02`, right = `B0 03`.
Verified live (left bar and right bar ramp independently). The meter shows the
value as a level/peak position, so feed it the deck's current level each tick.

**Pad-mode LEDs are mutually exclusive** — the unit keeps only one lit (it shows
the active pad mode), so to switch modes just light the new one.

## AURUM integration notes

- **`aurum-control` (MIDI):** the profile carries the init SysEx, the input
  bindings above (with per-control encoding + hi-res), and feedback rules
  (transport LEDs, pad-mode LEDs, **and the VU meters** — `vu_meter` level →
  `B0 02` / `B0 03`). Add a small **dead-band** on the tempo-fader inputs to
  swallow residual LSB jitter.
- **VU is MIDI feedback after all** — not the hardware-audio path I first guessed.
  AURUM's feedback driver maps each deck's output level to `B0 02`/`B0 03`. (If we
  *also* route audio through the FLX4's soundcard for headphone cue, that's an
  independent `audio-host` task.)
- Deck 2 inputs/outputs are inferred from deck 1 by bumping the channel, so the
  profile can express decks symmetrically.

## Sources
- Mixxx `Pioneer-DDJ-FLX4-script.js` / `.midi.xml` (init SysEx, LED map — with the
  VU bug this doc corrects).
- Pioneer DDJ-FLX4 MIDI Message List (official, gated).
- Live capture, this unit, via `ctl ui`.
