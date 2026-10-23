//! `books.access_log`: who reached the books through the server, and what
//! came of it (plan I-9, 4.x ops; books:admin).
//!
//! ```text
//! GOAL: the owner, through their agent, can read the access log, newest
//!       first, narrowed to a caller, an action or an outcome.
//! SOURCE: plan I-9 ("readable through books.access_log (admin)").
//! INVARIANTS:
//!   1. It reads the log whole and checks its chain before answering; a
//!      damaged log is reported, never shown in part as if whole.
//!   2. It answers at most MAX_RECORDS records, newest first, and says
//!      whether there are more.
//! ```

use super::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The most records one call returns (INVARIANT 2).
const MAX_RECORDS: usize = 500;
/// How many it returns when not told.
const DEFAULT_RECORDS: usize = 50;

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct AccessLogParams {
    /// How many records, newest first: default 50, at most 500.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Only records before this one (its seq), to page back.
    #[serde(default)]
    pub before: Option<u64>,
    /// Only calls by the token of this name.
    #[serde(default)]
    pub token: Option<String>,
    /// Only this action: tool, resource, or connect (a request refused at
    /// the door: no token, another book's, another session's).
    #[serde(default)]
    pub action: Option<String>,
    /// Only this outcome: ok, error or refused.
    #[serde(default)]
    pub outcome: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AccessLogOut {
    /// The records, newest first.
    pub records: Vec<serde_json::Value>,
    /// Records in the whole log.
    pub total: u64,
    /// Whether older records match too (page back with `before`).
    pub more: bool,
    /// Where the log's chain ends: its last seq and hash.
    pub head: serde_json::Value,
}

#[tool_router(router = access_router, vis = "pub(super)")]
impl BooksServer {
    #[tool(
        name = "books.access_log",
        title = "Access log",
        description = "Who reached these books through the server, newest first: each call's token, client, tool or resource, a digest of its arguments, and whether it ran, failed or was refused. Narrow it by token name, action (tool, resource, connect: refused at the door) or outcome (ok, error, refused); page back with before. Needs books:admin.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn access_log(
        &self,
        params: Parameters<AccessLogParams>,
    ) -> Result<Json<AccessLogOut>, String> {
        let p = params.0;
        let limit = p.limit.unwrap_or(DEFAULT_RECORDS);
        if limit == 0 || limit > MAX_RECORDS {
            return Err(format!("limit is 1 to {MAX_RECORDS}"));
        }
        if let Some(o) = &p.outcome
            && !["ok", "error", "refused"].contains(&o.as_str())
        {
            return Err("outcome is ok, error or refused".into());
        }
        let dir = self.lock()?.data_dir().to_path_buf();
        // INVARIANT 1.
        let (records, head) = ledger_store::access::read(&dir)
            .map_err(|e| format!("the access log can't be read: {e}"))?;
        let total = records.len() as u64;
        let mut matching = records.into_iter().rev().filter(|r| {
            p.before.is_none_or(|b| r.seq < b)
                && p.token
                    .as_ref()
                    .is_none_or(|t| &r.access.principal.label == t)
                && p.action.as_ref().is_none_or(|a| &r.access.action == a)
                && p.outcome.as_ref().is_none_or(|o| {
                    matches!(
                        (o.as_str(), &r.access.result),
                        ("ok", ledger_store::access::Outcome::Ok)
                            | ("error", ledger_store::access::Outcome::Error)
                            | ("refused", ledger_store::access::Outcome::Refused(_))
                    )
                })
        });
        let out: Vec<serde_json::Value> = matching
            .by_ref()
            .take(limit)
            .map(|r| serde_json::to_value(r).expect("a record serializes"))
            .collect();
        let more = matching.next().is_some();
        Ok(Json(AccessLogOut {
            records: out,
            total,
            more,
            head: serde_json::json!({"seq": head.seq, "hash": head.hash}),
        }))
    }
}
