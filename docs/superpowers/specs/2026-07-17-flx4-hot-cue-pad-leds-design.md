# FLX4 Hot Cue pad LEDs (with colour) — design

**Date:** 2026-07-17
**Repos:** `aurum-control` (LED source + palette), then `aurum-pro` (cue-colour model, telemetry, host wiring)
**Status:** Approved for planning
**Part of:** the FLX4 pad-LED effort. Sub-project 1 of 2 (Sub-project 2 = Pad FX "light while held", specced separately later).

## Problem

On the FLX4, the 8 performance pads are mode-layered — each pad mode has its own LED addresses, and the unit shows whichever layer matches the current mode. The **Beat-Loop** layer already lights (saved-loop slots, `SavedLoopSlot` → `0x60–0x67`). The **Hot Cue** layer (`0x00–0x07`) is completely dark: a pad with a hot cue set gives no visual indication. The user wants a set cue to illuminate its pad strongly — ideally in colour.

## Goal

Light each hot-cue pad to reflect its slot: a pad whose cue is **set** lights up (in colour where the hardware supports it), an **empty** slot stays dark/dim. Build the colour plumbing end-to-end (cue model → telemetry → LED) so colour-capable controllers — including the FLX4, whose hot-cue pads are RGB — can show per-cue colour, driven by sensible per-slot defaults until a per-cue colour UI exists.

Non-goals (this sub-project): Pad FX pad LEDs (sub-project 2); a UI to pick per-cue colours; the sampler/stem pad layer.

## Feasibility findings

- **Cues exist:** `audio-core` `CuePoints` holds `slots: [Option<usize>; 8]` per deck (`NUM_CUES = 8`) — a 1:1 map to the 8 hot-cue pads.
- **Not yet in telemetry:** `DeckSnapshot` surfaces `saved_loops`/`saved_loop_selected` but **no hot-cue state** — this sub-project adds it, mirroring `saved_loop_present`.
- **No colour in the cue model yet** — cues are just a frame. Colour is added here, defaulted per slot.
- **FLX4 hot-cue pads are RGB** — the device doc's protocol notes say colour is set via a **velocity→colour palette** (Pioneer table, currently a TODO). So colour is achievable on the FLX4 once the palette is captured on the unit.

## Design

### Colour model (device-agnostic)

Add a small, device-agnostic `LedColor` enum in `aurum-control` (`feedback`): the standard DJ hot-cue palette (e.g. Red, Orange, Yellow, Green, Cyan, Blue, Magenta, Pink, White). It is the shared vocabulary; each device profile maps it to its own hardware values.

### `aurum-control` (LED source + per-device palette)

- `FeedbackState.hot_cue: [[Option<LedColor>; 8]; 2]` — `None` = empty slot, `Some(color)` = a cue is set with that colour. (`[deck][slot]`.)
- `FeedbackSource::HotCueSlot(Deck, u8)` — renders the pad for `[deck][slot]`.
- **Per-profile colour palette:** the `Profile` gains an optional `LedColor → u8` (velocity) palette. `render()` for `HotCueSlot`:
  - `None` → `0x00` (off/dark).
  - `Some(color)` → the profile palette's velocity for that colour; if the profile has no palette (monochrome device), fall back to bright `0x7F`.
- FLX4 RON: 8 `HotCueSlot` feedback rules per deck on the hot-cue pad channel (`0x97` A / `0x99` B), notes `0x00–0x07`; plus the FLX4's `LedColor→velocity` palette (hardware-verified — see below).
- This mirrors the proven `SavedLoopSlot` rule structure; render stays a single 7-bit value (the FLX4 encodes colour as a palette velocity, so one byte suffices).

### `aurum-pro` (cue colour + telemetry + wiring)

- **`audio-core`:** give each hot cue a colour. When a cue is set, default its colour by slot index (a fixed Rekordbox-style per-slot palette) so pads show colour immediately with no UI. Colour travels with the cue; if hot cues persist (`cues.json`), persist the colour too (or re-derive the slot default on load — mirror the "snap on load" discipline so a reloaded cue isn't colourless).
- **`audio-host` telemetry:** expose `hot_cue_present[deck][8]` + `hot_cue_color[deck][8]` on `DeckSnapshot`, mirroring `saved_loop_present`/store.
- **`src-tauri` midi_service:** fill `FeedbackState.hot_cue` from telemetry each frame (map the engine colour → `aurum-control` `LedColor`).

### Hardware-verify step (with the unit)

Capture the FLX4's hot-cue-pad **velocity→colour palette**: send candidate velocities to `0x00–0x07` on `0x97` and record which colour each lights, to fill the FLX4 profile palette. If a clean palette can't be pinned quickly, ship a monochrome fallback (any set cue → bright) and refine the palette as a follow-up — the colour plumbing is unaffected either way.

## Data flow

`Deck.cues` (frame + colour) → `Telemetry.store` → `DeckSnapshot.hot_cue_present/color` → `midi_service` fills `FeedbackState.hot_cue` → `render(HotCueSlot)` maps colour via the FLX4 palette → MIDI note-on to `0x00–0x07` → the pad lights (shown in Hot Cue mode).

## Testing

- `aurum-control`: `render` maps `None`→off, `Some(color)`→palette velocity (and mono fallback→`0x7F`); FLX4 profile emits the 16 hot-cue rules; RON schema-drift guard.
- `aurum-pro`: telemetry round-trips `hot_cue_present`/`color`; a set cue defaults to its slot colour; `midi_service` maps telemetry → `FeedbackState.hot_cue`.
- Full gates per repo; hardware pass on the unit.

## Rollout

1. `aurum-control` PR: `LedColor`, `FeedbackState.hot_cue`, `HotCueSlot`, profile palette, FLX4 rules + palette, tests. CI green.
2. Hardware-verify the FLX4 palette (fold into PR 1 or a quick follow-up).
3. `aurum-pro` PR: cue colour in `audio-core`, telemetry, `midi_service` wiring, dep-bump. Local gate green.

## Out of scope / follow-ups

- **Sub-project 2:** Pad FX "light while held".
- A UI to set per-cue colours (+ persisting custom colours) — the plumbing here is ready for it.
- Sampler/stem pad LEDs (`0x30–0x37`) — the `StemMuted`/`StemSoloed` sources exist but are unwired on the FLX4; a separate small follow-up.
