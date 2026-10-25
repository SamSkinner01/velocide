//! Clock synchronisation: estimates server time from snapshot arrivals.
//!
//! Each snapshot carries the server tick it describes. On arrival we take the sample
//! `offset = snapshot_tick - local_time_in_ticks`; a smoothed offset gives the tick of the
//! newest snapshot we *should* have right now ([`ClockSync::latest_tick`]) — the reference
//! the interpolation delay is subtracted from. The current server tick is that plus half the
//! round trip ([`ClockSync::server_tick`]). The mean absolute deviation of the samples is the
//! arrival jitter, used to size the interpolation delay.

/// Offset samples further than this (ticks) from the estimate reset it (server restart,
/// big latency change).
const RESYNC_TICKS: f64 = 30.0;
const OFFSET_SMOOTHING: f64 = 0.05;
const JITTER_SMOOTHING: f64 = 0.1;

#[derive(Clone, Debug)]
pub struct ClockSync {
    tick_rate: f64,
    local_time: f64,
    offset: Option<f64>,
    jitter: f64,
    rtt: f64,
    samples: u64,
}

impl ClockSync {
    pub fn new(tick_rate: u32) -> Self {
        Self {
            tick_rate: tick_rate.max(1) as f64,
            local_time: 0.0,
            offset: None,
            jitter: 0.0,
            rtt: 0.0,
            samples: 0,
        }
    }

    pub fn set_tick_rate(&mut self, tick_rate: u32) {
        let new = tick_rate.max(1) as f64;
        if new != self.tick_rate {
            self.tick_rate = new;
            self.offset = None;
        }
    }

    pub fn advance(&mut self, dt: f64) {
        self.local_time += dt.max(0.0);
    }

    /// Round trip time in seconds (from the transport).
    pub fn set_rtt(&mut self, rtt: f64) {
        if rtt.is_finite() && rtt >= 0.0 {
            self.rtt = rtt;
        }
    }

    pub fn rtt(&self) -> f64 {
        self.rtt
    }

    fn local_ticks(&self) -> f64 {
        self.local_time * self.tick_rate
    }

    /// Feeds the arrival of a snapshot for `tick` (now).
    pub fn on_snapshot(&mut self, tick: u32) {
        let sample = tick as f64 - self.local_ticks();
        self.samples += 1;
        match self.offset {
            Some(offset) if (sample - offset).abs() < RESYNC_TICKS => {
                let dev = sample - offset;
                self.jitter += (dev.abs() - self.jitter) * JITTER_SMOOTHING;
                self.offset = Some(offset + dev * OFFSET_SMOOTHING);
            }
            _ => {
                self.offset = Some(sample);
                self.jitter = 0.0;
            }
        }
    }

    pub fn is_synced(&self) -> bool {
        self.offset.is_some()
    }

    /// Estimated tick of the newest snapshot that should have arrived by now.
    pub fn latest_tick(&self) -> Option<f64> {
        self.offset.map(|o| o + self.local_ticks())
    }

    /// Estimated current server tick (newest snapshot + one-way latency).
    pub fn server_tick(&self) -> Option<f64> {
        self.latest_tick()
            .map(|t| t + self.rtt * 0.5 * self.tick_rate)
    }

    /// Snapshot arrival jitter (mean absolute deviation) in ticks.
    pub fn jitter_ticks(&self) -> f64 {
        self.jitter
    }

    pub fn tick_rate(&self) -> f64 {
        self.tick_rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_server_time_through_jitter() {
        let mut c = ClockSync::new(60);
        c.set_rtt(0.2);
        // Server sends tick 2k at server time 2k/60; arrives 100 ms (+-20 ms) later. The
        // client's clock started 5 s after the server's.
        let mut seed = 99u32;
        let mut t = 0.0;
        for k in 300..900u32 {
            let tick = 2 * k;
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let jitter = ((seed >> 16) % 41) as f64 / 1000.0 - 0.02;
            let arrival = tick as f64 / 60.0 + 0.1 + jitter - 5.0;
            c.advance(arrival - t);
            t = arrival;
            c.on_snapshot(tick);
        }
        // At local time t the server is at (t + 5) s.
        let truth = (t + 5.0) * 60.0;
        let est = c.server_tick().unwrap();
        assert!((est - truth).abs() < 1.5, "est {est} truth {truth}");
        let latest = c.latest_tick().unwrap();
        assert!((latest - (truth - 6.0)).abs() < 1.5, "latest {latest}");
        let j = c.jitter_ticks();
        assert!(j > 0.2 && j < 1.2, "jitter {j}");
    }

    #[test]
    fn resyncs_on_big_jump() {
        let mut c = ClockSync::new(60);
        c.on_snapshot(100);
        c.advance(1.0);
        c.on_snapshot(5000);
        assert!((c.latest_tick().unwrap() - 5000.0).abs() < 1e-6);
    }
}
