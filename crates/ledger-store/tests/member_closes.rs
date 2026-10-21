//! Members closes in the journal (P3 defect 17, rules.md B-13, B-27):
//! the cumulative rule's feature declaration, its `since_seq`, what a
//! newer or damaged rounding tag does on replay, and the first close after
//! the upgrade on books an earlier version wrote. Every expected figure is
//! worked by hand in the comment beside it.

use chrono::NaiveDate;
use ledger_core::{
    AccountId, AccountType, CloseTo, Currency, Ledger, MemberCapital, MemberClose, Money, Side,
    members_equity,
};
use ledger_store::journal::read_and_verify_journal;
use ledger_store::{JOURNAL_FILE, JournalFormat, JournalWriter, Snapshot, Store, StoreError};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// A test's dir, removed when the test ends, pass or panic.
struct TestDir(PathBuf);

impl std::ops::Deref for TestDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn test_dir(name: &str) -> TestDir {
    let dir = std::env::temp_dir().join(format!(
        "wanderbooks-member-closes-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TestDir(dir)
}

fn d(m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, day).unwrap()
}

fn dec(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn month_end(m: u32) -> NaiveDate {
    if m == 12 {
        d(12, 31)
    } else {
        d(m + 1, 1).pred_opt().unwrap()
    }
}

/// Two members at half each, capital 3000 and 3001; sales 4000.
struct Books {
    store: Store,
    cash: AccountId,
    caps: [AccountId; 2],
    sales: AccountId,
}

fn books(dir: &Path) -> Books {
    let mut store = Store::create(dir, Currency::USD).unwrap();
    let mut add = |code: &str, t| store.add_account(code, code, t, None, false).unwrap();
    let cash = add("1000", AccountType::Asset);
    let caps = [
        add("3000", AccountType::Equity),
        add("3001", AccountType::Equity),
    ];
    let sales = add("4000", AccountType::Revenue);
    Books {
        store,
        cash,
        caps,
        sales,
    }
}

/// [`books`] as a version before the format gate made them: format 1,
/// nothing declared (D-39).
fn old_books(dir: &Path) -> Books {
    let mut store = Store::create_in_format(dir, Currency::USD, &JournalFormat::default()).unwrap();
    let mut add = |code: &str, t| store.add_account(code, code, t, None, false).unwrap();
    let cash = add("1000", AccountType::Asset);
    let caps = [
        add("3000", AccountType::Equity),
        add("3001", AccountType::Equity),
    ];
    let sales = add("4000", AccountType::Revenue);
    Books {
        store,
        cash,
        caps,
        sales,
    }
}

/// [`books`] opened again.
fn reopen(dir: &Path) -> Books {
    let store = Store::open(dir).unwrap();
    let id = |code: &str| store.ledger().chart().find_by_code(code).unwrap().id;
    let (cash, caps, sales) = (id("1000"), [id("3000"), id("3001")], id("4000"));
    Books {
        store,
        cash,
        caps,
        sales,
    }
}

fn half(caps: &[AccountId]) -> Vec<MemberClose> {
    caps.iter()
        .map(|&capital| MemberClose {
            capital,
            share: dec("0.5"),
            distributions: None,
        })
        .collect()
}

/// Earn `amount` on the 10th of month `m`, then close the month to `to`.
fn earn_and_close(b: &mut Books, m: u32, amount: &str, to: &CloseTo) {
    let e = b.store.create_entry(d(m, 10), "sale").unwrap();
    let money = Money::new(dec(amount), Currency::USD);
    b.store.add_line(e, b.cash, Side::Debit, money).unwrap();
    b.store.add_line(e, b.sales, Side::Credit, money).unwrap();
    b.store.post_entry(e).unwrap();
    let p = b
        .store
        .create_period(&format!("2026-{m:02}"), d(m, 1), month_end(m))
        .unwrap();
    b.store.close_period_to(p, to).unwrap();
}

/// The journal's lines as (type, payload).
fn journal(dir: &Path) -> Vec<(String, Value)> {
    read_and_verify_journal(&dir.join(JOURNAL_FILE))
        .unwrap()
        .into_iter()
        .map(|l| (l.line_type, l.payload))
        .collect()
}

/// Copy `from`'s journal into `to` through `JournalWriter` (a valid
/// chain), letting `edit` rewrite each payload.
fn rewrite_journal(from: &Path, to: &Path, edit: impl Fn(&str, Value) -> Value) {
    let mut w = JournalWriter::open(&to.join(JOURNAL_FILE)).unwrap();
    for (t, p) in journal(from) {
        w.append(&t, edit(&t, p)).unwrap();
    }
}

/// Books with three members closes of a cent each (Jan to Mar, 50/50).
fn three_cumulative_closes(dir: &Path) -> Books {
    let mut b = books(dir);
    let to = CloseTo::Members(half(&b.caps));
    for m in 1..=3 {
        earn_and_close(&mut b, m, "0.01", &to);
    }
    b
}

/// Contract (rules.md B-27; P3 defect 17; D-39): a members close counted
/// from its regime's start needs the `members_cumulative` feature, so an
/// older version stops at its declaration (it would replay the close under
/// the run rule). New books declare it on their second line, with the rest
/// of this version's format, and never again; books an older version made
/// (format 1) refuse a members close, journaling nothing, until the
/// owner's format upgrade declares it, and close to retained earnings as
/// ever.
#[test]
fn the_first_cumulative_close_declares_its_feature() {
    let dir = test_dir("declares");
    let b = three_cumulative_closes(&dir);
    drop(b);
    let lines = journal(&dir);
    let declares = |p: &Value| {
        p["features"]
            .as_array()
            .is_some_and(|f| f.contains(&json!("members_cumulative")))
    };
    let declarations: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, (t, p))| t == "journal.format_set" && declares(p))
        .map(|(i, _)| i)
        .collect();
    // Line 2: this version's whole format, as the books were created.
    assert_eq!(declarations, [1]);
    assert_eq!(lines[1].1["format"], 2);
    assert_eq!(
        lines[1].1["features"],
        json!(ledger_store::format::current().features)
    );
    assert_eq!(
        lines
            .iter()
            .filter(|(t, p)| t == "period.closed" && p["close_to"]["since_seq"] == 1)
            .count(),
        3
    );
    // Reopened, a fourth close declares nothing again.
    let mut b = reopen(&dir);
    let to = CloseTo::Members(half(&b.caps));
    earn_and_close(&mut b, 4, "0.01", &to);
    drop(b);
    let again = journal(&dir)
        .iter()
        .filter(|(t, p)| t == "journal.format_set" && declares(p))
        .count();
    assert_eq!(again, 1);

    // Books an older version made: a members close waits for the upgrade,
    // a close to retained earnings doesn't.
    let dir = test_dir("declares-old");
    let mut b = old_books(&dir);
    earn_and_close(
        &mut b,
        1,
        "0.01",
        &CloseTo::RetainedEarnings { sweep: vec![] },
    );
    let e = b.store.create_entry(d(2, 10), "sale").unwrap();
    let money = Money::new(dec("0.01"), Currency::USD);
    b.store.add_line(e, b.cash, Side::Debit, money).unwrap();
    b.store.add_line(e, b.sales, Side::Credit, money).unwrap();
    b.store.post_entry(e).unwrap();
    let p = b
        .store
        .create_period("2026-02", d(2, 1), month_end(2))
        .unwrap();
    let head = b.store.head();
    let to = CloseTo::Members(half(&b.caps));
    match b.store.close_period_to(p, &to) {
        Err(StoreError::FormatUpgradePending { need }) => {
            assert_eq!(need, "the journal feature members_cumulative")
        }
        other => panic!("expected FormatUpgradePending, got {other:?}"),
    }
    assert_eq!(b.store.head(), head, "refused whole");
    assert!(!journal(&dir).iter().any(|(_, p)| declares(p)));
    // The owner's go-ahead: the whole format at once, then the close.
    assert!(b.store.upgrade_format().unwrap().is_some());
    b.store.close_period_to(p, &to).unwrap().unwrap();
    drop(b);
    let lines = journal(&dir);
    let n = lines.len();
    assert_eq!(lines[n - 1].0, "period.closed");
    assert_eq!(lines[n - 2].0, "journal.format_set");
    assert_eq!(
        lines[n - 2].1["features"],
        json!(ledger_store::format::current().features)
    );
}

/// A change to a journaled close's target.
type Edit = dyn Fn(&mut Value);

/// Contract (rules.md B-27; P3 review R-9): a rounding rule only a newer
/// version writes is newer, never corrupt: an unknown name, or a value
/// that isn't a name. A `since_seq` beside another rule, or a
/// "cumulative_since" without a usable one, is damage.
#[test]
fn an_unknown_rounding_is_newer_not_corrupt() {
    let src = test_dir("rounding-src");
    drop(three_cumulative_closes(&src));
    let edit_last_close = |name: &str, f: &Edit| -> TestDir {
        let dir = test_dir(&format!("rounding-{name}"));
        let closes = journal(&src)
            .iter()
            .filter(|(t, _)| t == "period.closed")
            .count();
        let seen = std::cell::Cell::new(0);
        rewrite_journal(&src, &dir, |t, mut p| {
            if t == "period.closed" {
                seen.set(seen.get() + 1);
                if seen.get() == closes {
                    f(&mut p["close_to"]);
                }
            }
            p
        });
        dir
    };
    let newer: [(&str, &Edit); 2] = [
        ("unknown", &|c| c["rounding"] = json!("cumulative_by_day")),
        ("object", &|c| c["rounding"] = json!({"rule": "cumulative"})),
    ];
    for (name, f) in newer {
        let dir = edit_last_close(name, f);
        let err = Store::open(&dir).err().expect(name);
        assert!(
            matches!(err, StoreError::NewerJournal { ref event_type, .. } if event_type == "period.closed"),
            "{name}: {err}"
        );
        assert!(
            matches!(
                Snapshot::load(&dir).err(),
                Some(StoreError::NewerJournal { .. })
            ),
            "{name}: the viewer"
        );
    }
    let damaged: [(&str, &Edit); 4] = [
        ("since-beside-run", &|c| c["rounding"] = json!("cumulative")),
        ("no-since", &|c| {
            c.as_object_mut().unwrap().remove("since_seq");
        }),
        ("since-zero", &|c| c["since_seq"] = json!(0)),
        ("since-text", &|c| c["since_seq"] = json!("1")),
    ];
    for (name, f) in damaged {
        let dir = edit_last_close(name, f);
        let err = Store::open(&dir).err().expect(name);
        assert!(
            matches!(err, StoreError::JournalCorrupt(_)),
            "{name}: {err}"
        );
    }
}

/// Contract (B-13, B-8): a count that starts after the books' first close
/// journals the close it starts at, and replays to it. January's cent
/// closes at 50/50 (close 1, A's: the tie). February's at 60/40 starts a
/// new count at close 2 (0.006 / 0.004: A's), and March's continues it
/// (the running 0.02 is 0.012 / 0.008, so 0.01 / 0.01, less 0.01 / 0.00:
/// B's). The journal says since_seq 1, 2, 2; the books reopen, verify, and
/// stand at A 0.02, B 0.01.
#[test]
fn a_count_that_starts_later_journals_its_start() {
    let dir = test_dir("later-start");
    let mut b = books(&dir);
    let sixty = CloseTo::Members(
        b.caps
            .iter()
            .zip(["0.6", "0.4"])
            .map(|(&capital, share)| MemberClose {
                capital,
                share: dec(share),
                distributions: None,
            })
            .collect(),
    );
    let fifty = CloseTo::Members(half(&b.caps));
    earn_and_close(&mut b, 1, "0.01", &fifty);
    earn_and_close(&mut b, 2, "0.01", &sixty);
    earn_and_close(&mut b, 3, "0.01", &sixty);
    drop(b);
    let since: Vec<Value> = journal(&dir)
        .into_iter()
        .filter(|(t, _)| t == "period.closed")
        .map(|(_, p)| p["close_to"]["since_seq"].clone())
        .collect();
    assert_eq!(since, [json!(1), json!(2), json!(2)]);
    let b = reopen(&dir);
    b.store.verify().unwrap();
    let bal = |id| b.store.ledger().account_balance(id).unwrap();
    assert_eq!((bal(b.caps[0]), bal(b.caps[1])), (dec("0.02"), dec("0.01")));
}

/// Contract (B-8, B-13): replay re-derives where each count of members
/// closes starts and refuses a close journaled with another start, so a
/// wrong `since_seq` makes the books refuse to open, never show figures
/// split some other way.
#[test]
fn a_close_journaled_with_another_start_refuses_to_open() {
    let src = test_dir("start-src");
    drop(three_cumulative_closes(&src));
    let dir = test_dir("start");
    rewrite_journal(&src, &dir, |t, mut p| {
        if t == "period.closed" && p["id"] == 2 {
            p["close_to"]["since_seq"] = json!(2);
        }
        p
    });
    for err in [
        Store::open(&dir).err().expect("the writer"),
        Snapshot::load(&dir).err().expect("the viewer"),
    ] {
        assert!(
            err.to_string().contains("from close 2") && err.to_string().contains("from close 1"),
            "{err}"
        );
    }
}

/// The members of x_member_closes as the settings would name them: A
/// (capital 3000, distributions 3100) and B (3001), 50/50.
fn fixture_members(l: &Ledger) -> Vec<MemberCapital> {
    let id = |code: &str| l.chart().find_by_code(code).unwrap().id;
    vec![
        MemberCapital {
            name: "A".into(),
            capital_account: id("3000"),
            distribution_account: Some(id("3100")),
            share: dec("0.5"),
        },
        MemberCapital {
            name: "B".into(),
            capital_account: id("3001"),
            distribution_account: None,
            share: dec("0.5"),
        },
    ]
}

/// Each member's capital on the statement of members' equity as of `end`.
fn capital_on(l: &Ledger, end: NaiveDate) -> Vec<Decimal> {
    members_equity(l, d(1, 1), end, &fixture_members(l))
        .unwrap()
        .members
        .iter()
        .map(|m| m.ending)
        .collect()
}

/// Regression (P3 review R-2, D-10): books main wrote with members closes
/// under every rounding rule it had (fixture x_member_closes: three
/// untagged, three "largest_remainder", "cumulative" broken by a close to
/// retained earnings) open to the figures main computed (A 99.07, B
/// 100.05: earnings 0.07 / 0.05 of the 0.12 closed to them, exact 0.06
/// each), and the first close after the upgrade counts from the first
/// close of all (September's close to retained earnings skipped) and
/// carries the true-up. December earns 0.02: the running 0.14 splits
/// 0.07 / 0.07, less 0.07 / 0.05 given, so A gets 0.00 and B 0.02, and
/// both stand at their exact 0.07.
///
/// The statement of members' equity (D-38, R-1): as of Nov 30, A's share
/// of the 0.01 in retained earnings is what the next members close would
/// give of it, allocate(0.13) − allocate(0.12) = 0.07 − 0.06 = 0.01 (B:
/// 0.06 − 0.06 = 0), so A 99.08 and B 100.05, before and after December
/// closes: the pending true-up never shows early. As of Dec 31, before the
/// close, December's 0.02 and the 0.01 split allocate(0.15) −
/// allocate(0.12) = 0.08 − 0.06 / 0.07 − 0.06: A 99.09, B 100.06; after it,
/// allocate(0.15) − allocate(0.14) = 0.01 / 0.00 on capital 99.07 / 100.07:
/// A 99.08, B 100.07. The figure moves by exactly the true-up the close
/// posts (A −0.01, B +0.01), dated at it.
#[test]
fn the_first_close_after_the_upgrade_trues_up_the_old_rules() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/old-books/x_member_closes/journal.jsonl");
    let dir = test_dir("upgrade");
    std::fs::copy(&fixture, dir.join(JOURNAL_FILE)).unwrap();
    let mut store = Store::open(&dir).unwrap();
    let id = |s: &Store, code: &str| s.ledger().chart().find_by_code(code).unwrap().id;
    let (cash, a, b, sales) = (
        id(&store, "1000"),
        id(&store, "3000"),
        id(&store, "3001"),
        id(&store, "4000"),
    );
    let bal = |s: &Store, acct| s.ledger().account_balance(acct).unwrap();
    assert_eq!(
        (bal(&store, a), bal(&store, b)),
        (dec("99.07"), dec("100.05"))
    );
    let rules: Vec<_> = store
        .ledger()
        .list_periods()
        .iter()
        .map(|p| p.members_closed.as_ref().map(|m| m.rule))
        .collect();
    use ledger_core::MemberRule::{LastAbsorbs, PerClose, Run};
    assert_eq!(
        rules,
        [
            Some(LastAbsorbs),
            Some(LastAbsorbs),
            Some(LastAbsorbs),
            Some(PerClose),
            Some(PerClose),
            Some(PerClose),
            Some(Run),
            Some(Run),
            None,
            Some(Run),
            Some(Run),
        ]
    );
    assert_eq!(
        capital_on(store.ledger(), d(11, 30)),
        [dec("99.08"), dec("100.05")]
    );

    let e = store.create_entry(d(12, 10), "sale").unwrap();
    let cents = Money::new(dec("0.02"), Currency::USD);
    store.add_line(e, cash, Side::Debit, cents).unwrap();
    store.add_line(e, sales, Side::Credit, cents).unwrap();
    store.post_entry(e).unwrap();
    assert_eq!(
        capital_on(store.ledger(), d(12, 31)),
        [dec("99.09"), dec("100.06")]
    );
    let dec_period = store.create_period("2026-12", d(12, 1), d(12, 31)).unwrap();
    let members = vec![
        MemberClose {
            capital: a,
            share: dec("0.5"),
            distributions: Some(id(&store, "3100")),
        },
        MemberClose {
            capital: b,
            share: dec("0.5"),
            distributions: None,
        },
    ];
    // Books an earlier version wrote: the close counted from its regime's
    // start waits for the owner's go-ahead (D-39), journaling nothing; the
    // upgrade declares this version's whole format.
    let head = store.head();
    match store.close_period_to(dec_period, &CloseTo::Members(members.clone())) {
        Err(StoreError::FormatUpgradePending { need }) => {
            assert_eq!(need, "the journal feature members_cumulative")
        }
        other => panic!("expected FormatUpgradePending, got {other:?}"),
    }
    assert_eq!(store.head(), head, "refused whole");
    assert!(store.upgrade_format().unwrap().is_some());
    let closing = store
        .close_period_to(dec_period, &CloseTo::Members(members))
        .unwrap()
        .unwrap();
    let lines: Vec<(String, Side, Decimal)> = store
        .ledger()
        .get_entry(closing)
        .unwrap()
        .lines()
        .iter()
        .map(|l| {
            let code = store.ledger().chart().get(l.account).unwrap().code.clone();
            (code, l.side, l.amount.amount)
        })
        .collect();
    assert_eq!(
        lines,
        [
            ("4000".to_string(), Side::Debit, dec("0.02")),
            ("3001".to_string(), Side::Credit, dec("0.02")),
        ]
    );
    let figures = |s: &Ledger| {
        let (since, split) = s.member_close_split(dec_period).unwrap();
        let rows: Vec<_> = split.iter().map(|m| (m.part, m.running, m.exact)).collect();
        (since, rows)
    };
    let expected = (
        1,
        vec![
            (dec("0.00"), dec("0.07"), dec("0.07")),
            (dec("0.02"), dec("0.07"), dec("0.07")),
        ],
    );
    assert_eq!(figures(store.ledger()), expected);
    assert_eq!(
        (bal(&store, a), bal(&store, b)),
        (dec("99.07"), dec("100.07"))
    );
    assert_eq!(
        capital_on(store.ledger(), d(11, 30)),
        [dec("99.08"), dec("100.05")]
    );
    assert_eq!(
        capital_on(store.ledger(), d(12, 31)),
        [dec("99.08"), dec("100.07")]
    );
    // One declaration, the upgrade's, just before the close: this
    // version's whole format.
    drop(store);
    let lines = journal(&dir);
    let n = lines.len();
    assert_eq!(lines[n - 1].0, "period.closed");
    assert_eq!(lines[n - 1].1["close_to"]["since_seq"], 1);
    assert_eq!(lines[n - 2].0, "journal.format_set");
    assert_eq!(
        lines[n - 2].1["features"],
        json!(ledger_store::format::current().features)
    );
    // Reopened, and in the viewer: the same.
    let store = Store::open(&dir).unwrap();
    store.verify().unwrap();
    assert_eq!(figures(store.ledger()), expected);
    assert_eq!(
        (bal(&store, a), bal(&store, b)),
        (dec("99.07"), dec("100.07"))
    );
    let snap = Snapshot::load(&dir).unwrap();
    assert_eq!(figures(snap.ledger()), expected);
}
