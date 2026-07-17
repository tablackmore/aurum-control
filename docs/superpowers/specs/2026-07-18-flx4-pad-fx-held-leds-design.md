# FLX4 Pad FX "light while held" LEDs — design

**Date:** 2026-07-18
**Repos:** `aurum-control` (mechanism), then `aurum-pro` (host wiring, dep-bump)
**Status:** Approved for planning (design approved by Tom 2026-07-18)
**Part of:** FLX4 pad-LED effort. Sub-project 2 of 2 (Sub-project 1 = hot-cue pads, shipped).

## Problem

The FLX4's Pad FX pads (`0x10–0x1F` on `0x97` deck A / `0x99` deck B — 16 positions covering the Pad FX 1 and Pad FX 2 layers) map to momentary performance effects (beat-roll, vinyl brake, riser, pad-FX presets). Nothing lights them, so in Pad FX mode the grid is dark and there's no press feedback.

## Goal

A Pad FX pad lights **bright** the instant it is pressed and goes dark on release — a physical-pad "held" echo. Single-colour (the FLX4 pads are yellow; fine for a hold indicator).

Non-goals: reflecting *which* FX is engaged in the engine (this is a physical-pad echo, not engine state); colour; a static idle glow.

## Key design principle

Keep `aurum-pro`'s controller layer **device-agnostic** — it works off decoded *targets* and raw note-on/off, never a hard-coded FX-pad note range. The controller tracks **every** held note generically; the **profile** (RON) declares which addresses echo. So the device knowledge stays in `aurum-control`.

## Design

### `aurum-control` (mechanism)

- **`HeldNotes`** — a compact, `Copy` set of currently-held note addresses: a `[u64; 32]` bitset (2048 bits = every `(note-channel, note)` for the 16 note-on status bytes `0x90–0x9F` × 128 notes). API: `set(status, note, held: bool)`, `contains(status, note) -> bool`, `Default` (empty). Index = `(status & 0x0F) * 128 + note`.
- **`FeedbackState.held: HeldNotes`** — the app copies the current held-set in each frame. `FeedbackState` stays `Copy` (the bitset is `Copy`).
- **`FeedbackSource::HeldEcho`** — in `render`, this rule emits `0x7F` if `state.held.contains(rule.status, rule.data1)`, else `0x00`. (The arm reads the rule's own `status`/`data1`, which are already in scope in `render`.)
- FLX4 RON: 32 `HeldEcho` feedback rules — `0x10–0x1F` on `0x97` and `0x99`.

### `aurum-pro` (host wiring)

- A shared `Arc<Mutex<HeldNotes>>` created in `connect()`. In the MIDI input callback, on any note-on → `held.set(status, note, true)`, note-off (or note-on velocity 0) → `held.set(status, note, false)` — fully generic, no device constants. (Parse the raw `bytes` for status/note; this sits alongside the existing decode, and does not gate on the decoded target.)
- The feedback loop copies the shared `HeldNotes` into `FeedbackState.held` each frame.
- Dep-bump `aurum-control` to the merged Phase-1 commit.

## Data flow

FX pad pressed → note-on in `bytes` → `held.set(status, note, true)` → feedback loop copies `held` into `FeedbackState.held` → `render(HeldEcho @ 0x97/0x10)` sees the address held → `0x7F` → pad lights (shown in Pad FX mode). Release → note-off → `held.set(..., false)` → `0x00` → dark.

## Testing

- `aurum-control`: `HeldNotes` set/contains/clear round-trip; `render(HeldEcho)` bright-when-held / off-when-not; FLX4 profile emits the 32 rules; RON schema-drift guard.
- `aurum-pro`: the input callback flips a note's held bit on note-on/off; the feedback loop reflects it into the frame (a held FX pad emits `0x7F`).
- Full gates per repo; hardware pass.

## Rollout

1. `aurum-control` PR: `HeldNotes`, `FeedbackState.held`, `HeldEcho`, FLX4 rules, tests. CI green → merge.
2. `aurum-pro` PR: shared held-note tracking + feedback fill + dep-bump. Local gate green. (Left open for Tom's review/merge.)

## Out of scope / follow-ups

- Static idle glow / dim baseline (rejected in favour of pure hold-echo).
- Per-FX colour (FLX4 pads are single-colour; a future RGB controller could extend `HeldEcho` to a colour source).
- Sampler/stem pad LEDs (`0x30–0x37`) — a separate small follow-up.
