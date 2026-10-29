//! Audits and attestations over the wire (plan P8; rules.md B-45, D-9):
//! a deterministic run and workpaper, claims weighed against the bar when
//! asked, the CPA package that waits for a counted audit, and books that
//! have not declared `audits`.
//!
//! Expected behavior comes from `docs/mcp-interface.md` ("Tools — audit
//! and attestations") and rules.md B-45. Every call goes through a real
//! books server as an agent reaches it (`common.rs`).

mod common;

use common::{Books, answer_of};
use serde_json::{Value, json};

/// Cash at 1000, the default 25.00 receipt threshold.
const SETTINGS: &str = "[company]\nname = \"Audit Co\"\nfiscal_year_start = \"01-01\"\n\
[books]\ncash_accounts = [\"1000\"]\n";

fn chart(w: &mut common::Wire) {
    for (code, name, ty) in [
        ("1000", "Cash", "asset"),
        ("3000", "Capital", "equity"),
        ("4000", "Sales", "revenue"),
        ("6000", "Supplies", "expense"),
    ] {
        answer_of(&w.call(
            "books.add_account",
            json!({"code": code, "name": name, "account_type": ty}),
        ))
        .unwrap();
    }
}

/// A posted entry of `amount` supplies paid from cash on `date`.
fn post(w: &mut common::Wire, date: &str, what: &str, amount: &str) -> u64 {
    let r = answer_of(&w.call(
        "books.draft_entry",
        json!({"date": date, "description": what, "no_party_reason": "test"}),
    ))
    .unwrap();
    let id = r["entry_id"].as_u64().unwrap();
    for (account, side) in [("6000", "debit"), ("1000", "credit")] {
        answer_of(&w.call(
            "books.add_line",
            json!({"entry_id": id, "account": account, "side": side, "amount": amount}),
        ))
        .unwrap();
    }
    id
}

fn receipt(w: &mut common::Wire, entry: u64, text: &str) -> u64 {
    let upload = w.upload(text.as_bytes(), "receipt.txt", "text/plain", "receipt");
    answer_of(&w.call(
        "books.attach_receipt",
        json!({"entry_id": entry, "upload_id": upload}),
    ))
    .unwrap()["attachment_id"]
        .as_u64()
        .unwrap()
}

fn confirm_post(w: &mut common::Wire, id: u64) {
    let r = answer_of(&w.call(
        "books.post_entry",
        json!({"entry_id": id, "confirmed": true}),
    ))
    .unwrap();
    assert_eq!(r["status"], "posted", "{r}");
}

/// Capital in, a 40.00 purchase with its receipt, and a 60.00 one with
/// none. Returns (receipted entry, its receipt, the bare entry).
fn setup(tag: &str) -> (Books, common::Wire, u64, u64, u64) {
    let books = Books::with(tag, "USD", Some(SETTINGS));
    let mut w = books.session("p8");
    chart(&mut w);
    let cap = answer_of(&w.call(
        "books.draft_entry",
        json!({"date": "2026-03-01", "description": "Capital", "no_party_reason": "test"}),
    ))
    .unwrap()["entry_id"]
        .as_u64()
        .unwrap();
    for (account, side) in [("1000", "debit"), ("3000", "credit")] {
        answer_of(&w.call(
            "books.add_line",
            json!({"entry_id": cap, "account": account, "side": side, "amount": "1000.00"}),
        ))
        .unwrap();
    }
    receipt(&mut w, cap, "Opening deposit 1000.00 2026-03-01");
    confirm_post(&mut w, cap);
    let paper = post(&mut w, "2026-03-05", "Printer paper", "40.00");
    let rid = receipt(&mut w, paper, "Printer paper 40.00 2026-03-05");
    confirm_post(&mut w, paper);
    // The 60.00 purchase is posted while the books' receipt threshold was
    // higher, as it could have been before the rule was tightened.
    set_threshold(&mut w, "1000.00");
    let bare = post(&mut w, "2026-03-07", "Toner", "60.00");
    confirm_post(&mut w, bare);
    set_threshold(&mut w, "25.00");
    (books, w, paper, rid, bare)
}

fn set_threshold(w: &mut common::Wire, amount: &str) {
    answer_of(&w.call(
        "books.set_setting",
        json!({"decided_by": "Avery Quill",
               "changes": [{"key": "policy.evidence.threshold", "value": amount}]}),
    ))
    .unwrap();
}

fn audit(w: &mut common::Wire, scope: &str, through: &str) -> Value {
    answer_of(&w.call(
        "books.audit",
        json!({"scope": scope, "through": through, "confirmed": true}),
    ))
    .unwrap_or_else(|e| panic!("books.audit: {e}"))
}

fn codes(out: &Value) -> Vec<String> {
    out["findings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["code"].as_str().unwrap().to_string())
        .collect()
}

/// The same books, day and scope give the same run, findings and
/// workpaper; asking again files and journals nothing; an entry makes it a
/// new audit.
#[test]
fn audit_is_deterministic() {
    let (books, mut w, _, _, bare) = setup("p8-deterministic");
    let preview = answer_of(&w.call(
        "books.audit",
        json!({"scope": "full", "through": "2026-03-31"}),
    ))
    .unwrap();
    assert_eq!(preview["status"], "preview");
    assert!(preview["run"].is_null());
    assert!(
        preview["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["code"] == "evidence_missing")
    );
    let first = audit(&mut w, "full", "2026-03-31");
    assert_eq!(first["status"], "audited", "{first}");
    assert!(
        codes(&first).contains(&"evidence_missing".to_string()),
        "the bare 60.00 entry has no receipt: {first}"
    );
    let run = first["run"].clone();
    assert_eq!(run["at_current_head"], true);
    let journal = books.journal();
    let again = audit(&mut w, "full", "2026-03-31");
    assert_eq!(again["status"], "already_run");
    assert_eq!(again["run"]["id"], run["id"]);
    assert_eq!(again["run"]["findings_sha256"], run["findings_sha256"]);
    assert_eq!(again["findings"], first["findings"]);
    assert_eq!(books.journal(), journal, "asking again journals nothing");
    // The workpaper is the same twice, JSON and PDF.
    let id = run["id"].clone();
    let one = answer_of(&w.call("books.workpaper", json!({"id": id}))).unwrap();
    let two = answer_of(&w.call("books.workpaper", json!({"id": id}))).unwrap();
    assert_eq!(one["workpaper"], two["workpaper"]);
    assert_eq!(one["pdf"]["sha256"], two["pdf"]["sha256"]);
    assert_eq!(one["workpaper"]["findings_sha256"], run["findings_sha256"]);
    let pdf = w.fetch(&one["pdf"]);
    assert!(pdf.starts_with(b"%PDF"));
    // The audit fixes nothing and moves no figure; an entry makes it stale.
    let tb = answer_of(&w.call("books.trial_balance", json!({"as_of": "2026-03-31"}))).unwrap();
    assert_eq!(tb["balanced"], true);
    let extra = post(&mut w, "2026-03-09", "Stamps", "10.00");
    confirm_post(&mut w, extra);
    let third = audit(&mut w, "full", "2026-03-31");
    assert_eq!(third["status"], "audited");
    assert_ne!(third["run"]["id"], run["id"]);
    let _ = bare;
}

/// A reading that differs from the entry never counts; one that agrees
/// counts only at the bar, and the bar is read when asked.
#[test]
fn attestations_count_against_the_bar_when_asked() {
    let (_books, mut w, paper, rid, _) = setup("p8-bar");
    let good = answer_of(&w.call(
        "books.attest_receipt",
        json!({"attachment_id": rid, "entry_id": paper, "amount": "40.00", "date": "2026-03-05",
               "method": {"model": "opus", "reasoning_effort": "high"}}),
    ))
    .unwrap();
    assert_eq!(good["attestation"]["counted"], true, "{good}");
    assert_eq!(good["attestation"]["by"], "test");
    let wrong = answer_of(&w.call(
        "books.attest_receipt",
        json!({"attachment_id": rid, "entry_id": paper, "amount": "41.00", "date": "2026-03-05",
               "method": {"reasoning_effort": "max", "adversarial": true, "verifier_agents": 5}}),
    ))
    .unwrap();
    assert_eq!(wrong["attestation"]["disagrees"], true);
    assert_eq!(wrong["attestation"]["counted"], false);
    assert!(
        wrong["why_not_counted"]
            .as_str()
            .unwrap()
            .contains("differs")
    );
    let low = answer_of(&w.call(
        "books.review_entry",
        json!({"entry_id": paper, "method": {"reasoning_effort": "medium"}}),
    ))
    .unwrap();
    assert_eq!(low["attestation"]["counted"], false);
    assert!(low["why_not_counted"].as_str().unwrap().contains("high"));
    let none = answer_of(&w.call("books.review_entry", json!({"entry_id": paper}))).unwrap();
    assert_eq!(none["attestation"]["counted"], false, "no claim, no count");
    let listed = answer_of(&w.call("books.attestations", json!({}))).unwrap();
    assert_eq!(listed["attestations"].as_array().unwrap().len(), 4);
    assert_eq!(listed["counted"], 1);
    assert_eq!(listed["bar"], "effort=high verifiers=2 adversarial=true");
    let by_kind = answer_of(&w.call("books.attestations", json!({"kind": "entry"}))).unwrap();
    assert_eq!(by_kind["attestations"].as_array().unwrap().len(), 2);
    // The bar is read when asked: lowered, the medium review counts.
    answer_of(&w.call(
        "books.set_setting",
        json!({"decided_by": "Avery Quill",
               "changes": [{"key": "policy.attestation_bar", "value": "effort=medium"}]}),
    ))
    .unwrap();
    let after = answer_of(&w.call("books.attestations", json!({"kind": "entry"}))).unwrap();
    assert_eq!(after["bar"], "effort=medium verifiers=2 adversarial=true");
    assert_eq!(after["attestations"][0]["counted"], true);
    assert_eq!(after["attestations"][1]["counted"], false);
    // Refused: bad method words, an unposted entry.
    let bad = w.call(
        "books.review_entry",
        json!({"entry_id": paper, "method": {"reasoning_effort": "wild"}}),
    );
    assert!(answer_of(&bad).unwrap_err().contains("reasoning_effort"));
}

/// A period is attested only on an audit that still stands.
#[test]
fn a_period_is_attested_on_a_standing_audit() {
    let (_books, mut w, paper, _, _) = setup("p8-period");
    answer_of(&w.call(
        "books.create_period",
        json!({"name": "2026-03", "start": "2026-03-01", "end": "2026-03-31"}),
    ))
    .unwrap();
    let method = json!({"reasoning_effort": "high"});
    let none = w.call(
        "books.attest_period",
        json!({"period": "2026-03", "method": method}),
    );
    assert!(answer_of(&none).unwrap_err().contains("books.audit"));
    let run = audit(&mut w, "month", "2026-03-31");
    let ok = answer_of(&w.call(
        "books.attest_period",
        json!({"period": "2026-03", "method": method}),
    ))
    .unwrap();
    assert_eq!(ok["attestation"]["audit_run_id"], run["run"]["id"]);
    assert_eq!(
        ok["attestation"]["counted"], true,
        "a month needs the effort only"
    );
    // A change since the audit: refused until it is run again.
    let extra = post(&mut w, "2026-03-20", "Stamps", "5.00");
    confirm_post(&mut w, extra);
    let stale = w.call(
        "books.attest_period",
        json!({"period": "2026-03", "method": method}),
    );
    assert!(answer_of(&stale).unwrap_err().contains("books.audit"));
    let listing = answer_of(&w.call("books.workpaper", json!({"id": run["run"]["id"]}))).unwrap();
    assert_eq!(listing["run"]["at_current_head"], false);
    let _ = paper;
}

/// D-9: the CPA package waits for a counted audit for the handoff; a
/// reason overrides it and is journaled as a decision; the bar is the
/// whole bar for a year.
#[test]
fn cpa_package_refuses_without_a_counted_audit_unless_a_reason_is_given() {
    let (_books, mut w, paper, rid, bare) = setup("p8-cpa");
    // Unset, the package warns until the books hold an audit run; the
    // owner who wants it refused from the start says so.
    answer_of(&w.call(
        "books.set_setting",
        json!({"decided_by": "Avery Quill",
               "changes": [{"key": "policy.cpa_package_requires_audit", "value": "refuse"}]}),
    ))
    .unwrap();
    let export = |w: &mut common::Wire, extra: Value| {
        let mut args = json!({"as_of": "2026-03-31", "cash_accounts": ["1000"],
                              "include_receipts": false, "include_tins": false});
        for (k, v) in extra.as_object().unwrap() {
            args[k] = v.clone();
        }
        w.call("books.export_cpa_package", args)
    };
    let refused = export(&mut w, json!({}));
    let err: Value = serde_json::from_str(&answer_of(&refused).unwrap_err()).unwrap();
    assert_eq!(err["code"], "audit_required", "{err}");
    assert_eq!(err["remedy_tool"], "books.audit");
    // A reason lets it go, and is a decision in the journal.
    let before = answer_of(&w.call("books.decisions", json!({}))).unwrap();
    answer_of(&export(
        &mut w,
        json!({"without_audit_reason": "  the CPA asked for it early "}),
    ))
    .unwrap();
    let after = answer_of(&w.call("books.decisions", json!({}))).unwrap();
    assert_eq!(
        after["decisions"].as_array().unwrap().len(),
        before["decisions"].as_array().unwrap().len() + 1
    );
    assert!(after.to_string().contains("without a counted audit"));
    // An audit alone does not open it; nor one attested below the bar.
    answer_of(&w.call(
        "books.create_period",
        json!({"name": "2026-03", "start": "2026-03-01", "end": "2026-03-31"}),
    ))
    .unwrap();
    audit(&mut w, "cpa_handoff", "2026-03-31");
    assert!(answer_of(&export(&mut w, json!({}))).is_err());
    answer_of(&w.call(
        "books.attest_period",
        json!({"period": "2026-03", "method": {"reasoning_effort": "high"}}),
    ))
    .unwrap();
    assert!(
        answer_of(&export(&mut w, json!({}))).is_err(),
        "a year-level audit needs the adversarial check and the verifiers"
    );
    answer_of(&w.call(
        "books.attest_period",
        json!({"period": "2026-03", "method": {"reasoning_effort": "high",
               "adversarial": true, "verifier_agents": 2}}),
    ))
    .unwrap();
    let made = answer_of(&export(&mut w, json!({}))).unwrap();
    assert!(made["file"]["sha256"].is_string(), "{made}");
    // A change since: the audit no longer stands, and the package waits again.
    let extra = post(&mut w, "2026-03-20", "Stamps", "5.00");
    confirm_post(&mut w, extra);
    assert!(answer_of(&export(&mut w, json!({}))).is_err());
    // Under warn the package is made and says so.
    answer_of(&w.call(
        "books.set_setting",
        json!({"decided_by": "Avery Quill",
               "changes": [{"key": "policy.cpa_package_requires_audit", "value": "warn"}]}),
    ))
    .unwrap();
    let warned = answer_of(&export(&mut w, json!({}))).unwrap();
    assert!(
        warned["warnings"]
            .to_string()
            .contains("No counted audit stands behind this package"),
        "{warned}"
    );
    let _ = (paper, rid, bare);
}

/// 8.0.6: books the version before P8 wrote (the fixture `p8_before`)
/// open with no upgrade; the audit's preview works, its run and every
/// attestation are refused with format_upgrade_pending, nothing is
/// journaled, and the CPA package is not held up by an audit it cannot ask
/// for.
#[test]
fn audits_wait_for_the_upgrade() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../ledger-store/tests/fixtures/old-books/p8_before");
    let only = tempfile::tempdir().unwrap();
    std::fs::copy(src.join("journal.jsonl"), only.path().join("journal.jsonl")).unwrap();
    common::copy_private(&src.join("blobs"), &only.path().join("blobs"));
    let books = Books::placed("p8-waits", only.path());
    let mut w = books.session("p8");
    let preview = answer_of(&w.call(
        "books.audit",
        json!({"scope": "full", "through": "2026-02-28"}),
    ))
    .unwrap();
    assert_eq!(preview["status"], "preview");
    let journal = books.journal();
    for (tool, args) in [
        (
            "books.audit",
            json!({"scope": "full", "through": "2026-02-28", "confirmed": true}),
        ),
        (
            "books.review_entry",
            json!({"entry_id": 1, "method": {"reasoning_effort": "high"}}),
        ),
        (
            "books.attest_receipt",
            json!({"attachment_id": 1, "entry_id": 1, "amount": "1000.00",
                   "date": "2026-01-01"}),
        ),
    ] {
        let e = answer_of(&w.call(tool, args)).unwrap_err();
        assert!(e.starts_with("format_upgrade_pending"), "{tool}: {e}");
    }
    assert_eq!(books.journal(), journal, "a refusal journals nothing");
    // The package is held up only by what it always was: an open draft.
    let args = json!({"as_of": "2026-01-31", "cash_accounts": ["1000"],
                      "include_receipts": false, "include_tins": false});
    let made = answer_of(&w.call("books.export_cpa_package", args)).unwrap();
    assert!(made["file"]["sha256"].is_string(), "{made}");
    let listed = answer_of(&w.call("books.attestations", json!({}))).unwrap();
    assert!(listed["attestations"].as_array().unwrap().is_empty());
}
