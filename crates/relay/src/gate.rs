//! Fixed-size per-address tables.
//!
//! A hash map keyed by address grows with the number of strangers. These
//! tables do not: an address hashes (with a key picked at start) to one of
//! a fixed number of slots, and the slot holds a counter. Two addresses
//! may share a slot, which makes the limit slightly stricter for both,
//! never looser. The memory is the same however many addresses call.
//!
//! ```text
//! GOAL: limit what one address can hold open or ask for, in memory that
//!       does not depend on how many addresses there are.
//! SOURCE: WB-relay-7 (bounds).
//! INVARIANTS:
//!   1. The table never allocates after it is made.
//!   2. A permit's drop gives back exactly what it took: after every
//!      permit is dropped every slot reads zero.
//!   3. An address is refused once its slot holds `limit` permits, and
//!      never admitted past it.
//! SCENARIOS:
//!   1. A limit of 2: two permits for one address, the third is refused;
//!      dropping one lets a new one in.
//!   2. IPv6 addresses count by /64.
//! ```

use std::hash::{BuildHasher, RandomState};
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

/// A fixed table of per-address counters.
pub struct IpGate {
    slots: Box<[AtomicU32]>,
    mask: usize,
    state: RandomState,
    limit: u32,
}

/// One held place in an [`IpGate`]; dropping it frees the place.
pub struct IpPermit {
    gate: Arc<IpGate>,
    slot: usize,
}

impl IpGate {
    /// A table of `slots` (rounded up to a power of two), each address
    /// allowed `limit` permits at once.
    pub fn new(slots: usize, limit: u32) -> Arc<Self> {
        let n = slots.max(2).next_power_of_two();
        Arc::new(IpGate {
            slots: (0..n).map(|_| AtomicU32::new(0)).collect(),
            mask: n - 1,
            state: RandomState::new(),
            limit,
        })
    }

    fn slot(&self, ip: IpAddr) -> usize {
        let key = match ip {
            IpAddr::V4(v4) => u128::from(u32::from(v4)),
            // An IPv6 subscriber has a /64 at least: count them together.
            IpAddr::V6(v6) => {
                u128::from(u64::from_be_bytes(
                    v6.octets()[..8].try_into().unwrap_or([0; 8]),
                )) | (1 << 100)
            }
        };
        // Truncating a hash to a table index is the point.
        (self.state.hash_one(key) as usize) & self.mask
    }

    /// Take a place for `ip`, or `None` when it already holds its limit.
    pub fn acquire(self: &Arc<Self>, ip: IpAddr) -> Option<IpPermit> {
        let slot = self.slot(ip);
        let cell = &self.slots[slot];
        let mut cur = cell.load(Ordering::Relaxed);
        loop {
            if cur >= self.limit {
                return None;
            }
            match cell.compare_exchange_weak(cur, cur + 1, Ordering::AcqRel, Ordering::Relaxed) {
                Ok(_) => {
                    return Some(IpPermit {
                        gate: Arc::clone(self),
                        slot,
                    });
                }
                Err(seen) => cur = seen,
            }
        }
    }

    /// Permits held now, over all slots (for tests and the stats line).
    pub fn held(&self) -> u64 {
        self.slots
            .iter()
            .map(|c| u64::from(c.load(Ordering::Relaxed)))
            .fold(0, u64::saturating_add)
    }
}

impl Drop for IpPermit {
    fn drop(&mut self) {
        self.gate.slots[self.slot].fetch_sub(1, Ordering::AcqRel);
    }
}

/// A fixed table of token buckets, one per address slot, for a loop that
/// owns it (the UDP DNS loop). Memory is fixed.
pub struct RateTable {
    slots: Vec<(std::time::Instant, f64)>,
    mask: usize,
    state: RandomState,
    rate: f64,
    burst: f64,
}

impl RateTable {
    /// `rate` queries per second sustained, `burst` at once.
    pub fn new(slots: usize, rate: u32, burst: u32) -> Self {
        let n = slots.max(2).next_power_of_two();
        let now = std::time::Instant::now();
        RateTable {
            slots: vec![(now, f64::from(burst)); n],
            mask: n - 1,
            state: RandomState::new(),
            rate: f64::from(rate),
            burst: f64::from(burst),
        }
    }

    /// Spend one token for `ip` at `now`; false when it has none.
    pub fn allow(&mut self, ip: IpAddr, now: std::time::Instant) -> bool {
        let key = match ip {
            IpAddr::V4(v4) => u128::from(u32::from(v4)),
            IpAddr::V6(v6) => {
                u128::from(u64::from_be_bytes(
                    v6.octets()[..8].try_into().unwrap_or([0; 8]),
                )) | (1 << 100)
            }
        };
        let slot = (self.state.hash_one(key) as usize) & self.mask;
        let (last, tokens) = &mut self.slots[slot];
        let gained = now.saturating_duration_since(*last).as_secs_f64() * self.rate;
        *tokens = (*tokens + gained).min(self.burst);
        *last = now;
        if *tokens >= 1.0 {
            *tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_limit_of_two_admits_two_and_frees_on_drop() {
        let gate = IpGate::new(1024, 2);
        let ip: IpAddr = "203.0.113.9".parse().unwrap();
        let a = gate.acquire(ip).unwrap();
        let _b = gate.acquire(ip).unwrap();
        assert!(gate.acquire(ip).is_none());
        assert_eq!(gate.held(), 2);
        drop(a);
        let _c = gate.acquire(ip).expect("a place was freed");
        assert!(gate.acquire(ip).is_none());
        assert!(gate.acquire("198.51.100.1".parse().unwrap()).is_some());
    }

    #[test]
    fn every_permit_gives_back_what_it_took() {
        let gate = IpGate::new(64, 1000);
        let mut held = Vec::new();
        for i in 0..500u32 {
            let ip = IpAddr::from(std::net::Ipv4Addr::from(i.wrapping_mul(2_654_435_761)));
            if let Some(p) = gate.acquire(ip) {
                held.push(p);
            }
        }
        assert_eq!(gate.held(), held.len() as u64);
        drop(held);
        assert_eq!(gate.held(), 0);
    }

    #[test]
    fn ipv6_counts_by_the_slash_64() {
        let gate = IpGate::new(1 << 12, 1);
        let a: IpAddr = "2001:db8:1:2::1".parse().unwrap();
        let b: IpAddr = "2001:db8:1:2:ffff::9".parse().unwrap();
        let _p = gate.acquire(a).unwrap();
        assert!(gate.acquire(b).is_none(), "same /64");
    }

    #[test]
    fn a_rate_table_allows_a_burst_then_the_rate() {
        let mut t = RateTable::new(256, 10, 3);
        let ip: IpAddr = "203.0.113.9".parse().unwrap();
        let t0 = std::time::Instant::now();
        assert!(t.allow(ip, t0) && t.allow(ip, t0) && t.allow(ip, t0));
        assert!(!t.allow(ip, t0));
        assert!(t.allow(ip, t0 + std::time::Duration::from_millis(150)));
    }
}
