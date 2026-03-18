//! Behavioral simulation for bot-detection evasion.
//!
//! This module generates human-like mouse movements, typing cadences, and
//! scroll sequences so that automated browser interactions are
//! indistinguishable from organic user input.
//!
//! All randomness uses the `rand` crate and the Box-Muller transform for
//! Gaussian distributions.  Public methods are instrumented with `tracing`
//! for observability.

use rand::Rng;
use serde::{Deserialize, Serialize};
use tracing::{debug, instrument};

// ---------------------------------------------------------------------------
// Scroll step
// ---------------------------------------------------------------------------

/// A single discrete scroll action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrollStep {
    /// Number of pixels to scroll (positive = down).
    pub pixels: i32,
    /// Delay in milliseconds before this step is executed.
    pub delay_ms: u64,
}

// ---------------------------------------------------------------------------
// Random helpers
// ---------------------------------------------------------------------------

/// Generate a sample from a Gaussian (normal) distribution using the
/// Box-Muller transform.
///
/// Returns a value with the given `mean` and `std_dev`.
pub fn gaussian(mean: f64, std_dev: f64) -> f64 {
    let mut rng = rand::thread_rng();
    let u1: f64 = rng.gen_range(f64::MIN_POSITIVE..1.0);
    let u2: f64 = rng.gen_range(0.0..std::f64::consts::TAU);
    let z = (-2.0 * u1.ln()).sqrt() * u2.cos();
    mean + z * std_dev
}

/// Add uniform noise of up to `±pct`% to `value`.
pub fn jitter(value: f64, pct: f64) -> f64 {
    let mut rng = rand::thread_rng();
    let noise = rng.gen_range(-pct..pct) / 100.0;
    value * (1.0 + noise)
}

// ---------------------------------------------------------------------------
// HumanBehavior
// ---------------------------------------------------------------------------

/// Configuration for human-like behavioural simulation.
///
/// All public methods produce deterministic *shapes* (e.g. Bezier curves,
/// momentum ramps) but inject controlled randomness so that no two
/// invocations produce identical output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanBehavior {
    /// Target typing speed in words-per-minute (1 word = 5 chars).
    pub typing_speed_wpm: f64,
    /// Mouse speed multiplier (1.0 = normal, <1 = slower, >1 = faster).
    pub mouse_speed: f64,
    /// Scroll smoothness factor in `[0, 1]`. Higher values produce more
    /// intermediate steps and gentler acceleration curves.
    pub scroll_smoothness: f64,
}

impl Default for HumanBehavior {
    fn default() -> Self {
        Self {
            typing_speed_wpm: 60.0,
            mouse_speed: 1.0,
            scroll_smoothness: 0.8,
        }
    }
}

impl HumanBehavior {
    /// Create a new `HumanBehavior` with the given parameters.
    pub fn new(typing_speed_wpm: f64, mouse_speed: f64, scroll_smoothness: f64) -> Self {
        Self {
            typing_speed_wpm,
            mouse_speed,
            scroll_smoothness,
        }
    }

    // ---------------------------------------------------------------------
    // Mouse movement
    // ---------------------------------------------------------------------

    /// Compute movement duration in milliseconds using Fitts's Law.
    ///
    /// `movement_time = a + b * log2(distance / target_width + 1)`
    ///
    /// where `a = 150` and `b = 100`.  The result is scaled by the inverse
    /// of [`Self::mouse_speed`] so that higher speed values produce shorter
    /// durations.
    #[instrument(skip(self))]
    pub fn movement_duration_ms(&self, distance: f64, target_width: f64) -> u64 {
        let a: f64 = 150.0;
        let b: f64 = 100.0;
        let raw = a + b * (distance / target_width + 1.0).log2();
        let scaled = raw / self.mouse_speed;
        debug!(distance, target_width, raw, scaled, "fitts_law");
        scaled.max(1.0) as u64
    }

    /// Generate a human-like mouse path from `from` to `to`.
    ///
    /// The path is a cubic Bezier curve with randomised control points.  For
    /// distances greater than 500 px an overshoot point is appended (the
    /// cursor moves past the target and then corrects).  Each point receives
    /// Gaussian micro-jitter of 1-3 px.
    ///
    /// Returns roughly 20-50 intermediate points depending on distance.
    #[instrument(skip(self))]
    pub fn generate_mouse_path(
        &self,
        from: (f64, f64),
        to: (f64, f64),
    ) -> Vec<(f64, f64)> {
        let mut rng = rand::thread_rng();

        let dx = to.0 - from.0;
        let dy = to.1 - from.1;
        let distance = (dx * dx + dy * dy).sqrt();

        // Number of points scales with distance, clamped to [20, 50].
        let num_points = ((distance / 20.0).round() as usize).clamp(20, 50);

        // Perpendicular offset direction.
        let perp = if distance > 0.0 {
            (-dy / distance, dx / distance)
        } else {
            (0.0, 1.0)
        };

        // Randomised control points for a cubic Bezier.
        let spread = distance * 0.3;
        let offset1 = gaussian(0.0, spread);
        let offset2 = gaussian(0.0, spread);

        let cp1 = (
            from.0 + dx * 0.33 + perp.0 * offset1,
            from.1 + dy * 0.33 + perp.1 * offset1,
        );
        let cp2 = (
            from.0 + dx * 0.66 + perp.0 * offset2,
            from.1 + dy * 0.66 + perp.1 * offset2,
        );

        let mut path: Vec<(f64, f64)> = Vec::with_capacity(num_points + 10);

        for i in 0..=num_points {
            let t = i as f64 / num_points as f64;
            let it = 1.0 - t;

            let x = it.powi(3) * from.0
                + 3.0 * it.powi(2) * t * cp1.0
                + 3.0 * it * t.powi(2) * cp2.0
                + t.powi(3) * to.0;
            let y = it.powi(3) * from.1
                + 3.0 * it.powi(2) * t * cp1.1
                + 3.0 * it * t.powi(2) * cp2.1
                + t.powi(3) * to.1;

            // Micro-jitter: 1-3 px Gaussian noise.
            let jx = gaussian(0.0, rng.gen_range(1.0..3.0));
            let jy = gaussian(0.0, rng.gen_range(1.0..3.0));

            path.push((x + jx, y + jy));
        }

        // Overshoot for long distances.
        if distance > 500.0 {
            let overshoot_mag = rng.gen_range(5.0..20.0);
            let dir = if distance > 0.0 {
                (dx / distance, dy / distance)
            } else {
                (1.0, 0.0)
            };
            let overshoot = (
                to.0 + dir.0 * overshoot_mag + gaussian(0.0, 2.0),
                to.1 + dir.1 * overshoot_mag + gaussian(0.0, 2.0),
            );
            path.push(overshoot);

            // Correction steps back to target.
            let correction_steps = rng.gen_range(3..=6);
            for i in 1..=correction_steps {
                let t = i as f64 / correction_steps as f64;
                let cx = overshoot.0 + (to.0 - overshoot.0) * t + gaussian(0.0, 1.0);
                let cy = overshoot.1 + (to.1 - overshoot.1) * t + gaussian(0.0, 1.0);
                path.push((cx, cy));
            }
        }

        debug!(
            from = ?from,
            to = ?to,
            distance,
            points = path.len(),
            overshoot = distance > 500.0,
            "generate_mouse_path"
        );

        path
    }

    // ---------------------------------------------------------------------
    // Typing
    // ---------------------------------------------------------------------

    /// Generate inter-key delays (in ms) for every character in `text`.
    ///
    /// The cadence models realistic typing: common bigrams are faster,
    /// spaces and capitals are slower, and there are occasional pauses
    /// and simulated typos.
    #[instrument(skip(self))]
    pub fn generate_typing_delays(&self, text: &str) -> Vec<u64> {
        let mut rng = rand::thread_rng();

        // Base delay per character (1 word = 5 chars).
        let base_delay = 60_000.0 / (self.typing_speed_wpm * 5.0);
        let std_dev = base_delay * 0.20;

        let common_bigrams: &[&str] = &["th", "he", "in", "er", "an", "re"];

        let chars: Vec<char> = text.chars().collect();
        let mut delays: Vec<u64> = Vec::with_capacity(chars.len());
        let mut since_pause: usize = 0;
        let next_pause_at: usize = rng.gen_range(20..=40);
        let mut pause_counter = next_pause_at;

        for (i, &ch) in chars.iter().enumerate() {
            // 5 % chance of typo simulation.
            if rng.gen_range(0.0..1.0) < 0.05 {
                // Wrong key delay.
                let wrong = gaussian(base_delay, std_dev).max(30.0) as u64;
                // Backspace delay.
                let backspace = gaussian(base_delay * 0.6, std_dev * 0.5).max(30.0) as u64;
                // Correct key delay.
                let correct = gaussian(base_delay, std_dev).max(30.0) as u64;
                // We model the *total* extra time for this character position.
                delays.push(wrong + backspace + correct);
                since_pause += 1;
                if since_pause >= pause_counter {
                    since_pause = 0;
                    pause_counter = rng.gen_range(20..=40);
                }
                continue;
            }

            let mut delay = gaussian(base_delay, std_dev);

            // Bigram acceleration.
            if i > 0 {
                let bigram: String = [chars[i - 1], ch]
                    .iter()
                    .collect::<String>()
                    .to_lowercase();
                if common_bigrams.contains(&bigram.as_str()) {
                    delay *= 0.80;
                }
            }

            // Space after a word.
            if ch == ' ' {
                delay *= 1.20;
            }

            // Capital letter (shift required).
            if ch.is_uppercase() {
                delay *= 1.50;
            }

            delay = delay.max(30.0);
            delays.push(delay as u64);

            since_pause += 1;
            if since_pause >= pause_counter {
                // Inject a thinking pause.
                let pause = rng.gen_range(300..=600) as u64;
                if let Some(last) = delays.last_mut() {
                    *last += pause;
                }
                since_pause = 0;
                pause_counter = rng.gen_range(20..=40);
            }
        }

        debug!(
            text_len = text.len(),
            delays_len = delays.len(),
            base_delay,
            "generate_typing_delays"
        );

        delays
    }

    // ---------------------------------------------------------------------
    // Scrolling
    // ---------------------------------------------------------------------

    /// Generate a momentum-based scroll sequence that covers approximately
    /// `total_pixels`.
    ///
    /// The sequence accelerates from rest, reaches a plateau, then
    /// decelerates.  Micro-pauses are inserted every 500-1000 px.
    #[instrument(skip(self))]
    pub fn generate_scroll_sequence(&self, total_pixels: i32) -> Vec<ScrollStep> {
        let mut rng = rand::thread_rng();

        let direction: i32 = if total_pixels >= 0 { 1 } else { -1 };
        let total = total_pixels.unsigned_abs() as f64;
        if total < 1.0 {
            return vec![];
        }

        let mut steps: Vec<ScrollStep> = Vec::new();
        let mut scrolled: f64 = 0.0;
        let mut next_pause_at: f64 = rng.gen_range(500.0..1000.0);

        while scrolled < total {
            let remaining = total - scrolled;
            let progress = scrolled / total; // 0..1

            // Momentum envelope: slow-fast-slow (sine-ish).
            let envelope = (std::f64::consts::PI * progress).sin().max(0.15);

            // Pixel step: 30-120 scaled by envelope and smoothness.
            let base_step = 30.0 + 90.0 * envelope * self.scroll_smoothness;
            let step_px = jitter(base_step, 15.0).clamp(30.0, 120.0).min(remaining);
            let step_px_i = step_px.round() as i32;

            // Delay: faster at peak, slower at edges (16-50 ms).
            let base_delay = 50.0 - 34.0 * envelope;
            let delay = jitter(base_delay, 10.0).clamp(16.0, 50.0) as u64;

            steps.push(ScrollStep {
                pixels: step_px_i * direction,
                delay_ms: delay,
            });

            scrolled += step_px;

            // Micro-pause.
            if scrolled >= next_pause_at && scrolled < total {
                let pause = rng.gen_range(100..=200);
                steps.push(ScrollStep {
                    pixels: 0,
                    delay_ms: pause,
                });
                next_pause_at = scrolled + rng.gen_range(500.0..1000.0);
            }
        }

        debug!(
            total_pixels,
            steps = steps.len(),
            "generate_scroll_sequence"
        );

        steps
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn behavior() -> HumanBehavior {
        HumanBehavior::default()
    }

    // -- Mouse path --------------------------------------------------------

    #[test]
    fn mouse_path_reasonable_point_count() {
        let path = behavior().generate_mouse_path((0.0, 0.0), (400.0, 300.0));
        assert!(
            (20..=60).contains(&path.len()),
            "expected 20-60 points, got {}",
            path.len()
        );
    }

    #[test]
    fn mouse_path_start_end_close_to_from_to() {
        let from = (100.0, 200.0);
        let to = (500.0, 600.0);
        let path = behavior().generate_mouse_path(from, to);

        let first = path.first().unwrap();
        let last = path.last().unwrap();

        let start_dist = ((first.0 - from.0).powi(2) + (first.1 - from.1).powi(2)).sqrt();
        let end_dist = ((last.0 - to.0).powi(2) + (last.1 - to.1).powi(2)).sqrt();

        assert!(
            start_dist < 20.0,
            "start too far from `from`: {start_dist}"
        );
        assert!(end_dist < 30.0, "end too far from `to`: {end_dist}");
    }

    #[test]
    fn mouse_path_overshoot_for_long_distance() {
        let from = (0.0, 0.0);
        let to = (1000.0, 0.0);
        let path = behavior().generate_mouse_path(from, to);

        // With overshoot + correction the path should have more points than
        // a short-distance path.
        let short_path = behavior().generate_mouse_path((0.0, 0.0), (50.0, 50.0));
        assert!(
            path.len() > short_path.len(),
            "long path ({}) should have more points than short path ({})",
            path.len(),
            short_path.len()
        );

        // At least one point should exceed the target x coordinate.
        let max_x = path.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
        assert!(
            max_x > to.0,
            "expected overshoot past target x={}, max_x={max_x}",
            to.0
        );
    }

    // -- Fitts's Law -------------------------------------------------------

    #[test]
    fn movement_duration_scales_with_distance() {
        let b = behavior();
        let short = b.movement_duration_ms(100.0, 50.0);
        let long = b.movement_duration_ms(1000.0, 50.0);
        assert!(
            long > short,
            "longer distance should take more time: short={short}, long={long}"
        );
    }

    // -- Typing delays -----------------------------------------------------

    #[test]
    fn typing_delays_count_matches_text_length() {
        let text = "Hello, World!";
        let delays = behavior().generate_typing_delays(text);
        assert_eq!(
            delays.len(),
            text.chars().count(),
            "one delay per character"
        );
    }

    #[test]
    fn typing_delays_in_reasonable_range() {
        let text = "The quick brown fox jumps over the lazy dog.";
        let delays = behavior().generate_typing_delays(text);
        for (i, &d) in delays.iter().enumerate() {
            assert!(
                d >= 30 && d <= 1500,
                "delay[{i}] = {d} ms is out of range 30-1500"
            );
        }
    }

    // -- Scrolling ---------------------------------------------------------

    #[test]
    fn scroll_sequence_total_approximately_equals_target() {
        let target = 2000;
        let steps = behavior().generate_scroll_sequence(target);
        let total: i32 = steps.iter().map(|s| s.pixels).sum();
        let diff = (total - target).abs();
        assert!(
            diff <= 120,
            "scroll total {total} too far from target {target} (diff={diff})"
        );
    }

    #[test]
    fn scroll_sequence_negative_direction() {
        let target = -800;
        let steps = behavior().generate_scroll_sequence(target);
        let total: i32 = steps.iter().map(|s| s.pixels).sum();
        assert!(
            total < 0,
            "negative scroll should produce negative total, got {total}"
        );
    }

    // -- Gaussian helper ---------------------------------------------------

    #[test]
    fn gaussian_values_around_mean() {
        let mean = 100.0;
        let std = 10.0;
        let samples: Vec<f64> = (0..1000).map(|_| gaussian(mean, std)).collect();
        let avg: f64 = samples.iter().sum::<f64>() / samples.len() as f64;
        assert!(
            (avg - mean).abs() < 5.0,
            "average {avg} too far from mean {mean}"
        );
    }
}
