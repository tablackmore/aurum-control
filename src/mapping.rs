//! MIDI-learn mapping: bind an incoming control to a target parameter with
//! per-binding behaviour (absolute / relative-encoder / button modes, invert,
//! output range, soft-takeover), then resolve subsequent messages to an
//! [`Action`]. Persistable as versioned JSON.

use crate::{decode_relative, HighResDecoder, MidiMessage, RelEncoding};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Soft-takeover catch tolerance: accept once within 3/128 of the software value
/// (matches Mixxx's `kDefaultTakeoverThreshold`).
const TAKEOVER_THRESHOLD: f32 = 3.0 / 128.0;
/// Relative-encoder sensitivity: one tick = 1/128 of the parameter range.
const REL_STEP: f32 = 1.0 / 128.0;
/// Current on-disk schema version for a persisted map.
const SCHEMA_VERSION: u32 = 2;

/// Schema-2 on-disk shape.
#[derive(Serialize, Deserialize)]
struct Persisted {
    schema: u32,
    bindings: Vec<Binding>,
}

/// Legacy Phase-1 on-disk shape (untagged tuples), for migration only.
#[derive(Deserialize)]
struct LegacyMap {
    bindings: Vec<(ControlId, Target)>,
}

/// Which deck a target addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum Deck {
    A,
    B,
}

impl Deck {
    fn tag(self) -> &'static str {
        match self {
            Deck::A => "A",
            Deck::B => "B",
        }
    }
}

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

impl PadMode {
    /// The FLX4 has 4 physical pad-mode buttons, and each primary mode shares
    /// its button's lamp with its shift variant (Pad FX1/Pad FX2 are one lamp,
    /// Beat Jump/Beat-Loop are one lamp, etc. — hardware-confirmed 2026-07-17).
    /// So a lamp lights for a *family* of two modes, not a single mode.
    fn button(self) -> u8 {
        use PadMode::*;
        match self {
            HotCue | HotCueShift => 0,
            PadFx1 | PadFx2 => 1,
            BeatJump | BeatLoop => 2,
            Sampler | SamplerShift => 3,
        }
    }

    /// Whether these two modes share the same physical button lamp — i.e. a
    /// primary and its shift variant.
    pub fn same_button(self, other: PadMode) -> bool {
        self.button() == other.button()
    }
}

/// How a target reacts to incoming values — used to pick a sensible default mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    /// 0..1 continuous parameter (knob/fader).
    Continuous,
    /// On/off state that flips on press (button → LED).
    Toggle,
    /// Fires once on the press edge (hot cue, loop in/out, beat-jump).
    Trigger,
    /// Acts on both edges: press → `1.0`, release → `0.0` (hold-to-play, e.g. a
    /// slip hot cue that plays while held and returns on release).
    Momentary,
}

/// A controllable parameter (the deck-selectable param space).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Target {
    // ── Continuous ───────────────────────────────────────────────
    StemVolume(Deck, u8),
    EqLow(Deck),
    EqMid(Deck),
    EqHigh(Deck),
    Pan(Deck),
    ChannelVolume(Deck),
    Trim(Deck),
    Tempo(Deck),
    Seek(Deck),
    Crossfade,
    Master,
    CueMix,
    HeadphoneLevel,
    // ── Toggles ──────────────────────────────────────────────────
    StemMute(Deck, u8),
    StemSolo(Deck, u8),
    Play(Deck),
    /// Beat-sync toggle for this deck (the pressed deck becomes the single
    /// follower). Fires per press like a trigger: the HOST flips real engine
    /// state, because sync also changes from the UI, follower swaps and
    /// auto-mix — a mapping-side latch would go stale (same lesson as
    /// `LoopToggle`).
    Sync(Deck),
    Keylock(Deck),
    /// Set-time snap (grid-align cue/loop *positions* when placed).
    Quantize(Deck),
    /// Trigger-time launch quantize (grid-lock cue *jumps*).
    TriggerQuantize(Deck),
    /// Slip mode (held loops/cues play momentarily, playback returns in place).
    Slip(Deck),
    CueMonitor(Deck),
    /// Master headphone-cue: routes the master mix onto the CUE side of the
    /// headphone blend (FLX4 "MASTER CUE" button).
    CueMaster,
    LoopToggle(Deck),
    // ── Triggers ─────────────────────────────────────────────────
    HotCue(Deck, u8),
    HotCueClear(Deck, u8),
    // ── Momentary ────────────────────────────────────────────────
    /// Hold a hot cue: press plays from it, release returns to the ghost (slip).
    /// With slip off the engine treats the press as a plain jump and the release
    /// as a no-op, so it is safe to bind unconditionally.
    HotCueHold(Deck, u8),
    LoopIn(Deck),
    LoopOut(Deck),
    LoopHalve(Deck),
    LoopDouble(Deck),
    /// Jump by `beats` (negative = backwards).
    BeatJump(Deck, f32),
    /// Set an active loop of `beats` length.
    LoopSet(Deck, f32),
    /// One-button 4-beat auto-loop: sets a 4-beat loop at the playhead, or
    /// exits if a loop is already active (FLX4 "4 BEAT/EXIT").
    LoopFourOrExit(Deck),
    /// Re-activate the last-used loop region (FLX4 shift "ACTIVE").
    LoopReloop(Deck),
    /// Save the current loop region into the deck's memory (FLX4 shift "MEMORY").
    LoopSave(Deck),
    /// Recall the previous saved loop (FLX4 "CUE/LOOP CALL ◄").
    LoopCallPrev(Deck),
    /// Recall the next saved loop (FLX4 "CUE/LOOP CALL ►").
    LoopCallNext(Deck),
    /// Delete the selected saved loop (FLX4 shift "DEL").
    LoopDelete(Deck),
    /// Hot-cue-style save-or-recall of saved-loop slot `u8` (FLX4 Beat-Loop pad):
    /// empty slot saves the current loop, filled slot recalls it.
    LoopSlot(Deck, u8),
    /// Delete saved-loop slot `u8` (FLX4 shift + Beat-Loop pad).
    LoopSlotDelete(Deck, u8),
    /// Hold to arm the deck's jog to nudge the loop IN point (SHIFT + LOOP IN).
    /// Momentary; consumed by the decoder, never reaches the host.
    LoopInAdj(Deck),
    /// Hold to arm the deck's jog to nudge the loop OUT point (SHIFT + LOOP OUT).
    LoopOutAdj(Deck),
    /// Synthesized by the decoder while IN ADJ is held: a signed quarter-beat
    /// step for the loop in-point (relative Delta).
    LoopNudgeIn(Deck),
    /// Synthesized by the decoder while OUT ADJ is held: quarter-beat step, out-point.
    LoopNudgeOut(Deck),
    // ── Jog (relative; driven by a device profile, not MIDI-learn) ───────────
    /// Jog wheel touched (on/off) — gates scrub vs let-it-play.
    JogTouch(Deck),
    /// Jog platter scrub — a signed tick delta scrubs the playhead.
    JogScratch(Deck),
    /// Jog ring pitch-bend — a signed tick delta nudges tempo transiently.
    JogBend(Deck),
    // ── Library navigation (UI-bound; the app routes these to the browser, not
    // the engine) ────────────────────────────────────────────────────────────
    /// Scroll the library selection by a signed tick delta (browse encoder).
    LibraryScroll,
    /// Toggle the library panel open/closed (browse-encoder press).
    LibraryOpen,
    /// Load the highlighted library entry onto a deck (load buttons).
    LoadDeck(Deck),
    // ── Universal extended targets — part of the mapping vocabulary for all
    // builds. Whether a target is *implemented* is the host's responsibility;
    // a free host may no-op any of these. Profiles (e.g. FLX4) bind them
    // unconditionally so the same RON ships regardless of feature config. ───
    /// Per-stem FX send amount.
    StemSend(Deck, u8),
    /// Deck colour filter.
    Filter(Deck),
    /// Key transpose, in semitones (continuous, scaled into [min,max]).
    Transpose(Deck),
    /// Drum pitch-lock toggle.
    DrumPitchLock(Deck),
    /// FX bus mute toggle.
    FxBusMute(Deck),
    /// FX bus solo toggle.
    FxBusSolo(Deck),
    /// Enable an FX slot.
    FxSlotEnable(Deck, u8),
    /// FX slot wet/dry mix.
    FxSlotMix(Deck, u8),
    /// FX slot effect-type select (continuous; the host quantizes 0..1 to its effect list). `u8` = slot.
    FxSlotType(Deck, u8),
    /// FX slot parameter knob (continuous; scaled into [min,max] per binding). `u8`s = slot, param index.
    FxSlotParam(Deck, u8, u8),
    /// FX slot tempo-sync division (continuous; the host quantizes to the effect's divisions). `u8` = slot.
    FxSlotDivision(Deck, u8),
    /// Performance FX: vinyl turntable brake/start (held = brake, release = spin back up).
    VinylBrake(Deck),
    /// Performance FX: beat-repeat loop roll of `beats` length (held). Mirrors `BeatJump(Deck, f32)`.
    BeatRepeatRoll(Deck, f32),
    /// Performance FX: noise/riser build-up sweep (one-shot trigger).
    Riser(Deck),
    /// Momentary pad FX (held): `u8` = the host's pad-FX preset index
    /// (a cross-repo contract with the host app's pad-FX preset table).
    PadFx(Deck, u8),
    /// Cycle the tempo-fader range (FLX4 shift + BEAT SYNC "TEMPO RANGE").
    /// UI-bound like the library targets: the range ladder is a frontend
    /// setting, so the host routes this to the UI, not the engine.
    TempoRange(Deck),
    // ── Beat FX (Pioneer-style master beat-FX section; host = pro engine,
    // a free host may no-op these like the other extended targets) ──────────
    /// Beat FX assign flag for this deck (FLX4 slide switch `1/2/1&2` emits one
    /// on/off note per deck; the HOST combines the two flags — both on = both
    /// decks, both off transiently mid-move = no change).
    BeatFxAssign(Deck),
    /// Cycle the Beat FX selection: `1` = next (FX SELECT ▼), `-1` = prev (SHIFT).
    BeatFxSelect(i8),
    /// Step the Beat FX beat ladder: `-1` = ◀, `1` = ▶.
    BeatFxBeat(i8),
    /// Beat FX tempo follows the assigned deck again (SHIFT+◀ "AUTO").
    BeatFxAuto,
    /// Beat FX tap-tempo press (SHIFT+▶ "TAP").
    BeatFxTap,
    /// Beat FX LEVEL/DEPTH knob.
    BeatFxLevel,
    /// Beat FX ON/OFF. Fires per press like `Sync`/`LoopToggle` — the HOST
    /// flips real engine state (the UI can also flip it, so a mapping-side
    /// latch would go stale). LED via `FeedbackState::beat_fx_on`.
    BeatFxOn,
    /// Beat FX release: musical exit — the FX stops but rings out
    /// (FLX4 SHIFT + ON/OFF).
    BeatFxRelease,
    /// Pad-mode selector press (FLX4 Hot Cue / Pad FX1 / Beat Jump / Sampler +
    /// shift variants). Latching device display-state: the HOST records the mode
    /// and drives the cluster LEDs via `FeedbackState::pad_mode`; there is no
    /// engine action. `Kind::Trigger` — fires on press, host ignores the release.
    PadModeSelect(Deck, PadMode),
}

impl Target {
    /// How this target consumes input — drives the default [`Mode`].
    pub fn kind(self) -> Kind {
        use Target::*;
        match self {
            StemVolume(..) | EqLow(_) | EqMid(_) | EqHigh(_) | Pan(_) | ChannelVolume(_)
            | Trim(_) | Tempo(_) | Seek(_) | Crossfade | Master | CueMix | HeadphoneLevel
            | JogScratch(_) | JogBend(_) | LibraryScroll | LoopNudgeIn(_) | LoopNudgeOut(_) => {
                Kind::Continuous
            }
            StemMute(..) | StemSolo(..) | Play(_) | Keylock(_) | Quantize(_)
            | TriggerQuantize(_) | Slip(_) | CueMonitor(_) | CueMaster => Kind::Toggle,
            // JogTouch is momentary: the platter is "touched" while the top plate is
            // held and released on note-off. As a Toggle the release edge was dropped,
            // so `touched` stuck on and the deck stalled silent after a scratch.
            // VinylBrake/BeatRepeatRoll are hold-to-engage the same way: the brake
            // spins back up and the roll stops the moment the pad is released.
            HotCueHold(..) | JogTouch(_) | PadFx(..) | LoopInAdj(_) | LoopOutAdj(_)
            | BeatFxAssign(_) | VinylBrake(..) | BeatRepeatRoll(..) => Kind::Momentary,
            // LoopToggle/Sync flip engine-side state, so they fire per press like
            // a trigger — a mapping-side latch would go stale when the state
            // changes from the UI (or a sync follower swap / auto-mix).
            LoopToggle(_) | Sync(_) | HotCue(..) | HotCueClear(..) | LoopIn(_) | LoopOut(_)
            | LoopHalve(_) | LoopDouble(_) | BeatJump(..) | LoopSet(..) | LoopFourOrExit(_)
            | LoopReloop(_) | LoopSave(_) | LoopCallPrev(_) | LoopCallNext(_) | LoopDelete(_)
            | LoopSlot(..) | LoopSlotDelete(..) | LibraryOpen | LoadDeck(_) | TempoRange(_)
            | PadModeSelect(..) => Kind::Trigger,
            // Extended targets — continuous, toggle, or trigger as the param requires.
            StemSend(..) | Filter(_) | Transpose(_) | FxSlotMix(..) | FxSlotType(..)
            | FxSlotParam(..) | FxSlotDivision(..) | BeatFxLevel => Kind::Continuous,
            DrumPitchLock(_) | FxBusMute(_) | FxBusSolo(_) | FxSlotEnable(..) => Kind::Toggle,
            Riser(..) | BeatFxSelect(_) | BeatFxBeat(_) | BeatFxAuto | BeatFxTap | BeatFxOn
            | BeatFxRelease => Kind::Trigger,
        }
    }

    /// Human-readable label for the bindings UI.
    pub fn label(self) -> String {
        use Target::*;
        match self {
            StemVolume(d, s) => format!("Deck {} · Stem {} vol", d.tag(), s + 1),
            StemMute(d, s) => format!("Deck {} · Stem {} mute", d.tag(), s + 1),
            StemSolo(d, s) => format!("Deck {} · Stem {} solo", d.tag(), s + 1),
            EqLow(d) => format!("Deck {} · EQ low", d.tag()),
            EqMid(d) => format!("Deck {} · EQ mid", d.tag()),
            EqHigh(d) => format!("Deck {} · EQ high", d.tag()),
            Pan(d) => format!("Deck {} · Pan", d.tag()),
            ChannelVolume(d) => format!("Deck {} · Volume", d.tag()),
            Trim(d) => format!("Deck {} · Trim", d.tag()),
            Tempo(d) => format!("Deck {} · Tempo", d.tag()),
            Seek(d) => format!("Deck {} · Seek", d.tag()),
            Play(d) => format!("Deck {} · Play", d.tag()),
            Sync(d) => format!("Deck {} · Sync", d.tag()),
            TempoRange(d) => format!("Deck {} · Tempo range", d.tag()),
            Keylock(d) => format!("Deck {} · Keylock", d.tag()),
            Quantize(d) => format!("Deck {} · Snap", d.tag()),
            TriggerQuantize(d) => format!("Deck {} · Quantize", d.tag()),
            Slip(d) => format!("Deck {} · Slip", d.tag()),
            CueMonitor(d) => format!("Deck {} · Cue", d.tag()),
            CueMaster => "Master · Cue".into(),
            LoopToggle(d) => format!("Deck {} · Loop", d.tag()),
            LoopIn(d) => format!("Deck {} · Loop in", d.tag()),
            LoopOut(d) => format!("Deck {} · Loop out", d.tag()),
            LoopHalve(d) => format!("Deck {} · Loop ÷2", d.tag()),
            LoopDouble(d) => format!("Deck {} · Loop ×2", d.tag()),
            HotCue(d, s) => format!("Deck {} · Hot cue {}", d.tag(), s + 1),
            HotCueHold(d, s) => format!("Deck {} · Hot cue {} (hold)", d.tag(), s + 1),
            HotCueClear(d, s) => format!("Deck {} · Clear cue {}", d.tag(), s + 1),
            BeatJump(d, b) => format!("Deck {} · Beat-jump {:+}", d.tag(), b),
            LoopSet(d, b) => format!("Deck {} · Loop {} beat", d.tag(), b),
            Crossfade => "Crossfader".into(),
            Master => "Master".into(),
            CueMix => "Cue mix".into(),
            HeadphoneLevel => "Headphones".into(),
            JogTouch(d) => format!("Deck {} · Jog touch", d.tag()),
            JogScratch(d) => format!("Deck {} · Jog scratch", d.tag()),
            JogBend(d) => format!("Deck {} · Jog bend", d.tag()),
            LibraryScroll => "Library · Scroll".into(),
            LibraryOpen => "Library · Open".into(),
            LoopFourOrExit(d) => format!("Deck {} · 4-beat / exit", d.tag()),
            LoopReloop(d) => format!("Deck {} · Reloop", d.tag()),
            LoopSave(d) => format!("Deck {} · Save loop", d.tag()),
            LoopCallPrev(d) => format!("Deck {} · Call prev loop", d.tag()),
            LoopCallNext(d) => format!("Deck {} · Call next loop", d.tag()),
            LoopDelete(d) => format!("Deck {} · Delete loop", d.tag()),
            LoopSlot(d, s) => format!("Deck {} · Loop slot {}", d.tag(), s + 1),
            LoopSlotDelete(d, s) => format!("Deck {} · Delete loop slot {}", d.tag(), s + 1),
            LoopInAdj(d) => format!("Deck {} · Loop in adjust", d.tag()),
            LoopOutAdj(d) => format!("Deck {} · Loop out adjust", d.tag()),
            LoopNudgeIn(d) => format!("Deck {} · Nudge loop in", d.tag()),
            LoopNudgeOut(d) => format!("Deck {} · Nudge loop out", d.tag()),
            LoadDeck(d) => format!("Library · Load deck {}", d.tag()),
            StemSend(d, s) => format!("Deck {} · Stem {} send", d.tag(), s + 1),
            Filter(d) => format!("Deck {} · Filter", d.tag()),
            Transpose(d) => format!("Deck {} · Transpose", d.tag()),
            DrumPitchLock(d) => format!("Deck {} · Drum pitch-lock", d.tag()),
            FxBusMute(d) => format!("Deck {} · FX mute", d.tag()),
            FxBusSolo(d) => format!("Deck {} · FX solo", d.tag()),
            FxSlotEnable(d, s) => format!("Deck {} · FX {} on", d.tag(), s + 1),
            FxSlotMix(d, s) => format!("Deck {} · FX {} mix", d.tag(), s + 1),
            FxSlotType(d, s) => format!("Deck {} · FX{} Type", d.tag(), s + 1),
            FxSlotParam(d, s, p) => format!("Deck {} · FX{} P{}", d.tag(), s + 1, p + 1),
            FxSlotDivision(d, s) => format!("Deck {} · FX{} Beat", d.tag(), s + 1),
            VinylBrake(d) => format!("Deck {} · Vinyl Brake", d.tag()),
            BeatRepeatRoll(d, beats) => format!("Deck {} · Beat Roll {}", d.tag(), beats),
            Riser(d) => format!("Deck {} · Riser", d.tag()),
            PadFx(d, i) => format!("Deck {} · Pad FX {}", d.tag(), u16::from(i) + 1),
            BeatFxAssign(d) => format!("Beat FX · Assign deck {}", d.tag()),
            BeatFxSelect(dir) => format!(
                "Beat FX · Select {}",
                if dir >= 0 { "next" } else { "prev" }
            ),
            BeatFxBeat(dir) => format!("Beat FX · Beat {}", if dir >= 0 { "▶" } else { "◀" }),
            BeatFxAuto => "Beat FX · Auto BPM".into(),
            BeatFxTap => "Beat FX · Tap tempo".into(),
            BeatFxLevel => "Beat FX · Level/Depth".into(),
            BeatFxOn => "Beat FX · On/Off".into(),
            BeatFxRelease => "Beat FX · Release".into(),
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
        }
    }
}

/// An incoming control identity (channel + CC/note number).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum ControlId {
    Cc { channel: u8, controller: u8 },
    Note { channel: u8, note: u8 },
}

impl fmt::Display for ControlId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ControlId::Cc {
                channel,
                controller,
            } => write!(f, "CC {} · ch {}", controller, channel + 1),
            ControlId::Note { channel, note } => write!(f, "Note {} · ch {}", note, channel + 1),
        }
    }
}

/// How a binding interprets incoming values.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Mode {
    /// 0..127 maps directly to the parameter (optionally soft-takeover gated).
    Absolute,
    /// Endless encoder: each message is a signed delta.
    Relative(RelEncoding),
    /// Button: each press flips an on/off state.
    Toggle,
    /// Button: 1.0 while held, 0.0 on release.
    Momentary,
    /// Button: fires 1.0 once on the press edge.
    Trigger,
}

/// Per-binding behaviour options.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Options {
    pub mode: Mode,
    pub invert: bool,
    pub soft_takeover: bool,
    /// Output range the 0..1 value is scaled into (default 0..1).
    pub min: f32,
    pub max: f32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            mode: Mode::Absolute,
            invert: false,
            soft_takeover: false,
            min: 0.0,
            max: 1.0,
        }
    }
}

impl Options {
    /// Sensible defaults for a freshly-learned binding to `target`.
    pub fn for_target(target: Target) -> Self {
        let mode = match target.kind() {
            Kind::Continuous => Mode::Absolute,
            Kind::Toggle => Mode::Toggle,
            Kind::Trigger => Mode::Trigger,
            Kind::Momentary => Mode::Momentary,
        };
        // A few targets aren't 0..1 parameters and want a custom default range.
        let (min, max) = match target {
            // Tempo is a playback-rate ratio: default to ±8%.
            Target::Tempo(_) => (0.92, 1.08),
            // Transpose is in semitones, centred at 0: default to ±12.
            Target::Transpose(_) => (-12.0, 12.0),
            _ => (0.0, 1.0),
        };
        Self {
            mode,
            min,
            max,
            ..Self::default()
        }
    }
}

/// Non-persisted per-binding runtime state.
#[derive(Clone, Debug, Default)]
struct Runtime {
    /// On/off latch for Toggle mode (and the LED state).
    toggle_on: bool,
    /// Press state for edge detection (button modes).
    button_down: bool,
    /// Last value emitted == our approximation of the software value
    /// (soft-takeover reference + relative accumulator position).
    software: Option<f32>,
    /// Last physical input seen while catching up (for the crossing test).
    prev_input: Option<f32>,
    /// True once soft-takeover has caught the control.
    engaged: bool,
}

/// One resolved binding: control → target, with behaviour options + runtime.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Binding {
    pub control: ControlId,
    pub target: Target,
    #[serde(default)]
    pub options: Options,
    #[serde(skip)]
    rt: Runtime,
}

/// What a resolved MIDI message should do to the engine.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Action {
    pub target: Target,
    pub value: f32,
    /// Optional 3-byte MIDI message to send back for LED/feedback.
    pub feedback: Option<[u8; 3]>,
}

/// A normalized incoming sample, independent of note-vs-CC transport.
#[derive(Clone, Copy, Debug)]
enum Sample {
    /// Continuous CC: normalized 0..1 plus the raw 7-bit byte (for relative).
    Continuous {
        norm: f32,
        raw: u8,
    },
    Press,
    Release,
}

#[derive(Clone, Copy, PartialEq)]
enum Edge {
    Rising,
    Falling,
    None,
}

impl Binding {
    fn new(control: ControlId, target: Target, options: Options) -> Self {
        Self {
            control,
            target,
            options,
            rt: Runtime::default(),
        }
    }

    fn scale(&self, v: f32) -> f32 {
        self.options.min + v * (self.options.max - self.options.min)
    }

    fn pressed(s: Sample) -> bool {
        match s {
            Sample::Continuous { norm, .. } => norm >= 0.5,
            Sample::Press => true,
            Sample::Release => false,
        }
    }

    fn edge(&mut self, s: Sample) -> Edge {
        let pressed = Self::pressed(s);
        let prev = self.rt.button_down;
        self.rt.button_down = pressed;
        match (prev, pressed) {
            (false, true) => Edge::Rising,
            (true, false) => Edge::Falling,
            _ => Edge::None,
        }
    }

    /// Absolute soft-takeover gate. Returns the accepted value, or None if ignored.
    fn gate(&mut self, v: f32) -> Option<f32> {
        if !self.options.soft_takeover {
            self.rt.software = Some(v);
            return Some(v);
        }
        match self.rt.software {
            // No reference yet: start controlling from here.
            None => {
                self.rt.engaged = true;
                self.rt.software = Some(v);
                Some(v)
            }
            // Already caught up: track freely.
            Some(_) if self.rt.engaged => {
                self.rt.software = Some(v);
                Some(v)
            }
            // Catching up: accept once we cross or land within threshold.
            Some(sw) => {
                let diff = sw - v;
                let crossed = self
                    .rt
                    .prev_input
                    .map(|p| (sw - p).signum() != diff.signum())
                    .unwrap_or(false);
                let within = diff.abs() <= TAKEOVER_THRESHOLD;
                self.rt.prev_input = Some(v);
                if crossed || within {
                    self.rt.engaged = true;
                    self.rt.software = Some(v);
                    Some(v)
                } else {
                    None
                }
            }
        }
    }

    /// Re-arm soft-takeover against a known software value (e.g. after a deck swap).
    pub fn rearm(&mut self, software_value: f32) {
        self.rt.software = Some(software_value);
        self.rt.engaged = false;
        self.rt.prev_input = None;
    }

    /// On/off state (for feedback seeding).
    pub fn is_on(&self) -> bool {
        self.rt.toggle_on
    }

    /// The 3-byte feedback message for a Note-addressed button, if any.
    pub fn feedback(&self, on: bool) -> Option<[u8; 3]> {
        match (self.control, self.options.mode) {
            (ControlId::Note { channel, note }, Mode::Toggle | Mode::Momentary | Mode::Trigger) => {
                let status = 0x90 | (channel & 0x0F);
                Some([status, note, if on { 127 } else { 0 }])
            }
            _ => None,
        }
    }

    /// Resolve one incoming sample to an emitted value + optional feedback.
    fn process(&mut self, s: Sample) -> Option<(f32, Option<[u8; 3]>)> {
        match self.options.mode {
            Mode::Absolute => {
                let norm = match s {
                    Sample::Continuous { norm, .. } => norm,
                    Sample::Press => 1.0,
                    Sample::Release => 0.0,
                };
                let v = if self.options.invert {
                    1.0 - norm
                } else {
                    norm
                };
                let gated = self.gate(v)?;
                Some((self.scale(gated), None))
            }
            Mode::Relative(enc) => {
                let raw = match s {
                    Sample::Continuous { raw, .. } => raw,
                    _ => return None,
                };
                let mut step = decode_relative(enc, raw) as f32 * REL_STEP;
                if self.options.invert {
                    step = -step;
                }
                let pos = (self.rt.software.unwrap_or(0.5) + step).clamp(0.0, 1.0);
                self.rt.software = Some(pos);
                Some((self.scale(pos), None))
            }
            Mode::Toggle => {
                if self.edge(s) == Edge::Rising {
                    self.rt.toggle_on = !self.rt.toggle_on;
                    let on = self.rt.toggle_on;
                    Some((if on { 1.0 } else { 0.0 }, self.feedback(on)))
                } else {
                    None
                }
            }
            Mode::Momentary => match self.edge(s) {
                Edge::Rising => Some((1.0, self.feedback(true))),
                Edge::Falling => Some((0.0, self.feedback(false))),
                Edge::None => None,
            },
            Mode::Trigger => {
                if self.edge(s) == Edge::Rising {
                    Some((1.0, self.feedback(true)))
                } else {
                    None
                }
            }
        }
    }
}

/// A learnable, persistable control map.
#[derive(Default, Debug)]
pub struct MidiMap {
    bindings: Vec<Binding>,
    learn: Option<(Target, Options)>,
}

impl MidiMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Arm learn with default options for the target: the next control binds to it.
    pub fn arm_learn(&mut self, target: Target) {
        self.learn = Some((target, Options::for_target(target)));
    }

    /// Arm learn with explicit options.
    pub fn arm_learn_with(&mut self, target: Target, options: Options) {
        self.learn = Some((target, options));
    }

    pub fn cancel_learn(&mut self) {
        self.learn = None;
    }
    pub fn is_learning(&self) -> bool {
        self.learn.is_some()
    }
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }

    /// Remove the binding for `control`, if any. Returns true if one was removed.
    pub fn unbind(&mut self, control: ControlId) -> bool {
        let before = self.bindings.len();
        self.bindings.retain(|b| b.control != control);
        self.bindings.len() != before
    }

    /// Remove every binding.
    pub fn clear(&mut self) {
        self.bindings.clear();
    }

    /// Directly insert a binding (replacing any on the same control). Used to
    /// build presets programmatically.
    pub fn insert(&mut self, control: ControlId, target: Target, options: Options) {
        self.bind(control, target, options);
    }

    /// A sensible starter mapping for a generic CC/note controller: an 8-knob /
    /// 8-button layout on channel 1. Continuous knobs are absolute; transport
    /// buttons are notes (so they get LED feedback). A starting point users tweak.
    pub fn generic() -> Self {
        use Target::*;
        let cc = |c| ControlId::Cc {
            channel: 0,
            controller: c,
        };
        let note = |n| ControlId::Note {
            channel: 0,
            note: n,
        };
        let mut m = Self::new();
        // Knobs / faders (CC 1..=8).
        for (c, t) in [
            (1, EqHigh(Deck::A)),
            (2, EqMid(Deck::A)),
            (3, EqLow(Deck::A)),
            (4, ChannelVolume(Deck::A)),
            (5, EqHigh(Deck::B)),
            (6, EqMid(Deck::B)),
            (7, EqLow(Deck::B)),
            (8, ChannelVolume(Deck::B)),
        ] {
            m.insert(cc(c), t, Options::for_target(t));
        }
        m.insert(cc(9), Crossfade, Options::for_target(Crossfade));
        m.insert(cc(10), Master, Options::for_target(Master));
        // Transport / cue buttons (notes for LED feedback).
        for (n, t) in [
            (36, Play(Deck::A)),
            (37, Sync(Deck::A)),
            (38, CueMonitor(Deck::A)),
            (39, LoopToggle(Deck::A)),
            (40, Play(Deck::B)),
            (41, Sync(Deck::B)),
            (42, CueMonitor(Deck::B)),
            (43, LoopToggle(Deck::B)),
        ] {
            m.insert(note(n), t, Options::for_target(t));
        }
        m
    }

    /// Re-arm soft-takeover on all bindings against a single reference value.
    /// (The service calls this with the engine's current value after a deck swap.)
    pub fn rearm_all(&mut self, software_value: f32) {
        for b in &mut self.bindings {
            b.rearm(software_value);
        }
    }

    fn bind(&mut self, id: ControlId, target: Target, options: Options) {
        self.bindings.retain(|b| b.control != id);
        self.bindings.push(Binding::new(id, target, options));
    }

    /// Serialize to the current (schema 2) JSON format.
    pub fn to_json(&self) -> String {
        serde_json::to_string(&Persisted {
            schema: SCHEMA_VERSION,
            bindings: self.bindings.clone(),
        })
        .unwrap_or_default()
    }

    /// Deserialize, accepting schema 2 and migrating the legacy tuple format.
    pub fn from_json(s: &str) -> Self {
        // Current format: tagged with a schema version.
        if let Ok(p) = serde_json::from_str::<Persisted>(s) {
            return Self {
                bindings: p.bindings,
                learn: None,
            };
        }
        // Legacy Phase-1 format: { "bindings": [[ControlId, Target], …] }.
        if let Ok(legacy) = serde_json::from_str::<LegacyMap>(s) {
            let bindings = legacy
                .bindings
                .into_iter()
                .map(|(control, target)| Binding::new(control, target, Options::for_target(target)))
                .collect();
            return Self {
                bindings,
                learn: None,
            };
        }
        Self::new()
    }

    /// Process a parsed message. In learn mode, binds and returns None.
    /// Otherwise returns the [`Action`] for a mapped control.
    pub fn handle(&mut self, msg: MidiMessage, decoder: &mut HighResDecoder) -> Option<Action> {
        let (id, sample) = match msg {
            MidiMessage::ControlChange {
                channel,
                controller,
                value,
            } => {
                let (logical, v) = decoder.feed(channel, controller, value)?;
                (
                    ControlId::Cc {
                        channel: channel & 0x0F,
                        controller: logical,
                    },
                    Sample::Continuous {
                        norm: v,
                        raw: value & 0x7F,
                    },
                )
            }
            MidiMessage::NoteOn { channel, note, .. } => (
                ControlId::Note {
                    channel: channel & 0x0F,
                    note,
                },
                Sample::Press,
            ),
            MidiMessage::NoteOff { channel, note } => (
                ControlId::Note {
                    channel: channel & 0x0F,
                    note,
                },
                Sample::Release,
            ),
            MidiMessage::Other => return None,
        };

        if let Some((target, options)) = self.learn.take() {
            self.bind(id, target, options);
            return None;
        }

        let b = self.bindings.iter_mut().find(|b| b.control == id)?;
        let target = b.target;
        let (value, feedback) = b.process(sample)?;
        Some(Action {
            target,
            value,
            feedback,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    fn cc(controller: u8, value: u8) -> MidiMessage {
        parse(&[0xB0, controller, value]).unwrap()
    }
    fn note_on(note: u8) -> MidiMessage {
        parse(&[0x90, note, 100]).unwrap()
    }
    fn note_off(note: u8) -> MidiMessage {
        parse(&[0x80, note, 0]).unwrap()
    }

    fn learn_cc(target: Target, options: Options, controller: u8) -> (MidiMap, HighResDecoder) {
        let mut map = MidiMap::new();
        let mut dec = HighResDecoder::new();
        map.arm_learn_with(target, options);
        assert!(map.handle(cc(controller, 0), &mut dec).is_none());
        (map, dec)
    }

    // ── labels / display ─────────────────────────────────────────
    #[test]
    fn target_labels_are_human_readable() {
        assert_eq!(
            Target::StemVolume(Deck::A, 0).label(),
            "Deck A · Stem 1 vol"
        );
        assert_eq!(Target::Crossfade.label(), "Crossfader");
        assert_eq!(Target::HotCue(Deck::B, 1).label(), "Deck B · Hot cue 2");
        assert_eq!(
            Target::BeatJump(Deck::A, -4.0).label(),
            "Deck A · Beat-jump -4"
        );
    }

    #[test]
    fn control_id_display() {
        let cc = ControlId::Cc {
            channel: 0,
            controller: 7,
        };
        assert_eq!(cc.to_string(), "CC 7 · ch 1");
        let note = ControlId::Note {
            channel: 2,
            note: 36,
        };
        assert_eq!(note.to_string(), "Note 36 · ch 3");
    }

    #[test]
    fn loop_four_or_exit_and_reloop_are_triggers_with_labels() {
        assert_eq!(Target::LoopFourOrExit(Deck::A).kind(), Kind::Trigger);
        assert_eq!(Target::LoopReloop(Deck::B).kind(), Kind::Trigger);
        assert_eq!(
            Target::LoopFourOrExit(Deck::A).label(),
            "Deck A · 4-beat / exit"
        );
        assert_eq!(Target::LoopReloop(Deck::B).label(), "Deck B · Reloop");
    }

    #[test]
    fn loop_slot_targets_are_triggers_with_labels() {
        assert_eq!(Target::LoopSlot(Deck::A, 0).kind(), Kind::Trigger);
        assert_eq!(Target::LoopSlotDelete(Deck::B, 7).kind(), Kind::Trigger);
        assert_eq!(Target::LoopSlot(Deck::A, 2).label(), "Deck A · Loop slot 3");
        assert_eq!(
            Target::LoopSlotDelete(Deck::B, 0).label(),
            "Deck B · Delete loop slot 1"
        );
    }

    #[test]
    fn saved_loop_targets_are_triggers_with_labels() {
        assert_eq!(Target::LoopSave(Deck::A).kind(), Kind::Trigger);
        assert_eq!(Target::LoopCallPrev(Deck::A).kind(), Kind::Trigger);
        assert_eq!(Target::LoopCallNext(Deck::A).kind(), Kind::Trigger);
        assert_eq!(Target::LoopDelete(Deck::A).kind(), Kind::Trigger);
        assert_eq!(Target::LoopSave(Deck::A).label(), "Deck A · Save loop");
        assert_eq!(
            Target::LoopCallPrev(Deck::B).label(),
            "Deck B · Call prev loop"
        );
        assert_eq!(
            Target::LoopCallNext(Deck::B).label(),
            "Deck B · Call next loop"
        );
        assert_eq!(Target::LoopDelete(Deck::A).label(), "Deck A · Delete loop");
    }

    #[test]
    fn loop_adjust_and_nudge_targets_classify_and_label() {
        assert_eq!(Target::LoopInAdj(Deck::A).kind(), Kind::Momentary);
        assert_eq!(Target::LoopOutAdj(Deck::B).kind(), Kind::Momentary);
        assert_eq!(Target::LoopNudgeIn(Deck::A).kind(), Kind::Continuous);
        assert_eq!(Target::LoopNudgeOut(Deck::B).kind(), Kind::Continuous);
        assert_eq!(
            Target::LoopInAdj(Deck::A).label(),
            "Deck A · Loop in adjust"
        );
        assert_eq!(
            Target::LoopOutAdj(Deck::B).label(),
            "Deck B · Loop out adjust"
        );
        assert_eq!(
            Target::LoopNudgeIn(Deck::A).label(),
            "Deck A · Nudge loop in"
        );
        assert_eq!(
            Target::LoopNudgeOut(Deck::B).label(),
            "Deck B · Nudge loop out"
        );
    }

    #[test]
    fn kind_drives_default_mode() {
        assert_eq!(Options::for_target(Target::Crossfade).mode, Mode::Absolute);
        assert_eq!(
            Options::for_target(Target::Play(Deck::A)).mode,
            Mode::Toggle
        );
        assert_eq!(
            Options::for_target(Target::HotCue(Deck::A, 0)).mode,
            Mode::Trigger
        );
    }

    #[test]
    fn jog_touch_is_momentary() {
        // JogTouch must emit on BOTH edges (press → 1.0, release → 0.0). As a Toggle
        // the note-off release was dropped, so the platter stayed "touched" and the
        // deck stalled silent after a scratch.
        assert_eq!(Target::JogTouch(Deck::A).kind(), Kind::Momentary);
        assert_eq!(Target::JogTouch(Deck::B).kind(), Kind::Momentary);
    }

    // ── absolute + invert + range ────────────────────────────────
    #[test]
    fn absolute_passthrough() {
        let (mut map, mut dec) = learn_cc(Target::Crossfade, Options::default(), 10);
        let a = map.handle(cc(10, 127), &mut dec).unwrap();
        assert_eq!(a.target, Target::Crossfade);
        assert!((a.value - 1.0).abs() < 1e-6);
    }

    #[test]
    fn invert_flips_value() {
        let opts = Options {
            invert: true,
            ..Options::default()
        };
        let (mut map, mut dec) = learn_cc(Target::Crossfade, opts, 10);
        let a = map.handle(cc(10, 127), &mut dec).unwrap();
        assert!(a.value.abs() < 1e-6, "127 inverted → 0");
    }

    #[test]
    fn range_scales_output() {
        let opts = Options {
            min: 0.25,
            max: 0.75,
            ..Options::default()
        };
        let (mut map, mut dec) = learn_cc(Target::Master, opts, 10);
        let a = map.handle(cc(10, 127), &mut dec).unwrap();
        assert!((a.value - 0.75).abs() < 1e-6);
        let b = map.handle(cc(10, 0), &mut dec).unwrap();
        assert!((b.value - 0.25).abs() < 1e-6);
    }

    // ── relative encoders ────────────────────────────────────────
    #[test]
    fn relative_accumulates_from_midpoint() {
        let opts = Options {
            mode: Mode::Relative(RelEncoding::BinaryOffset),
            ..Options::default()
        };
        // controller 70 is >= 64 so highres treats it as plain 7-bit.
        let (mut map, mut dec) = learn_cc(Target::Master, opts, 70);
        let a = map.handle(cc(70, 65), &mut dec).unwrap(); // +1 from 0.5
        assert!((a.value - (0.5 + REL_STEP)).abs() < 1e-6);
        let b = map.handle(cc(70, 65), &mut dec).unwrap(); // +1 more
        assert!((b.value - (0.5 + 2.0 * REL_STEP)).abs() < 1e-6);
        let c = map.handle(cc(70, 63), &mut dec).unwrap(); // -1
        assert!((c.value - (0.5 + REL_STEP)).abs() < 1e-6);
    }

    #[test]
    fn relative_clamps_at_bounds() {
        let opts = Options {
            mode: Mode::Relative(RelEncoding::BinaryOffset),
            ..Options::default()
        };
        let (mut map, mut dec) = learn_cc(Target::Master, opts, 70);
        let a = map.handle(cc(70, 0), &mut dec).unwrap(); // delta -64
        assert!(a.value >= 0.0 && a.value < 0.5);
        let b = map.handle(cc(70, 0), &mut dec).unwrap();
        assert!(b.value.abs() < 1e-6, "clamped at 0");
    }

    // ── button modes ─────────────────────────────────────────────
    #[test]
    fn toggle_flips_on_each_press() {
        let opts = Options::for_target(Target::Play(Deck::A));
        let mut map = MidiMap::new();
        let mut dec = HighResDecoder::new();
        map.arm_learn_with(Target::Play(Deck::A), opts);
        map.handle(note_on(36), &mut dec); // learn
        let on = map.handle(note_on(36), &mut dec).unwrap();
        assert_eq!(on.value, 1.0);
        assert!(map.handle(note_off(36), &mut dec).is_none()); // release ignored
        let off = map.handle(note_on(36), &mut dec).unwrap();
        assert_eq!(off.value, 0.0);
    }

    #[test]
    fn momentary_tracks_hold_and_release() {
        let opts = Options {
            mode: Mode::Momentary,
            ..Options::default()
        };
        let mut map = MidiMap::new();
        let mut dec = HighResDecoder::new();
        map.arm_learn_with(Target::CueMonitor(Deck::A), opts);
        map.handle(note_on(36), &mut dec); // learn
        assert_eq!(map.handle(note_on(36), &mut dec).unwrap().value, 1.0);
        assert_eq!(map.handle(note_off(36), &mut dec).unwrap().value, 0.0);
    }

    #[test]
    fn trigger_fires_once_on_press_edge() {
        let opts = Options::for_target(Target::HotCue(Deck::A, 0));
        let mut map = MidiMap::new();
        let mut dec = HighResDecoder::new();
        map.arm_learn_with(Target::HotCue(Deck::A, 0), opts);
        map.handle(note_on(36), &mut dec); // learn
        assert_eq!(map.handle(note_on(36), &mut dec).unwrap().value, 1.0);
        assert!(map.handle(note_on(36), &mut dec).is_none()); // no re-fire while held
        assert!(map.handle(note_off(36), &mut dec).is_none());
        assert_eq!(map.handle(note_on(36), &mut dec).unwrap().value, 1.0); // fires again
    }

    // ── soft-takeover ────────────────────────────────────────────
    #[test]
    fn soft_takeover_ignores_until_crossing() {
        let opts = Options {
            soft_takeover: true,
            ..Options::default()
        };
        let mut map = MidiMap::new();
        let mut dec = HighResDecoder::new();
        map.arm_learn_with(Target::Crossfade, opts);
        map.handle(cc(10, 64), &mut dec); // learn captures control
        map.rearm_all(0.5); // engine param sits at 0.5

        // Physical fader is down at ~0.1 → ignored (would jump).
        assert!(map.handle(cc(10, 13), &mut dec).is_none());
        // Sweeps up but still below 0.5 → still ignored.
        assert!(map.handle(cc(10, 50), &mut dec).is_none());
        // Crosses 0.5 → caught, now controlling.
        let caught = map.handle(cc(10, 80), &mut dec).unwrap();
        assert!(caught.value > 0.5);
        // Subsequent values flow freely, even below 0.5.
        let free = map.handle(cc(10, 10), &mut dec).unwrap();
        assert!(free.value < 0.2);
    }

    // ── feedback ─────────────────────────────────────────────────
    #[test]
    fn toggle_emits_led_feedback() {
        let opts = Options::for_target(Target::Play(Deck::A));
        let mut map = MidiMap::new();
        let mut dec = HighResDecoder::new();
        map.arm_learn_with(Target::Play(Deck::A), opts);
        map.handle(note_on(36), &mut dec); // learn
        let on = map.handle(note_on(36), &mut dec).unwrap();
        assert_eq!(on.feedback, Some([0x90, 36, 127]));
        map.handle(note_off(36), &mut dec); // release between presses
        let off = map.handle(note_on(36), &mut dec).unwrap();
        assert_eq!(off.feedback, Some([0x90, 36, 0]));
    }

    #[test]
    fn continuous_cc_has_no_feedback() {
        let (mut map, mut dec) = learn_cc(Target::Crossfade, Options::default(), 10);
        let a = map.handle(cc(10, 64), &mut dec).unwrap();
        assert_eq!(a.feedback, None);
    }

    // ── management ───────────────────────────────────────────────
    #[test]
    fn unbind_and_clear() {
        let (mut map, mut dec) = learn_cc(Target::Crossfade, Options::default(), 10);
        assert_eq!(map.bindings().len(), 1);
        let id = ControlId::Cc {
            channel: 0,
            controller: 10,
        };
        assert!(map.unbind(id));
        assert_eq!(map.bindings().len(), 0);
        assert!(!map.unbind(id), "second unbind is a no-op");

        map.arm_learn(Target::Master);
        map.handle(cc(11, 0), &mut dec);
        assert_eq!(map.bindings().len(), 1);
        map.clear();
        assert_eq!(map.bindings().len(), 0);
    }

    #[test]
    fn rebinding_same_control_replaces() {
        let mut map = MidiMap::new();
        let mut dec = HighResDecoder::new();
        map.arm_learn(Target::Crossfade);
        map.handle(cc(10, 0), &mut dec);
        map.arm_learn(Target::Master);
        map.handle(cc(10, 0), &mut dec);
        assert_eq!(map.bindings().len(), 1);
        assert_eq!(map.bindings()[0].target, Target::Master);
    }

    // ── persistence + migration ──────────────────────────────────
    #[test]
    fn schema_2_round_trip_preserves_options() {
        let opts = Options {
            mode: Mode::Relative(RelEncoding::TwosComplement),
            invert: true,
            soft_takeover: true,
            min: 0.1,
            max: 0.9,
        };
        let (map, _dec) = learn_cc(Target::EqLow(Deck::A), opts, 20);
        let json = map.to_json();
        assert!(json.contains("\"schema\""), "carries a schema version");

        let restored = MidiMap::from_json(&json);
        assert_eq!(restored.bindings().len(), 1);
        let b = &restored.bindings()[0];
        assert_eq!(b.target, Target::EqLow(Deck::A));
        assert_eq!(b.options, opts);
        assert_eq!(
            b.control,
            ControlId::Cc {
                channel: 0,
                controller: 20
            }
        );
    }

    #[test]
    fn migrates_legacy_tuple_format() {
        // The Phase-1 format: { "bindings": [[ControlId, Target], ...] }.
        let legacy = r#"{"bindings":[
            [{"Cc":{"channel":0,"controller":7}},{"StemVolume":["A",0]}],
            [{"Note":{"channel":0,"note":36}},{"Play":"B"}]
        ]}"#;
        let map = MidiMap::from_json(legacy);
        assert_eq!(map.bindings().len(), 2);

        let vol = &map.bindings()[0];
        assert_eq!(vol.target, Target::StemVolume(Deck::A, 0));
        assert_eq!(vol.options.mode, Mode::Absolute, "CC continuous → Absolute");

        let play = &map.bindings()[1];
        assert_eq!(play.target, Target::Play(Deck::B));
        assert_eq!(play.options.mode, Mode::Toggle, "Play note → Toggle");
    }

    #[test]
    fn from_json_garbage_is_empty() {
        assert_eq!(MidiMap::from_json("not json").bindings().len(), 0);
        assert_eq!(MidiMap::from_json("").bindings().len(), 0);
    }

    #[test]
    fn generic_preset_resolves_a_known_control() {
        let mut map = MidiMap::generic();
        let mut dec = HighResDecoder::new();
        assert!(!map.bindings().is_empty());
        // CC 9 ch1 is the crossfader in the generic layout.
        let a = map.handle(cc(9, 127), &mut dec).unwrap();
        assert_eq!(a.target, Target::Crossfade);
        assert!((a.value - 1.0).abs() < 1e-6);
        // Note 36 ch1 is Deck A play (a toggle → LED feedback).
        let p = map.handle(note_on(36), &mut dec).unwrap();
        assert_eq!(p.target, Target::Play(Deck::A));
        assert_eq!(p.feedback, Some([0x90, 36, 127]));
    }

    #[test]
    fn insert_replaces_on_same_control() {
        let mut map = MidiMap::new();
        let id = ControlId::Cc {
            channel: 0,
            controller: 5,
        };
        map.insert(id, Target::Master, Options::default());
        map.insert(id, Target::Crossfade, Options::default());
        assert_eq!(map.bindings().len(), 1);
        assert_eq!(map.bindings()[0].target, Target::Crossfade);
    }

    // ── Extended targets (universal — always present) ────────────
    #[test]
    fn extended_targets_classify_and_label() {
        assert_eq!(Target::StemSend(Deck::A, 0).kind(), Kind::Continuous);
        assert_eq!(Target::FxSlotMix(Deck::B, 2).kind(), Kind::Continuous);
        assert_eq!(Target::DrumPitchLock(Deck::A).kind(), Kind::Toggle);
        assert_eq!(Target::FxSlotEnable(Deck::A, 0).kind(), Kind::Toggle);
        assert_eq!(Target::StemSend(Deck::A, 0).label(), "Deck A · Stem 1 send");
        assert_eq!(Target::FxSlotMix(Deck::B, 2).label(), "Deck B · FX 3 mix");
        assert_eq!(
            Target::DrumPitchLock(Deck::A).label(),
            "Deck A · Drum pitch-lock"
        );
    }

    #[test]
    fn cue_slip_targets_classify_and_label() {
        // Snap (set-time) and Quantize (trigger-time) are distinct toggles.
        assert_eq!(Target::Quantize(Deck::A).kind(), Kind::Toggle);
        assert_eq!(Target::TriggerQuantize(Deck::A).kind(), Kind::Toggle);
        assert_eq!(Target::Slip(Deck::B).kind(), Kind::Toggle);
        assert_eq!(Target::HotCueHold(Deck::A, 2).kind(), Kind::Momentary);
        assert_eq!(Target::Quantize(Deck::A).label(), "Deck A · Snap");
        assert_eq!(
            Target::TriggerQuantize(Deck::A).label(),
            "Deck A · Quantize"
        );
        assert_eq!(Target::Slip(Deck::B).label(), "Deck B · Slip");
        assert_eq!(
            Target::HotCueHold(Deck::A, 2).label(),
            "Deck A · Hot cue 3 (hold)"
        );
        // A Momentary target defaults to Momentary mode in the learn-map path too.
        assert_eq!(
            Options::for_target(Target::HotCueHold(Deck::A, 0)).mode,
            Mode::Momentary
        );
    }

    #[test]
    fn extended_default_modes_and_transpose_range() {
        assert_eq!(
            Options::for_target(Target::StemSend(Deck::A, 0)).mode,
            Mode::Absolute
        );
        assert_eq!(
            Options::for_target(Target::FxBusMute(Deck::A)).mode,
            Mode::Toggle
        );
        let tr = Options::for_target(Target::Transpose(Deck::A));
        assert_eq!((tr.min, tr.max), (-12.0, 12.0));
    }

    #[test]
    fn fx_perf_targets_kind_and_label() {
        // kind checks
        assert_eq!(Target::FxSlotType(Deck::A, 0).kind(), Kind::Continuous);
        assert_eq!(Target::FxSlotParam(Deck::A, 0, 0).kind(), Kind::Continuous);
        assert_eq!(Target::FxSlotDivision(Deck::A, 0).kind(), Kind::Continuous);
        assert_eq!(Target::VinylBrake(Deck::A).kind(), Kind::Momentary);
        assert_eq!(Target::BeatRepeatRoll(Deck::A, 1.0).kind(), Kind::Momentary);
        assert_eq!(Target::Riser(Deck::A).kind(), Kind::Trigger);
        assert_eq!(Target::PadFx(Deck::A, 3).kind(), Kind::Momentary);
        // label non-empty + contains deck tag
        for d in [Deck::A, Deck::B] {
            let tag = d.tag();
            assert!(Target::FxSlotType(d, 0).label().contains(tag));
            assert!(Target::FxSlotParam(d, 0, 0).label().contains(tag));
            assert!(Target::FxSlotDivision(d, 0).label().contains(tag));
            assert!(Target::VinylBrake(d).label().contains(tag));
            assert!(Target::BeatRepeatRoll(d, 1.0).label().contains(tag));
            assert!(Target::Riser(d).label().contains(tag));
            assert!(Target::PadFx(d, 0).label().contains(tag));
        }
    }

    #[test]
    fn hires_pair_dispatches_once_per_movement() {
        let (mut map, mut dec) = learn_cc(
            Target::EqHigh(Deck::A),
            Options::for_target(Target::EqHigh(Deck::A)),
            10,
        );
        // First movement: the pair isn't known yet — MSB dispatches coarse,
        // then the LSB refines it.
        assert!(map.handle(cc(10, 100), &mut dec).is_some());
        assert!(map.handle(cc(42, 50), &mut dec).is_some());
        // Every later movement: the MSB is suppressed, only the LSB (carrying
        // the full 14-bit value) dispatches — one action per knob movement.
        assert!(map.handle(cc(10, 101), &mut dec).is_none());
        let a = map.handle(cc(42, 60), &mut dec).expect("LSB dispatches");
        assert_eq!(a.target, Target::EqHigh(Deck::A));
    }

    #[test]
    fn beat_fx_target_kinds() {
        assert_eq!(Target::BeatFxAssign(Deck::A).kind(), Kind::Momentary);
        assert_eq!(Target::BeatFxLevel.kind(), Kind::Continuous);
        for t in [
            Target::BeatFxSelect(1),
            Target::BeatFxSelect(-1),
            Target::BeatFxBeat(-1),
            Target::BeatFxAuto,
            Target::BeatFxTap,
            Target::BeatFxOn,
            Target::BeatFxRelease,
        ] {
            assert_eq!(t.kind(), Kind::Trigger, "{t:?}");
        }
    }

    #[test]
    fn beat_fx_labels() {
        assert_eq!(
            Target::BeatFxAssign(Deck::B).label(),
            "Beat FX · Assign deck B"
        );
        assert_eq!(Target::BeatFxSelect(1).label(), "Beat FX · Select next");
        assert_eq!(Target::BeatFxSelect(-1).label(), "Beat FX · Select prev");
        assert_eq!(Target::BeatFxBeat(1).label(), "Beat FX · Beat ▶");
        assert_eq!(Target::BeatFxBeat(-1).label(), "Beat FX · Beat ◀");
        assert_eq!(Target::BeatFxOn.label(), "Beat FX · On/Off");
        assert_eq!(Target::BeatFxRelease.label(), "Beat FX · Release");
        assert_eq!(Target::BeatFxLevel.label(), "Beat FX · Level/Depth");
        assert_eq!(Target::BeatFxAuto.label(), "Beat FX · Auto BPM");
        assert_eq!(Target::BeatFxTap.label(), "Beat FX · Tap tempo");
    }

    #[test]
    fn pad_mode_select_is_trigger_and_defaults_to_hot_cue() {
        assert_eq!(PadMode::default(), PadMode::HotCue);
        assert_eq!(
            Target::PadModeSelect(Deck::A, PadMode::PadFx2).kind(),
            Kind::Trigger
        );
    }
}
