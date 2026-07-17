# FLX4 Hot Cue Pad LEDs (with colour) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Light the FLX4 hot-cue pads (`0x00–0x07`) to reflect set cues, with a colour model that flows cue → telemetry → LED, ready for RGB-capable controllers.

**Architecture:** Mirror the proven `SavedLoopSlot` pattern. `aurum-control` gains a device-agnostic `LedColor`, a `FeedbackState.hot_cue` field, a `FeedbackSource::HotCueSlot` source, and an optional per-`Profile` colour palette (`LedColor → velocity`); `pro` adds a colour to the cue model, surfaces `hot_cue_present`/`hot_cue_color` in telemetry, and fills the feedback state in `midi_service`.

**Tech Stack:** Rust (`aurum-control` lib + RON profile; `aurum-pro` audio-core/audio-host/src-tauri), `cargo test`, GPG-signed Conventional Commits.

## Global Constraints

- **Worktrees only.** Phase A: `/Users/tomblackmore/aurum/aurum-control-wt-hotcue` (branch `feat/flx4-hot-cue-pad-leds`, created). Phase B: a fresh `aurum-pro` worktree after Phase A merges.
- **`aurum-control` is a public repo** → commits carry **NO AI/assistant attribution**. `aurum-pro` is closed → normal attribution rules.
- **Conventional Commits**, **GPG-signed** (bare `git commit`; never disable gpgsign).
- **Rust gate before every commit:** `cargo fmt --all`, then `cargo clippy --workspace --all-targets -- -D warnings`, then `cargo test --workspace`. Prefix cargo with `source "$HOME/.cargo/env" &&`. Run clippy under the **CI toolchain** in aurum-control: `cargo +stable clippy --all-targets -- -D warnings` and `--features harness` (CI stable is 1.97+; a stale local clippy can pass code CI rejects).
- **`aurum-control` has real CI** — the PR must be green before merge.
- **`aurum-pro` merge gate is the FULL LOCAL gate** (fmt/clippy/`cargo test --workspace`/`tsc`/`vitest`); run vitest **serially, not concurrently with cargo test** (Mixer sync tests flake under CPU load).
- **LED convention:** empty slot → `0x00` (off); set slot → the colour's palette velocity, or bright `0x7F` when the profile has no palette (monochrome).
- **`NUM_CUES = 8`** per deck (`audio-core`); pads are `0x00–0x07` on pad channel `0x97` (deck A) / `0x99` (deck B).

## File Structure

**Phase A — `aurum-control` (`aurum-control-wt-hotcue/`):**
- `src/feedback.rs` — `LedColor` enum + per-slot default; `FeedbackState.hot_cue`; `FeedbackSource::HotCueSlot`; palette-aware render; tests.
- `src/profile.rs` — `Profile.palette: Vec<(LedColor, u8)>`; `render_feedback` threads it.
- `src/lib.rs` — export `LedColor`.
- `profiles/pioneer-ddj-flx4.ron` — 16 `HotCueSlot` rules (`0x00–0x07` × 2 decks).
- `src/profiles.rs` — decoder/feedback tests.
- `docs/devices/pioneer-ddj-flx4.md` — hot-cue LED row.

**Phase B — `aurum-pro` (fresh worktree):**
- `crates/audio-core/src/cue.rs` — colour per cue + per-slot default.
- `crates/audio-core/src/deck.rs` — expose cue presence + colour for telemetry.
- `crates/audio-host/src/telemetry.rs` — `hot_cue_present`/`hot_cue_color` on `DeckSnapshot`.
- `src-tauri/src/midi_service.rs` — fill `FeedbackState.hot_cue`.
- `Cargo.toml`/`Cargo.lock` — bump `aurum-control`.

---

## Phase A — aurum-control

### Task A1: `LedColor` + per-slot defaults

**Files:** Modify `src/feedback.rs` (add enum near top, after the `use`s), `src/lib.rs` (export).

**Interfaces:**
- Produces: `LedColor` (Copy, Eq, Serialize/Deserialize); `LedColor::default_for_slot(u8) -> LedColor`.

- [ ] **Step 1: Add the enum + default palette** in `src/feedback.rs`:

```rust
/// A device-agnostic hot-cue / pad colour. Each device profile maps these to
/// its own hardware values via [`Profile::palette`](crate::Profile); a device
/// with no palette renders any colour as plain bright.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum LedColor {
    Red,
    Orange,
    Yellow,
    Green,
    Cyan,
    Blue,
    Purple,
    Magenta,
}

impl LedColor {
    /// The default colour for hot-cue slot `n` (0-based), so pads show colour
    /// with no user picker yet — a fixed 8-slot palette.
    pub fn default_for_slot(slot: u8) -> LedColor {
        use LedColor::*;
        const PALETTE: [LedColor; 8] = [Red, Orange, Yellow, Green, Cyan, Blue, Purple, Magenta];
        PALETTE[(slot as usize) % PALETTE.len()]
    }
}
```

- [ ] **Step 2: Export** — add `LedColor` to the `pub use feedback::{...}` list in `src/lib.rs`.

- [ ] **Step 3: Test**

```rust
    #[test]
    fn led_color_default_palette_covers_eight_slots() {
        assert_eq!(LedColor::default_for_slot(0), LedColor::Red);
        assert_eq!(LedColor::default_for_slot(7), LedColor::Magenta);
        // Out-of-range wraps rather than panicking.
        assert_eq!(LedColor::default_for_slot(8), LedColor::Red);
    }
```

- [ ] **Step 4: Gate + commit**

```bash
source "$HOME/.cargo/env" && cargo fmt --all && cargo +stable clippy --all-targets -- -D warnings && cargo test --workspace
git add src/feedback.rs src/lib.rs
git commit -m "feat(feedback): add device-agnostic LedColor with per-slot defaults"
```

---

### Task A2: `FeedbackState.hot_cue` + `HotCueSlot` source + palette-aware render

**Files:** Modify `src/feedback.rs` (state field, source variant, render), `src/profile.rs` (palette field + `render_feedback`).

**Interfaces:**
- Consumes: `LedColor` (A1).
- Produces: `FeedbackState.hot_cue: [[Option<LedColor>; 8]; 2]`; `FeedbackSource::HotCueSlot(Deck, u8)`; `Profile.palette: Vec<(LedColor, u8)>`; `render_with_palette(rules, state, palette)`.

- [ ] **Step 1: Add the state field** to `FeedbackState` (after `pad_mode`):

```rust
    /// Per-deck hot-cue slots: `None` = empty, `Some(color)` = a cue is set with
    /// that colour (`[deck][slot]`, slot 0–7). Drives the hot-cue pad LEDs.
    pub hot_cue: [[Option<LedColor>; 8]; 2],
```

- [ ] **Step 2: Add the source variant** (near `SavedLoopSlot`):

```rust
    /// Hot-cue pad LED: off when the slot is empty, else the slot's colour mapped
    /// through the profile palette (or plain bright if the profile has none).
    HotCueSlot(Deck, u8),
```

- [ ] **Step 3: Make render palette-aware.** Change the free `render` to delegate to a palette-aware form, so existing callers (tests) keep working with no palette:

```rust
/// Render one full feedback frame with no colour palette (monochrome).
pub fn render(rules: &[FeedbackRule], state: &FeedbackState) -> Vec<[u8; 3]> {
    render_with_palette(rules, state, &[])
}

/// Render one full feedback frame; `palette` maps [`LedColor`] to a device
/// velocity for colour-capable sources (empty = monochrome bright).
pub fn render_with_palette(
    rules: &[FeedbackRule],
    state: &FeedbackState,
    palette: &[(LedColor, u8)],
) -> Vec<[u8; 3]> {
```

(Move the existing body into `render_with_palette`; add the `HotCueSlot` arm inside the `match`:)

```rust
                FeedbackSource::HotCueSlot(d, s) => {
                    let slot = s as usize;
                    match state.hot_cue[idx(d)].get(slot).copied().flatten() {
                        None => 0x00,
                        Some(color) => palette
                            .iter()
                            .find(|(c, _)| *c == color)
                            .map(|(_, v)| *v)
                            .unwrap_or(0x7F),
                    }
                }
```

- [ ] **Step 4: Add the palette to `Profile`** (`src/profile.rs`, after `feedback`):

```rust
    /// Optional colour palette: maps a [`LedColor`](crate::LedColor) to the
    /// device's velocity for colour-capable feedback (hot-cue pads). Empty =
    /// monochrome (any colour → bright).
    #[serde(default)]
    pub palette: Vec<(crate::LedColor, u8)>,
```

- [ ] **Step 5: Thread it through `render_feedback`.** Find `Profile::render_feedback` (calls `render(&self.feedback, state)`) and change it to `crate::feedback::render_with_palette(&self.feedback, state, &self.palette)`.

- [ ] **Step 6: Tests** (`src/feedback.rs`):

```rust
    #[test]
    fn hot_cue_slot_renders_off_empty_bright_when_no_palette() {
        let rules = [FeedbackRule {
            source: FeedbackSource::HotCueSlot(Deck::A, 0),
            status: 0x97,
            data1: 0x00,
        }];
        let mut st = FeedbackState::default();
        assert!(render(&rules, &st).contains(&[0x97, 0x00, 0x00]), "empty → off");
        st.hot_cue[0][0] = Some(LedColor::Green);
        assert!(render(&rules, &st).contains(&[0x97, 0x00, 0x7F]), "set, no palette → bright");
    }

    #[test]
    fn hot_cue_slot_uses_palette_velocity_when_present() {
        let rules = [FeedbackRule {
            source: FeedbackSource::HotCueSlot(Deck::A, 0),
            status: 0x97,
            data1: 0x00,
        }];
        let palette = [(LedColor::Green, 0x1A), (LedColor::Red, 0x06)];
        let mut st = FeedbackState::default();
        st.hot_cue[0][0] = Some(LedColor::Green);
        assert!(render_with_palette(&rules, &st, &palette).contains(&[0x97, 0x00, 0x1A]));
    }
```

- [ ] **Step 7: Gate + commit**

```bash
source "$HOME/.cargo/env" && cargo fmt --all && cargo +stable clippy --all-targets -- -D warnings && cargo test --workspace
git add src/feedback.rs src/profile.rs
git commit -m "feat(feedback): HotCueSlot source + hot_cue state + per-profile colour palette"
```

---

### Task A3: FLX4 hot-cue feedback rules + doc

**Files:** Modify `profiles/pioneer-ddj-flx4.ron`, `src/profiles.rs` (test), `docs/devices/pioneer-ddj-flx4.md`.

- [ ] **Step 1: Failing feedback test** in `src/profiles.rs`:

```rust
    #[test]
    fn flx4_feedback_renders_hot_cue_pads() {
        use crate::{FeedbackState, LedColor};
        let p = builtin_for_port("DDJ-FLX4").unwrap();
        let mut state = FeedbackState::default();
        state.hot_cue[0][0] = Some(LedColor::Red); // deck A slot 1 set
        state.hot_cue[1][7] = Some(LedColor::Blue); // deck B slot 8 set
        let frame = p.render_feedback(&state);
        // Set slots light (bright, no FLX4 palette yet); empty slots off.
        assert!(frame.contains(&[0x97, 0x00, 0x7F]));
        assert!(frame.contains(&[0x97, 0x01, 0x00]));
        assert!(frame.contains(&[0x99, 0x67, 0x7F]));
    }
```

- [ ] **Step 2: Run — expect FAIL** (`no ... 0x97 0x00` rule yet).

Run: `source "$HOME/.cargo/env" && cargo test --workspace flx4_feedback_renders_hot_cue_pads`
Expected: FAIL.

- [ ] **Step 3: Add the 16 rules** to the `feedback: [ ... ]` list in the RON (after the Beat-Loop `SavedLoopSlot` rules):

```
        // Hot-cue pad LEDs — off (empty) / colour (set). RGB pads: colour via the
        // profile palette below (monochrome-bright until the palette is filled).
        FeedbackRule(source: HotCueSlot(A, 0), status: 0x97, data1: 0x00),
        FeedbackRule(source: HotCueSlot(A, 1), status: 0x97, data1: 0x01),
        FeedbackRule(source: HotCueSlot(A, 2), status: 0x97, data1: 0x02),
        FeedbackRule(source: HotCueSlot(A, 3), status: 0x97, data1: 0x03),
        FeedbackRule(source: HotCueSlot(A, 4), status: 0x97, data1: 0x04),
        FeedbackRule(source: HotCueSlot(A, 5), status: 0x97, data1: 0x05),
        FeedbackRule(source: HotCueSlot(A, 6), status: 0x97, data1: 0x06),
        FeedbackRule(source: HotCueSlot(A, 7), status: 0x97, data1: 0x07),
        FeedbackRule(source: HotCueSlot(B, 0), status: 0x99, data1: 0x00),
        FeedbackRule(source: HotCueSlot(B, 1), status: 0x99, data1: 0x01),
        FeedbackRule(source: HotCueSlot(B, 2), status: 0x99, data1: 0x02),
        FeedbackRule(source: HotCueSlot(B, 3), status: 0x99, data1: 0x03),
        FeedbackRule(source: HotCueSlot(B, 4), status: 0x99, data1: 0x04),
        FeedbackRule(source: HotCueSlot(B, 5), status: 0x99, data1: 0x05),
        FeedbackRule(source: HotCueSlot(B, 6), status: 0x99, data1: 0x06),
        FeedbackRule(source: HotCueSlot(B, 7), status: 0x99, data1: 0x07),
```

(Leave `palette` out of the RON for now — absent → monochrome bright. It is filled by the hardware-verify task.)

- [ ] **Step 4: Run test + parse guard**

Run: `source "$HOME/.cargo/env" && cargo test --workspace flx4_feedback_renders_hot_cue_pads every_builtin_parses`
Expected: both PASS.

- [ ] **Step 5: Update the device doc** `docs/devices/pioneer-ddj-flx4.md` — in the LED outputs table, replace the "Hot-cue pad RGB (TODO)" note with: driven by `HotCueSlot` rules on `0x00–0x07` (`0x97`/`0x99`); off when empty, bright when set; RGB colour via the profile `palette` (velocity→colour), to be captured on the unit.

- [ ] **Step 6: Full gate + commit + push + PR**

```bash
source "$HOME/.cargo/env" && cargo fmt --all && cargo +stable clippy --all-targets -- -D warnings && cargo +stable clippy --all-targets --features harness -- -D warnings && cargo test --workspace
git add profiles/pioneer-ddj-flx4.ron src/profiles.rs docs/devices/pioneer-ddj-flx4.md
git commit -m "feat(flx4): hot-cue pad LED feedback rules"
git push -u origin feat/flx4-hot-cue-pad-leds
gh pr create --fill --title "feat(flx4): hot-cue pad LEDs (with colour model)"
```

Wait for CI green.

---

### Task A4: Hardware-verify the FLX4 colour palette (interactive, with Tom)

Runs against the unit; fills the FLX4 `palette` (RON data only, or a mono ship if the palette can't be pinned).

- [ ] **Step 1:** With the app running (or a small send-test), send note-on to `0x00` on `0x97` sweeping velocities `0x01..0x7F`; Tom records which velocity lights which colour.
- [ ] **Step 2:** Fill `palette: [(Red, <v>), (Orange, <v>), …]` in the FLX4 RON so each `LedColor` maps to its Pioneer velocity. Confirm a set cue shows its slot colour.
- [ ] **Step 3:** If no clean palette emerges, keep the palette empty (monochrome bright) and log it as a follow-up. Commit any RON change; CI green.

---

## Phase B — aurum-pro

Runs after Phase A merges. Fresh worktree: `git worktree add ../aurum-pro-wt-hotcue -b feat/flx4-hot-cue-pad-leds origin/main`; then `cargo clean -p tauri -p tauri-build`, `npm install`, and symlink the model dirs (`ln -sfn ../pro/crates/{separation,analysis}/models crates/{separation,analysis}/models`).

### Task B1: Cue colour in `audio-core`

**Files:** `crates/audio-core/src/cue.rs`, `crates/audio-core/src/deck.rs`.

**Interfaces:**
- Produces: per-slot cue colour reachable from `Deck` for telemetry — a method returning `[Option<CueColor>; NUM_CUES]` (or `(present, color)` pairs). `CueColor` is an `audio-core` enum mirroring `aurum-control::LedColor`'s variants.

- [ ] **Step 1:** Add a `CueColor` enum to `cue.rs` (same 8 variants as `LedColor`: Red…Magenta) with `default_for_slot(usize) -> CueColor` (same fixed palette).
- [ ] **Step 2:** Store colour alongside each set cue. `CuePoints.slots` currently `[Option<usize>; NUM_CUES]`; either add a parallel `colors: [CueColor; NUM_CUES]` or change slots to `[Option<(usize, CueColor)>; NUM_CUES]`. On `set(slot, frame)`, default the colour to `CueColor::default_for_slot(slot)` when the slot was empty (preserve an existing colour on re-set). Add `fn color(&self, slot) -> Option<CueColor>` returning `Some` iff the slot is set.
- [ ] **Step 3:** Add a `Deck` accessor `pub fn hot_cue_colors(&self) -> [Option<CueColor>; NUM_CUES]` (None = empty). Persistence: `cues.json` load path (`deck.rs:640` restores frames) — set colour to the slot default on load so a restored cue isn't colourless (matches the "snap/derive on load" discipline). Persisting a *custom* colour is out of scope (no UI sets one yet).
- [ ] **Step 4:** Unit tests: a set cue reports its slot-default colour; an empty slot reports `None`; re-setting a slot keeps its colour.
- [ ] **Step 5:** Gate + commit.

### Task B2: Telemetry `hot_cue_present`/`hot_cue_color`

**Files:** `crates/audio-host/src/telemetry.rs`.

- [ ] **Step 1:** Mirror `saved_loop_present`: add `hot_cue_present: [[AtomicBool; NUM_CUES]; 2]` and `hot_cue_color: [[AtomicU8; NUM_CUES]; 2]` (colour encoded as a `u8` code, `CueColor as u8`). Add a `store_hot_cues(deck_idx, &[Option<CueColor>; NUM_CUES])`.
- [ ] **Step 2:** Add to `DeckSnapshot`: `pub hot_cue: [Option<CueColor>; NUM_CUES]` (built in the snapshot fn from present+colour, like `saved_loops`).
- [ ] **Step 3:** Call `store_hot_cues` wherever `store_saved_loops` is called (the same telemetry-refresh path), sourcing from `deck.hot_cue_colors()`.
- [ ] **Step 4:** Test: store → snapshot round-trips presence + colour.
- [ ] **Step 5:** Gate + commit.

### Task B3: Dep bump + fill `FeedbackState.hot_cue`

**Files:** `Cargo.toml`/`Cargo.lock`, `src-tauri/src/midi_service.rs`.

- [ ] **Step 1:** Bump `aurum-control` to the merged Phase-A commit: `CARGO_NET_GIT_FETCH_WITH_CLI=true cargo update -p aurum-control`. Commit `chore(deps)`.
- [ ] **Step 2:** In `spawn_feedback_loop`'s `FeedbackState { … }` literal, set `hot_cue` from the snapshot, mapping `audio-core CueColor → midi::LedColor` (a small `match` helper `led_color(CueColor) -> midi::LedColor`):

```rust
hot_cue: [
    std::array::from_fn(|s| snap.decks[0].hot_cue[s].map(led_color)),
    std::array::from_fn(|s| snap.decks[1].hot_cue[s].map(led_color)),
],
```

- [ ] **Step 3:** Test: `led_color` maps every `CueColor` variant to the matching `LedColor`.
- [ ] **Step 4:** Full local gate (fmt/clippy/`cargo test --workspace`/`tsc`/`vitest` — vitest serial). Commit, push, PR.

### Task B4: Hardware acceptance

- [ ] With the Pro build + FLX4: set hot cues on both decks in Hot Cue mode; confirm the pads light (colour if the palette landed, else bright) and empty pads stay dark; clearing a cue darkens its pad.

---

## Self-Review

**Spec coverage:** hot-cue pads lit (A3/B) ✓; colour model end-to-end — `LedColor` (A1), `hot_cue` state + palette render (A2), FLX4 rules (A3), cue colour + telemetry + wiring (B1–B3) ✓; per-slot default colours (A1/B1) ✓; FLX4 RGB palette hardware-verify (A4) ✓; monochrome fallback (A2 render) ✓; two-repo rollout with dep-bump (B3) ✓.

**Placeholder scan:** the only deferred values are the FLX4 palette velocities (A4's explicit hardware purpose) and the Phase-B dep SHA (unknowable until Phase A merges) — both are genuine sequencing gates.

**Type consistency:** `LedColor` (aurum-control) ↔ `CueColor` (audio-core) share the 8 variants and are bridged by `led_color()` (B3); `FeedbackState.hot_cue: [[Option<LedColor>;8];2]`, `FeedbackSource::HotCueSlot(Deck,u8)`, `Profile.palette: Vec<(LedColor,u8)>`, and `render_with_palette` are used identically across tasks.
