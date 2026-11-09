//! The evidence requirement and the annotation's checks, branch by branch
//! (plan P14 unit V-D2; coverage found `evidence.rs` at 34% of lines from
//! the core's own suite, its callers being the store, the server and the
//! viewer).
//!
//! ```text
//! GOAL: which entry needs evidence, under which rule, and what an
//!       annotation may record, decided by the engine alone and read the
//!       same by every caller.
//! SOURCE: BS-entry-5; WB-entry-5; WB-entry-6; WB-ledger-15;
//!         IRS Pub. 583 (records that support entries).
//! ```
//!
//! Every test goes through the public API: `evidence::{requirement,
//! Policy, Annotation}` and the `Ledger` commands that make an entry of
//! each origin. A requirement is a pure question (WB-entry-5): each test
//! that asks one also checks the books are as they were.

use chrono::NaiveDate;
use ledger_core::evidence::{Annotation, AnnotationFields, Origin, Policy, Requirement};
use ledger_core::settings::{Change, Source};
use ledger_core::{
    AccountId, AccountType, Currency, DocumentLine, EntryId, EntryStatus, Frequency, Ledger,
    LedgerError, Money, NewAsset, NewSchedule, PayrollLine, ScheduleConvention, ScheduleKind, Side,
};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::str::FromStr;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn usd(s: &str) -> Money {
    Money::new(dec(s), Currency::USD)
}

fn src() -> Source {
    Source {
        decided_by: "Owner".into(),
        decided_on: d(2026, 11, 9),
        citation: None,
        note: None,
    }
}

/// Set books settings the way a human does: checked, then applied.
fn set(l: &mut Ledger, pairs: &[(&str, Value)]) {
    let changes: Vec<Change> = pairs
        .iter()
        .map(|(k, v)| Change {
            key: (*k).into(),
            value: Some(v.clone()),
        })
        .collect();
    let made = l.check_settings(&changes, &src()).unwrap();
    l.apply_settings(&made, &src(), 1).unwrap();
}

struct Books {
    l: Ledger,
    cash: AccountId,
    capital: AccountId,
    revenue: AccountId,
    supplies: AccountId,
    meals: AccountId,
}

fn books() -> Books {
    let mut l = Ledger::new(Currency::USD);
    let mut a = |code: &str, name: &str, t| l.add_account(code, name, t, None, false).unwrap();
    let cash = a("1000", "Cash", AccountType::Asset);
    let capital = a("3000", "Capital", AccountType::Equity);
    let revenue = a("4100", "Sales", AccountType::Revenue);
    let supplies = a("6200", "Supplies", AccountType::Expense);
    let meals = a("6300", "Meals", AccountType::Expense);
    Books {
        l,
        cash,
        capital,
        revenue,
        supplies,
        meals,
    }
}

/// A balanced draft: `debit` debited, `credit` credited, `amount` each way.
fn draft(
    l: &mut Ledger,
    date: NaiveDate,
    what: &str,
    debit: AccountId,
    credit: AccountId,
    amount: &str,
) -> EntryId {
    let e = l.create_entry(date, what);
    l.add_line(e, debit, Side::Debit, usd(amount)).unwrap();
    l.add_line(e, credit, Side::Credit, usd(amount)).unwrap();
    e
}

fn post(
    l: &mut Ledger,
    date: NaiveDate,
    what: &str,
    debit: AccountId,
    credit: AccountId,
    amount: &str,
) -> EntryId {
    let e = draft(l, date, what, debit, credit, amount);
    l.post_entry(e).unwrap();
    e
}

fn requirement(l: &Ledger, e: EntryId) -> Requirement {
    let before = l.clone();
    let r = ledger_core::evidence::requirement(l, l.get_entry(e).unwrap(), l.settings());
    assert_eq!(*l, before, "asking is pure (WB-entry-5)");
    r
}

fn required_reason(r: &Requirement) -> &str {
    match r {
        Requirement::Required { reason } => reason,
        Requirement::Exempt { source } => panic!("expected Required, got Exempt: {source}"),
    }
}

fn exempt_source(r: &Requirement) -> &'static str {
    match r {
        Requirement::Exempt { source } => source,
        Requirement::Required { reason } => panic!("expected Exempt, got Required: {reason}"),
    }
}

// ---------------------------------------------------------------- annotations

/// WB-ledger-15 invariant 4: an annotation's fields are a closed list; text is
/// kept trimmed and a blank one is not kept; every field is optional.
#[test]
fn annotation_fields_are_trimmed_and_blank_ones_dropped() {
    let fields = AnnotationFields {
        receipt_number: Some(7),
        payee: Some("  Fernhill Labs  ".into()),
        amount: Some(dec("20.00")),
        date: Some(d(2026, 11, 3)),
        instrument: Some("   ".into()),
        category: Some("\treceipt\n".into()),
        source: Some("".into()),
        reimbursable_to: Some(" Steve ".into()),
    };
    let kept = fields.checked().unwrap();
    assert_eq!(kept.receipt_number, Some(7));
    assert_eq!(kept.payee.as_deref(), Some("Fernhill Labs"));
    assert_eq!(kept.amount, Some(dec("20.00")));
    assert_eq!(kept.date, Some(d(2026, 11, 3)));
    assert_eq!(kept.instrument, None, "a blank text is no fact");
    assert_eq!(kept.category.as_deref(), Some("receipt"));
    assert_eq!(kept.source, None);
    assert_eq!(kept.reimbursable_to.as_deref(), Some("Steve"));
    assert!(!kept.is_empty());
}

/// WB-ledger-15 invariant 4: "nothing recorded" is judged field by field: any
/// one field alone makes the record non-empty, and none makes it empty.
#[test]
fn an_annotation_field_alone_is_a_record() {
    assert!(AnnotationFields::default().is_empty());
    let one = |f: fn(&mut AnnotationFields)| {
        let mut x = AnnotationFields::default();
        f(&mut x);
        x
    };
    let each = [
        one(|x| x.receipt_number = Some(1)),
        one(|x| x.payee = Some("a".into())),
        one(|x| x.amount = Some(Decimal::ZERO)),
        one(|x| x.date = Some(d(2026, 1, 1))),
        one(|x| x.instrument = Some("a".into())),
        one(|x| x.category = Some("a".into())),
        one(|x| x.source = Some("a".into())),
        one(|x| x.reimbursable_to = Some("a".into())),
    ];
    for (i, f) in each.iter().enumerate() {
        assert!(!f.is_empty(), "field {i} alone is a fact");
    }
}

/// WB-ledger-15 invariant 4 and its numbering (the receipt rule's): a
/// receipt number is positive, an amount is not negative, a date is in the
/// books' calendar (0001-01-01..=9999-12-31). A zero amount is a fact (a
/// no-charge receipt).
#[test]
fn an_annotation_refuses_a_zero_number_a_negative_amount_and_a_date_off_the_calendar() {
    let bad = |f: AnnotationFields| f.checked().unwrap_err();
    // The variant, and the fact each refusal states; the sentence around
    // it is the engine's own.
    assert!(matches!(
        bad(AnnotationFields {
            receipt_number: Some(0),
            ..Default::default()
        }),
        LedgerError::InvalidEvidence(m) if m.contains("positive")
    ));
    assert!(matches!(
        bad(AnnotationFields {
            amount: Some(dec("-0.01")),
            ..Default::default()
        }),
        LedgerError::InvalidEvidence(m) if m.contains("negative")
    ));
    let off = NaiveDate::MAX;
    assert_eq!(
        bad(AnnotationFields {
            date: Some(off),
            ..Default::default()
        }),
        LedgerError::DateOutOfRange(off)
    );
    // The edges that are fine.
    let ok = AnnotationFields {
        receipt_number: Some(1),
        amount: Some(Decimal::ZERO),
        date: Some(d(9999, 12, 31)),
        ..Default::default()
    };
    assert_eq!(ok.clone().checked().unwrap(), ok);
}

/// WB-ledger-15 invariant 4: an annotation records at least one field or a
/// note; the note is trimmed, and a blank one is no note.
#[test]
fn an_annotation_needs_a_field_or_a_note() {
    let ann = |fields, note: Option<&str>| Annotation {
        attachment_id: 3,
        fields,
        note: note.map(str::to_string),
    };
    let none = ann(AnnotationFields::default(), None);
    assert_eq!(
        none.checked().unwrap_err(),
        LedgerError::InvalidEvidence("an annotation records at least one field or a note".into())
    );
    // Texts that trim to nothing leave nothing.
    let blank = ann(
        AnnotationFields {
            payee: Some("  ".into()),
            ..Default::default()
        },
        Some(" \n"),
    );
    assert!(matches!(
        blank.checked().unwrap_err(),
        LedgerError::InvalidEvidence(_)
    ));
    // A note alone is enough, and is kept trimmed.
    let noted = ann(AnnotationFields::default(), Some("  faded, read twice  "))
        .checked()
        .unwrap();
    assert_eq!(noted.note.as_deref(), Some("faded, read twice"));
    assert_eq!(noted.attachment_id, 3);
    // A field alone is enough; its error from `AnnotationFields` comes through.
    assert!(
        ann(
            AnnotationFields {
                receipt_number: Some(9),
                ..Default::default()
            },
            None
        )
        .checked()
        .is_ok()
    );
    assert!(
        ann(
            AnnotationFields {
                receipt_number: Some(0),
                ..Default::default()
            },
            Some("note")
        )
        .checked()
        .is_err()
    );
}

// ---------------------------------------------------------------- the policy

/// BS-entry-5: the policy is the books' settings; the default threshold is
/// 25.00, and its words say how strictly.
#[test]
fn the_policy_reads_the_books_settings() {
    let mut b = books();
    let p = Policy::from_settings(b.l.settings());
    assert_eq!(p.threshold, dec("25.00"));
    assert!(p.required_accounts.is_empty() && p.exempt_accounts.is_empty());
    // The description states the threshold and that it is strict (the
    // wording is the engine's own).
    let described = p.describe();
    assert!(
        described.contains("strictly greater") && described.contains("25.00"),
        "{described}"
    );

    set(
        &mut b.l,
        &[
            ("policy.evidence.threshold", json!("0.00")),
            ("policy.evidence.required_accounts", json!(["6300"])),
            ("policy.evidence.exempt_accounts", json!(["1000", "3000"])),
        ],
    );
    let p = Policy::from_settings(b.l.settings());
    assert_eq!(p.threshold, Decimal::ZERO);
    assert_eq!(p.required_accounts, vec!["6300".to_string()]);
    assert_eq!(
        p.exempt_accounts,
        vec!["1000".to_string(), "3000".to_string()]
    );
    assert_eq!(p.describe(), "receipts required on every entry");
}

/// BS-entry-5, "strictly greater" (owner, 2026-10-12): at the threshold no
/// receipt, one cent over, a receipt. A required account needs one at any
/// amount; the exemption holds only when EVERY line is on an exempt account.
#[test]
fn the_threshold_is_strict_and_the_exemption_needs_every_line() {
    let policy = Policy {
        threshold: dec("25.00"),
        required_accounts: vec!["6300".into()],
        exempt_accounts: vec!["1000".into(), "6200".into()],
    };
    assert!(!policy.receipt_required_for(&["6200", "1000"], dec("25.00")));
    assert!(policy.receipt_required_for(&["6100", "1000"], dec("25.01")));
    // Over the threshold but every line exempt: none.
    assert!(!policy.receipt_required_for(&["6200", "1000"], dec("5000.00")));
    // One line off the exempt list brings the policy back.
    assert!(policy.receipt_required_for(&["6200", "1000", "6100"], dec("5000.00")));
    // A required account needs one at a cent, and beats the exemption.
    assert!(policy.receipt_required_for(&["6300", "1000"], dec("0.01")));
    assert!(policy.receipt_required_for(&["6300"], dec("0.00")));
    // An account the chart does not have is a code like another.
    assert!(!policy.receipt_required_for(&[], dec("25.00")));
}

/// BS-entry-5 / WB-entry-5: the answer carries its reason, in words a
/// refusal or a checklist can show; a hand entry under the threshold is
/// Exempt with the policy as its source.
#[test]
fn a_hand_entry_is_required_above_the_threshold_with_its_reason() {
    let mut b = books();
    let big = post(
        &mut b.l,
        d(2026, 11, 2),
        "Printer",
        b.supplies,
        b.cash,
        "40.00",
    );
    let small = post(
        &mut b.l,
        d(2026, 11, 2),
        "Pens",
        b.supplies,
        b.cash,
        "25.00",
    );
    let r = requirement(&b.l, big);
    assert!(r.is_required());
    // The reason carries the figures and the strict comparison (the
    // sentence is the engine's own).
    let why = required_reason(&r);
    assert!(
        why.contains("40.00") && why.contains("25.00") && why.contains("strictly greater"),
        "{why}"
    );
    let r = requirement(&b.l, small);
    assert!(!r.is_required());
    assert!(exempt_source(&r).contains("under the threshold, or on exempt accounts"));
}

/// BS-entry-5: a required account (`policy.evidence.required_accounts`)
/// makes any amount need a receipt, and the reason names the account.
#[test]
fn a_required_account_always_needs_a_receipt() {
    let mut b = books();
    set(
        &mut b.l,
        &[("policy.evidence.required_accounts", json!(["6300"]))],
    );
    let e = post(&mut b.l, d(2026, 11, 2), "Lunch", b.meals, b.cash, "4.50");
    assert_eq!(
        required_reason(&requirement(&b.l, e)),
        "it touches account 6300, which always needs a receipt"
    );
    let f = post(&mut b.l, d(2026, 11, 2), "Pens", b.supplies, b.cash, "4.50");
    assert!(!requirement(&b.l, f).is_required());
}

/// BS-entry-5: threshold 0.00 requires a receipt on everything, except an
/// entry whose every line is on an exempt account (an owner contribution
/// between cash and capital).
#[test]
fn exempt_accounts_exempt_only_whole_entries() {
    let mut b = books();
    set(
        &mut b.l,
        &[
            ("policy.evidence.threshold", json!("0.00")),
            ("policy.evidence.exempt_accounts", json!(["1000", "3000"])),
        ],
    );
    let contribution = post(
        &mut b.l,
        d(2026, 11, 1),
        "Capital",
        b.cash,
        b.capital,
        "9000.00",
    );
    let purchase = post(&mut b.l, d(2026, 11, 2), "Pens", b.supplies, b.cash, "0.50");
    assert!(!requirement(&b.l, contribution).is_required());
    assert!(requirement(&b.l, purchase).is_required());
}

/// BS-entry-4 and BS-entry-5: a draft's lines are as journaled, of any
/// size. Debits past exact arithmetic are refused (TooLargeToCompute) when
/// summed directly and are past any threshold when a requirement is
/// asked: Required, never a panic.
#[test]
fn debits_past_exact_arithmetic_are_refused_by_the_sum_and_past_the_threshold_for_the_gate() {
    let mut b = books();
    let e = b.l.create_entry(d(2026, 11, 2), "Absurd");
    for _ in 0..2 {
        b.l.replay_add_line(
            e,
            b.supplies,
            Side::Debit,
            Money::new(Decimal::MAX, Currency::USD),
        )
        .unwrap();
    }
    let entry = b.l.get_entry(e).unwrap();
    assert!(matches!(
        Policy::entry_debits(entry),
        Err(LedgerError::TooLargeToCompute(_))
    ));
    let r = requirement(&b.l, e);
    assert!(r.is_required());
    let policy = Policy::from_settings(b.l.settings());
    assert!(policy.receipt_required(&b.l, entry));
    // And no split group is reported for it: it is past the threshold alone.
    assert!(policy.split_group(&b.l, e, entry).is_empty());
}

// ---------------------------------------------------------------- split groups

/// WB-entry-6 (an entry "as part of a split group"), the anti-gaming
/// rule: four same-day, same-description, same-debit-account entries of
/// 10.00 are one 40.00 purchase. The draft
/// that brings the group past the threshold needs a receipt and names the
/// group; the ones before it, and a posted entry, do not (a posted entry's
/// decision was made when it posted).
#[test]
fn a_split_purchase_is_one_purchase() {
    let mut b = books();
    let day = d(2026, 11, 4);
    let a = post(&mut b.l, day, "Office  chairs", b.supplies, b.cash, "10.00");
    let c = post(&mut b.l, day, "office CHAIRS", b.supplies, b.cash, "10.00");
    // Two of 10.00 are 20.00: still under 25.00, nothing is flagged.
    let third = draft(&mut b.l, day, "Office chairs", b.supplies, b.cash, "10.00");
    let r = requirement(&b.l, third);
    assert!(
        required_reason(&r).contains("split group"),
        "30.00 together is over 25.00: {r:?}"
    );
    assert!(required_reason(&r).contains(&format!("{a}")));
    assert!(required_reason(&r).contains(&format!("{c}")));
    assert!(
        required_reason(&r).contains("applies to the group total"),
        "the reason says what the policy applies to"
    );
    // Posted entries are not judged again from their neighbours.
    b.l.post_entry(third).unwrap();
    assert!(!requirement(&b.l, a).is_required());
    assert!(!requirement(&b.l, third).is_required());
}

/// WB-entry-6: what is NOT a split group. A different day, a different
/// description, a different debit account, a group still under the
/// threshold and a reversal in the group (reversals are exempt) leave the
/// new draft alone.
#[test]
fn only_the_same_day_description_and_accounts_make_a_split_group() {
    let mut b = books();
    let day = d(2026, 11, 4);
    let first = post(&mut b.l, day, "Chairs", b.supplies, b.cash, "20.00");
    let policy = Policy::from_settings(b.l.settings());

    let other_day = draft(
        &mut b.l,
        d(2026, 11, 5),
        "Chairs",
        b.supplies,
        b.cash,
        "10.00",
    );
    let other_text = draft(&mut b.l, day, "Desks", b.supplies, b.cash, "10.00");
    let other_account = draft(&mut b.l, day, "Chairs", b.meals, b.cash, "10.00");
    for e in [other_day, other_text, other_account] {
        let entry = b.l.get_entry(e).unwrap();
        assert!(policy.split_group(&b.l, e, entry).is_empty());
        assert!(!requirement(&b.l, e).is_required());
    }

    // A reversal is exempt from the policy, so it is not counted: reversing
    // the 20.00 purchase leaves a posted reversal on the same day with the
    // same description that DEBITS cash. A 20.00 draft that also debits
    // cash is not "the same purchase" again, though 20.00 + 20.00 > 25.00.
    let rev = b.l.reverse_entry(first, day, "Chairs").unwrap();
    assert!(b.l.get_entry(rev).unwrap().reversal_of().is_some());
    let refund = draft(&mut b.l, day, "Chairs", b.cash, b.supplies, "20.00");
    let entry = b.l.get_entry(refund).unwrap();
    assert!(policy.split_group(&b.l, refund, entry).is_empty());
    assert!(!requirement(&b.l, refund).is_required());
}

// ---------------------------------------------------------------- the origins

/// WB-entry-6: a reversal is exempt, even of a large hand entry: its
/// evidence is its original's (D-47, D-12).
#[test]
fn a_reversal_of_a_large_entry_needs_no_receipt_of_its_own() {
    let mut b = books();
    let big = post(
        &mut b.l,
        d(2026, 11, 2),
        "Server",
        b.supplies,
        b.cash,
        "5000.00",
    );
    assert!(requirement(&b.l, big).is_required());
    let rev =
        b.l.reverse_entry(big, d(2026, 11, 3), "Server returned")
            .unwrap();
    let src = exempt_source(&requirement(&b.l, rev));
    assert!(src.starts_with("a reversal"), "{src}");
    assert!(src.contains("BS-entry-5"));
}

/// WB-entry-6: a period's closing entry is derived from the books.
#[test]
fn a_closing_entry_is_derived_so_it_needs_no_receipt() {
    let mut b = books();
    post(
        &mut b.l,
        d(2026, 1, 15),
        "Sales",
        b.cash,
        b.revenue,
        "900.00",
    );
    let p =
        b.l.create_period("Q1", d(2026, 1, 1), d(2026, 3, 31))
            .unwrap();
    let close = b.l.close_period(p).unwrap().expect("revenue to close");
    let entry = b.l.get_entry(close).unwrap();
    assert!(
        Policy::entry_debits(entry).unwrap() > dec("25.00"),
        "past the threshold, so only its origin exempts it"
    );
    assert!(exempt_source(&requirement(&b.l, close)).starts_with("a period's closing entry"));
}

/// WB-entry-6: a schedule month's posting has the schedule as its
/// evidence.
#[test]
fn a_schedule_months_posting_has_the_schedule_for_evidence() {
    let mut b = books();
    let prepaid =
        b.l.add_account("1300", "Prepaid", AccountType::Asset, None, false)
            .unwrap();
    post(
        &mut b.l,
        d(2026, 3, 1),
        "Insurance paid",
        prepaid,
        b.cash,
        "1200.00",
    );
    let id =
        b.l.register_schedule(NewSchedule {
            description: "Insurance".into(),
            kind: ScheduleKind::PrepaidExpense,
            total: usd("1200.00"),
            start: d(2026, 3, 1),
            months: 12,
            balance_account: prepaid,
            pl_account: b.supplies,
            convention: ScheduleConvention::MonthlyEven,
        })
        .unwrap();
    let month = b.l.post_schedule_month(id, d(2026, 3, 31)).unwrap();
    assert_eq!(b.l.get_entry(month).unwrap().status(), EntryStatus::Posted);
    assert!(exempt_source(&requirement(&b.l, month)).starts_with("a schedule month's posting"));
}

/// WB-entry-6: a depreciation run's posting has the asset's schedule as
/// its evidence.
#[test]
fn a_depreciation_run_has_the_assets_schedule_for_evidence() {
    let mut b = books();
    let equipment =
        b.l.add_account("1500", "Equipment", AccountType::Asset, None, false)
            .unwrap();
    let accum =
        b.l.add_account(
            "1510",
            "Accumulated Depreciation",
            AccountType::Asset,
            None,
            false,
        )
        .unwrap();
    let depr =
        b.l.add_account("6900", "Depreciation", AccountType::Expense, None, false)
            .unwrap();
    let asset =
        b.l.register_asset(NewAsset {
            description: "Laptop".into(),
            in_service_date: d(2026, 1, 1),
            cost: usd("1200.00"),
            salvage_value: usd("0.00"),
            useful_life_months: 12,
            cost_account: equipment,
            accum_depr_account: accum,
            expense_account: depr,
        })
        .unwrap();
    let run = post(
        &mut b.l,
        d(2026, 1, 31),
        "Depreciation 2026-01",
        depr,
        accum,
        "100.00",
    );
    // Before the run is recorded it is a hand entry over the threshold.
    assert!(requirement(&b.l, run).is_required());
    b.l.record_depreciation_run(asset, "2026-01", run, dec("100.00"))
        .unwrap();
    assert!(exempt_source(&requirement(&b.l, run)).starts_with("a depreciation run"));
}

/// WB-entry-6: an asset's disposal and a payroll tax remittance are named
/// by the origin the ledger was told; a hand entry of the same size is
/// still the policy's.
#[test]
fn disposals_and_remittances_are_exempt_by_their_recorded_origin() {
    let mut b = books();
    let disposal = post(
        &mut b.l,
        d(2026, 6, 1),
        "Sold laptop",
        b.cash,
        b.revenue,
        "600.00",
    );
    let remit = post(
        &mut b.l,
        d(2026, 6, 2),
        "Tax deposit",
        b.supplies,
        b.cash,
        "800.00",
    );
    let hand = post(
        &mut b.l,
        d(2026, 6, 3),
        "Same size",
        b.supplies,
        b.cash,
        "800.00",
    );
    b.l.note_entry_origin(disposal, Origin::AssetDisposal);
    b.l.note_entry_origin(remit, Origin::PayrollRemittance);
    assert!(exempt_source(&requirement(&b.l, disposal)).starts_with("an asset's disposal"));
    assert!(exempt_source(&requirement(&b.l, remit)).starts_with("a payroll tax remittance"));
    assert!(requirement(&b.l, hand).is_required());
}

/// WB-entry-6: an entry a document command posts (an invoice here) and a
/// payroll run carry their own evidence.
#[test]
fn a_document_entry_and_a_payroll_run_carry_their_own_evidence() {
    let mut b = books();
    let customer = b.l.add_customer("Fernhill Labs").unwrap();
    let inv =
        b.l.create_invoice(
            customer,
            d(2026, 11, 2),
            d(2026, 12, 2),
            vec![DocumentLine {
                description: "Consulting".into(),
                amount: dec("500.00"),
                class_id: None,
                account: None,
                quantity: None,
                unit_price: None,
            }],
        )
        .unwrap();
    let inv_entry = b.l.get_invoice(inv).unwrap().entry_id;
    assert!(
        Policy::entry_debits(b.l.get_entry(inv_entry).unwrap()).unwrap() > dec("25.00"),
        "over the threshold, so the document is what exempts it"
    );
    assert!(exempt_source(&requirement(&b.l, inv_entry)).starts_with("a document's entry"));

    let run =
        b.l.run_payroll(
            d(2026, 11, 15),
            vec![PayrollLine {
                employee_name: "Alice".into(),
                employee: None,
                gross_pay: dec("1000.00"),
                federal_withholding: dec("100.00"),
                state_withholding: dec("40.00"),
                ss_employee: dec("62.00"),
                medicare_employee: dec("14.50"),
                ss_employer: dec("62.00"),
                medicare_employer: dec("14.50"),
                other_deductions: Decimal::ZERO,
            }],
            b.cash,
        )
        .unwrap();
    let run_entry = b.l.get_payroll_run(run).unwrap().entry_id;
    assert!(exempt_source(&requirement(&b.l, run_entry)).starts_with("a payroll run"));
}

/// WB-entry-6 closes the list: recurring drafts are NOT exempt ("Every
/// other entry, hand entries and recurring drafts included, is the
/// policy's").
#[test]
fn a_recurring_draft_is_the_policys() {
    let mut b = books();
    let rec =
        b.l.register_recurring(ledger_core::NewRecurring {
            description: "Rent".into(),
            lines: vec![
                (b.supplies, Side::Debit, usd("900.00")),
                (b.cash, Side::Credit, usd("900.00")),
            ],
            frequency: Frequency::Monthly,
            start: d(2026, 11, 1),
            end: None,
        })
        .unwrap();
    let made = b.l.generate_recurring(rec, d(2026, 11, 1)).unwrap();
    assert_eq!(b.l.get_entry(made).unwrap().status(), EntryStatus::Draft);
    assert!(required_reason(&requirement(&b.l, made)).contains("900.00"));
}
