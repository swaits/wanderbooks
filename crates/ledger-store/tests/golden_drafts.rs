//! Golden file for draft states, discard and the one evidence gate
//! (`docs/parity.md`: "Draft states and discard" and "One evidence gate").
//!
//! Rules: BS-entry-5 (a draft waits on its evidence; one that
//! will never post can be discarded), D-47 (a reversal needs no receipt of
//! its own), BS-period-2 (a period with an open draft cannot close).
//!
//! It is a `Store` fixture, not a bare `Ledger`, because the gate's second
//! half (is a receipt on file) lives in the store. Every expected value is
//! computed by hand in
//! `plan/golden-worksheets/draft-states-and-evidence-gate.md`.

use chrono::NaiveDate;
use ledger_core::evidence::requirement;
use ledger_core::reports::trial_balance_as_of;
use ledger_core::{AccountType, Currency, DraftState, Money, Side};
use ledger_store::{Store, StoreError};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::path::PathBuf;

fn jan(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 1, day).unwrap()
}

/// Exact cents: `usd(1250)` is 12.50.
fn usd(cents: i64) -> Money {
    Money::new(Decimal::new(cents, 2), Currency::USD)
}

/// A test's dir, removed when the test ends, pass or panic.
struct TestDir(PathBuf);

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn test_dir(name: &str) -> TestDir {
    let dir = std::env::temp_dir().join(format!("wanderbooks-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TestDir(dir)
}

/// A draft of `cents` from `debit` to `credit`.
fn draft(
    s: &mut Store,
    date: NaiveDate,
    what: &str,
    (debit, credit): (u64, u64),
    cents: i64,
) -> u64 {
    let id = s.create_entry(date, what).unwrap();
    s.add_line(id, debit, Side::Debit, usd(cents)).unwrap();
    s.add_line(id, credit, Side::Credit, usd(cents)).unwrap();
    id
}

/// File a receipt (distinct bytes: identical bytes are one file) on `entry`.
fn receipt(s: &mut Store, entry: u64) {
    s.attach_receipt(
        Some(entry),
        "receipt.pdf",
        "application/pdf",
        format!("%PDF-1.4 receipt for entry {entry}").as_bytes(),
        None,
    )
    .unwrap();
}

/// One draft as the viewer lists it: its state and the gate's decision.
fn draft_record(s: &Store, id: u64) -> Value {
    let ledger = s.ledger();
    let e = ledger.get_entry(id).unwrap();
    let note = match e.draft_state() {
        DraftState::Building => Value::Null,
        DraftState::AwaitingEvidence { note } => json!(note),
    };
    json!({
        "entry_id": id,
        "date": e.date().to_string(),
        "description": e.description(),
        "debits": ledger_core::evidence::Policy::entry_debits(e).unwrap().to_string(),
        "state": e.draft_state().key(),
        "note": note,
        "receipts": s.list_attachments(Some(id)).unwrap().len(),
        "requirement": serde_json::to_value(requirement(ledger, e, ledger.settings())).unwrap(),
    })
}

fn drafts(s: &Store) -> Vec<Value> {
    s.ledger()
        .drafts()
        .iter()
        .map(|e| draft_record(s, e.id()))
        .collect()
}

fn trial_balance(s: &Store) -> Value {
    serde_json::to_value(trial_balance_as_of(s.ledger(), jan(31))).unwrap()
}

/// By-hand worksheet: `plan/golden-worksheets/draft-states-and-evidence-gate.md`.
#[test]
fn golden_draft_states_and_evidence_gate() {
    let dir = test_dir("golden-drafts");
    let mut s = Store::create(&dir.0, Currency::USD).unwrap();
    let cash = s
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let equity = s
        .add_account("3000", "Owner's Equity", AccountType::Equity, None, false)
        .unwrap();
    let software = s
        .add_account("6100", "Software", AccountType::Expense, None, false)
        .unwrap();

    // 1: the seed, with its receipt, posts through the gate.
    let seed = draft(&mut s, jan(2), "Seed", (cash, equity), 50_000);
    receipt(&mut s, seed);
    s.post_entry_checked(seed).unwrap();

    // 2..5: four drafts. 2 waits for its invoice; 5 already has its receipt.
    let figma = draft(
        &mut s,
        jan(10),
        "Figma annual plan",
        (software, cash),
        4_000,
    );
    let dup = draft(
        &mut s,
        jan(11),
        "Duplicate hosting invoice",
        (software, cash),
        6_000,
    );
    let supplies = draft(&mut s, jan(15), "Office supplies", (software, cash), 1_250);
    let hosting = draft(&mut s, jan(20), "Hosting", (software, cash), 10_000);
    assert_eq!((figma, dup, supplies, hosting), (2, 3, 4, 5));
    s.set_draft_state(
        figma,
        DraftState::AwaitingEvidence {
            note: "invoice due from Figma".into(),
        },
    )
    .unwrap();
    receipt(&mut s, hosting);
    let drafts_open = drafts(&s);

    // The gate refuses 2 and 3 (required, no receipt), journaling nothing.
    let mut gate_refusals = Vec::new();
    for id in [figma, dup] {
        let head = s.head();
        let refused = match s.post_entry_checked(id) {
            Err(StoreError::ReceiptRequired(info)) => {
                assert_eq!(info.entry_id, id);
                "receipt_required"
            }
            other => panic!("expected ReceiptRequired, got {other:?}"),
        };
        gate_refusals.push(json!({
            "entry_id": id,
            "refused": refused,
            "journal_head_unchanged": s.head() == head,
        }));
    }

    // 4 (under the threshold) and 5 (receipt on file) post; 5 is reversed
    // with no receipt of its own; 2 posts once its receipt is filed.
    s.post_entry_checked(supplies).unwrap();
    s.post_entry_checked(hosting).unwrap();
    let reversal = s
        .reverse_entry(hosting, jan(25), "Reverse hosting (billed in error)")
        .unwrap();
    assert_eq!(reversal, 6);
    receipt(&mut s, figma);
    s.post_entry_checked(figma).unwrap();

    // A period that draft 3 holds open.
    let period = s.create_period("2026-01", jan(1), jan(31)).unwrap();
    let close_refused = match s.close_period(period) {
        Err(StoreError::Ledger(ledger_core::LedgerError::DraftsInPeriod(p))) => {
            json!({"refused": "drafts_in_period", "period_id": p})
        }
        other => panic!("expected DraftsInPeriod, got {other:?}"),
    };
    // Only a draft can be discarded.
    let discard_posted = json!({
        "entry_id": seed,
        "refused": s.discard_draft(seed, "oops").is_err(),
    });

    let before_discard = json!({
        "drafts": drafts(&s),
        "next_entry_id": s.ledger().next_entry_id(),
        "trial_balance": trial_balance(&s),
    });
    s.discard_draft(dup, "entered twice").unwrap();
    let after_tb = trial_balance(&s);
    let after_discard = json!({
        "drafts": drafts(&s),
        "next_entry_id": s.ledger().next_entry_id(),
        "trial_balance": after_tb,
        "trial_balance_equals_before": after_tb == before_discard["trial_balance"],
    });

    // The close is accepted now.
    let closing = s
        .close_period(period)
        .unwrap()
        .expect("a loss is swept, so there is a closing entry");
    let closing_entry = s.ledger().get_entry(closing).unwrap().clone();
    let close_after_discard = json!({
        "closing_entry": closing,
        "debits": ledger_core::evidence::Policy::entry_debits(&closing_entry).unwrap().to_string(),
        "receipts": s.list_attachments(Some(closing)).unwrap().len(),
        "requirement": serde_json::to_value(
            requirement(s.ledger(), &closing_entry, s.ledger().settings())).unwrap(),
    });

    // Every entry that posted, as the gate sees it now (not 3: discarded).
    let posted_entries: Vec<Value> = [seed, figma, supplies, hosting, reversal]
        .iter()
        .map(|&id| {
            let e = s.ledger().get_entry(id).unwrap();
            json!({
                "entry_id": id,
                "description": e.description(),
                "reversal_of": e.reversal_of(),
                "debits": ledger_core::evidence::Policy::entry_debits(e).unwrap().to_string(),
                "receipts": s.list_attachments(Some(id)).unwrap().len(),
                "requirement": serde_json::to_value(
                    requirement(s.ledger(), e, s.ledger().settings())).unwrap(),
            })
        })
        .collect();

    // The journal's draft lines: a state and a discard, with their reasons.
    let draft_journal_lines: Vec<Value> =
        std::fs::read_to_string(dir.0.join(ledger_store::JOURNAL_FILE))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .filter(|l| {
                l["type"]
                    .as_str()
                    .is_some_and(|t| t.starts_with("entry.draft_"))
            })
            .map(|l| {
                let p = &l["payload"];
                match l["type"].as_str().unwrap() {
                    "entry.draft_state_set" => json!({
                        "type": l["type"], "id": p["id"], "state": p["state"], "note": p["note"],
                    }),
                    _ => json!({"type": l["type"], "id": p["id"], "reason": p["reason"]}),
                }
            })
            .collect();

    s.verify().unwrap();
    let got = json!({
        "drafts_open": drafts_open,
        "gate_refusals": gate_refusals,
        "posted_entries": posted_entries,
        "close_refused_with_draft": close_refused,
        "discard_of_posted_refused": discard_posted,
        "before_discard": before_discard,
        "draft_journal_lines": draft_journal_lines,
        "after_discard": after_discard,
        "close_after_discard": close_after_discard,
        "after_close": {"trial_balance": trial_balance(&s)},
    });
    let want: Value =
        serde_json::from_str(include_str!("golden/draft_states_and_evidence_gate.json")).unwrap();
    assert_eq!(
        got,
        want,
        "got:\n{}",
        serde_json::to_string_pretty(&got).unwrap()
    );
}
