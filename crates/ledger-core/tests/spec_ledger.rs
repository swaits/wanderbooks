//! Spec-derived tests for the `Ledger` command surface.
//!
//! Written from the spec blocks (GOAL / SOURCE / INVARIANTS / SCENARIOS) in
//! the public interface and from `docs/spec/rules.md`, never from the
//! implementation. Every expected value is a literal computed by hand from
//! the spec, or a law (a model kept by the test itself). A test whose name
//! does not already say its oracle says it in its doc comment.
//!
//! Tests marked `#[ignore = "SPEC-DISAGREEMENT: ..."]` were derived from the
//! spec and fail against the code; they are findings, not bugs in the test.

use std::collections::BTreeSet;
use std::str::FromStr;

use chrono::NaiveDate;
use ledger_core::reports::trial_balance;
use ledger_core::{
    AccountId, AccountType, CloseTo, Currency, DocumentLine, EntryId, EntryStatus, Ledger,
    LedgerError, MemberClose, Money, PeriodId, PeriodStatus, Side, trial_balance_as_of,
};
use proptest::prelude::*;
use rust_decimal::Decimal;

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).expect("valid date")
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).expect("valid decimal")
}

fn usd(s: &str) -> Money {
    Money::new(dec(s), Currency::USD)
}

/// A small partnership chart (the Fernhill shape).
struct Books {
    l: Ledger,
    cash: AccountId,
    card: AccountId,
    cap_a: AccountId,
    cap_b: AccountId,
    dist_a: AccountId,
    rev: AccountId,
    exp: AccountId,
    header: AccountId,
}

fn books() -> Books {
    let mut l = Ledger::new(Currency::USD);
    let header = l
        .add_account("1900", "Other Assets", AccountType::Asset, None, true)
        .unwrap();
    let cash = l
        .add_account("1000", "Bluevine Checking", AccountType::Asset, None, false)
        .unwrap();
    let card = l
        .add_account(
            "2100",
            "Due to Shareholders",
            AccountType::Liability,
            None,
            false,
        )
        .unwrap();
    let cap_a = l
        .add_account("3000", "Capital A", AccountType::Equity, None, false)
        .unwrap();
    let cap_b = l
        .add_account("3001", "Capital B", AccountType::Equity, None, false)
        .unwrap();
    let dist_a = l
        .add_account("3002", "Distributions A", AccountType::Equity, None, false)
        .unwrap();
    let rev = l
        .add_account("4000", "App Revenue", AccountType::Revenue, None, false)
        .unwrap();
    let exp = l
        .add_account("6000", "Software", AccountType::Expense, None, false)
        .unwrap();
    Books {
        l,
        cash,
        card,
        cap_a,
        cap_b,
        dist_a,
        rev,
        exp,
        header,
    }
}

/// Draft, add lines, post. Panics on any refusal (setup only).
fn post(l: &mut Ledger, date: NaiveDate, lines: &[(AccountId, Side, &str)]) -> EntryId {
    let id = l.try_create_entry(date, "test entry").unwrap();
    for (a, s, amt) in lines {
        l.add_line(id, *a, *s, usd(amt)).unwrap();
    }
    l.post_entry(id).unwrap();
    id
}

/// Build a draft from lines and try to post it. A refusal at `add_line`
/// counts as well as a refusal at `post_entry` (the spec says the amount
/// rules hold "at post"; it doesn't say whether `add_line` checks them
/// early). Returns the refusal, or `None` if the entry posted.
fn try_post(
    l: &mut Ledger,
    date: NaiveDate,
    lines: &[(AccountId, Side, Money)],
) -> Option<LedgerError> {
    let id = l.try_create_entry(date, "attempt").unwrap();
    for (a, s, m) in lines {
        if let Err(e) = l.add_line(id, *a, *s, *m) {
            return Some(e);
        }
    }
    l.post_entry(id).err()
}

/// [`try_post`] through the replay path, which books written before the
/// minor-unit rule (B-3, 2026-10-13) still take: for the kernel's own
/// limits below a cent.
fn try_replay_post(
    l: &mut Ledger,
    date: NaiveDate,
    lines: &[(AccountId, Side, Money)],
) -> Option<LedgerError> {
    let id = l.try_create_entry(date, "attempt").unwrap();
    for (a, s, m) in lines {
        if let Err(e) = l.replay_add_line(id, *a, *s, *m) {
            return Some(e);
        }
    }
    l.replay_post_entry(id).err()
}

/// Everything a refusal could disturb, as plain comparable data.
#[derive(Clone, Debug, PartialEq)]
struct Observed {
    chart: Vec<serde_json::Value>,
    entries: Vec<serde_json::Value>,
    posted_order: Vec<EntryId>,
    periods: serde_json::Value,
    next_ids: Vec<u64>,
    tb: serde_json::Value,
    entry_vendors: Vec<Option<u64>>,
    cleared: Vec<bool>,
    closing: BTreeSet<EntryId>,
    counts: (usize, usize, usize),
}

fn observe(l: &Ledger) -> Observed {
    let mut chart: Vec<_> = l.chart().all().collect();
    chart.sort_by(|a, b| a.code.cmp(&b.code));
    let chart = chart
        .into_iter()
        .map(|a| serde_json::to_value(a).unwrap())
        .collect();
    let horizon = l.next_entry_id() + 3;
    let entries = (0..horizon)
        .map(|i| serde_json::to_value(l.get_entry(i)).unwrap())
        .collect();
    let entry_vendors = (0..horizon).map(|i| l.entry_vendor(i)).collect();
    let mut cleared = Vec::new();
    for e in l.posted_entries() {
        for i in 0..e.lines().len() + 1 {
            cleared.push(l.is_cleared(e.id(), i));
        }
    }
    Observed {
        chart,
        entries,
        posted_order: l.posted_entries().map(|e| e.id()).collect(),
        periods: serde_json::to_value(l.list_periods()).unwrap(),
        next_ids: vec![
            l.next_account_id(),
            l.next_entry_id(),
            l.next_period_id(),
            l.next_customer_id(),
            l.next_vendor_id(),
            l.next_invoice_id(),
            l.next_bill_id(),
            u64::from(l.next_class_id()),
        ],
        tb: serde_json::to_value(trial_balance(l)).unwrap(),
        entry_vendors,
        cleared,
        closing: l.closing_entry_ids(),
        counts: (
            l.chart().len(),
            l.list_classes().len(),
            l.list_vendors().len(),
        ),
    }
}

/// [`observe`] with the account-id allocator masked out: the burned-id
/// finding (see `a_refused_account_never_uses_up_an_id`) is reported once,
/// so it doesn't hide other differences.
fn observe_but_account_ids(l: &Ledger) -> Observed {
    let mut o = observe(l);
    o.next_ids[0] = 0;
    o
}

fn lines_of(l: &Ledger, id: EntryId) -> Vec<(AccountId, Side, Decimal)> {
    l.get_entry(id)
        .unwrap()
        .lines()
        .iter()
        .map(|x| (x.account, x.side, x.amount.amount))
        .collect()
}

/// Signed movement (debit +, credit -) of one account within one entry,
/// computed from the lines by the test, not by the engine.
fn entry_movement(l: &Ledger, id: EntryId, account: AccountId) -> Decimal {
    lines_of(l, id)
        .into_iter()
        .filter(|(a, _, _)| *a == account)
        .map(|(_, s, amt)| if s == Side::Debit { amt } else { -amt })
        .sum()
}

fn tb_has_code(l: &Ledger, as_of: NaiveDate, code: &str) -> bool {
    trial_balance_as_of(l, as_of)
        .lines
        .iter()
        .any(|x| x.code == code)
}

// ---------------------------------------------------------------------------
// add_account
// ---------------------------------------------------------------------------

/// Oracle: R-4 / B-4: an added account is findable by its code with exactly
/// the type, parent, and placeholder flag it was given.
#[test]
fn an_added_account_is_registered_under_its_code() {
    let mut b = books();
    let n = b.l.chart().len();
    let id =
        b.l.add_account(
            "1010",
            "Petty Cash",
            AccountType::Asset,
            Some(b.cash),
            false,
        )
        .unwrap();
    let a = b.l.chart().find_by_code("1010").unwrap();
    assert_eq!(a.id, id);
    assert_eq!(a.name, "Petty Cash");
    assert_eq!(a.account_type, AccountType::Asset);
    assert_eq!(a.parent, Some(b.cash));
    assert!(!a.placeholder);
    assert_eq!(a.currency, Currency::USD);
    assert_eq!(b.l.chart().len(), n + 1);
}

/// Oracle: add_account SCENARIO 1: an empty code is refused.
#[test]
fn an_empty_account_code_is_refused() {
    let mut b = books();
    let before = observe_but_account_ids(&b.l);
    assert_eq!(
        b.l.add_account("", "Nameless", AccountType::Asset, None, false),
        Err(LedgerError::EmptyAccountCode)
    );
    assert_eq!(observe_but_account_ids(&b.l), before);
}

/// Oracle: add_account SCENARIO 1: a taken code is refused.
#[test]
fn a_duplicate_account_code_is_refused() {
    let mut b = books();
    let before = observe_but_account_ids(&b.l);
    assert_eq!(
        b.l.add_account("1000", "Other Cash", AccountType::Asset, None, false),
        Err(LedgerError::DuplicateAccountCode("1000".into()))
    );
    assert_eq!(observe_but_account_ids(&b.l), before);
}

/// Oracle: add_account SCENARIO 2: an unknown parent is refused.
#[test]
fn an_unknown_parent_account_is_refused() {
    let mut b = books();
    let ghost = 999_999;
    let before = observe_but_account_ids(&b.l);
    assert_eq!(
        b.l.add_account("1011", "Orphan", AccountType::Asset, Some(ghost), false),
        Err(LedgerError::UnknownParentAccount(ghost))
    );
    assert_eq!(observe_but_account_ids(&b.l), before);
}

/// Oracle: the refusal law ("a refusal MUST leave the books unchanged",
/// post_entry INVARIANT 3; create_period: "NEVER ... use up an id") applied
/// to add_account: a refused account never uses up an account id, so the
/// next account gets the id `next_account_id` announced before the refusal.
/// ASSUMED: the law covers add_account (its own spec block doesn't repeat
/// it; ParentTypeMismatch already behaves this way).
#[test]
fn a_refused_account_never_uses_up_an_id() {
    let mut b = books();
    let ghost = 999_999;
    let announced = b.l.next_account_id();
    assert!(
        b.l.add_account("", "x", AccountType::Asset, None, false)
            .is_err()
    );
    assert_eq!(b.l.next_account_id(), announced, "empty code");
    assert!(
        b.l.add_account("1000", "x", AccountType::Asset, None, false)
            .is_err()
    );
    assert_eq!(b.l.next_account_id(), announced, "duplicate code");
    assert!(
        b.l.add_account("1011", "x", AccountType::Asset, Some(ghost), false)
            .is_err()
    );
    assert_eq!(b.l.next_account_id(), announced, "unknown parent");
    let id =
        b.l.add_account("1011", "x", AccountType::Asset, None, false)
            .unwrap();
    assert_eq!(id, announced);
}

/// Oracle: add_account SCENARIO 3 (B-4 DECISION): a child of another type is
/// refused with ParentTypeMismatch and NEVER added.
#[test]
fn a_child_account_has_its_parents_type() {
    let mut b = books();
    let before = observe(&b.l);
    assert_eq!(
        b.l.add_account(
            "1950",
            "Loan",
            AccountType::Liability,
            Some(b.header),
            false
        ),
        Err(LedgerError::ParentTypeMismatch {
            code: "1950".into(),
            parent_code: "1900".into(),
            expected: AccountType::Asset,
            found: AccountType::Liability,
        })
    );
    assert!(b.l.chart().find_by_code("1950").is_none());
    assert_eq!(observe(&b.l), before);
    // Same type under a placeholder header: accepted.
    assert!(
        b.l.add_account("1950", "Deposit", AccountType::Asset, Some(b.header), false)
            .is_ok()
    );
}

// ---------------------------------------------------------------------------
// try_create_entry
// ---------------------------------------------------------------------------

/// Oracle: try_create_entry SCENARIO 2: each new id is one past the last,
/// and is the id `next_entry_id` announced.
#[test]
fn a_new_draft_gets_the_next_id_one_past_the_last() {
    let mut b = books();
    let announced = b.l.next_entry_id();
    let e1 = b.l.try_create_entry(d(2026, 1, 5), "a").unwrap();
    let e2 = b.l.try_create_entry(d(2026, 1, 6), "b").unwrap();
    assert_eq!(e1, announced);
    assert_eq!(e2, e1 + 1);
    let e = b.l.get_entry(e1).unwrap();
    assert_eq!(e.status(), EntryStatus::Draft);
    assert_eq!(e.date(), d(2026, 1, 5));
    assert_eq!(e.description(), "a");
    assert!(e.lines().is_empty());
}

/// Oracle: try_create_entry SCENARIO 1 (B-12): a date inside a closed period,
/// first and last day included, is refused with ClosedPeriod and creates
/// nothing; the day after the period is accepted.
#[test]
fn a_draft_is_never_created_in_a_closed_period() {
    let mut b = books();
    post(
        &mut b.l,
        d(2026, 1, 10),
        &[
            (b.cash, Side::Debit, "10.00"),
            (b.rev, Side::Credit, "10.00"),
        ],
    );
    let jan =
        b.l.create_period("2026-01", d(2026, 1, 1), d(2026, 1, 31))
            .unwrap();
    b.l.close_period(jan).unwrap();
    for day in [d(2026, 1, 1), d(2026, 1, 15), d(2026, 1, 31)] {
        let before = observe(&b.l);
        let r = b.l.try_create_entry(day, "late");
        assert!(
            matches!(r, Err(LedgerError::ClosedPeriod(p, _)) if p == jan),
            "{day}: {r:?}"
        );
        assert_eq!(observe(&b.l), before);
    }
    assert!(b.l.try_create_entry(d(2026, 2, 1), "next month").is_ok());
    assert!(b.l.try_create_entry(d(2025, 12, 31), "prior month").is_ok());
}

// ---------------------------------------------------------------------------
// add_line / add_line_fx
// ---------------------------------------------------------------------------

/// Oracle: JournalEntry::lines doc: lines are kept in the order added, with
/// the account, side, and amount given.
#[test]
fn lines_are_kept_in_the_order_added() {
    let mut b = books();
    let e = b.l.try_create_entry(d(2026, 3, 1), "split").unwrap();
    b.l.add_line(e, b.exp, Side::Debit, usd("12.34")).unwrap();
    b.l.add_line(e, b.cash, Side::Credit, usd("10.00")).unwrap();
    b.l.add_line(e, b.card, Side::Credit, usd("2.34")).unwrap();
    assert_eq!(
        lines_of(&b.l, e),
        vec![
            (b.exp, Side::Debit, dec("12.34")),
            (b.cash, Side::Credit, dec("10.00")),
            (b.card, Side::Credit, dec("2.34")),
        ]
    );
}

/// Oracle: B-1 DECISION (owner, 2026-10-13): a line on an unknown account
/// is refused when added (UnknownAccount), not only at post.
#[test]
fn a_line_on_an_unknown_account_is_refused_when_added() {
    let mut b = books();
    let e = b.l.try_create_entry(d(2026, 3, 1), "x").unwrap();
    let before = observe(&b.l);
    assert_eq!(
        b.l.add_line(e, 424_242, Side::Debit, usd("1.00")),
        Err(LedgerError::UnknownAccount(424_242))
    );
    assert_eq!(observe(&b.l), before);
}

/// Oracle: B-1 DECISION / R-4: a line on a placeholder account is refused
/// when added (PlaceholderAccount).
#[test]
fn a_line_on_a_placeholder_account_is_refused_when_added() {
    let mut b = books();
    let e = b.l.try_create_entry(d(2026, 3, 1), "x").unwrap();
    let before = observe(&b.l);
    let r = b.l.add_line(e, b.header, Side::Debit, usd("1.00"));
    assert!(
        matches!(r, Err(LedgerError::PlaceholderAccount(_))),
        "{r:?}"
    );
    assert_eq!(observe(&b.l), before);
}

/// Oracle: B-3 / R-5 and the add_line doc: a foreign-currency amount is
/// refused, never converted (CurrencyMismatch).
#[test]
fn a_foreign_currency_line_is_refused_not_converted() {
    let mut b = books();
    let e = b.l.try_create_entry(d(2026, 3, 1), "x").unwrap();
    let before = observe(&b.l);
    let r = b.l.add_line(
        e,
        b.exp,
        Side::Debit,
        Money::new(dec("5.00"), Currency::EUR),
    );
    assert!(
        matches!(r, Err(LedgerError::CurrencyMismatch { .. })),
        "{r:?}"
    );
    assert_eq!(observe(&b.l), before);
}

/// Oracle: R-3 / B-8 and the PostedEntryImmutable doc ("add lines"): a posted
/// entry takes no more lines.
#[test]
fn a_posted_entry_takes_no_more_lines() {
    let mut b = books();
    let e = post(
        &mut b.l,
        d(2026, 3, 1),
        &[(b.exp, Side::Debit, "5.00"), (b.cash, Side::Credit, "5.00")],
    );
    let before = observe(&b.l);
    assert_eq!(
        b.l.add_line(e, b.exp, Side::Debit, usd("1.00")),
        Err(LedgerError::PostedEntryImmutable)
    );
    assert_eq!(observe(&b.l), before);
}

/// Oracle: UnknownEntry doc: a line on an entry that does not exist is refused.
#[test]
fn a_line_on_an_unknown_entry_is_refused() {
    let mut b = books();
    let before = observe(&b.l);
    assert_eq!(
        b.l.add_line(777_777, b.exp, Side::Debit, usd("1.00")),
        Err(LedgerError::UnknownEntry(777_777))
    );
    assert_eq!(observe(&b.l), before);
}

/// Oracle: B-3 (ASC 830-20-30-1) and the add_line_fx doc: base = foreign ×
/// rate (100 EUR × 1.10 = 110.00 USD), the original kept; the entry then
/// balances in base currency.
#[test]
fn a_foreign_line_is_measured_at_its_rate_and_keeps_the_original() {
    let mut b = books();
    let e =
        b.l.try_create_entry(d(2026, 3, 1), "Berlin hosting")
            .unwrap();
    b.l.add_line_fx(
        e,
        b.exp,
        Side::Debit,
        Currency::EUR,
        dec("100"),
        dec("1.10"),
    )
    .unwrap();
    b.l.add_line(e, b.cash, Side::Credit, usd("110.00"))
        .unwrap();
    let line = &b.l.get_entry(e).unwrap().lines()[0];
    assert_eq!(line.amount, usd("110.00"));
    assert_eq!(line.amount.currency, Currency::USD);
    assert_eq!(line.foreign_currency, Some(Currency::EUR));
    assert_eq!(line.foreign_amount, Some(dec("100")));
    b.l.post_entry(e).unwrap();
    assert_eq!(b.l.account_balance(b.exp).unwrap(), dec("110.00"));
    assert_eq!(b.l.account_balance(b.cash).unwrap(), dec("-110.00"));
}

/// Oracle: InvalidFxRate doc: a rate that isn't positive is refused, the
/// draft unchanged.
#[test]
fn a_non_positive_fx_rate_is_refused() {
    let mut b = books();
    let e = b.l.try_create_entry(d(2026, 3, 1), "x").unwrap();
    for rate in ["0", "-1.10"] {
        let before = observe(&b.l);
        assert_eq!(
            b.l.add_line_fx(e, b.exp, Side::Debit, Currency::EUR, dec("100"), dec(rate)),
            Err(LedgerError::InvalidFxRate(dec(rate)))
        );
        assert_eq!(observe(&b.l), before);
    }
}

/// Oracle: rules.md B-3 "Checked: fx_overflow_is_an_error" and B-21 (exact
/// amounts): a product that can't be represented is refused, never rounded
/// or wrapped.
#[test]
fn fx_overflow_is_an_error() {
    let mut b = books();
    let e = b.l.try_create_entry(d(2026, 3, 1), "x").unwrap();
    let before = observe(&b.l);
    assert!(
        b.l.add_line_fx(e, b.exp, Side::Debit, Currency::EUR, Decimal::MAX, dec("2"))
            .is_err()
    );
    assert_eq!(observe(&b.l), before);
}

/// Oracle: B-1 DECISION applies to add_line_fx too: unknown account refused
/// when added.
#[test]
fn a_foreign_line_on_an_unknown_account_is_refused_when_added() {
    let mut b = books();
    let e = b.l.try_create_entry(d(2026, 3, 1), "x").unwrap();
    let before = observe(&b.l);
    assert_eq!(
        b.l.add_line_fx(e, 424_242, Side::Debit, Currency::EUR, dec("1"), dec("1.1")),
        Err(LedgerError::UnknownAccount(424_242))
    );
    assert_eq!(observe(&b.l), before);
}

/// Oracle: B-3 + line range (AmountOutOfRange: at most one quadrillion):
/// a foreign line converting to 2,000,000,000,000,000 USD never posts.
#[test]
fn a_foreign_line_beyond_line_range_never_posts() {
    let mut b = books();
    let e = b.l.try_create_entry(d(2026, 3, 1), "x").unwrap();
    let big = dec("1000000000000000");
    let added =
        b.l.add_line_fx(e, b.exp, Side::Debit, Currency::EUR, big, dec("2"));
    if added.is_ok() {
        let _ =
            b.l.add_line_fx(e, b.cash, Side::Credit, Currency::EUR, big, dec("2"));
        assert_eq!(b.l.post_entry(e), Err(LedgerError::AmountOutOfRange));
    }
    assert_eq!(b.l.posted_entries().count(), 0);
}

// ---------------------------------------------------------------------------
// post_entry and JournalEntry::validate
// ---------------------------------------------------------------------------

/// Oracle: post_entry SCENARIO 1: an unknown entry is refused (UnknownEntry).
#[test]
fn posting_an_unknown_entry_is_refused() {
    let mut b = books();
    let before = observe(&b.l);
    assert_eq!(
        b.l.post_entry(31_337),
        Err(LedgerError::UnknownEntry(31_337))
    );
    assert_eq!(observe(&b.l), before);
}

/// Oracle: post_entry SCENARIO 2 (B-8): a posted entry is never posted
/// twice (EntryAlreadyPosted), the log unchanged.
#[test]
fn a_posted_entry_is_never_posted_twice() {
    let mut b = books();
    let e = post(
        &mut b.l,
        d(2026, 3, 1),
        &[(b.exp, Side::Debit, "5.00"), (b.cash, Side::Credit, "5.00")],
    );
    let before = observe(&b.l);
    assert_eq!(b.l.post_entry(e), Err(LedgerError::EntryAlreadyPosted(e)));
    assert_eq!(observe(&b.l), before);
    assert_eq!(b.l.posted_entries().count(), 1);
}

/// Oracle: post_entry SCENARIO 3 (B-12): an entry dated in a closed period
/// is refused (ClosedPeriod), even one drafted through the unchecked
/// constructor.
#[test]
fn an_entry_dated_in_a_closed_period_never_posts() {
    let mut b = books();
    let jan =
        b.l.create_period("2026-01", d(2026, 1, 1), d(2026, 1, 31))
            .unwrap();
    b.l.close_period(jan).unwrap();
    let e = b.l.create_entry(d(2026, 1, 20), "backdated");
    // add_line may itself refuse the closed date; either way it never posts.
    let _ = b.l.add_line(e, b.exp, Side::Debit, usd("5.00"));
    let _ = b.l.add_line(e, b.cash, Side::Credit, usd("5.00"));
    let before = observe(&b.l);
    let r = b.l.post_entry(e);
    assert!(
        matches!(r, Err(LedgerError::ClosedPeriod(p, _)) if p == jan),
        "{r:?}"
    );
    assert_eq!(observe(&b.l), before);
}

/// Oracle: R-1/B-1: fewer than two lines is refused (TooFewLines), for zero
/// lines and for one.
#[test]
fn an_entry_with_fewer_than_two_lines_never_posts() {
    let mut b = books();
    assert_eq!(
        try_post(&mut b.l, d(2026, 3, 1), &[]),
        Some(LedgerError::TooFewLines)
    );
    assert_eq!(
        try_post(
            &mut b.l,
            d(2026, 3, 1),
            &[(b.cash, Side::Debit, usd("1.00"))]
        ),
        Some(LedgerError::TooFewLines)
    );
    assert_eq!(b.l.posted_entries().count(), 0);
}

/// Oracle: R-1/B-1 (no epsilon): 100.00 against 99.99 is refused with the
/// offending totals.
#[test]
fn an_unbalanced_entry_is_refused_with_its_totals() {
    let mut b = books();
    let r = try_post(
        &mut b.l,
        d(2026, 3, 1),
        &[
            (b.exp, Side::Debit, usd("100.00")),
            (b.cash, Side::Credit, usd("99.99")),
        ],
    );
    assert_eq!(
        r,
        Some(LedgerError::UnbalancedEntry {
            debits: dec("100.00"),
            credits: dec("99.99"),
        })
    );
    // One ten-billionth is still a difference (on replayed books; new
    // work refuses anything finer than a cent first).
    let r = try_replay_post(
        &mut b.l,
        d(2026, 3, 1),
        &[
            (b.exp, Side::Debit, usd("1.0000000001")),
            (b.cash, Side::Credit, usd("1")),
        ],
    );
    assert!(
        matches!(r, Some(LedgerError::UnbalancedEntry { .. })),
        "{r:?}"
    );
    assert_eq!(b.l.posted_entries().count(), 0);
}

/// Oracle: R-2 exact decimals: 0.10 + 0.20 credits balance a 0.30 debit
/// exactly (a float would not).
#[test]
fn exact_decimal_sums_balance() {
    let mut b = books();
    let r = try_post(
        &mut b.l,
        d(2026, 3, 1),
        &[
            (b.exp, Side::Debit, usd("0.30")),
            (b.cash, Side::Credit, usd("0.10")),
            (b.card, Side::Credit, usd("0.20")),
        ],
    );
    assert_eq!(r, None);
}

/// Oracle: R-6 / B-2: a zero-amount line is refused (ZeroAmountLine).
#[test]
fn a_zero_amount_line_never_posts() {
    let mut b = books();
    let r = try_post(
        &mut b.l,
        d(2026, 3, 1),
        &[
            (b.exp, Side::Debit, usd("5.00")),
            (b.cash, Side::Credit, usd("5.00")),
            (b.card, Side::Credit, usd("0.00")),
        ],
    );
    assert_eq!(r, Some(LedgerError::ZeroAmountLine));
    assert_eq!(b.l.posted_entries().count(), 0);
}

/// Oracle: B-2: a negative amount is refused (NegativeAmountLine): the side
/// says the direction.
#[test]
fn a_negative_amount_line_never_posts() {
    let mut b = books();
    let r = try_post(
        &mut b.l,
        d(2026, 3, 1),
        &[
            (b.exp, Side::Debit, usd("5.00")),
            (b.cash, Side::Debit, usd("-5.00")),
        ],
    );
    assert_eq!(r, Some(LedgerError::NegativeAmountLine));
    assert_eq!(b.l.posted_entries().count(), 0);
}

/// Oracle: AmountOutOfRange doc ("at most 1000000000000000, ten decimal
/// places"): exactly one quadrillion and exactly ten places post; one cent
/// more, or an eleventh place, never does.
#[test]
fn line_range_is_one_quadrillion_and_ten_places() {
    let mut b = books();
    let ok = |b: &mut Books, amt: &str| {
        try_post(
            &mut b.l,
            d(2026, 3, 1),
            &[
                (b.exp, Side::Debit, usd(amt)),
                (b.cash, Side::Credit, usd(amt)),
            ],
        )
    };
    assert_eq!(ok(&mut b, "1000000000000000"), None);
    assert_eq!(
        ok(&mut b, "1000000000000000.01"),
        Some(LedgerError::AmountOutOfRange)
    );
    // Below a cent, new work is refused first (B-3); replayed books meet
    // the kernel's ten-place limit.
    assert!(matches!(
        ok(&mut b, "0.0000000001"),
        Some(LedgerError::FinerThanMinorUnit { .. })
    ));
    let replayed = |b: &mut Books, amt: &str| {
        try_replay_post(
            &mut b.l,
            d(2026, 3, 1),
            &[
                (b.exp, Side::Debit, usd(amt)),
                (b.cash, Side::Credit, usd(amt)),
            ],
        )
    };
    assert_eq!(replayed(&mut b, "0.0000000001"), None);
    assert_eq!(
        replayed(&mut b, "0.00000000001"),
        Some(LedgerError::AmountOutOfRange)
    );
    assert_eq!(b.l.posted_entries().count(), 2);
}

/// Oracle: post_entry SCENARIO 4 / JournalEntry::validate: a draft carrying
/// an unknown or placeholder account (smuggled in through the replay path)
/// is refused at post with that reason.
#[test]
fn a_draft_with_an_unknown_or_placeholder_account_never_posts() {
    let mut b = books();
    let e = b.l.try_create_entry(d(2026, 3, 1), "x").unwrap();
    b.l.replay_add_line(e, b.cash, Side::Debit, usd("1.00"))
        .unwrap();
    b.l.replay_add_line(e, 424_242, Side::Credit, usd("1.00"))
        .unwrap();
    assert_eq!(
        b.l.get_entry(e).unwrap().validate(b.l.chart()),
        Err(LedgerError::UnknownAccount(424_242))
    );
    assert_eq!(b.l.post_entry(e), Err(LedgerError::UnknownAccount(424_242)));

    let e = b.l.try_create_entry(d(2026, 3, 1), "y").unwrap();
    b.l.replay_add_line(e, b.cash, Side::Debit, usd("1.00"))
        .unwrap();
    b.l.replay_add_line(e, b.header, Side::Credit, usd("1.00"))
        .unwrap();
    let v = b.l.get_entry(e).unwrap().validate(b.l.chart());
    assert!(
        matches!(v, Err(LedgerError::PlaceholderAccount(_))),
        "{v:?}"
    );
    let r = b.l.post_entry(e);
    assert!(
        matches!(r, Err(LedgerError::PlaceholderAccount(_))),
        "{r:?}"
    );
    assert_eq!(b.l.posted_entries().count(), 0);
}

/// Oracle: post_entry INVARIANT 2 / R-11: the log is in posting order, not id
/// order, and posting never changes an earlier posted entry.
#[test]
fn the_log_is_in_posting_order() {
    let mut b = books();
    let e1 =
        b.l.try_create_entry(d(2026, 3, 1), "first drafted")
            .unwrap();
    let e2 =
        b.l.try_create_entry(d(2026, 2, 1), "second drafted")
            .unwrap();
    for e in [e1, e2] {
        b.l.add_line(e, b.exp, Side::Debit, usd("1.00")).unwrap();
        b.l.add_line(e, b.cash, Side::Credit, usd("1.00")).unwrap();
    }
    b.l.post_entry(e2).unwrap();
    let e2_then = serde_json::to_value(b.l.get_entry(e2)).unwrap();
    b.l.post_entry(e1).unwrap();
    let order: Vec<_> = b.l.posted_entries().map(|e| e.id()).collect();
    assert_eq!(order, vec![e2, e1]);
    assert_eq!(serde_json::to_value(b.l.get_entry(e2)).unwrap(), e2_then);
    assert_eq!(b.l.get_entry(e1).unwrap().status(), EntryStatus::Posted);
}

// ---------------------------------------------------------------------------
// check_reversal / reverse_entry
// ---------------------------------------------------------------------------

/// Oracle: B-9 (Pacioli's storno): the reversal has the same lines with
/// sides flipped, is posted, dated as asked, linked by `reversal_of`, and
/// returns every balance to what it was.
#[test]
fn a_reversal_mirrors_the_original_and_cancels_it() {
    let mut b = books();
    let e = post(
        &mut b.l,
        d(2026, 3, 1),
        &[
            (b.exp, Side::Debit, "30.00"),
            (b.cash, Side::Credit, "20.00"),
            (b.card, Side::Credit, "10.00"),
        ],
    );
    let r = b.l.reverse_entry(e, d(2026, 3, 5), "oops").unwrap();
    let rev = b.l.get_entry(r).unwrap();
    assert_eq!(rev.status(), EntryStatus::Posted);
    assert_eq!(rev.date(), d(2026, 3, 5));
    assert_eq!(rev.reversal_of(), Some(e));
    assert_eq!(
        lines_of(&b.l, r),
        vec![
            (b.exp, Side::Credit, dec("30.00")),
            (b.cash, Side::Debit, dec("20.00")),
            (b.card, Side::Debit, dec("10.00")),
        ]
    );
    for a in [b.exp, b.cash, b.card] {
        assert_eq!(b.l.account_balance(a).unwrap(), Decimal::ZERO);
    }
    // The original is untouched (B-8): never erased.
    assert_eq!(b.l.get_entry(e).unwrap().status(), EntryStatus::Posted);
    assert_eq!(b.l.posted_entries().count(), 2);
}

/// Oracle: B-10 / check_reversal SCENARIO 2: an entry is reversed at most
/// once (EntryAlreadyReversed(entry, by)); reversing the reversal is allowed
/// and re-instates it, and does not reopen the original.
#[test]
fn an_entry_is_reversed_at_most_once() {
    let mut b = books();
    let e = post(
        &mut b.l,
        d(2026, 3, 1),
        &[
            (b.exp, Side::Debit, "30.00"),
            (b.cash, Side::Credit, "30.00"),
        ],
    );
    let r1 = b.l.reverse_entry(e, d(2026, 3, 2), "undo").unwrap();
    let before = observe(&b.l);
    assert_eq!(
        b.l.check_reversal(e, d(2026, 3, 3)),
        Err(LedgerError::EntryAlreadyReversed(e, r1))
    );
    assert_eq!(
        b.l.reverse_entry(e, d(2026, 3, 3), "undo again"),
        Err(LedgerError::EntryAlreadyReversed(e, r1))
    );
    assert_eq!(observe(&b.l), before);

    let r2 = b.l.reverse_entry(r1, d(2026, 3, 4), "redo").unwrap();
    assert_eq!(b.l.account_balance(b.exp).unwrap(), dec("30.00"));
    assert_eq!(
        b.l.reverse_entry(r1, d(2026, 3, 5), "x"),
        Err(LedgerError::EntryAlreadyReversed(r1, r2))
    );
    assert_eq!(
        b.l.reverse_entry(e, d(2026, 3, 5), "x"),
        Err(LedgerError::EntryAlreadyReversed(e, r1))
    );
}

/// Oracle: check_reversal SCENARIO 1: unknown (UnknownEntry) or draft
/// (EntryNotPosted) entries are refused.
#[test]
fn only_posted_entries_are_reversed() {
    let mut b = books();
    let draft = b.l.try_create_entry(d(2026, 3, 1), "draft").unwrap();
    b.l.add_line(draft, b.exp, Side::Debit, usd("1.00"))
        .unwrap();
    b.l.add_line(draft, b.cash, Side::Credit, usd("1.00"))
        .unwrap();
    let before = observe(&b.l);
    assert_eq!(
        b.l.check_reversal(draft, d(2026, 3, 2)),
        Err(LedgerError::EntryNotPosted(draft))
    );
    assert_eq!(
        b.l.reverse_entry(draft, d(2026, 3, 2), "x"),
        Err(LedgerError::EntryNotPosted(draft))
    );
    assert_eq!(
        b.l.reverse_entry(9_999, d(2026, 3, 2), "x"),
        Err(LedgerError::UnknownEntry(9_999))
    );
    assert_eq!(observe(&b.l), before);
}

/// Oracle: B-10 / check_reversal SCENARIO 3: a closing entry is never
/// reversed (ClosingEntryIrreversible), even dated in an open period.
#[test]
fn a_closing_entry_is_never_reversed() {
    let mut b = books();
    post(
        &mut b.l,
        d(2026, 1, 10),
        &[
            (b.cash, Side::Debit, "100.00"),
            (b.rev, Side::Credit, "100.00"),
        ],
    );
    let jan =
        b.l.create_period("2026-01", d(2026, 1, 1), d(2026, 1, 31))
            .unwrap();
    let c = b.l.close_period(jan).unwrap().expect("something to close");
    let before = observe(&b.l);
    assert_eq!(
        b.l.check_reversal(c, d(2026, 2, 1)),
        Err(LedgerError::ClosingEntryIrreversible(c))
    );
    assert_eq!(
        b.l.reverse_entry(c, d(2026, 2, 1), "reopen"),
        Err(LedgerError::ClosingEntryIrreversible(c))
    );
    assert_eq!(observe(&b.l), before);
}

/// Oracle: check_reversal SCENARIO 4 (B-12): a reversal dated in a closed
/// period is refused (ClosedPeriod); dated after it, accepted.
#[test]
fn a_reversal_is_never_dated_in_a_closed_period() {
    let mut b = books();
    let e = post(
        &mut b.l,
        d(2026, 1, 10),
        &[
            (b.card, Side::Debit, "40.00"),
            (b.cash, Side::Credit, "40.00"),
        ],
    );
    let jan =
        b.l.create_period("2026-01", d(2026, 1, 1), d(2026, 1, 31))
            .unwrap();
    b.l.close_period(jan).unwrap();
    let before = observe(&b.l);
    let r = b.l.check_reversal(e, d(2026, 1, 31));
    assert!(
        matches!(r, Err(LedgerError::ClosedPeriod(p, _)) if p == jan),
        "{r:?}"
    );
    let r = b.l.reverse_entry(e, d(2026, 1, 31), "x");
    assert!(
        matches!(r, Err(LedgerError::ClosedPeriod(p, _)) if p == jan),
        "{r:?}"
    );
    assert_eq!(observe(&b.l), before);
    assert!(b.l.reverse_entry(e, d(2026, 2, 1), "x").is_ok());
}

// ---------------------------------------------------------------------------
// create_period
// ---------------------------------------------------------------------------

/// Oracle: create_period SCENARIO 1: end before start is refused
/// (InvalidPeriodRange); a one-day period (end == start) is valid.
#[test]
fn a_period_ending_before_it_starts_is_refused() {
    let mut b = books();
    let before = observe(&b.l);
    let r = b.l.create_period("bad", d(2026, 2, 1), d(2026, 1, 31));
    assert!(
        matches!(r, Err(LedgerError::InvalidPeriodRange { .. })),
        "{r:?}"
    );
    assert_eq!(observe(&b.l), before);
    assert!(
        b.l.create_period("one day", d(2026, 2, 1), d(2026, 2, 1))
            .is_ok()
    );
}

/// Oracle: create_period SCENARIO 2: a month in a quarter in a year nest,
/// in either creation order; adjacent periods are accepted.
#[test]
fn periods_nest() {
    let mut b = books();
    let q1 =
        b.l.create_period("Q1", d(2026, 1, 1), d(2026, 3, 31))
            .unwrap();
    let jan =
        b.l.create_period("Jan", d(2026, 1, 1), d(2026, 1, 31))
            .unwrap();
    let year =
        b.l.create_period("2026", d(2026, 1, 1), d(2026, 12, 31))
            .unwrap();
    let feb =
        b.l.create_period("Feb", d(2026, 2, 1), d(2026, 2, 28))
            .unwrap();
    let q2 =
        b.l.create_period("Q2", d(2026, 4, 1), d(2026, 6, 30))
            .unwrap();
    let ids = [q1, jan, year, feb, q2];
    let distinct: BTreeSet<PeriodId> = ids.iter().copied().collect();
    assert_eq!(distinct.len(), 5);
    let p = b.l.get_period(feb).unwrap();
    assert_eq!((p.start, p.end), (d(2026, 2, 1), d(2026, 2, 28)));
    assert_eq!(p.status, PeriodStatus::Open);
    assert_eq!(p.closing_entry, None);
    assert_eq!(b.l.list_periods().len(), 5);
}

/// Oracle: create_period SCENARIO 3 (DECISION 2026-10-13): a partial overlap,
/// even by one day, is refused (PeriodsOverlap) and NEVER uses up an id.
#[test]
fn periods_nest_but_never_partially_overlap() {
    let mut b = books();
    b.l.create_period("Jan", d(2026, 1, 1), d(2026, 1, 31))
        .unwrap();
    let next = b.l.next_period_id();
    let before = observe(&b.l);
    assert_eq!(
        b.l.create_period("Bad", d(2026, 1, 31), d(2026, 2, 28)),
        Err(LedgerError::PeriodsOverlap {
            name: "Bad".into(),
            start: d(2026, 1, 31),
            end: d(2026, 2, 28),
            other: "Jan".into(),
            other_start: d(2026, 1, 1),
            other_end: d(2026, 1, 31),
        })
    );
    let r = b.l.create_period("Bad2", d(2025, 12, 15), d(2026, 1, 15));
    assert!(
        matches!(r, Err(LedgerError::PeriodsOverlap { .. })),
        "{r:?}"
    );
    assert_eq!(observe(&b.l), before);
    assert_eq!(b.l.next_period_id(), next);
    assert_eq!(
        b.l.create_period("Feb", d(2026, 2, 1), d(2026, 2, 28)),
        Ok(next)
    );
}

// ---------------------------------------------------------------------------
// close_period_to
// ---------------------------------------------------------------------------

/// Books with January revenue 1000.00 and expense 300.00, a February sale of
/// 50.00, and a January period.
fn jan_books() -> (Books, PeriodId, EntryId) {
    let mut b = books();
    post(
        &mut b.l,
        d(2026, 1, 10),
        &[
            (b.cash, Side::Debit, "1000.00"),
            (b.rev, Side::Credit, "1000.00"),
        ],
    );
    post(
        &mut b.l,
        d(2026, 1, 31),
        &[
            (b.exp, Side::Debit, "300.00"),
            (b.card, Side::Credit, "300.00"),
        ],
    );
    let feb = post(
        &mut b.l,
        d(2026, 2, 1),
        &[
            (b.cash, Side::Debit, "50.00"),
            (b.rev, Side::Credit, "50.00"),
        ],
    );
    let jan =
        b.l.create_period("2026-01", d(2026, 1, 1), d(2026, 1, 31))
            .unwrap();
    (b, jan, feb)
}

fn assert_balanced(l: &Ledger, id: EntryId) {
    let (dr, cr) =
        lines_of(l, id)
            .into_iter()
            .fold(
                (Decimal::ZERO, Decimal::ZERO),
                |(dr, cr), (_, s, a)| match s {
                    Side::Debit => (dr + a, cr),
                    Side::Credit => (dr, cr + a),
                },
            );
    assert_eq!(dr, cr, "closing entry must balance");
}

/// Oracle: close INVARIANTS 1-3 and B-11: closing to retained earnings
/// zeroes revenue and expense as of Jan 31, credits 3999 (created, equity)
/// with exactly the net 700.00, is dated Jan 31, balances, locks the
/// period, and leaves February's 50.00 sale untouched.
#[test]
fn a_close_sweeps_only_through_the_period_end() {
    let (mut b, jan, feb) = jan_books();
    let feb_then = serde_json::to_value(b.l.get_entry(feb)).unwrap();
    let c = b.l.close_period(jan).unwrap().expect("something to close");

    let ce = b.l.get_entry(c).unwrap();
    assert_eq!(ce.date(), d(2026, 1, 31));
    assert_eq!(ce.status(), EntryStatus::Posted);
    assert_balanced(&b.l, c);
    let re = b.l.chart().find_by_code("3999").expect("3999 created");
    assert_eq!(re.account_type, AccountType::Equity);
    assert!(!re.placeholder);
    assert_eq!(entry_movement(&b.l, c, re.id), dec("-700.00"));
    assert_eq!(b.l.account_balance(re.id).unwrap(), dec("700.00"));

    assert!(!tb_has_code(&b.l, d(2026, 1, 31), "4000"));
    assert!(!tb_has_code(&b.l, d(2026, 1, 31), "6000"));
    assert_eq!(b.l.account_balance(b.rev).unwrap(), dec("50.00"));
    assert_eq!(b.l.account_balance(b.exp).unwrap(), Decimal::ZERO);
    assert_eq!(serde_json::to_value(b.l.get_entry(feb)).unwrap(), feb_then);

    let p = b.l.get_period(jan).unwrap();
    assert_eq!(p.status, PeriodStatus::Closed);
    assert_eq!(p.closing_entry, Some(c));
    assert!(b.l.closing_entry_ids().contains(&c));
    assert!(trial_balance(&b.l).balanced());
}

/// Oracle: close INVARIANT 2 for a loss: expense 300.00 with no revenue
/// debits retained earnings 300.00.
#[test]
fn a_loss_closes_as_a_debit_to_equity() {
    let mut b = books();
    post(
        &mut b.l,
        d(2026, 1, 5),
        &[
            (b.exp, Side::Debit, "300.00"),
            (b.card, Side::Credit, "300.00"),
        ],
    );
    let jan =
        b.l.create_period("2026-01", d(2026, 1, 1), d(2026, 1, 31))
            .unwrap();
    let c = b.l.close_period(jan).unwrap().unwrap();
    let re = b.l.chart().find_by_code("3999").unwrap().id;
    assert_eq!(entry_movement(&b.l, c, re), dec("300.00"));
    assert_eq!(b.l.account_balance(re).unwrap(), dec("-300.00"));
    assert_eq!(b.l.account_balance(b.exp).unwrap(), Decimal::ZERO);
}

/// Oracle: close INVARIANT 5: nothing to close posts nothing and returns
/// None. ASSUMED: the period is still marked closed (step 3 of the doc).
#[test]
fn a_close_with_nothing_to_close_posts_no_entry() {
    let mut b = books();
    post(
        &mut b.l,
        d(2026, 1, 5),
        &[
            (b.cash, Side::Debit, "10.00"),
            (b.card, Side::Credit, "10.00"),
        ],
    );
    let jan =
        b.l.create_period("2026-01", d(2026, 1, 1), d(2026, 1, 31))
            .unwrap();
    let posted = b.l.posted_entries().count();
    let next = b.l.next_entry_id();
    assert_eq!(b.l.preview_close_period(jan), Ok(None));
    assert_eq!(b.l.close_period(jan), Ok(None));
    assert_eq!(b.l.posted_entries().count(), posted);
    assert_eq!(b.l.next_entry_id(), next);
    assert_eq!(b.l.get_period(jan).unwrap().status, PeriodStatus::Closed);
}

/// Oracle: close SCENARIO 1: unknown (UnknownPeriod) or already closed
/// (PeriodAlreadyClosed) periods are refused.
#[test]
fn a_period_closes_once() {
    let (mut b, jan, _) = jan_books();
    let before = observe(&b.l);
    assert_eq!(
        b.l.close_period(4_040),
        Err(LedgerError::UnknownPeriod(4_040))
    );
    assert_eq!(observe(&b.l), before);
    b.l.close_period(jan).unwrap();
    let before = observe(&b.l);
    assert_eq!(
        b.l.close_period(jan),
        Err(LedgerError::PeriodAlreadyClosed(jan))
    );
    assert_eq!(observe(&b.l), before);
}

/// Oracle: close SCENARIO 2: a draft dated inside the period blocks the
/// close (DraftsInPeriod).
#[test]
fn a_period_with_drafts_never_closes() {
    let (mut b, jan, _) = jan_books();
    b.l.try_create_entry(d(2026, 1, 20), "unfinished").unwrap();
    let before = observe(&b.l);
    assert_eq!(b.l.close_period(jan), Err(LedgerError::DraftsInPeriod(jan)));
    assert_eq!(observe(&b.l), before);
}

/// Oracle: close SCENARIO 3 (B-12): closing out of order is refused
/// (CloseOutOfOrder(id, later)).
#[test]
fn periods_close_in_order() {
    let (mut b, jan, _) = jan_books();
    let feb =
        b.l.create_period("2026-02", d(2026, 2, 1), d(2026, 2, 28))
            .unwrap();
    b.l.close_period(feb).unwrap();
    let before = observe(&b.l);
    assert_eq!(
        b.l.close_period(jan),
        Err(LedgerError::CloseOutOfOrder(jan, feb))
    );
    assert_eq!(observe(&b.l), before);
}

/// Oracle: close SCENARIO 5 / ControlAccountMismatch: an existing 3999 that
/// is not equity, or is a placeholder, is refused, never posted to.
#[test]
fn retained_earnings_must_be_a_postable_equity_account() {
    let (mut b, jan, _) = jan_books();
    b.l.add_account("3999", "Suspense", AccountType::Asset, None, false)
        .unwrap();
    let before = observe(&b.l);
    let r = b.l.close_period(jan);
    assert!(
        matches!(
            &r,
            Err(LedgerError::ControlAccountMismatch {
                code,
                name,
                expected: AccountType::Equity,
                found: AccountType::Asset,
                placeholder: false,
                ..
            }) if code == "3999" && name == "Suspense"
        ),
        "{r:?}"
    );
    assert_eq!(observe(&b.l), before);

    let (mut b, jan, _) = jan_books();
    b.l.add_account("3999", "Equity Header", AccountType::Equity, None, true)
        .unwrap();
    let before = observe(&b.l);
    let r = b.l.close_period(jan);
    assert!(
        matches!(
            &r,
            Err(LedgerError::ControlAccountMismatch {
                expected: AccountType::Equity,
                found: AccountType::Equity,
                placeholder: true,
                ..
            })
        ),
        "{r:?}"
    );
    assert_eq!(observe(&b.l), before);
}

/// Oracle: close to an existing postable equity 3999: it is used, not
/// duplicated.
#[test]
fn an_existing_retained_earnings_account_is_used() {
    let (mut b, jan, _) = jan_books();
    let re =
        b.l.add_account(
            "3999",
            "Retained Earnings",
            AccountType::Equity,
            None,
            false,
        )
        .unwrap();
    let n = b.l.chart().len();
    b.l.close_period(jan).unwrap();
    assert_eq!(b.l.chart().len(), n);
    assert_eq!(b.l.account_balance(re).unwrap(), dec("700.00"));
}

/// Oracle: close INVARIANT 4: a dividend sweep account ends at zero, its
/// balance moved into retained earnings (700.00 net - 200.00 dividends).
#[test]
fn a_sweep_account_closes_into_its_target() {
    let (mut b, jan, _) = jan_books();
    let div =
        b.l.add_account("3100", "Dividends", AccountType::Equity, None, false)
            .unwrap();
    post(
        &mut b.l,
        d(2026, 1, 20),
        &[
            (div, Side::Debit, "200.00"),
            (b.cash, Side::Credit, "200.00"),
        ],
    );
    b.l.close_period_to(jan, &CloseTo::RetainedEarnings { sweep: vec![div] })
        .unwrap();
    let re = b.l.chart().find_by_code("3999").unwrap().id;
    assert_eq!(b.l.account_balance(div).unwrap(), Decimal::ZERO);
    assert_eq!(b.l.account_balance(re).unwrap(), dec("500.00"));
}

/// Oracle: CloseTo::Account: a sole proprietor's capital gets the 700.00
/// net, less 100.00 draws swept into it.
#[test]
fn a_close_into_one_capital_account_carries_draws() {
    let (mut b, jan, _) = jan_books();
    post(
        &mut b.l,
        d(2026, 1, 25),
        &[
            (b.dist_a, Side::Debit, "100.00"),
            (b.cash, Side::Credit, "100.00"),
        ],
    );
    let c =
        b.l.close_period_to(
            jan,
            &CloseTo::Account {
                into: b.cap_a,
                sweep: vec![b.dist_a],
            },
        )
        .unwrap()
        .unwrap();
    assert_balanced(&b.l, c);
    assert_eq!(b.l.account_balance(b.cap_a).unwrap(), dec("600.00"));
    assert_eq!(b.l.account_balance(b.dist_a).unwrap(), Decimal::ZERO);
    assert!(b.l.chart().find_by_code("3999").is_none());
}

fn members(shares: &[(AccountId, &str, Option<AccountId>)]) -> CloseTo {
    CloseTo::Members(
        shares
            .iter()
            .map(|(capital, share, distributions)| MemberClose {
                capital: *capital,
                share: dec(share),
                distributions: *distributions,
            })
            .collect(),
    )
}

/// Oracle: B-13 / AGENTS.md (50/50): net 700.00 splits 350.00 / 350.00; each
/// member's distributions close into their own capital.
#[test]
fn a_partnership_close_splits_by_share() {
    let (mut b, jan, _) = jan_books();
    post(
        &mut b.l,
        d(2026, 1, 25),
        &[
            (b.dist_a, Side::Debit, "40.00"),
            (b.cash, Side::Credit, "40.00"),
        ],
    );
    let to = members(&[(b.cap_a, "0.5", Some(b.dist_a)), (b.cap_b, "0.5", None)]);
    let c = b.l.close_period_to(jan, &to).unwrap().unwrap();
    assert_balanced(&b.l, c);
    assert_eq!(b.l.account_balance(b.cap_a).unwrap(), dec("310.00"));
    assert_eq!(b.l.account_balance(b.cap_b).unwrap(), dec("350.00"));
    assert_eq!(b.l.account_balance(b.dist_a).unwrap(), Decimal::ZERO);
    assert_eq!(b.l.account_balance(b.rev).unwrap(), dec("50.00"));
}

/// Books whose January net income is exactly `net` (revenue only).
fn net_books(net: &str) -> (Books, PeriodId, AccountId) {
    let mut b = books();
    let cap_c =
        b.l.add_account("3003", "Capital C", AccountType::Equity, None, false)
            .unwrap();
    let (side_rev, side_cash) = if net.starts_with('-') {
        (Side::Debit, Side::Credit)
    } else {
        (Side::Credit, Side::Debit)
    };
    let amt = net.trim_start_matches('-');
    post(
        &mut b.l,
        d(2026, 1, 9),
        &[(b.cash, side_cash, amt), (b.rev, side_rev, amt)],
    );
    let jan =
        b.l.create_period("2026-01", d(2026, 1, 1), d(2026, 1, 31))
            .unwrap();
    (b, jan, cap_c)
}

/// Oracle: B-13 DECISION (largest remainder, ties to the earlier member),
/// hand-computed: 1.05 at 0.3/0.3/0.4 is exactly 0.315/0.315/0.42; rounded
/// down 0.31/0.31/0.42 leaves one cent, remainders .005/.005/0 tie, so the
/// first member gets it: 0.32/0.31/0.42. (The retired last-absorbs rule
/// would give 0.32/0.32/0.41.)
#[test]
fn members_split_by_largest_remainder_ties_to_the_earlier() {
    let (mut b, jan, cap_c) = net_books("1.05");
    let to = members(&[
        (b.cap_a, "0.3", None),
        (b.cap_b, "0.3", None),
        (cap_c, "0.4", None),
    ]);
    b.l.close_period_to(jan, &to).unwrap().unwrap();
    assert_eq!(b.l.account_balance(b.cap_a).unwrap(), dec("0.32"));
    assert_eq!(b.l.account_balance(b.cap_b).unwrap(), dec("0.31"));
    assert_eq!(b.l.account_balance(cap_c).unwrap(), dec("0.42"));
}

/// Oracle: B-13 / kernel::apportion INVARIANT 3, hand-computed: 0.07 at
/// 0.2/0.3/0.5 is exactly 0.014/0.021/0.035; rounded down 0.01/0.02/0.03
/// (sum 0.06), remainders .004/.001/.005, so the largest remainder (the
/// third) gets the cent: 0.01/0.02/0.04, wherever it sits in the list.
#[test]
fn the_odd_cent_goes_to_the_largest_remainder_not_by_position() {
    let (mut b, jan, cap_c) = net_books("0.07");
    let to = members(&[
        (b.cap_a, "0.2", None),
        (b.cap_b, "0.3", None),
        (cap_c, "0.5", None),
    ]);
    b.l.close_period_to(jan, &to).unwrap().unwrap();
    assert_eq!(b.l.account_balance(b.cap_a).unwrap(), dec("0.01"));
    assert_eq!(b.l.account_balance(b.cap_b).unwrap(), dec("0.02"));
    assert_eq!(b.l.account_balance(cap_c).unwrap(), dec("0.04"));

    // Remainders .004/.001/.005 again, but the largest is first this time:
    // 0.07 at 0.5/0.3/0.2 = 0.035/0.021/0.014 -> 0.04/0.02/0.01.
    let (mut b, jan, cap_c) = net_books("0.07");
    let to = members(&[
        (b.cap_a, "0.5", None),
        (b.cap_b, "0.3", None),
        (cap_c, "0.2", None),
    ]);
    b.l.close_period_to(jan, &to).unwrap().unwrap();
    assert_eq!(b.l.account_balance(b.cap_a).unwrap(), dec("0.04"));
    assert_eq!(b.l.account_balance(b.cap_b).unwrap(), dec("0.02"));
    assert_eq!(b.l.account_balance(cap_c).unwrap(), dec("0.01"));
}

/// Oracle: periods::allocate doc ("a loss splits as the negation of the same
/// gain"): a 1.05 loss at 0.3/0.3/0.4 is -0.32/-0.31/-0.42.
#[test]
fn a_partnership_loss_splits_as_the_negated_gain() {
    let (mut b, jan, cap_c) = net_books("-1.05");
    let to = members(&[
        (b.cap_a, "0.3", None),
        (b.cap_b, "0.3", None),
        (cap_c, "0.4", None),
    ]);
    b.l.close_period_to(jan, &to).unwrap().unwrap();
    assert_eq!(b.l.account_balance(b.cap_a).unwrap(), dec("-0.32"));
    assert_eq!(b.l.account_balance(b.cap_b).unwrap(), dec("-0.31"));
    assert_eq!(b.l.account_balance(cap_c).unwrap(), dec("-0.42"));
}

/// Oracle: close SCENARIO 4: member shares that don't sum to 1, a share that
/// isn't positive, a non-equity or repeated capital account, or a target
/// that isn't equity are refused, the books unchanged.
#[test]
fn an_invalid_close_target_is_refused() {
    let (mut b, jan, _) = jan_books();
    let bad: Vec<CloseTo> = vec![
        members(&[(b.cap_a, "0.5", None), (b.cap_b, "0.4", None)]),
        members(&[(b.cap_a, "0.5", None), (b.cap_b, "0.6", None)]),
        members(&[(b.cap_a, "1.5", None), (b.cap_b, "-0.5", None)]),
        members(&[(b.cap_a, "1", None), (b.cap_b, "0", None)]),
        members(&[(b.cap_a, "0.5", None), (b.cash, "0.5", None)]),
        members(&[(b.cap_a, "0.5", None), (b.cap_a, "0.5", None)]),
        members(&[(b.cap_a, "0.5", Some(b.cap_b)), (b.cap_b, "0.5", None)]),
        members(&[]),
        CloseTo::Account {
            into: b.cash,
            sweep: vec![],
        },
        CloseTo::Account {
            into: b.cap_a,
            sweep: vec![b.cap_a],
        },
        CloseTo::Account {
            into: b.cap_a,
            sweep: vec![b.exp],
        },
        CloseTo::RetainedEarnings {
            sweep: vec![b.cash],
        },
    ];
    for to in bad {
        let before = observe(&b.l);
        assert!(
            b.l.preview_close_period_to(jan, &to).is_err(),
            "preview {to:?}"
        );
        assert!(b.l.close_period_to(jan, &to).is_err(), "{to:?}");
        assert_eq!(observe(&b.l), before, "{to:?}");
    }
}

/// Oracle: preview_close_period_to doc: pure, and returns the id the closing
/// entry will receive.
#[test]
fn a_close_preview_is_pure_and_names_the_closing_entry() {
    let (mut b, jan, _) = jan_books();
    let to = members(&[(b.cap_a, "0.5", None), (b.cap_b, "0.5", None)]);
    let before = observe(&b.l);
    let preview = b.l.preview_close_period_to(jan, &to).unwrap();
    assert_eq!(observe(&b.l), before);
    assert_eq!(preview, Some(b.l.next_entry_id()));
    assert_eq!(b.l.close_period_to(jan, &to).unwrap(), preview);
}

// ---------------------------------------------------------------------------
// mark_cleared / reconciliation_report
// ---------------------------------------------------------------------------

/// Oracle: reconciliation INVARIANTS 1-4, hand-computed: cash +1000.00
/// (Jan 5, cleared), -200.00 (Jan 10, uncleared), +50.00 (Feb 2, cleared).
/// As of Jan 31 vs a 1000.00 statement: book 850.00, cleared 1000.00,
/// difference 0, outstanding = only the Jan 10 check.
#[test]
fn a_reconciliation_compares_cleared_lines_to_the_statement() {
    let mut b = books();
    let dep = post(
        &mut b.l,
        d(2026, 1, 5),
        &[
            (b.cash, Side::Debit, "1000.00"),
            (b.cap_a, Side::Credit, "1000.00"),
        ],
    );
    let chk = post(
        &mut b.l,
        d(2026, 1, 10),
        &[
            (b.exp, Side::Debit, "200.00"),
            (b.cash, Side::Credit, "200.00"),
        ],
    );
    let late = post(
        &mut b.l,
        d(2026, 2, 2),
        &[
            (b.cash, Side::Debit, "50.00"),
            (b.rev, Side::Credit, "50.00"),
        ],
    );
    let tb_before = serde_json::to_value(trial_balance(&b.l)).unwrap();
    let posted_before: Vec<_> =
        b.l.posted_entries()
            .map(|e| serde_json::to_value(e).unwrap())
            .collect();
    b.l.mark_cleared(dep, 0, true).unwrap();
    b.l.mark_cleared(late, 0, true).unwrap();
    assert!(b.l.is_cleared(dep, 0));
    assert!(!b.l.is_cleared(chk, 1));

    // Marking changes no amount and no report.
    assert_eq!(
        serde_json::to_value(trial_balance(&b.l)).unwrap(),
        tb_before
    );
    let posted_after: Vec<_> =
        b.l.posted_entries()
            .map(|e| serde_json::to_value(e).unwrap())
            .collect();
    assert_eq!(posted_after, posted_before);

    let r =
        b.l.reconciliation_report(b.cash, d(2026, 1, 31), dec("1000.00"))
            .unwrap();
    assert_eq!(r.account, b.cash);
    assert_eq!(r.book_balance, dec("850.00"));
    assert_eq!(r.cleared_balance, dec("1000.00"));
    assert_eq!(r.statement_balance, dec("1000.00"));
    assert_eq!(r.difference, Decimal::ZERO);
    assert_eq!(r.uncleared_lines.len(), 1);
    let u = &r.uncleared_lines[0];
    assert_eq!((u.entry_id, u.line_index), (chk, 1));
    assert_eq!(u.date, d(2026, 1, 10));
    assert_eq!(u.side, Side::Credit);
    assert_eq!(u.amount, dec("200.00"));

    // Clear the check: cleared 800.00 against 1000.00 is 200.00 short.
    b.l.mark_cleared(chk, 1, true).unwrap();
    let r =
        b.l.reconciliation_report(b.cash, d(2026, 1, 31), dec("1000.00"))
            .unwrap();
    assert_eq!(r.cleared_balance, dec("800.00"));
    assert_eq!(r.difference, dec("-200.00"));
    assert!(r.uncleared_lines.is_empty());

    // Un-clearing is allowed and undoes it.
    b.l.mark_cleared(chk, 1, false).unwrap();
    assert!(!b.l.is_cleared(chk, 1));
    let r =
        b.l.reconciliation_report(b.cash, d(2026, 2, 28), dec("1050.00"))
            .unwrap();
    assert_eq!(r.cleared_balance, dec("1050.00"));
    assert_eq!(r.difference, Decimal::ZERO);
    assert_eq!(r.uncleared_lines.len(), 1);
}

/// Oracle: mark_cleared SCENARIO 1: unknown entry (UnknownEntry), draft
/// (ClearedOnDraft), or no such line (LineIndexOutOfRange) is refused.
#[test]
fn only_a_posted_line_can_clear() {
    let mut b = books();
    let e = post(
        &mut b.l,
        d(2026, 1, 5),
        &[
            (b.cash, Side::Debit, "10.00"),
            (b.cap_a, Side::Credit, "10.00"),
        ],
    );
    let draft = b.l.try_create_entry(d(2026, 1, 6), "draft").unwrap();
    b.l.add_line(draft, b.cash, Side::Debit, usd("1.00"))
        .unwrap();
    let before = observe(&b.l);
    assert_eq!(
        b.l.mark_cleared(5_555, 0, true),
        Err(LedgerError::UnknownEntry(5_555))
    );
    assert_eq!(
        b.l.mark_cleared(draft, 0, true),
        Err(LedgerError::ClearedOnDraft(draft))
    );
    assert_eq!(
        b.l.mark_cleared(e, 2, true),
        Err(LedgerError::LineIndexOutOfRange(2, e))
    );
    assert_eq!(observe(&b.l), before);
}

// ---------------------------------------------------------------------------
// account_balance
// ---------------------------------------------------------------------------

/// Oracle: account_balance INVARIANT 1 / B-5 and trial balance SCENARIO 2:
/// asset debited 100 and credited 30 is 70; credit-normal accounts read
/// credits minus debits; a contra balance is negative.
#[test]
fn balances_read_on_the_normal_side() {
    let mut b = books();
    post(
        &mut b.l,
        d(2026, 1, 1),
        &[
            (b.cash, Side::Debit, "100.00"),
            (b.rev, Side::Credit, "100.00"),
        ],
    );
    post(
        &mut b.l,
        d(2026, 1, 2),
        &[
            (b.exp, Side::Debit, "30.00"),
            (b.cash, Side::Credit, "30.00"),
        ],
    );
    post(
        &mut b.l,
        d(2026, 1, 3),
        &[
            (b.card, Side::Debit, "5.00"),
            (b.exp, Side::Credit, "45.00"),
            (b.cash, Side::Debit, "40.00"),
        ],
    );
    assert_eq!(b.l.account_balance(b.cash).unwrap(), dec("110.00"));
    assert_eq!(b.l.account_balance(b.rev).unwrap(), dec("100.00"));
    assert_eq!(b.l.account_balance(b.exp).unwrap(), dec("-15.00"));
    assert_eq!(b.l.account_balance(b.card).unwrap(), dec("-5.00"));
    assert_eq!(b.l.account_balance(b.cap_a).unwrap(), Decimal::ZERO);
    let tb = trial_balance_as_of(&b.l, d(2026, 1, 2));
    let cash = tb.lines.iter().find(|x| x.code == "1000").unwrap();
    assert_eq!((cash.debit, cash.credit), (dec("70.00"), Decimal::ZERO));
}

/// Oracle: account_balance INVARIANT 2: never negative zero, for every type,
/// untouched or netted to zero.
#[test]
fn a_zero_balance_is_never_negative_zero() {
    let mut b = books();
    for a in [b.cash, b.card, b.cap_a, b.rev, b.exp] {
        let z = b.l.account_balance(a).unwrap();
        assert!(z.is_zero() && !z.is_sign_negative(), "untouched {a}: {z:?}");
    }
    post(
        &mut b.l,
        d(2026, 1, 1),
        &[(b.card, Side::Debit, "5.00"), (b.rev, Side::Credit, "5.00")],
    );
    post(
        &mut b.l,
        d(2026, 1, 2),
        &[(b.rev, Side::Debit, "5.00"), (b.card, Side::Credit, "5.00")],
    );
    for a in [b.card, b.rev] {
        let z = b.l.account_balance(a).unwrap();
        assert!(z.is_zero() && !z.is_sign_negative(), "netted {a}: {z:?}");
    }
}

/// Oracle: account_balance SCENARIO 1: an unknown account is refused.
/// ASSUMED: with UnknownAccount (the error the catalog names for it).
#[test]
fn the_balance_of_an_unknown_account_is_refused() {
    let b = books();
    assert_eq!(
        b.l.account_balance(123_456),
        Err(LedgerError::UnknownAccount(123_456))
    );
}

// ---------------------------------------------------------------------------
// set_line_class / set_entry_vendor
// ---------------------------------------------------------------------------

/// Oracle: set_line_class GOAL: a draft line is tagged (and cleared with
/// None); SCENARIO 1 (B-8): a posted line NEVER changes; SCENARIO 2: an
/// unknown line or class is refused.
#[test]
fn only_draft_lines_take_a_class() {
    let mut b = books();
    let retail = b.l.add_class("Retail").unwrap();
    let e = b.l.try_create_entry(d(2026, 1, 5), "x").unwrap();
    b.l.add_line(e, b.exp, Side::Debit, usd("9.00")).unwrap();
    b.l.add_line(e, b.cash, Side::Credit, usd("9.00")).unwrap();
    b.l.set_line_class(e, 0, Some(retail)).unwrap();
    assert_eq!(b.l.get_entry(e).unwrap().lines()[0].class_id, Some(retail));
    assert_eq!(b.l.get_entry(e).unwrap().lines()[1].class_id, None);
    b.l.set_line_class(e, 0, None).unwrap();
    assert_eq!(b.l.get_entry(e).unwrap().lines()[0].class_id, None);

    let before = observe(&b.l);
    assert!(b.l.set_line_class(e, 2, Some(retail)).is_err());
    assert_eq!(
        b.l.set_line_class(e, 0, Some(retail + 100)),
        Err(LedgerError::UnknownClass(retail + 100))
    );
    assert_eq!(
        b.l.set_line_class(8_888, 0, Some(retail)),
        Err(LedgerError::UnknownEntry(8_888))
    );
    assert_eq!(observe(&b.l), before);

    b.l.set_line_class(e, 1, Some(retail)).unwrap();
    b.l.post_entry(e).unwrap();
    let before = observe(&b.l);
    assert!(b.l.set_line_class(e, 0, Some(retail)).is_err());
    assert!(b.l.set_line_class(e, 1, None).is_err());
    assert_eq!(observe(&b.l), before);
    assert_eq!(b.l.get_entry(e).unwrap().lines()[1].class_id, Some(retail));
}

/// Oracle: set_entry_vendor SCENARIO 2: tagging a posted entry is allowed
/// and changes no amount; clearing with None removes the tag.
#[test]
fn a_vendor_tag_changes_no_amount() {
    let mut b = books();
    let v = b.l.add_vendor("Landlord LLC").unwrap();
    let e = post(
        &mut b.l,
        d(2026, 1, 5),
        &[
            (b.exp, Side::Debit, "2500.00"),
            (b.cash, Side::Credit, "2500.00"),
        ],
    );
    let entry_then = serde_json::to_value(b.l.get_entry(e)).unwrap();
    let tb_then = serde_json::to_value(trial_balance(&b.l)).unwrap();
    b.l.set_entry_vendor(e, Some(v)).unwrap();
    assert_eq!(b.l.entry_vendor(e), Some(v));
    assert_eq!(serde_json::to_value(b.l.get_entry(e)).unwrap(), entry_then);
    assert_eq!(serde_json::to_value(trial_balance(&b.l)).unwrap(), tb_then);
    b.l.set_entry_vendor(e, None).unwrap();
    assert_eq!(b.l.entry_vendor(e), None);

    let draft = b.l.try_create_entry(d(2026, 1, 6), "draft").unwrap();
    b.l.set_entry_vendor(draft, Some(v)).unwrap();
    assert_eq!(b.l.entry_vendor(draft), Some(v));
}

/// Oracle: set_entry_vendor SCENARIO 1: unknown entry (UnknownEntry) or
/// vendor (UnknownVendor) is refused.
#[test]
fn a_vendor_tag_needs_a_real_entry_and_vendor() {
    let mut b = books();
    let v = b.l.add_vendor("Landlord LLC").unwrap();
    let e = post(
        &mut b.l,
        d(2026, 1, 5),
        &[(b.exp, Side::Debit, "1.00"), (b.cash, Side::Credit, "1.00")],
    );
    let before = observe(&b.l);
    assert_eq!(
        b.l.set_entry_vendor(6_666, Some(v)),
        Err(LedgerError::UnknownEntry(6_666))
    );
    assert_eq!(
        b.l.set_entry_vendor(e, Some(v + 50)),
        Err(LedgerError::UnknownVendor(v + 50))
    );
    assert_eq!(observe(&b.l), before);
}

// ---------------------------------------------------------------------------
// control accounts
// ---------------------------------------------------------------------------

fn one_line(amount: &str) -> Vec<DocumentLine> {
    vec![DocumentLine {
        description: "consulting".into(),
        amount: dec(amount),
    }]
}

/// Oracle: ControlAccount doc + create_invoice INVARIANT 3: receivables
/// (1200) of the wrong type is refused (ControlAccountMismatch, expected
/// Asset), never posted to, the books unchanged.
#[test]
fn an_invoice_refuses_a_receivables_account_of_the_wrong_type() {
    let mut b = books();
    b.l.add_account("1200", "Car Loan", AccountType::Liability, None, false)
        .unwrap();
    let c = b.l.add_customer("Acme").unwrap();
    let before = observe(&b.l);
    let r =
        b.l.create_invoice(c, d(2026, 1, 5), d(2026, 2, 5), one_line("100.00"));
    assert!(
        matches!(
            &r,
            Err(LedgerError::ControlAccountMismatch {
                code,
                expected: AccountType::Asset,
                found: AccountType::Liability,
                placeholder: false,
                ..
            }) if code == "1200"
        ),
        "{r:?}"
    );
    assert_eq!(observe(&b.l), before);
}

/// Oracle: ControlAccount doc: revenue (4000) that exists as an expense is
/// refused (expected Revenue).
#[test]
fn an_invoice_refuses_a_revenue_account_of_the_wrong_type() {
    let mut l = Ledger::new(Currency::USD);
    l.add_account("4000", "Contractors", AccountType::Expense, None, false)
        .unwrap();
    let c = l.add_customer("Acme").unwrap();
    let before = observe(&l);
    let r = l.create_invoice(c, d(2026, 1, 5), d(2026, 2, 5), one_line("100.00"));
    assert!(
        matches!(
            &r,
            Err(LedgerError::ControlAccountMismatch {
                code,
                expected: AccountType::Revenue,
                found: AccountType::Expense,
                ..
            }) if code == "4000"
        ),
        "{r:?}"
    );
    assert_eq!(observe(&l), before);
}

/// Oracle: ControlAccount doc: payables (2000) as an asset, or general
/// expenses (6999) as a placeholder expense, is refused by create_bill.
#[test]
fn a_bill_refuses_control_accounts_of_the_wrong_kind() {
    let mut l = Ledger::new(Currency::USD);
    l.add_account("2000", "Deposits", AccountType::Asset, None, false)
        .unwrap();
    let v = l.add_vendor("Supplier").unwrap();
    let before = observe(&l);
    let r = l.create_bill(v, d(2026, 1, 5), d(2026, 2, 5), one_line("80.00"));
    assert!(
        matches!(
            &r,
            Err(LedgerError::ControlAccountMismatch {
                code,
                expected: AccountType::Liability,
                found: AccountType::Asset,
                ..
            }) if code == "2000"
        ),
        "{r:?}"
    );
    assert_eq!(observe(&l), before);

    let mut l = Ledger::new(Currency::USD);
    l.add_account("6999", "Expenses", AccountType::Expense, None, true)
        .unwrap();
    let v = l.add_vendor("Supplier").unwrap();
    let before = observe(&l);
    let r = l.create_bill(v, d(2026, 1, 5), d(2026, 2, 5), one_line("80.00"));
    assert!(
        matches!(
            &r,
            Err(LedgerError::ControlAccountMismatch {
                code,
                placeholder: true,
                found: AccountType::Expense,
                ..
            }) if code == "6999"
        ),
        "{r:?}"
    );
    assert_eq!(observe(&l), before);
}

/// Oracle: create_invoice INVARIANT 1: an existing control account of the
/// right type is used (Dr 1200 / Cr 4000 for 100.00), not replaced.
#[test]
fn an_invoice_posts_to_existing_control_accounts_of_the_right_type() {
    let mut b = books();
    let ar =
        b.l.add_account("1200", "Receivables", AccountType::Asset, None, false)
            .unwrap();
    let c = b.l.add_customer("Acme").unwrap();
    let n = b.l.chart().len();
    b.l.create_invoice(c, d(2026, 1, 5), d(2026, 2, 5), one_line("100.00"))
        .unwrap();
    assert_eq!(b.l.chart().len(), n);
    assert_eq!(b.l.account_balance(ar).unwrap(), dec("100.00"));
    assert_eq!(b.l.account_balance(b.rev).unwrap(), dec("100.00"));
}

// ---------------------------------------------------------------------------
// properties
// ---------------------------------------------------------------------------

/// One random transaction: (day of 2026 from Jan 1, 0..90), debit account
/// index, credit account index, cents.
type Tx = (u32, usize, usize, i64);

fn txs() -> impl Strategy<Value = Vec<Tx>> {
    prop::collection::vec((0u32..90, 0usize..7, 0usize..7, 1i64..1_000_000), 0..20).prop_map(|v| {
        v.into_iter()
            .map(|(day, dr, cr, c)| (day, dr, if cr == dr { (cr + 1) % 7 } else { cr }, c))
            .collect()
    })
}

fn postable(b: &Books) -> [AccountId; 7] {
    [b.cash, b.card, b.cap_a, b.cap_b, b.dist_a, b.rev, b.exp]
}

fn day(n: u32) -> NaiveDate {
    d(2026, 1, 1) + chrono::Days::new(u64::from(n))
}

fn cents(c: i64) -> Decimal {
    Decimal::new(c, 2)
}

fn post_tx(b: &mut Books, t: &Tx) -> EntryId {
    let accts = postable(b);
    let id = b.l.try_create_entry(day(t.0), "random").unwrap();
    b.l.add_line(
        id,
        accts[t.1],
        Side::Debit,
        Money::new(cents(t.3), Currency::USD),
    )
    .unwrap();
    b.l.add_line(
        id,
        accts[t.2],
        Side::Credit,
        Money::new(cents(t.3), Currency::USD),
    )
    .unwrap();
    b.l.post_entry(id).unwrap();
    id
}

/// The test's own model: signed movement (debit +) per account index, for
/// transactions dated on or before `through`.
fn model(t: &[Tx], through: u32) -> [Decimal; 7] {
    let mut m = [Decimal::ZERO; 7];
    for (dd, dr, cr, c) in t {
        if *dd <= through {
            m[*dr] += cents(*c);
            m[*cr] -= cents(*c);
        }
    }
    m
}

const NORMAL_DEBIT: [bool; 7] = [true, false, false, false, false, false, true];

/// Fixture for the refusal law: random books with a closed January, a
/// reversed entry, a posted closing entry, an unbalanced February draft, a
/// draft dated in the closed January, a vendor, a class, a customer, and a
/// wrong-type 1200.
struct Fix {
    b: Books,
    jan: PeriodId,
    feb: PeriodId,
    mar: PeriodId,
    posted: EntryId,
    reversed: EntryId,
    closing: EntryId,
    feb_draft: EntryId,
    jan_draft: EntryId,
    class: u32,
    customer: u64,
}

fn fixture(t: &[Tx]) -> Fix {
    let mut b = books();
    let reversed = post(
        &mut b.l,
        d(2026, 1, 15),
        &[
            (b.cash, Side::Debit, "100.00"),
            (b.rev, Side::Credit, "100.00"),
        ],
    );
    let mut last = reversed;
    for x in t {
        last = post_tx(&mut b, x);
    }
    let jan =
        b.l.create_period("Jan", d(2026, 1, 1), d(2026, 1, 31))
            .unwrap();
    let feb =
        b.l.create_period("Feb", d(2026, 2, 1), d(2026, 2, 28))
            .unwrap();
    let mar =
        b.l.create_period("Mar", d(2026, 3, 1), d(2026, 3, 31))
            .unwrap();
    b.l.reverse_entry(reversed, d(2026, 3, 1), "reverse")
        .unwrap();
    // January still has at least the expense or revenue of the fixed entry
    // in the original form plus whatever random entries landed there; add a
    // sure January expense so the close always has something to sweep.
    post(
        &mut b.l,
        d(2026, 1, 20),
        &[(b.exp, Side::Debit, "7.00"), (b.cash, Side::Credit, "7.00")],
    );
    let to = members(&[(b.cap_a, "0.5", Some(b.dist_a)), (b.cap_b, "0.5", None)]);
    let closing =
        b.l.close_period_to(jan, &to)
            .unwrap()
            .expect("expense to close");
    let feb_draft = b.l.try_create_entry(d(2026, 2, 10), "unbalanced").unwrap();
    b.l.add_line(feb_draft, b.cash, Side::Debit, usd("10.00"))
        .unwrap();
    b.l.add_line(feb_draft, b.rev, Side::Credit, usd("5.00"))
        .unwrap();
    let jan_draft = b.l.create_entry(d(2026, 1, 25), "backdated");
    let _ = b.l.add_line(jan_draft, b.exp, Side::Debit, usd("3.00"));
    let _ = b.l.add_line(jan_draft, b.cash, Side::Credit, usd("3.00"));
    b.l.add_vendor("Vendor").unwrap();
    let class = b.l.add_class("Retail").unwrap();
    let customer = b.l.add_customer("Acme").unwrap();
    b.l.add_account("1200", "Car Loan", AccountType::Liability, None, false)
        .unwrap();
    Fix {
        b,
        jan,
        feb,
        mar,
        posted: last,
        reversed,
        closing,
        feb_draft,
        jan_draft,
        class,
        customer,
    }
}

const INVALID_COMMANDS: usize = 30;

/// Run invalid command `k` against the fixture; it must be refused.
fn invalid_command(f: &mut Fix, k: usize) -> Result<(), LedgerError> {
    let Fix { b, .. } = f;
    let l = &mut b.l;
    let ghost: u64 = 9_000_000;
    match k {
        0 => l.post_entry(ghost),
        1 => l.post_entry(f.posted),
        2 => l.post_entry(f.feb_draft),
        3 => l.add_line(f.posted, b.exp, Side::Debit, usd("1.00")),
        4 => l.add_line(f.feb_draft, ghost, Side::Debit, usd("1.00")),
        5 => l.add_line(f.feb_draft, b.header, Side::Debit, usd("1.00")),
        6 => l.add_line(
            f.feb_draft,
            b.exp,
            Side::Debit,
            Money::new(dec("1.00"), Currency::GBP),
        ),
        7 => l
            .add_account("1000", "dup", AccountType::Asset, None, false)
            .map(|_| ()),
        8 => l
            .add_account("", "empty", AccountType::Asset, None, false)
            .map(|_| ()),
        9 => l
            .add_account("1999", "orphan", AccountType::Asset, Some(ghost), false)
            .map(|_| ()),
        10 => l
            .add_account(
                "1999",
                "mismatch",
                AccountType::Expense,
                Some(b.header),
                false,
            )
            .map(|_| ()),
        11 => l
            .reverse_entry(f.reversed, d(2026, 3, 15), "again")
            .map(|_| ()),
        12 => l
            .reverse_entry(f.closing, d(2026, 3, 15), "reopen")
            .map(|_| ()),
        13 => l
            .reverse_entry(f.feb_draft, d(2026, 3, 15), "draft")
            .map(|_| ()),
        14 => l
            .reverse_entry(f.posted, d(2026, 1, 31), "into closed")
            .map(|_| ()),
        15 => l
            .create_period("overlap", d(2026, 1, 15), d(2026, 2, 15))
            .map(|_| ()),
        16 => l
            .create_period("backwards", d(2026, 5, 2), d(2026, 5, 1))
            .map(|_| ()),
        17 => l.try_create_entry(d(2026, 1, 1), "closed").map(|_| ()),
        18 => l.close_period(f.jan).map(|_| ()),
        19 => l.close_period(ghost).map(|_| ()),
        20 => l.close_period(f.feb).map(|_| ()),
        21 => {
            let to = members(&[(b.cap_a, "0.5", None), (b.cap_b, "0.4", None)]);
            l.close_period_to(f.mar, &to).map(|_| ())
        }
        22 => l.mark_cleared(f.feb_draft, 0, true),
        23 => l.mark_cleared(f.posted, 2, true),
        24 => l.set_line_class(f.posted, 0, Some(f.class)),
        25 => l.set_line_class(f.feb_draft, 0, Some(f.class + 1)),
        26 => l.set_entry_vendor(f.posted, Some(ghost)),
        27 => l.add_line_fx(
            f.feb_draft,
            b.exp,
            Side::Debit,
            Currency::EUR,
            dec("1"),
            dec("0"),
        ),
        28 => l.post_entry(f.jan_draft),
        29 => l
            .create_invoice(f.customer, d(2026, 3, 5), d(2026, 4, 5), one_line("10.00"))
            .map(|_| ()),
        _ => unreachable!(),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Oracle: post_entry INVARIANT 3 (and the same clause on every
    /// command): for random books and a random invalid command, the command
    /// is refused and every observable — entries, log, chart, periods, next
    /// ids, trial balance, tags, cleared flags — is identical before and
    /// after.
    #[test]
    fn a_refusal_leaves_the_books_unchanged(t in txs(), k in 0..INVALID_COMMANDS) {
        let mut f = fixture(&t);
        // The account-id allocator is checked by the ignored
        // `a_refused_account_never_uses_up_an_id` (a known disagreement).
        let before = observe_but_account_ids(&f.b.l);
        let r = invalid_command(&mut f, k);
        prop_assert!(r.is_err(), "command {} was accepted", k);
        prop_assert_eq!(observe_but_account_ids(&f.b.l), before, "command {} changed the books ({:?})", k, r);
    }

    /// Oracle: B-1/B-6 and post_entry INVARIANT 2 against the test's own
    /// model: every balanced random entry posts, the log is exactly the
    /// posting sequence, earlier entries never change, every balance equals
    /// the model on its normal side, and the trial balance balances.
    #[test]
    fn posted_books_match_the_model(t in txs()) {
        let mut b = books();
        let mut ids = Vec::new();
        let mut snaps = Vec::new();
        for x in &t {
            let id = post_tx(&mut b, x);
            for (i, s) in ids.iter().zip(&snaps) {
                prop_assert_eq!(&serde_json::to_value(b.l.get_entry(*i)).unwrap(), s);
            }
            ids.push(id);
            snaps.push(serde_json::to_value(b.l.get_entry(id)).unwrap());
        }
        let order: Vec<_> = b.l.posted_entries().map(|e| e.id()).collect();
        prop_assert_eq!(order, ids);
        let m = model(&t, u32::MAX);
        for (i, a) in postable(&b).iter().enumerate() {
            let want = if NORMAL_DEBIT[i] { m[i] } else { -m[i] };
            prop_assert_eq!(b.l.account_balance(*a).unwrap(), want);
        }
        prop_assert!(trial_balance(&b.l).balanced());
    }

    /// Oracle: close INVARIANTS 1-4 and B-13 against the test's own model,
    /// for random books closed 50/50 to members at Jan 31: revenue and
    /// expense are zero as of Jan 31; what remains is exactly February and
    /// March; the members' credits sum to January's net with A taking the
    /// odd cent (tie to the earlier member); A's distributions are swept;
    /// no earlier entry changes; the closing entry balances and is dated
    /// Jan 31.
    #[test]
    fn a_close_matches_the_model(t in txs()) {
        let mut b = books();
        for x in &t {
            post_tx(&mut b, x);
        }
        let jan = b.l.create_period("Jan", d(2026, 1, 1), d(2026, 1, 31)).unwrap();
        let before: Vec<_> = b.l.posted_entries().map(|e| serde_json::to_value(e).unwrap()).collect();
        let to = members(&[(b.cap_a, "0.5", Some(b.dist_a)), (b.cap_b, "0.5", None)]);
        let preview = b.l.preview_close_period_to(jan, &to).unwrap();
        let c = b.l.close_period_to(jan, &to).unwrap();
        prop_assert_eq!(c, preview);

        let jan_m = model(&t, 30);
        let all_m = model(&t, u32::MAX);
        // indices: 2 cap_a, 3 cap_b, 4 dist_a, 5 rev, 6 exp
        let net = -(jan_m[5] + jan_m[6]);
        let something = !jan_m[4].is_zero() || !jan_m[5].is_zero() || !jan_m[6].is_zero();
        prop_assert_eq!(c.is_some(), something);

        let after: Vec<_> = b.l.posted_entries().take(before.len()).map(|e| serde_json::to_value(e).unwrap()).collect();
        prop_assert_eq!(after, before);

        prop_assert!(!tb_has_code(&b.l, d(2026, 1, 31), "4000"));
        prop_assert!(!tb_has_code(&b.l, d(2026, 1, 31), "6000"));
        prop_assert!(!tb_has_code(&b.l, d(2026, 1, 31), "3002"));
        prop_assert_eq!(b.l.account_balance(b.rev).unwrap(), -(all_m[5] - jan_m[5]));
        prop_assert_eq!(b.l.account_balance(b.exp).unwrap(), all_m[6] - jan_m[6]);
        prop_assert_eq!(b.l.account_balance(b.dist_a).unwrap(), -(all_m[4] - jan_m[4]));

        // 50/50 by largest remainder: equal remainders, so the first member
        // takes the odd cent; a loss is the negated gain.
        let n_cents = (net * Decimal::from(100)).abs();
        let half_down = (n_cents / Decimal::from(2)).floor();
        let a_cents = n_cents - half_down;
        let sign = if net < Decimal::ZERO { -Decimal::ONE } else { Decimal::ONE };
        let share_a = sign * a_cents / Decimal::from(100);
        let share_b = sign * half_down / Decimal::from(100);
        prop_assert_eq!(share_a + share_b, net);
        prop_assert_eq!(b.l.account_balance(b.cap_a).unwrap(), -all_m[2] + share_a - jan_m[4]);
        prop_assert_eq!(b.l.account_balance(b.cap_b).unwrap(), -all_m[3] + share_b);

        if let Some(c) = c {
            let ce = b.l.get_entry(c).unwrap();
            prop_assert_eq!(ce.date(), d(2026, 1, 31));
            let (dr, cr) = lines_of(&b.l, c).into_iter().fold((Decimal::ZERO, Decimal::ZERO), |(dr, cr), (_, s, a)| {
                if s == Side::Debit { (dr + a, cr) } else { (dr, cr + a) }
            });
            prop_assert_eq!(dr, cr);
            // Equity is credited with exactly the net (the members' capital
            // movement, apart from the distribution sweep).
            let cap_net = -(entry_movement(&b.l, c, b.cap_a) + entry_movement(&b.l, c, b.cap_b)) + jan_m[4];
            prop_assert_eq!(cap_net, net);
        }
        prop_assert!(trial_balance(&b.l).balanced());
    }

    /// Oracle: check_reversal INVARIANTS 1-2: it never mutates, and it
    /// accepts exactly what reverse_entry accepts (same refusal otherwise),
    /// for random targets and dates around a closed January.
    #[test]
    fn check_reversal_agrees_with_reverse_entry(t in txs(), target in 0u64..40, dd in 0u32..120) {
        let f = fixture(&t);
        let before = observe(&f.b.l);
        let check = f.b.l.check_reversal(target, day(dd));
        prop_assert_eq!(observe(&f.b.l), before.clone());
        let mut l2 = f.b.l.clone();
        let act = l2.reverse_entry(target, day(dd), "r");
        match (&check, &act) {
            (Ok(()), Ok(_)) => {}
            (Err(a), Err(b2)) => {
                prop_assert_eq!(a, b2);
                prop_assert_eq!(observe(&l2), before);
            }
            _ => prop_assert!(false, "check {:?} but reverse {:?}", check, act),
        }
    }

    /// Oracle: preview_close_period_to doc: pure, and it succeeds exactly
    /// when close_period_to does, naming the same entry id (or None).
    #[test]
    fn a_close_preview_agrees_with_the_close(t in txs(), which in 0usize..4, target in 0usize..4) {
        let f = fixture(&t);
        let periods = [f.jan, f.feb, f.mar, 9_999];
        let tos = [
            members(&[(f.b.cap_a, "0.5", None), (f.b.cap_b, "0.5", None)]),
            members(&[(f.b.cap_a, "0.7", None), (f.b.cap_b, "0.4", None)]),
            CloseTo::RetainedEarnings { sweep: vec![] },
            CloseTo::Account { into: f.b.cap_a, sweep: vec![f.b.dist_a] },
        ];
        let before = observe(&f.b.l);
        let preview = f.b.l.preview_close_period_to(periods[which], &tos[target]);
        prop_assert_eq!(observe(&f.b.l), before.clone());
        let mut l2 = f.b.l.clone();
        let act = l2.close_period_to(periods[which], &tos[target]);
        prop_assert_eq!(&preview, &act);
        if act.is_err() {
            prop_assert_eq!(observe(&l2), before);
        }
    }

    /// Oracle: JournalEntry::validate is the gate (post_entry INVARIANT 1):
    /// for a random two-line draft (possibly unbalanced, zero, or negative),
    /// post succeeds exactly when validate does, with the same reason.
    #[test]
    fn post_accepts_exactly_what_validate_accepts(dr in -3i64..300, cr in -3i64..300) {
        let mut b = books();
        let e = b.l.try_create_entry(d(2026, 3, 1), "x").unwrap();
        // replay_add_line skips the account check only, so amounts that
        // add_line might refuse early still reach validate.
        let r1 = b.l.replay_add_line(e, b.exp, Side::Debit, Money::new(cents(dr), Currency::USD));
        let r2 = b.l.replay_add_line(e, b.cash, Side::Credit, Money::new(cents(cr), Currency::USD));
        prop_assume!(r1.is_ok() && r2.is_ok());
        let v = b.l.get_entry(e).unwrap().validate(b.l.chart());
        let expected = if dr == 0 || cr == 0 {
            Err(LedgerError::ZeroAmountLine)
        } else if dr < 0 || cr < 0 {
            Err(LedgerError::NegativeAmountLine)
        } else if dr != cr {
            Err(LedgerError::UnbalancedEntry { debits: cents(dr), credits: cents(cr) })
        } else {
            Ok(())
        };
        // Zero and negative may be reported in either order when both occur.
        if (dr == 0 || cr == 0) && (dr < 0 || cr < 0) {
            prop_assert!(matches!(v, Err(LedgerError::ZeroAmountLine | LedgerError::NegativeAmountLine)));
        } else {
            prop_assert_eq!(&v, &expected);
        }
        prop_assert_eq!(b.l.post_entry(e), v);
    }
}
