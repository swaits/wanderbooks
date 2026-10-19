//! The viewer's figures at the edges P0 review round 4 checked (rules.md
//! B-28): the party period of a year chrono can't hold, the party chart's
//! last months, one party's part of a split entry, an amortization
//! schedule due, and the page state tests set.
//!
//! ```text
//! GOAL: what the viewer derives from books is the books' answer, at the
//!       edges too: never a panic, never another party's money.
//! SOURCE: books.rs party_span (years out of chrono clip to its edges),
//!         last_months, party_what_for, share_of_what_for; attention.rs
//!         (an amortization schedule with months due is a to-do).
//! ```

use chrono::{Datelike, NaiveDate};
use egui_kittest::Harness;
use ledger_core::parties::{EntryFlow, entry_flow, money_accounts};
use ledger_core::{AccountType, Currency, Money, PartyShare, Role, Share, Side};
use ledger_gui::books::{Books, PartyPeriod, share_of_what_for};
use ledger_gui::nav::Route;
use ledger_gui::{App, Source, attention, data};
use ledger_store::Store;
use rust_decimal::Decimal;
use std::path::PathBuf;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn usd(c: i64) -> Money {
    Money::new(Decimal::new(c, 2), Currency::USD)
}

fn today() -> NaiveDate {
    d(2026, 10, 15)
}

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Books with cash, rent and fees, an insurance schedule from January
/// (months due by today), and a $100.00 payment from cash, rent $70 and
/// fees $30, split $60 to Sam and $40 to Riley (neither a member).
struct Fixture {
    _dir: TempDir,
    books: Books,
    entry: u64,
    sam: u64,
    riley: u64,
    cash: u64,
    rent: u64,
    fees: u64,
}

fn fixture(tag: &str) -> Fixture {
    let dir = TempDir(
        std::env::temp_dir().join(format!("ledger-gui-edges-{tag}-{}", std::process::id())),
    );
    let _ = std::fs::remove_dir_all(&dir.0);
    let data = dir.0.join("books-data");
    let mut s = Store::create(&data, Currency::USD).unwrap();
    let mut add = |code: &str, ty| s.add_account(code, code, ty, None, false).unwrap();
    let cash = add("1000", AccountType::Asset);
    let prepaid = add("1300", AccountType::Asset);
    let rent = add("6000", AccountType::Expense);
    let fees = add("6010", AccountType::Expense);
    let insurance = add("6300", AccountType::Expense);
    s.register_schedule(ledger_core::NewSchedule {
        description: "Insurance".into(),
        kind: ledger_core::ScheduleKind::PrepaidExpense,
        total: usd(120_000),
        start: d(2026, 1, 1),
        months: 12,
        balance_account: prepaid,
        pl_account: insurance,
    })
    .unwrap();
    let sam = s.add_party("Sam", &[Role::Contractor]).unwrap();
    let riley = s.add_party("Riley", &[Role::Contractor]).unwrap();
    // The cash line first: a part that lands on the entry's first line
    // instead of on what it paid for is told apart.
    let entry = s.create_entry(d(2026, 3, 1), "shared").unwrap();
    s.add_line(entry, cash, Side::Credit, usd(10_000)).unwrap();
    s.add_line(entry, rent, Side::Debit, usd(7_000)).unwrap();
    s.add_line(entry, fees, Side::Debit, usd(3_000)).unwrap();
    s.attach_receipt(Some(entry), "r.pdf", "application/pdf", b"%PDF r", None)
        .unwrap();
    s.post_entry(entry).unwrap();
    s.set_entry_parties(
        entry,
        &[
            PartyShare {
                party: sam,
                share: Share::Amount(Decimal::new(6_000, 2)),
            },
            PartyShare {
                party: riley,
                share: Share::Amount(Decimal::new(4_000, 2)),
            },
        ],
    )
    .unwrap();
    drop(s);
    let books = data::load_once(&Source {
        data_dir: data,
        today: Some(today()),
    })
    .unwrap();
    Fixture {
        _dir: dir,
        books,
        entry,
        sam,
        riley,
        cash,
        rent,
        fees,
    }
}

/// party_span: a year chrono can't hold is clipped to its edge (a year
/// past its last starts at its last day; one before its first is its
/// first day), never a panic.
#[test]
fn a_party_year_past_chrono_is_clipped_to_its_edge() {
    let b = fixture("span").books;
    let after = NaiveDate::MAX.year() + 1;
    let before = NaiveDate::MIN.year() - 1;
    assert_eq!(b.party_span(PartyPeriod::Year(after)).0, NaiveDate::MAX);
    assert_eq!(
        b.party_span(PartyPeriod::Year(before)),
        (NaiveDate::MIN, NaiveDate::MIN)
    );
    // Years chrono holds, year 0 included, are their own.
    assert_eq!(
        b.party_span(PartyPeriod::Year(0)),
        (d(0, 1, 1), d(0, 12, 31))
    );
    assert_eq!(
        b.party_span(PartyPeriod::Year(2025)),
        (d(2025, 1, 1), d(2025, 12, 31))
    );
}

/// last_months: the last n months of a span, from the first of the
/// month n - 1 months before its end; the whole span when shorter; and
/// at chrono's first months, from its first day.
#[test]
fn the_last_months_of_a_span() {
    let span = (d(2026, 1, 15), d(2026, 10, 17));
    assert_eq!(
        Books::last_months(span, 1),
        (d(2026, 10, 1), d(2026, 10, 17))
    );
    assert_eq!(
        Books::last_months(span, 3),
        (d(2026, 8, 1), d(2026, 10, 17))
    );
    assert_eq!(Books::last_months(span, 24), span);
    let first = (NaiveDate::MIN, d(NaiveDate::MIN.year(), 3, 9));
    assert_eq!(Books::last_months(first, 24), first);
}

/// party_what_for: on an entry two parties split, each one's part of
/// what it paid for is its fraction of every line that isn't a member's
/// capital, summing to its share.
#[test]
fn a_partys_part_of_a_split_entry_is_its_fraction_of_what_was_bought() {
    let f = fixture("what-for");
    let e = f.books.ledger.get_entry(f.entry).unwrap();
    let money = money_accounts(&f.books.ledger, &[f.cash]);
    let flow = entry_flow(e, &money).unwrap();
    let dec = |c: i64| Decimal::new(c, 2);
    assert_eq!(
        f.books.party_what_for(e, f.sam, &flow),
        vec![(f.rent, dec(4_200)), (f.fees, dec(1_800))]
    );
    assert_eq!(
        f.books.party_what_for(e, f.riley, &flow),
        vec![(f.rent, dec(2_800)), (f.fees, dec(1_200))]
    );
}

/// share_of_what_for doc: the whole what-for for the whole net; nothing
/// for a zero share or a zero net; else each account times share / net,
/// toward zero to the cent, the leftover cents to the first, summing to
/// the share; None past exact arithmetic.
#[test]
fn a_share_of_what_for_sums_to_the_share() {
    let dec = |c: i64| Decimal::new(c, 2);
    let flow = EntryFlow {
        net: dec(10_000),
        what_for: vec![(6, dec(3_333)), (7, dec(6_667))],
    };
    assert_eq!(
        share_of_what_for(&flow, dec(10_000)),
        Some(flow.what_for.clone())
    );
    assert_eq!(share_of_what_for(&flow, Decimal::ZERO), Some(vec![]));
    assert_eq!(
        share_of_what_for(
            &EntryFlow {
                net: Decimal::ZERO,
                ..flow.clone()
            },
            dec(5)
        ),
        Some(vec![])
    );
    assert_eq!(
        share_of_what_for(
            &EntryFlow {
                net: dec(5),
                what_for: vec![]
            },
            dec(1)
        ),
        Some(vec![])
    );
    // A third: 11.11 and 22.22, rounded down, and the cent left over to
    // the first.
    assert_eq!(
        share_of_what_for(&flow, dec(3_334)),
        Some(vec![(6, dec(1_112)), (7, dec(2_222))])
    );
    let huge = EntryFlow {
        net: Decimal::ONE,
        what_for: vec![(6, Decimal::MAX)],
    };
    assert_eq!(share_of_what_for(&huge, Decimal::TWO), None);
}

/// attention: an amortization schedule with months due by today is a
/// to-do, totalling them.
#[test]
fn a_schedule_with_months_due_is_a_todo() {
    let f = fixture("schedule");
    let items = attention::find(&f.books);
    let due: Vec<_> = items
        .iter()
        .filter(|a| a.title == "Amortization due: Insurance")
        .collect();
    assert_eq!(
        due.len(),
        1,
        "{:?}",
        items.iter().map(|a| &a.title).collect::<Vec<_>>()
    );
    // January through September: nine months of $100.00.
    assert!(
        due[0].detail.starts_with("9 months totaling $900"),
        "{}",
        due[0].detail
    );
}

/// App::state_mut is the app's own state: what a test sets there is
/// what the pages read.
#[test]
fn the_page_state_a_test_sets_is_the_apps() {
    let f = fixture("state");
    let mut h = Harness::builder()
        .with_size(egui::vec2(800.0, 600.0))
        .build_eframe(|cc| App::with_books(&cc.egui_ctx, f.books, false, Route::Overview));
    h.state_mut().state_mut().parties.period = PartyPeriod::Year(2025);
    assert_eq!(
        h.state_mut().state_mut().parties.period,
        PartyPeriod::Year(2025)
    );
    assert_eq!(
        h.state_mut().state_mut().parties.period,
        PartyPeriod::Year(2025)
    );
}
