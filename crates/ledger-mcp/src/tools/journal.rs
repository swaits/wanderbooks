//! `books.journal_history`: the journal in words (plan P5 step 4).
//!
//! ```text
//! GOAL: an agent can trace what the books did, line by line, without the
//!       journal file: the only way to "read the journal".
//! SOURCE: plan/served.md 4.3 (tracing figures), P5 step 4; the describer
//!         is ledger_store::describe.
//! INVARIANTS:
//!   1. Read-only; needs books:read; at most 200 lines a call, 50 by
//!      default, in journal order from the line after `after_seq`.
//!   2. A sentence quotes no setting value and no full TIN
//!      (ledger_store::describe INVARIANT 2).
//!   3. `subject` is `kind:id`; a bad one is refused with the shapes.
//! ```

use super::*;
use ledger_store::describe::{HistoryFilter, parse_subject};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The most lines one call returns.
const HISTORY_MAX: usize = 200;
/// The lines a call returns when it names no limit.
const HISTORY_DEFAULT: usize = 50;

/// Parameters for `books.journal_history`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct JournalHistoryParams {
    /// Only lines after this journal seq (default: from the first).
    #[serde(default)]
    pub after_seq: Option<u64>,
    /// Only these line types: an exact type (`entry.posted`) or a group
    /// ending in a dot (`entry.`).
    #[serde(default)]
    pub types: Vec<String>,
    /// Only lines about one subject: `entry:12`, `invoice:3`, `bill:3`,
    /// `account:10`, `party:5` or `asset:2`.
    #[serde(default)]
    pub subject: Option<String>,
    /// At most this many lines (default 50, most 200).
    #[serde(default)]
    pub limit: Option<u32>,
}

/// One journal line, in words.
#[derive(Debug, Serialize, JsonSchema)]
pub struct JournalLineOut {
    /// Its seq.
    pub seq: u64,
    /// When it was written (UTC).
    pub ts: String,
    /// Its type, e.g. `entry.posted`.
    #[serde(rename = "type")]
    pub line_type: String,
    /// What it did, in a sentence.
    pub summary: String,
    /// What it is about, as `kind:id`, if it is about one thing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    /// The entry it belongs to, if it is about one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<u64>,
}

/// A torn journal tail set aside when the books opened.
#[derive(Debug, Serialize, JsonSchema)]
pub struct TailRepairOut {
    /// When (UTC).
    pub at: String,
    /// How many bytes were set aside.
    pub bytes: u64,
}

/// Result of `books.journal_history`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct JournalHistoryOut {
    /// The lines, in journal order.
    pub lines: Vec<JournalLineOut>,
    /// Whether more lines match after the last (ask again with its seq as
    /// after_seq).
    pub more: bool,
    /// The journal's last seq.
    pub head_seq: u64,
    /// Torn tails set aside on open, oldest first.
    pub tail_repairs: Vec<TailRepairOut>,
}

#[tool_router(router = journal_router, vis = "pub(super)")]
impl BooksServer {
    #[tool(
        name = "books.journal_history",
        title = "Journal history",
        description = "The journal in words, in order: each line's seq, time, type and a sentence saying what it did, to what, and the entry it belongs to. Filter by after_seq, types (an exact type like entry.posted, or a group like entry.) and subject (entry:12, invoice:3, bill:3, account:10, party:5, asset:2). The way to trace a figure back to what produced it, and the only way to see what the journal holds. Read-only; at most 200 lines a call.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub(super) async fn journal_history(
        &self,
        params: Parameters<JournalHistoryParams>,
    ) -> Result<Json<JournalHistoryOut>, String> {
        let p = params.0;
        let subject = p
            .subject
            .as_deref()
            .map(|s| parse_subject(s).map(|(k, id)| format!("{k}:{id}")))
            .transpose()?;
        let limit = (p.limit.unwrap_or(HISTORY_DEFAULT as u32) as usize).clamp(1, HISTORY_MAX);
        let filter = HistoryFilter {
            after_seq: p.after_seq.unwrap_or(0),
            types: p.types,
            subject,
            limit,
        };
        let store = self.lock()?;
        let h = store.journal_history(&filter).map_err(|e| e.to_string())?;
        Ok(Json(JournalHistoryOut {
            lines: h
                .lines
                .into_iter()
                .map(|d| JournalLineOut {
                    seq: d.seq,
                    ts: d.ts,
                    line_type: d.line_type,
                    summary: d.summary,
                    subject: d.subjects.first().cloned(),
                    entry_id: d.entry_id,
                })
                .collect(),
            more: h.more,
            head_seq: h.head_seq,
            tail_repairs: h
                .tail_repairs
                .into_iter()
                .map(|(at, bytes)| TailRepairOut { at, bytes })
                .collect(),
        }))
    }
}
