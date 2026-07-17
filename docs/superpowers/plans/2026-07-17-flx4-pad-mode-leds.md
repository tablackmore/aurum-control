# DDJ-FLX4 Pad-Mode LED Indicator — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the FLX4 pad-mode cluster LEDs (Hot Cue / Pad FX1 / Beat Jump / Sampler + shift variants) reflect the selected pad mode — selected bright, others dim — updated on button press.

**Architecture:** Bind the 8 mode-select notes to a new latching `Target::PadModeSelect(Deck, PadMode)` in `aurum-control`; the pro host records the mode per deck and writes it into `FeedbackState.pad_mode`; a new `FeedbackSource::PadModeLed(Deck, PadMode)` renders each lamp bright/dim through the existing feedback + diff path. No audio/engine code changes — pads already self-disambiguate by note range.

**Tech Stack:** Rust (`aurum-control` lib crate + RON device profile; `aurum-pro` Tauri host), `cargo test`, GPG-signed Conventional Commits.

## Global Constraints

- **Worktrees only** — Phase 1 runs in `/Users/tomblackmore/aurum/aurum-control-wt-padmode` (branch `feat/flx4-pad-mode-leds`, already created). Phase 2 runs in a fresh `aurum-pro` worktree.
- **`aurum-control` is a public repo** — commits carry **NO AI/assistant attribution**.
- **Conventional Commits**, **GPG-signed** (bare `git commit` signs here; do NOT pass `-c commit.gpgsign=false`).
- **Rust gate before every commit:** `cargo fmt --all`, then `cargo clippy --workspace --all-targets -- -D warnings`, then `cargo test --workspace`. Prefix cargo with `source "$HOME/.cargo/env" &&`.
- **`aurum-control` has real CI** — the branch PR must be green before merge.
- **`aurum-pro` merge gate is the FULL LOCAL gate** (fmt/clippy/test + `tsc`/vitest); pro CI runs only on release tags.
- **LED convention:** bright = `0x7F` (selected), dim = `0x20` (available, resting) — never fully off; mirrors the existing `SavedLoopSlot` rule.
- **Verified MIDI (this unit, 2026-07-17):** mode-select notes on deck channel `0x90` (A) / `0x91` (B): Hot Cue `0x1B`, Hot Cue-shift `0x69`, Pad FX1 `0x1E`, Pad FX2 `0x6B`, Beat Jump `0x20`, Beat-Loop `0x6D`, Sampler `0x22`, Sampler-shift `0x6F`. Momentary (note-on `0x7F` / note-off `0x00`).

## File Structure

**Phase 1 — `aurum-control` (`aurum-control-wt-padmode/`):**
- `src/mapping.rs` — add `PadMode` enum; `Target::PadModeSelect(Deck, PadMode)`; `kind()` (Trigger) and `label()` arms.
- `src/lib.rs` — export `PadMode`.
- `src/feedback.rs` — `FeedbackState.pad_mode` field; `FeedbackSource::PadModeLed(Deck, PadMode)`; `render()` arm; unit tests.
- `profiles/pioneer-ddj-flx4.ron` — 16 input bindings + 16 feedback rules.
- `src/profiles.rs` — decoder + feedback integration tests.
- `docs/devices/pioneer-ddj-flx4.md` — mark mode notes verified; record LED addresses.

**Phase 2 — `aurum-pro` (fresh worktree):**
- `Cargo.toml` / `Cargo.lock` — bump the `midi` (aurum-control) git dependency rev.
- `src-tauri/src/midi_service.rs` — shared per-deck pad-mode state; intercept `PadModeSelect` on input; feed `pad_mode` into `FeedbackState`.

---

## Phase 1 — aurum-control

### Task 1: `PadMode` enum + `PadModeSelect` target

**Files:**
- Modify: `src/mapping.rs` (add enum near `Deck` ~line 32; add `Target` variant ~line 200; `kind()` Trigger arm ~line 246; `label()` ~line 260+)
- Modify: `src/lib.rs:13` (export `PadMode`)

**Interfaces:**
- Produces: `PadMode` enum (`HotCue` default, `HotCueShift`, `PadFx1`, `PadFx2`, `BeatJump`, `BeatLoop`, `Sampler`, `SamplerShift`); `Target::PadModeSelect(Deck, PadMode)`.

- [ ] **Step 1: Add the `PadMode` enum** in `src/mapping.rs`, immediately after the `Deck` impl block:

```rust
/// Pioneer pad-mode selection — which function the 8 performance pads perform.
/// Latching device display-state used only to drive the mode-cluster LEDs; the
/// pads self-disambiguate by note range, so the engine never needs this.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize, Hash)]
pub enum PadMode {
    #[default]
    HotCue,
    HotCueShift,
    PadFx1,
    PadFx2,
    BeatJump,
    BeatLoop,
    Sampler,
    SamplerShift,
}
```

- [ ] **Step 2: Add the `Target` variant** at the end of the `Target` enum (after `BeatFxRelease`):

```rust
    /// Pad-mode selector press (FLX4 Hot Cue / Pad FX1 / Beat Jump / Sampler +
    /// shift variants). Latching device display-state: the HOST records the mode
    /// and drives the cluster LEDs via `FeedbackState::pad_mode`; there is no
    /// engine action. `Kind::Trigger` — fires on press, host ignores the release.
    PadModeSelect(Deck, PadMode),
```

- [ ] **Step 3: Classify it in `kind()`** — add `PadModeSelect(..)` to the `Kind::Trigger` match arm (the arm containing `Sync(_)` / `TempoRange(_)`):

```rust
            LoopToggle(_) | Sync(_) | HotCue(..) | HotCueClear(..) | LoopIn(_) | LoopOut(_)
            | LoopHalve(_) | LoopDouble(_) | BeatJump(..) | LoopSet(..) | LoopFourOrExit(_)
            | LoopReloop(_) | LoopSave(_) | LoopCallPrev(_) | LoopCallNext(_) | LoopDelete(_)
            | LoopSlot(..) | LoopSlotDelete(..) | LibraryOpen | LoadDeck(_) | TempoRange(_)
            | PadModeSelect(..) => Kind::Trigger,
```

- [ ] **Step 4: Add a `label()` arm** (in the `Target::label()` match):

```rust
            PadModeSelect(d, m) => {
                let name = match m {
                    PadMode::HotCue => "Hot Cue",
                    PadMode::HotCueShift => "Hot Cue (shift)",
                    PadMode::PadFx1 => "Pad FX1",
                    PadMode::PadFx2 => "Pad FX2",
                    PadMode::BeatJump => "Beat Jump",
                    PadMode::BeatLoop => "Beat-Loop",
                    PadMode::Sampler => "Sampler",
                    PadMode::SamplerShift => "Sampler (shift)",
                };
                format!("Deck {} · Pad mode: {name}", d.tag())
            }
```

- [ ] **Step 5: Export `PadMode`** — extend the `mapping` re-export in `src/lib.rs:13`:

```rust
pub use mapping::{Action, Binding, ControlId, Deck, Kind, MidiMap, Mode, Options, PadMode, Target};
```

- [ ] **Step 6: Add a unit test** at the bottom of `src/mapping.rs`'s `#[cfg(test)] mod tests` (create the test fn):

```rust
    #[test]
    fn pad_mode_select_is_trigger_and_defaults_to_hot_cue() {
        assert_eq!(PadMode::default(), PadMode::HotCue);
        assert_eq!(
            Target::PadModeSelect(Deck::A, PadMode::PadFx2).kind(),
            Kind::Trigger
        );
    }
```

- [ ] **Step 7: Gate + verify**

Run: `source "$HOME/.cargo/env" && cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace`
Expected: builds clean, all tests PASS (including the new one).

- [ ] **Step 8: Commit**

```bash
git add src/mapping.rs src/lib.rs
git commit -m "feat(mapping): add PadMode + PadModeSelect target for FLX4 pad-mode LEDs"
```

---

### Task 2: Decoder bindings in the FLX4 profile

**Files:**
- Modify: `profiles/pioneer-ddj-flx4.ron` (add input bindings in deck A and deck B input sections)
- Modify: `src/profiles.rs` (add a decoder test)

**Interfaces:**
- Consumes: `Target::PadModeSelect`, `PadMode` (Task 1).
- Produces: the FLX4 profile decodes the 8 mode notes × 2 decks.

- [ ] **Step 1: Write the failing decoder test** in `src/profiles.rs` `mod tests` (add `PadMode` to the test-module imports if `Target`/`Deck` are imported there; otherwise reference `crate::PadMode`):

```rust
    #[test]
    fn flx4_decodes_pad_mode_selectors() {
        let p = builtin_for_port("DDJ-FLX4").unwrap();
        let cases = [
            (0u8, 0x1B, Target::PadModeSelect(Deck::A, PadMode::HotCue)),
            (0, 0x69, Target::PadModeSelect(Deck::A, PadMode::HotCueShift)),
            (0, 0x1E, Target::PadModeSelect(Deck::A, PadMode::PadFx1)),
            (0, 0x6B, Target::PadModeSelect(Deck::A, PadMode::PadFx2)),
            (0, 0x20, Target::PadModeSelect(Deck::A, PadMode::BeatJump)),
            (0, 0x6D, Target::PadModeSelect(Deck::A, PadMode::BeatLoop)),
            (0, 0x22, Target::PadModeSelect(Deck::A, PadMode::Sampler)),
            (0, 0x6F, Target::PadModeSelect(Deck::A, PadMode::SamplerShift)),
            (1, 0x1E, Target::PadModeSelect(Deck::B, PadMode::PadFx1)),
            (1, 0x6D, Target::PadModeSelect(Deck::B, PadMode::BeatLoop)),
        ];
        for (channel, note, want) in cases {
            let a = p
                .decode(&MidiMessage::NoteOn { channel, note, velocity: 127 })
                .unwrap_or_else(|| panic!("no binding for ch {channel} note {note:#x}"));
            assert_eq!(a.target, want);
            assert_eq!(a.value, ActionValue::Absolute(1.0));
        }
        // Release carries through as 0.0 at the raw layer; the stateful decoder
        // drops the Trigger falling edge and the host acts on press only.
        let rel = p.decode(&MidiMessage::NoteOff { channel: 0, note: 0x1E }).unwrap();
        assert_eq!(rel.target, Target::PadModeSelect(Deck::A, PadMode::PadFx1));
        assert_eq!(rel.value, ActionValue::Absolute(0.0));
    }
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `source "$HOME/.cargo/env" && cargo test --workspace flx4_decodes_pad_mode_selectors`
Expected: FAIL — `no binding for ch 0 note 0x1b` (bindings not added yet).

- [ ] **Step 3: Add the deck-A input bindings** to `profiles/pioneer-ddj-flx4.ron`, in the deck-A input section (e.g. just after the SYNC / TEMPO RANGE bindings, before the pad bindings):

```
        // Pad-mode selector buttons (deck channel 0x90) — latching display state
        // that drives the mode-cluster LEDs. Notes hardware-verified 2026-07-17.
        InputBinding(status: 0x90, data1: 0x1B, target: PadModeSelect(A, HotCue)),
        InputBinding(status: 0x90, data1: 0x69, target: PadModeSelect(A, HotCueShift)),
        InputBinding(status: 0x90, data1: 0x1E, target: PadModeSelect(A, PadFx1)),
        InputBinding(status: 0x90, data1: 0x6B, target: PadModeSelect(A, PadFx2)),
        InputBinding(status: 0x90, data1: 0x20, target: PadModeSelect(A, BeatJump)),
        InputBinding(status: 0x90, data1: 0x6D, target: PadModeSelect(A, BeatLoop)),
        InputBinding(status: 0x90, data1: 0x22, target: PadModeSelect(A, Sampler)),
        InputBinding(status: 0x90, data1: 0x6F, target: PadModeSelect(A, SamplerShift)),
```

- [ ] **Step 4: Add the deck-B input bindings** to the deck-B input section (same notes on `0x91`):

```
        // Pad-mode selector buttons (deck channel 0x91) — mirror of deck A.
        InputBinding(status: 0x91, data1: 0x1B, target: PadModeSelect(B, HotCue)),
        InputBinding(status: 0x91, data1: 0x69, target: PadModeSelect(B, HotCueShift)),
        InputBinding(status: 0x91, data1: 0x1E, target: PadModeSelect(B, PadFx1)),
        InputBinding(status: 0x91, data1: 0x6B, target: PadModeSelect(B, PadFx2)),
        InputBinding(status: 0x91, data1: 0x20, target: PadModeSelect(B, BeatJump)),
        InputBinding(status: 0x91, data1: 0x6D, target: PadModeSelect(B, BeatLoop)),
        InputBinding(status: 0x91, data1: 0x22, target: PadModeSelect(B, Sampler)),
        InputBinding(status: 0x91, data1: 0x6F, target: PadModeSelect(B, SamplerShift)),
```

- [ ] **Step 5: Run the test + the RON schema-drift guard**

Run: `source "$HOME/.cargo/env" && cargo test --workspace flx4_decodes_pad_mode_selectors every_builtin_parses`
Expected: both PASS (bindings decode; RON still parses in-build).

- [ ] **Step 6: Full gate + commit**

```bash
source "$HOME/.cargo/env" && cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add profiles/pioneer-ddj-flx4.ron src/profiles.rs
git commit -m "feat(flx4): decode pad-mode selector buttons on both decks"
```

---

### Task 3: `FeedbackState.pad_mode` + `PadModeLed` render

**Files:**
- Modify: `src/feedback.rs` (import `PadMode`; add `FeedbackState.pad_mode`; add `FeedbackSource::PadModeLed`; add `render()` arm; add unit tests)

**Interfaces:**
- Consumes: `PadMode` (Task 1).
- Produces: `FeedbackState.pad_mode: [PadMode; 2]`; `FeedbackSource::PadModeLed(Deck, PadMode)` rendering `0x7F` selected / `0x20` otherwise.

- [ ] **Step 1: Import `PadMode`** — extend the `use crate::{...}` at the top of `src/feedback.rs` to include `PadMode` alongside `Deck`.

- [ ] **Step 2: Add the state field** to `FeedbackState` (after `stem_soloed`):

```rust
    /// Per-deck selected pad mode (FLX4 pad-mode cluster LEDs). Default
    /// `HotCue` matches the controller's power-on lamp.
    pub pad_mode: [PadMode; 2],
```

- [ ] **Step 3: Add the `FeedbackSource` variant** (after `SavedLoopSlot` / near `BeatFxOn`):

```rust
    /// Pad-mode selector LED (FLX4 mode cluster): bright (`0x7F`) when this mode
    /// is the deck's selected pad mode, dim (`0x20`) otherwise. The unselected
    /// buttons stay dimly lit on the hardware, never fully off.
    PadModeLed(Deck, PadMode),
```

- [ ] **Step 4: Add the `render()` arm** in the `match rule.source` inside `render()` (alongside the other LED arms):

```rust
                FeedbackSource::PadModeLed(d, mode) => {
                    if state.pad_mode[idx(d)] == mode {
                        0x7F
                    } else {
                        0x20
                    }
                }
```

- [ ] **Step 5: Write the failing tests** in `src/feedback.rs` `mod tests`:

```rust
    #[test]
    fn renders_pad_mode_bright_selected_dim_others() {
        let rules = [
            FeedbackRule { source: FeedbackSource::PadModeLed(Deck::A, PadMode::HotCue), status: 0x90, data1: 0x1B },
            FeedbackRule { source: FeedbackSource::PadModeLed(Deck::A, PadMode::PadFx1), status: 0x90, data1: 0x1E },
        ];
        let st = FeedbackState { pad_mode: [PadMode::PadFx1, PadMode::HotCue], ..Default::default() };
        let frame = render(&rules, &st);
        assert!(frame.contains(&[0x90, 0x1B, 0x20]), "Hot Cue dim (not selected)");
        assert!(frame.contains(&[0x90, 0x1E, 0x7F]), "Pad FX1 bright (selected)");
    }

    #[test]
    fn switching_pad_mode_diffs_only_the_two_changed_lamps() {
        let rules = [
            FeedbackRule { source: FeedbackSource::PadModeLed(Deck::A, PadMode::HotCue), status: 0x90, data1: 0x1B },
            FeedbackRule { source: FeedbackSource::PadModeLed(Deck::A, PadMode::PadFx1), status: 0x90, data1: 0x1E },
        ];
        let mut diff = FeedbackDiff::new();
        let start = FeedbackState { pad_mode: [PadMode::HotCue, PadMode::HotCue], ..Default::default() };
        let _ = diff.changed(&render(&rules, &start)); // prime
        let moved = FeedbackState { pad_mode: [PadMode::PadFx1, PadMode::HotCue], ..Default::default() };
        let changed = diff.changed(&render(&rules, &moved));
        assert_eq!(changed.len(), 2);
        assert!(changed.contains(&[0x90, 0x1B, 0x20]));
        assert!(changed.contains(&[0x90, 0x1E, 0x7F]));
    }
```

- [ ] **Step 6: Run to verify PASS** (implementation from Steps 2–4 is already in place)

Run: `source "$HOME/.cargo/env" && cargo test --workspace renders_pad_mode switching_pad_mode`
Expected: both PASS.

- [ ] **Step 7: Full gate + commit**

```bash
source "$HOME/.cargo/env" && cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add src/feedback.rs
git commit -m "feat(feedback): PadModeLed source + pad_mode state (bright/dim render)"
```

---

### Task 4: FLX4 feedback rules + device-doc update

**Files:**
- Modify: `profiles/pioneer-ddj-flx4.ron` (add feedback rules)
- Modify: `src/profiles.rs` (feedback integration test)
- Modify: `docs/devices/pioneer-ddj-flx4.md` (verified notes + LED addresses)

**Interfaces:**
- Consumes: `FeedbackSource::PadModeLed`, `FeedbackState.pad_mode` (Task 3).

- [ ] **Step 1: Write the failing feedback integration test** in `src/profiles.rs`:

```rust
    #[test]
    fn flx4_feedback_renders_pad_mode_leds() {
        use crate::{FeedbackState, PadMode};
        let p = builtin_for_port("DDJ-FLX4").unwrap();
        let state = FeedbackState { pad_mode: [PadMode::BeatJump, PadMode::HotCue], ..Default::default() };
        let frame = p.render_feedback(&state);
        // Deck A in Beat Jump → Beat Jump bright, Hot Cue dim.
        assert!(frame.contains(&[0x90, 0x20, 0x7F]));
        assert!(frame.contains(&[0x90, 0x1B, 0x20]));
        // Deck B default Hot Cue → Hot Cue bright, Sampler dim.
        assert!(frame.contains(&[0x91, 0x1B, 0x7F]));
        assert!(frame.contains(&[0x91, 0x22, 0x20]));
    }
```

- [ ] **Step 2: Run to confirm it fails**

Run: `source "$HOME/.cargo/env" && cargo test --workspace flx4_feedback_renders_pad_mode_leds`
Expected: FAIL (no rules emit those addresses yet).

- [ ] **Step 3: Add the feedback rules** to the `feedback: [ ... ]` list in `profiles/pioneer-ddj-flx4.ron` (after the Beat FX ON/OFF rule):

```
        // Pad-mode cluster LEDs — bright (0x7F) selected / dim (0x20) available.
        // Primary addresses hardware-verified 2026-07-17; the shift addresses
        // (0x69/0x6B/0x6D/0x6F) drive the shared lamp — verify the lit style live.
        FeedbackRule(source: PadModeLed(A, HotCue),       status: 0x90, data1: 0x1B),
        FeedbackRule(source: PadModeLed(A, HotCueShift),  status: 0x90, data1: 0x69),
        FeedbackRule(source: PadModeLed(A, PadFx1),       status: 0x90, data1: 0x1E),
        FeedbackRule(source: PadModeLed(A, PadFx2),       status: 0x90, data1: 0x6B),
        FeedbackRule(source: PadModeLed(A, BeatJump),     status: 0x90, data1: 0x20),
        FeedbackRule(source: PadModeLed(A, BeatLoop),     status: 0x90, data1: 0x6D),
        FeedbackRule(source: PadModeLed(A, Sampler),      status: 0x90, data1: 0x22),
        FeedbackRule(source: PadModeLed(A, SamplerShift), status: 0x90, data1: 0x6F),
        FeedbackRule(source: PadModeLed(B, HotCue),       status: 0x91, data1: 0x1B),
        FeedbackRule(source: PadModeLed(B, HotCueShift),  status: 0x91, data1: 0x69),
        FeedbackRule(source: PadModeLed(B, PadFx1),       status: 0x91, data1: 0x1E),
        FeedbackRule(source: PadModeLed(B, PadFx2),       status: 0x91, data1: 0x6B),
        FeedbackRule(source: PadModeLed(B, BeatJump),     status: 0x91, data1: 0x20),
        FeedbackRule(source: PadModeLed(B, BeatLoop),     status: 0x91, data1: 0x6D),
        FeedbackRule(source: PadModeLed(B, Sampler),      status: 0x91, data1: 0x22),
        FeedbackRule(source: PadModeLed(B, SamplerShift), status: 0x91, data1: 0x6F),
```

- [ ] **Step 4: Run the test + parse guard**

Run: `source "$HOME/.cargo/env" && cargo test --workspace flx4_feedback_renders_pad_mode_leds every_builtin_parses`
Expected: both PASS.

- [ ] **Step 5: Update the device doc** `docs/devices/pioneer-ddj-flx4.md`:
  - In the inputs table, change the "Pad-mode select" row note to: **verified 2026-07-17**, listing all 8 modes (primary `0x1B`/`0x1E`/`0x20`/`0x22`, shift `0x69`/`0x6B`/`0x6D`/`0x6F`).
  - In the LED outputs table, update the "Pad-mode LEDs" row: bright `0x7F` selected / dim `0x20` available; addresses per mode; note the shift-address lit-style is pending live confirmation.
  - Correct the "mutually exclusive — the unit keeps only one lit" claim: the hardware does **not** self-manage; software drives selected-bright / others-dim.

- [ ] **Step 6: Full gate + commit**

```bash
source "$HOME/.cargo/env" && cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
git add profiles/pioneer-ddj-flx4.ron src/profiles.rs docs/devices/pioneer-ddj-flx4.md
git commit -m "feat(flx4): pad-mode cluster LED feedback rules + verified-note docs"
```

- [ ] **Step 7: Push + open PR** (per Tom's "push + open PR, don't local-merge" preference)

```bash
git push -u origin feat/flx4-pad-mode-leds
gh pr create --fill --title "feat(flx4): pad-mode cluster LED indicator"
```

Wait for aurum-control CI to go green.

---

### Task 5: Hardware-verify the LED addresses (interactive, with Tom)

Do this against the physical FLX4 before final RON lock (may amend Task 4's commit / add a follow-up commit on the branch). Uses the built `ctl` monitor is input-only, so verification is by **sending** candidate LED bytes and Tom watching the lamps.

- [ ] **Step 1:** With the Pro build running (or a small send-test), light each primary mode in turn; confirm `0x1B`/`0x1E`/`0x20`/`0x22` bright lights the expected button and `0x20` dim matches the resting glow.
- [ ] **Step 1b (whole-branch review finding — do NOT skip):** Test a **primary** mode selected (e.g. Hot Cue), not only the shift modes. The branch ships 8 *distinct* LED addresses ordered primary-then-shift, so there is no conflict while the shift addresses are inert. But if Step 2 finds a shift address *does* drive the shared lamp, the current order makes the host send `0x1B=0x7F` then `0x69=0x20` for a selected primary → last-write-wins leaves the **primary lamp dim**. The shift-selected case survives by luck; the primary case is the one that breaks. So the live check must confirm a *primary* mode lights bright.
- [ ] **Step 2:** Enter Pad FX2 and Beat-Loop; confirm whether the shift addresses (`0x6B`/`0x6D`) give a *distinct* lamp style vs simply lighting the parent. If they do nothing distinct, repoint the 4 shift feedback rules at the parent addresses (`0x1E`, `0x20`, `0x1B`, `0x22`) or **drop them entirely** — **RON data change only, no code change**. **Do NOT leave two rules with the same `(status, data1)`**: `render()` emits both and `FeedbackDiff` is last-write-wins, so a duplicate address is non-deterministic. Dropping the 4 shift rules (leaving the primary rule as the sole driver of each lamp) is the clean resolution if the lamp is shared and undifferentiated.
- [ ] **Step 3:** Tune the DIM velocity if `0x20` reads too bright/dark; update the `render()` constant and the RON comment together if changed.
- [ ] **Step 4:** Commit any RON/doc adjustments; ensure CI green.

---

## Phase 2 — aurum-pro (host wiring)

Runs **after** Phase 1 merges. Create a fresh pro worktree:
`git worktree add ../aurum-pro-wt-padmode -b feat/flx4-pad-mode-leds origin/main` (then `cargo clean -p tauri -p tauri-build` before the first tauri build; `npm install` if `package.json` moved).

### Task 6: Bump the aurum-control dependency

**Files:** Modify `Cargo.toml` (the `midi`/aurum-control git dep `rev`), refresh `Cargo.lock`.

- [ ] **Step 1:** Set the `midi` git dependency `rev` to the merged Phase-1 commit SHA on `aurum-control` `main`.
- [ ] **Step 2:** `source "$HOME/.cargo/env" && cargo update -p midi --precise <sha>` (or edit `rev` + `cargo build -p midi`) so `Cargo.lock` resolves.
- [ ] **Step 3:** Commit: `git add Cargo.toml Cargo.lock && git commit -m "chore(deps): bump aurum-control for pad-mode LEDs"`.

### Task 7: Track pad mode and feed the LEDs

**Files:** Modify `src-tauri/src/midi_service.rs`.

**Interfaces:**
- Consumes: `Target::PadModeSelect(Deck, PadMode)`, `PadMode`, `FeedbackState.pad_mode` (Phase 1).

- [ ] **Step 1: Add shared per-deck pad-mode state.** Near the feedback-loop setup, create `let pad_mode = Arc::new([AtomicU8::new(0), AtomicU8::new(0)]);` (0 == `PadMode::HotCue`). Clone one handle for the input thread and one for `spawn_feedback_loop`.

- [ ] **Step 2: Intercept `PadModeSelect` on input.** In the profile-decode dispatch `match action.target` block (the one that special-cases `Target::LibraryScroll | LibraryOpen | LoadDeck`), add — *before* the pro-gate / `apply_action` — an arm:

```rust
Target::PadModeSelect(d, m) => {
    // Latching display-state only: record on press, never touch the engine.
    if let ActionValue::Absolute(v) = action.value {
        if v >= 0.5 {
            let idx = if matches!(d, midi::Deck::A) { 0 } else { 1 };
            pad_mode_in[idx].store(pad_mode_code(m), Ordering::Relaxed);
        }
    }
}
```

where `pad_mode_code(PadMode) -> u8` maps the 8 variants to `0..=7` and `pad_mode_from_code(u8) -> PadMode` is its inverse (add both as small free fns in this module). `pad_mode_in` is the input thread's Arc clone.

- [ ] **Step 3: Feed it into `FeedbackState`.** Thread the feedback-thread Arc clone into `spawn_feedback_loop` (add a `pad_mode: Arc<[AtomicU8; 2]>` param) and set the new field in the `FeedbackState { .. }` literal:

```rust
pad_mode: [
    pad_mode_from_code(pad_mode[0].load(Ordering::Relaxed)),
    pad_mode_from_code(pad_mode[1].load(Ordering::Relaxed)),
],
```

- [ ] **Step 4: Confirm `PadModeSelect` is NOT pro-gated.** Verify `is_pro_target` (in the `aurum-api`/engine crate) does not classify `PadModeSelect` as pro — it is free controller ergonomics. Because the input arm returns before the gate, it is already exempt, but do not add it to the pro list.

- [ ] **Step 5: Full local gate**

Run: `source "$HOME/.cargo/env" && cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && npx tsc --noEmit && npx vitest run`
Expected: all green (models must be present for the tauri crate — symlink from the host checkout).

- [ ] **Step 6: Commit + push + PR**

```bash
git add src-tauri/src/midi_service.rs
git commit -m "feat(flx4): drive pad-mode cluster LEDs from selected mode"
git push -u origin feat/flx4-pad-mode-leds
gh pr create --fill --title "feat(flx4): pad-mode LED indicator (host wiring)"
```

### Task 8: Hardware acceptance

- [ ] Run the Pro build with the FLX4 connected. Press Hot Cue / Pad FX1 / Beat Jump / Sampler (and the shift variants) on **both** decks; confirm the lit button tracks the selected mode and the others dim. Confirm no audio/engine side effects. Note any DIM-velocity or shift-lamp tweaks back onto the Phase-1 RON if needed.

---

## Self-Review

**Spec coverage:** all 8 modes modelled (Task 1 enum) ✓; decode both decks (Task 2) ✓; bright/dim render mirroring `SavedLoopSlot` (Task 3) ✓; RON rules + doc correction (Task 4) ✓; hardware-verify of addresses/shift-lamp/dim level (Task 5) ✓; pro host tracks + forwards mode, no engine change, not pro-gated (Tasks 6–7) ✓; two-repo rollout with CI/local gate (Task 4 push, Phase 2) ✓.

**Placeholder scan:** every code step shows real code; the only deferred value is the Phase-2 dep SHA (unknowable until Phase 1 merges) and the hardware-verified DIM/shift-lamp specifics (Task 5's explicit purpose) — both are genuine sequencing gates, not vague TODOs.

**Type consistency:** `PadMode` variant names (`HotCue`/`HotCueShift`/`PadFx1`/`PadFx2`/`BeatJump`/`BeatLoop`/`Sampler`/`SamplerShift`), `Target::PadModeSelect(Deck, PadMode)`, `FeedbackSource::PadModeLed(Deck, PadMode)`, `FeedbackState.pad_mode`, and the `0x7F`/`0x20` velocities are used identically across all tasks and the RON.
