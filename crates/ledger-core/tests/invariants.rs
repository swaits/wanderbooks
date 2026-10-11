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
use ledger_core::{AccountId, AccountType, Currency, EntryStatus, Ledger, LedgerError, Money, Side};
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
    let cash = ledger.add_account("1000", "Cash", AccountType::Asset, None, false).unwrap();
    let ar = ledger
        .add_account("1200", "Accounts Receivable", AccountType::Asset, None, false)
        .unwrap();
    let equipment = ledger
        .add_account("1500", "Equipment", AccountType::Asset, None, false)
        .unwrap();
    let ap = ledger
        .add_account("2000", "Accounts Payable", AccountType::Liability, None, false)
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
    post(&mut ledger, date(2), "Owner investment", &[
        (cash, Side::Debit, 10_000),
        (equity, Side::Credit, 10_000),
    ]);
    // Buy equipment for $2,500 cash.
    post(&mut ledger, date(5), "Buy equipment", &[
        (equipment, Side::Debit, 2_500),
        (cash, Side::Credit, 2_500),
    ]);
    // Bill a client $4,000.
    post(&mut ledger, date(10), "Invoice client", &[
        (ar, Side::Debit, 4_000),
        (revenue, Side::Credit, 4_000),
    ]);
    // Pay January rent, $800.
    post(&mut ledger, date(15), "January rent", &[
        (rent, Side::Debit, 800),
        (cash, Side::Credit, 800),
    ]);
    // Collect $1,500 of receivables.
    post(&mut ledger, date(20), "Client payment", &[
        (cash, Side::Debit, 1_500),
        (ar, Side::Credit, 1_500),
    ]);

    Company { ledger, cash, ar, equipment, _ap: ap, equity, revenue, rent }
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
    ledger.add_line(id, c.equity, Side::Credit, usd(99)).unwrap();
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
    ledger.add_line(id, c.equity, Side::Credit, usd(100)).unwrap();
    assert_eq!(
        ledger.post_entry(id).unwrap_err(),
        LedgerError::ZeroAmountLine
    );
}

#[test]
fn unknown_account_is_rejected() {
    let c = fixture_company();
    let mut ledger = c.ledger;
    let id = ledger.create_entry(date(21), "Ghost account");
    ledger.add_line(id, c.cash, Side::Debit, usd(100)).unwrap();
    ledger.add_line(id, 999_999, Side::Credit, usd(100)).unwrap();
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
    ledger.add_line(id, c.equity, Side::Credit, usd(100)).unwrap();
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
    ledger.add_line(id, c.equity, Side::Credit, usd(50)).unwrap();
    ledger.post_entry(id).unwrap();

    // Cannot add lines after posting.
    assert_eq!(
        ledger.add_line(id, c.cash, Side::Debit, usd(1)).unwrap_err(),
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

    let rev = ledger.reverse_entry(id, date(22), "Reverse mistake").unwrap();
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
    let accounts = [c.cash, c.ar, c.equipment, c._ap, c.equity, c.revenue, c.rent];

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
