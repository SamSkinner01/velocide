//! Per-player input queue on the server.
//!
//! Clients send every input several times (redundancy), possibly out of order and in bursts.
//! The queue:
//! - keeps only inputs newer than the last processed one, deduplicated by `seq`;
//! - hands out inputs in `seq` order, normally **one per tick** (each input is exactly one
//!   movement step, which keeps client prediction exact);
//! - holds back a small adaptive **de-jitter buffer** (target = 2x measured arrival jitter, at
//!   most [`MAX_BUFFER_TICKS`]) so bursty arrival doesn't make the player stutter for everyone
//!   else;
//! - **catches up** (2, or up to [`MAX_INPUTS_PER_TICK`], per tick) when the queue grows, but
//!   only while a token bucket refilled at ~1 input per tick allows it, so a client sending
//!   inputs faster than real time (speed hack) gains nothing;
//! - drops the oldest inputs if more than [`MAX_PENDING_INPUTS`] pile up.
//!
//! When an input never arrives (a loss burst longer than the redundancy), the next available
//! one is processed and the gap is counted; the client's reconciliation corrects the result.

use std::collections::BTreeMap;

use shared::input::PlayerInput;

/// Hard cap on inputs simulated for one player in one tick.
pub const MAX_INPUTS_PER_TICK: usize = 4;
/// Inputs further ahead than this are dropped (about 1 s at 60 Hz).
pub const MAX_PENDING_INPUTS: usize = 64;
/// Inputs with a `seq` this far beyond the last processed one are rejected as bogus.
pub const MAX_SEQ_AHEAD: u32 = 1024;
/// Upper bound for the adaptive de-jitter buffer.
pub const MAX_BUFFER_TICKS: usize = 4;
/// Token bucket: refill per tick (slightly above 1 to absorb clock drift) and capacity.
const TOKEN_REFILL: f32 = 1.01;
const TOKEN_CAPACITY: f32 = 8.0;

/// Counters returned by [`InputQueue::receive`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReceiveStats {
    pub accepted: u32,
    /// Already queued (redundant copy).
    pub duplicate: u32,
    /// Already processed (redundant copy of an old input) or bogus seq.
    pub stale: u32,
    /// Dropped because too many inputs were queued.
    pub overflow: u32,
}

/// Counters returned by [`InputQueue::take_for_tick`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TakeStats {
    pub processed: u32,
    /// Sequence numbers that never arrived and were skipped.
    pub skipped: u32,
    /// The queue ran dry this tick (the player did not move).
    pub stalled: bool,
}

#[derive(Clone, Debug)]
pub struct InputQueue {
    pending: BTreeMap<u32, PlayerInput>,
    last_seq: u32,
    newest_seq: u32,
    tokens: f32,
    primed: bool,
    /// Smoothed arrival jitter in ticks (RFC 3550 style).
    jitter: f32,
    last_transit: Option<f64>,
}

impl Default for InputQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl InputQueue {
    pub fn new() -> Self {
        Self {
            pending: BTreeMap::new(),
            last_seq: 0,
            newest_seq: 0,
            tokens: TOKEN_CAPACITY,
            primed: false,
            jitter: 0.0,
            last_transit: None,
        }
    }

    /// Sequence number of the last input handed out for simulation.
    pub fn last_seq(&self) -> u32 {
        self.last_seq
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Measured arrival jitter in ticks.
    pub fn jitter_ticks(&self) -> f32 {
        self.jitter
    }

    /// Forget the arrival timing (after a pause, inputs resume with a gap that is not jitter).
    pub fn reset_timing(&mut self) {
        self.last_transit = None;
        self.primed = false;
    }

    /// Current de-jitter buffer target (inputs held back).
    pub fn target_buffer(&self) -> usize {
        ((self.jitter * 2.0).ceil() as usize).min(MAX_BUFFER_TICKS)
    }

    /// Adds the (already sanitised) inputs of one packet. `arrival_tick` is the server time of
    /// arrival in (fractional) ticks, used to measure jitter.
    pub fn receive(&mut self, inputs: &[PlayerInput], arrival_tick: f64) -> ReceiveStats {
        let mut stats = ReceiveStats::default();
        let mut newest = 0;
        for input in inputs {
            let seq = input.seq;
            if seq <= self.last_seq || seq > self.last_seq.saturating_add(MAX_SEQ_AHEAD) {
                stats.stale += 1;
                continue;
            }
            newest = newest.max(seq);
            if self.pending.contains_key(&seq) {
                stats.duplicate += 1;
                continue;
            }
            self.pending.insert(seq, *input);
            stats.accepted += 1;
        }

        if newest > self.newest_seq {
            // Transit time = arrival - send time; the client sends input `seq` at its tick
            // `seq`, so differences between packets measure jitter.
            let transit = arrival_tick - newest as f64;
            if let Some(prev) = self.last_transit {
                let d = (transit - prev).abs() as f32;
                self.jitter += (d.min(30.0) - self.jitter) / 16.0;
            }
            self.last_transit = Some(transit);
            self.newest_seq = newest;
        }

        while self.pending.len() > MAX_PENDING_INPUTS {
            if let Some((seq, _)) = self.pending.pop_first() {
                self.last_seq = seq;
                stats.overflow += 1;
            }
        }
        stats
    }

    /// Moves the inputs to simulate this tick into `out` (ascending seq).
    pub fn take_for_tick(&mut self, out: &mut Vec<PlayerInput>) -> TakeStats {
        let mut stats = TakeStats::default();
        self.tokens = (self.tokens + TOKEN_REFILL).min(TOKEN_CAPACITY);
        let target = self.target_buffer();

        if !self.primed {
            if self.pending.len() > target {
                self.primed = true;
            } else {
                return stats;
            }
        }
        if self.pending.is_empty() {
            self.primed = false;
            stats.stalled = true;
            return stats;
        }

        let len = self.pending.len();
        let want = if len > target + 8 {
            MAX_INPUTS_PER_TICK
        } else if len > target + 2 {
            2
        } else {
            1
        };
        for _ in 0..want {
            if self.tokens < 1.0 {
                break;
            }
            let Some((seq, input)) = self.pending.pop_first() else {
                break;
            };
            stats.skipped += seq - self.last_seq - 1;
            self.last_seq = seq;
            self.tokens -= 1.0;
            stats.processed += 1;
            out.push(input);
        }
        stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(seq: u32) -> PlayerInput {
        PlayerInput {
            seq,
            ..Default::default()
        }
    }

    fn take(q: &mut InputQueue) -> Vec<u32> {
        let mut out = Vec::new();
        q.take_for_tick(&mut out);
        out.iter().map(|i| i.seq).collect()
    }

    #[test]
    fn redundant_copies_are_ignored_and_order_kept() {
        let mut q = InputQueue::new();
        let s = q.receive(&[input(1), input(2), input(3)], 3.0);
        assert_eq!(s.accepted, 3);
        let s = q.receive(&[input(2), input(3), input(4)], 4.0);
        assert_eq!((s.accepted, s.duplicate), (1, 2));
        let mut seen = Vec::new();
        for _ in 0..10 {
            seen.extend(take(&mut q));
        }
        assert_eq!(seen, vec![1, 2, 3, 4]);
        let s = q.receive(&[input(3), input(4), input(5)], 5.0);
        assert_eq!((s.accepted, s.stale), (1, 2));
        assert_eq!(q.receive(&[input(5000)], 6.0).stale, 1);
    }

    #[test]
    fn one_per_tick_at_steady_rate() {
        let mut q = InputQueue::new();
        for seq in 1..=100u32 {
            q.receive(&[input(seq)], seq as f64);
            let got = take(&mut q);
            assert!(got.len() <= 1, "tick {seq}: {got:?}");
        }
        assert!(q.last_seq() >= 99);
    }

    #[test]
    fn gaps_are_skipped_and_counted() {
        let mut q = InputQueue::new();
        q.receive(&[input(1), input(2), input(5)], 5.0);
        let mut out = Vec::new();
        let mut skipped = 0;
        for _ in 0..5 {
            skipped += q.take_for_tick(&mut out).skipped;
        }
        assert_eq!(out.iter().map(|i| i.seq).collect::<Vec<_>>(), vec![1, 2, 5]);
        assert_eq!(skipped, 2);
    }

    #[test]
    fn catches_up_after_a_burst() {
        let mut q = InputQueue::new();
        // Nothing for 10 ticks, then 10 inputs at once.
        for _ in 0..10 {
            assert!(take(&mut q).is_empty());
        }
        let burst: Vec<_> = (1..=10).map(input).collect();
        q.receive(&burst, 10.0);
        let mut ticks = 0;
        while !q.is_empty() {
            take(&mut q);
            ticks += 1;
        }
        assert!(ticks < 8, "took {ticks} ticks");
    }

    #[test]
    fn speed_hack_is_capped() {
        let mut q = InputQueue::new();
        let mut seq = 0;
        let mut processed = 0;
        for tick in 0..600 {
            // Client claims two inputs per tick.
            let batch: Vec<_> = (0..2)
                .map(|_| {
                    seq += 1;
                    input(seq)
                })
                .collect();
            q.receive(&batch, tick as f64);
            processed += take(&mut q).len();
        }
        assert!(
            processed <= 620,
            "processed {processed} inputs in 600 ticks"
        );
        assert!(q.len() <= MAX_PENDING_INPUTS);
    }

    #[test]
    fn jitter_grows_buffer() {
        let mut q = InputQueue::new();
        let mut seed = 7u32;
        for seq in 1..=300u32 {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let jitter = ((seed >> 16) % 5) as f64 - 2.0; // +-2 ticks
            q.receive(&[input(seq)], seq as f64 + jitter);
        }
        assert!(q.target_buffer() >= 2, "jitter {}", q.jitter_ticks());
    }
}
