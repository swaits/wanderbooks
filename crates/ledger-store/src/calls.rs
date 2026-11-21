//! Calls: every write a tool makes is one transaction that happens once
//! (plan P15 2a, D-88, D-89).
//!
//! A call that writes journal lines frames them with two line types; a
//! call that wrote none leaves nothing in the journal at all:
//!
//! * `call.begun { tool, token, key, args }`, written lazily, just before
//!   the call's first line;
//! * `call.closed { tool, token, key, args, outcome, first_seq, last_seq,
//!   answer | answer_sha256 + answer_bytes | error }`, written when the
//!   call is over. `outcome` is `ok` (the answer the caller was given is
//!   `answer`, kept as text so a repeat returns it byte for byte; above
//!   [`ANSWER_CAP`] bytes the closing line keeps `answer_sha256` and
//!   `answer_bytes` instead, and a repeat is told in words that the call
//!   was done), `failed` (the call got as far as writing lines and then
//!   failed: `error` says how), or `unfinished` (written by the next open
//!   of books whose last call never closed).
//!
//! `args` is a keyed digest of the call's arguments (HMAC-SHA256 under
//! the books' own call key, `keys/calls.key`, never journaled): the
//! arguments themselves, and any tax id in them, are not recoverable or
//! testable from the journal.
//!
//! The lines between a begin and its close count only if the close says
//! `ok`: that is how a reader tells a complete call from an incomplete
//! one, and the one rule every reader applies ([`CallGate`], [`standings`],
//! [`committed`]). Ids are handed out again after a failed call (nothing
//! it wrote counted), so a reader that takes the journal raw would pin the
//! dead lines of a failed call on whatever takes the id next: every reader
//! that derives a fact reads [`committed`]. The envelope
//! `{seq, ts, type, payload, prev}` is untouched.
//!
//! ```text
//! GOAL: a call's lines commit together or not at all, a failed call is
//!       on the record, and a repeated request key answers what the first
//!       call answered and writes nothing.
//! SOURCE: WB-format-8, WB-format-9, WB-format-10, WB-format-11, WB-format-12;
//!         plan/p15-app-and-relay.md 2a (D-88, D-89).
//! INVARIANTS:
//!   1. Replay applies a call's lines only at its `ok` closing line, in
//!      order, and never any line of a call that closed `failed` or
//!      `unfinished`, or that never closed: no state is ever built from
//!      half a call. A replay of any prefix that ends inside a call
//!      equals the replay of the prefix before that call's begin line.
//!   2. Lines outside a begin..close frame (every line written before the
//!      books declared the feature, and the owner's own acts: the format
//!      declaration, a repaired tail) apply as ever.
//!   3. A close names the frame it ends: its `first_seq` is the begin
//!      line's seq and its `last_seq` the line before it. A close with no
//!      frame, a begin inside a frame, a mismatched close, or an
//!      `unfinished` close with no frame is a corrupt journal.
//!   4. The projection never holds a line of a call that did not commit:
//!      the call's SQLite changes live in one transaction that is released
//!      only after the `ok` closing line is on disk.
//!   5. The key table (token, key) -> call is built from `ok` closing
//!      lines alone, so a restart rebuilds it from the journal. A call
//!      that wrote no line has no closing line: its key and answer are
//!      kept in memory for the life of the store only, in a table bounded
//!      in count and bytes, and a repeat after a restart runs again (it
//!      wrote nothing, so nothing is done twice).
//!   6. Books that haven't declared `call_transactions` take no call: a
//!      begin is refused (FormatUpgradePending), journaling nothing.
//!   7. Every reader that derives a fact (an entry's origin, an asset's
//!      trail, a party's events, an attention item, a preview of what
//!      adopting would keep) reads [`committed`], never the raw lines; a
//!      reader that shows history shows every line and says in words of
//!      each line of a call that did not count that it did not ([`Standing`]).
//!   8. A closing line's answer is at most [`ANSWER_CAP`] bytes; above
//!      that the line keeps the answer's SHA-256 and length, and a repeat
//!      says in words that the call was done and writes nothing.
//! SCENARIOS:
//!   1. IF the process dies between a call's lines and its closing line:
//!      SHOULD, at the next open, apply none of them, write an
//!      `unfinished` close, and rebuild the projection if it holds them.
//!   2. IF a call fails after writing lines: SHOULD journal a `failed`
//!      close, roll its SQLite changes back, and replay the journal
//!      without its lines.
//!   3. IF a call fails before writing a line: SHOULD journal nothing.
//!   4. IF a call's closing line is on disk but its SQLite changes can't
//!      be committed: SHOULD say so (the call counts), heal from the
//!      journal, and refuse more writes until it has.
//! ```

use std::collections::{HashMap, VecDeque};
use std::path::Path;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{Store, apply_line_to_sqlite, get_str, get_u64};
use crate::error::StoreError;
use crate::format::{self, CALL_TRANSACTIONS};
use crate::journal::JournalLine;

/// The line type that opens a call's frame.
pub const BEGUN: &str = "call.begun";
/// The line type that closes it.
pub const CLOSED: &str = "call.closed";

/// A request key is at least this many characters.
pub const MIN_KEY_LEN: usize = 8;
/// A request key is at most this many characters.
pub const MAX_KEY_LEN: usize = 128;

/// The longest answer (its JSON text, in bytes) a closing line keeps
/// whole: 4 KiB. Above it the line keeps the answer's SHA-256 and length
/// (INVARIANT 8).
pub const ANSWER_CAP: usize = 4096;

/// The most calls that wrote no line whose key and answer are kept in
/// memory (INVARIANT 5); the oldest go first.
pub const SESSION_KEYS_MAX: usize = 512;
/// The most bytes of answers kept for them: 2 MiB.
pub const SESSION_BYTES_MAX: usize = 2 * 1024 * 1024;

/// The file, in the books' `keys/` folder, holding the secret the
/// journal's argument digests are keyed with.
pub const CALLS_KEY_FILE: &str = "calls.key";

/// Who made a call, what it is, and what it asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallId {
    /// The token the call came with (a key is scoped to its token).
    pub token: String,
    /// The tool's name.
    pub tool: String,
    /// The request key.
    pub key: String,
    /// The digest of the call's arguments, without the key.
    pub args: String,
}

/// How a call ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallOutcome {
    /// It committed: its lines count.
    Ok,
    /// It failed after writing lines: none of them count.
    Failed,
    /// It never closed: written by the open that found it.
    Unfinished,
}

impl CallOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Failed => "failed",
            Self::Unfinished => "unfinished",
        }
    }
}

/// A closing line, read.
#[derive(Clone, Debug)]
pub struct Closed {
    /// The call.
    pub id: CallId,
    /// How it ended.
    pub outcome: CallOutcome,
    /// The seq of the begin line, when the call wrote lines.
    pub first_seq: Option<u64>,
    /// The seq of the call's last line, when it wrote lines.
    pub last_seq: Option<u64>,
    /// The answer it gave (an `ok` call), when short enough to keep:
    /// JSON text.
    pub answer: Option<String>,
    /// The SHA-256 of the answer, hex, when it was too long to keep.
    pub answer_sha256: Option<String>,
    /// The length of that answer in bytes.
    pub answer_bytes: Option<u64>,
    /// How it failed (a `failed` call).
    pub error: Option<String>,
}

/// What a repeat of a call that committed is answered with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Replay {
    /// The answer it gave, as JSON text, byte for byte.
    Answer(String),
    /// Its answer was too long to keep: what the closing line says of it.
    Summary {
        /// The seq of its begin line.
        first_seq: u64,
        /// The seq of its last line before the closing line.
        last_seq: u64,
        /// The seq of the closing line.
        closed_seq: u64,
        /// The SHA-256 of the answer it gave, hex.
        answer_sha256: String,
        /// The answer's length in bytes.
        answer_bytes: u64,
    },
}

/// What the books kept of a call that committed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recorded {
    /// The tool.
    pub tool: String,
    /// The keyed digest of its arguments.
    pub args: String,
    /// What a repeat is answered with.
    pub replay: Replay,
}

/// What the table keeps of a call with lines: no answer (a repeat reads
/// it from the closing line), so the table's size is one small entry per
/// committed call, whatever the answers' sizes.
#[derive(Clone, Debug)]
struct Entry {
    tool: String,
    args: String,
    /// The seq of the closing line.
    seq: u64,
}

/// The calls that wrote no line, kept for the life of the store: bounded
/// by count and by bytes of answers, oldest first out (INVARIANT 5).
#[derive(Clone, Debug, Default)]
struct Session {
    by: HashMap<(String, String), (String, String, String)>,
    order: VecDeque<(String, String)>,
    bytes: usize,
}

impl Session {
    fn note(&mut self, id: &CallId, answer: &str) {
        if answer.len() > ANSWER_CAP {
            // Not kept: a repeat of a call that wrote nothing just runs
            // again.
            return;
        }
        let k = (id.token.clone(), id.key.clone());
        if self.by.contains_key(&k) {
            return;
        }
        self.bytes += answer.len();
        self.order.push_back(k.clone());
        self.by
            .insert(k, (id.tool.clone(), id.args.clone(), answer.to_string()));
        while self.order.len() > SESSION_KEYS_MAX || self.bytes > SESSION_BYTES_MAX {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            if let Some((_, _, a)) = self.by.remove(&old) {
                self.bytes -= a.len();
            }
        }
    }
}

/// The keys of every call that committed, by (token, key).
#[derive(Clone, Debug, Default)]
pub struct Keys {
    by: HashMap<(String, String), Entry>,
    session: Session,
}

impl Keys {
    /// Every `ok` closing line of `lines`, oldest first (INVARIANT 5). The
    /// first call with a key wins, as it did when it ran.
    pub(crate) fn from_lines(lines: &[JournalLine]) -> Result<Self, StoreError> {
        let mut keys = Self::default();
        for line in lines.iter().filter(|l| l.line_type == CLOSED) {
            let closed = closed_of(line)?;
            if closed.outcome == CallOutcome::Ok {
                keys.note(&closed, line.seq);
            }
        }
        Ok(keys)
    }

    fn note(&mut self, closed: &Closed, seq: u64) {
        self.by
            .entry((closed.id.token.clone(), closed.id.key.clone()))
            .or_insert(Entry {
                tool: closed.id.tool.clone(),
                args: closed.id.args.clone(),
                seq,
            });
    }

    /// How many calls with lines the table knows (a test's question).
    pub fn journaled(&self) -> usize {
        self.by.len()
    }

    /// How many calls that wrote nothing it holds, and the bytes of their
    /// answers.
    pub fn session_size(&self) -> (usize, usize) {
        (self.session.order.len(), self.session.bytes)
    }
}

fn corrupt(line: &JournalLine, what: &str) -> StoreError {
    StoreError::JournalCorrupt(format!("seq {}: {} {what}", line.seq, line.line_type))
}

fn call_id_of(line: &JournalLine) -> Result<CallId, StoreError> {
    let p = &line.payload;
    let field = |k: &str| {
        get_str(p, k)
            .map(str::to_string)
            .map_err(|_| corrupt(line, k))
    };
    Ok(CallId {
        token: field("token")?,
        tool: field("tool")?,
        key: field("key")?,
        args: field("args")?,
    })
}

/// A `call.begun` line's call: a missing or mistyped key is corrupt.
pub fn begun_of(line: &JournalLine) -> Result<CallId, StoreError> {
    call_id_of(line)
}

/// A `call.closed` line, read: a missing or mistyped key is corrupt, an
/// outcome this version doesn't know is newer (BS-format-1).
pub fn closed_of(line: &JournalLine) -> Result<Closed, StoreError> {
    let p = &line.payload;
    let id = call_id_of(line)?;
    let outcome = match get_str(p, "outcome").map_err(|_| corrupt(line, "outcome"))? {
        "ok" => CallOutcome::Ok,
        "failed" => CallOutcome::Failed,
        "unfinished" => CallOutcome::Unfinished,
        other => {
            return Err(format::newer_content(
                line,
                format!("call outcome {other:?}"),
            ));
        }
    };
    let seq_of = |k: &str| match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => get_u64(p, k).map(Some).map_err(|_| corrupt(line, k)),
    };
    let text_of = |k: &str| match p.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_str()
            .map(|s| Some(s.to_string()))
            .ok_or_else(|| corrupt(line, k)),
    };
    let answer = text_of("answer")?;
    let answer_sha256 = text_of("answer_sha256")?;
    let answer_bytes = seq_of("answer_bytes")?;
    if outcome == CallOutcome::Ok {
        let kept_whole = answer.is_some() && answer_sha256.is_none() && answer_bytes.is_none();
        let kept_hash = answer.is_none()
            && answer_bytes.is_some_and(|n| n > ANSWER_CAP as u64)
            && answer_sha256
                .as_deref()
                .is_some_and(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()));
        if !(kept_whole || kept_hash) {
            return Err(corrupt(line, "answer"));
        }
        if answer.as_ref().is_some_and(|a| a.len() > ANSWER_CAP) {
            return Err(corrupt(line, "answer (longer than the cap)"));
        }
    }
    let (first_seq, last_seq) = (seq_of("first_seq")?, seq_of("last_seq")?);
    if first_seq.is_none() || last_seq.is_none() {
        return Err(corrupt(line, "first_seq and last_seq"));
    }
    Ok(Closed {
        id,
        outcome,
        first_seq,
        last_seq,
        answer,
        answer_sha256,
        answer_bytes,
        error: text_of("error")?,
    })
}

fn begun_payload(id: &CallId) -> Value {
    json!({ "tool": id.tool, "token": id.token, "key": id.key, "args": id.args })
}

/// What a closing line says of the answer: the answer itself, or, above
/// [`ANSWER_CAP`], its hash and length.
enum Kept<'a> {
    Answer(&'a str),
    Nothing,
}

fn closed_payload(
    id: &CallId,
    outcome: CallOutcome,
    range: (u64, u64),
    answer: Kept<'_>,
    error: Option<&str>,
) -> Value {
    let mut p = begun_payload(id);
    p["outcome"] = json!(outcome.as_str());
    p["first_seq"] = json!(range.0);
    p["last_seq"] = json!(range.1);
    if let Kept::Answer(a) = answer {
        if a.len() <= ANSWER_CAP {
            p["answer"] = json!(a);
        } else {
            p["answer_sha256"] = json!(hex::encode(Sha256::digest(a.as_bytes())));
            p["answer_bytes"] = json!(a.len());
        }
    }
    if let Some(e) = error {
        p["error"] = json!(e);
    }
    p
}

/// A frame that never closed, at the end of a journal.
#[derive(Clone, Debug)]
pub(crate) struct Unfinished {
    pub(crate) id: CallId,
    pub(crate) first_seq: u64,
    pub(crate) last_seq: u64,
}

struct Pending {
    id: CallId,
    begun_seq: u64,
    events: Vec<usize>,
    last_seq: u64,
}

/// The one rule every replay applies (INVARIANTS 1 to 3): fed the lines
/// in order, it says which of them to apply, now.
///
/// A line outside a frame applies at once; the begin line applies at once
/// (it changes nothing); the lines inside a frame wait, and are released
/// with the closing line when it says `ok`, and dropped when it says
/// anything else.
#[derive(Default)]
pub(crate) struct CallGate {
    pending: Option<Pending>,
    skipped: Vec<(u64, u64)>,
}

impl CallGate {
    /// Take the line at `index`; the indices (into the caller's lines) to
    /// apply now, in order.
    pub(crate) fn push(
        &mut self,
        index: usize,
        line: &JournalLine,
    ) -> Result<Vec<usize>, StoreError> {
        match line.line_type.as_str() {
            BEGUN => {
                if self.pending.is_some() {
                    return Err(corrupt(line, "begins inside a call that never closed"));
                }
                self.pending = Some(Pending {
                    id: begun_of(line)?,
                    begun_seq: line.seq,
                    events: Vec::new(),
                    last_seq: line.seq,
                });
                Ok(vec![index])
            }
            CLOSED => {
                let closed = closed_of(line)?;
                let Some(call) = self.pending.take() else {
                    return Err(corrupt(line, "closes a call that never began"));
                };
                if closed.id != call.id
                    || closed.first_seq != Some(call.begun_seq)
                    || closed.last_seq != Some(line.seq.saturating_sub(1))
                {
                    return Err(corrupt(line, "doesn't close the call it follows"));
                }
                if closed.outcome == CallOutcome::Ok {
                    let mut out = call.events;
                    out.push(index);
                    Ok(out)
                } else {
                    if !call.events.is_empty() {
                        self.skipped.push((call.begun_seq + 1, call.last_seq));
                    }
                    Ok(vec![index])
                }
            }
            _ => match &mut self.pending {
                Some(call) => {
                    call.events.push(index);
                    call.last_seq = line.seq;
                    Ok(Vec::new())
                }
                None => Ok(vec![index]),
            },
        }
    }

    /// The frame that has not closed yet, if any.
    pub(crate) fn unfinished(&self) -> Option<Unfinished> {
        self.pending.as_ref().map(|c| Unfinished {
            id: c.id.clone(),
            first_seq: c.begun_seq,
            last_seq: c.last_seq,
        })
    }

    /// Whether `seq` is a line of a call that did not commit: closed
    /// `failed` or `unfinished`, or not closed yet (the begin line apart).
    pub(crate) fn is_uncommitted(&self, seq: u64) -> bool {
        self.skipped.iter().any(|&(a, b)| (a..=b).contains(&seq))
            || self
                .pending
                .as_ref()
                .is_some_and(|c| seq > c.begun_seq && seq <= c.last_seq)
    }
}

/// [`CallGate`] over a whole journal: the indices to apply, in order, the
/// frame left open at the end, and the gate (for what it knows of the
/// lines that did not commit).
pub(crate) fn gate_all(lines: &[JournalLine]) -> Result<(Vec<usize>, CallGate), StoreError> {
    let mut gate = CallGate::default();
    let mut released = Vec::with_capacity(lines.len());
    for (i, line) in lines.iter().enumerate() {
        released.extend(gate.push(i, line)?);
    }
    Ok((released, gate))
}

/// Whether a journal line counts, and if not, why (INVARIANT 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    /// It counts: outside any call, or inside one that closed `ok`.
    Counts,
    /// A `call.begun` or `call.closed` line: the frame, which says what
    /// became of the lines between and is no fact of the books itself.
    Frame,
    /// Inside a call that closed `failed`: it did not count.
    Failed,
    /// Inside a call that closed `unfinished`: it did not count.
    Unfinished,
    /// Inside a call that has not closed yet: it does not count yet.
    Pending,
}

impl Standing {
    /// Whether the line is a fact of the books.
    pub fn counts(self) -> bool {
        self == Self::Counts
    }

    /// In words, why the line does not count; None for a line that
    /// counts and for a frame line.
    pub fn note(self) -> Option<&'static str> {
        match self {
            Self::Counts | Self::Frame => None,
            Self::Failed => Some("not counted: the call that wrote it failed"),
            Self::Unfinished => Some("not counted: the call that wrote it never finished"),
            Self::Pending => Some("not counted yet: the call that wrote it is still running"),
        }
    }
}

/// The standing of every line of `lines`, in order: the one question
/// every reader asks of the journal (INVARIANTS 1, 7). A journal that is
/// not a sound sequence of frames is corrupt.
pub fn standings(lines: &[JournalLine]) -> Result<Vec<Standing>, StoreError> {
    let mut gate = CallGate::default();
    let mut out = Vec::with_capacity(lines.len());
    let mut frame: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        gate.push(i, line)?;
        match line.line_type.as_str() {
            BEGUN => {
                frame = Some(i);
                out.push(Standing::Frame);
            }
            CLOSED => {
                out.push(Standing::Frame);
                if let Some(begun) = frame.take() {
                    let settled = match closed_of(line)?.outcome {
                        CallOutcome::Ok => Standing::Counts,
                        CallOutcome::Failed => Standing::Failed,
                        CallOutcome::Unfinished => Standing::Unfinished,
                    };
                    for s in &mut out[begun + 1..i] {
                        *s = settled;
                    }
                }
            }
            _ => out.push(if frame.is_some() {
                Standing::Pending
            } else {
                Standing::Counts
            }),
        }
    }
    Ok(out)
}

/// The committed journal: the lines that are facts of the books, in
/// order: those outside any call and those of calls that closed `ok`,
/// the frame lines themselves left out (INVARIANT 7). What every reader
/// that derives a fact from the journal's lines reads.
pub fn committed(lines: &[JournalLine]) -> Result<Vec<JournalLine>, StoreError> {
    let standing = standings(lines)?;
    Ok(lines
        .iter()
        .zip(standing)
        .filter(|(_, s)| s.counts())
        .map(|(l, _)| l.clone())
        .collect())
}

/// HMAC-SHA256 (RFC 2104) of `msg` under `key`, hex.
pub(crate) fn hmac_sha256_hex(key: &[u8], msg: &[u8]) -> String {
    const BLOCK: usize = 64;
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    inner.update(msg);
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    hex::encode(outer.finalize())
}

/// The secret a book's call arguments are keyed with: 32 random bytes in
/// `keys/calls.key` of its folder (mode 0600, the folder 0700), made on
/// first need and never written to the journal, a log or an answer. The
/// error names no path.
fn calls_key(data_dir: &Path) -> Result<Vec<u8>, StoreError> {
    let unsure = |what: &str| {
        StoreError::CallRefused(format!(
            "the books' call key can't be {what}, so the call was not run and nothing was changed"
        ))
    };
    let file = data_dir.join(crate::root::KEYS_DIR).join(CALLS_KEY_FILE);
    match std::fs::read_to_string(&file) {
        Ok(text) => hex::decode(text.trim())
            .ok()
            .filter(|b| b.len() == 32)
            .ok_or_else(|| unsure("read: its file isn't a key")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            crate::root::private_book_folder(data_dir, crate::root::KEYS_DIR)
                .map_err(|_| unsure("kept: the keys folder isn't usable"))?;
            let mut key = [0u8; 32];
            getrandom::getrandom(&mut key).map_err(|_| unsure("made: no randomness"))?;
            match crate::root::write_new_private_file(&file, hex::encode(key).as_bytes()) {
                Ok(()) => Ok(key.to_vec()),
                // Another process made it first: use theirs.
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => calls_key(data_dir),
                Err(_) => Err(unsure("kept: it can't be written")),
            }
        }
        Err(_) => Err(unsure("read")),
    }
}

/// The keyed digest of a call's arguments: HMAC-SHA256 over their
/// canonical JSON, unmasked, under the books' call key. Two requests that
/// differ only in a tax id differ here; nothing in it lets anyone recover
/// or test a guess of one without the key, which is never journaled.
pub fn args_mac(data_dir: &Path, args: &Value) -> Result<String, StoreError> {
    let key = calls_key(data_dir)?;
    Ok(hmac_sha256_hex(
        &key,
        crate::journal::canonical_json(args).as_bytes(),
    ))
}

/// The books' id: the one `store.created` carries (books made since
/// `call_transactions`), else the one the format upgrade added to its
/// `journal.format_set`. None for books that have had neither yet.
pub fn books_id_of(lines: &[JournalLine]) -> Option<String> {
    lines
        .iter()
        .filter(|l| l.line_type == "store.created" || l.line_type == format::FORMAT_SET)
        .find_map(|l| l.payload.get("books_id").and_then(Value::as_str))
        .map(str::to_string)
}

/// A fresh books id: 128 random bits, in hex.
pub(crate) fn new_books_id() -> Result<String, StoreError> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|e| {
        StoreError::Io(std::io::Error::other(format!(
            "no randomness for the books' id: {e}"
        )))
    })?;
    Ok(hex::encode(bytes))
}

/// The call a store is in, between [`Store::begin_call`] and
/// [`Store::close_call`].
pub(crate) struct ActiveCall {
    id: CallId,
    /// The seq of its begin line once it has written one.
    begun: Option<u64>,
}

/// How a call ended, as [`Store::close_call`] is told.
pub enum CallEnd<'a> {
    /// It answered; this is the answer (JSON text).
    Done(&'a str),
    /// It failed; this is how.
    Failed(&'a str),
}

impl Store {
    /// The books' id, if they have one (see [`books_id_of`]).
    pub fn books_id(&self) -> Option<&str> {
        self.books_id.as_deref()
    }

    /// Whether these books declared `call_transactions`: they take calls.
    pub fn takes_calls(&self) -> bool {
        self.format.has_feature(CALL_TRANSACTIONS)
    }

    /// Refuse unless the books take calls (D-89), journaling nothing: the
    /// refusal says what the owner does.
    pub fn ensure_takes_calls(&self) -> Result<(), StoreError> {
        self.ensure_feature(CALL_TRANSACTIONS)
    }

    /// The keyed digest of a call's arguments ([`args_mac`]), under this
    /// book's call key.
    pub fn args_mac(&self, args: &Value) -> Result<String, StoreError> {
        args_mac(&self.data_dir, args)
    }

    /// What the books kept of the call that ran under this token and key,
    /// if one committed: from the session's table for a call that wrote
    /// nothing, else from the journal's closing line.
    pub fn recorded_call(&self, token: &str, key: &str) -> Result<Option<Recorded>, StoreError> {
        let k = (token.to_string(), key.to_string());
        if let Some((tool, args, answer)) = self.keys.session.by.get(&k) {
            return Ok(Some(Recorded {
                tool: tool.clone(),
                args: args.clone(),
                replay: Replay::Answer(answer.clone()),
            }));
        }
        let Some(entry) = self.keys.by.get(&k) else {
            return Ok(None);
        };
        // Read from the closing line: the table holds no answers.
        let lines = crate::journal::read_and_verify_journal(self.journal.path())?;
        let line = lines.iter().find(|l| l.seq == entry.seq).ok_or_else(|| {
            StoreError::JournalCorrupt(format!("seq {}: no closing line", entry.seq))
        })?;
        let closed = closed_of(line)?;
        let replay = match closed.answer {
            Some(answer) => Replay::Answer(answer),
            None => Replay::Summary {
                first_seq: closed.first_seq.unwrap_or_default(),
                last_seq: closed.last_seq.unwrap_or_default(),
                closed_seq: line.seq,
                answer_sha256: closed.answer_sha256.unwrap_or_default(),
                answer_bytes: closed.answer_bytes.unwrap_or_default(),
            },
        };
        Ok(Some(Recorded {
            tool: entry.tool.clone(),
            args: entry.args.clone(),
            replay,
        }))
    }

    /// The table's sizes: calls with lines, and the session's calls that
    /// wrote none with the bytes of their answers.
    pub fn call_table(&self) -> (usize, (usize, usize)) {
        (self.keys.journaled(), self.keys.session_size())
    }

    /// Whether a call is open.
    pub fn in_call(&self) -> bool {
        self.call.is_some()
    }

    /// Whether the open call has written a line (its begin line is on
    /// disk).
    pub fn call_has_lines(&self) -> bool {
        self.call.as_ref().is_some_and(|c| c.begun.is_some())
    }

    /// Open a call: from here every line this store journals belongs to
    /// it, until [`Self::close_call`]. Journals nothing yet (the begin
    /// line comes with the first line); the projection's changes are held
    /// in one transaction.
    ///
    /// ```text
    /// SCENARIOS:
    ///   1. IF the books haven't declared call_transactions: SHOULD
    ///      refuse (FormatUpgradePending), journaling nothing.
    ///   2. IF a call is already open, or an earlier call's closing line
    ///      couldn't be written: SHOULD refuse (CallRefused).
    /// ```
    pub fn begin_call(&mut self, id: CallId) -> Result<(), StoreError> {
        self.ensure_takes_calls()?;
        if self.dangling {
            return Err(StoreError::CallRefused(
                "an earlier call's closing line couldn't be written: the books must be opened again before they take another".into(),
            ));
        }
        if self.call.is_some() {
            return Err(StoreError::CallRefused(
                "a call is already open in these books".into(),
            ));
        }
        self.conn.execute_batch("SAVEPOINT call_tx")?;
        self.call = Some(ActiveCall { id, begun: None });
        Ok(())
    }

    /// Write the begin line of the open call, if it has none yet. Every
    /// append comes through here (`append_journal`).
    pub(super) fn begin_frame(&mut self) -> Result<(), StoreError> {
        if self.dangling {
            return Err(StoreError::CallRefused(
                "an earlier call's closing line couldn't be written: the books must be opened again before they take another".into(),
            ));
        }
        let Some(call) = &self.call else {
            return Ok(());
        };
        if call.begun.is_some() {
            return Ok(());
        }
        let line = self.journal.append(BEGUN, begun_payload(&call.id))?;
        apply_line_to_sqlite(&self.conn, &line)?;
        if let Some(call) = &mut self.call {
            call.begun = Some(line.seq);
        }
        Ok(())
    }

    /// Close the open call.
    ///
    /// `Done`, with lines written: journal its closing line with the
    /// answer (or, above [`ANSWER_CAP`], the answer's hash and length),
    /// and release the projection's transaction: the call committed. With
    /// none written: journal nothing, release the transaction, and keep
    /// the key and answer in memory (INVARIANT 5).
    ///
    /// `Failed`: if the call wrote lines, journal a `failed` closing
    /// line, roll the projection back and rebuild the live ledger without
    /// those lines; if it wrote none, journal nothing.
    ///
    /// If the closing line cannot be written, the call has not committed
    /// (it will close `unfinished` at the next open), the store refuses
    /// every further write, and the error is returned. If it is on disk
    /// but the projection can't be brought up to it, the call *has*
    /// committed: the store heals from the journal where it can, and
    /// where it can't refuses every further write and returns
    /// [`StoreError::CallCommitted`] (SCENARIO 4).
    pub fn close_call(&mut self, end: CallEnd<'_>) -> Result<(), StoreError> {
        let call = self
            .call
            .take()
            .ok_or_else(|| StoreError::CallRefused("no call is open".into()))?;
        let range = call.begun.map(|first| (first, self.journal.head().0));
        match (end, range) {
            (CallEnd::Done(answer), None) => {
                self.conn.execute_batch("RELEASE call_tx")?;
                self.keys.session.note(&call.id, answer);
                Ok(())
            }
            (CallEnd::Done(answer), Some(range)) => {
                let payload =
                    closed_payload(&call.id, CallOutcome::Ok, range, Kept::Answer(answer), None);
                let line = self
                    .journal
                    .append(CLOSED, payload)
                    .map_err(|e| self.dangle(e))?;
                // The closing line is on disk: the call committed, whatever
                // becomes of the projection (INVARIANT 4).
                let closed = closed_of(&line)?;
                self.keys.note(&closed, line.seq);
                let applied = apply_line_to_sqlite(&self.conn, &line)
                    .and_then(|()| Ok(self.conn.execute_batch("RELEASE call_tx")?));
                if let Err(e) = applied {
                    // Roll back what the transaction held, then bring the
                    // projection up from the journal, as an open would.
                    let _ = self
                        .conn
                        .execute_batch("ROLLBACK TO call_tx; RELEASE call_tx");
                    if self.reload().is_err() {
                        self.dangling = true;
                        return Err(StoreError::CallCommitted(format!(
                            "the call's closing line is saved, so it counts, but the books' working copy could not be brought up to it ({e}); it is rebuilt from the journal when the books are opened again"
                        )));
                    }
                }
                Ok(())
            }
            (CallEnd::Failed(_), None) => {
                self.conn
                    .execute_batch("ROLLBACK TO call_tx; RELEASE call_tx")?;
                Ok(())
            }
            (CallEnd::Failed(error), Some(range)) => {
                let payload = closed_payload(
                    &call.id,
                    CallOutcome::Failed,
                    range,
                    Kept::Nothing,
                    Some(error),
                );
                self.journal
                    .append(CLOSED, payload)
                    .map_err(|e| self.dangle(e))?;
                self.conn
                    .execute_batch("ROLLBACK TO call_tx; RELEASE call_tx")
                    .map_err(StoreError::from)
                    .and_then(|()| self.reload())
                    .map_err(|e| self.dangle(e))?;
                Ok(())
            }
        }
    }

    /// An open call that can't be finished (its closing line can't be
    /// written, or what follows it fails): the call stays unfinished in the
    /// journal or its effects are unsure, the projection's changes are dropped,
    /// and the store takes nothing more.
    fn dangle(&mut self, e: StoreError) -> StoreError {
        self.dangling = true;
        let _ = self
            .conn
            .execute_batch("ROLLBACK TO call_tx; RELEASE call_tx");
        e
    }

    /// The live ledger and the projection, brought to what a restart
    /// would build: the journal read through the gate, the ledger replayed
    /// from it, and the projection (rolled back to before the call) caught
    /// up with the lines that count. After a call failed with lines
    /// written, or when a committed call's projection could not be
    /// released.
    fn reload(&mut self) -> Result<(), StoreError> {
        let lines = crate::journal::read_and_verify_journal(self.journal.path())?;
        let (released, _) = gate_all(&lines)?;
        let mut ledger = ledger_core::Ledger::new(self.base_currency);
        for &i in &released {
            super::replay_line_into_ledger(&mut ledger, &lines[i], self.base_currency)?;
        }
        self.ledger = ledger;
        self.catch_up(&lines, &released)
    }

    /// At open: the journal's last call never closed. Journal an
    /// `unfinished` close for it, so the record says what became of the
    /// lines and the next call begins on a closed frame.
    pub(super) fn close_unfinished(&mut self, u: &Unfinished) -> Result<(), StoreError> {
        let payload = closed_payload(
            &u.id,
            CallOutcome::Unfinished,
            (u.first_seq, u.last_seq),
            Kept::Nothing,
            Some(
                "the call never finished: the books were closed or the program stopped before its closing line was written",
            ),
        );
        let line = self.journal.append(CLOSED, payload)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 4231, test cases 1 and 2 (and 6, a key longer than the block).
    #[test]
    fn hmac_sha256_matches_rfc_4231() {
        assert_eq!(
            hmac_sha256_hex(&[0x0b; 20], b"Hi There"),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        assert_eq!(
            hmac_sha256_hex(b"Jefe", b"what do ya want for nothing?"),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        assert_eq!(
            hmac_sha256_hex(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            ),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    /// Two requests that differ only in a tax id differ in their digests;
    /// the digest holds neither, and is the same for the same request.
    #[test]
    fn args_mac_tells_tax_ids_apart_and_holds_none() {
        let tmp = tempfile::tempdir().unwrap();
        let a = json!({"party": 1, "tin": "111-22-3333"});
        let b = json!({"party": 1, "tin": "111-22-3334"});
        let (ma, mb) = (
            args_mac(tmp.path(), &a).unwrap(),
            args_mac(tmp.path(), &b).unwrap(),
        );
        assert_ne!(ma, mb);
        assert_eq!(ma, args_mac(tmp.path(), &a).unwrap());
        // Not the plain hash of the arguments: a guess can't be tested
        // without the books' key.
        assert_ne!(
            ma,
            hex::encode(Sha256::digest(journal_canonical(&a).as_bytes()))
        );
        assert_eq!(ma.len(), 64);
        // Another book has another key.
        let other = tempfile::tempdir().unwrap();
        assert_ne!(ma, args_mac(other.path(), &a).unwrap());
        // The key is a file of its own, not the vault's.
        assert!(
            tmp.path()
                .join(crate::root::KEYS_DIR)
                .join(CALLS_KEY_FILE)
                .is_file()
        );
    }

    fn journal_canonical(v: &Value) -> String {
        crate::journal::canonical_json(v)
    }
}
