//! Names: four words from the curated list, and the address they make.
//!
//! An app's public name is four words from `words.txt` joined by hyphens
//! (`ledger-accrual-tally-mint`). Its endpoint is a long random label
//! beneath it (`<secret>.<four-words>.wanderbooks.link`). The relay routes
//! on the four words and checks only that the secret label has the right
//! shape; the app, which made the label, is the one that checks its value.
//!
//! ```text
//! GOAL: an app's public name is four words from a curated finance and
//!       accounting list, never a word a customer would not want in an
//!       address; the relay accepts no other name, so no stranger can
//!       claim `relay`, `ns1`, `_acme-challenge` or anything else that
//!       is not four listed words.
//! SOURCE: WB-relay-3 (names), D-82 in plan/p15-app-and-relay.md.
//! INVARIANTS:
//!   1. A `Name` is four indices into the list, so a name that exists is
//!      four listed words: there is no way to hold any other.
//!   2. Parsing is exact: `Name::parse(&name.to_string()) == name`, and
//!      any string that is not four listed words, hyphen-joined in
//!      lower case, is refused.
//!   3. A host name is routed only as `<secret>.<name>.<domain>`; the
//!      domain's apex, the relay's own host and every other shape are
//!      not an app's.
//! SCENARIOS:
//!   1. `ledger-accrual-tally-mint` parses, and prints as itself.
//!   2. `ledger-accrual-tally` (three words) and `ledger-accrual-tally-zzzz`
//!      (an unlisted word) are refused.
//!   3. `<secret>.ledger-accrual-tally-mint.wanderbooks.link` is that
//!      app's; `relay.wanderbooks.link` is the relay's own host.
//! ```

use std::fmt;
use std::sync::OnceLock;

use ring::rand::{SecureRandom, SystemRandom};

const WORDS_TXT: &str = include_str!("../words.txt");

/// The curated list: one lower-case word per line, sorted, no duplicates.
/// The words are for recognition, so they are finance and accounting
/// terms and pleasant plain words, and none a customer would not want in
/// an address (the list's tests refuse a set of unwelcome words).
pub fn words() -> &'static [&'static str] {
    static WORDS: OnceLock<Vec<&'static str>> = OnceLock::new();
    WORDS.get_or_init(|| WORDS_TXT.lines().filter(|l| !l.is_empty()).collect())
}

/// How many bits of choice four words from the list make.
pub fn name_bits() -> f64 {
    // The list is a few thousand words at most, far inside f64's exact range.
    4.0 * (words().len() as f64).log2()
}

/// Why a name or host was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NameError {
    /// Not exactly four hyphen-joined words.
    #[error("a name is four words joined by hyphens")]
    Shape,
    /// A word that is not on the list.
    #[error("a word that is not on the list")]
    UnknownWord,
}

/// An app's public name: four words from the list, held as their indices.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Name([u16; 4]);

impl Name {
    /// Parse `a-b-c-d`: four listed words, lower case, hyphen-joined.
    pub fn parse(text: &str) -> Result<Self, NameError> {
        let list = words();
        let mut idx = [0u16; 4];
        let mut parts = text.split('-');
        for slot in &mut idx {
            let word = parts.next().ok_or(NameError::Shape)?;
            let at = list
                .binary_search(&word)
                .map_err(|_| NameError::UnknownWord)?;
            *slot = u16::try_from(at).map_err(|_| NameError::Shape)?;
        }
        if parts.next().is_some() {
            return Err(NameError::Shape);
        }
        Ok(Name(idx))
    }

    /// A uniformly random name (rejection sampling, no modulo bias).
    pub fn random() -> Self {
        let rng = SystemRandom::new();
        // Words are fewer than 65 536, so an index fits in a `u16` and the
        // largest multiple of the list's length below 2^16 is the cut.
        let n = u32::try_from(words().len()).unwrap_or(u32::MAX);
        let limit = (0x1_0000 / n) * n;
        let mut idx = [0u16; 4];
        for slot in &mut idx {
            loop {
                let mut b = [0u8; 2];
                if rng.fill(&mut b).is_err() {
                    // The system generator failing is not recoverable.
                    panic!("the system random number generator failed");
                }
                let v = u32::from(u16::from_be_bytes(b));
                if v < limit {
                    *slot = u16::try_from(v % n).unwrap_or(0);
                    break;
                }
            }
        }
        Name(idx)
    }

    /// The four words.
    pub fn words(&self) -> [&'static str; 4] {
        let list = words();
        self.0
            .map(|i| list.get(usize::from(i)).copied().unwrap_or(""))
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let w = self.words();
        write!(f, "{}-{}-{}-{}", w[0], w[1], w[2], w[3])
    }
}

impl fmt::Debug for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Name({self})")
    }
}

/// The shortest and longest secret label the relay will route. The app
/// makes the label, from at least 128 random bits (25 or more random
/// base 36 characters: 25 of them are 129.2 bits); the relay checks only
/// its shape, and refuses a label too short to hold that much.
pub const SECRET_LEN: std::ops::RangeInclusive<usize> = 25..=63;

/// What a host name (a TLS server name or a DNS name) is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKind {
    /// The relay's own host: an app's line dials it.
    Relay,
    /// An assistant reaching the app of this name.
    App(Name),
    /// Anything else.
    Other,
}

/// Which of the relay's own host names is the line's, and the domain.
#[derive(Debug, Clone)]
pub struct Zone {
    domain: String,
    relay_host: String,
}

impl Zone {
    /// `domain` is `wanderbooks.link`, `relay_host` is
    /// `relay.wanderbooks.link`; both are lowered and unrooted.
    pub fn new(domain: &str, relay_host: &str) -> Self {
        Zone {
            domain: normalize(domain),
            relay_host: normalize(relay_host),
        }
    }

    /// The domain this relay serves.
    pub fn domain(&self) -> &str {
        &self.domain
    }

    /// Classify a host name from a ClientHello.
    pub fn classify(&self, host: &str) -> HostKind {
        let host = normalize(host);
        if host == self.relay_host {
            return HostKind::Relay;
        }
        let Some(front) = host
            .strip_suffix(&self.domain)
            .and_then(|f| f.strip_suffix('.'))
        else {
            return HostKind::Other;
        };
        let mut labels = front.split('.');
        let (Some(secret), Some(name), None) = (labels.next(), labels.next(), labels.next()) else {
            return HostKind::Other;
        };
        if !SECRET_LEN.contains(&secret.len())
            || !secret
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        {
            return HostKind::Other;
        }
        match Name::parse(name) {
            Ok(n) => HostKind::App(n),
            Err(_) => HostKind::Other,
        }
    }

    /// The part of a DNS name below the zone, as labels from the left, or
    /// `None` when the name is outside the zone. The apex gives no labels.
    pub fn below<'a>(&self, host: &'a str) -> Option<Vec<&'a str>> {
        let host = host.strip_suffix('.').unwrap_or(host);
        if host.eq_ignore_ascii_case(&self.domain) {
            return Some(Vec::new());
        }
        let cut = host.len().checked_sub(self.domain.len() + 1)?;
        let (front, rest) = host.split_at_checked(cut)?;
        let rest = rest.strip_prefix('.')?;
        if !rest.eq_ignore_ascii_case(&self.domain) {
            return None;
        }
        Some(front.split('.').collect())
    }
}

fn normalize(host: &str) -> String {
    host.trim_end_matches('.').to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_sorted_unique_plain_and_welcome() {
        let list = words();
        assert!(list.len() >= 1024, "{} words", list.len());
        assert!(list.windows(2).all(|w| w[0] < w[1]), "sorted, no repeats");
        for w in list {
            assert!(
                (3..=11).contains(&w.len()) && w.bytes().all(|b| b.is_ascii_lowercase()),
                "{w}"
            );
        }
        // Words a customer would not want in an address.
        for bad in [
            "ass", "sex", "gay", "nig", "fuck", "shit", "damn", "hell", "death", "dead", "kill",
            "war", "gun", "bomb", "poor", "debt", "fail", "bust", "scam", "fraud", "loss", "lose",
            "owe", "deficit", "default", "bankrupt", "arrears", "penalty", "fine", "cancer",
            "porn", "rape", "nazi", "slave", "hate", "drug", "sin",
        ] {
            assert!(list.binary_search(&bad).is_err(), "{bad} is on the list");
        }
        // Words with a reading a customer would not want in an address, and
        // words of death and estates (an assistant's name should not be
        // "executor" or "testament" of anything).
        for bad in ["dong", "probate", "testament", "executor"] {
            assert!(list.binary_search(&bad).is_err(), "{bad} is on the list");
        }
        assert!(name_bits() > 40.0, "{} bits", name_bits());
    }

    /// Four words must be at least 40 bits of choice (1 024 words is the
    /// floor), so a name is hard to guess and rare to collide. Removing
    /// words for being confusable must be made up with finance and
    /// accounting words before this goes red.
    #[test]
    fn four_words_are_at_least_40_bits() {
        assert!(words().len() >= 1024, "{} words", words().len());
        assert!(name_bits() >= 40.0, "{} bits", name_bits());
    }

    /// A name read aloud must be unambiguous: no word is the inflection of
    /// another on the list (the base form stays), and no two words sound
    /// alike.
    #[test]
    fn no_word_is_an_inflection_or_a_homophone_of_another() {
        let list = words();
        let has = |w: &str| list.binary_search(&w).is_ok();
        for w in list {
            for suffix in ["s", "es", "d", "ed", "ing", "ies", "ied"] {
                assert!(!has(&format!("{w}{suffix}")), "{w} and {w}{suffix}");
            }
            if let Some(stem) = w.strip_suffix('e') {
                assert!(!has(&format!("{stem}ing")), "{w} and {stem}ing");
            }
            if let Some(stem) = w.strip_suffix('y') {
                for suffix in ["ied", "ies"] {
                    assert!(!has(&format!("{stem}{suffix}")), "{w} and {stem}{suffix}");
                }
            }
            if let Some(last) = w.chars().last() {
                for suffix in ["ed", "ing"] {
                    assert!(
                        !has(&format!("{w}{last}{suffix}")),
                        "{w} and {w}{last}{suffix}"
                    );
                }
            }
        }
        // Words that sound alike: at most one of each group is listed.
        for group in [
            ["peer", "pier", "", ""],
            ["sell", "cell", "", ""],
            ["current", "currant", "", ""],
            ["real", "riel", "reel", ""],
            ["sum", "som", "some", ""],
            ["sole", "sol", "soul", ""],
            ["site", "sight", "cite", ""],
            ["cache", "cash", "", ""],
            ["fair", "fare", "", ""],
            ["steel", "steal", "", ""],
            ["suite", "sweet", "", ""],
            ["route", "root", "", ""],
            ["counsel", "council", "", ""],
            ["principal", "principle", "", ""],
            ["profit", "prophet", "", ""],
            ["stake", "steak", "", ""],
        ] {
            let listed: Vec<_> = group.iter().filter(|w| !w.is_empty() && has(w)).collect();
            assert!(listed.len() <= 1, "alike-sounding words listed: {listed:?}");
        }
        assert!(!has("fulfil"), "it is fulfill");
        assert!(has("fulfill"));
    }

    #[test]
    fn a_name_is_four_listed_words_and_prints_as_itself() {
        let n = Name::parse("ledger-accrual-tally-mint").expect("listed words");
        assert_eq!(n.to_string(), "ledger-accrual-tally-mint");
        assert_eq!(Name::parse(&n.to_string()), Ok(n));
        assert_eq!(Name::parse("ledger-accrual-tally"), Err(NameError::Shape));
        assert_eq!(
            Name::parse("ledger-accrual-tally-mint-mint"),
            Err(NameError::Shape)
        );
        assert_eq!(
            Name::parse("ledger-accrual-tally-zzzz"),
            Err(NameError::UnknownWord)
        );
        assert_eq!(
            Name::parse("Ledger-accrual-tally-mint"),
            Err(NameError::UnknownWord)
        );
        for reserved in ["relay", "ns1", "_acme-challenge", "www", "", "-", "---"] {
            assert!(Name::parse(reserved).is_err(), "{reserved}");
        }
        let r = Name::random();
        assert_eq!(Name::parse(&r.to_string()), Ok(r));
    }

    #[test]
    fn a_host_is_routed_only_as_secret_dot_name_dot_domain() {
        let zone = Zone::new("wanderbooks.link", "relay.wanderbooks.link");
        let secret = "k3j9x0q7m2z8c5v1b4n6a9s2d";
        let name = Name::parse("ledger-accrual-tally-mint").unwrap();
        let host = format!("{secret}.ledger-accrual-tally-mint.wanderbooks.link");
        assert_eq!(zone.classify(&host), HostKind::App(name));
        assert_eq!(zone.classify(&format!("{host}.")), HostKind::App(name));
        assert_eq!(zone.classify(&host.to_uppercase()), HostKind::App(name));
        assert_eq!(zone.classify("relay.wanderbooks.link"), HostKind::Relay);
        for other in [
            "ledger-accrual-tally-mint.wanderbooks.link",
            "wanderbooks.link",
            "short.ledger-accrual-tally-mint.wanderbooks.link",
            "k3j9x0q7m2z8c5v1b4n6a9s2d.ledger-accrual-tally.wanderbooks.link",
            "a.k3j9x0q7m2z8c5v1b4n6a9s2d.ledger-accrual-tally-mint.wanderbooks.link",
            "k3j9x0q7m2z8c5v1b4n6a9s2d.ledger-accrual-tally-mint.example.com",
            "K3J9X0Q7M2Z8C5V1B4N6A9S2D_.ledger-accrual-tally-mint.wanderbooks.link",
            "xwanderbooks.link",
            "",
        ] {
            assert_eq!(zone.classify(other), HostKind::Other, "{other}");
        }
    }

    /// The secret label is at least 128 bits when the app makes it (25 or
    /// more random base 36 characters); a label too short to hold that is
    /// not routed, whatever it spells.
    #[test]
    fn a_secret_label_under_25_characters_is_not_routed() {
        let zone = Zone::new("wanderbooks.link", "relay.wanderbooks.link");
        let name = Name::parse("ledger-accrual-tally-mint").unwrap();
        let on = |secret: &str| {
            zone.classify(&format!(
                "{secret}.ledger-accrual-tally-mint.wanderbooks.link"
            ))
        };
        assert_eq!(on("aaaaaaaaaaaaaaaa"), HostKind::Other, "16 characters");
        assert_eq!(on(&"a".repeat(24)), HostKind::Other, "24 characters");
        assert_eq!(on(&"a".repeat(25)), HostKind::App(name), "25 characters");
        assert_eq!(on(&"a".repeat(63)), HostKind::App(name), "63 characters");
        assert_eq!(on(&"a".repeat(64)), HostKind::Other, "64 characters");
        assert!(
            25.0 * 36f64.log2() >= 128.0,
            "25 base 36 characters are 128 bits"
        );
        assert!(24.0 * 36f64.log2() < 128.0);
    }

    #[test]
    fn below_names_the_labels_under_the_zone() {
        let zone = Zone::new("wanderbooks.link", "relay.wanderbooks.link");
        assert_eq!(zone.below("wanderbooks.link."), Some(vec![]));
        assert_eq!(zone.below("A.b.Wanderbooks.Link"), Some(vec!["A", "b"]));
        assert_eq!(zone.below("xwanderbooks.link"), None);
        assert_eq!(zone.below("example.com"), None);
        assert_eq!(zone.below("link"), None);
    }
}
