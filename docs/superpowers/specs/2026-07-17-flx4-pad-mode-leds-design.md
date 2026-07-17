# DDJ-FLX4 pad-mode LED indicator — design

**Date:** 2026-07-17
**Repos:** `aurum-control` (mechanism), then `aurum-pro` (host wiring, dep-bump)
**Status:** Approved for planning

## Problem

On the DDJ-FLX4 the four pad-mode buttons — **Hot Cue**, **Pad FX1**, **Beat
Jump**, **Sampler** — select which function the 8 performance pads perform. The
pads themselves already work in AURUM: the hardware switches which MIDI note
range the pads emit, and AURUM routes each range correctly by note. But the
**mode LEDs never update**. At rest the unit powers up with **Hot Cue bright**
and the other three **dimly lit**, and pressing a mode button changes the pad
behaviour without moving the light. So the controller always *looks* like it is
in Hot Cue mode even when it is not.

Nothing in AURUM currently drives these LEDs: the FLX4 profile header notes
"pad-mode/feedback controls are not bound here yet", there is no `FeedbackSource`
for pad mode, and the buttons are unbound in the RON.

## Goal

Make the pad-mode LED cluster reflect the currently-selected pad mode: the
selected button lit **bright**, the others **dim** — updated the instant a mode
button is pressed. Cover all eight hardware modes (four primary + four shift) so
no mode leaves the LED stale.

Non-goal: changing what the pads *do* in any mode. This is a display-state
feature only; the audio/engine path is untouched.

## Hardware findings (verified 2026-07-17, this unit, via `ctl ui` monitor)

Every mode button emits a **momentary** note (note-on `0x7F` on press, note-off
`0x00` on release) on the deck's channel — deck A `0x90`, deck B `0x91`. SHIFT is
its own note (`0x3F`); the hardware **fuses** shift + button into one distinct
note, so each mode is a single unambiguous note — we never track shift-state for
these.

| Mode | Note (deck A) | Physical lamp |
|---|---|---|
| Hot Cue | `0x1B` | Hot Cue |
| Hot Cue (shift) | `0x69` | Hot Cue (shared) |
| Pad FX1 | `0x1E` | Pad FX1 |
| Pad FX2 (shift) | `0x6B` | Pad FX1 (shared) |
| Beat Jump | `0x20` | Beat Jump |
| Beat-Loop (shift) | `0x6D` | Beat Jump (shared) |
| Sampler | `0x22` | Sampler |
| Sampler (shift) | `0x6F` | Sampler (shared) |

Deck B is the same notes on `0x91` (the profile already infers deck B by
bumping the channel).

**Key behavioural facts:**
- Pad mode is a **latching** state (stays until you pick another) even though the
  button is momentary — so AURUM must *remember* the mode; the note only says
  *when* it changed.
- The LEDs are **not self-managed** by the hardware. Software must set the
  selected lamp bright and the rest dim.
- The LED convention is **bright = selected, dim = available** (not on/off) — the
  power-on state (Hot Cue bright, rest dim) already shows the dim target level.
- There are **8 modes but only 4 physical lamps**: the shift modes share a lamp
  with their primary. Whether a shift mode lights its lamp in a *distinct* style
  (e.g. blink, via the `0x69/0x6B/0x6D/0x6F` address) or just lights the parent
  lamp is the one open output question — resolved by the hardware-verify step
  below, and expressed entirely in RON data, not code.

## Approach

**App-tracked pad mode driven through the existing feedback system** — the same
path every other FLX4 LED uses (play, sync, cue, beat-FX, saved-loop pads). This
fits the grain of the code, is unit-testable in `feedback.rs`, and reuses the
existing feedback diff so only the two changed lamps are sent per switch.

Rejected alternative — a device-local reactive LED echo living entirely inside
`aurum-control` (host never hears about pad mode). Conceptually purer (pad mode
has no engine meaning) but introduces a new reactive-feedback path that does not
exist today, for no real benefit.

## Design

### `aurum-control` (mechanism)

1. **`PadMode` enum** — eight variants:
   `HotCue`, `HotCueShift`, `PadFx1`, `PadFx2`, `BeatJump`, `BeatLoop`,
   `Sampler`, `SamplerShift`. `Default` = `HotCue` (matches power-on).

2. **`Target::PadModeSelect(Deck, PadMode)`** — the decoded press. Classified
   `Kind::Trigger` in `Target::kind()`: it fires per press and latches host-side
   like `Sync`/`BeatFxOn`, so a mapping-side latch would go stale (the UI could
   also change the mode indicator in future). A `label()` arm gives the bindings
   UI a human string (e.g. "Deck A · Pad mode: Pad FX2"). Release (note-off) is a
   no-op — the mode is latching.

3. **Input bindings in `pioneer-ddj-flx4.ron`** — 8 notes × 2 decks = 16
   `InputBinding`s mapping each mode note on `0x90`/`0x91` to
   `PadModeSelect(deck, mode)`.

4. **`FeedbackState.pad_mode: [PadMode; 2]`** — the host writes the current mode
   per deck each frame (default `HotCue`).

5. **`FeedbackSource::PadModeLed(Deck, PadMode)`** — one rule per (deck, mode).
   `render()` yields the lamp velocity for that rule:
   `state.pad_mode[deck] == mode ? BRIGHT (0x7F) : DIM (0x20)` — mirroring the
   `SavedLoopSlot` bright/dim precedent (never fully off; dim is the resting
   level). The RON binds each rule to that mode's LED **address** (`0x1B`, `0x69`,
   …). Because only one mode is selected at a time and the diff sends only
   changed messages, switching modes naturally re-lights: old lamp → dim, new
   lamp → bright.

6. **Feedback rules in the RON** — one `FeedbackRule` per mode per deck. The LED
   addresses + the exact BRIGHT/DIM velocities are locked by the hardware-verify
   step. If verification shows shift modes have *no* distinct lamp style, the four
   shift rules simply point at the parent lamp address (or are dropped) — a data
   change, no code change.

7. **Tests** (`feedback.rs` + `profiles.rs`), mirroring the existing LED tests:
   - `render` lights the selected mode bright and the rest dim.
   - Switching the selected mode moves "bright" and the diff reports exactly the
     two changed lamps.
   - Decoder: each of the 8 notes on `0x90`/`0x91` decodes to the right
     `PadModeSelect(deck, mode)`; note-off is inert.
   - Schema-drift guard already parses the RON in-build.

### `aurum-pro` (host wiring)

8. The controller layer stores `pad_mode[deck]` when a `PadModeSelect` target
   arrives and writes it into `FeedbackState.pad_mode` each frame. No
   engine/audio code changes — pad routing already works. Delivered as a
   dep-bump PR pointing at the merged `aurum-control` commit, following the
   established FLX4 two-repo flow (e.g. control #21 → pro #170).

### Hardware-verify step (in the plan, before locking the RON)

Send candidate LED bytes to the unit and have Tom watch:
1. Confirm BRIGHT (`0x7F`) vs the resting DIM level (`0x20` candidate; tune).
2. Confirm each primary lamp address (`0x1B`/`0x1E`/`0x20`/`0x22`) lights the
   expected button.
3. Determine whether the shift addresses (`0x69`/`0x6B`/`0x6D`/`0x6F`) drive the
   shared lamp in a *distinct* style, or whether shift modes should just light
   the parent lamp. Lock the answer into the RON.

Update `docs/devices/pioneer-ddj-flx4.md`: mark the mode notes hardware-verified
(2026-07-17), record the LED addresses/velocities and the shared-lamp decision.

## Testing

- `cargo test --workspace` (aurum-control) — new feedback + decoder tests, plus
  the existing RON schema-drift guard.
- `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all`.
- Hardware pass: press each mode button (and shift variants) on the FLX4 with the
  Pro build running; confirm the LED tracks selection on both decks.

## Rollout

1. `aurum-control` PR: enum, target, bindings, feedback source/state, RON rules,
   tests, device-doc update. CI must be green (aurum-control has real CI).
2. Hardware-verify → lock RON addresses/velocities (may fold into PR 1).
3. `aurum-pro` PR: dep-bump + host stores/forwards `pad_mode`. Local full gate
   green, then squash-merge.

## Out of scope / follow-ups

- Changing pad behaviour in any mode (this is LED-only).
- A software-side UI mirror of pad mode (the on-screen deck could echo it later).
- The broader "extend hot cue / pad fx / beat jump / sampler" functionality Tom
  mentioned — a separate effort once the LED indicator lands.
