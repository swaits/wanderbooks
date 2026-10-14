//! Invariant tests: the gates every change must pass.
//!
//! These are not example tests. They encode the accounting laws the engine
//! exists to uphold: entries balance, posted entries are immutable, the trial
//! balance balances, and the accounting equation holds. A feature that breaks
//! any of these is not a feature.
//!
//! Plus the golden test: the canonical fixture company's reports must match
//! `golden/fixture_company.json` byte-for-value. That file is the seed of the
//! semantic oracle (see `docs/oracle.md`).

use chrono::NaiveDate;
use ledger_core::reports::{balance_sheet, snapshot, trial_balance, unclosed_earnings};
use ledger_core::{
    AccountId, AccountType, ChartOfAccounts, Currency, EntryStatus, JournalEntry, Ledger,
    LedgerError, Money, Side,
};
use rust_decimal::Decimal;

// ---------------------------------------------------------------------------
// Fixture: a tiny service company, January 2026.
// ---------------------------------------------------------------------------

fn date(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 1, day).unwrap()
}

fn usd(amount: u64) -> Money {
    Money::new(Decimal::from(amount), Currency::USD)
}

/// Post a balanced entry in one call. Panics on programmer error: fixtures
/// must be valid by construction.
fn post(ledger: &mut Ledger, date: NaiveDate, description: &str, lines: &[(AccountId, Side, u64)]) {
    let id = ledger.create_entry(date, description);
    for (account, side, amount) in lines {
        ledger.add_line(id, *account, *side, usd(*amount)).unwrap();
    }
    ledger.post_entry(id).unwrap();
}

struct Company {
    ledger: Ledger,
    cash: AccountId,
    ar: AccountId,
    equipment: AccountId,
    _ap: AccountId,
    equity: AccountId,
    revenue: AccountId,
    rent: AccountId,
}

fn fixture_company() -> Company {
    let mut ledger = Ledger::new(Currency::USD);
    let cash = ledger
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let ar = ledger
        .add_account(
            "1200",
            "Accounts Receivable",
            AccountType::Asset,
            None,
            false,
        )
        .unwrap();
    let equipment = ledger
        .add_account("1500", "Equipment", AccountType::Asset, None, false)
        .unwrap();
    let ap = ledger
        .add_account(
            "2000",
            "Accounts Payable",
            AccountType::Liability,
            None,
            false,
        )
        .unwrap();
    let equity = ledger
        .add_account("3000", "Owner's Equity", AccountType::Equity, None, false)
        .unwrap();
    let revenue = ledger
        .add_account("4000", "Service Revenue", AccountType::Revenue, None, false)
        .unwrap();
    let rent = ledger
        .add_account("6000", "Rent Expense", AccountType::Expense, None, false)
        .unwrap();

    // Owner invests $10,000 cash.
    post(
        &mut ledger,
        date(2),
        "Owner investment",
        &[(cash, Side::Debit, 10_000), (equity, Side::Credit, 10_000)],
    );
    // Buy equipment for $2,500 cash.
    post(
        &mut ledger,
        date(5),
        "Buy equipment",
        &[(equipment, Side::Debit, 2_500), (cash, Side::Credit, 2_500)],
    );
    // Bill a client $4,000.
    post(
        &mut ledger,
        date(10),
        "Invoice client",
        &[(ar, Side::Debit, 4_000), (revenue, Side::Credit, 4_000)],
    );
    // Pay January rent, $800.
    post(
        &mut ledger,
        date(15),
        "January rent",
        &[(rent, Side::Debit, 800), (cash, Side::Credit, 800)],
    );
    // Collect $1,500 of receivables.
    post(
        &mut ledger,
        date(20),
        "Client payment",
        &[(cash, Side::Debit, 1_500), (ar, Side::Credit, 1_500)],
    );

    Company {
        ledger,
        cash,
        ar,
        equipment,
        _ap: ap,
        equity,
        revenue,
        rent,
    }
}

// ---------------------------------------------------------------------------
// Validation: the engine refuses bad entries.
// ---------------------------------------------------------------------------

#[test]
fn unbalanced_entry_is_rejected() {
    let c = fixture_company();
    let mut ledger = c.ledger;
    let id = ledger.create_entry(date(21), "Bad entry");
    ledger.add_line(id, c.cash, Side::Debit, usd(100)).unwrap();
    ledger
        .add_line(id, c.equity, Side::Credit, usd(99))
        .unwrap();
    let err = ledger.post_entry(id).unwrap_err();
    assert_eq!(
        err,
        LedgerError::UnbalancedEntry {
            debits: Decimal::from(100),
            credits: Decimal::from(99),
        }
    );
}

#[test]
fn single_line_entry_is_rejected() {
    let c = fixture_company();
    let mut ledger = c.ledger;
    let id = ledger.create_entry(date(21), "One line");
    ledger.add_line(id, c.cash, Side::Debit, usd(100)).unwrap();
    assert_eq!(ledger.post_entry(id).unwrap_err(), LedgerError::TooFewLines);
}

#[test]
fn zero_amount_line_is_rejected() {
    let c = fixture_company();
    let mut ledger = c.ledger;
    let id = ledger.create_entry(date(21), "Zero line");
    ledger.add_line(id, c.cash, Side::Debit, usd(100)).unwrap();
    ledger.add_line(id, c.cash, Side::Debit, usd(0)).unwrap();
    ledger
        .add_line(id, c.equity, Side::Credit, usd(100))
        .unwrap();
    assert_eq!(
        ledger.post_entry(id).unwrap_err(),
        LedgerError::ZeroAmountLine
    );
}

#[test]
fn negative_amount_line_is_rejected() {
    let c = fixture_company();
    let mut ledger = c.ledger;
    let id = ledger.create_entry(date(21), "Negative line");
    let minus = Money::new(Decimal::from(-100), Currency::USD);
    ledger.add_line(id, c.cash, Side::Debit, minus).unwrap();
    ledger.add_line(id, c.equity, Side::Credit, minus).unwrap();
    assert_eq!(
        ledger.post_entry(id).unwrap_err(),
        LedgerError::NegativeAmountLine
    );
}

#[test]
fn out_of_range_amount_is_rejected() {
    let c = fixture_company();
    let mut ledger = c.ledger;
    // Big enough to overflow a sum: an error, never a panic.
    let id = ledger.create_entry(date(21), "Too big");
    let big = Money::new(Decimal::MAX, Currency::USD);
    ledger.add_line(id, c.cash, Side::Debit, big).unwrap();
    ledger.add_line(id, c.cash, Side::Debit, big).unwrap();
    ledger.add_line(id, c.equity, Side::Credit, big).unwrap();
    assert_eq!(
        ledger.post_entry(id).unwrap_err(),
        LedgerError::AmountOutOfRange
    );
    // Too precise: refused as soon as it is added (finer than a cent)...
    let id = ledger.create_entry(date(21), "Too precise");
    let tiny = Money::new("0.00000000001".parse().unwrap(), Currency::USD);
    assert!(matches!(
        ledger.add_line(id, c.cash, Side::Debit, tiny),
        Err(LedgerError::FinerThanMinorUnit { places: 2, .. })
    ));
    // ...and out of range even for books replayed from before that rule.
    ledger
        .replay_add_line(id, c.cash, Side::Debit, tiny)
        .unwrap();
    ledger
        .replay_add_line(id, c.equity, Side::Credit, tiny)
        .unwrap();
    assert_eq!(
        ledger.replay_post_entry(id).unwrap_err(),
        LedgerError::AmountOutOfRange
    );
}

/// B-3 (DECISION, Stephen 2026-10-13: a ledger in one money holds no
/// fractions of its smallest unit). Regression: a payroll run posted
/// $65.27794 (found by the store's refusal property in CI, 2026-10-13).
#[test]
fn no_entry_posts_a_fraction_of_the_minor_unit() {
    let c = fixture_company();
    let mut ledger = c.ledger;
    let usd = |s: &str| Money::new(s.parse().unwrap(), Currency::USD);
    // Whole cents, with or without trailing zeros, are fine.
    for ok in ["65.27", "65.270", "65"] {
        let id = ledger.create_entry(date(21), "cents");
        ledger.add_line(id, c.cash, Side::Debit, usd(ok)).unwrap();
        ledger
            .add_line(id, c.equity, Side::Credit, usd(ok))
            .unwrap();
        ledger.post_entry(id).unwrap();
    }
    // A fraction of a cent is refused when added...
    let id = ledger.create_entry(date(21), "sub-cent");
    assert!(matches!(
        ledger.add_line(id, c.cash, Side::Debit, usd("65.27794")),
        Err(LedgerError::FinerThanMinorUnit { places: 2, .. })
    ));
    // ...and when posted, whatever path put the line on the draft.
    ledger
        .replay_add_line(id, c.cash, Side::Debit, usd("65.275"))
        .unwrap();
    ledger
        .replay_add_line(id, c.equity, Side::Credit, usd("65.275"))
        .unwrap();
    assert!(matches!(
        ledger.post_entry(id),
        Err(LedgerError::FinerThanMinorUnit { .. })
    ));
    assert_eq!(
        ledger.get_entry(id).unwrap().status(),
        ledger_core::EntryStatus::Draft
    );
    // Books from before the rule still replay as booked.
    ledger.replay_post_entry(id).unwrap();
}

#[test]
fn fx_overflow_is_an_error() {
    let c = fixture_company();
    let mut ledger = c.ledger;
    let id = ledger.create_entry(date(21), "Huge FX");
    assert_eq!(
        ledger
            .add_line_fx(
                id,
                c.cash,
                Side::Debit,
                Currency::EUR,
                Decimal::MAX,
                Decimal::from(2)
            )
            .unwrap_err(),
        LedgerError::AmountOutOfRange
    );
}

#[test]
fn unknown_account_is_rejected() {
    // Refused when the line is added (since 2026-10-13); a line that got
    // in anyway (replayed from an older journal) is still refused at post.
    let c = fixture_company();
    let mut ledger = c.ledger;
    let id = ledger.create_entry(date(21), "Ghost account");
    ledger.add_line(id, c.cash, Side::Debit, usd(100)).unwrap();
    assert_eq!(
        ledger.add_line(id, 999_999, Side::Credit, usd(100)),
        Err(LedgerError::UnknownAccount(999_999))
    );
    ledger
        .replay_add_line(id, 999_999, Side::Credit, usd(100))
        .unwrap();
    assert_eq!(
        ledger.post_entry(id).unwrap_err(),
        LedgerError::UnknownAccount(999_999)
    );
}

#[test]
fn placeholder_account_is_rejected() {
    let c = fixture_company();
    let mut ledger = c.ledger;
    let header = ledger
        .add_account("100", "Assets header", AccountType::Asset, None, true)
        .unwrap();
    let id = ledger.create_entry(date(21), "Post to header");
    assert_eq!(
        ledger.add_line(id, header, Side::Debit, usd(100)),
        Err(LedgerError::PlaceholderAccount("100".to_string()))
    );
    // Got in anyway (an older journal): still refused at post.
    ledger
        .replay_add_line(id, header, Side::Debit, usd(100))
        .unwrap();
    ledger
        .add_line(id, c.equity, Side::Credit, usd(100))
        .unwrap();
    assert_eq!(
        ledger.post_entry(id).unwrap_err(),
        LedgerError::PlaceholderAccount("100".to_string())
    );
}

#[test]
fn foreign_currency_line_is_rejected() {
    let c = fixture_company();
    let mut ledger = c.ledger;
    let id = ledger.create_entry(date(21), "Euro line");
    let eur = Money::new(Decimal::from(100), Currency::EUR);
    let err = ledger.add_line(id, c.cash, Side::Debit, eur).unwrap_err();
    assert_eq!(
        err,
        LedgerError::CurrencyMismatch {
            expected: "USD".to_string(),
            found: "EUR".to_string(),
        }
    );
}

/// R-1 / B-3: one currency throughout an entry. Even when every line
/// matches its own account's currency (a chart the ledger never builds
/// today, R-5), an entry mixing currencies is refused.
#[test]
fn an_entry_mixing_currencies_is_refused_even_on_matching_accounts() {
    let c = fixture_company();
    let mut ledger = c.ledger;
    let id = ledger.create_entry(date(21), "two moneys");
    ledger.add_line(id, c.cash, Side::Debit, usd(100)).unwrap();
    ledger
        .add_line(id, c.revenue, Side::Credit, usd(100))
        .unwrap();
    let mut json = serde_json::to_value(ledger.get_entry(id).unwrap()).unwrap();
    json["lines"][1]["amount"]["currency"] = serde_json::to_value(Currency::EUR).unwrap();
    let mixed: JournalEntry = serde_json::from_value(json).unwrap();

    let mut chart = ChartOfAccounts::default();
    chart
        .add(ledger.chart().get(c.cash).unwrap().clone())
        .unwrap();
    let mut revenue = ledger.chart().get(c.revenue).unwrap().clone();
    revenue.currency = Currency::EUR;
    chart.add(revenue).unwrap();

    assert_eq!(
        mixed.validate(&chart),
        Err(LedgerError::CurrencyMismatch {
            expected: "USD".to_string(),
            found: "EUR".to_string(),
        })
    );
}

#[test]
fn duplicate_account_code_is_rejected() {
    let c = fixture_company();
    let mut ledger = c.ledger;
    let err = ledger
        .add_account("1000", "Cash again", AccountType::Asset, None, false)
        .unwrap_err();
    assert_eq!(err, LedgerError::DuplicateAccountCode("1000".to_string()));
}

// ---------------------------------------------------------------------------
// Immutability: posted entries can only be corrected by reversal.
// ---------------------------------------------------------------------------

#[test]
fn posted_entry_is_immutable() {
    let c = fixture_company();
    let mut ledger = c.ledger;
    let id = ledger.create_entry(date(21), "To post");
    ledger.add_line(id, c.cash, Side::Debit, usd(50)).unwrap();
    ledger
        .add_line(id, c.equity, Side::Credit, usd(50))
        .unwrap();
    ledger.post_entry(id).unwrap();

    // Cannot add lines after posting.
    assert_eq!(
        ledger
            .add_line(id, c.cash, Side::Debit, usd(1))
            .unwrap_err(),
        LedgerError::PostedEntryImmutable
    );
    // Cannot post twice.
    assert_eq!(
        ledger.post_entry(id).unwrap_err(),
        LedgerError::EntryAlreadyPosted(id)
    );
    assert_eq!(ledger.get_entry(id).unwrap().status(), EntryStatus::Posted);
}

#[test]
fn reversal_nets_to_zero_and_links_back() {
    let c = fixture_company();
    let mut ledger = c.ledger;
    let id = ledger.create_entry(date(21), "Mistake");
    ledger.add_line(id, c.rent, Side::Debit, usd(500)).unwrap();
    ledger.add_line(id, c.cash, Side::Credit, usd(500)).unwrap();
    ledger.post_entry(id).unwrap();
    assert_eq!(ledger.account_movement(c.rent), Decimal::from(1300));

    let rev = ledger
        .reverse_entry(id, date(22), "Reverse mistake")
        .unwrap();
    let rev_entry = ledger.get_entry(rev).unwrap();
    assert_eq!(rev_entry.reversal_of(), Some(id));
    assert_eq!(rev_entry.status(), EntryStatus::Posted);

    // The mistake and its reversal cancel exactly: back to fixture values.
    assert_eq!(ledger.account_movement(c.rent), Decimal::from(800));
    assert_eq!(ledger.account_movement(c.cash), Decimal::from(8200));
    // And the books still balance everywhere.
    assert!(trial_balance(&ledger).balanced());
    assert!(balance_sheet(&ledger).unwrap().balanced());
}

/// A posted mistake, for the reversal rules below.
fn posted_mistake(ledger: &mut Ledger, c: &Company) -> ledger_core::EntryId {
    let id = ledger.create_entry(date(21), "Mistake");
    ledger.add_line(id, c.rent, Side::Debit, usd(500)).unwrap();
    ledger.add_line(id, c.cash, Side::Credit, usd(500)).unwrap();
    ledger.post_entry(id).unwrap();
    id
}

/// Spec (Stephen, 2026-10-13): an entry is reversed at most once. A second
/// reversal would undo the original twice.
#[test]
fn an_entry_is_reversed_at_most_once() {
    let c = fixture_company();
    let mut ledger = c.ledger.clone();
    let id = posted_mistake(&mut ledger, &c);
    let rev = ledger.reverse_entry(id, date(22), "Reverse").unwrap();
    let before = ledger.next_entry_id();
    assert_eq!(
        ledger.reverse_entry(id, date(23), "Again").unwrap_err(),
        LedgerError::EntryAlreadyReversed(id, rev)
    );
    // Refused means untouched: no entry allocated, no draft left behind.
    assert_eq!(ledger.next_entry_id(), before);
    assert_eq!(ledger.account_movement(c.rent), Decimal::from(800));
}

/// Spec: reversing the reversal re-instates the original, and is allowed
/// (once, like any other entry).
#[test]
fn a_reversal_can_itself_be_reversed_once() {
    let c = fixture_company();
    let mut ledger = c.ledger.clone();
    let id = posted_mistake(&mut ledger, &c);
    let rev = ledger.reverse_entry(id, date(22), "Reverse").unwrap();
    let back = ledger.reverse_entry(rev, date(23), "Re-instate").unwrap();
    assert_eq!(ledger.account_movement(c.rent), Decimal::from(1300));
    assert_eq!(
        ledger.reverse_entry(rev, date(24), "Again").unwrap_err(),
        LedgerError::EntryAlreadyReversed(rev, back)
    );
}

/// Spec (Stephen, 2026-10-13): a closing entry is never reversed. Undoing
/// it would leave the period marked closed with its earnings unswept.
#[test]
fn a_closing_entry_is_never_reversed() {
    let c = fixture_company();
    let mut ledger = c.ledger.clone();
    let jan = ledger.create_period("2026-01", date(1), date(31)).unwrap();
    let closing = ledger.close_period(jan).unwrap().unwrap();
    let feb = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
    let before = ledger.next_entry_id();
    assert_eq!(
        ledger
            .reverse_entry(closing, feb, "Undo close")
            .unwrap_err(),
        LedgerError::ClosingEntryIrreversible(closing)
    );
    assert_eq!(ledger.next_entry_id(), before);
}

/// Regression (max-testing pass, 2026-10-13): a reversal dated in a closed
/// period was refused only at post time, after allocating an id and
/// leaving a draft behind. Refused means untouched.
#[test]
fn a_reversal_into_a_closed_period_leaves_no_trace() {
    let c = fixture_company();
    let mut ledger = c.ledger.clone();
    let id = posted_mistake(&mut ledger, &c);
    let jan = ledger.create_period("2026-01", date(1), date(31)).unwrap();
    // Nothing is open in January now except our posted mistake; close it.
    ledger.close_period(jan).unwrap();
    let before = ledger.next_entry_id();
    assert!(matches!(
        ledger.reverse_entry(id, date(25), "Into the closed month"),
        Err(LedgerError::ClosedPeriod(..))
    ));
    assert_eq!(ledger.next_entry_id(), before);
    assert!(ledger.check_reversal(id, date(25)).is_err());
}

// ---------------------------------------------------------------------------
// Reports: the accounting laws hold.
// ---------------------------------------------------------------------------

#[test]
fn trial_balance_balances() {
    let c = fixture_company();
    let tb = trial_balance(&c.ledger);
    assert!(tb.balanced());
    assert_eq!(tb.total_debits, Decimal::from(14_000));
    assert_eq!(tb.total_credits, Decimal::from(14_000));
    // Six accounts with non-zero balances, sorted by code.
    let codes: Vec<_> = tb.lines.iter().map(|l| l.code.as_str()).collect();
    assert_eq!(codes, vec!["1000", "1200", "1500", "3000", "4000", "6000"]);
}

#[test]
fn accounting_equation_holds() {
    let c = fixture_company();
    let bs = balance_sheet(&c.ledger).unwrap();
    assert!(bs.balanced());
    assert_eq!(bs.assets, Decimal::from(13_200));
    assert_eq!(bs.liabilities, Decimal::ZERO);
    // Equity = owner's capital 10,000 + net income 3,200.
    assert_eq!(bs.equity, Decimal::from(13_200));
    assert_eq!(bs.current_earnings, Decimal::from(3_200));
}

#[test]
fn profit_and_loss_is_correct() {
    let c = fixture_company();
    let pl = unclosed_earnings(&c.ledger).unwrap();
    assert_eq!(pl.revenue, Decimal::from(4_000));
    assert_eq!(pl.expenses, Decimal::from(800));
    assert_eq!(pl.net_income(), Decimal::from(3_200));
}

// ---------------------------------------------------------------------------
// Property-ish test: random balanced journals preserve every invariant.
// ---------------------------------------------------------------------------

/// Tiny deterministic PRNG. No dependencies, reproducible seeds.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) % bound
    }
}

/// Split `total` into `parts` positive whole-unit pieces.
fn split_total(rng: &mut Lcg, total: u64, parts: usize) -> Vec<u64> {
    assert!(total >= parts as u64 && parts >= 1);
    let mut cuts = vec![0u64, total];
    while cuts.len() < parts + 1 {
        let c = rng.next(total - 1) + 1; // 1..total
        if !cuts.contains(&c) {
            cuts.push(c);
        }
    }
    cuts.sort_unstable();
    cuts.windows(2).map(|w| w[1] - w[0]).collect()
}

#[test]
fn random_balanced_journals_preserve_invariants() {
    let mut rng = Lcg(0x1234_5678_9abc_def0);
    let c = fixture_company();
    let mut ledger = c.ledger;
    let accounts = [
        c.cash,
        c.ar,
        c.equipment,
        c._ap,
        c.equity,
        c.revenue,
        c.rent,
    ];

    for i in 0..300u32 {
        let total = rng.next(100_000) + 100;
        let nd = (rng.next(3) + 1) as usize;
        let nc = (rng.next(3) + 1) as usize;
        let id = ledger.create_entry(date(21), &format!("random {i}"));
        for part in split_total(&mut rng, total, nd) {
            let acct = accounts[rng.next(accounts.len() as u64) as usize];
            ledger.add_line(id, acct, Side::Debit, usd(part)).unwrap();
        }
        for part in split_total(&mut rng, total, nc) {
            let acct = accounts[rng.next(accounts.len() as u64) as usize];
            ledger.add_line(id, acct, Side::Credit, usd(part)).unwrap();
        }
        ledger.post_entry(id).unwrap();
    }

    // Every invariant still holds after 300 random balanced entries.
    let tb = trial_balance(&ledger);
    assert!(tb.balanced());
    assert!(balance_sheet(&ledger).unwrap().balanced());

    // Conservation: the sum of all account movements is exactly zero.
    let mut sum = Decimal::ZERO;
    for a in accounts {
        sum += ledger.account_movement(a);
    }
    assert!(sum.is_zero());
}

// ---------------------------------------------------------------------------
// Golden test: the oracle seed.
// ---------------------------------------------------------------------------

#[test]
fn golden_fixture_company_reports() {
    let c = fixture_company();
    let snap = snapshot(&c.ledger).unwrap();
    let golden: serde_json::Value =
        serde_json::from_str(include_str!("golden/fixture_company.json")).unwrap();
    assert_eq!(snap, golden);
}

#[test]
fn parent_account_validation() {
    let mut ledger = Ledger::new(Currency::USD);
    // Unknown parent is rejected.
    let err = ledger
        .add_account("1100", "Child", AccountType::Asset, Some(999), false)
        .unwrap_err();
    assert_eq!(err, LedgerError::UnknownParentAccount(999));
    // A real parent works, and the child records it.
    let parent = ledger
        .add_account("1000", "Parent", AccountType::Asset, None, true)
        .unwrap();
    let child = ledger
        .add_account("1100", "Child", AccountType::Asset, Some(parent), false)
        .unwrap();
    assert_eq!(ledger.chart().get(child).unwrap().parent, Some(parent));
    // Chart accessors report honestly.
    assert!(Ledger::new(Currency::USD).chart().is_empty());
    assert!(!ledger.chart().is_empty());
    assert_eq!(ledger.chart().len(), 2);
    assert!(ledger.chart().find_by_code("1000").is_some());
    assert!(ledger.chart().find_by_code("nope").is_none());
}

#[test]
fn account_type_balance_sheet_predicate() {
    assert!(AccountType::Asset.is_balance_sheet());
    assert!(AccountType::Liability.is_balance_sheet());
    assert!(AccountType::Equity.is_balance_sheet());
    assert!(!AccountType::Revenue.is_balance_sheet());
    assert!(!AccountType::Expense.is_balance_sheet());
}

#[test]
fn entry_movements_sum_to_zero() {
    let mut c = fixture_company();
    let cash = c.cash;
    let revenue = c.revenue;
    let id = c.ledger.create_entry(date(15), "sale");
    c.ledger.add_line(id, cash, Side::Debit, usd(1000)).unwrap();
    c.ledger
        .add_line(id, revenue, Side::Credit, usd(1000))
        .unwrap();
    c.ledger.post_entry(id).unwrap();
    let entry = c.ledger.get_entry(id).unwrap();
    let m = entry.movements();
    assert_eq!(m[&cash], Decimal::new(100000, 2));
    assert_eq!(m[&revenue], Decimal::new(-100000, 2));
    let total: Decimal = m.values().copied().sum();
    assert_eq!(total, Decimal::ZERO);
}

// ---------------------------------------------------------------------------
// Cash flow statement (indirect method).
// ---------------------------------------------------------------------------

use ledger_core::NewAsset;
use ledger_core::reports::{CashFlowParams, cash_flow};

fn cash_flow_params(cash: AccountId) -> CashFlowParams {
    CashFlowParams {
        start: date(1),
        end: date(31),
        cash_accounts: vec![cash],
        financing_accounts: vec![],
        investing_accounts: vec![],
    }
}

#[test]
fn cash_flow_reconciles_to_change_in_cash() {
    let c = fixture_company();
    let cf = cash_flow(&c.ledger, &cash_flow_params(c.cash)).unwrap();

    // The three sections must sum to the net change in cash. This is the
    // accounting law the statement exists to uphold.
    let sections = cf.cash_from_operations + cf.cash_from_investing + cf.cash_from_financing;
    assert_eq!(sections, cf.net_change_in_cash);
    assert_eq!(cf.cash_ending - cf.cash_beginning, cf.net_change_in_cash);

    // Hand-computed from the fixture:
    // Cash: +10,000 (invest) - 2,500 (equipment) - 800 (rent) + 1,500 (collect)
    //     = 8,200. Beginning 0.
    assert_eq!(cf.cash_beginning, Decimal::ZERO);
    assert_eq!(cf.cash_ending, Decimal::new(820000, 2));
    assert_eq!(cf.net_change_in_cash, Decimal::new(820000, 2));

    // Net income: 4,000 revenue - 800 rent = 3,200.
    assert_eq!(cf.net_income, Decimal::new(320000, 2));
    // No depreciation: the fixture's equipment is not a registered asset.
    assert_eq!(cf.depreciation, Decimal::ZERO);

    // Operating: 3,200 net income
    //   - 2,500 increase in AR (4,000 billed - 1,500 collected)
    //   - 2,500 equipment purchase (not a registered asset: working capital)
    //   + 10,000 owner investment is equity -> financing, not operating.
    // Financing: +10,000 owner contribution.
    // => operating = 3,200 - 2,500 - 2,500 = -1,800
    // => investing = 0, financing = 10,000. Sum = 8,200. Reconciles.
    assert_eq!(cf.cash_from_operations, Decimal::new(-180000, 2));
    assert_eq!(cf.cash_from_investing, Decimal::ZERO);
    assert_eq!(cf.cash_from_financing, Decimal::new(1000000, 2));
}

#[test]
fn cash_flow_with_registered_asset_and_disposal() {
    let mut ledger = Ledger::new(Currency::USD);
    let cash = ledger
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let cost = ledger
        .add_account("1500", "Equipment at cost", AccountType::Asset, None, false)
        .unwrap();
    let accum = ledger
        .add_account(
            "1501",
            "Accum. depr. — equip.",
            AccountType::Asset,
            None,
            false,
        )
        .unwrap();
    let depr_exp = ledger
        .add_account(
            "6000",
            "Depreciation expense",
            AccountType::Expense,
            None,
            false,
        )
        .unwrap();
    ledger
        .add_account("4000", "Service Revenue", AccountType::Revenue, None, false)
        .unwrap();
    let gain = ledger
        .add_account("4100", "Gain on sale", AccountType::Revenue, None, false)
        .unwrap();
    let equity = ledger
        .add_account("3000", "Owner's Equity", AccountType::Equity, None, false)
        .unwrap();

    // Owner invests $100,000.
    post(
        &mut ledger,
        date(2),
        "Owner investment",
        &[
            (cash, Side::Debit, 100_000),
            (equity, Side::Credit, 100_000),
        ],
    );
    // Register a $60,000 van, 60-month life, bought Jan 15.
    let asset = ledger
        .register_asset(NewAsset {
            description: "Van".to_string(),
            purchase_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            cost: usd(60_000),
            salvage_value: usd(0),
            useful_life_months: 60,
            cost_account: cost,
            accum_depr_account: accum,
            expense_account: depr_exp,
        })
        .unwrap();
    // Buy the van for cash.
    post(
        &mut ledger,
        date(15),
        "Buy van",
        &[(cost, Side::Debit, 60_000), (cash, Side::Credit, 60_000)],
    );
    // January depreciation: $1,000 * 17/31 = $548.39.
    post(
        &mut ledger,
        date(31),
        "January depreciation",
        &[(depr_exp, Side::Debit, 548), (accum, Side::Credit, 548)],
    );
    // Sell the van Jan 31 for $59,600 cash. Book value 60,000 - 548 = 59,452.
    // Gain = 59,600 - 59,452 = 148.
    // Note: amounts are whole dollars here to keep the fixture simple.
    let _ = asset;
    post(
        &mut ledger,
        date(31),
        "Sell van",
        &[
            (cash, Side::Debit, 59_600),
            (accum, Side::Debit, 548),
            (cost, Side::Credit, 60_000),
            (gain, Side::Credit, 148),
        ],
    );

    let cf = cash_flow(&ledger, &cash_flow_params(cash)).unwrap();

    // Reconciliation is the law.
    let sections = cf.cash_from_operations + cf.cash_from_investing + cf.cash_from_financing;
    assert_eq!(sections, cf.net_change_in_cash);

    // Cash: 100,000 - 60,000 + 59,600 = 99,600.
    assert_eq!(cf.cash_ending, Decimal::new(9960000, 2));
    // Net income: 148 gain - 548 depreciation = -400.
    assert_eq!(cf.net_income, Decimal::new(-40000, 2));
    // Depreciation added back: 548.
    assert_eq!(cf.depreciation, Decimal::new(54800, 2));
    // Operating: -400 + 548 - 148 (gain removed) = 0.
    assert_eq!(cf.cash_from_operations, Decimal::ZERO);
    // Investing: -60,000 purchase + 59,600 proceeds = -400.
    assert_eq!(cf.cash_from_investing, Decimal::new(-40000, 2));
    // Financing: +100,000 owner investment.
    assert_eq!(cf.cash_from_financing, Decimal::new(10000000, 2));
    // Sum: 0 - 400 + 100,000 = 99,600. Matches cash.
}

// ---------------------------------------------------------------------------
// Control accounts (Stephen, 2026-10-13): the engine posts invoices, bills,
// payroll, and closes to accounts it finds by code. When an account with
// that code exists but is the wrong kind, it refuses, says why, and changes
// nothing, rather than posting to it or forcing the chart into its
// numbering. SOURCE: rules.md B-4 (the elements), DECISION.
// ---------------------------------------------------------------------------

fn books_with(code: &str, name: &str, t: AccountType, placeholder: bool) -> Ledger {
    let mut l = Ledger::new(Currency::USD);
    l.add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    l.add_account(code, name, t, None, placeholder).unwrap();
    l
}

fn doc(amount: u64) -> Vec<ledger_core::DocumentLine> {
    vec![ledger_core::DocumentLine {
        description: "work".into(),
        amount: Decimal::from(amount),
    }]
}

/// Refused means untouched: same chart, same ids, nothing posted.
fn assert_untouched(before: &Ledger, after: &Ledger) {
    assert_eq!(after.chart().len(), before.chart().len());
    assert_eq!(after.next_entry_id(), before.next_entry_id());
    assert_eq!(after.next_invoice_id(), before.next_invoice_id());
    assert_eq!(after.next_bill_id(), before.next_bill_id());
    assert_eq!(
        after.posted_entries().count(),
        before.posted_entries().count()
    );
}

#[test]
fn an_invoice_refuses_a_1200_that_is_not_receivables() {
    let mut l = books_with("1200", "Customer Deposits", AccountType::Liability, false);
    let c = l.add_customer("Acme").unwrap();
    let before = l.clone();
    let err = l
        .create_invoice(c, date(5), date(30), doc(100))
        .unwrap_err();
    assert_eq!(
        err,
        LedgerError::ControlAccountMismatch {
            code: "1200".into(),
            name: "Customer Deposits".into(),
            role: "accounts receivable".into(),
            expected: AccountType::Asset,
            found: AccountType::Liability,
            placeholder: false,
        }
    );
    // The reason names the account, what it is, and what was needed.
    let why = err.to_string();
    assert!(
        why.contains("1200") && why.contains("Customer Deposits"),
        "{why}"
    );
    assert!(why.contains("Liability") && why.contains("Asset"), "{why}");
    assert_untouched(&before, &l);
}

#[test]
fn an_invoice_refuses_a_placeholder_1200() {
    let mut l = books_with("1200", "Receivables", AccountType::Asset, true);
    let c = l.add_customer("Acme").unwrap();
    let before = l.clone();
    assert!(matches!(
        l.create_invoice(c, date(5), date(30), doc(100)),
        Err(LedgerError::ControlAccountMismatch {
            placeholder: true,
            ..
        })
    ));
    assert_untouched(&before, &l);
}

#[test]
fn an_invoice_creates_nothing_when_its_second_account_is_wrong() {
    // 1200 is missing (would be created) but 4000 is the wrong type: the
    // refusal must come before 1200 is added.
    let mut l = books_with("4000", "Owner Loans", AccountType::Liability, false);
    let c = l.add_customer("Acme").unwrap();
    let before = l.clone();
    assert!(matches!(
        l.create_invoice(c, date(5), date(30), doc(100)),
        Err(LedgerError::ControlAccountMismatch { .. })
    ));
    assert!(l.chart().find_by_code("1200").is_none());
    assert_untouched(&before, &l);
}

#[test]
fn a_bill_refuses_a_2000_that_is_not_payables() {
    let mut l = books_with("2000", "Savings", AccountType::Asset, false);
    let v = l.add_vendor("Vend").unwrap();
    let before = l.clone();
    assert!(matches!(
        l.create_bill(v, date(5), date(30), doc(100)),
        Err(LedgerError::ControlAccountMismatch {
            expected: AccountType::Liability,
            ..
        })
    ));
    assert_untouched(&before, &l);
}

#[test]
fn a_close_refuses_a_3999_that_is_not_equity() {
    let mut l = books_with("3999", "Suspense", AccountType::Asset, false);
    let sales = l
        .add_account("4000", "Sales", AccountType::Revenue, None, false)
        .unwrap();
    let e = l.create_entry(date(5), "sale");
    l.add_line(e, 1, Side::Debit, usd(100)).unwrap();
    l.add_line(e, sales, Side::Credit, usd(100)).unwrap();
    l.post_entry(e).unwrap();
    let jan = l.create_period("2026-01", date(1), date(31)).unwrap();
    let before = l.clone();
    assert!(matches!(
        l.close_period(jan),
        Err(LedgerError::ControlAccountMismatch {
            expected: AccountType::Equity,
            ..
        })
    ));
    assert_untouched(&before, &l);
    assert!(l.closed_period_containing(date(5)).is_none());
}

// ---------------------------------------------------------------------------
// Chart, periods, and lines: rules confirmed by Stephen, 2026-10-13.
// ---------------------------------------------------------------------------

/// A child account subdivides its parent, so it is the same kind of
/// account. SOURCE: rules.md B-4; DECISION (Stephen, 2026-10-13).
#[test]
fn a_child_account_has_its_parents_type() {
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, true)
        .unwrap();
    l.add_account("1010", "Checking", AccountType::Asset, Some(cash), false)
        .unwrap();
    let before = l.chart().len();
    assert_eq!(
        l.add_account("1020", "Rent", AccountType::Expense, Some(cash), false),
        Err(LedgerError::ParentTypeMismatch {
            code: "1020".into(),
            parent_code: "1000".into(),
            expected: AccountType::Asset,
            found: AccountType::Expense,
        })
    );
    assert_eq!(l.chart().len(), before);
    assert!(l.chart().find_by_code("1020").is_none());
}

/// Periods nest (a month in a quarter in a year) but never partially
/// overlap. DECISION (Stephen, 2026-10-13).
#[test]
fn periods_nest_but_never_partially_overlap() {
    let d = |m: u32, day: u32| NaiveDate::from_ymd_opt(2026, m, day).unwrap();
    let mut l = Ledger::new(Currency::USD);
    l.create_period("2026-01", d(1, 1), d(1, 31)).unwrap();
    l.create_period("2026-02", d(2, 1), d(2, 28)).unwrap();
    // A quarter around both months, and a year around everything.
    l.create_period("Q1 2026", d(1, 1), d(3, 31)).unwrap();
    l.create_period("2026", d(1, 1), d(12, 31)).unwrap();
    // A week inside January nests too.
    l.create_period("week", d(1, 5), d(1, 11)).unwrap();
    let next = l.next_period_id();
    // Mid-January to mid-February straddles two months: refused.
    assert!(matches!(
        l.create_period("straddle", d(1, 15), d(2, 15)),
        Err(LedgerError::PeriodsOverlap { .. })
    ));
    // So is a quarter that starts inside Q1.
    assert!(matches!(
        l.create_period("odd quarter", d(2, 1), d(4, 30)),
        Err(LedgerError::PeriodsOverlap { .. })
    ));
    assert_eq!(l.next_period_id(), next, "refused means no id burned");
    // Disjoint is fine.
    l.create_period(
        "2027-01",
        NaiveDate::from_ymd_opt(2027, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(2027, 1, 31).unwrap(),
    )
    .unwrap();
}

/// A line names a real, postable account when it is added, not only when
/// the entry posts. DECISION (Stephen, 2026-10-13); rules.md B-1.
#[test]
fn a_line_on_an_unknown_or_placeholder_account_is_refused_when_added() {
    let c = fixture_company();
    let mut l = c.ledger.clone();
    let header = l
        .add_account("1900", "Other assets", AccountType::Asset, None, true)
        .unwrap();
    let e = l.create_entry(date(21), "Lines");
    assert_eq!(
        l.add_line(e, 999_999, Side::Debit, usd(10)),
        Err(LedgerError::UnknownAccount(999_999))
    );
    assert_eq!(
        l.add_line(e, header, Side::Debit, usd(10)),
        Err(LedgerError::PlaceholderAccount("1900".into()))
    );
    assert!(matches!(
        l.add_line_fx(
            e,
            header,
            Side::Debit,
            Currency::EUR,
            Decimal::from(10),
            Decimal::from(2)
        ),
        Err(LedgerError::PlaceholderAccount(_))
    ));
    assert!(
        l.get_entry(e).unwrap().lines().is_empty(),
        "refused lines aren't added"
    );
}

/// Spec (rules.md B-3; DECISION, Stephen 2026-10-13, after the GnuCash
/// cross-check): a foreign line is booked in the base currency's smallest
/// unit, half away from zero; the foreign original is kept.
#[test]
fn a_foreign_line_is_booked_in_whole_base_units() {
    let book = |base: Currency, foreign: Currency, amount: &str, rate: &str| {
        let mut l = Ledger::new(base);
        let a = l
            .add_account("1000", "Cash", AccountType::Asset, None, false)
            .unwrap();
        let e = l.create_entry(date(5), "fx");
        let r = l.add_line_fx(
            e,
            a,
            Side::Debit,
            foreign,
            amount.parse().unwrap(),
            rate.parse().unwrap(),
        );
        r.map(|()| {
            let line = l.get_entry(e).unwrap().lines()[0].clone();
            (line.amount.amount, line.foreign_amount)
        })
    };
    let dec = |s: &str| s.parse::<Decimal>().unwrap();
    // 100.00 EUR x 1.0834567 = 108.34567 USD -> 108.35.
    assert_eq!(
        book(Currency::USD, Currency::EUR, "100.00", "1.0834567").unwrap(),
        (dec("108.35"), Some(dec("100.00")))
    );
    // 10.00 USD x 151.237 = 1512.37 JPY -> 1512 (yen have no minor unit).
    assert_eq!(
        book(Currency::JPY, Currency::USD, "10.00", "151.237")
            .unwrap()
            .0,
        dec("1512")
    );
    // Exactly half a cent rounds away from zero.
    assert_eq!(
        book(Currency::USD, Currency::EUR, "1", "0.125").unwrap().0,
        dec("0.13")
    );
    // Rounds to nothing: refused.
    assert!(book(Currency::USD, Currency::EUR, "0.001", "1").is_err());
}

// ---------------------------------------------------------------------------
// Ledger bookkeeping surfaces: id allocators, FX table, tax percent,
// reconciliation, and closing variants (mutation-testing gaps, 2026-10-13).
// ---------------------------------------------------------------------------

fn cents(c: i64) -> Decimal {
    Decimal::new(c, 2)
}

/// `next_*_id` predicts the id the next record receives, and each record
/// gets its own.
#[test]
fn id_allocators_predict_the_next_id_and_never_repeat() {
    use ledger_core::{Frequency, NewRecurring, NewSchedule, ScheduleKind};
    let c = fixture_company();
    let mut l = c.ledger;
    let schedule = |n: &str| NewSchedule {
        description: n.into(),
        kind: ScheduleKind::PrepaidExpense,
        total: usd(1_200),
        start: date(1),
        months: 12,
        balance_account: c.equipment,
        pl_account: c.rent,
    };
    let mut seen = Vec::new();
    for n in ["a", "b"] {
        let predicted = l.next_schedule_id();
        let id = l.register_schedule(schedule(n)).unwrap();
        assert_eq!(id, predicted);
        assert!(!seen.contains(&id));
        seen.push(id);
    }
    let mut listed: Vec<_> = l.schedules().map(|s| s.id).collect();
    listed.sort();
    assert_eq!(listed, seen);

    let recurring = || NewRecurring {
        description: "rent".into(),
        lines: vec![
            (c.rent, Side::Debit, usd(800)),
            (c.cash, Side::Credit, usd(800)),
        ],
        frequency: Frequency::Monthly,
        start: date(1),
        end: None,
    };
    let mut seen = Vec::new();
    for _ in 0..2 {
        let predicted = l.next_recurring_id();
        let id = l.register_recurring(recurring()).unwrap();
        assert_eq!(id, predicted);
        assert!(!seen.contains(&id));
        seen.push(id);
    }

    let v = l.add_vendor("Vend").unwrap();
    for _ in 0..2 {
        let predicted = l.next_bill_id();
        let id = l.create_bill(v, date(5), date(30), doc(100)).unwrap();
        assert_eq!(id, predicted);
        assert_ne!(l.next_bill_id(), id);
    }
}

/// The FX table records positive rates for foreign currencies only, and
/// gives back exactly what was recorded.
#[test]
fn fx_rates_are_positive_foreign_and_read_back_exactly() {
    let mut l = Ledger::new(Currency::USD);
    let rate: Decimal = "1.0834".parse().unwrap();
    assert_eq!(l.get_fx_rate(Currency::EUR, date(5)), None);
    assert_eq!(
        l.set_fx_rate(Currency::EUR, date(5), Decimal::ZERO),
        Err(LedgerError::InvalidFxRate(Decimal::ZERO))
    );
    assert!(l.set_fx_rate(Currency::EUR, date(5), -rate).is_err());
    assert!(l.set_fx_rate(Currency::USD, date(5), rate).is_err());
    assert_eq!(l.get_fx_rate(Currency::USD, date(5)), None);
    l.set_fx_rate(Currency::EUR, date(5), rate).unwrap();
    assert_eq!(l.get_fx_rate(Currency::EUR, date(5)), Some(rate));
    assert_eq!(l.get_fx_rate(Currency::EUR, date(6)), None);
}

/// A deductible percent is 0 to 100 inclusive (set_account_tax's spec);
/// 0 is real (penalties).
#[test]
fn deductible_percent_bounds_are_inclusive() {
    let c = fixture_company();
    let mut l = c.ledger;
    let rent = c.rent;
    l.set_account_tax(rent, Some("meals"), Some(Decimal::ZERO))
        .unwrap();
    l.set_account_tax(rent, Some("meals"), Some(Decimal::from(100)))
        .unwrap();
    assert!(matches!(
        l.set_account_tax(rent, Some("meals"), Some(cents(-1))),
        Err(LedgerError::InvalidTaxCategory(_))
    ));
}

/// Spec (pay_bill scenario 2): a payment may not predate its bill; the
/// bill's own date is fine.
#[test]
fn a_bill_can_be_paid_on_its_own_date() {
    let c = fixture_company();
    let mut l = c.ledger;
    let v = l.add_vendor("Vend").unwrap();
    let bill = l.create_bill(v, date(21), date(31), doc(100)).unwrap();
    l.pay_bill(bill, date(21), cents(1_000), c.cash).unwrap();
}

/// Reconciliation invariants 1 and 4: on a credit-normal account the book
/// balance is the account's balance, and a line dated on the statement
/// date is outstanding until cleared.
#[test]
fn reconciling_a_card_reads_its_normal_side_and_includes_the_statement_date() {
    let c = fixture_company();
    let mut l = c.ledger;
    let card = l
        .add_account("2100", "Card", AccountType::Liability, None, false)
        .unwrap();
    post(
        &mut l,
        date(21),
        "charge",
        &[(c.rent, Side::Debit, 500), (card, Side::Credit, 500)],
    );
    post(
        &mut l,
        date(25),
        "pay card",
        &[(card, Side::Debit, 200), (c.cash, Side::Credit, 200)],
    );
    let r = l
        .reconciliation_report(card, date(25), Decimal::from(300))
        .unwrap();
    assert_eq!(r.book_balance, Decimal::from(300));
    assert_eq!(r.book_balance, l.account_balance(card).unwrap());
    let outstanding: Vec<_> = r.uncleared_lines.iter().map(|u| u.date).collect();
    assert_eq!(outstanding, vec![date(21), date(25)]);
}

/// B-11: closing to retained earnings with nothing to close but a swept
/// account's balance still sweeps it.
#[test]
fn a_close_sweeps_dividends_even_without_income() {
    let mut l = books_with("3100", "Dividends", AccountType::Equity, false);
    let dividends = 2;
    post(
        &mut l,
        date(10),
        "dividend",
        &[(dividends, Side::Debit, 100), (1, Side::Credit, 100)],
    );
    let jan = l.create_period("2026-01", date(1), date(31)).unwrap();
    let to = ledger_core::CloseTo::RetainedEarnings {
        sweep: vec![dividends],
    };
    let predicted = l.preview_close_period_to(jan, &to).unwrap();
    assert_eq!(predicted, Some(l.next_entry_id()));
    assert_eq!(l.close_period_to(jan, &to).unwrap(), predicted);
    assert_eq!(l.account_balance(dividends).unwrap(), Decimal::ZERO);
    let re = l.chart().find_by_code("3999").unwrap().id;
    assert_eq!(l.account_balance(re).unwrap(), Decimal::from(-100));
}

/// `preview_close_period` names the entry a close with income will post.
#[test]
fn preview_close_names_the_closing_entry() {
    let mut c = fixture_company();
    let jan = c
        .ledger
        .create_period("2026-01", date(1), date(31))
        .unwrap();
    let next = c.ledger.next_entry_id();
    assert_eq!(c.ledger.preview_close_period(jan).unwrap(), Some(next));
    assert_eq!(c.ledger.close_period(jan).unwrap(), Some(next));
}

/// Closes journaled under the pre-2026-10-13 rule replay under it: each
/// share rounded half away from zero, the last member absorbing the rest.
/// Net 1.05 at 30/30/40 is 0.32, 0.32, 0.41 (largest remainder would give
/// 0.32, 0.31, 0.42).
#[test]
fn a_last_absorbs_close_gives_the_rounding_to_the_last_member() {
    let mut l = books_with("4000", "Sales", AccountType::Revenue, false);
    let capital: Vec<AccountId> = ["3001", "3002", "3003"]
        .iter()
        .map(|code| {
            l.add_account(code, "Capital", AccountType::Equity, None, false)
                .unwrap()
        })
        .collect();
    let e = l.create_entry(date(5), "sale");
    let m = Money::new(cents(105), Currency::USD);
    l.add_line(e, 1, Side::Debit, m).unwrap();
    l.add_line(e, 2, Side::Credit, m).unwrap();
    l.post_entry(e).unwrap();
    let jan = l.create_period("2026-01", date(1), date(31)).unwrap();
    let members = capital
        .iter()
        .zip(["0.3", "0.3", "0.4"])
        .map(|(&capital, share)| ledger_core::MemberClose {
            capital,
            share: share.parse().unwrap(),
            distributions: None,
        })
        .collect();
    l.close_period_to(jan, &ledger_core::CloseTo::MembersLastAbsorbs(members))
        .unwrap();
    let got: Vec<Decimal> = capital
        .iter()
        .map(|&a| l.account_balance(a).unwrap())
        .collect();
    assert_eq!(got, vec![cents(32), cents(32), cents(41)]);
}

/// B-12: a period whose closing date already lies in a closed period
/// can't close; the refusal names that period.
#[test]
fn a_period_ending_inside_a_closed_period_cannot_close() {
    let mut c = fixture_company();
    let q1 = c
        .ledger
        .create_period(
            "2026-Q1",
            date(1),
            NaiveDate::from_ymd_opt(2026, 3, 31).unwrap(),
        )
        .unwrap();
    let jan = c
        .ledger
        .create_period("2026-01", date(1), date(31))
        .unwrap();
    c.ledger.close_period(q1).unwrap();
    assert_eq!(
        c.ledger.close_period(jan),
        Err(LedgerError::ClosedPeriod(q1, date(31).to_string()))
    );
}

/// Replay (for journaling stores) rebuilds invoice and bill records
/// exactly: ids follow the allocator, payments accumulate, vendors tag
/// the entries. A record replayed out of order is refused.
#[test]
fn replayed_documents_and_payments_rebuild_the_records() {
    use ledger_core::DocumentStatus;
    let mut l = books_with("2000", "Accounts Payable", AccountType::Liability, false);
    let c = l.add_customer("Acme").unwrap();
    let v = l.add_vendor("Vend").unwrap();

    let inv = l.next_invoice_id();
    assert_eq!(
        l.replay_invoice(inv + 1, c, date(5), date(30), doc(100), 10),
        Err(LedgerError::ReplayIdMismatch(inv + 1, inv))
    );
    l.replay_invoice(inv, c, date(5), date(30), doc(100), 10)
        .unwrap();
    // Each later record replays at the id the allocator now predicts.
    let mut ids = vec![inv];
    for entry in [13, 14] {
        let next = l.next_invoice_id();
        l.replay_invoice(next, c, date(5), date(30), doc(1), entry)
            .unwrap();
        ids.push(next);
    }
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 3);
    assert!(ids.iter().all(|&i| l.get_invoice(i).is_some()));
    l.replay_invoice_payment(inv, Decimal::from(30), 11)
        .unwrap();
    l.replay_invoice_payment(inv, Decimal::from(20), 12)
        .unwrap();
    let i = l.get_invoice(inv).unwrap();
    assert_eq!(i.amount_paid, Decimal::from(50));
    assert_eq!(i.balance(), Decimal::from(50));
    assert_eq!(i.status, DocumentStatus::Partial);

    let bill = l.next_bill_id();
    assert_eq!(
        l.replay_bill(bill + 1, v, date(5), date(30), doc(100), 20),
        Err(LedgerError::ReplayIdMismatch(bill + 1, bill))
    );
    assert_eq!(l.entry_vendor(20), None);
    l.replay_bill(bill, v, date(5), date(30), doc(100), 20)
        .unwrap();
    assert_eq!(l.entry_vendor(20), Some(v));
    let mut ids = vec![bill];
    for entry in [23, 24] {
        let next = l.next_bill_id();
        l.replay_bill(next, v, date(5), date(30), doc(1), entry)
            .unwrap();
        ids.push(next);
    }
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 3);
    l.replay_bill_payment(bill, Decimal::from(30), 21).unwrap();
    l.replay_bill_payment(bill, Decimal::from(70), 22).unwrap();
    let b = l.get_bill(bill).unwrap();
    assert_eq!(b.amount_paid, Decimal::from(100));
    assert_eq!(b.status, DocumentStatus::Paid);
    assert_eq!(l.entry_vendor(22), Some(v));
}
