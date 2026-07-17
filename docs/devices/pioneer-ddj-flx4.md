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
| Loop IN / OUT | note `0x10` / `0x11` (`0x90`) | button (shift: IN ADJ `0x4C` / OUT ADJ `0x4E`) |
| IN / OUT ADJ (loop-point nudge) | note `0x4C` / `0x4E` (`0x90`/`0x91`), held | Hold (SHIFT+LOOP IN/OUT) + turn jog to nudge the active loop's in/out point by a grid-snapped quarter-beat. Jog flows as `JogBend` CC `0x21`; the decoder reroutes it to `LoopNudgeIn`/`LoopNudgeOut` while held. |
| 4 BEAT/EXIT | note `0x4D` (`0x90`) | button (shift: ACTIVE `0x50`) |
| CUE/LOOP CALL ◄ / ► | note `0x51` / `0x53` (`0x90`) | button (shift: DEL `0x3E` / MEMORY `0x3D`) |
| MASTER / BEAT SYNC | note `0x58` (`0x90`) | button → `Sync` (fires per press; the app flips engine state). Shift: TEMPO RANGE `0x60` → `TempoRange` (frontend range-ladder cycle) |
| Beat-Loop pads 1–8 | notes `0x60–0x67` (status `0x97` A / `0x99` B; +shift `0x98`/`0x9A`) | button — saved-loop slots (SHIFT+pad = delete) |
| Beat-Loop mode-select (SHIFT+BEAT JUMP) | note `0x6D` (`0x90`/`0x91`) | button + LED |
| SHIFT | note `0x3F` (`0x90` left / `0x91` right) | modifier — hardware emits distinct notes for shifted buttons |
| Hot-cue-mode pads 1–8 | notes `0x00–0x07` (status `0x97`; +shift `0x98`) | button |
| Sampler-mode pads 1–8 | notes `0x30–0x37` (status `0x97`) | button — **repurposed**: top row `0x30–0x33` → stem **mute** 0–3, bottom `0x34–0x37` → stem **solo** 0–3 |
| Pad-mode select: Hot Cue / Pad FX1 / Beat Jump / Sampler (+ shift: Hot Cue / Pad FX2 / Beat Loop / Sampler) | primary notes `0x1B` / `0x1E` / `0x20` / `0x22`, shift notes `0x69` / `0x6B` / `0x6D` / `0x6F` (`0x90` deck 1 / `0x91` deck 2) — **all 8 hardware-verified 2026-07-17** | button (also LED, see below) |
| Trim · EQ Hi · EQ Mid · EQ Low | CC `0x04` · `0x07` · `0x0B` · `0x0F` (+`+0x20` LSB), `0xB0` | 14-bit |
| Channel fader | CC `0x13` (+LSB), `0xB0` | 14-bit |
| Tempo fader | CC `0x00` (+LSB), `0xB0` | 14-bit |
| Color / Filter | CC `0x17` (+LSB), `0xB6` | 14-bit |
| Crossfader | CC `0x1F` (+LSB), `0xB6` | 14-bit |
| **Master level rotary** | CC `0x08` (+`0x28` LSB), `0xB6` | 14-bit — captured live 2026-07-03 (Mixxx doesn't map it) |
| Headphones mix | CC `0x0C` (+`0x2C` LSB), `0xB6` | 14-bit |
| Headphones level | CC `0x0D` (+`0x2D` LSB), `0xB6` | 14-bit — captured live 2026-07-03 (it DOES send MIDI) |
| Headphone-cue (PFL) 1 / 2 | note `0x54` (`0x90` / `0x91`) | button (LED echoes state) |
| Master cue | note `0x63` (`0x96`) | button (LED echoes state) — routes master onto the phones CUE side |
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
| **VU meter — RIGHT (deck 2)** | **CC `0x02`, ch 2 (`0xB1`)**, value `level×127` |
| Headphone-cue (PFL) LED | note `0x54` (`0x90` deck 1, `0x91` deck 2) |
| Master-cue LED | note `0x63` (`0x96`) |
| Play / Cue LED | note `0x0B` / `0x0C` (`0x90` deck 1, `0x91` deck 2) |
| BEAT SYNC LED | note `0x58` (`0x90` deck 1, `0x91` deck 2) — assumed LED = input note (PLAY/CUE pattern); **not yet hardware-verified** |
| Pad-mode cluster LEDs (4 buttons: Hot Cue/Pad FX1/Beat Jump/Sampler) | bright `0x7F` (selected family) / dim `0x20` (available) at the **4 primary notes** `0x1B`/`0x1E`/`0x20`/`0x22` (`0x90` deck 1 / `0x91` deck 2) — all hardware-verified 2026-07-17. Shift variants share the primary's lamp (see below), so only the primary address is driven; a shift mode lights its primary lamp via `PadMode::same_button` |
| Hot-cue pad RGB | notes `0x00–0x07` on `0x97` (deck 1) / `0x99` (deck 2), driven by `HotCueSlot` feedback rules — off (`0x00`) when the slot is empty, bright (`0x7F`) when set; RGB colour via the profile `palette` (velocity→colour), to be captured on the unit |

**⚠️ VU meters — Mixxx was right:** an earlier revision of this doc claimed both
meters live on channel 1 (right = `B0 03`) and called Mixxx's `B1 02` a bug. A
controlled ramp test (2026-07-03: each candidate address ramped in isolation,
meters observed live) proved that wrong — `B0 03` leaves the right meter dark;
the right meter is **`B1 02`**, on the deck-2 channel like every other deck-2
control. The meter shows the value as a level/peak position, so feed it the
deck's current level each tick.

**Pad-mode LEDs are NOT hardware-managed** — unlike some Pioneer units, the FLX4
does not keep only one lit on its own; the host must drive this explicitly:
light the selected mode bright (`0x7F`) and every other mode in the cluster dim
(`0x20`) on each pad-mode change.

**Shared lamps — confirmed 2026-07-17.** There are only **4 physical lamps** (one
per button); each primary's shift variant shares its lamp — the shift addresses
`0x69`/`0x6B`/`0x6D`/`0x6F` drive the **same** lamp as `0x1B`/`0x1E`/`0x20`/`0x22`.
Driving both per frame fought last-write-wins and left the lamp dim (bright then
dim to one lamp). So the host drives **only the 4 primary addresses**, one rule
per button; `PadMode::same_button` lights a lamp bright when the selected mode is
that button's primary **or** its shift variant. (Whether the shift address gives a
distinct lit *style* was not needed — a solid family indication suffices.)

## AURUM integration notes

- **`aurum-control` (MIDI):** the profile carries the init SysEx, the input
  bindings above (with per-control encoding + hi-res), and feedback rules
  (transport LEDs, pad-mode LEDs, **and the VU meters** — `vu_meter` level →
  `B0 02` / `B1 02`). Add a small **dead-band** on the tempo-fader inputs to
  swallow residual LSB jitter.
- **VU is MIDI feedback after all** — not the hardware-audio path I first guessed.
  AURUM's feedback driver maps each deck's output level to `B0 02`/`B1 02`. (If we
  *also* route audio through the FLX4's soundcard for headphone cue, that's an
  independent `audio-host` task.)
- Deck 2 inputs/outputs are inferred from deck 1 by bumping the channel, so the
  profile can express decks symmetrically.

## Sources
- Mixxx `Pioneer-DDJ-FLX4-script.js` / `.midi.xml` (init SysEx, LED map — with the
  VU bug this doc corrects).
- Pioneer DDJ-FLX4 MIDI Message List (official, gated).
- Live capture, this unit, via `ctl ui`.
