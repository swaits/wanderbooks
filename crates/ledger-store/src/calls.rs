//! Calls: every write a tool makes is one transaction that happens once
//! (plan P15 2a, D-88, D-89).
//!
//! A call that writes frames its journal lines with two line types, and
//! a call that wrote nothing is just a closing line:
//!
//! * `call.begun { tool, token, key, args }`, written lazily, just before
//!   the call's first line;
//! * `call.closed { tool, token, key, args, outcome, first_seq, last_seq,
//!   answer | error }`, written when the call is over. `outcome` is `ok`
//!   (the answer the caller was given is `answer`, kept as text so a
//!   repeat returns it byte for byte), `failed` (the call got as far as
//!   writing lines and then failed: `error` says how), or `unfinished`
//!   (written by the next open of books whose last call never closed).
//!
//! The lines between a begin and its close count only if the close says
//! `ok`: that is how a reader tells a complete call from an incomplete
//! one, and the one rule every replay uses ([`CallGate`]). The envelope
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
//!      line's seq and its `last_seq` the line before it, or both are
//!      absent when the call wrote no lines. A begin inside a frame, a
//!      mismatched close, or an `unfinished` close with no frame is a
//!      corrupt journal.
//!   4. The projection never holds a line of a call that did not commit:
//!      the call's SQLite changes live in one transaction that is released
//!      only after the `ok` closing line is on disk.
//!   5. The key table (token, key) -> answer is built from `ok` closing
//!      lines alone, so a restart rebuilds it from the journal.
//!   6. Books that haven't declared `call_transactions` take no call: a
//!      begin is refused (FormatUpgradePending), journaling nothing.
//! SCENARIOS:
//!   1. IF the process dies between a call's lines and its closing line:
//!      SHOULD, at the next open, apply none of them, write an
//!      `unfinished` close, and rebuild the projection if it holds them.
//!   2. IF a call fails after writing lines: SHOULD journal a `failed`
//!      close, roll its SQLite changes back, and replay the journal
//!      without its lines.
//!   3. IF a call fails before writing a line: SHOULD journal nothing.
//! ```

use std::collections::HashMap;

use serde_json::{Value, json};

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
    /// The answer it gave (an `ok` call): JSON text.
    pub answer: Option<String>,
    /// How it failed (a `failed` call).
    pub error: Option<String>,
}

/// What the books kept of a call that committed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recorded {
    /// The tool.
    pub tool: String,
    /// The digest of its arguments, without the key.
    pub args: String,
    /// The answer it gave, as JSON text.
    pub answer: String,
    /// The seq of its closing line.
    pub seq: u64,
}

/// The keys of every call that committed, by (token, key).
#[derive(Clone, Debug, Default)]
pub struct Keys {
    by: HashMap<(String, String), Recorded>,
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
        let Some(answer) = closed.answer.clone() else {
            return;
        };
        self.by
            .entry((closed.id.token.clone(), closed.id.key.clone()))
            .or_insert(Recorded {
                tool: closed.id.tool.clone(),
                args: closed.id.args.clone(),
                answer,
                seq,
            });
    }

    fn get(&self, token: &str, key: &str) -> Option<&Recorded> {
        self.by.get(&(token.to_string(), key.to_string()))
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
    if outcome == CallOutcome::Ok && answer.is_none() {
        return Err(corrupt(line, "answer"));
    }
    Ok(Closed {
        id,
        outcome,
        first_seq: seq_of("first_seq")?,
        last_seq: seq_of("last_seq")?,
        answer,
        error: text_of("error")?,
    })
}

fn begun_payload(id: &CallId) -> Value {
    json!({ "tool": id.tool, "token": id.token, "key": id.key, "args": id.args })
}

fn closed_payload(
    id: &CallId,
    outcome: CallOutcome,
    range: Option<(u64, u64)>,
    answer: Option<&str>,
    error: Option<&str>,
) -> Value {
    let mut p = begun_payload(id);
    p["outcome"] = json!(outcome.as_str());
    if let Some((first, last)) = range {
        p["first_seq"] = json!(first);
        p["last_seq"] = json!(last);
    }
    if let Some(a) = answer {
        p["answer"] = json!(a);
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
                    if closed.first_seq.is_some()
                        || closed.last_seq.is_some()
                        || closed.outcome == CallOutcome::Unfinished
                    {
                        return Err(corrupt(line, "closes a call that never began"));
                    }
                    return Ok(vec![index]);
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

    /// What the books kept of the call that ran under this token and key,
    /// if one committed.
    pub fn recorded_call(&self, token: &str, key: &str) -> Option<&Recorded> {
        self.keys.get(token, key)
    }

    /// Whether a call is open.
    pub fn in_call(&self) -> bool {
        self.call.is_some()
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

    /// Close the open call. `Done`: journal its closing line with the
    /// answer, and release the projection's transaction: the call
    /// committed. `Failed`: if the call wrote lines, journal a `failed`
    /// closing line, roll the projection back and rebuild the live ledger
    /// without those lines; if it wrote none, journal nothing.
    ///
    /// If the closing line cannot be written, the call has not committed
    /// (it will close `unfinished` at the next open), the store refuses
    /// every further write, and the error is returned: the caller stops
    /// taking requests.
    pub fn close_call(&mut self, end: CallEnd<'_>) -> Result<(), StoreError> {
        let call = self
            .call
            .take()
            .ok_or_else(|| StoreError::CallRefused("no call is open".into()))?;
        let range = call.begun.map(|first| (first, self.journal.head().0));
        match end {
            CallEnd::Done(answer) => {
                let payload = closed_payload(&call.id, CallOutcome::Ok, range, Some(answer), None);
                let line = self
                    .journal
                    .append(CLOSED, payload)
                    .map_err(|e| self.dangle(e))?;
                // The projection commits with the line: released only
                // once the closing line is on disk (INVARIANT 4).
                apply_line_to_sqlite(&self.conn, &line)
                    .and_then(|()| Ok(self.conn.execute_batch("RELEASE call_tx")?))
                    .map_err(|e| self.dangle(e))?;
                let closed = closed_of(&line)?;
                self.keys.note(&closed, line.seq);
                Ok(())
            }
            CallEnd::Failed(error) => {
                if range.is_none() {
                    self.conn
                        .execute_batch("ROLLBACK TO call_tx; RELEASE call_tx")?;
                    return Ok(());
                }
                let payload =
                    closed_payload(&call.id, CallOutcome::Failed, range, None, Some(error));
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
    /// written.
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
            Some((u.first_seq, u.last_seq)),
            None,
            Some(
                "the call never finished: the books were closed or the program stopped before its closing line was written",
            ),
        );
        let line = self.journal.append(CLOSED, payload)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }
}
