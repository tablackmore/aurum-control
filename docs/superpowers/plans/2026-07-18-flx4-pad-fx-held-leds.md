# FLX4 Pad FX "light while held" LEDs — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Light a FLX4 Pad FX pad (`0x10–0x1F` on `0x97`/`0x99`) bright while it's held, dark on release — a generic held-note echo, with the controller layer staying device-agnostic.

**Architecture:** `aurum-control` gains a `Copy` `HeldNotes` bitset on `FeedbackState` and a `FeedbackSource::HeldEcho` that lights a rule's address when it's in the held-set; the FLX4 RON declares the 32 echo rules. `aurum-pro` tracks every held note generically in the input callback and copies the set into `FeedbackState` each frame.

**Tech Stack:** Rust (`aurum-control` lib + RON; `aurum-pro` src-tauri), `cargo test`, GPG-signed Conventional Commits.

## Global Constraints

- **Worktrees only.** Phase A: `/Users/tomblackmore/aurum/aurum-control-wt-padfx` (branch `feat/flx4-pad-fx-held-leds`, created). Phase B: a fresh `aurum-pro` worktree after Phase A merges.
- **`aurum-control` public → NO AI/assistant attribution** in commits. `aurum-pro` closed → normal attribution.
- **Conventional Commits**, **GPG-signed** (bare `git commit`).
- **Rust gate before every commit:** `cargo fmt --all`; clippy under the CI toolchain `cargo +stable clippy --all-targets -- -D warnings` **and** `--features harness`; `cargo test --workspace`. Prefix cargo with `source "$HOME/.cargo/env" &&`.
- **`aurum-control` has real CI** (must be green before merge). **`aurum-pro` merge gate is the full local gate** (fmt/clippy/`cargo test --workspace`/`tsc`/`vitest` — run vitest **serially**, not concurrently with cargo test).
- **LED convention:** held → `0x7F`, not held → `0x00`.
- **HeldNotes index:** `(status & 0x0F) * 128 + note`, into a `[u64; 32]` (2048 bits).

## File Structure

**Phase A — `aurum-control` (`aurum-control-wt-padfx/`):**
- `src/feedback.rs` — `HeldNotes` type; `FeedbackState.held`; `FeedbackSource::HeldEcho`; render arm; tests.
- `src/lib.rs` — export `HeldNotes`.
- `profiles/pioneer-ddj-flx4.ron` — 32 `HeldEcho` rules.
- `src/profiles.rs` — feedback test.
- `docs/devices/pioneer-ddj-flx4.md` — Pad FX LED row.

**Phase B — `aurum-pro` (fresh worktree):**
- `src-tauri/src/midi_service.rs` — shared `HeldNotes`, input tracking, feedback fill.
- `Cargo.lock` — dep bump.

---

## Phase A — aurum-control

### Task A1: `HeldNotes` + `FeedbackState.held` + `HeldEcho` render

**Files:** Modify `src/feedback.rs`, `src/lib.rs`.

**Interfaces:**
- Produces: `HeldNotes` (Copy, Default) with `set(u8,u8,bool)` / `contains(u8,u8)->bool`; `FeedbackState.held: HeldNotes`; `FeedbackSource::HeldEcho`.

- [ ] **Step 1: Add `HeldNotes`** in `src/feedback.rs` (after the `use`s):

```rust
/// A compact, `Copy` set of currently-held note addresses — one bit per
/// `(note-channel, note)`. Used to echo a controller pad's LED while it is held.
/// Index = `(status & 0x0F) * 128 + note` into 2048 bits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeldNotes {
    bits: [u64; 32],
}

impl Default for HeldNotes {
    fn default() -> Self {
        HeldNotes { bits: [0; 32] }
    }
}

impl HeldNotes {
    fn index(status: u8, note: u8) -> usize {
        ((status & 0x0F) as usize) * 128 + (note & 0x7F) as usize
    }

    /// Mark `(status, note)` held or released.
    pub fn set(&mut self, status: u8, note: u8, held: bool) {
        let i = Self::index(status, note);
        let (w, b) = (i / 64, i % 64);
        if held {
            self.bits[w] |= 1 << b;
        } else {
            self.bits[w] &= !(1 << b);
        }
    }

    /// Whether `(status, note)` is currently held.
    pub fn contains(&self, status: u8, note: u8) -> bool {
        let i = Self::index(status, note);
        (self.bits[i / 64] >> (i % 64)) & 1 == 1
    }
}
```

- [ ] **Step 2: Add the state field** to `FeedbackState` (after `hot_cue`):

```rust
    /// Currently-held note addresses, for pads that echo their LED while held
    /// (FLX4 Pad FX pads). The app updates this from raw note-on/off.
    pub held: HeldNotes,
```

- [ ] **Step 3: Add the source variant** (near `HotCueSlot`):

```rust
    /// Echo a pad's LED while its note address is held: bright (`0x7F`) when
    /// `FeedbackState::held` contains this rule's own `(status, data1)`, else off.
    HeldEcho,
```

- [ ] **Step 4: Add the render arm** in the `match r.source` inside `render_with_palette` (the closure binds `r`, so `r.status`/`r.data1` are in scope):

```rust
                FeedbackSource::HeldEcho => {
                    if state.held.contains(r.status, r.data1) {
                        0x7F
                    } else {
                        0x00
                    }
                }
```

- [ ] **Step 5: Export** — add `HeldNotes` to the `pub use feedback::{...}` list in `src/lib.rs`.

- [ ] **Step 6: Tests** in `src/feedback.rs`:

```rust
    #[test]
    fn held_notes_set_contains_clear() {
        let mut h = HeldNotes::default();
        assert!(!h.contains(0x97, 0x10));
        h.set(0x97, 0x10, true);
        assert!(h.contains(0x97, 0x10));
        assert!(!h.contains(0x99, 0x10), "different channel is independent");
        assert!(!h.contains(0x97, 0x11), "different note is independent");
        h.set(0x97, 0x10, false);
        assert!(!h.contains(0x97, 0x10));
    }

    #[test]
    fn held_echo_renders_bright_only_when_held() {
        let rules = [FeedbackRule { source: FeedbackSource::HeldEcho, status: 0x97, data1: 0x10 }];
        let mut st = FeedbackState::default();
        assert!(render(&rules, &st).contains(&[0x97, 0x10, 0x00]), "not held → off");
        st.held.set(0x97, 0x10, true);
        assert!(render(&rules, &st).contains(&[0x97, 0x10, 0x7F]), "held → bright");
    }
```

- [ ] **Step 7: Gate + commit**

```bash
source "$HOME/.cargo/env" && cargo fmt --all && cargo +stable clippy --all-targets -- -D warnings && cargo +stable clippy --all-targets --features harness -- -D warnings && cargo test --workspace
git add src/feedback.rs src/lib.rs
git commit -m "feat(feedback): HeldNotes + HeldEcho source for hold-to-light pads"
```

---

### Task A2: FLX4 Pad FX HeldEcho rules + doc

**Files:** Modify `profiles/pioneer-ddj-flx4.ron`, `src/profiles.rs`, `docs/devices/pioneer-ddj-flx4.md`.

- [ ] **Step 1: Failing feedback test** in `src/profiles.rs`:

```rust
    #[test]
    fn flx4_feedback_renders_held_pad_fx_pads() {
        use crate::FeedbackState;
        let p = builtin_for_port("DDJ-FLX4").unwrap();
        let mut state = FeedbackState::default();
        state.held.set(0x97, 0x13, true); // deck A brake pad held
        state.held.set(0x99, 0x1F, true); // deck B last FX pad held
        let frame = p.render_feedback(&state);
        assert!(frame.contains(&[0x97, 0x13, 0x7F]), "held FX pad bright");
        assert!(frame.contains(&[0x97, 0x10, 0x00]), "un-held FX pad off");
        assert!(frame.contains(&[0x99, 0x1F, 0x7F]));
    }
```

- [ ] **Step 2: Run — expect FAIL** (no HeldEcho rules yet).

Run: `source "$HOME/.cargo/env" && cargo test --workspace flx4_feedback_renders_held_pad_fx_pads`
Expected: FAIL.

- [ ] **Step 3: Add 32 `HeldEcho` rules** to the `feedback: [ ... ]` list in the RON (after the hot-cue rules). Deck A on `0x97`, deck B on `0x99`, notes `0x10–0x1F`:

```
        // Pad FX pads — echo the pad LED bright while held (0x10–0x1F, both FX
        // layers). Single-colour on the FLX4. Held state comes from the app's
        // generic note tracking; `HeldEcho` lights a rule when its own address
        // is held.
        FeedbackRule(source: HeldEcho, status: 0x97, data1: 0x10),
        FeedbackRule(source: HeldEcho, status: 0x97, data1: 0x11),
        FeedbackRule(source: HeldEcho, status: 0x97, data1: 0x12),
        FeedbackRule(source: HeldEcho, status: 0x97, data1: 0x13),
        FeedbackRule(source: HeldEcho, status: 0x97, data1: 0x14),
        FeedbackRule(source: HeldEcho, status: 0x97, data1: 0x15),
        FeedbackRule(source: HeldEcho, status: 0x97, data1: 0x16),
        FeedbackRule(source: HeldEcho, status: 0x97, data1: 0x17),
        FeedbackRule(source: HeldEcho, status: 0x97, data1: 0x18),
        FeedbackRule(source: HeldEcho, status: 0x97, data1: 0x19),
        FeedbackRule(source: HeldEcho, status: 0x97, data1: 0x1A),
        FeedbackRule(source: HeldEcho, status: 0x97, data1: 0x1B),
        FeedbackRule(source: HeldEcho, status: 0x97, data1: 0x1C),
        FeedbackRule(source: HeldEcho, status: 0x97, data1: 0x1D),
        FeedbackRule(source: HeldEcho, status: 0x97, data1: 0x1E),
        FeedbackRule(source: HeldEcho, status: 0x97, data1: 0x1F),
        FeedbackRule(source: HeldEcho, status: 0x99, data1: 0x10),
        FeedbackRule(source: HeldEcho, status: 0x99, data1: 0x11),
        FeedbackRule(source: HeldEcho, status: 0x99, data1: 0x12),
        FeedbackRule(source: HeldEcho, status: 0x99, data1: 0x13),
        FeedbackRule(source: HeldEcho, status: 0x99, data1: 0x14),
        FeedbackRule(source: HeldEcho, status: 0x99, data1: 0x15),
        FeedbackRule(source: HeldEcho, status: 0x99, data1: 0x16),
        FeedbackRule(source: HeldEcho, status: 0x99, data1: 0x17),
        FeedbackRule(source: HeldEcho, status: 0x99, data1: 0x18),
        FeedbackRule(source: HeldEcho, status: 0x99, data1: 0x19),
        FeedbackRule(source: HeldEcho, status: 0x99, data1: 0x1A),
        FeedbackRule(source: HeldEcho, status: 0x99, data1: 0x1B),
        FeedbackRule(source: HeldEcho, status: 0x99, data1: 0x1C),
        FeedbackRule(source: HeldEcho, status: 0x99, data1: 0x1D),
        FeedbackRule(source: HeldEcho, status: 0x99, data1: 0x1E),
        FeedbackRule(source: HeldEcho, status: 0x99, data1: 0x1F),
```

- [ ] **Step 4: Run test + parse guard**

Run: `source "$HOME/.cargo/env" && cargo test --workspace flx4_feedback_renders_held_pad_fx_pads every_builtin_parses`
Expected: both PASS.

- [ ] **Step 5: Update the device doc** `docs/devices/pioneer-ddj-flx4.md` — add a Pad FX pad LED row to the outputs table: notes `0x10–0x1F` on `0x97`/`0x99`, driven by `HeldEcho` rules — bright while the pad is held, off otherwise; single-colour.

- [ ] **Step 6: Full gate + commit + push + PR**

```bash
source "$HOME/.cargo/env" && cargo fmt --all && cargo +stable clippy --all-targets -- -D warnings && cargo +stable clippy --all-targets --features harness -- -D warnings && cargo test --workspace
git add profiles/pioneer-ddj-flx4.ron src/profiles.rs docs/devices/pioneer-ddj-flx4.md
git commit -m "feat(flx4): Pad FX pad hold-to-light LED rules"
git push -u origin feat/flx4-pad-fx-held-leds
gh pr create --fill --title "feat(flx4): Pad FX pads light while held"
```

Wait for CI green, then squash-merge (`--admin`), note the merge SHA for Phase B.

---

## Phase B — aurum-pro

Runs after Phase A merges. Fresh worktree: `git worktree add ../aurum-pro-wt-padfx -b feat/flx4-pad-fx-held-leds origin/main`; then `cargo clean -p tauri -p tauri-build`, `npm install`, symlink model dirs (`ln -sfn ../pro/crates/{separation,analysis}/models crates/{separation,analysis}/models`).

### Task B1: dep bump + generic held-note tracking + feedback fill

**Files:** `Cargo.lock`, `src-tauri/src/midi_service.rs`.

- [ ] **Step 1: Dep bump** (own commit): `CARGO_NET_GIT_FETCH_WITH_CLI=true cargo update -p aurum-control --precise <merged Phase-A SHA>`; confirm `Cargo.lock`; commit `chore(deps): bump aurum-control for Pad FX hold LEDs`.

- [ ] **Step 2: Shared held state in `connect()`.** Create `let held = Arc::new(Mutex::new(midi::HeldNotes::default()));` before the feedback-handle block; clone one for the feedback thread and one (`held_in`) for the input callback (a `move` closure — separate clones, like the pad-mode `pad_mode`/`pad_mode_in` pattern).

- [ ] **Step 3: Track held notes in the input callback.** In the callback that has raw `bytes`, before/independent of decode, update the held set from note-on/off. Use the crate's `parse(bytes)` (already imported) to get a `MidiMessage`, then:

```rust
                    match midi::parse(bytes) {
                        Some(midi::MidiMessage::NoteOn { channel, note, velocity }) => {
                            // velocity 0 == note-off
                            held_in.lock().unwrap().set(0x90 | channel, note, velocity > 0);
                        }
                        Some(midi::MidiMessage::NoteOff { channel, note }) => {
                            held_in.lock().unwrap().set(0x90 | channel, note, false);
                        }
                        _ => {}
                    }
```

Place this so it runs for profile-mode messages (it is additive telemetry — it must not swallow or gate the normal decode/dispatch that follows). Reconstruct the status byte as `0x90 | channel` to match how the profile's feedback rules are addressed (`0x97`/`0x99`). (If `MidiMessage`'s variant/field names differ, match the real definition in `message.rs`.)

- [ ] **Step 4: Copy into `FeedbackState`.** Pass the feedback-thread `held` clone into `spawn_feedback_loop` (new param `held: Arc<Mutex<HeldNotes>>`); in the `FeedbackState { … }` literal add:

```rust
held: *held.lock().unwrap(),
```

- [ ] **Step 5: Tests** (midi_service.rs `#[cfg(test)]`): a focused test that `HeldNotes` round-trips a note the way the callback uses it (`set(0x90 | channel, note, true)` then `contains`), proving the status reconstruction lines up with the `0x97`/`0x99` feedback addresses.

- [ ] **Step 6: Full local gate** (serial vitest) → commit `feat(flx4): track held pads → light Pad FX pads while held`. **Do NOT merge** — push and open the PR, leave it for review.

### Task B2: Hardware acceptance (with Tom)

- [ ] Pro build + FLX4: in Pad FX mode, press/hold FX pads on both decks; confirm each lights bright while held and darkens on release; confirm no stuck-on pads (note-off clears).

---

## Self-Review

**Spec coverage:** hold-to-light echo (A1 render + A2 rules) ✓; device-agnostic pro tracking (B1 generic note parse) ✓; both decks + both FX layers `0x10–0x1F` (A2 32 rules) ✓; dep-bump rollout (B1) ✓; single-colour acknowledged (A2 doc) ✓.

**Placeholder scan:** only deferred value is Phase B's dep SHA (unknowable until Phase A merges) — a genuine sequencing gate.

**Type consistency:** `HeldNotes` (`set(status,note,bool)`/`contains(status,note)`), `FeedbackState.held`, `FeedbackSource::HeldEcho`, and the `(status,data1)` addressing are used identically across `render`, the RON rules, and the pro fill. The pro callback reconstructs `status = 0x90 | channel` to match the RON's `0x97`/`0x99`.
