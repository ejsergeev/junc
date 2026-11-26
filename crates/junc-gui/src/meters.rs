// SPDX-License-Identifier: GPL-3.0-or-later

//! Meter ballistics (architecture §7.6): what a `JMeter` shows from the
//! peaks [`junc_pw::meter::Meter`] collects between two ticks of the
//! ~7 Hz timer. Plain data, tested without a window.

use std::collections::HashMap;
use std::time::Duration;

/// The meter tick: "~7 times a second" (Spec).
pub const TICK: Duration = Duration::from_millis(143);
/// Silence and below, in dBFS (the left end of a `JMeter`).
pub const FLOOR: f32 = -60.0;
/// How fast the bar and the peak tick fall, in dB per second.
const FALL: f32 = 24.0;
/// How long the peak tick stays before it falls.
const HOLD: Duration = Duration::from_millis(1500);

/// dBFS of a linear magnitude (1.0 = full scale), never below [`FLOOR`].
pub fn db(linear: f32) -> f32 {
    if linear.is_finite() && linear > 0.0 {
        (20.0 * linear.log10()).max(FLOOR)
    } else {
        FLOOR
    }
}

/// One meter's bar and peak tick, in dBFS.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ballistics {
    pub level: f32,
    pub peak: f32,
    held: Duration,
}

impl Default for Ballistics {
    fn default() -> Self {
        Self {
            level: FLOOR,
            peak: FLOOR,
            held: Duration::ZERO,
        }
    }
}

impl Ballistics {
    /// Takes the peak since the last tick, `dt` ago: the bar jumps up and
    /// falls at [`FALL`]; the tick holds the highest bar for [`HOLD`], then
    /// falls the same way.
    pub fn tick(&mut self, linear: f32, dt: Duration) {
        let now = db(linear);
        let fall = FALL * dt.as_secs_f32();
        self.level = now.max(self.level - fall).max(FLOOR);
        if self.level >= self.peak {
            self.peak = self.level;
            self.held = Duration::ZERO;
        } else if self.held < HOLD {
            self.held += dt;
        } else {
            self.peak = (self.peak - fall).max(self.level);
        }
    }

    /// What the widget shows, rounded to 0.5 dB so that changes below what
    /// a 4–6 px bar can show cause no repaint.
    pub fn shown(&self) -> (f32, f32) {
        let r = |v: f32| (v * 2.0).round() / 2.0;
        (r(self.level), r(self.peak))
    }
}

/// The loudest channel of each metered node.
pub fn loudest(read: Vec<(String, Vec<f32>)>) -> HashMap<String, f32> {
    read.into_iter()
        .map(|(node, peaks)| (node, peaks.into_iter().fold(0.0, f32::max)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decibels_of_full_scale_half_and_silence() {
        assert!(db(1.0).abs() < 1e-4);
        assert!((db(0.5) + 6.0206).abs() < 1e-3);
        assert!((db(0.0) - FLOOR).abs() < f32::EPSILON);
        assert!((db(1e-9) - FLOOR).abs() < f32::EPSILON);
        assert!((db(f32::NAN) - FLOOR).abs() < f32::EPSILON);
    }

    #[test]
    fn the_bar_jumps_up_and_falls_slowly_the_tick_holds() {
        let mut b = Ballistics::default();
        b.tick(1.0, TICK);
        assert_eq!(b.shown(), (0.0, 0.0), "attack is instant");
        b.tick(0.0, TICK);
        let (level, peak) = b.shown();
        assert!((level + 3.5).abs() < 0.6, "falls ~24 dB/s: {level}");
        assert!(peak.abs() < f32::EPSILON, "the tick holds");
        for _ in 0..12 {
            b.tick(0.0, TICK);
        }
        assert!(b.peak < -1.0, "after the hold the tick falls: {}", b.peak);
        assert!(b.peak >= b.level);
        for _ in 0..30 {
            b.tick(0.0, TICK);
        }
        assert_eq!(b.shown(), (FLOOR, FLOOR), "back to silence");
    }

    #[test]
    fn the_loudest_channel_stands_for_the_node() {
        let l = loudest(vec![
            ("junc.a".into(), vec![0.1, 0.4]),
            ("junc.b".into(), vec![]),
        ]);
        assert!((l["junc.a"] - 0.4).abs() < f32::EPSILON);
        assert!(l["junc.b"].abs() < f32::EPSILON);
    }
}
