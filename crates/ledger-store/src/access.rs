//! A book's access log (plan I-9, 3.4: `books/<id>/access.jsonl`): one
//! record for every call that reaches the book through the server,
//! refusals included.
//!
//! ```text
//! GOAL: the owner can see who reached the books, when, what they asked
//!       for and what came of it, and a record can't be changed or
//!       dropped unseen.
//! SOURCE: plan I-9 ("every access through the server is logged, and
//!         logging fails closed"), 3.4 (hash-chained, own genesis
//!         "wanderbooks-access-v1"), the access-log record in section 7.
//! INVARIANTS:
//!   1. The log is a chain (chain.rs) with its own genesis: a record is
//!      acknowledged only once it is on disk, and a changed or dropped
//!      one is found when the log is read.
//!   2. A record holds what the call was, never what it carried: the
//!      arguments are a SHA-256 of their canonical JSON with what may be a
//!      TIN masked first (digest: the TIN shapes it knows), and every text a client gave (its
//!      name and version, a request id, the tool or resource it named)
//!      has any full TIN masked and is then cut to 64 characters.
//!   3. Records go in one at a time, in the order they are made.
//! ```

use crate::chain::{self, ChainError, ChainLog, Head};
use crate::journal::{canonical_json, sha256_hex};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::Path;
use std::sync::Mutex;

/// The access log, in a book's folder.
pub const ACCESS_FILE: &str = "access.jsonl";
/// Its genesis text.
pub const ACCESS_GENESIS: &str = "wanderbooks-access-v1";
/// The longest text a client asserts that a record keeps.
pub const MAX_ASSERTED: usize = 64;

/// Who made a call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Principal {
    /// The token's id; empty when there was no token, or none that
    /// matched.
    pub id: String,
    /// `agent`, or `unknown` when no token matched.
    pub kind: String,
    /// The token's name.
    pub label: String,
}

impl Principal {
    /// A caller with no token that matched.
    pub fn unknown() -> Self {
        Self {
            id: String::new(),
            kind: "unknown".into(),
            label: String::new(),
        }
    }
}

/// The client program, as it named itself (asserted, never checked).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Client {
    /// Its name.
    pub name: String,
    /// Its version.
    pub version: String,
}

/// What came of a call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// It ran and answered.
    Ok,
    /// It ran and answered with an error (a refusal of its own: an entry
    /// that doesn't balance, say).
    Error,
    /// It wasn't run, with the reason's code.
    Refused(String),
}

/// One access, as recorded (`seq`, `ts` and `prev` are the chain's).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Access {
    /// Who.
    pub principal: Principal,
    /// With which program, if it said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client: Option<Client>,
    /// The request's own id, if it had one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// `tool`, `resource`, or `connect` (a request refused before it
    /// reached the books).
    pub action: String,
    /// The tool's name, the resource's address, or what was refused.
    pub subject: String,
    /// The SHA-256 of the arguments' canonical JSON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args_digest: Option<String>,
    /// What came of it.
    pub result: Outcome,
    /// The journal's head after the call's last write, when it wrote
    /// (whatever it answered).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_after: Option<Head>,
}

/// Mask any full TIN in a client's text, then cut it to
/// [`MAX_ASSERTED`] characters (INVARIANT 2).
pub fn asserted(text: &str) -> String {
    // Masked first: a TIN cut in two at the limit would keep most of its
    // digits (review of slice 4).
    let masked = ledger_core::parties::redact_full_tins(text);
    masked.chars().take(MAX_ASSERTED).collect()
}

/// The digest of a call's arguments: the SHA-256 of their canonical JSON,
/// with every TIN masked first (INVARIANT 2), so that the digest can't be
/// matched against guesses of one. Without this a TIN whose last four the
/// books show is found among 10^5 guesses (review of slice 4). Masked:
/// any value under a key naming a TIN; any key or text holding one; any
/// number of eight digits or more (whatever its sign or fraction: a TIN
/// may start with a zero). No argument is a file's bytes (D-46: files
/// come by upload), so every text is scanned.
pub fn digest(args: &Value) -> String {
    sha256_hex(canonical_json(&masked(args)).as_bytes())
}

/// `v` with every TIN masked (see [`digest`]).
fn masked(v: &Value) -> Value {
    const MASK: &str = "[TIN]";
    match v {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| {
                    let k_lower = k.to_ascii_lowercase();
                    let names_a_tin = k_lower == "tin"
                        || k_lower.ends_with("_tin")
                        || k_lower == "ssn"
                        || k_lower == "ein"
                        || k_lower == "itin";
                    let v = if names_a_tin && !v.is_null() {
                        Value::from(MASK)
                    } else {
                        masked(v)
                    };
                    (ledger_core::parties::redact_full_tins(k).into_owned(), v)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(masked).collect()),
        Value::String(s) => Value::from(ledger_core::parties::redact_full_tins(s).into_owned()),
        Value::Number(n) if n.as_f64().is_none_or(|f| f.abs() >= 1e7) => Value::from(MASK),
        other => other.clone(),
    }
}

/// A book's access log, open for recording.
#[derive(Debug)]
pub struct AccessLog {
    log: Mutex<ChainLog>,
}

impl AccessLog {
    /// Open the access log in a book's folder (made if missing), checking
    /// it whole (INVARIANT 1).
    pub fn open(book_dir: &Path) -> Result<Self, ChainError> {
        let (log, records) = ChainLog::open(&book_dir.join(ACCESS_FILE), ACCESS_GENESIS)?;
        // Every record an access, as read would take it: a log that reads
        // only as a chain isn't whole.
        for (i, record) in records.into_iter().enumerate() {
            as_recorded(i, record)?;
        }
        Ok(Self {
            log: Mutex::new(log),
        })
    }

    /// Record one access; Ok only once it is on disk (INVARIANTS 1, 3).
    pub fn record(&self, access: &Access) -> Result<Head, ChainError> {
        let mut access = access.clone();
        access.principal.label = asserted(&access.principal.label);
        if let Some(c) = &mut access.client {
            c.name = asserted(&c.name);
            c.version = asserted(&c.version);
        }
        access.request_id = access.request_id.as_deref().map(asserted);
        access.subject = asserted(&access.subject);
        let Value::Object(fields) = serde_json::to_value(&access).expect("a record serializes")
        else {
            unreachable!("a record is an object")
        };
        let mut log = self.log.lock().unwrap_or_else(|e| e.into_inner());
        log.append(fields).map(|(_, head)| head)
    }

    /// Where the log ends now.
    pub fn head(&self) -> Head {
        self.log
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .head()
            .clone()
    }
}

/// One record as read back: the access and the chain's fields.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recorded {
    /// Its number.
    pub seq: u64,
    /// When (UTC, RFC 3339).
    pub ts: String,
    /// The access.
    #[serde(flatten)]
    pub access: Access,
}

/// Read a book's access log whole, checking it (INVARIANT 1). An
/// unfinished last line (a record being written, or cut short by a
/// crash) is left out.
pub fn read(book_dir: &Path) -> Result<(Vec<Recorded>, Head), ChainError> {
    let read = chain::read(&book_dir.join(ACCESS_FILE), ACCESS_GENESIS)?;
    let mut out = Vec::with_capacity(read.records.len());
    for (i, record) in read.records.into_iter().enumerate() {
        out.push(as_recorded(i, record)?);
    }
    Ok((out, read.head))
}

/// The `i`th record (from 0) read as an access.
fn as_recorded(i: usize, mut record: Map<String, Value>) -> Result<Recorded, ChainError> {
    record.remove("prev");
    record.remove("hash");
    serde_json::from_value(Value::Object(record)).map_err(|e| ChainError::Damaged {
        line: i as u64 + 1,
        why: format!("it isn't an access record ({e})"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn access(n: u64) -> Access {
        Access {
            principal: Principal {
                id: "tok_abc".into(),
                kind: "agent".into(),
                label: "claude-code".into(),
            },
            client: Some(Client {
                name: "claude-code".into(),
                version: "2.1".into(),
            }),
            request_id: Some(n.to_string()),
            action: "tool".into(),
            subject: "books.trial_balance".into(),
            args_digest: Some(digest(&serde_json::json!({"as_of": "2026-10-19"}))),
            result: Outcome::Ok,
            head_after: None,
        }
    }

    #[test]
    fn records_read_back_as_written() {
        let dir = tempfile::tempdir().unwrap();
        let log = AccessLog::open(dir.path()).unwrap();
        let mut refused = access(2);
        refused.result = Outcome::Refused("insufficient_scope".into());
        refused.principal = Principal::unknown();
        let mut posted = access(3);
        posted.subject = "books.post_entry".into();
        posted.head_after = Some(Head {
            seq: 41,
            hash: "ab".repeat(32),
        });
        let mut errored = access(4);
        errored.result = Outcome::Error;
        errored.client = None;
        errored.request_id = None;
        errored.args_digest = None;
        let all = [access(1), refused, posted, errored];
        for (i, a) in all.iter().enumerate() {
            assert_eq!(log.record(a).unwrap().seq, i as u64 + 1);
        }
        assert_eq!(log.head().seq, 4);
        let (back, head) = read(dir.path()).unwrap();
        assert_eq!(head, log.head());
        assert_eq!(back.len(), 4);
        for (i, r) in back.iter().enumerate() {
            assert_eq!(r.seq, i as u64 + 1);
            assert_eq!(r.access, all[i]);
        }
        // The wire form the plan names.
        let text = std::fs::read_to_string(dir.path().join(ACCESS_FILE)).unwrap();
        let second: Value = serde_json::from_str(text.lines().nth(1).unwrap()).unwrap();
        assert_eq!(
            second["result"],
            serde_json::json!({"refused": "insufficient_scope"})
        );
        let first: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(first["result"], "ok");
        assert_eq!(first["principal"]["kind"], "agent");
    }

    #[test]
    fn what_a_client_asserts_is_cut_and_masked() {
        let dir = tempfile::tempdir().unwrap();
        let log = AccessLog::open(dir.path()).unwrap();
        let mut a = access(1);
        a.client = Some(Client {
            name: "x".repeat(500),
            version: "123-45-6789".into(),
        });
        a.request_id = Some("ssn 123-45-6789".into());
        a.principal.label = "y".repeat(100);
        a.subject = "books.123-45-6789".into();
        log.record(&a).unwrap();
        let text = std::fs::read_to_string(dir.path().join(ACCESS_FILE)).unwrap();
        assert!(!text.contains("123-45-6789"), "{text}");
        assert!(!text.contains(&"x".repeat(MAX_ASSERTED + 1)));
        let (back, _) = read(dir.path()).unwrap();
        let client = back[0].access.client.as_ref().unwrap();
        assert_eq!(client.name.chars().count(), MAX_ASSERTED);
        assert_eq!(back[0].access.principal.label.len(), MAX_ASSERTED);
        assert!(!ledger_core::parties::has_full_tin(&client.version));
    }

    #[test]
    fn a_digest_gives_no_tin_away() {
        // The same call with another TIN: the same digest, so no guess
        // of the TIN can be checked against it.
        for (a, b) in [
            (
                serde_json::json!({"party": 4, "tin": "123-45-6789", "tin_type": "ssn"}),
                serde_json::json!({"party": 4, "tin": "987-81-4321", "tin_type": "ssn"}),
            ),
            (
                serde_json::json!({"party": 4, "tin": 123456789}),
                serde_json::json!({"party": 4, "tin": 987814321}),
            ),
            (
                serde_json::json!({"w9": {"payee_tin": "12-3456789"}}),
                serde_json::json!({"w9": {"payee_tin": "98-7814321"}}),
            ),
            (
                serde_json::json!({"note": "SSN 123-45-6789 on file"}),
                serde_json::json!({"note": "SSN 987-81-4321 on file"}),
            ),
            (
                serde_json::json!({"memo": [123456789]}),
                serde_json::json!({"memo": [987814321]}),
            ),
            // Round 2: as a key, as a float, negative, with a leading zero
            // lost, or run into another digit.
            (
                serde_json::json!({"structured": {"123-45-6789": "6100"}}),
                serde_json::json!({"structured": {"987-81-4321": "6100"}}),
            ),
            (
                serde_json::json!({"x": 123456789.0}),
                serde_json::json!({"x": 987814321.0}),
            ),
            (
                serde_json::json!({"x": -123456789}),
                serde_json::json!({"x": -987814321}),
            ),
            (
                serde_json::json!({"x": 12345678}),
                serde_json::json!({"x": 98765432}),
            ),
            (
                serde_json::json!({"x": 1123456789}),
                serde_json::json!({"x": 1987814321}),
            ),
        ] {
            assert_eq!(digest(&a), digest(&b), "{a}");
        }
        // Everything else still counts.
        assert_ne!(
            digest(&serde_json::json!({"party": 4, "tin": "123-45-6789"})),
            digest(&serde_json::json!({"party": 5, "tin": "123-45-6789"}))
        );
        assert_ne!(
            digest(&serde_json::json!({"amount": "12.34"})),
            digest(&serde_json::json!({"amount": "12.35"}))
        );
        assert_ne!(
            digest(&serde_json::json!({"entry_id": 1234567})),
            digest(&serde_json::json!({"entry_id": 1234568}))
        );
        // Text with no TIN in it counts as it is.
        assert_ne!(
            digest(&serde_json::json!({"upload_id": "up_MTIzNDU2Nzg5"})),
            digest(&serde_json::json!({"upload_id": "up_OTg3NjU0MzIx"}))
        );
    }

    #[test]
    fn a_value_under_a_key_naming_a_tin_is_masked_whatever_it_looks_like() {
        // Even a value that doesn't look like a TIN (a last four, a typo):
        // the key says what it is.
        for key in [
            "tin",
            "TIN",
            "payee_tin",
            "Member_TIN",
            "ssn",
            "ein",
            "itin",
        ] {
            assert_eq!(
                digest(&serde_json::json!({ key: "6789" })),
                digest(&serde_json::json!({ key: "1234" })),
                "{key}"
            );
            assert_eq!(
                digest(&serde_json::json!({ key: {"last4": 6789} })),
                digest(&serde_json::json!({ key: ["anything"] })),
                "{key}"
            );
            // A TIN not given isn't one: null stays null.
            assert_ne!(
                digest(&serde_json::json!({ key: null })),
                digest(&serde_json::json!({ key: "6789" })),
                "{key}"
            );
        }
        // Only those keys: one that merely holds the letters counts.
        assert_ne!(
            digest(&serde_json::json!({"tint": "6789"})),
            digest(&serde_json::json!({"tint": "1234"}))
        );
    }

    #[test]
    fn digits_inside_any_text_are_masked() {
        // No argument is a file's bytes any more (D-46): a run of nine
        // digits is masked wherever it is, under any key.
        for key in ["memo", "description", "upload_id"] {
            assert_eq!(
                digest(&serde_json::json!({ key: "QUJD123456789QUJD" })),
                digest(&serde_json::json!({ key: "QUJD987814321QUJD" })),
                "{key}"
            );
        }
    }

    #[test]
    fn a_tin_at_the_cut_is_masked_whole() {
        // The TIN straddles the 64-character limit.
        let text = format!("{}123-45-6789", "x".repeat(58));
        let kept = asserted(&text);
        assert!(!kept.contains("123-45-67"), "{kept}");
        assert!(kept.chars().count() <= MAX_ASSERTED);
    }

    #[test]
    fn the_digest_is_of_canonical_arguments() {
        let a = serde_json::json!({"b": 1, "a": [2, {"d": 3, "c": 4}]});
        let b: Value = serde_json::from_str(r#"{"a":[2,{"c":4,"d":3}],"b":1}"#).unwrap();
        assert_eq!(digest(&a), digest(&b));
        assert_ne!(digest(&a), digest(&serde_json::json!({"b": 2})));
        assert_eq!(digest(&a).len(), 64);
    }

    #[test]
    fn another_chains_records_dont_pass() {
        let dir = tempfile::tempdir().unwrap();
        let (mut journal_like, _) =
            ChainLog::open(&dir.path().join(ACCESS_FILE), "wanderbooks-principals-v1").unwrap();
        let Value::Object(fields) = serde_json::to_value(access(1)).unwrap() else {
            unreachable!()
        };
        journal_like.append(fields).unwrap();
        assert!(read(dir.path()).is_err());
        assert!(AccessLog::open(dir.path()).is_err());
    }

    #[test]
    fn a_record_that_isnt_an_access_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (mut log, _) = ChainLog::open(&dir.path().join(ACCESS_FILE), ACCESS_GENESIS).unwrap();
        let mut m = Map::new();
        m.insert("what".into(), Value::from("nothing"));
        log.append(m).unwrap();
        assert!(matches!(
            read(dir.path()).unwrap_err(),
            ChainError::Damaged { line: 1, .. }
        ));
        // Nor is such a log opened (its book stays closed).
        assert!(matches!(
            AccessLog::open(dir.path()).unwrap_err(),
            ChainError::Damaged { line: 1, .. }
        ));
    }

    #[test]
    fn records_from_many_threads_all_land_in_one_chain() {
        let dir = tempfile::tempdir().unwrap();
        let log = std::sync::Arc::new(AccessLog::open(dir.path()).unwrap());
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let log = log.clone();
                std::thread::spawn(move || {
                    for n in 0..25 {
                        log.record(&access(t * 100 + n)).unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        let (back, head) = read(dir.path()).unwrap();
        assert_eq!(back.len(), 200);
        assert_eq!(head.seq, 200);
        let mut ids: Vec<String> = back
            .iter()
            .map(|r| r.access.request_id.clone().unwrap())
            .collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 200);
    }
}
