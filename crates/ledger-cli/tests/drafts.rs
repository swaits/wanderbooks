//! Drafts, the evidence gate and the preview digest over the wire (plan
//! P4; rules.md B-43, D-47, D-50): a draft waits for its receipt and
//! posts when it arrives, a discard unblocks a close, a report and the CPA
//! package say when drafts are open, a reversal needs no receipt of its
//! own, and every tool with `confirmed` takes the `preview` it returned
//! and refuses a stale one.
//!
//! Expected behavior comes from `docs/mcp-interface.md` ("Tools — drafts",
//! "The preview digest") and rules.md B-43. Every call goes through a real
//! books server as an agent reaches it (`common.rs`).

mod common;

use common::{Books, answer_of};
use serde_json::{Value, json};

/// Cash at 1000, the default 25.00 receipt threshold.
const SETTINGS: &str = "[company]\nname = \"Drafts Co\"\nfiscal_year_start = \"01-01\"\n\
[books]\ncash_accounts = [\"1000\"]\n";

fn chart(w: &mut common::Wire) {
    for (code, name, ty) in [
        ("1000", "Cash", "asset"),
        ("3000", "Capital", "equity"),
        ("4000", "Sales", "revenue"),
        ("6000", "Rent", "expense"),
    ] {
        answer_of(&w.call(
            "books.add_account",
            json!({"code": code, "name": name, "account_type": ty}),
        ))
        .unwrap();
    }
}

/// A draft of `amount` rent paid from cash on `date`.
fn draft(w: &mut common::Wire, date: &str, what: &str, amount: &str) -> u64 {
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

fn a_receipt(w: &mut common::Wire, entry: u64) -> u64 {
    let bytes = b"%PDF-1.4 a receipt";
    let upload = w.upload(bytes, "receipt.pdf", "application/pdf", "receipt");
    answer_of(&w.call(
        "books.attach_receipt",
        json!({"entry_id": entry, "upload_id": upload}),
    ))
    .unwrap()["attachment_id"]
        .as_u64()
        .unwrap()
}

fn setup(tag: &str) -> (Books, common::Wire) {
    let books = Books::with(tag, "USD", Some(SETTINGS));
    let mut w = books.session("p4");
    chart(&mut w);
    (books, w)
}

/// The refusal a call met, parsed when it is structured.
fn refusal(r: &Value) -> Value {
    let text = answer_of(r).expect_err("expected a refusal");
    serde_json::from_str(&text).unwrap_or(Value::String(text))
}

/// A draft finished but waiting for its receipt says so, is listed as
/// missing its evidence, is refused at the post, and posts the moment the
/// receipt is attached.
#[test]
fn a_draft_waits_for_its_receipt_and_posts() {
    let (_books, mut w) = setup("p4-waits");
    let id = draft(&mut w, "2026-03-01", "Figma annual plan", "100.00");
    let set = answer_of(&w.call(
        "books.set_draft_state",
        json!({"entry_id": id, "state": "awaiting_evidence", "note": "Figma invoice due"}),
    ))
    .unwrap();
    assert_eq!(set["state"], "awaiting_evidence");
    let listed = answer_of(&w.call("books.list_drafts", json!({}))).unwrap();
    let d = &listed["drafts"][0];
    assert_eq!(d["entry_id"], id);
    assert_eq!(d["state"], "awaiting_evidence");
    assert_eq!(d["note"], "Figma invoice due");
    assert_eq!(d["total"], "100.00");
    assert_eq!(d["missing_evidence"], true);
    let waiting = answer_of(&w.call(
        "books.list_drafts",
        json!({"state": "awaiting_evidence", "as_of": "2026-03-31"}),
    ))
    .unwrap();
    assert_eq!(waiting["drafts"].as_array().unwrap().len(), 1);
    let building = answer_of(&w.call("books.list_drafts", json!({"state": "building"}))).unwrap();
    assert!(building["drafts"].as_array().unwrap().is_empty());
    // The preview names the need; the confirm is refused with the error the
    // MCP always gave.
    let p = answer_of(&w.call("books.preview_entry", json!({"entry_id": id}))).unwrap();
    assert_eq!(p["status"], "preview");
    assert_eq!(p["receipt_required"], true);
    assert_eq!(p["receipt_missing"], true);
    let digest = p["preview"].as_str().unwrap().to_string();
    let r = w.call(
        "books.post_entry",
        json!({"entry_id": id, "confirmed": true, "preview": digest}),
    );
    let err = refusal(&r);
    assert_eq!(err["code"], "receipt_required", "{err}");
    // The receipt arrives; the same draft posts.
    a_receipt(&mut w, id);
    let r = w.call(
        "books.post_entry",
        json!({"entry_id": id, "confirmed": true}),
    );
    let posted = answer_of(&r).unwrap();
    assert_eq!(posted["status"], "posted");
    let listed = answer_of(&w.call("books.list_drafts", json!({}))).unwrap();
    assert!(listed["drafts"].as_array().unwrap().is_empty());
}

/// B-43: the structured `receipt_required` error is the one the MCP built
/// before the gate moved into the store.
#[test]
fn the_gate_moved_and_the_mcp_error_is_unchanged() {
    let (_books, mut w) = setup("p4-error");
    let id = draft(&mut w, "2026-03-01", "Rent", "100.00");
    let r = w.call(
        "books.post_entry",
        json!({"entry_id": id, "confirmed": true}),
    );
    let err = refusal(&r);
    assert_eq!(err["code"], "receipt_required");
    assert_eq!(err["entry_id"], id);
    assert_eq!(err["debits"], "100.00");
    assert_eq!(
        err["policy"],
        "receipts required when entry debits are strictly greater than 25.00"
    );
    assert_eq!(err["remedy_tool"], "books.attach_receipt");
    assert!(
        err["message"]
            .as_str()
            .unwrap()
            .starts_with(&format!("entry {id} moves 100.00 in debits ("))
    );
    assert!(
        err["remedy_hint"]
            .as_str()
            .unwrap()
            .contains("then retry books.post_entry with confirmed=true")
    );
    assert!(err.get("split_group").is_none());
}

/// D-47: a reversal posts at once with no receipt of its own; its evidence
/// is its original's.
#[test]
fn a_reversal_needs_no_receipt_of_its_own() {
    let (_books, mut w) = setup("p4-reversal");
    let id = draft(&mut w, "2026-03-01", "Rent", "100.00");
    a_receipt(&mut w, id);
    answer_of(&w.call(
        "books.post_entry",
        json!({"entry_id": id, "confirmed": true}),
    ))
    .unwrap();
    let r = answer_of(&w.call(
        "books.reverse_entry",
        json!({"entry_id": id, "date": "2026-03-05", "description": "Undo", "confirmed": true}),
    ))
    .unwrap();
    assert_eq!(r["status"], "posted");
    let d = answer_of(&w.call("books.digest", json!({"since": "2026-03-01"}))).unwrap();
    for e in d["entries"].as_array().unwrap() {
        if e["reversal_of"].is_number() {
            assert_eq!(e["receipt_required"], false, "{e}");
            assert_eq!(e["receipt_missing"], false, "{e}");
        }
    }
}

/// A period holding an open draft cannot close; discarded (previewed, then
/// confirmed with its digest), it can.
#[test]
fn discard_unblocks_the_close_over_the_wire() {
    let (_books, mut w) = setup("p4-discard");
    let seed = draft(&mut w, "2026-03-01", "Seed", "10.00");
    answer_of(&w.call(
        "books.post_entry",
        json!({"entry_id": seed, "confirmed": true}),
    ))
    .unwrap();
    let per = answer_of(&w.call(
        "books.create_period",
        json!({"name": "2026-03", "start": "2026-03-01", "end": "2026-03-31"}),
    ))
    .unwrap()["period_id"]
        .as_u64()
        .unwrap();
    let id = draft(&mut w, "2026-03-20", "Entered twice", "40.00");
    let r = w.call(
        "books.close_period",
        json!({"period_id": per, "confirmed": true}),
    );
    assert!(
        answer_of(&r).is_err(),
        "a close is refused with a draft open"
    );
    let p = answer_of(&w.call(
        "books.discard_draft",
        json!({"entry_id": id, "reason": "entered twice"}),
    ))
    .unwrap();
    assert_eq!(p["status"], "preview");
    assert_eq!(p["holds_open"], json!(["2026-03"]));
    let digest = p["preview"].as_str().unwrap().to_string();
    // A reason is the preview's; a different one is a different preview.
    let r = w.call_raw(
        "books.discard_draft",
        json!({"entry_id": id, "reason": "something else", "confirmed": true, "preview": digest}),
    );
    assert_eq!(refusal(&r)["code"], "stale_preview");
    let done = answer_of(&w.call_raw(
        "books.discard_draft",
        json!({"entry_id": id, "reason": "entered twice", "confirmed": true, "preview": digest}),
    ))
    .unwrap();
    assert_eq!(done["status"], "discarded");
    assert!(
        answer_of(&w.call("books.list_drafts", json!({}))).unwrap()["drafts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let closed = answer_of(&w.call(
        "books.close_period",
        json!({"period_id": per, "confirmed": true}),
    ))
    .unwrap();
    assert_eq!(closed["status"], "closed");
    // Only a draft: the posted seed is never discarded.
    let r = w.call(
        "books.discard_draft",
        json!({"entry_id": seed, "reason": "no"}),
    );
    assert!(answer_of(&r).unwrap_err().contains("posted"));
}

/// A report says what is open as of its end; strict refuses instead.
#[test]
fn report_warns_on_draft_before_as_of() {
    let (_books, mut w) = setup("p4-report");
    let id = draft(&mut w, "2026-03-10", "Open", "40.00");
    let tb = answer_of(&w.call("books.trial_balance", json!({"as_of": "2026-03-31"}))).unwrap();
    let open = &tb["as_of_status"]["drafts_open_on_or_before"];
    assert_eq!(open[0]["entry_id"], id);
    assert_eq!(open[0]["state"], "building");
    let early = answer_of(&w.call("books.trial_balance", json!({"as_of": "2026-03-01"}))).unwrap();
    assert!(
        early["as_of_status"]["drafts_open_on_or_before"]
            .as_array()
            .unwrap()
            .is_empty(),
        "a draft dated after the report's end is not open in it"
    );
    for tool in [
        "books.trial_balance",
        "books.balance_sheet",
        "books.profit_and_loss",
        "books.cash_flow",
    ] {
        let args = match tool {
            "books.trial_balance" | "books.balance_sheet" => {
                json!({"as_of": "2026-03-31", "strict": true})
            }
            _ => json!({"start": "2026-03-01", "end": "2026-03-31", "strict": true}),
        };
        let r = w.call(tool, args);
        let why = answer_of(&r).expect_err(tool);
        assert!(
            why.contains("strict") && why.contains("draft"),
            "{tool}: {why}"
        );
    }
    // Posted, the report is clean and strict gives it.
    a_receipt(&mut w, id);
    answer_of(&w.call(
        "books.post_entry",
        json!({"entry_id": id, "confirmed": true}),
    ))
    .unwrap();
    let ok = answer_of(&w.call(
        "books.trial_balance",
        json!({"as_of": "2026-03-31", "strict": true}),
    ));
    // Cash is not reconciled, so strict still names it: the refusal says
    // so and not "draft".
    if let Err(why) = ok {
        assert!(!why.contains("draft"), "{why}");
    }
}

/// The CPA package is refused while a draft dated in it is open, unless
/// the human says why; the reason is journaled as a decision.
#[test]
fn cpa_package_refuses_open_draft() {
    let (books, mut w) = setup("p4-cpa");
    let id = draft(&mut w, "2026-03-10", "Open", "40.00");
    let args = json!({"as_of": "2026-03-31", "cash_accounts": ["1000"]});
    let err = refusal(&w.call("books.export_cpa_package", args.clone()));
    assert_eq!(err["code"], "open_drafts", "{err}");
    assert!(err["message"].as_str().unwrap().contains(&format!("#{id}")));
    // A draft dated after as_of does not stop it.
    let before = books.journal().len();
    let mut allowed = args.clone();
    allowed["allow_open_drafts"] = json!({"reason": "the CPA wants it before the receipt comes"});
    let out = answer_of(&w.call("books.export_cpa_package", allowed)).unwrap();
    assert!(out["file"].is_object(), "{out}");
    let journal = String::from_utf8(books.journal()).unwrap();
    assert!(books.journal().len() > before);
    assert!(journal.contains("decision.recorded"));
    assert!(journal.contains("the CPA wants it before the receipt comes"));
}

/// D-50: a confirm takes the preview it was shown; without it, or after a
/// changed argument or a write by another session, it is refused with
/// `stale_preview` before anything is written.
#[test]
fn stale_preview_refused() {
    let (books, mut w) = setup("p4-stale");
    let id = draft(&mut w, "2026-03-01", "Rent", "10.00");
    // A draft's preview is the draft's state: a line added since is stale.
    let p = answer_of(&w.call("books.preview_entry", json!({"entry_id": id}))).unwrap();
    let digest = p["preview"].as_str().unwrap().to_string();
    // The same preview from post_entry itself.
    let p2 = answer_of(&w.call("books.post_entry", json!({"entry_id": id}))).unwrap();
    assert_eq!(p2["preview"], p["preview"]);
    // No preview at all.
    let r = w.call_raw(
        "books.post_entry",
        json!({"entry_id": id, "confirmed": true}),
    );
    assert_eq!(refusal(&r)["code"], "stale_preview");
    // A line added since the preview.
    answer_of(&w.call(
        "books.add_line",
        json!({"entry_id": id, "account": "6000", "side": "debit", "amount": "1.00"}),
    ))
    .unwrap();
    answer_of(&w.call(
        "books.add_line",
        json!({"entry_id": id, "account": "1000", "side": "credit", "amount": "1.00"}),
    ))
    .unwrap();
    let before = books.journal();
    let r = w.call_raw(
        "books.post_entry",
        json!({"entry_id": id, "confirmed": true, "preview": digest}),
    );
    let err = refusal(&r);
    assert_eq!(err["code"], "stale_preview");
    assert_eq!(err["remedy_tool"], "books.post_entry");
    assert_eq!(books.journal(), before, "a stale confirm writes nothing");
    // An operation's preview moves with the journal's head: a post by
    // another session in between forces a re-preview.
    let other = draft(&mut w, "2026-03-02", "Other", "5.00");
    let first = answer_of(&w.call(
        "books.reverse_entry",
        json!({"entry_id": other, "date": "2026-03-03", "description": "x"}),
    ));
    assert!(first.is_err(), "a draft is not reversible");
    answer_of(&w.call(
        "books.post_entry",
        json!({"entry_id": other, "confirmed": true}),
    ))
    .unwrap();
    let p = answer_of(&w.call(
        "books.reverse_entry",
        json!({"entry_id": other, "date": "2026-03-03", "description": "x"}),
    ))
    .unwrap();
    let digest = p["preview"].as_str().unwrap().to_string();
    // The same call with another description is another preview...
    let r = w.call_raw(
        "books.reverse_entry",
        json!({"entry_id": other, "date": "2026-03-03", "description": "y",
               "confirmed": true, "preview": digest}),
    );
    assert_eq!(refusal(&r)["code"], "stale_preview");
    // ...and so is a write between preview and confirm.
    answer_of(&w.call(
        "books.add_account",
        json!({"code": "6100", "name": "Software", "account_type": "expense"}),
    ))
    .unwrap();
    let r = w.call_raw(
        "books.reverse_entry",
        json!({"entry_id": other, "date": "2026-03-03", "description": "x",
               "confirmed": true, "preview": digest}),
    );
    assert_eq!(refusal(&r)["code"], "stale_preview");
    // Re-previewed, it confirms.
    let r = w.call(
        "books.reverse_entry",
        json!({"entry_id": other, "date": "2026-03-03", "description": "x",
               "confirmed": true}),
    );
    assert_eq!(answer_of(&r).unwrap()["status"], "posted");
}

/// Every tool that takes `confirmed` takes `preview` (D-50, from P4 on).
#[test]
fn every_confirm_tool_takes_a_preview() {
    let books = Books::new("p4-schema");
    let mut w = books.session("p4");
    let tools = w.rpc("tools/list", json!({}))["result"]["tools"].clone();
    let mut confirm = Vec::new();
    for t in tools.as_array().expect("tools") {
        let props = &t["inputSchema"]["properties"];
        if props.get("confirmed").is_some() {
            confirm.push(t["name"].as_str().unwrap().to_string());
            assert!(
                props.get("preview").is_some(),
                "{} takes confirmed but not preview",
                t["name"]
            );
        }
    }
    for tool in [
        "books.post_entry",
        "books.reverse_entry",
        "books.run_schedules",
        "books.dispose_asset",
        "books.reconcile",
        "books.close_period",
        "books.create_invoice",
        "books.create_bill",
        "books.receive_payment",
        "books.pay_bill",
        "books.write_off_invoice",
        "books.discard_draft",
    ] {
        assert!(confirm.iter().any(|t| t == tool), "{tool} has no confirmed");
    }
}

/// Defect 6: an entry dated in the past but posted just now is in the
/// digest asked for by the journal sequence number, and not in the one
/// asked for by date.
#[test]
fn digest_since_seq_is_posting_time() {
    let (_books, mut w) = setup("p4-digest");
    let early = draft(&mut w, "2026-01-05", "Backdated", "20.00");
    let head =
        answer_of(&w.call("books.digest", json!({"since": "2026-01-01"}))).unwrap()["head_seq"]
            .as_u64()
            .unwrap();
    answer_of(&w.call(
        "books.post_entry",
        json!({"entry_id": early, "confirmed": true}),
    ))
    .unwrap();
    let by_seq = answer_of(&w.call("books.digest", json!({"since_seq": head}))).unwrap();
    let ids: Vec<u64> = by_seq["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["entry_id"].as_u64().unwrap())
        .collect();
    assert_eq!(ids, vec![early]);
    assert!(by_seq["head_seq"].as_u64().unwrap() > head);
    let by_date = answer_of(&w.call("books.digest", json!({"since": "2026-02-01"}))).unwrap();
    assert!(by_date["entries"].as_array().unwrap().is_empty());
    // Nothing after the new head.
    let none =
        answer_of(&w.call("books.digest", json!({"since_seq": by_seq["head_seq"]}))).unwrap();
    assert!(none["entries"].as_array().unwrap().is_empty());
    // An account touched whose net movement is zero is listed, not lost.
    let second = draft(&mut w, "2026-01-06", "Refund", "20.00");
    // (reverse the sign on the same accounts: debit cash, credit rent)
    answer_of(&w.call(
        "books.post_entry",
        json!({"entry_id": second, "confirmed": true}),
    ))
    .unwrap();
    let rev = answer_of(&w.call(
        "books.reverse_entry",
        json!({"entry_id": second, "date": "2026-01-07", "description": "Undo",
               "confirmed": true}),
    ))
    .unwrap();
    assert_eq!(rev["status"], "posted");
    let d = answer_of(&w.call("books.digest", json!({"since_seq": head}))).unwrap();
    let zero: Vec<&str> = d["zero_net_accounts"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        zero.is_empty(),
        "the first entry still moves rent: {zero:?}"
    );
    let only_pair = answer_of(&w.call(
        "books.digest",
        json!({"since": "2026-01-06", "as_of": "2026-01-31"}),
    ))
    .unwrap();
    let zero: Vec<&str> = only_pair["zero_net_accounts"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(zero, vec!["1000 Cash", "6000 Rent"]);
    assert_eq!(only_pair["drafts_by_state"], json!({}));
    assert_eq!(only_pair["never_reconciled"], json!(["1000"]));
}

/// The viewer and the server ask one function whether an entry lacks its
/// evidence (plan P4 step 2, I-5): on random books under three policies
/// the viewer's missing-evidence set equals the server's, and both equal
/// what the viewer decided before the gate moved (an entry's origin read
/// from the journal's events: a hand entry or a recurring draft is
/// covered; a document's, a payment's, a close, a schedule month's, a
/// depreciation's, a payroll's and a reversal are not).
#[cfg(feature = "gui")]
#[test]
fn gui_and_server_gate_agree() {
    use chrono::NaiveDate;
    use ledger_core::settings::{Change, Source};
    use ledger_core::{AccountType, Currency, Money, Side};
    use ledger_gui::data;
    use rust_decimal::Decimal;
    use std::collections::{BTreeMap, BTreeSet};

    let day = |m: u32, d: u32| NaiveDate::from_ymd_opt(2026, m, d).unwrap();
    let usd = |c: i64| Money::new(Decimal::new(c, 2), Currency::USD);
    for (n, (threshold, required, exempt)) in [
        ("0.00", vec![], vec![]),
        ("25.00", vec!["6200"], vec!["6300"]),
        ("100.00", vec![], vec!["6300", "6100"]),
    ]
    .into_iter()
    .enumerate()
    {
        let root = tempfile::Builder::new()
            .prefix(&format!("wb-p4-gate-{n}-"))
            .tempdir()
            .unwrap();
        let dir = root.path().join("book");
        std::fs::create_dir_all(&dir).unwrap();
        let mut st = ledger_store::Store::create(&dir, Currency::USD).unwrap();
        st.change_settings(
            &[
                Change {
                    key: "policy.evidence.threshold".into(),
                    value: Some(json!(threshold)),
                },
                Change {
                    key: "policy.evidence.required_accounts".into(),
                    value: Some(json!(required)),
                },
                Change {
                    key: "policy.evidence.exempt_accounts".into(),
                    value: Some(json!(exempt)),
                },
            ],
            &Source {
                decided_by: "test".into(),
                decided_on: day(1, 1),
                citation: None,
                note: None,
            },
        )
        .unwrap();
        let mut acct =
            |code: &str, name: &str, ty| st.add_account(code, name, ty, None, false).unwrap();
        let bank = acct("1000", "Bank", AccountType::Asset);
        let prepaid = acct("1300", "Prepaid", AccountType::Asset);
        let capital = acct("3000", "Capital", AccountType::Equity);
        let sales = acct("4000", "Sales", AccountType::Revenue);
        let expenses = [
            acct("6100", "Supplies", AccountType::Expense),
            acct("6200", "Contractors", AccountType::Expense),
            acct("6300", "Fees", AccountType::Expense),
        ];
        let seed = st.create_entry(day(1, 1), "Capital").unwrap();
        st.add_line(seed, bank, Side::Debit, usd(10_000_000))
            .unwrap();
        st.add_line(seed, capital, Side::Credit, usd(10_000_000))
            .unwrap();
        st.post_entry(seed).unwrap();
        // A deterministic pseudo-random run of hand entries.
        let mut x: u64 = 0x2545F4914F6CDD1D ^ (n as u64);
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let mut posted = Vec::new();
        for i in 0..36u32 {
            let e = st
                .create_entry(day(2, 1 + (i % 27)), &format!("Entry {i}"))
                .unwrap();
            let cents = 100 + (next() % 30_000) as i64;
            let ex = expenses[(next() % 3) as usize];
            st.add_line(e, ex, Side::Debit, usd(cents)).unwrap();
            st.add_line(e, bank, Side::Credit, usd(cents)).unwrap();
            if next() % 3 == 0 {
                st.attach_receipt(Some(e), "r.pdf", "application/pdf", b"%PDF r", None)
                    .unwrap();
            }
            // Some stay drafts.
            if next() % 6 != 0 {
                st.post_entry(e).unwrap();
                posted.push(e);
            }
        }
        for e in posted.iter().take(5) {
            st.reverse_entry(*e, day(3, 2), "Reversed").unwrap();
        }
        let customer = st.add_customer("Fernhill Labs").unwrap();
        let inv = st
            .create_invoice(
                customer,
                day(3, 3),
                day(4, 2),
                vec![ledger_core::arap::DocumentLine {
                    description: "Design".into(),
                    amount: Decimal::new(250_000, 2),
                    class_id: None,
                    account: None,
                    quantity: None,
                    unit_price: None,
                }],
            )
            .unwrap();
        st.receive_payment(inv, day(3, 20), Decimal::new(100_000, 2), bank)
            .unwrap();
        let sched = st
            .register_schedule(ledger_core::NewSchedule {
                description: "Insurance".into(),
                kind: ledger_core::ScheduleKind::PrepaidExpense,
                total: usd(120_000),
                start: day(1, 1),
                months: 12,
                balance_account: prepaid,
                pl_account: expenses[0],
                convention: ledger_core::ScheduleConvention::MonthlyEven,
            })
            .unwrap();
        st.post_schedule_month(sched, day(1, 31)).unwrap();
        let _ = sales;
        let per = st.create_period("2026-01", day(1, 1), day(1, 31)).unwrap();
        let _ = st.close_period(per);
        st.register_recurring(ledger_core::NewRecurring {
            description: "Figma".into(),
            lines: vec![
                (expenses[0], Side::Debit, usd(4_000)),
                (bank, Side::Credit, usd(4_000)),
            ],
            frequency: ledger_core::Frequency::Monthly,
            start: day(5, 10),
            end: None,
        })
        .unwrap();
        st.run_recurring(day(5, 31)).unwrap();
        // What the server decides: the engine, with the receipts filed.
        let server: BTreeSet<u64> = (1..st.ledger().next_entry_id())
            .filter(|id| {
                let Some(e) = st.ledger().get_entry(*id) else {
                    return false;
                };
                ledger_core::evidence::requirement(st.ledger(), e, st.ledger().settings())
                    .is_required()
                    && st.list_attachments(Some(*id)).unwrap().is_empty()
            })
            .collect();
        // What the viewer decided before: origin from the journal's events.
        let policy = ledger_core::evidence::Policy::from_settings(st.ledger().settings());
        let journal = String::from_utf8(std::fs::read(dir.join("journal.jsonl")).unwrap()).unwrap();
        let mut refs: BTreeMap<u64, Vec<String>> = BTreeMap::new();
        for line in journal.lines() {
            let v: Value = serde_json::from_str(line).unwrap();
            let ty = v["type"].as_str().unwrap().to_string();
            let p = &v["payload"];
            let mut ids = Vec::new();
            if ty.starts_with("entry.") {
                ids.extend(p["id"].as_u64());
            }
            ids.extend(p["entry_id"].as_u64());
            ids.extend(p["closing_entry"].as_u64());
            ids.extend(p["entry"]["id"].as_u64());
            for id in ids {
                refs.entry(id).or_default().push(ty.clone());
            }
        }
        let closings = st.ledger().closing_entry_ids();
        let old: BTreeSet<u64> = (1..st.ledger().next_entry_id())
            .filter(|id| {
                let Some(e) = st.ledger().get_entry(*id) else {
                    return false;
                };
                let has = |t: &str| refs.get(id).is_some_and(|v| v.iter().any(|x| x == t));
                let by_hand = !closings.contains(id)
                    && !has("entry.reversed")
                    && !has("schedule.posted")
                    && !has("asset.depreciation_posted")
                    && !has("asset.disposed")
                    && !has("bill.created")
                    && !has("bill.paid")
                    && !has("invoice.created")
                    && !has("invoice.payment_received")
                    && !has("payroll.run")
                    && !has("payroll.taxes_remitted");
                by_hand
                    && e.reversal_of().is_none()
                    && st.list_attachments(Some(*id)).unwrap().is_empty()
                    && policy.receipt_required(st.ledger(), e)
            })
            .collect();
        drop(st);
        let books = data::load_once(&data::Source {
            data_dir: dir.clone(),
            today: Some(day(6, 1)),
        })
        .unwrap();
        let viewer: BTreeSet<u64> = (1..=books.ledger.next_entry_id())
            .filter(|id| books.entry(*id).is_some_and(|e| e.missing_receipt))
            .collect();
        assert!(!server.is_empty(), "policy {threshold}: nothing to compare");
        assert_eq!(viewer, server, "policy {threshold}: viewer vs server");
        assert_eq!(server, old, "policy {threshold}: engine vs the old reading");
    }
}

/// Books an older version wrote hold a reversal and drafts (the fixture
/// `p4_before`): opened under this version, with no upgrade, the figures
/// are the writer's, the reversal needs no receipt of its own, and the
/// drafts are open and building.
#[test]
fn legacy_reversed_replays_identically() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../ledger-store/tests/fixtures/old-books/p4_before");
    let only = tempfile::tempdir().unwrap();
    std::fs::copy(src.join("journal.jsonl"), only.path().join("journal.jsonl")).unwrap();
    let books = Books::placed("p4-legacy", only.path());
    let mut w = books.session("p4");
    let d = answer_of(&w.call("books.digest", json!({"since": "2026-01-01"}))).unwrap();
    let reversals: Vec<&Value> = d["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["reversal_of"].is_number())
        .collect();
    assert_eq!(reversals.len(), 1);
    assert_eq!(reversals[0]["receipt_required"], false);
    assert_eq!(reversals[0]["receipt_missing"], false);
    let drafts = answer_of(&w.call("books.list_drafts", json!({}))).unwrap();
    let open: Vec<&Value> = drafts["drafts"].as_array().unwrap().iter().collect();
    assert_eq!(open.len(), 3, "{drafts}");
    assert!(open.iter().all(|d| d["state"] == "building"));
    assert_eq!(d["drafts_by_state"], json!({"building": 3}));
    let tb = answer_of(&w.call("books.trial_balance", json!({"as_of": "2026-12-31"}))).unwrap();
    assert_eq!(tb["balanced"], true);
}
