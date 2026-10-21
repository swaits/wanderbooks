//! The odd cent in the viewer (P3 defect 17, D-38): a member's page shows
//! their capital the way the closes split earnings, so the figure for a
//! day never moves by a cent when its month is later closed.

use chrono::NaiveDate;
use ledger_core::{AccountType, CloseTo, Currency, MemberClose, Money, Role, Side};
use ledger_gui::books::Books;
use ledger_gui::{Source, data};
use ledger_store::Store;
use rust_decimal::Decimal;
use std::path::{Path, PathBuf};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let d =
            std::env::temp_dir().join(format!("ledger-gui-odd-cent-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        TempDir(d)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn d(m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, day).unwrap()
}

fn today() -> NaiveDate {
    d(10, 15)
}

fn load(dir: &Path) -> Books {
    data::load_once(&Source {
        data_dir: dir.to_path_buf(),
        today: Some(today()),
    })
    .unwrap()
}

/// Each member's closing capital on their page for the spans from Jan 1
/// to every day through `last`.
fn every_day(b: &Books, members: &[ledger_core::PartyId], last: NaiveDate) -> Vec<Vec<Decimal>> {
    let mut out = Vec::new();
    let mut day = d(1, 1);
    while day <= last {
        out.push(
            members
                .iter()
                .map(|&m| b.capital_statement(m, (d(1, 1), day)).unwrap().closing)
                .collect(),
        );
        day = day.succ_opt().unwrap();
    }
    out
}

/// D-38: two members, Ann and Bo, at 50% each; January to June each earn
/// a cent, each month closed in turn. The cumulative split, by hand: the
/// running 0.01, 0.02, …, 0.06 splits A 0.01 / B 0.00, 0.01 / 0.01, 0.02
/// / 0.01, 0.02 / 0.02, 0.03 / 0.02, 0.03 / 0.03 (each odd cent's tie to
/// Ann, listed first). The viewer shows each member's capital for every
/// day of the year so far the same before and after each month closes
/// (the split of each month's cent alone gave Ann every cent until its
/// close moved it), and ends at 0.03 each.
#[test]
fn a_members_page_shows_the_same_capital_before_and_after_a_close() {
    let dir = TempDir::new("pages");
    let books = dir.0.join("books-data");
    let mut s = Store::create(&books, Currency::USD).unwrap();
    let mut add = |code: &str, name: &str, t| s.add_account(code, name, t, None, false).unwrap();
    let cash = add("1000", "Cash", AccountType::Asset);
    let caps = [
        add("3000", "Capital Ann", AccountType::Equity),
        add("3001", "Capital Bo", AccountType::Equity),
    ];
    let sales = add("4000", "Sales", AccountType::Revenue);
    let members = [
        s.add_party("Ann", &[Role::Member]).unwrap(),
        s.add_party("Bo", &[Role::Member]).unwrap(),
    ];
    let import = ledger_config::codec::import(
        "[books]\ncash_accounts = [\"1000\"]\n[company]\nname = \"Example Partners LLC\"\n\
         [[company.members]]\nname = \"Ann\"\nshare = \"50%\"\ncapital_account = \"3000\"\n\
         [[company.members]]\nname = \"Bo\"\nshare = \"50%\"\ncapital_account = \"3001\"\n",
        None,
        s.ledger().settings(),
        "USD",
    )
    .unwrap();
    s.adopt_settings(
        &import.changes,
        &ledger_core::settings::Source {
            decided_by: "the test".into(),
            decided_on: today(),
            citation: None,
            note: None,
        },
    )
    .unwrap();
    let to = CloseTo::Members(
        caps.iter()
            .map(|&capital| MemberClose {
                capital,
                share: Decimal::new(5, 1),
                distributions: None,
            })
            .collect(),
    );
    let cent = Money::new(Decimal::new(1, 2), Currency::USD);
    drop(s);
    for m in 1..=6 {
        let mut s = Store::open(&books).unwrap();
        let e = s.create_entry(d(m, 10), "a cent").unwrap();
        s.add_line(e, cash, Side::Debit, cent).unwrap();
        s.add_line(e, sales, Side::Credit, cent).unwrap();
        s.post_entry(e).unwrap();
        drop(s);
        let end = d(m + 1, 1).pred_opt().unwrap();
        let before = every_day(&load(&books), &members, end);
        let mut s = Store::open(&books).unwrap();
        let p = s
            .create_period(&format!("2026-{m:02}"), d(m, 1), end)
            .unwrap();
        s.close_period_to(p, &to).unwrap();
        drop(s);
        let after = every_day(&load(&books), &members, end);
        assert_eq!(after, before, "month {m}");
    }
    let b = load(&books);
    let closing: Vec<Decimal> = members
        .iter()
        .map(|&m| b.capital_statement(m, (d(1, 1), d(6, 30))).unwrap().closing)
        .collect();
    assert_eq!(closing, [Decimal::new(3, 2), Decimal::new(3, 2)]);
}

/// A member's page in a yen book splits in whole yen (B-3, B-13), as the
/// closes and the statement do. Ann and Bo at 50% each. "Owed to them"
/// (CapitalStatement INVARIANT 5): 101 yen due to the members, 50.5 each
/// exactly, the tied yen to Ann, listed first: Ann 51, Bo 50. Property
/// they contributed together (K-1 INVARIANT 4): Dr Equipment 101, Dr Cash
/// 1 / Cr Ann's capital 51, Cr Bo's 51, so 50.5 each of property, the yen
/// to Ann: Ann 51 property, Bo 50 property and 1 cash. Split to cents, both
/// were 50.50 each: half a yen, which no yen amount holds.
#[test]
fn a_yen_members_page_splits_in_whole_yen() {
    let dir = TempDir::new("yen");
    let books = dir.0.join("books-data");
    let mut s = Store::create(&books, Currency::JPY).unwrap();
    let mut add = |code: &str, name: &str, t| s.add_account(code, name, t, None, false).unwrap();
    let cash = add("1000", "Cash", AccountType::Asset);
    let equipment = add("1500", "Equipment", AccountType::Asset);
    let due = add("2100", "Due to members", AccountType::Liability);
    let caps = [
        add("3000", "Capital Ann", AccountType::Equity),
        add("3001", "Capital Bo", AccountType::Equity),
    ];
    let fees = add("6000", "Fees", AccountType::Expense);
    let members = [
        s.add_party("Ann", &[Role::Member]).unwrap(),
        s.add_party("Bo", &[Role::Member]).unwrap(),
    ];
    let import = ledger_config::codec::import(
        "[books]\ncash_accounts = [\"1000\"]\n[company]\nname = \"Example Partners LLC\"\n\
         [[company.members]]\nname = \"Ann\"\nshare = \"50%\"\ncapital_account = \"3000\"\n\
         [[company.members]]\nname = \"Bo\"\nshare = \"50%\"\ncapital_account = \"3001\"\n",
        None,
        s.ledger().settings(),
        "JPY",
    )
    .unwrap();
    s.adopt_settings(
        &import.changes,
        &ledger_core::settings::Source {
            decided_by: "the test".into(),
            decided_on: today(),
            citation: None,
            note: None,
        },
    )
    .unwrap();
    let yen = |n: i64| Money::new(Decimal::from(n), Currency::JPY);
    let e = s.create_entry(d(3, 1), "contributed together").unwrap();
    s.add_line(e, equipment, Side::Debit, yen(101)).unwrap();
    s.add_line(e, cash, Side::Debit, yen(1)).unwrap();
    s.add_line(e, caps[0], Side::Credit, yen(51)).unwrap();
    s.add_line(e, caps[1], Side::Credit, yen(51)).unwrap();
    s.post_entry(e).unwrap();
    let e = s.create_entry(d(4, 1), "paid for the company").unwrap();
    s.add_line(e, fees, Side::Debit, yen(101)).unwrap();
    s.add_line(e, due, Side::Credit, yen(101)).unwrap();
    s.post_entry(e).unwrap();
    drop(s);
    let b = load(&books);
    let page: Vec<_> = members
        .iter()
        .map(|&m| {
            let st = b.capital_statement(m, (d(1, 1), today())).unwrap();
            (
                st.due_to_members,
                st.owed_to_member,
                st.contributions_property,
                st.contributions_cash,
            )
        })
        .collect();
    let y = |n: i64| Decimal::from(n);
    assert_eq!(
        page,
        [(y(101), y(51), y(51), y(0)), (y(101), y(50), y(50), y(1))]
    );
}
