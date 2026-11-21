//! Fixed-size per-address tables.
//!
//! A hash map keyed by address grows with the number of strangers. These
//! tables do not: an address hashes (with a key picked at start) to one of
//! a fixed number of slots, and the slot holds a counter. Two addresses
//! may share a slot, which makes the limit slightly stricter for both,
//! never looser. The memory is the same however many addresses call.
//!
//! Addresses are counted by what a subscriber holds, never by the full
//! address. An IPv4-mapped IPv6 address (`::ffff:a.b.c.d`, what a
//! dual-stack listener reports for an IPv4 caller) is made canonical
//! first, so every IPv4 caller is its own and not all of them one /64
//! with zero for a prefix. Then:
//!
//! - *host* reach: an IPv4 address, or an IPv6 /64;
//! - *net* reach: an IPv4 /24, or an IPv6 /48 (a site's allocation).
//!
//! ```text
//! GOAL: limit what one address, and one network, can hold open or ask
//!       for, in memory that does not depend on how many addresses there
//!       are.
//! SOURCE: WB-relay-7 (bounds).
//! INVARIANTS:
//!   1. The table never allocates after it is made.
//!   2. A permit's drop gives back exactly what it took: after every
//!      permit is dropped every slot reads zero.
//!   3. An address is refused once its slot holds `limit` permits, and
//!      never admitted past it.
//!   4. An IPv4-mapped IPv6 address and the IPv4 address it maps are one
//!      address, in every table.
//!   5. A windowed count never lets more than its limit through in one
//!      window, for the host prefix and for the net prefix both.
//! SCENARIOS:
//!   1. A limit of 2: two permits for one address, the third is refused;
//!      dropping one lets a new one in.
//!   2. IPv6 addresses count by /64 (host reach) and /48 (net reach).
//!   3. Fifty distinct IPv6 addresses in one /64 get the /64's limit, not
//!      fifty times it.
//!   4. Two different IPv4 callers on a dual-stack listener do not share
//!      a slot.
//! ```

use std::hash::{BuildHasher, RandomState};
use std::net::IpAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How much of an address is one "address" for a table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// An IPv4 address, or an IPv6 /64: one subscriber.
    Host,
    /// An IPv4 /24, or an IPv6 /48: one site or one small provider's block.
    Net,
}

/// The key an address has at this reach, after making it canonical.
pub fn prefix_key(ip: IpAddr, reach: Reach) -> u128 {
    match ip.to_canonical() {
        IpAddr::V4(v4) => {
            let bits = u32::from(v4);
            u128::from(match reach {
                Reach::Host => bits,
                Reach::Net => bits & 0xffff_ff00,
            })
        }
        IpAddr::V6(v6) => {
            let octets = v6.octets();
            let take = match reach {
                Reach::Host => 8,
                Reach::Net => 6,
            };
            let mut key = 0u64;
            for b in &octets[..take] {
                key = (key << 8) | u64::from(*b);
            }
            u128::from(key) | (1 << 100)
        }
    }
}

/// A fixed table of per-address counters.
pub struct IpGate {
    slots: Box<[AtomicU32]>,
    mask: usize,
    state: RandomState,
    limit: u32,
    reach: Reach,
}

/// One held place in an [`IpGate`]; dropping it frees the place.
pub struct IpPermit {
    gate: Arc<IpGate>,
    slot: usize,
}

impl IpGate {
    /// A table of `slots` (rounded up to a power of two), each address
    /// (an IPv4 address, an IPv6 /64) allowed `limit` permits at once.
    pub fn new(slots: usize, limit: u32) -> Arc<Self> {
        Self::with_reach(slots, limit, Reach::Host)
    }

    /// The same, counting a whole network (an IPv4 /24, an IPv6 /48).
    pub fn net(slots: usize, limit: u32) -> Arc<Self> {
        Self::with_reach(slots, limit, Reach::Net)
    }

    fn with_reach(slots: usize, limit: u32, reach: Reach) -> Arc<Self> {
        let n = slots.max(2).next_power_of_two();
        Arc::new(IpGate {
            slots: (0..n).map(|_| AtomicU32::new(0)).collect(),
            mask: n - 1,
            state: RandomState::new(),
            limit,
            reach,
        })
    }

    fn slot(&self, ip: IpAddr) -> usize {
        // Truncating a hash to a table index is the point.
        (self.state.hash_one(prefix_key(ip, self.reach)) as usize) & self.mask
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

/// One token bucket.
#[derive(Debug, Clone, Copy)]
pub struct TokenBucket {
    last: Instant,
    tokens: f64,
    rate: f64,
    burst: f64,
}

impl TokenBucket {
    /// `rate` tokens a second sustained, `burst` at once; starts full.
    pub fn new(rate: u32, burst: u32, now: Instant) -> Self {
        TokenBucket {
            last: now,
            tokens: f64::from(burst),
            rate: f64::from(rate),
            burst: f64::from(burst),
        }
    }

    /// Spend one token at `now`; false when there is none.
    pub fn allow(&mut self, now: Instant) -> bool {
        let gained = now.saturating_duration_since(self.last).as_secs_f64() * self.rate;
        self.tokens = (self.tokens + gained).min(self.burst);
        self.last = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// A fixed table of token buckets, one per address slot, for a loop that
/// owns it (the UDP DNS loop). Memory is fixed.
pub struct RateTable {
    slots: Vec<TokenBucket>,
    mask: usize,
    state: RandomState,
}

impl RateTable {
    /// `rate` queries per second sustained, `burst` at once.
    pub fn new(slots: usize, rate: u32, burst: u32) -> Self {
        let n = slots.max(2).next_power_of_two();
        RateTable {
            slots: vec![TokenBucket::new(rate, burst, Instant::now()); n],
            mask: n - 1,
            state: RandomState::new(),
        }
    }

    /// Spend one token for `ip` at `now`; false when it has none.
    pub fn allow(&mut self, ip: IpAddr, now: Instant) -> bool {
        let slot = (self.state.hash_one(prefix_key(ip, Reach::Host)) as usize) & self.mask;
        self.slots[slot].allow(now)
    }
}

/// A windowed count per address prefix, in fixed memory: at most `host`
/// events per window for one IPv4 address or IPv6 /64, and at most `net`
/// for one IPv4 /24 or IPv6 /48. A count is `(window start, events)` in a
/// slot chosen by a hash with a key picked at start; two prefixes sharing
/// a slot only make the limit stricter for both.
pub struct PrefixLimit {
    inner: Mutex<PrefixTables>,
    state: RandomState,
    mask: usize,
    base: Instant,
    host: u32,
    net: u32,
    window: Duration,
}

struct PrefixTables {
    host: Vec<(u32, u32)>,
    net: Vec<(u32, u32)>,
}

impl PrefixLimit {
    /// Tables of `slots` each (a power of two is made of it).
    pub fn new(slots: usize, host: u32, net: u32, window: Duration) -> Self {
        let n = slots.max(2).next_power_of_two();
        PrefixLimit {
            inner: Mutex::new(PrefixTables {
                host: vec![(0, 0); n],
                net: vec![(0, 0); n],
            }),
            state: RandomState::new(),
            mask: n - 1,
            base: Instant::now(),
            host,
            net,
            window,
        }
    }

    /// Count one event for `ip` at `now`; false (and nothing counted) when
    /// either its host prefix or its network prefix is at its limit.
    pub fn allow(&self, ip: IpAddr, now: Instant) -> bool {
        let secs =
            u32::try_from(now.saturating_duration_since(self.base).as_secs()).unwrap_or(u32::MAX);
        let window = u32::try_from(self.window.as_secs()).unwrap_or(u32::MAX);
        let hs = (self.state.hash_one(prefix_key(ip, Reach::Host)) as usize) & self.mask;
        let ns = (self.state.hash_one(prefix_key(ip, Reach::Net)) as usize) & self.mask;
        let mut guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let t = &mut *guard;
        for cell in [&mut t.host[hs], &mut t.net[ns]] {
            if secs.saturating_sub(cell.0) >= window {
                *cell = (secs, 0);
            }
        }
        if t.host[hs].1 >= self.host || t.net[ns].1 >= self.net {
            return false;
        }
        t.host[hs].1 += 1;
        t.net[ns].1 += 1;
        true
    }
}

/// Addresses that held a proven line lately, in a fixed bitmap: two
/// generations of a two-hash Bloom filter, the old one dropped every
/// `period`. An address is "known" if it marked itself within one to two
/// periods. A false "known" lets a stranger into the small hello reserve;
/// a false "unknown" cannot happen inside the window.
pub struct KnownAddrs {
    inner: Mutex<KnownGens>,
    state: RandomState,
    mask: usize,
    period: Duration,
}

struct KnownGens {
    cur: Vec<u64>,
    prev: Vec<u64>,
    rotated: Instant,
}

impl KnownAddrs {
    /// A filter of `bits` bits per generation (rounded up to a power of
    /// two, at least 64), generations `period` long.
    pub fn new(bits: usize, period: Duration) -> Self {
        let n = bits.max(64).next_power_of_two();
        KnownAddrs {
            inner: Mutex::new(KnownGens {
                cur: vec![0; n / 64],
                prev: vec![0; n / 64],
                rotated: Instant::now(),
            }),
            state: RandomState::new(),
            mask: n - 1,
            period,
        }
    }

    fn bits(&self, ip: IpAddr) -> [usize; 2] {
        let key = prefix_key(ip, Reach::Host);
        // Truncating a hash to a bit index is the point.
        [
            (self.state.hash_one((key, 0u8)) as usize) & self.mask,
            (self.state.hash_one((key, 1u8)) as usize) & self.mask,
        ]
    }

    fn rotate(&self, g: &mut KnownGens, now: Instant) {
        if now.saturating_duration_since(g.rotated) >= self.period {
            std::mem::swap(&mut g.cur, &mut g.prev);
            g.cur.fill(0);
            g.rotated = now;
        }
    }

    /// Remember `ip` as an address that held a proven line.
    pub fn mark(&self, ip: IpAddr, now: Instant) {
        let bits = self.bits(ip);
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        self.rotate(&mut g, now);
        for b in bits {
            g.cur[b / 64] |= 1 << (b % 64);
        }
    }

    /// Did `ip` hold a proven line within one to two periods?
    pub fn contains(&self, ip: IpAddr, now: Instant) -> bool {
        let bits = self.bits(ip);
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        self.rotate(&mut g, now);
        let has = |bitmap: &[u64]| bits.iter().all(|b| bitmap[b / 64] & (1 << (b % 64)) != 0);
        has(&g.cur) || has(&g.prev)
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
    fn v6(a: &str) -> IpAddr {
        a.parse().unwrap()
    }

    #[test]
    fn two_ipv4_callers_on_a_dual_stack_listener_do_not_share_a_slot() {
        // `::ffff:a.b.c.d` has a zero first 64 bits: keyed by /64 raw, every
        // IPv4 caller would be one address and one would exhaust the limit.
        let gate = IpGate::new(1 << 12, 1);
        let _a = gate.acquire(v6("::ffff:203.0.113.9")).unwrap();
        assert!(
            gate.acquire(v6("::ffff:198.51.100.1")).is_some(),
            "another IPv4 caller shared the first one's slot"
        );
        // And the mapped form and the plain form are one address.
        assert!(gate.acquire("203.0.113.9".parse().unwrap()).is_none());
        let mut rates = RateTable::new(1 << 12, 1, 1);
        let t0 = Instant::now();
        assert!(rates.allow(v6("::ffff:203.0.113.9"), t0));
        assert!(rates.allow(v6("::ffff:198.51.100.1"), t0));
        assert!(!rates.allow("203.0.113.9".parse().unwrap(), t0));
    }

    #[test]
    fn a_net_gate_counts_a_slash_48_and_an_ipv4_slash_24() {
        let gate = IpGate::net(1 << 12, 2);
        let a = gate.acquire(v6("2001:db8:1:1::1")).unwrap();
        let _b = gate.acquire(v6("2001:db8:1:ffff::1")).unwrap(); // same /48
        assert!(gate.acquire(v6("2001:db8:1:7::9")).is_none());
        assert!(gate.acquire(v6("2001:db8:2::1")).is_some(), "another /48");
        drop(a);
        assert!(gate.acquire(v6("2001:db8:1:5::1")).is_some());
        let v4 = IpGate::net(1 << 12, 1);
        let _c = v4.acquire("203.0.113.9".parse().unwrap()).unwrap();
        assert!(v4.acquire("203.0.113.200".parse().unwrap()).is_none());
        assert!(v4.acquire("203.0.114.9".parse().unwrap()).is_some());
    }

    #[test]
    fn fifty_addresses_in_one_slash_64_get_the_slash_64s_limit_not_fifty_times_it() {
        let limit = PrefixLimit::new(1 << 12, 5, 1000, Duration::from_secs(3600));
        let t0 = Instant::now();
        let allowed = (1..=50u32)
            .filter(|i| limit.allow(v6(&format!("2001:db8:1:2::{i:x}")), t0))
            .count();
        assert_eq!(allowed, 5);
        // The same addresses written as one /64's other half too.
        assert!(!limit.allow(v6("2001:db8:1:2:aaaa:bbbb:cccc:dddd"), t0));
        // Another /64 is another subscriber.
        assert!(limit.allow(v6("2001:db8:1:3::1"), t0));
    }

    #[test]
    fn one_slash_48_cannot_use_many_slash_64s_to_beat_the_limit() {
        // 5 each per /64, 12 for the /48: a hundred /64s in one /48 get 12.
        let limit = PrefixLimit::new(1 << 12, 5, 12, Duration::from_secs(3600));
        let t0 = Instant::now();
        let allowed = (0..100u32)
            .filter(|i| limit.allow(v6(&format!("2001:db8:9:{i:x}::1")), t0))
            .count();
        assert_eq!(allowed, 12);
        // IPv4: five per address, twelve per /24.
        let allowed = (0..100u32)
            .filter(|i| limit.allow(format!("203.0.113.{i}").parse().unwrap(), t0))
            .count();
        assert_eq!(allowed, 12);
        assert!(
            limit.allow("203.0.114.1".parse().unwrap(), t0),
            "another /24"
        );
    }

    #[test]
    fn a_mapped_address_is_counted_with_its_ipv4_address() {
        let limit = PrefixLimit::new(1 << 12, 2, 100, Duration::from_secs(3600));
        let t0 = Instant::now();
        assert!(limit.allow(v6("::ffff:203.0.113.9"), t0));
        assert!(limit.allow("203.0.113.9".parse().unwrap(), t0));
        assert!(!limit.allow(v6("::ffff:203.0.113.9"), t0));
        // Another mapped IPv4 address is not the same /64 as the first.
        assert!(limit.allow(v6("::ffff:198.51.100.1"), t0));
    }

    #[test]
    fn a_prefix_limit_resets_with_its_window_and_a_refusal_counts_nothing() {
        let limit = PrefixLimit::new(1 << 12, 1, 2, Duration::from_secs(60));
        let t0 = Instant::now();
        assert!(limit.allow("203.0.113.1".parse().unwrap(), t0));
        assert!(limit.allow("203.0.113.2".parse().unwrap(), t0));
        // The /24 is at 2 now; a third address is refused, and that refusal
        // did not spend its own host count.
        assert!(!limit.allow("203.0.113.3".parse().unwrap(), t0));
        let later = t0 + Duration::from_secs(61);
        assert!(limit.allow("203.0.113.3".parse().unwrap(), later));
    }

    #[test]
    fn known_addresses_are_remembered_for_one_to_two_periods() {
        let known = KnownAddrs::new(1 << 16, Duration::from_secs(100));
        let t0 = Instant::now();
        let a: IpAddr = "203.0.113.9".parse().unwrap();
        assert!(!known.contains(a, t0));
        known.mark(a, t0);
        assert!(known.contains(a, t0));
        assert!(known.contains(v6("::ffff:203.0.113.9"), t0), "mapped form");
        assert!(known.contains(a, t0 + Duration::from_secs(150)));
        assert!(!known.contains(a, t0 + Duration::from_secs(250)));
        // A stranger, among many marked addresses, is almost never known.
        let k2 = KnownAddrs::new(1 << 20, Duration::from_secs(100));
        for i in 0..5000u32 {
            k2.mark(
                IpAddr::from(std::net::Ipv4Addr::from(i.wrapping_mul(2_654_435_761))),
                t0,
            );
        }
        let hits = (0..5000u32)
            .filter(|i| {
                k2.contains(
                    IpAddr::from(std::net::Ipv4Addr::from(
                        i.wrapping_mul(40_503).wrapping_add(0x0a00_0001) ^ 0x5555_aaaa,
                    )),
                    t0,
                )
            })
            .count();
        assert!(hits < 50, "{hits} of 5000 strangers looked known");
    }
}
