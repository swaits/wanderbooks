//! The viewer's numbers are the books' numbers: the view model
//! (books::Books, books::Kpis) against ledger-core, computed here
//! independently, on random books.

use chrono::{Datelike, Days, Months, NaiveDate};
use ledger_core::reports::{balance_sheet_as_of, profit_and_loss_for_period, trial_balance_as_of};
use ledger_core::{AccountType, Currency, DocumentLine, Money, Side};
use ledger_gui::{Source, data, fmt};
use ledger_store::Store;
use proptest::prelude::*;
use rust_decimal::{Decimal, RoundingStrategy};
use std::path::PathBuf;

const CHART: [(&str, &str, AccountType); 9] = [
    ("1000", "Checking", AccountType::Asset),
    ("1010", "Savings", AccountType::Asset),
    ("1200", "Accounts Receivable", AccountType::Asset),
    ("1500", "Equipment", AccountType::Asset),
    ("2000", "Accounts Payable", AccountType::Liability),
    ("2100", "Due to owners", AccountType::Liability),
    ("3000", "Capital", AccountType::Equity),
    ("4000", "Revenue", AccountType::Revenue),
    ("6000", "Supplies", AccountType::Expense),
];
const CASH: [&str; 2] = ["1000", "1010"];

#[derive(Clone, Debug)]
struct Plan {
    today: NaiveDate,
    fy_month: u32,
    /// (days before today, debit index, credit index, cents)
    entries: Vec<(u64, usize, usize, i64)>,
}

fn plan() -> impl Strategy<Value = Plan> {
    (
        // Month ends and leap days included.
        prop_oneof![
            Just(NaiveDate::from_ymd_opt(2026, 3, 31).unwrap()),
            Just(NaiveDate::from_ymd_opt(2028, 2, 29).unwrap()),
            Just(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()),
            (0u64..900).prop_map(|d| NaiveDate::from_ymd_opt(2025, 1, 1).unwrap() + Days::new(d)),
        ],
        1u32..=12,
        proptest::collection::vec((0u64..200, 0usize..9, 0usize..9, 1i64..5_000_000), 0..25),
    )
        .prop_map(|(today, fy_month, entries)| Plan {
            today,
            fy_month,
            entries: entries.into_iter().filter(|e| e.1 != e.2).collect(),
        })
}

/// A test's dir, removed when the test ends, pass or panic.
struct TestDir(PathBuf);

impl std::ops::Deref for TestDir {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn dir(tag: &str) -> TestDir {
    let d = std::env::temp_dir().join(format!("ledger-gui-vm-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    TestDir(d)
}

/// The books, where they are (kept while the guard lives), and their ledger.
fn build(plan: &Plan, tag: &str) -> (TestDir, Source, ledger_core::Ledger) {
    let d = dir(tag);
    let mut s = Store::create(&d.join("books-data"), Currency::USD).unwrap();
    let ids: Vec<_> = CHART
        .iter()
        .map(|(c, n, t)| s.add_account(c, n, *t, None, false).unwrap())
        .collect();
    // As replay posts them: random lines land on receivables and payables
    // by hand, as books an earlier version wrote can hold them (the
    // control guard, rules.md B-38, refuses them as new work), and the
    // overview must still read them.
    ledger_core::parties::replaying(|| {
        for &(back, dr, cr, cents) in &plan.entries {
            let e = s.create_entry(plan.today - Days::new(back), "x").unwrap();
            let m = Money::new(Decimal::new(cents, 2), Currency::USD);
            s.add_line(e, ids[dr], Side::Debit, m).unwrap();
            s.add_line(e, ids[cr], Side::Credit, m).unwrap();
            s.post_entry(e).unwrap();
        }
    });
    journal_settings(
        &mut s,
        &format!(
            "[company]\nname = \"Oracle Co\"\nfiscal_year_start = \"{:02}-01\"\n[books]\ncash_accounts = [\"1000\", \"1010\"]\nreceipt_threshold = \"25.00\"\n",
            plan.fy_month
        ),
    );
    let ledger = s.ledger().clone();
    let source = Source {
        data_dir: d.join("books-data"),
        today: Some(plan.today),
    };
    (d, source, ledger)
}

/// An account's normal-side balance at the end of `date`, from the trial
/// balance.
fn balance(l: &ledger_core::Ledger, code: &str, date: NaiveDate) -> Decimal {
    let t = l.chart().find_by_code(code).unwrap().account_type;
    let tb = trial_balance_as_of(l, date);
    let net = tb
        .lines
        .iter()
        .find(|x| x.code == code)
        .map(|x| x.debit - x.credit)
        .unwrap_or_default();
    match t {
        AccountType::Asset | AccountType::Expense => net,
        _ => -net,
    }
}

fn cash(l: &ledger_core::Ledger, date: NaiveDate) -> Decimal {
    CASH.iter().map(|c| balance(l, c, date)).sum()
}

fn first_of(d: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(d.year(), d.month(), 1).unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    /// Kpis INVARIANTS 1-7 and the period helpers, against ledger-core.
    #[test]
    fn the_overview_numbers_are_the_books_numbers(p in plan()) {
        let (_dir, source, l) = build(&p, "kpi");
        let b = data::load_once(&source).unwrap();
        let k = &b.kpis;
        let t = p.today;
        let m0 = first_of(t);
        prop_assert_eq!(b.month_start(), m0);
        prop_assert_eq!(k.cash, cash(&l, t));
        prop_assert_eq!(k.cash_30d_ago, cash(&l, t - Days::new(30)));
        prop_assert_eq!(k.liabilities, balance_sheet_as_of(&l, t).unwrap().total_liabilities);
        prop_assert_eq!(k.receivables, balance(&l, "1200", t));
        let pl = |s, e| profit_and_loss_for_period(&l, s, e, None).unwrap();
        let mtd = pl(m0, t);
        prop_assert_eq!(k.revenue_mtd, mtd.total_revenue);
        prop_assert_eq!(k.expenses_mtd, mtd.total_expenses);
        let prev_start = m0 - Months::new(1);
        let prev_last = m0 - Days::new(1);
        let same = NaiveDate::from_ymd_opt(prev_start.year(), prev_start.month(), t.day())
            .unwrap_or(prev_last);
        let prev = pl(prev_start, same);
        prop_assert_eq!(k.revenue_prev_month, prev.total_revenue);
        prop_assert_eq!(k.expenses_prev_month, prev.total_expenses);
        // Fiscal year: its first day is the last fiscal-start first-of-month
        // on or before today.
        let mut fy = NaiveDate::from_ymd_opt(t.year(), p.fy_month, 1).unwrap();
        if fy > t {
            fy = fy - Months::new(12);
        }
        prop_assert_eq!(b.fiscal_ytd(), (fy, t));
        // One default period everywhere (rules.md B-30): what a report or
        // register opens on is the fiscal year to date, the server's
        // resolver's own answer for no dates, and party totals count this
        // calendar year.
        prop_assert_eq!(b.default_period(), (fy, t));
        prop_assert_eq!(b.periods().statement(None, None), Ok((fy, t)));
        prop_assert_eq!(ledger_gui::pages::reports::Range::default().dates(&b), (fy, t));
        prop_assert_eq!(
            ledger_gui::pages::accounts::RegisterRange::default().dates(&b),
            (fy, t)
        );
        let jan1 = NaiveDate::from_ymd_opt(t.year(), 1, 1).unwrap();
        prop_assert_eq!(
            b.party_span(ledger_gui::books::PartyPeriod::ThisYear).0,
            jan1
        );
        let ytd = pl(fy, t);
        prop_assert_eq!(k.net_ytd, ytd.total_revenue - ytd.total_expenses);
        let first = p.entries.iter().map(|e| t - Days::new(e.0)).min();
        prop_assert_eq!(k.first_entry, first);
        // Burn: the three full months before m0, averaged over those the
        // books cover.
        let three = pl(m0 - Months::new(3), prev_last);
        let loss = three.total_expenses - three.total_revenue;
        let covered = match first {
            None => 0,
            Some(f) => (0..3)
                .filter(|i| first_of(m0 - Months::new(i + 1)) >= first_of(f))
                .count() as i64,
        };
        let burn = (loss > Decimal::ZERO && covered > 0).then(|| {
            (loss / Decimal::from(covered))
                .round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)
        });
        prop_assert_eq!(k.monthly_burn, burn);
        // Trailing months: n months ending with this one.
        for n in 1..=12u32 {
            prop_assert_eq!(b.trailing_months(n), (m0 - Months::new(n - 1), t));
        }
        // Cash history: each point is the cash balance that day, dates
        // strictly rising, through the last day cash moved.
        let h = b.cash_history();
        for w in h.windows(2) {
            prop_assert!(w[0].0 < w[1].0);
        }
        for (d, v) in &h {
            prop_assert_eq!(*v, cash(&l, *d), "cash on {}", d);
        }
        // ... and it isn't empty when a cash account moved: its last point
        // is the last day one did (a transfer between them included).
        let cash_idx = [0usize, 1];
        let last_move = p
            .entries
            .iter()
            .filter(|e| cash_idx.contains(&e.1) || cash_idx.contains(&e.2))
            .map(|e| t - Days::new(e.0))
            .max();
        prop_assert_eq!(h.last().map(|x| x.0), last_move);
        // Names and labels.
        for (code, name, _) in CHART {
            let id = l.chart().find_by_code(code).unwrap().id;
            prop_assert_eq!(b.account_label(id), format!("{code} {name}"));
            prop_assert_eq!(b.account_name(id), name);
        }
        prop_assert_eq!(b.company_name(), "Oracle Co");
        let events = std::fs::read_to_string(source.data_dir.join("journal.jsonl"))
            .unwrap()
            .lines()
            .count() as u64;
        prop_assert_eq!(b.seq(), events);
        prop_assert_eq!(b.receipt_rule(), "over the $25.00 receipt threshold");
        let _ = std::fs::remove_dir_all(source.data_dir.parent().unwrap());
    }
}

/// Where each entry came from, and who it was with, from the journal.
#[test]
fn origins_and_parties() {
    let d = dir("origin");
    let today = NaiveDate::from_ymd_opt(2026, 5, 20).unwrap();
    let mut s = Store::create(&d.join("books-data"), Currency::USD).unwrap();
    let cash = s
        .add_account("1000", "Checking", AccountType::Asset, None, false)
        .unwrap();
    let sup = s
        .add_account("6000", "Supplies", AccountType::Expense, None, false)
        .unwrap();
    let cust = s.add_customer("Acme Buyer").unwrap();
    let vend = s.add_vendor("Paper Co").unwrap();
    let line = |a: i64| {
        vec![DocumentLine {
            description: "x".into(),
            amount: Decimal::new(a, 2),
            class_id: None,
        }]
    };
    let inv = s
        .create_invoice(cust, today, today + Days::new(30), line(10_000))
        .unwrap();
    s.create_bill(vend, today, today + Days::new(30), line(5_000))
        .unwrap();
    let paid = s
        .receive_payment(inv, today, Decimal::new(10_000, 2), cash)
        .unwrap();
    let e = s.create_entry(today, "Paper").unwrap();
    let m = Money::new(Decimal::new(1_000, 2), Currency::USD);
    s.add_line(e, sup, Side::Debit, m).unwrap();
    s.add_line(e, cash, Side::Credit, m).unwrap();
    s.post_entry(e).unwrap();
    let (blob, _, _) = s
        .attach_receipt(Some(e), "r.pdf", "application/pdf", b"%PDF receipt", None)
        .unwrap();
    let rev = s.reverse_entry(e, today, "undo").unwrap();
    journal_settings(&mut s, "[books]\nreceipt_threshold = \"0.00\"\n");
    drop(s);
    let source = Source {
        data_dir: d.join("books-data"),
        today: Some(today),
    };
    let b = data::load_once(&source).unwrap();
    let by_origin = |o: &str| b.entries.iter().filter(|x| x.origin == o).count();
    assert_eq!(by_origin("Invoice"), 1);
    assert_eq!(by_origin("Bill"), 1);
    assert_eq!(by_origin("Customer payment"), 1);
    assert_eq!(by_origin("Reversal"), 1);
    assert_eq!(b.entry(paid).unwrap().party.as_deref(), Some("Acme Buyer"));
    let bill_entry = b.entries.iter().find(|x| x.origin == "Bill").unwrap();
    assert_eq!(bill_entry.party.as_deref(), Some("Paper Co"));
    assert_eq!(b.entry(e).unwrap().reversed_by, Some(rev));
    assert_eq!(b.entry(rev).unwrap().reversal_of, Some(e));
    // Every entry's journal events name it.
    for x in &b.entries {
        assert!(!x.events.is_empty(), "entry {} has no events", x.id);
    }
    assert_eq!(b.receipt_rule(), "every entry needs a receipt");
    // The receipt's bytes are where the blob root says.
    let a = b.attachments.iter().find(|a| a.id == blob).unwrap();
    let path = b
        .blob_root()
        .join(&a.blob_sha[..2])
        .join(&a.blob_sha[2..4])
        .join(&a.blob_sha);
    assert_eq!(std::fs::read(path).unwrap(), b"%PDF receipt");
    assert_eq!(b.integrity.files_ok, 1);
    assert!(b.integrity.file_failures.is_empty());
}

/// fmt helpers the viewer shows: percent change and tax forms.
#[test]
fn percent_change_and_tax_forms() {
    let d = |s: &str| s.parse::<Decimal>().unwrap();
    assert_eq!(fmt::pct_change(d("0"), d("5")), None);
    assert_eq!(fmt::pct_change(d("100"), d("150")), Some(50.0));
    assert_eq!(fmt::pct_change(d("200"), d("150")), Some(-25.0));
    assert_eq!(fmt::pct_change(d("-100"), d("-50")), Some(50.0));
    use ledger_core::TaxForm::*;
    for (f, number) in [
        (ScheduleC, "Schedule C"),
        (Form1065, "1065"),
        (Form1120S, "1120-S"),
        (Form1120, "1120"),
    ] {
        assert!(fmt::tax_form(f).contains(number), "{f:?}");
    }
    assert_eq!(fmt::compact(d("-1234")), "\u{2212}$1.2k");
}

fn store_at(tag: &str) -> (TestDir, Store) {
    let d = dir(tag);
    let s = Store::create(&d.join("books-data"), Currency::USD).unwrap();
    (d, s)
}

/// The settings `toml` says, journaled into `s` as the server adopts a
/// settings file: the viewer reads settings only from the journal.
fn journal_settings(s: &mut Store, toml: &str) {
    let import = ledger_config::codec::import(toml, None, s.ledger().settings(), "USD").unwrap();
    s.adopt_settings(
        &import.changes,
        &ledger_core::settings::Source {
            decided_by: "the test".into(),
            decided_on: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            citation: None,
            note: None,
        },
    )
    .unwrap();
}

/// The books in `d`, with the settings `toml` says journaled (none when
/// it's empty: the defaults).
fn source(d: &std::path::Path, toml: &str, today: NaiveDate) -> Source {
    if !toml.is_empty() {
        journal_settings(&mut Store::open(&d.join("books-data")).unwrap(), toml);
    }
    Source {
        data_dir: d.join("books-data"),
        today: Some(today),
    }
}

fn post(s: &mut Store, date: NaiveDate, dr: u64, cr: u64, cents: i64) -> u64 {
    let e = s.create_entry(date, "x").unwrap();
    let m = Money::new(Decimal::new(cents, 2), Currency::USD);
    s.add_line(e, dr, Side::Debit, m).unwrap();
    s.add_line(e, cr, Side::Credit, m).unwrap();
    s.post_entry(e).unwrap();
    e
}

/// Kpis INVARIANTS 4 and 6 on fixed dates: the prior month runs to the
/// same day; burn averages over only the months the books cover.
#[test]
fn comparisons_and_burn_on_fixed_dates() {
    let (d, mut s) = store_at("burn");
    let cash = s
        .add_account("1000", "Checking", AccountType::Asset, None, false)
        .unwrap();
    let rev = s
        .add_account("4000", "Sales", AccountType::Revenue, None, false)
        .unwrap();
    let exp = s
        .add_account("6000", "Rent", AccountType::Expense, None, false)
        .unwrap();
    let today = NaiveDate::from_ymd_opt(2026, 5, 10).unwrap();
    // The books start in March: burn covers March and April, not February.
    post(
        &mut s,
        NaiveDate::from_ymd_opt(2026, 3, 3).unwrap(),
        exp,
        cash,
        30_000,
    );
    post(
        &mut s,
        NaiveDate::from_ymd_opt(2026, 4, 10).unwrap(),
        exp,
        cash,
        10_000,
    );
    post(
        &mut s,
        NaiveDate::from_ymd_opt(2026, 4, 11).unwrap(),
        cash,
        rev,
        7_000,
    ); // after the same day
    post(
        &mut s,
        NaiveDate::from_ymd_opt(2026, 5, 2).unwrap(),
        cash,
        rev,
        1_000,
    );
    drop(s);
    let b = data::load_once(&source(&d, "[books]\ncash_accounts = [\"1000\"]\n", today)).unwrap();
    let k = &b.kpis;
    assert_eq!(k.revenue_mtd, Decimal::new(1_000, 2));
    assert_eq!(k.expenses_prev_month, Decimal::new(10_000, 2), "April 1-10");
    assert_eq!(
        k.revenue_prev_month,
        Decimal::ZERO,
        "April 11 is after the same day"
    );
    // Feb-Apr loss: 300 + 100 - 70 = 330 over the 2 months covered.
    assert_eq!(k.monthly_burn, Some(Decimal::new(16_500, 2)));
    // A profitable stretch has no burn; books starting this month have none.
    let (d2, mut s2) = store_at("burn-new");
    let c2 = s2
        .add_account("1000", "Checking", AccountType::Asset, None, false)
        .unwrap();
    let e2 = s2
        .add_account("6000", "Rent", AccountType::Expense, None, false)
        .unwrap();
    post(
        &mut s2,
        NaiveDate::from_ymd_opt(2026, 5, 1).unwrap(),
        e2,
        c2,
        5_000,
    );
    drop(s2);
    let b2 = data::load_once(&source(&d2, "", today)).unwrap();
    assert_eq!(b2.kpis.monthly_burn, None);
    // Three full months of loss: divided by 3, half away from zero.
    let (d3, mut s3) = store_at("burn-three");
    let c3 = s3
        .add_account("1000", "Checking", AccountType::Asset, None, false)
        .unwrap();
    let e3 = s3
        .add_account("6000", "Rent", AccountType::Expense, None, false)
        .unwrap();
    post(
        &mut s3,
        NaiveDate::from_ymd_opt(2025, 12, 1).unwrap(),
        e3,
        c3,
        1,
    );
    post(
        &mut s3,
        NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
        e3,
        c3,
        1_000,
    );
    drop(s3);
    let b3 = data::load_once(&source(&d3, "", today)).unwrap();
    // 10.00 / 3 = 3.333... -> 3.33
    assert_eq!(b3.kpis.monthly_burn, Some(Decimal::new(333, 2)));
    // Books covering only last month: one month, not two.
    let (d4, mut s4) = store_at("burn-one");
    let c4 = s4
        .add_account("1000", "Checking", AccountType::Asset, None, false)
        .unwrap();
    let e4 = s4
        .add_account("6000", "Rent", AccountType::Expense, None, false)
        .unwrap();
    post(
        &mut s4,
        NaiveDate::from_ymd_opt(2026, 4, 20).unwrap(),
        e4,
        c4,
        9_000,
    );
    drop(s4);
    let b4 = data::load_once(&source(&d4, "", today)).unwrap();
    assert_eq!(b4.kpis.monthly_burn, Some(Decimal::new(9_000, 2)));
    // Breaking even burns nothing.
    let (d5, mut s5) = store_at("burn-even");
    let c5 = s5
        .add_account("1000", "Checking", AccountType::Asset, None, false)
        .unwrap();
    let r5 = s5
        .add_account("4000", "Sales", AccountType::Revenue, None, false)
        .unwrap();
    let e5 = s5
        .add_account("6000", "Rent", AccountType::Expense, None, false)
        .unwrap();
    post(
        &mut s5,
        NaiveDate::from_ymd_opt(2026, 3, 5).unwrap(),
        e5,
        c5,
        4_000,
    );
    post(
        &mut s5,
        NaiveDate::from_ymd_opt(2026, 3, 6).unwrap(),
        c5,
        r5,
        4_000,
    );
    drop(s5);
    let b5 = data::load_once(&source(&d5, "", today)).unwrap();
    assert_eq!(b5.kpis.monthly_burn, None);
}

/// The view model's indexes: cash accounts by name when the settings name
/// none, card accounts, each entry's accounts (debits first, once each),
/// and what's new since the last refresh.
#[test]
fn indexes_cash_cards_accounts_and_fresh() {
    let (d, mut s) = store_at("index");
    let checking = s
        .add_account("1000", "Business Checking", AccountType::Asset, None, false)
        .unwrap();
    let savings = s
        .add_account("1010", "Savings", AccountType::Asset, None, false)
        .unwrap();
    let group = s
        .add_account("1090", "Cash accounts", AccountType::Asset, None, true)
        .unwrap();
    let equip = s
        .add_account("1500", "Equipment", AccountType::Asset, None, false)
        .unwrap();
    let card = s
        .add_account(
            "2050",
            "Chase Ink Card",
            AccountType::Liability,
            None,
            false,
        )
        .unwrap();
    let cardish = s
        .add_account("1600", "Card reader", AccountType::Asset, None, false)
        .unwrap();
    let rent = s
        .add_account("6000", "Rent", AccountType::Expense, None, false)
        .unwrap();
    let today = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
    // Two debits to one account, then two credits.
    let e = s.create_entry(today, "split").unwrap();
    let m = |c| Money::new(Decimal::new(c, 2), Currency::USD);
    s.add_line(e, rent, Side::Debit, m(100)).unwrap();
    s.add_line(e, rent, Side::Debit, m(200)).unwrap();
    s.add_line(e, card, Side::Credit, m(250)).unwrap();
    s.add_line(e, checking, Side::Credit, m(50)).unwrap();
    s.post_entry(e).unwrap();
    drop(s);
    let src = source(&d, "", today);
    let b = data::load_once(&src).unwrap();
    let mut cash = b.cash_accounts.clone();
    cash.sort();
    assert_eq!(
        cash,
        vec![checking, savings],
        "by name, postable assets only"
    );
    let _ = (group, equip, cardish);
    assert_eq!(b.card_accounts, vec![card], "liabilities named card");
    assert_eq!(b.entry(e).unwrap().accounts, vec![rent, card, checking]);
    assert!(b.fresh.is_empty(), "a first load marks nothing new");
    // A new entry after a refresh is the only fresh one.
    let seq = b.seq();
    let mut s = Store::open(&d.join("books-data")).unwrap();
    let e2 = post(&mut s, today, rent, checking, 999);
    drop(s);
    let snap = ledger_store::Snapshot::load(&src.data_dir).unwrap();
    let settings = ledger_config::codec::books_settings(snap.ledger().settings(), "USD");
    let mut verified = std::collections::HashSet::new();
    let integrity = ledger_gui::books::Integrity::check(&snap, &mut verified);
    let b2 = ledger_gui::books::Books::build(&snap, settings, today, Some(seq), integrity).unwrap();
    assert_eq!(b2.fresh, [e2].into_iter().collect());
}

/// Integrity counts every stored file, read once: a second check with the
/// same memory counts them again without rereading; a damaged one fails.
#[test]
fn integrity_counts_files_once_verified() {
    let (d, mut s) = store_at("integrity");
    let cash = s
        .add_account("1000", "Checking", AccountType::Asset, None, false)
        .unwrap();
    let rent = s
        .add_account("6000", "Rent", AccountType::Expense, None, false)
        .unwrap();
    let e = post(
        &mut s,
        NaiveDate::from_ymd_opt(2026, 1, 2).unwrap(),
        rent,
        cash,
        100,
    );
    s.attach_receipt(Some(e), "a.pdf", "application/pdf", b"one", None)
        .unwrap();
    s.attach_receipt(Some(e), "b.pdf", "application/pdf", b"two", None)
        .unwrap();
    drop(s);
    let snap = ledger_store::Snapshot::load(&d.join("books-data")).unwrap();
    let mut verified = std::collections::HashSet::new();
    let first = ledger_gui::books::Integrity::check(&snap, &mut verified);
    assert_eq!((first.files_ok, first.file_failures.len()), (2, 0));
    let again = ledger_gui::books::Integrity::check(&snap, &mut verified);
    assert_eq!((again.files_ok, again.file_failures.len()), (2, 0));
}

/// Origins and parties for the rest of the journal's kinds: the invoice
/// itself shows its customer, a bill payment its vendor, and a payroll
/// tax remittance is labeled as one (so the receipt rule for hand-made
/// entries doesn't flag it).
#[test]
fn more_origins_and_parties() {
    let (d, mut s) = store_at("origin2");
    let cash = s
        .add_account("1000", "Checking", AccountType::Asset, None, false)
        .unwrap();
    let today = NaiveDate::from_ymd_opt(2026, 7, 15).unwrap();
    let cust = s.add_customer("Acme Buyer").unwrap();
    let vend = s.add_vendor("Paper Co").unwrap();
    let line = |a: i64| {
        vec![DocumentLine {
            description: "x".into(),
            amount: Decimal::new(a, 2),
            class_id: None,
        }]
    };
    s.create_invoice(cust, today, today + Days::new(30), line(10_000))
        .unwrap();
    let bill = s
        .create_bill(vend, today, today + Days::new(30), line(5_000))
        .unwrap();
    let paid = s
        .pay_bill(bill, today, Decimal::new(5_000, 2), cash)
        .unwrap();
    s.run_payroll(
        today,
        vec![ledger_core::PayrollLine {
            employee_name: "Pat".into(),
            employee: None,
            gross_pay: Decimal::new(100_000, 2),
            federal_withholding: Decimal::new(10_000, 2),
            state_withholding: Decimal::new(3_000, 2),
            ss_employee: Decimal::new(6_200, 2),
            medicare_employee: Decimal::new(1_450, 2),
            ss_employer: Decimal::new(6_200, 2),
            medicare_employer: Decimal::new(1_450, 2),
            other_deductions: Decimal::ZERO,
        }],
        cash,
    )
    .unwrap();
    let remit = s.remit_payroll_taxes(today, cash).unwrap();
    drop(s);
    let b = data::load_once(&source(
        &d,
        "[books]\nreceipt_threshold = \"0.00\"\n",
        today,
    ))
    .unwrap();
    let inv = b.entries.iter().find(|x| x.origin == "Invoice").unwrap();
    assert_eq!(inv.party.as_deref(), Some("Acme Buyer"));
    let pay = b.entry(paid).unwrap();
    assert_eq!(pay.origin, "Bill payment");
    assert_eq!(pay.party.as_deref(), Some("Paper Co"));
    assert_eq!(
        b.entries.iter().filter(|x| x.origin == "Payroll").count(),
        1
    );
    let r = b.entry(remit).unwrap();
    assert_eq!(r.origin, "Payroll taxes");
    assert!(!r.missing_receipt, "a remittance isn't a hand-made entry");
}

/// Defect 8, repros 1.4.2 and 1.4.3 (rules.md B-32): Needs attention lists
/// every revenue and expense account without a tax category, with
/// activity or not: one never used, one used only last fiscal year; never
/// a header, an account with a category, or an asset. Info under warn,
/// Todo under refuse.
#[test]
fn needs_attention_flags_every_uncategorized_pl_account_defect_8() {
    let today = NaiveDate::from_ymd_opt(2026, 10, 20).unwrap();
    let d = dir("attention-tax");
    let mut s = Store::create(&d.join("books-data"), Currency::USD).unwrap();
    let cash = s
        .add_account("1000", "Checking", AccountType::Asset, None, false)
        .unwrap();
    let ops = s
        .add_account("6000", "Operating", AccountType::Expense, None, true)
        .unwrap();
    s.add_account("6150", "Software", AccountType::Expense, Some(ops), false)
        .unwrap();
    let hosting = s
        .add_account("6160", "Hosting", AccountType::Expense, Some(ops), false)
        .unwrap();
    s.add_account_with_tax(
        "4000",
        "Sales",
        AccountType::Revenue,
        None,
        false,
        Some(("gross_receipts", None)),
    )
    .unwrap();
    let e = s
        .create_entry(NaiveDate::from_ymd_opt(2025, 6, 1).unwrap(), "last year")
        .unwrap();
    let m = Money::new(Decimal::new(1_200, 2), Currency::USD);
    s.add_line(e, hosting, Side::Debit, m).unwrap();
    s.add_line(e, cash, Side::Credit, m).unwrap();
    s.post_entry(e).unwrap();
    journal_settings(
        &mut s,
        "[company]\nname = \"Fernhill Labs LLC\"\n[books]\ncash_accounts = [\"1000\"]\n",
    );
    let source = Source {
        data_dir: d.join("books-data"),
        today: Some(today),
    };
    let item = |b: &ledger_gui::books::Books| {
        ledger_gui::attention::find(b)
            .into_iter()
            .find(|a| a.title.contains("without a tax category"))
    };
    let b = data::load_once(&source).unwrap();
    let a = item(&b).expect("listed");
    assert_eq!(a.title, "2 accounts without a tax category");
    assert!(
        a.detail.starts_with("6150 Software, 6160 Hosting. Each revenue and expense account maps to a line of the return"),
        "{}",
        a.detail
    );
    assert_eq!(a.severity, ledger_gui::books::Severity::Info);
    assert!(
        a.prompt.contains("books.tax_categories") && a.prompt.contains("books.set_tax_category")
    );
    // Under refuse, a to-do.
    s.change_settings(
        &[ledger_core::settings::Change {
            key: "policy.require_tax_category".into(),
            value: Some(serde_json::json!("refuse")),
        }],
        &ledger_core::settings::Source {
            decided_by: "Tobin Harlow".into(),
            decided_on: today,
            citation: None,
            note: None,
        },
    )
    .unwrap();
    let b = data::load_once(&source).unwrap();
    assert_eq!(
        item(&b).unwrap().severity,
        ledger_gui::books::Severity::Todo
    );
    // Categorized: no item at all.
    for code in ["6150", "6160"] {
        let id = s.ledger().chart().find_by_code(code).unwrap().id;
        s.set_account_tax(id, Some("other_deductions"), None)
            .unwrap();
    }
    let b = data::load_once(&source).unwrap();
    assert!(item(&b).is_none());
}

/// "Show all dates" is every date (P3 settled in design, rules.md B-30):
/// the register's and the reports' all-dates range runs from the first
/// posted entry through the later of today and the last, so an entry
/// dated after today is never left out of it (as party totals' "All time"
/// already ends); with no such entry it ends today.
#[test]
fn all_dates_holds_an_entry_dated_after_today() {
    use ledger_gui::pages::accounts::RegisterRange;
    use ledger_gui::pages::reports::Range;
    let today = NaiveDate::from_ymd_opt(2026, 10, 20).unwrap();
    let d = dir("all-dates-future");
    let mut s = Store::create(&d.join("books-data"), Currency::USD).unwrap();
    let cash = s
        .add_account("1000", "Checking", AccountType::Asset, None, false)
        .unwrap();
    let capital = s
        .add_account("3000", "Capital", AccountType::Equity, None, false)
        .unwrap();
    let source = Source {
        data_dir: d.join("books-data"),
        today: Some(today),
    };
    let post = |s: &mut Store, day: NaiveDate| {
        let e = s.create_entry(day, "x").unwrap();
        let m = Money::new(Decimal::new(500, 2), Currency::USD);
        s.add_line(e, cash, Side::Debit, m).unwrap();
        s.add_line(e, capital, Side::Credit, m).unwrap();
        s.post_entry(e).unwrap();
    };
    let first = NaiveDate::from_ymd_opt(2025, 3, 15).unwrap();
    post(&mut s, first);
    let b = data::load_once(&source).unwrap();
    assert_eq!(b.all_dates(), (first, today));
    let later = NaiveDate::from_ymd_opt(2026, 12, 1).unwrap();
    post(&mut s, later);
    let b = data::load_once(&source).unwrap();
    assert_eq!(b.kpis.last_entry, Some(later));
    assert_eq!(RegisterRange::All.dates(&b), (first, later));
    assert_eq!(Range::AllTime.dates(&b), (first, later));
}
