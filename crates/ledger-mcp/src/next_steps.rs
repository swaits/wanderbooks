//! The result envelope: what every tool result carries besides its own
//! fields (plan 4.6).
//!
//! ```text
//! GOAL: an agent never has to guess what to do after a call: every result
//!       says what follows (`next_steps`), what refused it and which tool
//!       remedies that (`guards`), what to tell the human (`warnings`),
//!       and, on a tool that can change the books, where the journal ends
//!       now (`head`).
//! SOURCE: plan/served.md 4.6 and P5 step 1; DECISION (P5).
//! INVARIANTS:
//!   1. The envelope is applied in one place (`apply`, called by
//!      `call_tool`), so no tool can forget it.
//!   2. Every result of a tool that is not read-only has a `head` (the
//!      journal after the call's last write) and at least one next step; a
//!      read has `next_steps` when something follows it and nothing else (a
//!      resource and its tool agree). `guards` is on a refusal, and a
//!      success without warnings has no `warnings` key.
//!   3. A tool's own fields are never overwritten: a payload that already
//!      has `next_steps`, `guards`, `warnings` or `head` keeps it.
//!   4. A refused call has no head and no steps; a refusal that is a
//!      guard (guards.rs) carries it as `structuredContent.guards`.
//!   5. Next steps from the books' attention list are the viewer's own
//!      items (ledger_package::attention), filtered to the subject the
//!      call touched.
//! ```

use super::BooksServer;
use super::guards;
use ledger_package::{Integrity, Severity, Target};
use rmcp::model::{CallToolResponse, ContentBlock};
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use std::sync::OnceLock;

/// One thing to do next.
#[derive(Clone, Debug)]
pub(crate) struct Step {
    /// The tool to call.
    pub tool: Option<String>,
    /// Or the prompt to run.
    pub prompt: Option<String>,
    /// Or a command for the human's computer (a download).
    pub command: Option<String>,
    /// Its arguments.
    pub args: Option<Value>,
    /// Why.
    pub why: String,
    /// Whether the work is not done without it.
    pub required: bool,
}

impl Step {
    pub(crate) fn tool(tool: &str, args: Option<Value>, why: &str, required: bool) -> Step {
        Step {
            tool: Some(tool.into()),
            prompt: None,
            command: None,
            args,
            why: why.into(),
            required,
        }
    }

    fn to_json(&self) -> Value {
        let mut o = Map::new();
        if let Some(t) = &self.tool {
            o.insert("tool".into(), json!(t));
        }
        if let Some(p) = &self.prompt {
            o.insert("prompt".into(), json!(p));
        }
        if let Some(c) = &self.command {
            o.insert("command".into(), json!(c));
        }
        if let Some(a) = &self.args {
            o.insert("args".into(), a.clone());
        }
        o.insert("why".into(), json!(self.why));
        o.insert("required".into(), json!(self.required));
        Value::Object(o)
    }
}

/// The tools that are not read-only: their results carry a `head`.
pub(crate) fn mutating_tools() -> &'static HashSet<String> {
    static SET: OnceLock<HashSet<String>> = OnceLock::new();
    SET.get_or_init(|| {
        BooksServer::tool_router()
            .list_all()
            .into_iter()
            .filter(|t| {
                !t.annotations
                    .as_ref()
                    .and_then(|a| a.read_only_hint)
                    .unwrap_or(false)
            })
            .map(|t| t.name.to_string())
            .collect()
    })
}

/// The steps a tool's own result implies (no books needed): the confirm
/// after a preview, and what follows a particular act.
pub(crate) fn tool_steps(
    tool: &str,
    args: Option<&Map<String, Value>>,
    result: &Value,
) -> Vec<Step> {
    let mut steps = Vec::new();
    let confirmed = args
        .and_then(|a| a.get("confirmed"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let entry = result
        .get("entry_id")
        .or_else(|| args.and_then(|a| a.get("entry_id")))
        .and_then(Value::as_u64);
    let with_entry = |id: u64| Some(json!({ "entry_id": id }));
    // A preview: what is missing first, then the human confirms, then the
    // same call with the digest.
    if let (false, Some(preview), Some(args)) = (
        confirmed,
        result.get("preview").and_then(Value::as_str),
        args,
    ) {
        if tool == "books.post_entry"
            && result.get("receipt_missing").and_then(Value::as_bool) == Some(true)
            && let Some(id) = entry
        {
            steps.push(Step::tool(
                "books.attach_receipt",
                with_entry(id),
                "This entry needs a receipt before it posts: attach the file (upload it first).",
                true,
            ));
            steps.push(Step::tool(
                "books.set_draft_state",
                Some(json!({ "entry_id": id, "state": "awaiting_evidence" })),
                "Or leave the draft waiting, with a note saying what for, until the receipt arrives.",
                false,
            ));
        }
        let mut confirm = args.clone();
        confirm.insert("confirmed".into(), json!(true));
        confirm.insert("preview".into(), json!(preview));
        steps.push(Step::tool(
            tool,
            Some(Value::Object(confirm)),
            "Show this preview to the human; only when they agree, make the same call with confirmed true and this preview.",
            true,
        ));
        return steps;
    }
    match tool {
        "books.set_setting" | "books.add_account" => {
            steps.push(Step::tool(
                "books.add_account",
                None,
                "Add the accounts the human's chart needs, one at a time.",
                false,
            ));
            steps.push(Step::tool(
                "books.draft_entry",
                None,
                "Or record a transaction: draft it, add its lines, preview it, then post.",
                false,
            ));
        }
        "books.draft_entry" => {
            if let Some(id) = entry {
                steps.push(Step::tool(
                    "books.add_line",
                    with_entry(id),
                    "Add the draft's lines: each is an account, a side and an amount; debits must equal credits.",
                    true,
                ));
            }
        }
        "books.add_line" => {
            if let Some(id) = entry {
                steps.push(Step::tool(
                    "books.add_line",
                    with_entry(id),
                    "Add the next line of the draft.",
                    false,
                ));
                steps.push(Step::tool(
                    "books.post_entry",
                    with_entry(id),
                    "When the lines balance, preview the post for the human.",
                    false,
                ));
            }
        }
        "books.set_draft_state" => {
            if let Some(id) = entry {
                steps.push(Step::tool(
                    "books.attach_receipt",
                    with_entry(id),
                    "When the document arrives, upload and attach it to this draft.",
                    false,
                ));
            }
        }
        "books.attach_receipt" => {
            if let Some(id) = entry {
                steps.push(Step::tool(
                    "books.post_entry",
                    with_entry(id),
                    "The evidence is attached: preview the post again for the human.",
                    false,
                ));
            }
        }
        "books.post_entry" | "books.reverse_entry" => {
            if let Some(id) = entry {
                steps.push(Step::tool(
                    "books.show",
                    with_entry(id),
                    "Show the human the entry and what it moved.",
                    false,
                ));
                if tool == "books.post_entry" {
                    steps.push(Step::tool(
                        "books.reverse_entry",
                        with_entry(id),
                        "Only if this was posted in error: reverse it (posted entries are never edited).",
                        false,
                    ));
                }
            }
        }
        "books.draft_from_template" => {
            if let Some(id) = entry {
                steps.push(Step::tool(
                    "books.post_entry",
                    with_entry(id),
                    "Preview the draft for the human, who then confirms it.",
                    false,
                ));
            }
        }
        "books.record_statement" => {
            steps.push(Step::tool(
                "books.mark_cleared",
                None,
                "Mark each transaction the statement shows as cleared.",
                true,
            ));
            let (account, end, close, attachment) = (
                result.get("account_code"),
                result.get("period_end"),
                result.get("closing_balance"),
                result.get("attachment_id"),
            );
            if let (Some(account), Some(end), Some(close), Some(attachment)) =
                (account, end, close, attachment)
            {
                steps.push(Step::tool(
                    "books.reconcile",
                    Some(json!({
                        "account": account,
                        "statement_date": end,
                        "statement_balance": close,
                        "statement_attachment": attachment,
                    })),
                    "When every line is marked, preview the reconciliation against the statement.",
                    false,
                ));
            }
        }
        "books.mark_cleared" => steps.push(Step::tool(
            "books.reconcile",
            None,
            "Mark the rest, then preview the reconciliation against the statement.",
            false,
        )),
        "books.reconcile" => steps.push(Step::tool(
            "books.reconciliations",
            None,
            "See the reconciliation among the account's others.",
            false,
        )),
        "books.list_periods" => {
            for p in result
                .get("periods")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|p| p.get("status").and_then(Value::as_str) == Some("open"))
                .take(3)
            {
                if let Some(id) = p.get("id").and_then(Value::as_u64) {
                    steps.push(Step::tool(
                        "books.close_period",
                        Some(json!({ "period_id": id })),
                        "Once everything in this period is recorded and reconciled, preview its close.",
                        false,
                    ));
                }
            }
        }
        "books.close_period" => steps.push(Step::tool(
            "books.verify",
            None,
            "Verify the books after a close: the chain, the replay and every stored file.",
            false,
        )),
        "books.export_cpa_package" => {
            if let Some(cmd) = result.pointer("/file/download").and_then(Value::as_str) {
                steps.push(Step {
                    tool: None,
                    prompt: None,
                    command: Some(cmd.to_string()),
                    args: None,
                    why: "Run this to save the package in a folder you name, within 15 minutes."
                        .into(),
                    required: true,
                });
            }
        }
        _ => {}
    }
    steps
}

/// The attention items about the subject of the call, as steps.
fn attention_steps(
    server: &BooksServer,
    subject: Subject,
    store: &ledger_store::Store,
) -> Vec<Step> {
    let cfg = server.settings();
    let settings = ledger_package::Settings {
        company: &cfg.company,
        cash_accounts: &cfg.roles.cash,
        financing_accounts: &cfg.roles.financing,
        investing_accounts: &cfg.roles.investing,
    };
    let today = ledger_core::settings::today_in(cfg.timezone.as_deref());
    let items = ledger_package::find_with(store, today, &settings, &Integrity::default());
    items
        .into_iter()
        .filter(|a| match (subject, a.target) {
            (Subject::Entry(s), Target::Entry(t)) => s == t,
            (Subject::Invoice(s), Target::Invoice(t)) => s == t,
            _ => false,
        })
        .filter_map(|a| {
            let (tool, args) = a.tool?;
            let args = if args.is_null() {
                match (a.target, tool.as_str()) {
                    (
                        Target::Entry(id),
                        "books.attach_receipt"
                        | "books.set_entry_party"
                        | "books.set_entry_settles",
                    ) => Some(json!({ "entry_id": id })),
                    _ => None,
                }
            } else {
                Some(args)
            };
            let title: String = a.title.chars().take(160).collect();
            Some(Step::tool(
                &tool,
                args,
                &title,
                a.severity == Severity::Problem,
            ))
        })
        .take(4)
        .collect()
}

/// What a call touched, for the attention filter.
#[derive(Clone, Copy)]
enum Subject {
    Entry(u64),
    Invoice(u64),
}

fn subject_of(args: Option<&Map<String, Value>>, result: &Value) -> Option<Subject> {
    let pick = |k: &str| {
        result
            .get(k)
            .or_else(|| args.and_then(|a| a.get(k)))
            .and_then(Value::as_u64)
    };
    pick("entry_id")
        .map(Subject::Entry)
        .or_else(|| pick("invoice_id").map(Subject::Invoice))
}

/// The fallback of a tool that changes things and has nothing particular
/// to follow it.
fn digest_step() -> Step {
    Step::tool(
        "books.digest",
        None,
        "See what changed and offer it to the human.",
        false,
    )
}

/// Put the envelope on a tool's answer (INVARIANT 1). `wrote` is the head
/// after the call's last write, when it wrote.
pub(crate) fn apply(
    server: &BooksServer,
    tool: &str,
    args: Option<&Map<String, Value>>,
    out: Result<CallToolResponse, rmcp::ErrorData>,
) -> Result<CallToolResponse, rmcp::ErrorData> {
    let Ok(CallToolResponse::Complete(mut r)) = out else {
        return out;
    };
    if r.is_error == Some(true) {
        // INVARIANT 4.
        let text = r
            .content
            .iter()
            .find_map(|c| match c {
                ContentBlock::Text(t) => Some(t.text.as_str()),
                _ => None,
            })
            .unwrap_or("");
        let guards = guards::from_refusal(tool, text);
        if !guards.is_empty() && r.structured_content.is_none() {
            r.structured_content = Some(json!({ "guards": guards }));
        }
        return Ok(CallToolResponse::Complete(r));
    }
    let Some(Value::Object(mut body)) = r.structured_content.take() else {
        return Ok(CallToolResponse::Complete(r));
    };
    let before = Value::Object(body.clone());
    let mutating = mutating_tools().contains(tool);
    let result = Value::Object(body.clone());
    let mut steps = tool_steps(tool, args, &result);
    let head = if mutating {
        server.lock().ok().map(|store| {
            let (seq, hash) = store.head();
            if let Some(subject) = subject_of(args, &result)
                && steps.iter().all(|s| !s.required)
            {
                steps.extend(attention_steps(server, subject, &store));
            }
            json!({ "seq": seq, "hash": hash })
        })
    } else {
        None
    };
    if mutating && steps.is_empty() {
        steps.push(digest_step());
    }
    // A tool that can change the books always says what follows; a read
    // says it only when something does, so what it says is what a resource
    // of the same name says (`books://settings` is `books.settings`).
    // `guards` is on a refusal (above) and `warnings` is the tool's own:
    // a success that has none says none, as the tools always did.
    if !body.contains_key("next_steps") && (mutating || !steps.is_empty()) {
        body.insert(
            "next_steps".into(),
            Value::Array(steps.iter().map(Step::to_json).collect()),
        );
    }

    if let Some(head) = head {
        body.entry("head").or_insert(head);
    }
    let after = Value::Object(body);
    // The same JSON in the text block (server.rs INVARIANT 1).
    for c in &mut r.content {
        if let ContentBlock::Text(t) = c
            && serde_json::from_str::<Value>(&t.text).is_ok_and(|v| v == before)
        {
            t.text = after.to_string();
        }
    }
    r.structured_content = Some(after);
    Ok(CallToolResponse::Complete(r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preview_gets_its_confirm_step() {
        let args = json!({"entry_id": 3}).as_object().cloned().unwrap();
        let steps = tool_steps(
            "books.post_entry",
            Some(&args),
            &json!({"entry_id": 3, "preview": "abc", "status": "preview"}),
        );
        assert_eq!(steps.len(), 1);
        let a = steps[0].args.as_ref().unwrap();
        assert_eq!(a["confirmed"], true);
        assert_eq!(a["preview"], "abc");
        assert!(steps[0].required);
    }

    #[test]
    fn every_step_a_tool_names_exists() {
        let names: HashSet<String> = BooksServer::tool_router()
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        let result = json!({"entry_id": 1, "file": {"download": "x"}});
        for tool in mutating_tools() {
            for s in tool_steps(tool, Some(&Map::new()), &result)
                .into_iter()
                .chain([digest_step()])
            {
                if let Some(t) = s.tool {
                    assert!(
                        names.contains(&t),
                        "{tool}: step names {t}, which is no tool"
                    );
                }
            }
        }
    }
}
