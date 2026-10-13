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
use ledger_core::reports::{balance_sheet, profit_and_loss, snapshot, trial_balance};
use ledger_core::{
    AccountId, AccountType, Currency, EntryStatus, Ledger, LedgerError, Money, Side,
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
    // Too precise.
    let id = ledger.create_entry(date(21), "Too precise");
    let tiny = Money::new("0.00000000001".parse().unwrap(), Currency::USD);
    ledger.add_line(id, c.cash, Side::Debit, tiny).unwrap();
    ledger.add_line(id, c.equity, Side::Credit, tiny).unwrap();
    assert_eq!(
        ledger.post_entry(id).unwrap_err(),
        LedgerError::AmountOutOfRange
    );
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
    let c = fixture_company();
    let mut ledger = c.ledger;
    let id = ledger.create_entry(date(21), "Ghost account");
    ledger.add_line(id, c.cash, Side::Debit, usd(100)).unwrap();
    ledger
        .add_line(id, 999_999, Side::Credit, usd(100))
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
    ledger.add_line(id, header, Side::Debit, usd(100)).unwrap();
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

/// Spec (owner, 2026-10-13): an entry is reversed at most once. A second
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

/// Spec (owner, 2026-10-13): a closing entry is never reversed. Undoing
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
    let pl = profit_and_loss(&c.ledger).unwrap();
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
