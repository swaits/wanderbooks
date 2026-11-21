//! The line to the relay, as the app holds it: dialling, the words for
//! what goes wrong, backoff, and keeping watch for a computer that slept.
//!
//! ```text
//! GOAL: one outbound connection to the relay that comes back by itself
//!       after anything (no network, the relay down, a laptop that slept
//!       and woke on another network), never spins hot against a relay
//!       that is down, and never keeps a door open once it is told to
//!       close.
//! SOURCE: WB-link-2 (the line), plan p15-app-and-relay.md 6a ("Unit
//!         B3's duties: ping at least every 60 s (a line silent for 180 s
//!         is dropped) ... a yamux stream limit at least the relay's"),
//!         D-80; relay `client.rs` (the line), `config.rs` (limits).
//! INVARIANTS:
//!   1. A dial that fails waits a growing, jittered time (full jitter, at
//!      most `backoff_max`, at least `backoff_min` after a failure) and
//!      starts again. A line that held for `line_stable` (60 s) resets the
//!      backoff and is redialled after a wait spread over `redial_spread`
//!      (0 to 30 s, jittered), so a relay that restarts is not met by
//!      every app in the same second; a line that ended sooner (a relay
//!      that accepts and then drops) does not reset it and waits as a
//!      failed dial does. A nudge (the network changed) resets it and
//!      dials now.
//!   2. The line is pinged at most every `ping_every` (30 s, under the
//!      relay's 60 s duty and 180 s silence limit); a ping that is not
//!      answered in `ping_timeout` ends the line and a new one is made.
//!   3. A step of wall-clock time much longer than the monotonic time
//!      that passed (the computer slept) ends the line and dials again at
//!      once: a connection held across a sleep is not trusted.
//!   4. A relay certificate that is expired or not yet valid is a wrong
//!      clock, said so in words, not "the relay is down".
//!   5. The stream limit offered the relay is more than the relay's own
//!      (callers, control streams and slack).
//! SCENARIOS:
//!   1. The backoff grows from its minimum to its maximum and no further,
//!      never exceeds the maximum with jitter, and resets; a line that
//!      ended young does not reset it, one that held does, and its
//!      redial is spread over `redial_spread`.
//!   2. A wall-clock jump with no monotonic jump is a wake; a small one is
//!      not.
//!   3. Errors read as the right trouble: a refused connection is the
//!      relay down, a name that does not resolve is no internet, an
//!      expired relay certificate is a wrong clock.
//! ```

use crate::status::Trouble;
use relay::client::ClientError;
use relay::proto::Refusal;
use ring::rand::{SecureRandom, SystemRandom};
use std::io;
use std::net::SocketAddr;
use std::time::{Duration, Instant, SystemTime};

/// Where the relay is.
#[derive(Clone, Debug)]
pub enum Dial {
    /// A host name and port, resolved at every dial (so a new network
    /// finds the relay again).
    Host(String, u16),
    /// A fixed address (tests).
    Addr(SocketAddr),
}

/// Every time the link waits, in one place (tests make them short).
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    /// A ping at least this often.
    pub ping_every: Duration,
    /// A ping not answered in this long ends the line.
    pub ping_timeout: Duration,
    /// How often the line looks for a sleep and for a ping due.
    pub tick: Duration,
    /// A dial that takes longer than this has failed.
    pub connect_timeout: Duration,
    /// The shortest wait after a failed dial.
    pub backoff_min: Duration,
    /// The longest wait after a failed dial.
    pub backoff_max: Duration,
    /// A caller's TLS handshake must finish in this long.
    pub handshake: Duration,
    /// The shortest wait after a failed certificate order.
    pub cert_retry_min: Duration,
    /// The longest wait after a failed certificate order.
    pub cert_retry_max: Duration,
    /// An order is given up on after this long.
    pub order_timeout: Duration,
    /// Renewal information is asked again this often.
    pub ari_recheck: Duration,
    /// A wall-clock step this much longer than the monotonic one is a
    /// sleep.
    pub wake_gap: Duration,
    /// A line must have held this long for the backoff to start again from
    /// its minimum.
    pub line_stable: Duration,
    /// A line that held and then ended is redialled after a wait spread
    /// over this long.
    pub redial_spread: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            ping_every: Duration::from_secs(30),
            ping_timeout: Duration::from_secs(15),
            tick: Duration::from_secs(5),
            connect_timeout: Duration::from_secs(20),
            backoff_min: Duration::from_secs(1),
            backoff_max: Duration::from_secs(300),
            handshake: Duration::from_secs(10),
            cert_retry_min: Duration::from_secs(60),
            cert_retry_max: Duration::from_secs(6 * 3600),
            order_timeout: Duration::from_secs(300),
            ari_recheck: Duration::from_secs(6 * 3600),
            wake_gap: Duration::from_secs(30),
            line_stable: Duration::from_secs(60),
            redial_spread: Duration::from_secs(30),
        }
    }
}

/// Streams the app lets the line carry at once: the relay's own limits
/// (8 callers, 2 control, plus slack, relay `config.rs`) with room to
/// spare (INVARIANT 5).
pub const MAX_STREAMS: usize = 64;

/// A random duration in `0..=max`.
pub fn jitter(max: Duration) -> Duration {
    let mut b = [0u8; 4];
    if SystemRandom::new().fill(&mut b).is_err() {
        return max / 2;
    }
    let frac = f64::from(u32::from_be_bytes(b)) / f64::from(u32::MAX);
    max.mul_f64(frac)
}

/// The wait between tries: doubling, with full jitter (INVARIANT 1).
#[derive(Debug)]
pub struct Backoff {
    min: Duration,
    max: Duration,
    failures: u32,
}

impl Backoff {
    /// A backoff from `min` to `max`.
    pub fn new(min: Duration, max: Duration) -> Self {
        Backoff {
            min,
            max,
            failures: 0,
        }
    }

    /// A line held: start from the minimum again.
    pub fn reset(&mut self) {
        self.failures = 0;
    }

    /// The ceiling of the next wait, before jitter.
    pub fn ceiling(&self) -> Duration {
        let doubled = self
            .min
            .saturating_mul(1u32.checked_shl(self.failures.min(20)).unwrap_or(u32::MAX));
        doubled.min(self.max)
    }

    /// The next wait: between the minimum (or the ceiling, if lower) and
    /// the ceiling.
    pub fn wait(&mut self) -> Duration {
        let ceiling = self.ceiling();
        self.failures = self.failures.saturating_add(1);
        let floor = self.min.min(ceiling);
        floor + jitter(ceiling - floor)
    }
}

/// When to dial again, after a dial that failed or a line that ended
/// (INVARIANT 1).
#[derive(Debug)]
pub struct Redial {
    backoff: Backoff,
    stable: Duration,
    spread: Duration,
}

impl Redial {
    /// The waits `t` sets.
    pub fn new(t: &Timing) -> Self {
        Redial {
            backoff: Backoff::new(t.backoff_min, t.backoff_max),
            stable: t.line_stable,
            spread: t.redial_spread,
        }
    }

    /// A dial failed: the next wait.
    pub fn failed(&mut self) -> Duration {
        self.backoff.wait()
    }

    /// A line that was up for `held` has ended: the wait before the next
    /// dial. Only a line that held `line_stable` starts the backoff over.
    pub fn ended(&mut self, held: Duration) -> Duration {
        if held >= self.stable {
            self.backoff.reset();
            jitter(self.spread)
        } else {
            self.backoff.wait()
        }
    }

    /// The network changed: start over.
    pub fn reset(&mut self) {
        self.backoff.reset();
    }

    /// The ceiling of the next failed wait (tests).
    pub fn ceiling(&self) -> Duration {
        self.backoff.ceiling()
    }
}

/// Notices a computer that slept (INVARIANT 3).
pub struct WakeDetector {
    mono: Instant,
    wall: SystemTime,
    gap: Duration,
}

impl WakeDetector {
    /// Start watching.
    pub fn new(gap: Duration) -> Self {
        WakeDetector {
            mono: Instant::now(),
            wall: SystemTime::now(),
            gap,
        }
    }

    /// Whether more wall-clock time than monotonic time has passed since
    /// the last look (and now look again from here).
    pub fn woke(&mut self) -> bool {
        self.woke_with(Instant::now(), SystemTime::now())
    }

    /// [`WakeDetector::woke`] with the two clocks given.
    pub fn woke_with(&mut self, mono: Instant, wall: SystemTime) -> bool {
        let m = mono.saturating_duration_since(self.mono);
        // The wall clock may go backwards (set by hand, or by the network):
        // that is not a sleep.
        let w = wall.duration_since(self.wall).unwrap_or_default();
        self.mono = mono;
        self.wall = wall;
        w > m + self.gap
    }
}

fn is_clock_error(e: &io::Error) -> bool {
    use rustls::CertificateError as C;
    let mut inner: Option<&(dyn std::error::Error + 'static)> =
        e.get_ref().map(|e| e as &(dyn std::error::Error + 'static));
    while let Some(err) = inner {
        if let Some(rustls::Error::InvalidCertificate(c)) = err.downcast_ref::<rustls::Error>() {
            return matches!(
                c,
                C::Expired
                    | C::NotValidYet
                    | C::ExpiredContext { .. }
                    | C::NotValidYetContext { .. }
            );
        }
        inner = err.source();
    }
    false
}

/// An i/o error from dialling, as the trouble a person is told.
pub fn trouble_of_io(e: &io::Error) -> Trouble {
    use io::ErrorKind as K;
    if is_clock_error(e) {
        return Trouble::ClockWrong;
    }
    match e.kind() {
        K::NetworkUnreachable | K::HostUnreachable | K::NetworkDown | K::NotFound => {
            Trouble::NoInternet
        }
        // A name that does not resolve comes back as `Uncategorized` or
        // `Other` from the resolver; the callers say so with `NotFound`.
        _ => Trouble::RelayDown,
    }
}

/// What a line's failure means.
pub enum Failure {
    /// Nothing wrong with this app: say it and try again.
    Trouble(Trouble),
    /// The relay refused.
    Refused(Refusal),
}

/// A client error, as what to do about it.
pub fn failure_of(e: &ClientError) -> Failure {
    match e {
        ClientError::Io(e) => Failure::Trouble(trouble_of_io(e)),
        ClientError::Timeout | ClientError::Closed => Failure::Trouble(Trouble::RelayDown),
        ClientError::Frame(_) => Failure::Trouble(Trouble::Protocol),
        ClientError::Refused(r) => Failure::Refused(*r),
    }
}

/// A refusal, as the trouble a person is told (a name taken is handled by
/// the caller before this).
pub fn trouble_of_refusal(r: Refusal, claimed: bool) -> Trouble {
    match r {
        Refusal::Busy => Trouble::RelayBusy,
        Refusal::RateLimited => Trouble::RateLimited,
        Refusal::Blocked => Trouble::Blocked,
        Refusal::BadRequest => Trouble::Protocol,
        // A claimed name the relay turns away as unknown is one it lost
        // (the relay answers a wrong key and an unknown name alike).
        Refusal::BadProof | Refusal::UnknownName | Refusal::Taken => {
            if claimed {
                Trouble::NameLost
            } else {
                Trouble::RelayBusy
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backoff_grows_stays_under_its_ceiling_and_resets() {
        let mut b = Backoff::new(Duration::from_secs(1), Duration::from_secs(300));
        let mut ceilings = vec![];
        for _ in 0..14 {
            ceilings.push(b.ceiling());
            let w = b.wait();
            assert!(w >= Duration::from_secs(1), "{w:?}");
            assert!(w <= Duration::from_secs(300), "{w:?}");
        }
        assert_eq!(ceilings[0], Duration::from_secs(1));
        assert_eq!(ceilings[1], Duration::from_secs(2));
        assert_eq!(ceilings[3], Duration::from_secs(8));
        assert_eq!(*ceilings.last().unwrap(), Duration::from_secs(300));
        assert!(ceilings.windows(2).all(|w| w[0] <= w[1]));
        b.reset();
        assert_eq!(b.ceiling(), Duration::from_secs(1));
        // Jitter really varies.
        let mut b = Backoff::new(Duration::from_secs(1), Duration::from_secs(1000));
        for _ in 0..8 {
            b.wait();
        }
        let spread: std::collections::BTreeSet<u128> = (0..20)
            .map(|_| {
                let mut c = Backoff::new(Duration::from_secs(1), Duration::from_secs(1000));
                c.failures = 8;
                c.wait().as_millis()
            })
            .collect();
        assert!(spread.len() > 5);
    }

    #[test]
    fn a_line_that_ends_young_keeps_the_backoff_and_one_that_held_is_spread() {
        let t = Timing::default();
        let mut r = Redial::new(&t);
        // A relay that accepts and then drops, again and again: the waits
        // grow to the ceiling and stay there, never the old 1 to 2 s.
        let mut last = Duration::ZERO;
        for _ in 0..12 {
            let c = r.ceiling();
            let w = r.ended(Duration::from_secs(2));
            assert!(
                w >= t.backoff_min && w <= c.max(t.backoff_min),
                "{w:?} {c:?}"
            );
            last = last.max(c);
        }
        assert_eq!(last, t.backoff_max);
        assert_eq!(r.ceiling(), t.backoff_max);
        // A line that held 60 s resets it, and the redial is spread over 0
        // to 30 s: many apps do not come back in the same second.
        let waits: Vec<Duration> = (0..200)
            .map(|_| {
                let mut r = Redial::new(&t);
                r.ended(Duration::from_secs(61))
            })
            .collect();
        assert!(waits.iter().all(|w| *w <= t.redial_spread));
        assert!(waits.iter().any(|w| *w > Duration::from_secs(20)));
        assert!(waits.iter().any(|w| *w < Duration::from_secs(10)));
        r.ended(Duration::from_secs(61));
        assert_eq!(r.ceiling(), t.backoff_min, "held: the backoff starts over");
        // A line that held 59 s does not.
        let mut r = Redial::new(&t);
        r.failed();
        r.failed();
        r.ended(Duration::from_secs(59));
        assert!(r.ceiling() > t.backoff_min * 4);
    }

    #[test]
    fn a_wall_clock_jump_with_no_monotonic_jump_is_a_sleep() {
        let mut w = WakeDetector::new(Duration::from_secs(30));
        let (m0, s0) = (w.mono, w.wall);
        // Five seconds of both: not a sleep.
        assert!(!w.woke_with(m0 + Duration::from_secs(5), s0 + Duration::from_secs(5)));
        // Ten minutes of wall time in five seconds of monotonic time.
        let (m1, s1) = (w.mono, w.wall);
        assert!(w.woke_with(m1 + Duration::from_secs(5), s1 + Duration::from_secs(605)));
        // The clock set backwards is not a sleep.
        let (m2, s2) = (w.mono, w.wall);
        assert!(!w.woke_with(m2 + Duration::from_secs(5), s2 - Duration::from_secs(3600)));
    }

    #[test]
    fn errors_read_as_the_right_trouble() {
        let refused = io::Error::from(io::ErrorKind::ConnectionRefused);
        assert_eq!(trouble_of_io(&refused), Trouble::RelayDown);
        let down = io::Error::from(io::ErrorKind::NetworkUnreachable);
        assert_eq!(trouble_of_io(&down), Trouble::NoInternet);
        let expired = io::Error::new(
            io::ErrorKind::InvalidData,
            rustls::Error::InvalidCertificate(rustls::CertificateError::Expired),
        );
        assert_eq!(trouble_of_io(&expired), Trouble::ClockWrong);
        let early = io::Error::new(
            io::ErrorKind::InvalidData,
            rustls::Error::InvalidCertificate(rustls::CertificateError::NotValidYet),
        );
        assert_eq!(trouble_of_io(&early), Trouble::ClockWrong);
        let unknown_ca = io::Error::new(
            io::ErrorKind::InvalidData,
            rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer),
        );
        assert_eq!(trouble_of_io(&unknown_ca), Trouble::RelayDown);
        assert_eq!(
            trouble_of_refusal(Refusal::BadProof, true),
            Trouble::NameLost
        );
        assert_eq!(
            trouble_of_refusal(Refusal::RateLimited, false),
            Trouble::RateLimited
        );
        assert_eq!(trouble_of_refusal(Refusal::Blocked, true), Trouble::Blocked);
    }
}
