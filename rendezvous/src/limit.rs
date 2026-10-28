//! Rate limits: token buckets, and per-IP buckets in a table of bounded size.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;

/// `capacity` tokens, refilled at `rate` per second.
#[derive(Clone, Debug)]
pub struct TokenBucket {
    tokens: f64,
    capacity: f64,
    rate: f64,
    last: Duration,
}

impl TokenBucket {
    /// Starts full.
    pub fn new(capacity: f64, rate: f64, now: Duration) -> Self {
        Self {
            tokens: capacity,
            capacity,
            rate,
            last: now,
        }
    }

    fn refill(&mut self, now: Duration) {
        let dt = now.saturating_sub(self.last).as_secs_f64();
        self.last = self.last.max(now);
        self.tokens = (self.tokens + dt * self.rate).min(self.capacity);
    }

    /// Takes `amount` tokens if there are that many.
    pub fn take(&mut self, now: Duration, amount: f64) -> bool {
        self.refill(now);
        if self.tokens >= amount {
            self.tokens -= amount;
            true
        } else {
            false
        }
    }

    /// Whether `amount` tokens could be taken now (takes none).
    pub fn can_take(&mut self, now: Duration, amount: f64) -> bool {
        self.refill(now);
        self.tokens >= amount
    }

    /// Whether the bucket is full again (so its entry carries no state worth keeping).
    pub fn is_full(&mut self, now: Duration) -> bool {
        self.refill(now);
        self.tokens >= self.capacity
    }
}

/// Limits for one source IP.
#[derive(Clone, Copy, Debug)]
pub struct IpRates {
    /// Control requests (heartbeats, lists, binds...): burst and per second.
    pub control_burst: f64,
    pub control_rate: f64,
    /// Join attempts (each one guesses a code): burst and per second.
    pub join_burst: f64,
    pub join_rate: f64,
}

impl Default for IpRates {
    fn default() -> Self {
        Self {
            control_burst: 40.0,
            control_rate: 10.0,
            join_burst: 5.0,
            join_rate: 0.5,
        }
    }
}

struct IpEntry {
    control: TokenBucket,
    joins: TokenBucket,
}

/// Per-IP buckets for addresses that proved they receive packets (cookie checked first, so a
/// spoofed flood can't fill the table). At most `max_entries` IPs are tracked; when the table
/// is full and none can be pruned, new IPs are refused until it drains.
pub struct IpLimiter {
    rates: IpRates,
    entries: HashMap<IpAddr, IpEntry>,
    max_entries: usize,
}

impl IpLimiter {
    pub fn new(rates: IpRates, max_entries: usize) -> Self {
        Self {
            rates,
            entries: HashMap::new(),
            max_entries,
        }
    }

    fn entry(&mut self, ip: IpAddr, now: Duration) -> Option<&mut IpEntry> {
        if !self.entries.contains_key(&ip) {
            if self.entries.len() >= self.max_entries {
                self.prune(now);
                if self.entries.len() >= self.max_entries {
                    return None;
                }
            }
            let r = self.rates;
            self.entries.insert(
                ip,
                IpEntry {
                    control: TokenBucket::new(r.control_burst, r.control_rate, now),
                    joins: TokenBucket::new(r.join_burst, r.join_rate, now),
                },
            );
        }
        self.entries.get_mut(&ip)
    }

    pub fn allow_control(&mut self, ip: IpAddr, now: Duration) -> bool {
        self.entry(ip, now)
            .is_some_and(|e| e.control.take(now, 1.0))
    }

    pub fn allow_join(&mut self, ip: IpAddr, now: Duration) -> bool {
        self.entry(ip, now).is_some_and(|e| e.joins.take(now, 1.0))
    }

    /// Forgets IPs whose buckets are full again (nothing to remember about them).
    pub fn prune(&mut self, now: Duration) {
        self.entries
            .retain(|_, e| !(e.control.is_full(now) && e.joins.is_full(now)));
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(secs: f64) -> Duration {
        Duration::from_secs_f64(secs)
    }

    #[test]
    fn bucket_bursts_then_refills() {
        let mut b = TokenBucket::new(3.0, 1.0, s(0.0));
        assert!(b.take(s(0.0), 1.0));
        assert!(b.take(s(0.0), 2.0));
        assert!(!b.take(s(0.0), 1.0));
        assert!(!b.take(s(0.5), 1.0));
        assert!(b.take(s(1.0), 1.0));
        // Never above capacity.
        assert!(!b.take(s(100.0), 4.0));
        assert!(b.take(s(100.0), 3.0));
        // Time going backwards doesn't refill.
        assert!(!b.take(s(50.0), 1.0));
    }

    #[test]
    fn joins_are_limited_per_ip() {
        let mut l = IpLimiter::new(IpRates::default(), 16);
        let a: IpAddr = "1.1.1.1".parse().unwrap();
        let b: IpAddr = "2.2.2.2".parse().unwrap();
        let burst = IpRates::default().join_burst as usize;
        for _ in 0..burst {
            assert!(l.allow_join(a, s(0.0)));
        }
        assert!(!l.allow_join(a, s(0.0)));
        assert!(l.allow_join(b, s(0.0)), "other IPs are unaffected");
        assert!(l.allow_join(a, s(2.0)), "refilled at 0.5/s");
    }

    #[test]
    fn table_is_bounded() {
        let mut l = IpLimiter::new(IpRates::default(), 4);
        for i in 0..4u8 {
            assert!(l.allow_control(IpAddr::from([10, 0, 0, i]), s(0.0)));
        }
        // Full, nothing prunable yet (buckets aren't full again): refused.
        assert!(!l.allow_control("10.0.0.9".parse().unwrap(), s(0.0)));
        assert_eq!(l.len(), 4);
        // Later the old entries refill and are pruned.
        assert!(l.allow_control("10.0.0.9".parse().unwrap(), s(60.0)));
        assert_eq!(l.len(), 1);
    }
}
