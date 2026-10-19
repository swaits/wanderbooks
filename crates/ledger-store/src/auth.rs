//! Who may reach the books through the server: the root's principals
//! (plan 3.3, 3.4), kept in `auth/principals.jsonl`.
//!
//! ```text
//! GOAL: an agent reaches a book through the server only with a token
//!       minted for that book, and only for what its scopes allow; a
//!       revoked token stops working at once.
//! SOURCE: plan 3.3 ("a static bearer wb_ + 32 random bytes, stored only
//!         as a sha256, compared in constant time ... revocable"), 3.4
//!         (auth/principals.jsonl, hash-chained), 3.5.
//! INVARIANTS:
//!   1. A token's text is never written: the log holds its SHA-256. Its
//!      text leaves create once, to the caller, and nothing else returns
//!      it.
//!   2. The live tokens are exactly the log replayed: every create adds
//!      one (and ends the one it replaces), every revoke ends one. The
//!      log is a chain (chain.rs), so a changed record is found when it
//!      is read; one this version can't read is refused whole.
//!   3. Per book, a name belongs to at most one live token.
//!   4. check compares the presented token's SHA-256 with every live
//!      token's, in constant time, with no early exit, and answers the
//!      token it matched.
//!   5. Nothing here names a path.
//! ```

use crate::chain::{ChainError, ChainLog};
use crate::root::BookId;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::Path;

/// The principals' log, in the root's `auth/`.
pub const PRINCIPALS_FILE: &str = "principals.jsonl";
/// Its genesis text.
pub const PRINCIPALS_GENESIS: &str = "wanderbooks-principals-v1";
/// How every token begins.
pub const TOKEN_PREFIX: &str = "wb_";

/// What a token may do (plan 3.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Scope {
    /// Reads, reports, `start`, the handbook, receipts.
    #[serde(rename = "books:read")]
    Read,
    /// Drafts, lines, attachments, notes: what changes nothing posted.
    #[serde(rename = "books:write")]
    Write,
    /// Posting, closing, reconciling, disposing, running schedules,
    /// committing settings.
    #[serde(rename = "books:post")]
    Post,
    /// The access log (later: backups, invitations).
    #[serde(rename = "books:admin")]
    Admin,
}

impl Scope {
    /// Every scope, in order.
    pub const ALL: [Scope; 4] = [Scope::Read, Scope::Write, Scope::Post, Scope::Admin];

    /// Its name, `books:read` and so on.
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Read => "books:read",
            Scope::Write => "books:write",
            Scope::Post => "books:post",
            Scope::Admin => "books:admin",
        }
    }

    /// A scope by its name, with or without `books:`.
    pub fn parse(text: &str) -> Option<Scope> {
        let short = text.strip_prefix("books:").unwrap_or(text);
        Scope::ALL
            .into_iter()
            .find(|s| s.as_str().strip_prefix("books:") == Some(short))
    }
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A live token, as the log records it (never its text: INVARIANT 1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    /// Its id, `tok_` and 12 hex digits.
    pub id: String,
    /// The book it opens.
    pub book: String,
    /// Its name, unique among the book's live tokens.
    pub name: String,
    /// What it may do.
    pub scopes: BTreeSet<Scope>,
    /// When it was made (UTC, RFC 3339).
    pub created: String,
    /// The hex SHA-256 of its text.
    #[serde(skip_serializing)]
    pub sha256: String,
}

/// A token's text, which is shown once (INVARIANT 1). Debug hides it.
pub struct Secret(String);

impl Secret {
    /// The text itself.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(…)")
    }
}

/// Why the principals can't do what was asked. Never names a path.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    /// The log can't be read or written.
    #[error("the record of agent tokens can't be used: {0}")]
    Log(ChainError),
    /// A record this version doesn't know (a newer wanderbooks wrote it).
    #[error("the record of agent tokens was written by a newer wanderbooks (record {0})")]
    Newer(u64),
    /// A record that contradicts the ones before it.
    #[error("the record of agent tokens is inconsistent at record {0}: {1}")]
    Inconsistent(u64, String),
    /// A name that can't name a token.
    #[error(
        "{0:?} can't name a token: use 1 to 32 lowercase letters, digits and hyphens, starting with a letter or digit"
    )]
    BadName(String),
    /// The book already has a live token by that name.
    #[error(
        "the books {book} already have a token called {name:?}: replace it, or revoke it first"
    )]
    Exists {
        /// The book.
        book: String,
        /// The name.
        name: String,
    },
    /// No live token by that name.
    #[error("the books {book} have no token called {name:?}")]
    NoSuch {
        /// The book.
        book: String,
        /// The name.
        name: String,
    },
    /// A token with no scopes would open nothing.
    #[error("a token needs at least one scope")]
    NoScopes,
    /// The system gave no randomness.
    #[error("the system gave no randomness for a token")]
    NoRandom,
}

impl From<ChainError> for AuthError {
    fn from(e: ChainError) -> Self {
        AuthError::Log(e)
    }
}

/// Check a token's name: 1 to 32 lowercase letters, digits and hyphens,
/// starting with a letter or digit.
pub fn check_name(name: &str) -> Result<(), AuthError> {
    let ok = (1..=32).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-');
    if ok {
        Ok(())
    } else {
        Err(AuthError::BadName(name.chars().take(64).collect()))
    }
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn random_hex(n: usize) -> Result<String, AuthError> {
    let mut buf = vec![0u8; n];
    getrandom::getrandom(&mut buf).map_err(|_| AuthError::NoRandom)?;
    Ok(hex::encode(buf))
}

/// The root's principals: the log and the live tokens it replays to.
#[derive(Debug)]
pub struct Principals {
    log: ChainLog,
    live: Vec<Token>,
    /// Every token id ever made, live or not: a new one is never one of
    /// them (the log refuses an id made twice).
    ids: BTreeSet<String>,
}

impl Principals {
    /// Open the principals in an `auth/` folder (made empty if missing),
    /// replaying the log (INVARIANT 2).
    pub fn open(auth_dir: &Path) -> Result<Self, AuthError> {
        let (log, records) = ChainLog::open(&auth_dir.join(PRINCIPALS_FILE), PRINCIPALS_GENESIS)?;
        let mut live: Vec<Token> = Vec::new();
        let mut seen = BTreeSet::new();
        for record in records {
            let seq = record.get("seq").and_then(Value::as_u64).unwrap_or(0);
            let bad = |why: &str| AuthError::Inconsistent(seq, why.to_string());
            let event = record.get("event").and_then(Value::as_str);
            let text = |key: &str| -> Result<String, AuthError> {
                record
                    .get(key)
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| bad(&format!("no {key}")))
            };
            match event {
                Some("token.created") => {
                    let id = text("id")?;
                    if !seen.insert(id.clone()) {
                        return Err(bad("a token id made twice"));
                    }
                    let scopes: BTreeSet<Scope> = record
                        .get("scopes")
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                        .ok_or_else(|| bad("its scopes can't be read"))?;
                    if scopes.is_empty() {
                        return Err(bad("a token with no scopes"));
                    }
                    let token = Token {
                        id,
                        book: text("book")?,
                        name: text("name")?,
                        scopes,
                        created: text("ts")?,
                        sha256: text("sha256")?,
                    };
                    if let Some(old) = record.get("replaces") {
                        let old = old.as_str().ok_or_else(|| bad("replaces no token"))?;
                        let at = live
                            .iter()
                            .position(|t| {
                                t.id == old && t.book == token.book && t.name == token.name
                            })
                            .ok_or_else(|| bad("it replaces a token that isn't live"))?;
                        live.remove(at);
                    }
                    if live
                        .iter()
                        .any(|t| t.book == token.book && t.name == token.name)
                    {
                        return Err(bad("a name given twice"));
                    }
                    live.push(token);
                }
                Some("token.revoked") => {
                    let id = text("id")?;
                    let at = live
                        .iter()
                        .position(|t| t.id == id)
                        .ok_or_else(|| bad("it revokes a token that isn't live"))?;
                    live.remove(at);
                }
                _ => return Err(AuthError::Newer(seq)),
            }
        }
        Ok(Self {
            log,
            live,
            ids: seen,
        })
    }

    /// The live tokens, oldest first.
    pub fn tokens(&self) -> &[Token] {
        &self.live
    }

    /// Make a token for `book` (INVARIANTS 1, 3). With `replace`, a live
    /// token of the same name ends in the same record; without it, one
    /// is refused.
    pub fn create(
        &mut self,
        book: &BookId,
        name: &str,
        scopes: BTreeSet<Scope>,
        replace: bool,
    ) -> Result<(Token, Secret), AuthError> {
        check_name(name)?;
        if scopes.is_empty() {
            return Err(AuthError::NoScopes);
        }
        let old = self
            .live
            .iter()
            .position(|t| t.book == book.as_str() && t.name == name);
        if old.is_some() && !replace {
            return Err(AuthError::Exists {
                book: book.to_string(),
                name: name.to_string(),
            });
        }
        let text = format!("{TOKEN_PREFIX}{}", random_hex(32)?);
        let id = loop {
            let id = format!("tok_{}", random_hex(6)?);
            if !self.ids.contains(&id) {
                break id;
            }
        };
        let sha = hex::encode(sha256(text.as_bytes()));
        let mut fields = Map::new();
        fields.insert("event".into(), Value::from("token.created"));
        fields.insert("id".into(), Value::from(id.clone()));
        fields.insert("book".into(), Value::from(book.as_str()));
        fields.insert("name".into(), Value::from(name));
        fields.insert(
            "scopes".into(),
            serde_json::to_value(&scopes).expect("scopes serialize"),
        );
        fields.insert("sha256".into(), Value::from(sha.clone()));
        if let Some(at) = old {
            fields.insert("replaces".into(), Value::from(self.live[at].id.clone()));
        }
        let (record, _) = self.log.append(fields)?;
        if let Some(at) = old {
            self.live.remove(at);
        }
        let token = Token {
            id,
            book: book.to_string(),
            name: name.to_string(),
            scopes,
            created: record["ts"].as_str().unwrap_or_default().to_string(),
            sha256: sha,
        };
        self.ids.insert(token.id.clone());
        self.live.push(token.clone());
        Ok((token, Secret(text)))
    }

    /// End `book`'s token called `name`.
    pub fn revoke(&mut self, book: &BookId, name: &str) -> Result<Token, AuthError> {
        self.revoke_if(book, name, None)
    }

    /// End `book`'s token called `name`, only if its id is `id` when one
    /// is given (taking back a token just made, which another may have
    /// replaced meanwhile).
    pub fn revoke_if(
        &mut self,
        book: &BookId,
        name: &str,
        id: Option<&str>,
    ) -> Result<Token, AuthError> {
        let at = self
            .live
            .iter()
            .position(|t| {
                t.book == book.as_str() && t.name == name && id.is_none_or(|id| t.id == id)
            })
            .ok_or_else(|| AuthError::NoSuch {
                book: book.to_string(),
                name: name.chars().take(64).collect(),
            })?;
        let mut fields = Map::new();
        fields.insert("event".into(), Value::from("token.revoked"));
        fields.insert("id".into(), Value::from(self.live[at].id.clone()));
        self.log.append(fields)?;
        Ok(self.live.remove(at))
    }

    /// The live token whose text this is, if any (INVARIANT 4).
    pub fn check(&self, presented: &str) -> Option<&Token> {
        use subtle::ConstantTimeEq;
        let presented = sha256(presented.as_bytes());
        let mut found = None;
        for token in &self.live {
            let Ok(sha) = hex::decode(&token.sha256) else {
                continue;
            };
            if bool::from(sha.as_slice().ct_eq(&presented)) {
                found = Some(token);
            }
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(id: &str) -> BookId {
        BookId::parse(id).unwrap()
    }

    fn all() -> BTreeSet<Scope> {
        Scope::ALL.into_iter().collect()
    }

    #[test]
    fn scopes_read_by_name() {
        for s in Scope::ALL {
            assert_eq!(Scope::parse(s.as_str()), Some(s));
            assert_eq!(Scope::parse(&s.as_str()[6..]), Some(s));
            assert_eq!(
                serde_json::to_value(s).unwrap(),
                Value::from(s.as_str()),
                "the log spells it as the plan does"
            );
        }
        assert_eq!(Scope::parse("books:view"), None);
        assert_eq!(Scope::parse(""), None);
        assert_eq!(Scope::parse("books:"), None);
    }

    #[test]
    fn names_are_checked() {
        for good in ["agent", "claude-code", "a", "0x", &"a".repeat(32)] {
            assert!(check_name(good).is_ok(), "{good}");
        }
        for bad in ["", "-a", "Agent", "a b", "a/b", "a.b", &"a".repeat(33), "é"] {
            assert!(check_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_token_is_checked_until_revoked_and_its_text_is_never_kept() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = Principals::open(dir.path()).unwrap();
        let (token, secret) = p.create(&book("acme"), "agent", all(), false).unwrap();
        let text = secret.expose().to_string();
        assert!(text.starts_with(TOKEN_PREFIX));
        assert_eq!(text.len(), 3 + 64);
        assert!(token.id.starts_with("tok_"));
        assert_eq!(p.check(&text), Some(&token));
        assert_eq!(p.check(&text[..text.len() - 1]), None);
        assert_eq!(p.check(""), None);
        assert_eq!(format!("{secret:?}"), "Secret(…)");
        let log = std::fs::read_to_string(dir.path().join(PRINCIPALS_FILE)).unwrap();
        assert!(
            !log.contains(&text[3..]),
            "the text itself is never written"
        );
        assert!(log.contains(&token.sha256));
        // Another book's token is another token.
        let (other, other_secret) = p.create(&book("beta"), "agent", all(), false).unwrap();
        assert_ne!(other.id, token.id);
        assert_eq!(p.check(other_secret.expose()).unwrap().book, "beta");
        let revoked = p.revoke(&book("acme"), "agent").unwrap();
        assert_eq!(revoked.id, token.id);
        assert_eq!(p.check(&text), None);
        assert!(p.check(other_secret.expose()).is_some());
        assert!(matches!(
            p.revoke(&book("acme"), "agent"),
            Err(AuthError::NoSuch { .. })
        ));
        // Taken back only if it is still the one named.
        assert!(matches!(
            p.revoke_if(&book("beta"), "agent", Some("tok_000000000000")),
            Err(AuthError::NoSuch { .. })
        ));
        assert!(p.revoke_if(&book("beta"), "agent", Some(&other.id)).is_ok());
        let (other, other_secret) = p.create(&book("beta"), "agent", all(), false).unwrap();
        // Reopened, the log replays to the same live tokens.
        drop(p);
        let p = Principals::open(dir.path()).unwrap();
        assert_eq!(p.tokens().len(), 1);
        assert_eq!(p.check(&text), None);
        assert_eq!(p.check(other_secret.expose()).unwrap().id, other.id);
    }

    #[test]
    fn a_name_is_taken_until_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = Principals::open(dir.path()).unwrap();
        let (first, first_secret) = p.create(&book("acme"), "agent", all(), false).unwrap();
        assert!(matches!(
            p.create(&book("acme"), "agent", all(), false),
            Err(AuthError::Exists { .. })
        ));
        let read_only: BTreeSet<Scope> = [Scope::Read].into();
        let (second, second_secret) = p
            .create(&book("acme"), "agent", read_only.clone(), true)
            .unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(p.check(first_secret.expose()), None, "the old one ended");
        assert_eq!(p.check(second_secret.expose()).unwrap().scopes, read_only);
        assert_eq!(p.tokens().len(), 1);
        drop(p);
        let p = Principals::open(dir.path()).unwrap();
        assert_eq!(p.tokens(), &[second]);
        // A replace with nothing to replace is a create.
        let mut p = p;
        p.create(&book("acme"), "other", all(), true).unwrap();
        assert_eq!(p.tokens().len(), 2);
    }

    #[test]
    fn bad_requests_write_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = Principals::open(dir.path()).unwrap();
        assert!(matches!(
            p.create(&book("acme"), "Bad Name", all(), false),
            Err(AuthError::BadName(_))
        ));
        assert_eq!(
            p.create(&book("acme"), "agent", BTreeSet::new(), false)
                .unwrap_err(),
            AuthError::NoScopes
        );
        let len = std::fs::metadata(dir.path().join(PRINCIPALS_FILE))
            .unwrap()
            .len();
        assert_eq!(len, 0);
    }

    /// Append a raw record to a fresh log, then open it.
    fn open_with(records: &[Value]) -> Result<Principals, AuthError> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(PRINCIPALS_FILE);
        let (mut log, _) = ChainLog::open(&path, PRINCIPALS_GENESIS).unwrap();
        for r in records {
            log.append(r.as_object().unwrap().clone()).unwrap();
        }
        drop(log);
        Principals::open(dir.path())
    }

    fn created(id: &str, name: &str) -> Value {
        serde_json::json!({"event": "token.created", "id": id, "book": "acme",
            "name": name, "scopes": ["books:read"], "sha256": "00"})
    }

    #[test]
    fn a_log_that_contradicts_itself_is_refused() {
        assert!(open_with(&[created("tok_1", "a")]).is_ok());
        let cases = [
            vec![created("tok_1", "a"), created("tok_1", "b")],
            vec![created("tok_1", "a"), created("tok_2", "a")],
            vec![serde_json::json!({"event": "token.revoked", "id": "tok_9"})],
            vec![{
                let mut r = created("tok_1", "a");
                r["replaces"] = "tok_0".into();
                r
            }],
            vec![{
                let mut r = created("tok_1", "a");
                r["scopes"] = serde_json::json!([]);
                r
            }],
            vec![{
                let mut r = created("tok_1", "a");
                r["scopes"] = serde_json::json!(["books:everything"]);
                r
            }],
            vec![{
                let mut r = created("tok_1", "a");
                r.as_object_mut().unwrap().remove("book");
                r
            }],
        ];
        for records in cases {
            let e = open_with(&records).unwrap_err();
            assert!(matches!(e, AuthError::Inconsistent(..)), "{records:?}: {e}");
        }
        let e = open_with(&[serde_json::json!({"event": "token.renamed"})]).unwrap_err();
        assert_eq!(e, AuthError::Newer(1));
    }

    #[test]
    fn a_changed_record_is_found() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = Principals::open(dir.path()).unwrap();
        p.create(&book("acme"), "agent", all(), false).unwrap();
        p.create(&book("acme"), "other", all(), false).unwrap();
        drop(p);
        let path = dir.path().join(PRINCIPALS_FILE);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text.replacen("books:admin", "books:read", 1)).unwrap();
        assert!(matches!(
            Principals::open(dir.path()).unwrap_err(),
            AuthError::Log(ChainError::Damaged { line: 1, .. })
        ));
        // The last record too.
        let lines: Vec<&str> = text.lines().collect();
        std::fs::write(
            &path,
            format!(
                "{}
{}
",
                lines[0],
                lines[1].replacen("books:admin", "books:read", 1)
            ),
        )
        .unwrap();
        assert!(matches!(
            Principals::open(dir.path()).unwrap_err(),
            AuthError::Log(ChainError::Damaged { line: 2, .. })
        ));
    }

    #[test]
    fn errors_name_no_path() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(PRINCIPALS_FILE), "x\n").unwrap();
        let e = Principals::open(dir.path()).unwrap_err().to_string();
        assert!(!e.contains(&dir.path().display().to_string()), "{e}");
    }
}
