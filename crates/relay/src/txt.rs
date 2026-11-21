//! The proof records: what the relay's DNS says at `_acme-challenge.<name>`.
//!
//! An app that wants a certificate for its name asks the relay, over its
//! own proven line, to publish the ACME DNS-01 value at
//! `_acme-challenge.<its name>.<domain>`. The record is held in memory for
//! minutes and answered by the DNS server; nothing is stored per user.
//!
//! The request carries no name. The name is the line's, proven at the
//! handshake, so an app can publish for its own name and for no other.
//!
//! ```text
//! GOAL: a certificate for a name can be proven by the app that holds the
//!       name's key and by nobody else, and the relay never proves a name
//!       on its own account.
//! SOURCE: WB-relay-5 (the proof records and the domain's DNS), RFC 8555
//!         section 8.4 (the DNS-01 challenge), D-87.
//! INVARIANTS:
//!   1. `publish` takes the name from the caller's authenticated line;
//!      no value can be placed under any other name.
//!   2. A value is exactly the 43 base64url characters of a SHA-256
//!      digest; any other text is refused.
//!   3. Records expire: after `ttl` they are not answered and the sweep
//!      frees them.
//!   4. Memory is bounded: at most `max_entries` names, `per_name` values
//!      each, and a name's publishes are counted per hour.
//! SCENARIOS:
//!   1. A published value is answered until its ttl, then not.
//!   2. A fifth value for one name (limit 4) is refused; the first four
//!      stand.
//!   3. A store at its entry limit refuses a new name and still serves
//!      the old ones.
//!   4. A thirteenth publish within the hour is refused.
//! ```

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::names::Name;

/// Why a publish was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PublishError {
    /// The value is not a DNS-01 proof value.
    #[error("not a proof value")]
    BadValue,
    /// The store is full, or this name publishes too often.
    #[error("too many proof records")]
    Limit,
}

struct Slot {
    values: Vec<(String, Instant)>,
    window_start: Instant,
    publishes: u32,
}

/// The proof records, in memory.
pub struct TxtStore {
    inner: Mutex<HashMap<Name, Slot>>,
    ttl: Duration,
    max_entries: usize,
    per_name: usize,
    per_hour: u32,
}

const HOUR: Duration = Duration::from_secs(3600);

/// Is this an ACME DNS-01 value: base64url of a SHA-256 digest, no padding?
pub fn is_proof_value(text: &str) -> bool {
    text.len() == 43
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl TxtStore {
    /// A store with these limits.
    pub fn new(ttl: Duration, max_entries: usize, per_name: usize, per_hour: u32) -> Self {
        TxtStore {
            inner: Mutex::new(HashMap::new()),
            ttl,
            max_entries,
            per_name: per_name.max(1),
            per_hour,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<Name, Slot>> {
        // A panic while holding the lock cannot leave the map half-edited
        // in a way that matters more than refusing to serve.
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Publish `value` for `name` at `now`.
    pub fn publish(&self, name: Name, value: &str, now: Instant) -> Result<(), PublishError> {
        if !is_proof_value(value) {
            return Err(PublishError::BadValue);
        }
        let mut map = self.lock();
        if !map.contains_key(&name) && map.len() >= self.max_entries {
            sweep_locked(&mut map, self.ttl, now);
            if map.len() >= self.max_entries {
                return Err(PublishError::Limit);
            }
        }
        let slot = map.entry(name).or_insert_with(|| Slot {
            values: Vec::new(),
            window_start: now,
            publishes: 0,
        });
        if now.saturating_duration_since(slot.window_start) >= HOUR {
            slot.window_start = now;
            slot.publishes = 0;
        }
        if slot.publishes >= self.per_hour {
            return Err(PublishError::Limit);
        }
        slot.values
            .retain(|(_, at)| now.saturating_duration_since(*at) < self.ttl);
        if let Some(held) = slot.values.iter_mut().find(|(v, _)| v == value) {
            held.1 = now; // the same value again only renews it
        } else if slot.values.len() >= self.per_name {
            return Err(PublishError::Limit);
        } else {
            slot.values.push((value.to_owned(), now));
        }
        slot.publishes += 1;
        Ok(())
    }

    /// The live values for `name` at `now`.
    pub fn get(&self, name: &Name, now: Instant) -> Vec<String> {
        self.lock()
            .get(name)
            .map(|s| {
                s.values
                    .iter()
                    .filter(|(_, at)| now.saturating_duration_since(*at) < self.ttl)
                    .map(|(v, _)| v.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Drop every value of `name`. Its publish count stays for the hour.
    pub fn clear(&self, name: &Name) {
        if let Some(slot) = self.lock().get_mut(name) {
            slot.values.clear();
        }
    }

    /// Free everything expired. Called every half minute by the relay.
    pub fn sweep(&self, now: Instant) {
        sweep_locked(&mut self.lock(), self.ttl, now);
    }

    /// Names held (for tests and the stats line).
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// True when nothing is held.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn sweep_locked(map: &mut HashMap<Name, Slot>, ttl: Duration, now: Instant) {
    map.retain(|_, slot| {
        slot.values
            .retain(|(_, at)| now.saturating_duration_since(*at) < ttl);
        !slot.values.is_empty() || now.saturating_duration_since(slot.window_start) < HOUR
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(c: char) -> String {
        std::iter::repeat_n(c, 43).collect()
    }

    fn name(s: &str) -> Name {
        Name::parse(s).unwrap()
    }

    #[test]
    fn a_value_is_answered_until_its_ttl() {
        let s = TxtStore::new(Duration::from_secs(60), 10, 4, 12);
        let n = name("ledger-accrual-tally-mint");
        let t0 = Instant::now();
        s.publish(n, &value('a'), t0).unwrap();
        assert_eq!(s.get(&n, t0 + Duration::from_secs(59)), vec![value('a')]);
        assert!(s.get(&n, t0 + Duration::from_secs(60)).is_empty());
        s.sweep(t0 + Duration::from_secs(61));
        assert_eq!(s.len(), 1, "the publish count is kept for the hour");
        s.sweep(t0 + HOUR + Duration::from_secs(1));
        assert!(s.is_empty());
    }

    #[test]
    fn only_proof_values_are_taken() {
        let s = TxtStore::new(Duration::from_secs(60), 10, 4, 12);
        let n = name("ledger-accrual-tally-mint");
        let t0 = Instant::now();
        for bad in [
            "",
            "short",
            &"a".repeat(44),
            &format!("{}=", "a".repeat(42)),
            &format!("{}.", "a".repeat(42)),
        ] {
            assert_eq!(
                s.publish(n, bad, t0),
                Err(PublishError::BadValue),
                "{bad:?}"
            );
        }
        assert!(s.get(&n, t0).is_empty());
    }

    #[test]
    fn values_per_name_and_names_in_all_are_bounded() {
        let s = TxtStore::new(Duration::from_secs(60), 2, 2, 100);
        let (a, b, c) = (
            name("ledger-accrual-tally-mint"),
            name("ledger-accrual-tally-vault"),
            name("ledger-accrual-tally-audit"),
        );
        let t0 = Instant::now();
        s.publish(a, &value('a'), t0).unwrap();
        s.publish(a, &value('b'), t0).unwrap();
        assert_eq!(s.publish(a, &value('c'), t0), Err(PublishError::Limit));
        s.publish(a, &value('a'), t0)
            .expect("the same value renews");
        s.publish(b, &value('a'), t0).unwrap();
        assert_eq!(s.publish(c, &value('a'), t0), Err(PublishError::Limit));
        assert_eq!(s.get(&a, t0).len(), 2);
        assert_eq!(s.get(&b, t0).len(), 1);
        // After a's values expire there is room again.
        s.publish(c, &value('a'), t0 + Duration::from_secs(61))
            .unwrap_err();
        s.sweep(t0 + HOUR + Duration::from_secs(1));
        s.publish(c, &value('a'), t0 + HOUR + Duration::from_secs(2))
            .unwrap();
    }

    #[test]
    fn publishes_are_counted_per_hour() {
        let s = TxtStore::new(Duration::from_secs(5), 10, 4, 3);
        let n = name("ledger-accrual-tally-mint");
        let t0 = Instant::now();
        for _ in 0..3 {
            s.publish(n, &value('a'), t0).unwrap();
        }
        assert_eq!(s.publish(n, &value('a'), t0), Err(PublishError::Limit));
        s.publish(n, &value('a'), t0 + HOUR).expect("a new hour");
    }

    #[test]
    fn clear_removes_the_values() {
        let s = TxtStore::new(Duration::from_secs(60), 10, 4, 12);
        let n = name("ledger-accrual-tally-mint");
        let t0 = Instant::now();
        s.publish(n, &value('a'), t0).unwrap();
        s.clear(&n);
        assert!(s.get(&n, t0).is_empty());
    }
}
