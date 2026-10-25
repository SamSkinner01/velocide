//! Network simulator: injects latency, jitter, packet loss and duplication.
//!
//! A [`NetSim`] is a delay queue for raw UDP datagrams. Our transports
//! ([`crate::transport::UdpLink`]) own two of them — one for outgoing, one for incoming packets —
//! sitting between the UDP socket and netcode, so the simulation applies to *everything*
//! (handshake, keep-alives, renet acks, both channels) in both directions, on the client, the
//! server, or both. Time is driven by the transport's `update(dt)`, so tests running on a
//! virtual clock get deterministic behaviour (with a fixed [`NetSimConfig::seed`]).

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;

/// Simulated network conditions, applied independently to each direction.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NetSimConfig {
    /// One-way delay added to every packet (ms). Applied in both directions, so the round
    /// trip grows by `2 * latency_ms`.
    pub latency_ms: f32,
    /// Each packet's delay varies uniformly within `+-jitter_ms` (packets can be reordered).
    pub jitter_ms: f32,
    /// Chance (0-100) that a packet is dropped.
    pub loss_percent: f32,
    /// Chance (0-100) that a packet is delivered twice (netcode's replay protection rejects
    /// the copy and logs it).
    pub duplicate_percent: f32,
    /// RNG seed for reproducible loss/jitter patterns. 0 = seed from the clock.
    pub seed: u64,
}

impl NetSimConfig {
    pub fn new(latency_ms: f32, jitter_ms: f32, loss_percent: f32) -> Self {
        Self {
            latency_ms,
            jitter_ms,
            loss_percent,
            ..Default::default()
        }
    }

    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    /// Parses `latency[,jitter[,loss[,duplicate]]]`, e.g. `100,20,2` (ms, ms, %, %).
    pub fn parse(text: &str) -> Result<Self, String> {
        let parts: Vec<&str> = text.split(',').map(str::trim).collect();
        if parts.is_empty() || parts.len() > 4 || parts[0].is_empty() {
            return Err(format!(
                "invalid netsim '{text}', expected latency_ms[,jitter_ms[,loss_percent[,duplicate_percent]]]"
            ));
        }
        let mut values = [0.0f32; 4];
        for (i, part) in parts.iter().enumerate() {
            let v: f32 = part
                .parse()
                .map_err(|_| format!("invalid netsim value '{part}' in '{text}'"))?;
            if !v.is_finite() || v < 0.0 {
                return Err(format!("netsim values must be >= 0 (got '{part}')"));
            }
            values[i] = v;
        }
        Ok(Self {
            latency_ms: values[0],
            jitter_ms: values[1],
            loss_percent: values[2].min(100.0),
            duplicate_percent: values[3].min(100.0),
            seed: 0,
        })
    }

    /// True if this configuration changes anything.
    pub fn is_active(&self) -> bool {
        self.latency_ms > 0.0
            || self.jitter_ms > 0.0
            || self.loss_percent > 0.0
            || self.duplicate_percent > 0.0
    }
}

impl FromStr for NetSimConfig {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl std::fmt::Display for NetSimConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}ms +-{}ms, {}% loss, {}% dup",
            self.latency_ms, self.jitter_ms, self.loss_percent, self.duplicate_percent
        )
    }
}

/// Counters of what the simulator did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetSimStats {
    pub packets: u64,
    pub dropped: u64,
    pub duplicated: u64,
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Delayed {
    due: Duration,
    order: u64,
    addr: SocketAddr,
    data: Vec<u8>,
}

/// Delay queue for datagrams (one direction).
pub struct NetSim {
    config: NetSimConfig,
    rng: u64,
    now: Duration,
    order: u64,
    queue: BinaryHeap<Reverse<Delayed>>,
    stats: NetSimStats,
}

impl NetSim {
    pub fn new(config: NetSimConfig) -> Self {
        let seed = if config.seed != 0 {
            config.seed
        } else {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(0x5eed)
        };
        Self {
            config,
            rng: splitmix64(seed) | 1,
            now: Duration::ZERO,
            order: 0,
            queue: BinaryHeap::new(),
            stats: NetSimStats::default(),
        }
    }

    pub fn config(&self) -> &NetSimConfig {
        &self.config
    }

    pub fn stats(&self) -> NetSimStats {
        self.stats
    }

    /// Advances the simulator clock.
    pub fn advance(&mut self, dt: Duration) {
        self.now += dt;
    }

    /// Queues a datagram (or drops it).
    pub fn push(&mut self, addr: SocketAddr, data: &[u8]) {
        self.stats.packets += 1;
        if self.chance(self.config.loss_percent) {
            self.stats.dropped += 1;
            return;
        }
        let copies = if self.chance(self.config.duplicate_percent) {
            self.stats.duplicated += 1;
            2
        } else {
            1
        };
        for _ in 0..copies {
            let jitter = (self.next_f32() * 2.0 - 1.0) * self.config.jitter_ms;
            let delay_ms = (self.config.latency_ms + jitter).max(0.0);
            let due = self.now + Duration::from_secs_f64(delay_ms as f64 / 1000.0);
            self.order += 1;
            self.queue.push(Reverse(Delayed {
                due,
                order: self.order,
                addr,
                data: data.to_vec(),
            }));
        }
    }

    /// Next datagram whose delay has elapsed.
    pub fn pop_due(&mut self) -> Option<(SocketAddr, Vec<u8>)> {
        if self.queue.peek().is_some_and(|d| d.0.due <= self.now) {
            self.queue.pop().map(|Reverse(d)| (d.addr, d.data))
        } else {
            None
        }
    }

    /// Datagrams still in flight.
    pub fn pending(&self) -> usize {
        self.queue.len()
    }

    /// Time until the next datagram is due, if any.
    pub fn time_to_next(&self) -> Option<Duration> {
        self.queue.peek().map(|d| d.0.due.saturating_sub(self.now))
    }

    fn chance(&mut self, percent: f32) -> bool {
        percent > 0.0 && self.next_f32() * 100.0 < percent
    }

    fn next_f32(&mut self) -> f32 {
        // xorshift64*
        self.rng ^= self.rng >> 12;
        self.rng ^= self.rng << 25;
        self.rng ^= self.rng >> 27;
        let v = self.rng.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (v >> 40) as f32 / (1u64 << 24) as f32
    }
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr() -> SocketAddr {
        "127.0.0.1:1234".parse().unwrap()
    }

    #[test]
    fn parses_cli_format() {
        let c = NetSimConfig::parse("100,20,2").unwrap();
        assert_eq!(
            (c.latency_ms, c.jitter_ms, c.loss_percent),
            (100.0, 20.0, 2.0)
        );
        assert_eq!(NetSimConfig::parse("50").unwrap().latency_ms, 50.0);
        assert_eq!(
            NetSimConfig::parse("1,2,3,4").unwrap().duplicate_percent,
            4.0
        );
        assert!(NetSimConfig::parse("").is_err());
        assert!(NetSimConfig::parse("a,b").is_err());
        assert!(NetSimConfig::parse("-5").is_err());
        assert!(NetSimConfig::parse("1,2,3,4,5").is_err());
    }

    #[test]
    fn delays_packets() {
        let mut sim = NetSim::new(NetSimConfig::new(100.0, 0.0, 0.0).with_seed(1));
        sim.push(addr(), &[1, 2, 3]);
        sim.advance(Duration::from_millis(99));
        assert!(sim.pop_due().is_none());
        sim.advance(Duration::from_millis(1));
        assert_eq!(sim.pop_due().unwrap().1, vec![1, 2, 3]);
        assert_eq!(sim.pending(), 0);
    }

    #[test]
    fn jitter_stays_in_bounds_and_loss_matches_rate() {
        let mut sim = NetSim::new(NetSimConfig::new(50.0, 20.0, 10.0).with_seed(42));
        for i in 0..10_000u32 {
            sim.push(addr(), &i.to_le_bytes());
        }
        let dropped = sim.stats().dropped;
        assert!((800..1200).contains(&dropped), "dropped {dropped}");
        sim.advance(Duration::from_millis(29));
        assert!(sim.pop_due().is_none());
        sim.advance(Duration::from_millis(42));
        let mut delivered = 0;
        while sim.pop_due().is_some() {
            delivered += 1;
        }
        assert_eq!(delivered as u64, 10_000 - dropped);
    }
}
