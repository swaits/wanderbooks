//! The admin socket's messages (`run/admin.sock`, plan 3.1, 3.3, 3.9): how
//! the command line asks the books server to list, create, adopt and
//! restore books, to make, list and revoke agents' tokens, and how the
//! host hands it the owner's go-ahead for a format upgrade.
//!
//! ```text
//! GOAL: the command line makes and adopts books without ever touching
//!       the data root itself: the server, its sole writer, does it.
//! SOURCE: plan/served.md 3.1 (the CLI "talks to run/admin.sock ... it can
//!         create or adopt books"), 5.5 (adoption), 3.9 (protocol versions
//!         N and N-1).
//! INVARIANTS:
//!   1. One request per connection, one JSON line each way, at most
//!      MAX_LINE bytes; anything else is refused with a plain reason.
//!   2. A request carries its protocol version; the server accepts this
//!      version and the one before, and refuses others with a message
//!      that says to restart.
//!   3. No answer names the data root or a path in it, nor echoes a path
//!      the caller gave (which could be the root under another name).
//!   4. Adopting and restoring are two steps: without `confirmed` the
//!      answer is a preview, and nothing is written; `confirmed: true`
//!      adopts (restores), and only books (a backup) still at the head the
//!      preview read (`expect_head`).
//!   5. Nothing in a settings file is dropped silently: entries the books
//!      don't know are refused unless `skip_unknown`, and its notes and
//!      comments are in the preview.
//!   6. A token's text is in exactly one answer, TokenCreated, to the
//!      request that made it; nothing else carries it (auth.rs
//!      INVARIANT 1): a check (TokenCheck) asks by the text's SHA-256.
//!   7. A format upgrade is asked with the owner passphrase, which the
//!      server checks before anything is done (plan 3.9, D-39); no answer
//!      carries it, and Debug never shows it.
//! ```

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// This protocol's version (INVARIANT 2).
pub const PROTO: u32 = 1;

/// The oldest version this side still accepts (INVARIANT 2: this one and
/// the one before; there is none before the first).
pub const OLDEST: u32 = 1;

/// Whether a request of protocol version `proto` is accepted.
pub fn accepts(proto: u32) -> bool {
    (OLDEST..=PROTO).contains(&proto)
}

/// The longest line either side sends, its newline not counted
/// (INVARIANT 1): settings texts ride in an adopt request.
pub const MAX_LINE: usize = 4 * 1024 * 1024;

/// A request, with its protocol version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    /// The sender's protocol version.
    pub proto: u32,
    /// What is asked.
    #[serde(flatten)]
    pub request: Request,
}

/// What the command line can ask.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    /// The books the server holds.
    List,
    /// Start new, empty books.
    Create {
        /// The new books' id.
        id: String,
        /// Their base currency (ISO 4217), fixed for good.
        currency: String,
    },
    /// Adopt existing books (plan 5.5).
    Adopt {
        /// The new books' id.
        id: String,
        /// The data dir to adopt (`journal.jsonl`, `blobs/`).
        source: PathBuf,
        /// The settings file's text, for books that keep no settings.
        #[serde(default)]
        settings_toml: Option<String>,
        /// `config.json`'s text, for its receipt policy.
        #[serde(default)]
        config_json: Option<String>,
        /// The source's TIN vault folder.
        #[serde(default)]
        vault_dir: Option<PathBuf>,
        /// The v1 vault key file.
        #[serde(default)]
        vault_key: Option<PathBuf>,
        /// Adopt (true), or only preview what would be moved in.
        #[serde(default)]
        confirmed: bool,
        /// The head the preview read: adopted only if still so.
        #[serde(default)]
        expect_head: Option<(u64, String)>,
        /// Leave out settings-file entries the books don't know.
        #[serde(default)]
        skip_unknown: bool,
    },
    /// Restore a backup (plan 3.7): as new books, or replacing books
    /// (which are moved into `legacy/`, kept).
    Restore {
        /// The books' id.
        id: String,
        /// The backup: a git repository's address, or its folder
        /// (`<book>.git` in the backup folder).
        source: String,
        /// Replace the books called `id`.
        #[serde(default)]
        replace: bool,
        /// Restore (true), or only preview it.
        #[serde(default)]
        confirmed: bool,
        /// The head the preview read: restored only if still so.
        #[serde(default)]
        expect_head: Option<(u64, String)>,
    },
    /// Back books up now (plan 3.7): to every target their settings name.
    BackUpNow {
        /// The books.
        id: String,
        /// The backup folder's disk is new: begin these books' backup
        /// there again.
        #[serde(default)]
        new_disk: bool,
    },
    /// Make a token for an agent (plan 3.3).
    TokenCreate {
        /// The books it opens.
        book: String,
        /// Its name, unique among the books' tokens ("agent").
        name: String,
        /// What it may do: books:read, books:write, books:post,
        /// books:admin.
        scopes: Vec<String>,
        /// End a token of the same name in the same step.
        #[serde(default)]
        replace: bool,
    },
    /// The live tokens.
    TokenList,
    /// Whether a token kept elsewhere (this computer's keyring) is the
    /// live one of its books and name: asked by its SHA-256 alone, never
    /// its text (INVARIANT 6).
    TokenCheck {
        /// Its books.
        book: String,
        /// Its name.
        name: String,
        /// The hex SHA-256 of the text kept.
        sha256: String,
    },
    /// The owner's go-ahead for the format upgrade of books waiting for
    /// one (plan 3.9, D-39): a verified backup with the keys, then this
    /// version's journal format declared. The host sends it, for the
    /// lease socket's `consent_format_upgrade`.
    UpgradeFormat {
        /// The books.
        id: String,
        /// The owner passphrase, checked by the server (INVARIANT 7).
        passphrase: Passphrase,
    },
    /// End a token.
    TokenRevoke {
        /// Its books.
        book: String,
        /// Its name.
        name: String,
        /// Only if it is still this one (a token just made, taken back).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
}

/// A token's text in an answer: Debug never shows it (INVARIANT 6).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TokenText(pub String);

impl std::fmt::Debug for TokenText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenText(…)")
    }
}

/// The owner passphrase in a request (and in the lease socket's consent,
/// which the host hands on as this): Debug never shows it (INVARIANT 7),
/// and it is wiped from memory when dropped, as the command line wipes
/// what is typed.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Passphrase(pub String);

impl std::fmt::Debug for Passphrase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Passphrase(…)")
    }
}

impl Drop for Passphrase {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.0);
    }
}

/// A format upgrade books wait for (plan 3.9, D-39): written by an older
/// version, they open on their old format until the owner says go.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatWaiting {
    /// The journal format they are in.
    pub format: u32,
    /// The features they declared.
    pub features: Vec<String>,
    /// What the upgrade adds, in words ("journal format 2").
    pub adds: Vec<String>,
}

impl From<&ledger_store::FormatUpgrade> for FormatWaiting {
    fn from(u: &ledger_store::FormatUpgrade) -> Self {
        Self {
            format: u.from.format,
            features: u.from.features.clone(),
            adds: u.adds(),
        }
    }
}

/// What a format upgrade did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatUpgraded {
    /// The books.
    pub id: String,
    /// What it added, in words; empty when the books were already in this
    /// version's format (nothing was done).
    pub added: Vec<String>,
    /// The verified backup made first: the books as they were.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup: Option<UpgradeBackup>,
    /// The books' head now.
    pub head: (u64, String),
}

/// The backup a format upgrade made first, proven to hold the books as
/// they were, their vault and keys too.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpgradeBackup {
    /// Its commit in the books' own backup repository.
    pub commit: String,
    /// The head it holds.
    pub head: (u64, String),
}

/// A token, as listed (never its text).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenInfo {
    /// Its id.
    pub id: String,
    /// The books it opens.
    pub book: String,
    /// Its name.
    pub name: String,
    /// What it may do.
    pub scopes: Vec<String>,
    /// When it was made (UTC).
    pub created: String,
}

impl From<&ledger_store::auth::Token> for TokenInfo {
    fn from(t: &ledger_store::auth::Token) -> Self {
        Self {
            id: t.id.clone(),
            book: t.book.clone(),
            name: t.name.clone(),
            scopes: t.scopes.iter().map(|s| s.as_str().to_string()).collect(),
            created: t.created.clone(),
        }
    }
}

/// One book, as listed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookStatus {
    /// Its id.
    pub id: String,
    /// Whether it is open and served.
    pub open: bool,
    /// Why it couldn't be opened, if it couldn't.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    /// The format upgrade it waits for, if it does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_upgrade: Option<FormatWaiting>,
    /// The projection database's file in use (a name, no folder), if it
    /// opened (ledger_store::db_file_for).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    /// Other schemas' projection files beside it, left as they are for the
    /// versions that wrote them (never deleted).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub other_databases: Vec<String>,
}

/// A setting an adoption would move in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingChange {
    /// Its key.
    pub key: String,
    /// Its value, as JSON (`None`: the setting is cleared).
    pub value: Option<serde_json::Value>,
}

/// What an unconfirmed adoption would do (INVARIANT 4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdoptPreview {
    /// The source's last event, `(seq, hash)`, as read now.
    pub source_head: (u64, String),
    /// The source already keeps its own settings: none would be moved in.
    pub keeps_settings: bool,
    /// The settings that would be moved in.
    pub settings: Vec<SettingChange>,
    /// What the settings file says that isn't a books setting.
    pub notes: Vec<String>,
    /// Entries in it the books don't know (not moved in).
    pub unknown: Vec<String>,
    /// The settings file's comments, in order (they aren't settings: keep
    /// what matters as a convention or a decision).
    pub comments: Vec<String>,
    /// The vault's records, each checked to open with the key given.
    pub vault_records: usize,
}

/// What an adoption did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Adopted {
    /// The new books' id.
    pub id: String,
    /// The source's head, verified before copying.
    pub source_head: (u64, String),
    /// The adopted books' head.
    pub head: (u64, String),
    /// Stored files copied.
    pub blob_files: u64,
    /// Stored files the books name, re-hashed through the store.
    pub blobs_verified: u64,
    /// Stored files copied that the books don't name.
    pub unnamed_blobs: u64,
    /// Files left behind among the stored ones (unfinished writes, hidden
    /// files).
    pub skipped_files: u64,
    /// Vault records copied.
    pub vault_files: u64,
    /// Whether the v1 key was copied.
    pub key_copied: bool,
    /// Settings moved in.
    pub settings_imported: usize,
    /// Settings were given, but the books keep their own.
    pub settings_ignored: bool,
    /// The source had no lock file, so one was made beside its journal.
    pub source_lock_created: bool,
    /// The copy was moved to this version's journal format (the source's
    /// is older, and keeps its own).
    #[serde(default)]
    pub format_upgraded: bool,
    /// Why the adopted books didn't open, if they didn't (they are kept;
    /// closing the books and opening them again tries again).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// What a restore would bring in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestorePreview {
    /// The backup's head, proven.
    pub head: (u64, String),
    /// Stored files in it, each re-hashed.
    pub blobs: u64,
    /// TIN vault records in it.
    pub vault_files: u64,
    /// Whether books of that id exist now.
    pub exists: bool,
    /// The id of the books it holds, as they were backed up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

/// What a restore did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Restored {
    /// The books' id.
    pub id: String,
    /// Their head: the backup's.
    pub head: (u64, String),
    /// Stored files the books name, re-hashed through the store.
    pub blobs_verified: u64,
    /// TIN vault records restored.
    pub vault_files: u64,
    /// The `legacy/` folder the replaced books were moved to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaced: Option<String>,
    /// The id of the books it held, as they were backed up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// Why the restored books didn't open, if they didn't (they are kept).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// How one backup target took a backup now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackedUpTarget {
    /// "repository" or "folder".
    pub target: String,
    /// Where (no path in the data root).
    pub at: String,
    /// The journal seq it holds now, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub through_seq: Option<u64>,
    /// Why it holds no backup of the latest, if it doesn't.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// The server's answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Response {
    /// The books held.
    Books(Vec<BookStatus>),
    /// New books made.
    Created {
        /// Their id.
        id: String,
        /// Why they didn't open, if they didn't (they are kept).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        problem: Option<String>,
    },
    /// What an adoption would do.
    Preview(AdoptPreview),
    /// Books adopted and open.
    Adopted(Adopted),
    /// What a restore would bring in.
    RestorePreview(RestorePreview),
    /// A backup restored, and open.
    Restored(Restored),
    /// Books backed up now: their head, and each target's outcome (none
    /// named: none set up).
    BackedUp {
        /// The books' head seq.
        head_seq: u64,
        /// Each target.
        targets: Vec<BackedUpTarget>,
    },
    /// A token made: its text, shown this once (INVARIANT 6).
    TokenCreated {
        /// The token.
        token: TokenInfo,
        /// Its text.
        text: TokenText,
    },
    /// The live tokens.
    Tokens(Vec<TokenInfo>),
    /// A token ended.
    TokenRevoked(TokenInfo),
    /// What a token check found: the books' live token of that name, if
    /// any, and whether the text asked about is it.
    TokenChecked {
        /// The live token of that books and name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        live: Option<TokenInfo>,
        /// The text asked about is that live token's.
        matches: bool,
    },
    /// A format upgrade made (or none needed).
    FormatUpgraded(FormatUpgraded),
    /// Refused, with why (no path in the data root: INVARIANT 3).
    Refused(String),
}

/// Why an admin call has no answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallError {
    /// The request never reached the server: nothing was done.
    NotSent(String),
    /// It was sent, and no answer came: it may or may not have been done.
    Unknown(String),
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallError::NotSent(why) | CallError::Unknown(why) => f.write_str(why),
        }
    }
}

/// Ask the server on `socket` (the root's `run/admin.sock`), waiting as
/// long as it takes (a create or an adoption).
#[cfg(unix)]
pub fn call(socket: &std::path::Path, request: Request) -> Result<Response, CallError> {
    call_within(socket, request, None)
}

/// [`call`], waiting at most `wait` for the answer (a quick look, such
/// as `doctor`'s).
#[cfg(unix)]
pub fn call_within(
    socket: &std::path::Path,
    request: Request,
    wait: Option<std::time::Duration>,
) -> Result<Response, CallError> {
    use std::io::{BufRead, Write};
    let not_sent = |why: String| CallError::NotSent(why);
    // A list changes nothing: its only question is whether it answered.
    let look = match request {
        Request::List | Request::TokenList | Request::TokenCheck { .. } => None,
        Request::Create { .. } | Request::Adopt { .. } | Request::Restore { .. } => {
            Some("`wanderbooks books list` shows the books there are")
        }
        Request::BackUpNow { .. } => {
            Some("`wanderbooks books back-up` again backs up whatever it didn't")
        }
        Request::TokenCreate { .. } | Request::TokenRevoke { .. } => {
            Some("`wanderbooks token list` shows the tokens there are")
        }
        Request::UpgradeFormat { .. } => {
            Some("`wanderbooks books list` says whether the books still wait for it")
        }
    };
    let unknown = move |why: String| match look {
        None => CallError::Unknown(why),
        Some(look) => CallError::Unknown(format!(
            "{why}, so it may or may not have been done: {look}"
        )),
    };
    let quiet = |e: std::io::Error| match e.kind() {
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => {
            "the books server isn't answering".to_string()
        }
        kind => format!("the books server didn't answer ({kind})"),
    };
    let (dir, name) = (
        socket.parent().unwrap_or(std::path::Path::new(".")),
        socket
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default(),
    );
    let short = ledger_store::root::SocketPath::new(dir, name)
        .map_err(|e| not_sent(format!("can't reach the books server: {e}")))?;
    let stream =
        std::os::unix::net::UnixStream::connect(short.path()).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => {
                not_sent("the books server isn't running".to_string())
            }
            _ => not_sent(format!("can't reach the books server: {}", e.kind())),
        })?;
    crate::local::check_peer_std(&stream).map_err(not_sent)?;
    stream
        .set_read_timeout(wait)
        .map_err(|e| not_sent(e.to_string()))?;
    let mut line = serde_json::to_string(&Envelope {
        proto: PROTO,
        request,
    })
    .map_err(|e| not_sent(e.to_string()))?;
    line.push('\n');
    // A write cut part way may still have reached the server whole.
    (&stream).write_all(line.as_bytes()).map_err(|e| {
        unknown(format!(
            "the books server stopped taking the request ({})",
            e.kind()
        ))
    })?;
    let mut answer = String::new();
    std::io::BufReader::new(std::io::Read::take(&stream, MAX_LINE as u64 + 1))
        .read_line(&mut answer)
        .map_err(|e| unknown(quiet(e)))?;
    if answer.is_empty() {
        return Err(unknown("the books server stopped before answering".into()));
    }
    // INVARIANT 1: an answer is one line of at most MAX_LINE bytes; a
    // longer one, or one cut off, is refused, even if what came parses.
    if !answer.ends_with('\n') {
        return Err(unknown(
            "the books server's answer is too long, or cut off".into(),
        ));
    }
    serde_json::from_str(answer.trim_end())
        .map_err(|e| unknown(format!("the books server's answer isn't understood ({e})")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn requests_are_one_flat_object_with_their_version() {
        let line = serde_json::to_value(Envelope {
            proto: PROTO,
            request: Request::Create {
                id: "acme".into(),
                currency: "USD".into(),
            },
        })
        .unwrap();
        assert_eq!(
            line,
            json!({"proto": 1, "op": "create", "id": "acme", "currency": "USD"})
        );
        let back: Envelope = serde_json::from_value(line).unwrap();
        assert_eq!(back.proto, 1);
        let list: Envelope = serde_json::from_value(json!({"proto": 1, "op": "list"})).unwrap();
        assert_eq!(list.request, Request::List);
    }

    #[test]
    fn an_adopt_request_is_a_preview_unless_confirmed() {
        let r: Envelope = serde_json::from_value(
            json!({"proto": 1, "op": "adopt", "id": "acme", "source": "/x/books-data"}),
        )
        .unwrap();
        match r.request {
            Request::Adopt {
                confirmed,
                settings_toml,
                vault_dir,
                expect_head,
                skip_unknown,
                ..
            } => {
                assert!(!confirmed && !skip_unknown);
                assert!(settings_toml.is_none() && vault_dir.is_none() && expect_head.is_none());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_restore_request_is_a_preview_unless_confirmed() {
        let r: Envelope = serde_json::from_value(
            json!({"proto": 1, "op": "restore", "id": "acme", "source": "/backups/acme.git"}),
        )
        .unwrap();
        assert_eq!(
            r.request,
            Request::Restore {
                id: "acme".into(),
                source: "/backups/acme.git".into(),
                replace: false,
                confirmed: false,
                expect_head: None,
            }
        );
    }

    #[test]
    fn unknown_operations_and_fields_are_refused() {
        for bad in [
            json!({"proto": 1, "op": "delete", "id": "acme"}),
            json!({"proto": 1, "op": "create", "id": "acme", "currency": "USD", "path": "/x"}),
            json!({"proto": 1}),
        ] {
            assert!(
                serde_json::from_value::<Envelope>(bad.clone()).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn answers_round_trip() {
        for r in [
            Response::Books(vec![
                BookStatus {
                    id: "acme".into(),
                    open: false,
                    problem: Some("damaged".into()),
                    format_upgrade: None,
                    database: None,
                    other_databases: Vec::new(),
                },
                BookStatus {
                    id: "beta".into(),
                    open: true,
                    problem: None,
                    format_upgrade: Some(FormatWaiting {
                        format: 1,
                        features: Vec::new(),
                        adds: vec!["journal format 2".into()],
                    }),
                    database: Some("wanderbooks.v13.db".into()),
                    other_databases: vec!["wanderbooks.db".into()],
                },
            ]),
            Response::FormatUpgraded(FormatUpgraded {
                id: "beta".into(),
                added: vec!["journal format 2".into()],
                backup: Some(UpgradeBackup {
                    commit: "c".repeat(40),
                    head: (7, "ab".into()),
                }),
                head: (8, "cd".into()),
            }),
            Response::Created {
                id: "acme".into(),
                problem: None,
            },
            Response::Refused("no".into()),
            Response::TokenCreated {
                token: TokenInfo {
                    id: "tok_1".into(),
                    book: "acme".into(),
                    name: "agent".into(),
                    scopes: vec!["books:read".into()],
                    created: "2026-10-19T00:00:00.000Z".into(),
                },
                text: TokenText("wb_00".into()),
            },
            Response::Tokens(vec![]),
            Response::RestorePreview(RestorePreview {
                head: (3, "ab".into()),
                blobs: 2,
                vault_files: 1,
                exists: true,
                origin: Some("acme".into()),
            }),
            Response::Restored(Restored {
                id: "acme".into(),
                head: (3, "ab".into()),
                blobs_verified: 2,
                vault_files: 1,
                replaced: Some("acme-20261019-120000".into()),
                origin: Some("acme".into()),
                problem: None,
            }),
        ] {
            let text = serde_json::to_string(&r).unwrap();
            assert_eq!(serde_json::from_str::<Response>(&text).unwrap(), r);
        }
    }

    /// INVARIANT 7: the go-ahead for a format upgrade carries the owner
    /// passphrase as written, and Debug never shows it; a book listed by
    /// an earlier server (no format_upgrade) reads as waiting for none.
    #[test]
    fn an_upgrade_request_carries_the_passphrase_and_never_shows_it() {
        let r: Envelope = serde_json::from_value(json!({"proto": 1, "op": "upgrade_format",
            "id": "acme", "passphrase": "correct horse battery"}))
        .unwrap();
        assert_eq!(
            r.request,
            Request::UpgradeFormat {
                id: "acme".into(),
                passphrase: Passphrase("correct horse battery".into()),
            }
        );
        let shown = format!("{r:?}");
        assert!(!shown.contains("horse"), "{shown}");
        assert!(shown.contains("Passphrase(…)"), "{shown}");
        assert!(
            serde_json::from_value::<Envelope>(
                json!({"proto": 1, "op": "upgrade_format", "id": "acme"})
            )
            .is_err(),
            "an upgrade names the passphrase"
        );
        let listed: BookStatus =
            serde_json::from_value(json!({"id": "acme", "open": true})).unwrap();
        assert_eq!(listed.format_upgrade, None);
        let up = ledger_store::FormatUpgrade {
            from: ledger_store::JournalFormat::default(),
            to: ledger_store::format::current(),
        };
        let waiting = FormatWaiting::from(&up);
        assert_eq!((waiting.format, waiting.adds), (1, up.adds()));
    }

    #[test]
    fn token_requests_read_as_written() {
        let r: Envelope = serde_json::from_value(json!({"proto": 1, "op": "token_create",
            "book": "acme", "name": "agent", "scopes": ["books:read"]}))
        .unwrap();
        assert_eq!(
            r.request,
            Request::TokenCreate {
                book: "acme".into(),
                name: "agent".into(),
                scopes: vec!["books:read".into()],
                replace: false,
            }
        );
        let shown = format!(
            "{:?}",
            Response::TokenCreated {
                token: TokenInfo {
                    id: "tok_1".into(),
                    book: "acme".into(),
                    name: "agent".into(),
                    scopes: vec![],
                    created: String::new(),
                },
                text: TokenText("wb_secret".into()),
            }
        );
        assert!(!shown.contains("wb_secret"), "{shown}");
        // The text is there, and hidden: never shown as nothing.
        assert!(shown.contains("text: TokenText(…)"), "{shown}");
        let r: Envelope = serde_json::from_value(json!({"proto": 1, "op": "token_list"})).unwrap();
        assert_eq!(r.request, Request::TokenList);
        let r: Envelope = serde_json::from_value(json!({"proto": 1, "op": "token_check",
            "book": "acme", "name": "agent", "sha256": "ab"}))
        .unwrap();
        assert_eq!(
            r.request,
            Request::TokenCheck {
                book: "acme".into(),
                name: "agent".into(),
                sha256: "ab".into()
            }
        );
        assert!(
            serde_json::from_value::<Envelope>(
                json!({"proto": 1, "op": "token_check", "book": "acme", "name": "agent", "text": "wb_x"})
            )
            .is_err(),
            "a check carries no text"
        );
        assert!(
            serde_json::from_value::<Envelope>(
                json!({"proto": 1, "op": "token_revoke", "book": "acme"})
            )
            .is_err(),
            "a revoke names the token"
        );
    }

    #[test]
    fn a_listed_token_carries_no_text_or_hash() {
        let dir = tempfile::tempdir().unwrap();
        let mut p = ledger_store::auth::Principals::open(dir.path()).unwrap();
        let book = ledger_store::root::BookId::parse("acme").unwrap();
        let (t, secret) = p
            .create(
                &book,
                "agent",
                [ledger_store::auth::Scope::Read].into(),
                false,
            )
            .unwrap();
        let text = serde_json::to_string(&Response::Tokens(vec![TokenInfo::from(&t)])).unwrap();
        assert!(!text.contains(&secret.expose()[3..]));
        assert!(!text.contains(&t.sha256));
        assert!(text.contains("books:read"));
    }

    /// Why an admin call has no answer is what the command line shows.
    #[test]
    fn a_call_error_says_why() {
        assert_eq!(
            CallError::NotSent("the books server isn't running".into()).to_string(),
            "the books server isn't running"
        );
        assert_eq!(
            CallError::Unknown("no answer came".into()).to_string(),
            "no answer came"
        );
    }

    /// A fake books server on the socket `name` in `dir`: it takes one
    /// request and answers `answer`, byte for byte.
    #[cfg(unix)]
    fn answering(
        dir: &std::path::Path,
        name: &str,
        answer: Vec<u8>,
    ) -> (std::path::PathBuf, std::thread::JoinHandle<()>) {
        let socket = dir.join(name);
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            use std::io::{BufRead, Write};
            let (stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            std::io::BufReader::new(&stream)
                .read_line(&mut request)
                .unwrap();
            // A client that stops reading part way breaks the pipe: fine.
            let _ = (&stream).write_all(&answer);
        });
        (socket, server)
    }

    /// A refusal padded with spaces to `len` bytes, then its newline.
    fn padded(len: usize) -> Vec<u8> {
        let mut answer = br#"{"refused":"no"}"#.to_vec();
        answer.resize(len, b' ');
        answer.push(b'\n');
        answer
    }

    /// INVARIANT 1, the command line's side: an answer of 4 MiB, its
    /// newline not counted, is taken whole (settings texts ride in them);
    /// a byte more is refused, as is an answer cut off before its
    /// newline, even when what came parses.
    #[cfg(unix)]
    #[test]
    fn an_answer_is_one_line_of_at_most_four_mib() {
        let four_mib = 4 * 1024 * 1024;
        let dir = tempfile::tempdir().unwrap();
        let (socket, server) = answering(dir.path(), "whole.sock", padded(four_mib));
        assert_eq!(
            call(&socket, Request::List),
            Ok(Response::Refused("no".into()))
        );
        server.join().unwrap();
        let too_long_or_cut = Err(CallError::Unknown(
            "the books server's answer is too long, or cut off".into(),
        ));
        let (socket, server) = answering(dir.path(), "long.sock", padded(four_mib + 1));
        assert_eq!(call(&socket, Request::List), too_long_or_cut);
        server.join().unwrap();
        let (socket, server) = answering(dir.path(), "cut.sock", br#"{"refused":"no"}"#.into());
        assert_eq!(call(&socket, Request::List), too_long_or_cut);
        server.join().unwrap();
    }

    /// No server on the socket: the request never left, and the command
    /// line says the server isn't running, whether there is no socket or
    /// one a stopped server left behind.
    #[cfg(unix)]
    #[test]
    fn no_server_means_nothing_was_sent() {
        let dir = tempfile::tempdir().unwrap();
        let not_running = Err(CallError::NotSent("the books server isn't running".into()));
        assert_eq!(
            call(&dir.path().join("admin.sock"), Request::List),
            not_running
        );
        let stale = dir.path().join("stale.sock");
        drop(std::os::unix::net::UnixListener::bind(&stale).unwrap());
        assert_eq!(
            call(
                &stale,
                Request::Create {
                    id: "acme".into(),
                    currency: "USD".into(),
                }
            ),
            not_running
        );
    }
}
