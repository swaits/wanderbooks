//! Agent evals (plan P5 step 7): scripted transcripts, no model. Each is a
//! JSON file in `agent_evals/`, played against a real books server through
//! the wire as an agent reaches it. The rule they enforce is the point of
//! the result envelope: **an agent that begins with `books.start` can do
//! its work by calling only what the previous result's `next_steps` named.**
//!
//! A transcript is `{ "setup": [..], "settings": "<toml>", "steps": [..] }`.
//! `setup` calls are made directly (the chart and the books an agent finds
//! when it arrives; they are not the agent's moves). A step is one of:
//!
//! - `{"call": "books.start", "args": {..}}`: only the first step, and
//!   only `books.start`.
//! - `{"follow": "books.x", "with": {..}}`: the previous result's
//!   `next_steps` must name tool `books.x`; the call is made with that
//!   step's `args` and `with` laid over them (what the agent decides: an
//!   amount, a date).
//! - `{"follow_prompt": "onboard"}`: the previous result's `next_steps`
//!   must name that prompt; it is fetched with `prompts/get`.
//! - `{"expect_next_steps_contains": "books.x"}` and
//!   `{"expect": {"/json/pointer": value}}` on the previous result.
//!
//! In `with` and `args`, `{"$upload": {"name", "mime", "text"}}` is the
//! upload_id of that file, uploaded first as an agent does, and
//! `{"$var": "x"}` is a value a setup call saved with `"save": {"x":
//! "/pointer"}`.
//!
//! Expected behavior comes from `docs/mcp-interface.md` ("The result
//! envelope") and plan 4.6.

mod common;

use common::{Books, Wire, answer_of};
use serde_json::{Map, Value, json};
use std::collections::HashMap;

/// A value with `$upload` and `$var` laid in.
fn resolve(w: &mut Wire, vars: &HashMap<String, Value>, v: &Value) -> Value {
    match v {
        Value::Object(o) if o.len() == 1 && o.contains_key("$upload") => {
            let u = &o["$upload"];
            let text = u["text"].as_str().expect("an upload has text");
            json!(w.upload(
                text.as_bytes(),
                u["name"].as_str().unwrap_or("file.pdf"),
                u["mime"].as_str().unwrap_or("application/pdf"),
                u["purpose"].as_str().unwrap_or("receipt"),
            ))
        }
        Value::Object(o) if o.len() == 1 && o.contains_key("$var") => vars
            .get(o["$var"].as_str().unwrap())
            .unwrap_or_else(|| panic!("no saved value {}", o["$var"]))
            .clone(),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, x)| (k.clone(), resolve(w, vars, x)))
                .collect(),
        ),
        Value::Array(a) => Value::Array(a.iter().map(|x| resolve(w, vars, x)).collect()),
        other => other.clone(),
    }
}

/// Play one transcript; returns the tools it called after `books.start`.
fn play(name: &str, script: &Value) -> Vec<String> {
    let books = Books::with(&format!("eval-{name}"), "USD", script["settings"].as_str());
    let mut w = books.session("agent-evals");
    let mut vars: HashMap<String, Value> = HashMap::new();
    for s in script["setup"].as_array().into_iter().flatten() {
        let args = resolve(&mut w, &vars, &s["args"]);
        let tool = s["call"].as_str().unwrap();
        let out =
            answer_of(&w.call(tool, args)).unwrap_or_else(|e| panic!("{name}: setup {tool}: {e}"));
        for (var, pointer) in s["save"].as_object().into_iter().flatten() {
            vars.insert(
                var.clone(),
                out.pointer(pointer.as_str().unwrap())
                    .unwrap_or_else(|| panic!("{name}: setup {tool} has no {pointer}: {out}"))
                    .clone(),
            );
        }
    }
    let mut last: Option<Value> = None;
    let mut called = Vec::new();
    for (i, step) in script["steps"]
        .as_array()
        .expect("steps")
        .iter()
        .enumerate()
    {
        let here = format!("{name} step {i}");
        if let Some(tool) = step["call"].as_str() {
            assert_eq!(
                i, 0,
                "{here}: only the first step may call without following"
            );
            assert_eq!(tool, "books.start", "{here}: the first call is books.start");
            let args = resolve(&mut w, &vars, &step["args"]);
            last =
                Some(answer_of(&w.call_raw(tool, args)).unwrap_or_else(|e| panic!("{here}: {e}")));
        } else if let Some(want) = step["follow"].as_str() {
            let prev = last
                .as_ref()
                .unwrap_or_else(|| panic!("{here}: follow before start"));
            let offered = prev["next_steps"].as_array().cloned().unwrap_or_default();
            let named = offered
                .iter()
                .find(|s| s["tool"] == json!(want))
                .unwrap_or_else(|| {
                    panic!(
                        "{here}: {want} is not in the previous result's next_steps: {:?}",
                        offered
                            .iter()
                            .map(|s| s["tool"].clone())
                            .collect::<Vec<_>>()
                    )
                });
            let mut args: Map<String, Value> =
                named["args"].as_object().cloned().unwrap_or_default();
            for (k, v) in step["with"].as_object().into_iter().flatten() {
                args.insert(k.clone(), resolve(&mut w, &vars, v));
            }
            let out = answer_of(&w.call_raw(want, Value::Object(args)))
                .unwrap_or_else(|e| panic!("{here}: {want} was refused: {e}"));
            called.push(want.to_string());
            last = Some(out);
        } else if let Some(want) = step["follow_prompt"].as_str() {
            let prev = last
                .as_ref()
                .unwrap_or_else(|| panic!("{here}: follow before start"));
            assert!(
                prev["next_steps"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|s| s["prompt"] == json!(want)),
                "{here}: prompt {want} is not in next_steps"
            );
            let got = w.rpc("prompts/get", json!({"name": want, "arguments": {}}));
            let text = got["result"]["messages"][0]["content"]["text"]
                .as_str()
                .unwrap_or_else(|| panic!("{here}: the prompt did not render: {got}"));
            assert!(text.contains("books."), "{here}: {text}");
        } else if let Some(tool) = step["expect_next_steps_contains"].as_str() {
            let prev = last.as_ref().unwrap();
            assert!(
                prev["next_steps"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|s| s["tool"] == json!(tool)),
                "{here}: next_steps lack {tool}: {}",
                prev["next_steps"]
            );
        } else if let Some(expect) = step["expect"].as_object() {
            let prev = last.as_ref().unwrap();
            for (pointer, want) in expect {
                assert_eq!(
                    prev.pointer(pointer),
                    Some(want),
                    "{here}: {pointer} in {prev}"
                );
            }
        } else {
            panic!("{here}: not a step: {step}");
        }
    }
    called
}

/// Every transcript follows only what it was told, and ends where it says.
#[test]
fn agent_evals_follow_only_next_steps() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/agent_evals");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    let names: Vec<String> = files
        .iter()
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        names,
        [
            "draft_awaiting_a_receipt",
            "month_end_close",
            "onboarding_to_first_post",
            "receipt_backed_post",
            "reconciliation",
            "reversal"
        ],
        "the six evals the plan names"
    );
    for (name, path) in names.iter().zip(&files) {
        let script: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        let called = play(name, &script);
        assert!(!called.is_empty(), "{name} calls nothing");
    }
}
