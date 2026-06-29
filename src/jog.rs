//! Jog-wheel math — device- and engine-agnostic. Turns a relative jog-tick delta
//! (decoded from a controller via its profile) into either a playhead scrub
//! (degrees of platter rotation, while the platter is touched) or a transient
//! tempo-bend ratio (the outer ring, while playing). The engine applies these;
//! `aurum-control` owns only the arithmetic so it's unit-testable without audio.

/// Platter degrees scrubbed per jog tick. DJ jogs emit hundreds–thousands of
/// ticks per revolution, so one tick is a small rotation: a slow nudge moves a
/// little, a fast spin scrubs a lot.
pub const SCRUB_DEGREES_PER_TICK: f32 = 0.5;

/// Tempo change per jog-bend tick (fraction of playback rate).
const BEND_PER_TICK: f32 = 0.01;
/// Maximum bend (±fraction), so a hard spin can't overspeed the deck.
const BEND_LIMIT: f32 = 0.10;

/// Degrees of platter rotation to scrub for a signed jog-tick `delta`.
pub fn scrub_degrees(delta: i32) -> f32 {
    delta as f32 * SCRUB_DEGREES_PER_TICK
}

/// Transient tempo multiplier for a signed jog-bend `delta` (1.0 = no change),
/// clamped to ±`BEND_LIMIT`.
pub fn bend_ratio(delta: i32) -> f32 {
    1.0 + (delta as f32 * BEND_PER_TICK).clamp(-BEND_LIMIT, BEND_LIMIT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_is_linear_and_signed() {
        assert_eq!(scrub_degrees(2), 1.0);
        assert_eq!(scrub_degrees(-2), -1.0);
        assert_eq!(scrub_degrees(0), 0.0);
    }

    #[test]
    fn bend_is_centred_at_one_and_clamped() {
        assert_eq!(bend_ratio(0), 1.0);
        assert!((bend_ratio(1) - 1.01).abs() < 1e-6);
        assert!((bend_ratio(-1) - 0.99).abs() < 1e-6);
        assert_eq!(bend_ratio(1000), 1.10); // clamped up
        assert_eq!(bend_ratio(-1000), 0.90); // clamped down
    }
}
