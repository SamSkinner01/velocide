//! Fixed-timestep simulation clock.
//!
//! The main loop feeds real frame time into [`FixedTimestep::advance`], runs the returned number
//! of simulation ticks, then renders with [`FixedTimestep::alpha`] to interpolate between the
//! previous and current simulation states.

/// Accumulates real time and hands out whole simulation ticks.
#[derive(Debug, Clone)]
pub struct FixedTimestep {
    dt: f64,
    accumulator: f64,
    /// Upper bound on ticks per `advance` call so a long stall can't spiral.
    max_ticks_per_frame: u32,
}

impl FixedTimestep {
    pub fn new(hz: f64) -> Self {
        Self {
            dt: 1.0 / hz,
            accumulator: 0.0,
            max_ticks_per_frame: 8,
        }
    }

    /// Seconds per tick.
    pub fn dt(&self) -> f64 {
        self.dt
    }

    pub fn hz(&self) -> f64 {
        1.0 / self.dt
    }

    pub fn set_hz(&mut self, hz: f64) {
        self.dt = 1.0 / hz;
        self.accumulator = self.accumulator.min(self.dt);
    }

    /// Adds `frame_seconds` of real time and returns how many ticks should run now.
    pub fn advance(&mut self, frame_seconds: f64) -> u32 {
        self.accumulator += frame_seconds.max(0.0);
        let mut ticks = 0;
        while self.accumulator >= self.dt && ticks < self.max_ticks_per_frame {
            self.accumulator -= self.dt;
            ticks += 1;
        }
        if ticks == self.max_ticks_per_frame {
            // Drop the backlog instead of trying to catch up forever.
            self.accumulator = self.accumulator.min(self.dt);
        }
        ticks
    }

    /// Fraction of a tick that has elapsed since the last simulated tick, in `[0, 1)`.
    pub fn alpha(&self) -> f32 {
        (self.accumulator / self.dt).clamp(0.0, 1.0) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_are_frame_rate_independent() {
        let mut slow = FixedTimestep::new(60.0);
        let mut fast = FixedTimestep::new(60.0);
        let slow_ticks: u32 = (0..30).map(|_| slow.advance(1.0 / 30.0)).sum();
        let fast_ticks: u32 = (0..240).map(|_| fast.advance(1.0 / 240.0)).sum();
        assert!((slow_ticks as i32 - 60).abs() <= 1);
        assert!((fast_ticks as i32 - 60).abs() <= 1);
    }

    #[test]
    fn long_stall_is_clamped() {
        let mut t = FixedTimestep::new(60.0);
        assert_eq!(t.advance(10.0), 8);
        assert!(t.alpha() <= 1.0);
    }
}
