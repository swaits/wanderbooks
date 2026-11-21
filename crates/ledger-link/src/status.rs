//! What the link says about itself, in words a person can use.
//!
//! ```text
//! GOAL: every state the link can be in ends in plain words and in a
//!       state that recovers by itself: no spinner forever, no jargon, and
//!       never a claim that the local assistants or the window are
//!       affected by the network.
//! SOURCE: WB-link-5 (the states the Connect page can show), plan
//!         p15-app-and-relay.md 3.3 ("progress in plain words ... the
//!         state when the relay cannot be reached, in words, with nothing
//!         lost locally").
//! INVARIANTS:
//!   1. A status is data; the words come from [`LinkStatus::words`] and
//!      [`Trouble::words`], one place, so a test can read what a person
//!      reads.
//!   2. A trouble that ends by itself (no internet, the relay down, a
//!      busy relay) says it is trying again; one that needs the person
//!      (a wrong clock, the address lost) says what to do.
//!   3. No status holds a key, a secret label or a token: the address in
//!      it is only there while the link is on, and a status prints
//!      without it.
//! SCENARIOS:
//!   1. A status with no line and no trouble reads "Connecting to the
//!      relay"; with a line and no certificate, "Getting a certificate";
//!      with both, "Connected".
//!   2. Every trouble has words, and the words of one that mend
//!      themselves say so.
//! ```

/// Where the link has got to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Off: no line, no address served.
    Off,
    /// Getting the address: making the name and having the relay accept it.
    GettingAddress,
    /// The line to the relay is being made again.
    Connecting,
    /// The line is up and the certificate is being got.
    GettingCertificate,
    /// The line is up and the certificate is in place.
    Connected,
}

/// Why the link is not further along.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trouble {
    /// The computer cannot reach the internet (no network, or no way to
    /// find the relay).
    NoInternet,
    /// The internet is there; the relay does not answer.
    RelayDown,
    /// The relay is busy.
    RelayBusy,
    /// Too many new addresses lately, here or everywhere.
    RateLimited,
    /// The relay has turned this address off.
    Blocked,
    /// The relay no longer knows this address.
    NameLost,
    /// The computer's clock is wrong: the relay's certificate looks
    /// expired or not yet valid.
    ClockWrong,
    /// The relay answered in a way this version does not understand.
    Protocol,
    /// The files that keep the address could not be read or written.
    Files(String),
}

impl Trouble {
    /// What a person reads, and what happens next (INVARIANT 2).
    pub fn words(&self) -> String {
        match self {
            Trouble::NoInternet => "This computer can't reach the internet. Your assistants on this computer still work. Web assistants will connect again as soon as the internet is back.".into(),
            Trouble::RelayDown => "Wanderbooks can't reach its relay right now. Nothing is lost, and your assistants on this computer still work. It keeps trying.".into(),
            Trouble::RelayBusy => "The relay is busy. It keeps trying.".into(),
            Trouble::RateLimited => "Many people are setting up web assistants right now, so the relay asked Wanderbooks to wait. It keeps trying, a little later each time.".into(),
            Trouble::Blocked => "The relay has turned this address off. Get a new address, or write to us if you think this is a mistake.".into(),
            Trouble::NameLost => "The relay doesn't know this address any more. Get a new address to carry on; web assistants will need the new one.".into(),
            Trouble::ClockWrong => "This computer's clock looks wrong, so the relay can't be trusted yet. Set the date and time (turn on \"set automatically\"), and Wanderbooks will connect by itself.".into(),
            Trouble::Protocol => "The relay speaks a newer version than this Wanderbooks. Update Wanderbooks to connect.".into(),
            Trouble::Files(why) => format!("Wanderbooks can't keep its web address on this computer: {why}."),
        }
    }

    /// Whether this ends without the person doing anything.
    pub fn mends_itself(&self) -> bool {
        matches!(
            self,
            Trouble::NoInternet
                | Trouble::RelayDown
                | Trouble::RelayBusy
                | Trouble::RateLimited
                | Trouble::ClockWrong
        )
    }
}

/// The link, as it stands.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct LinkStatus {
    /// On or off (the person's switch).
    pub on: bool,
    /// The address, once there is one and while the link is on.
    pub address: Option<String>,
    /// The relay has accepted the name.
    pub claimed: bool,
    /// The line is up.
    pub line_up: bool,
    /// A certificate is in place and valid: when it ends (unix seconds).
    pub cert_expires: Option<i64>,
    /// Why the line is not up.
    pub trouble: Option<Trouble>,
    /// Why the certificate is not in place (or not renewed), in words.
    pub cert_trouble: Option<String>,
    /// Callers served so far.
    pub served: u64,
}

impl std::fmt::Debug for LinkStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The address holds the secret label: it is not printed (3).
        f.debug_struct("LinkStatus")
            .field("on", &self.on)
            .field("has_address", &self.address.is_some())
            .field("claimed", &self.claimed)
            .field("line_up", &self.line_up)
            .field("cert_expires", &self.cert_expires)
            .field("trouble", &self.trouble)
            .field("cert_trouble", &self.cert_trouble)
            .field("served", &self.served)
            .finish()
    }
}

impl LinkStatus {
    /// Where the link has got to.
    pub fn step(&self) -> Step {
        if !self.on {
            Step::Off
        } else if !self.line_up && !self.claimed {
            Step::GettingAddress
        } else if !self.line_up {
            Step::Connecting
        } else if self.cert_expires.is_none() {
            Step::GettingCertificate
        } else {
            Step::Connected
        }
    }

    /// True when a web assistant can reach the books now.
    pub fn reachable(&self) -> bool {
        self.step() == Step::Connected
    }

    /// The one line a person reads about progress.
    pub fn words(&self) -> String {
        match self.step() {
            Step::Off => "Web assistants are off.".into(),
            Step::GettingAddress => "Getting an address…".into(),
            Step::Connecting => "Connecting to the relay…".into(),
            Step::GettingCertificate => "Getting a certificate…".into(),
            Step::Connected => "Connected. Web assistants can reach these books.".into(),
        }
    }

    /// What is wrong, in words, if anything: the line's trouble, else the
    /// certificate's.
    pub fn problem(&self) -> Option<String> {
        if let Some(t) = &self.trouble {
            return Some(t.words());
        }
        self.cert_trouble.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_steps_read_in_order() {
        let mut s = LinkStatus::default();
        assert_eq!(s.step(), Step::Off);
        s.on = true;
        assert_eq!(s.step(), Step::GettingAddress);
        assert_eq!(s.words(), "Getting an address…");
        s.claimed = true;
        assert_eq!(s.step(), Step::Connecting);
        s.line_up = true;
        assert_eq!(s.step(), Step::GettingCertificate);
        s.cert_expires = Some(1);
        assert_eq!(s.step(), Step::Connected);
        assert!(s.reachable());
    }

    #[test]
    fn every_trouble_has_words_and_the_mending_ones_say_so() {
        let all = [
            Trouble::NoInternet,
            Trouble::RelayDown,
            Trouble::RelayBusy,
            Trouble::RateLimited,
            Trouble::Blocked,
            Trouble::NameLost,
            Trouble::ClockWrong,
            Trouble::Protocol,
            Trouble::Files("the disk is full".into()),
        ];
        for t in all {
            let w = t.words();
            assert!(w.ends_with('.'), "{w}");
            assert!(!w.to_lowercase().contains("mcp"));
            if matches!(
                t,
                Trouble::NoInternet
                    | Trouble::RelayDown
                    | Trouble::RelayBusy
                    | Trouble::RateLimited
            ) {
                assert!(
                    w.contains("keeps trying") || w.contains("as soon as"),
                    "{w}"
                );
            }
        }
        // The ones that need the person say what to do.
        assert!(Trouble::ClockWrong.words().contains("Set the date"));
        assert!(Trouble::NameLost.words().contains("Get a new address"));
        // Only local assistants are said to keep working, never anything
        // about the window.
        assert!(
            Trouble::NoInternet
                .words()
                .contains("on this computer still work")
        );
    }

    #[test]
    fn a_status_prints_without_its_address() {
        let s = LinkStatus {
            address: Some("https://secretlabel.a-b-c-d.wanderbooks.link".into()),
            ..LinkStatus::default()
        };
        assert!(!format!("{s:?}").contains("secretlabel"));
    }
}
