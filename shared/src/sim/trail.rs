//! Short position history of a simulated entity (projectiles, thorns, enemies), recorded at
//! the end of every tick — the same moment the server records hitboxes for lag compensation.
//! Lets the simulation rewind *non-hitbox* things (parryable projectiles, ricochet prisms,
//! enemies for melee parries) to a player's view time.

use std::collections::VecDeque;

use glam::Vec3;

/// Samples kept: 40 ticks ≈ 667 ms at 60 Hz (more than `lag_comp_max_ms` + tolerance).
pub const TRAIL_LEN: usize = 40;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Trail {
    samples: VecDeque<(u32, Vec3)>,
}

impl Trail {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the position after simulating `tick` (ticks must increase; a repeated tick
    /// replaces the last sample).
    pub fn record(&mut self, tick: u32, position: Vec3) {
        while self.samples.back().is_some_and(|(t, _)| *t >= tick) {
            self.samples.pop_back();
        }
        if self.samples.len() == TRAIL_LEN {
            self.samples.pop_front();
        }
        self.samples.push_back((tick, position));
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn oldest_tick(&self) -> Option<u32> {
        self.samples.front().map(|s| s.0)
    }

    pub fn newest_tick(&self) -> Option<u32> {
        self.samples.back().map(|s| s.0)
    }

    /// Position at `time` (fractional ticks), interpolated between samples. Before the first
    /// sample: `None` (the entity did not exist yet). After the last: the last sample.
    pub fn at(&self, time: f64) -> Option<Vec3> {
        let (first_tick, first) = *self.samples.front()?;
        if time < first_tick as f64 - 1e-6 {
            return None;
        }
        if time <= first_tick as f64 {
            return Some(first);
        }
        let idx = self.samples.partition_point(|(t, _)| (*t as f64) <= time);
        let (ta, a) = self.samples[idx - 1];
        match self.samples.get(idx) {
            Some(&(tb, b)) => {
                let f = ((time - ta as f64) / (tb - ta).max(1) as f64).clamp(0.0, 1.0) as f32;
                Some(a.lerp(b, f))
            }
            None => Some(a),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolates_and_clamps() {
        let mut tr = Trail::new();
        for t in 10..20u32 {
            tr.record(t, Vec3::new(t as f32, 0.0, 0.0));
        }
        assert_eq!(tr.at(9.0), None);
        assert_eq!(tr.at(10.0), Some(Vec3::new(10.0, 0.0, 0.0)));
        assert!((tr.at(12.5).unwrap().x - 12.5).abs() < 1e-5);
        assert_eq!(tr.at(50.0), Some(Vec3::new(19.0, 0.0, 0.0)));
        for t in 20..100u32 {
            tr.record(t, Vec3::ZERO);
        }
        assert_eq!(tr.oldest_tick(), Some(100 - TRAIL_LEN as u32));
    }
}
