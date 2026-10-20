//! The lease socket's messages (`run/lease.sock`, plan 3.9).
//!
//! ```text
//! GOAL: holders hold the books open, and the owner's few requests reach
//!       the server, through one socket that accepts nothing else.
//! SOURCE: plan/served.md 3.9 ("Who holds them", "Three local channels":
//!         leases record their kind and PID; exactly five owner intents;
//!         consent carries the owner passphrase (a replacing restore no
//!         longer does: D-31, the books it replaces are kept in legacy/);
//!         "Closing": the host says why on each lease; protocol versions N
//!         and N-1).
//! INVARIANTS:
//!   1. A connection's first line is one Ask; a Hold keeps the
//!      connection (the lease lasts until it closes), anything else is
//!      answered once and the connection closed.
//!   2. Exactly five intents exist: close, back_up_now, new_books,
//!      restore (a backup's address or folder, as `books restore` takes
//!      it) and consent_format_upgrade; any other request is refused.
//!   3. consent_format_upgrade carries the owner passphrase; without one
//!      it is refused before anything is done.
//!   4. Before the host ends a lease it sends the reason; a lease that
//!      ends with no reason means the host is gone.
//!   5. Each side accepts its own protocol version and the one before.
//!   6. Lines are at most MAX_LINE bytes.
//!   7. A first line is exactly its version and one ask: anything more
//!      (two asks, an unknown key) is refused, never half-read.
//! ```

use serde::{Deserialize, Serialize};

/// This protocol's version (INVARIANT 5).
pub const PROTO: u32 = 1;
/// The oldest version still accepted (there is none before the first).
pub const OLDEST: u32 = 1;
/// The host's socket, in the root's `run/`.
pub const LEASE_SOCKET: &str = "lease.sock";
/// Held by the one host for a root.
pub const HOST_LOCK: &str = "host.lock";
/// Held while the host restarts; openers wait on it.
pub const UPDATE_LOCK: &str = "update.lock";
/// The longest line either side sends (INVARIANT 6).
pub const MAX_LINE: usize = 64 * 1024;

/// Whether a message of protocol version `proto` is accepted.
pub fn accepts(proto: u32) -> bool {
    (OLDEST..=PROTO).contains(&proto)
}

/// What kind of holder a lease is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// The app (the viewer).
    App,
    /// An agent session's bridge (`wanderbooks mcp`).
    Bridge,
    /// A terminal running `wanderbooks serve`.
    Serve,
    /// A command that needs the server (`wanderbooks books ...`).
    Cli,
}

impl Kind {
    /// How a person names it.
    pub fn label(self) -> &'static str {
        match self {
            Kind::App => "the app",
            Kind::Bridge => "an agent session",
            Kind::Serve => "wanderbooks serve",
            Kind::Cli => "a wanderbooks command",
        }
    }
}

/// A connection's first line (INVARIANT 1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    /// The sender's protocol version.
    pub proto: u32,
    /// What it asks.
    #[serde(flatten)]
    pub ask: Ask,
}

/// What a connection asks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Ask {
    /// Hold the books open while this connection lasts.
    Hold {
        /// Who holds them.
        kind: Kind,
        /// The holder's process.
        pid: u32,
        /// The book it works on, if one.
        #[serde(default)]
        book: Option<String>,
    },
    /// The leases held now (the app and `wanderbooks doctor` list them).
    Leases,
    /// One of the owner's intents.
    Intent(Intent),
}

/// The owner's intents: exactly these five (INVARIANT 2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Intent {
    /// Close the books now: every lease ends.
    Close,
    /// Back a book up now.
    BackUpNow {
        /// Which.
        book: String,
    },
    /// Start new books.
    NewBooks {
        /// Their id.
        id: String,
        /// Their base currency.
        currency: String,
    },
    /// Restore a backup, as its own book or replacing one (kept in
    /// legacy/).
    Restore {
        /// The backup: a repository's address, or its folder (`<book>.git`
        /// in the backup folder).
        source: String,
        /// The books' id.
        id: String,
        /// Replace the books of that id.
        #[serde(default)]
        replace: bool,
    },
    /// Let the server upgrade a book's journal format.
    ConsentFormatUpgrade {
        /// Which.
        book: String,
        /// The owner passphrase (INVARIANT 3).
        #[serde(default)]
        passphrase: Option<String>,
    },
}

impl Intent {
    /// Refuse an intent that needs the passphrase and has none
    /// (INVARIANT 3).
    pub fn check_passphrase(&self) -> Result<(), String> {
        let missing = |p: &Option<String>| p.as_deref().is_none_or(|p| p.is_empty());
        match self {
            Intent::ConsentFormatUpgrade { passphrase, .. } if missing(passphrase) => Err(
                "consenting to a format upgrade needs the owner passphrase, typed in the app"
                    .into(),
            ),
            _ => Ok(()),
        }
    }
}

/// Why the host ends a lease (INVARIANT 4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// The owner closed the books ("Close the books", `wanderbooks
    /// close`): reopen only in a new session.
    ClosedByOwner,
    /// The host is restarting (an update): hold again once it is back.
    Restarting,
    /// The host is closing because nothing held it, or its restart or
    /// start went wrong (never the owner's close, which is
    /// `ClosedByOwner`): hold again, and a new host will answer.
    Closing,
    /// The books server stopped or couldn't start.
    ServerFailed,
}

/// One lease, as listed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseInfo {
    /// Its number.
    pub id: u64,
    /// Who holds it.
    pub kind: Kind,
    /// The holder's process.
    pub pid: u32,
    /// The book it works on, if one.
    #[serde(default)]
    pub book: Option<String>,
}

/// What the host sends.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum HostMsg {
    /// The lease is held; the books are open or opening.
    Held {
        /// The lease's number.
        lease: u64,
    },
    /// The books server is ready.
    Ready,
    /// The lease ends, and why (INVARIANT 4).
    Ending {
        /// Why.
        reason: Reason,
        /// More, in plain words (no path).
        #[serde(default)]
        detail: Option<String>,
    },
    /// An intent was carried out.
    Done {
        /// What came of it, in plain words.
        #[serde(default)]
        detail: Option<String>,
    },
    /// An intent was sent on, and no answer came back: it may or may not
    /// have been carried out, in plain words.
    Uncertain(String),
    /// The leases held now, and whether the books are open yet.
    Leases {
        /// The books server is ready.
        ready: bool,
        /// The leases.
        leases: Vec<LeaseInfo>,
    },
    /// Refused, with why.
    Refused(String),
}

/// A connection's first line, read strictly (INVARIANT 7).
pub fn parse_hello(text: &str) -> Result<Hello, String> {
    strict_pair(text).and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

/// `text` as a JSON object of exactly two keys, `proto` and one more.
pub(crate) fn strict_pair(text: &str) -> Result<serde_json::Value, String> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    match value.as_object() {
        Some(o) if o.len() == 2 && o.contains_key("proto") => Ok(value),
        Some(_) => Err("a message is its protocol version and exactly one request".into()),
        None => Err("a message is a JSON object".into()),
    }
}

/// One line: the message as JSON and a newline.
pub fn line<T: Serialize>(message: &T) -> String {
    let mut text = serde_json::to_string(message).expect("a lease message is plain data");
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(v: serde_json::Value) -> Result<Hello, serde_json::Error> {
        serde_json::from_value(v)
    }

    #[test]
    fn a_hold_names_its_kind_pid_and_book() {
        let h = parse(json!({"proto": 1, "hold": {"kind": "bridge", "pid": 7, "book": "acme"}}))
            .unwrap();
        assert_eq!(
            h.ask,
            Ask::Hold {
                kind: Kind::Bridge,
                pid: 7,
                book: Some("acme".into())
            }
        );
        assert_eq!(
            serde_json::to_value(&h).unwrap(),
            json!({"proto": 1, "hold": {"kind": "bridge", "pid": 7, "book": "acme"}})
        );
    }

    #[test]
    fn exactly_five_intents_are_understood() {
        for intent in [
            json!("close"),
            json!({"back_up_now": {"book": "acme"}}),
            json!({"new_books": {"id": "acme", "currency": "USD"}}),
            json!({"restore": {"source": "/backups/acme.git", "id": "acme"}}),
            json!({"consent_format_upgrade": {"book": "acme", "passphrase": "x"}}),
        ] {
            let h = parse(json!({"proto": 1, "intent": intent})).unwrap();
            assert!(matches!(h.ask, Ask::Intent(_)), "{intent}");
        }
        for bad in [
            json!({"proto": 1, "intent": "delete"}),
            json!({"proto": 1, "intent": {"restore": {"path": "/x"}}}),
            json!({"proto": 1, "intent": {"restore": {"source": "/x", "id": "a", "path": "/x"}}}),
            json!({"proto": 1, "intent": {"restore": {"staged": "s1"}}}),
            json!({"proto": 1, "intent": {"run": {"command": "sh"}}}),
            json!({"proto": 1, "exec": "sh"}),
            json!({"proto": 1}),
        ] {
            assert!(parse(bad.clone()).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_hello_is_exactly_its_version_and_one_ask() {
        assert!(parse_hello(r#"{"proto":1,"leases":null}"#).is_ok());
        for bad in [
            r#"{"proto":1,"intent":{"back_up_now":{"book":"a"}},"leases":null}"#,
            r#"{"proto":1,"leases":null,"extra":1}"#,
            r#"{"leases":null,"intent":"close"}"#,
            r#"{"proto":1}"#,
            r#"[1]"#,
            "not json",
        ] {
            assert!(parse_hello(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn only_consent_needs_the_passphrase() {
        let consent = |p: Option<&str>| Intent::ConsentFormatUpgrade {
            book: "acme".into(),
            passphrase: p.map(str::to_string),
        };
        assert!(consent(None).check_passphrase().is_err());
        assert!(consent(Some("")).check_passphrase().is_err());
        assert!(consent(Some("pw")).check_passphrase().is_ok());
        for other in [
            // D-31: replacing keeps the old books in legacy/; no passphrase.
            Intent::Restore {
                source: "/backups/acme.git".into(),
                id: "acme".into(),
                replace: true,
            },
            Intent::Close,
            Intent::BackUpNow { book: "a".into() },
            Intent::NewBooks {
                id: "a".into(),
                currency: "USD".into(),
            },
        ] {
            assert!(other.check_passphrase().is_ok());
        }
    }

    #[test]
    fn versions_this_one_and_the_one_before() {
        assert!(accepts(PROTO));
        assert!(!accepts(PROTO + 1));
        assert!(!accepts(0));
    }

    #[test]
    fn host_messages_round_trip_as_one_line() {
        for m in [
            HostMsg::Held { lease: 3 },
            HostMsg::Ready,
            HostMsg::Ending {
                reason: Reason::Restarting,
                detail: None,
            },
            HostMsg::Done {
                detail: Some("closed".into()),
            },
            HostMsg::Leases {
                ready: true,
                leases: vec![LeaseInfo {
                    id: 1,
                    kind: Kind::Serve,
                    pid: 9,
                    book: None,
                }],
            },
            HostMsg::Refused("no".into()),
            HostMsg::Uncertain("maybe".into()),
        ] {
            let text = line(&m);
            assert!(text.ends_with('\n') && !text[..text.len() - 1].contains('\n'));
            assert_eq!(serde_json::from_str::<HostMsg>(text.trim_end()).unwrap(), m);
        }
        assert_eq!(Kind::Bridge.label(), "an agent session");
    }
}
