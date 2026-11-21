//! Every write a tool makes is one transaction that happens once (plan
//! P15 2a, D-88, D-89): the request key, the repeat, and the frame around
//! a call. The journal's side is `ledger_store::calls`.
//!
//! ```text
//! GOAL: a write whose answer is lost and which is then retried happens
//!       once; a call half done is never left looking done; a failed
//!       write is on the record.
//! SOURCE: WB-format-8, WB-format-9, WB-format-10, WB-format-11;
//!         plan/p15-app-and-relay.md 2a.
//! INVARIANTS:
//!   1. Every tool that can change anything (no readOnlyHint, but for
//!      READS_THAT_SEAL, which write nothing and may answer with a TIN) takes a
//!      required `request_key` in its schema; a call that writes without
//!      one is refused in words, before it runs. A preview (a confirmed
//!      tool called without confirmed: true) writes nothing and needs
//!      none. The key is taken out of the arguments before the tool sees
//!      them, so a tool's own checks and previews are unchanged.
//!   2. A write call runs alone among writes: they take turns, so a
//!      call's lines are one run in the journal, and a repeat that
//!      arrives while its original still runs waits for it, then gets its
//!      answer.
//!   3. A repeat of a (token, key) whose call committed, with the same
//!      tool and arguments, returns that call's answer as it was given and
//!      writes nothing; with other arguments (or another tool) it is
//!      refused as a misuse of the key. The access log records a repeat
//!      as `repeat`.
//!   4. A call that answered ok commits with a closing line holding its
//!      answer; one that answered with an error, was stopped by a close or
//!      failed by any other means rolls back, is journaled as `failed`
//!      when it had written lines, and leaves its key unused.
//!   5. Books that haven't declared call_transactions take no write: every
//!      write call is refused with the upgrade remedy, before it runs.
//!      Reads are never refused for it.
//!   6. No full TIN is kept in the journal (BS-partner-2): no answer holds
//!      one (READS_THAT_SEAL, the three that can, are no transaction), and
//!      an error text that holds one is kept masked.
//! SCENARIOS:
//!   1. IF a write is called again with the same key after its answer was
//!      lost: SHOULD return the first answer, and the books hold one of
//!      its effects.
//!   2. IF the same key comes with a different request: SHOULD refuse,
//!      saying so and that nothing was changed.
//!   3. IF the process dies between the call's lines: SHOULD leave none
//!      of them in the books (ledger_store::calls).
//! ```

use ledger_store::calls::{CallEnd, CallId, MAX_KEY_LEN, MIN_KEY_LEN};
use rmcp::model::{CallToolResponse, CallToolResult};
use serde_json::{Map, Value, json};

use super::BooksServer;
use crate::scopes::{Needs, TABLE};

/// The argument every write call carries.
pub const KEY_FIELD: &str = "request_key";

/// Why a write was refused before it ran, for its access record.
pub(crate) const MISSING_KEY: &str = "request_key_missing";
/// The key was used for another request.
pub(crate) const KEY_REUSED: &str = "request_key_reused";
/// The books are waiting for the owner's format upgrade (D-89).
pub(crate) const UPGRADE_PENDING: &str = "format_upgrade_pending";
/// The call repeated one that committed: its answer was returned.
pub(crate) const REPEAT: &str = "repeat";

/// The words that tell a model what the key is, whenever it is asked for.
const KEY_WORDS: &str = "request_key: any text you make up, 8 to 128 characters (a UUID is fine), new for each different action and never reused for another. If a call's answer is lost, send the same call again with the same key: the books will not do it twice, and will answer what they answered the first time";

/// The tools that are not read-only by their annotation but write nothing
/// to the books: they read what is sealed or put it into a file, and the
/// answer of each may hold a full TIN, which no journal line may (BS-partner-2).
/// They take no request key and are no transaction; they are read as any
/// read is (INVARIANT 1).
pub(crate) const READS_THAT_SEAL: [&str; 3] = [
    "books.vendor_1099",
    "books.get_w9",
    "books.export_cpa_package",
];

/// The tools whose calls can write: no readOnlyHint (`next_steps::
/// mutating_tools`), except [`READS_THAT_SEAL`] (83 of the 86).
pub(crate) fn writes(tool: &str) -> bool {
    super::next_steps::mutating_tools().contains(tool) && !READS_THAT_SEAL.contains(&tool)
}

/// Whether a call to `tool` with `args` writes: a write tool, except a
/// confirmed tool called as a preview (INVARIANT 1).
pub(crate) fn is_write_call(tool: &str, args: Option<&Map<String, Value>>) -> bool {
    if !writes(tool) {
        return false;
    }
    let previews = TABLE
        .iter()
        .any(|(n, needs)| *n == tool && *needs == Needs::PostWhenConfirmed);
    !(previews
        && !args
            .and_then(|a| a.get("confirmed"))
            .and_then(Value::as_bool)
            .unwrap_or(false))
}

/// Take the request key out of the arguments (INVARIANT 1).
pub(crate) fn take_key(args: &mut Option<Map<String, Value>>) -> Option<Value> {
    args.as_mut().and_then(|a| a.remove(KEY_FIELD))
}

/// What is wrong with a call's key, in words; None when it will do.
pub(crate) fn key_problem(key: Option<&Value>) -> Option<String> {
    match key {
        None | Some(Value::Null) => Some(format!(
            "this call changes the books and carries no request_key, so it was not run and nothing was changed. Send it again with a {KEY_WORDS}."
        )),
        Some(Value::String(k)) => {
            let n = k.chars().count();
            if k != k.trim() || k.chars().any(char::is_control) {
                Some("the request_key has spaces at its ends or control characters in it, so the call was not run and nothing was changed. Send it again with a key of plain text".into())
            } else if !(MIN_KEY_LEN..=MAX_KEY_LEN).contains(&n) {
                Some(format!(
                    "the request_key is {n} characters; it must be {MIN_KEY_LEN} to {MAX_KEY_LEN}, so the call was not run and nothing was changed. Send it again with a {KEY_WORDS}."
                ))
            } else {
                None
            }
        }
        Some(_) => Some(format!(
            "the request_key must be text, so the call was not run and nothing was changed. Send it again with a {KEY_WORDS}."
        )),
    }
}

/// The key as text, once it has passed [`key_problem`].
pub(crate) fn key_text(key: &Value) -> String {
    key.as_str().unwrap_or_default().to_string()
}

/// The tool list's schema for a write tool: `request_key` is a required
/// property (INVARIANT 1). A read-only tool's is left as it is.
pub(crate) fn add_key_to_schema(tool: &mut rmcp::model::Tool) {
    if !writes(&tool.name) {
        return;
    }
    let mut schema: Map<String, Value> = (*tool.input_schema).clone();
    let props = schema
        .entry("properties")
        .or_insert_with(|| Value::Object(Map::new()));
    if let Value::Object(props) = props {
        props.insert(
            KEY_FIELD.into(),
            json!({
                "type": "string",
                "minLength": MIN_KEY_LEN,
                "maxLength": MAX_KEY_LEN,
                "description": KEY_WORDS,
            }),
        );
    }
    let required = schema
        .entry("required")
        .or_insert_with(|| Value::Array(Vec::new()));
    if let Value::Array(required) = required
        && !required.iter().any(|r| r == KEY_FIELD)
    {
        required.push(json!(KEY_FIELD));
    }
    tool.input_schema = std::sync::Arc::new(schema);
}

/// What opening a call came to.
pub(crate) enum Begun {
    /// The call is open: run the tool.
    Run,
    /// Not run: this is the answer, and the access record's code.
    Answer(Box<CallToolResponse>, &'static str),
}

fn refusal(text: String) -> CallToolResponse {
    CallToolResponse::Complete(CallToolResult::error(vec![
        rmcp::model::ContentBlock::text(text),
    ]))
}

/// Open a call (INVARIANTS 3, 5): refuse it if the books take no calls;
/// answer it from the books if its key already committed; otherwise open
/// it. Takes the store's lock for the moment it needs; the caller holds
/// the write turn (`SharedStore::write_turn`).
pub(crate) fn begin(server: &BooksServer, id: CallId) -> Result<Begun, String> {
    let mut store = server.lock()?;
    if let Err(e) = store.ensure_takes_calls() {
        return Ok(Begun::Answer(
            Box::new(refusal(e.to_string())),
            UPGRADE_PENDING,
        ));
    }
    if let Some(kept) = store.recorded_call(&id.token, &id.key) {
        if kept.tool == id.tool && kept.args == id.args {
            return match serde_json::from_str::<CallToolResult>(&kept.answer) {
                Ok(answer) => Ok(Begun::Answer(
                    Box::new(CallToolResponse::Complete(answer)),
                    REPEAT,
                )),
                Err(e) => Err(format!(
                    "the answer kept for this request_key can't be read back: {e}"
                )),
            };
        }
        return Ok(Begun::Answer(
            Box::new(refusal(format!(
                "this request_key was already used for {} with other arguments, so the call was not run and nothing was changed. A key names one action: send this one again with a key of its own",
                if kept.tool == id.tool {
                    "the same tool".to_string()
                } else {
                    kept.tool.clone()
                }
            ))),
            KEY_REUSED,
        ));
    }
    store.begin_call(id).map_err(|e| e.to_string())?;
    Ok(Begun::Run)
}

/// The text of the first text block, or "".
fn first_text(r: &CallToolResult) -> &str {
    r.content
        .iter()
        .find_map(|c| match c {
            rmcp::model::ContentBlock::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .unwrap_or("")
}

/// Whether a tool's final answer is one that committed: a result that is
/// not an error, and the call not stopped by a close (INVARIANT 4).
pub(crate) fn answered_ok(
    out: &Result<CallToolResponse, rmcp::ErrorData>,
    stopped_by_close: bool,
) -> bool {
    !stopped_by_close
        && matches!(out, Ok(CallToolResponse::Complete(r)) if r.is_error != Some(true))
}

/// Close the open call with how it came out (INVARIANTS 4, 6). An error
/// here means the call could not be closed: the books server stops, and
/// the caller is told so.
pub(crate) fn close(
    server: &BooksServer,
    out: &Result<CallToolResponse, rmcp::ErrorData>,
    ok: bool,
) -> Result<(), String> {
    use ledger_core::parties::{has_full_tin, redact_full_tins};
    let mut store = server.lock()?;
    if !store.in_call() {
        return Ok(());
    }
    let masked = |text: String| {
        if has_full_tin(&text) {
            redact_full_tins(&text).into_owned()
        } else {
            text
        }
    };
    let ended = if ok {
        let Ok(CallToolResponse::Complete(r)) = out else {
            return Err("a call that committed has no answer".into());
        };
        // Kept as given: no tool's answer holds a full TIN (the three that
        // can are no transaction), and a digest of the books' head can hold
        // nine digits in a row, which masking would spoil on a repeat.
        let text = serde_json::to_string(r).map_err(|e| e.to_string())?;
        store.close_call(CallEnd::Done(&text))
    } else {
        let why = match out {
            Ok(CallToolResponse::Complete(r)) => first_text(r).to_string(),
            Ok(_) => "the call did not complete".to_string(),
            Err(e) => e.message.to_string(),
        };
        let why = if why.is_empty() {
            "the call failed".to_string()
        } else {
            why
        };
        store.close_call(CallEnd::Failed(&masked(why)))
    };
    ended.map_err(|e| e.to_string())
}

/// What a call is told when its closing line could not be written.
pub(crate) const NOT_CLOSED: &str = "this call could not be completed: its closing line could not be written, so none of it counts, and the books server stopped taking requests to protect the books. Ask the human to restart it, then send the call again with the same request_key";
