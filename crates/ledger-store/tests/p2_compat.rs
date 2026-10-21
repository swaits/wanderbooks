//! Books P2 wrote, served by this version before the owner's go-ahead
//! (plan/served.md 3.9, D-39; rules.md B-27, B-28): nothing is written
//! that P2 can't read, or would read with another meaning, and every line
//! P2 wrote opens.
//!
//! Oracle: P2 itself (9c99a0cf, the revision before P3), run from end to
//! end on P2-shaped books in the P3 data-safety review (2026-10-21): each
//! write below, made by this version on books in format 2 with no
//! features, left books P2 refused to open (`books.schedule_convention`,
//! the tax categories P3 added, a disposal before the in-service date) or
//! read wrong (a depreciation run carrying another month's depreciation,
//! which P2 reads as the month's own, so its months caught up show as not
//! yet run, and running them in P2 posts them twice). And a name P2 took
//! held what P3's wider TIN detector reads as a full TIN, so P3 refused to
//! open the books.
//!
//! ```text
//! INVARIANTS:
//!   1. On books in format 2 with no features, each such write is refused
//!      (FormatUpgradePending), journaling nothing; after the upgrade it is
//!      made.
//!   2. The same command shaped as P2 could write it is made before the
//!      upgrade (a category P2 knew, months run in order, a disposal on or
//!      after the in-service date).
//!   3. A line P2 accepted replays, in the writer and the viewer, however
//!      the commands' checks have widened since (B-28).
//! ```

use chrono::NaiveDate;
use ledger_core::settings::{Change, Source};
use ledger_core::{AccountType, Currency, Money, NewAsset, Side};
use ledger_store::{JOURNAL_FILE, JournalFormat, JournalWriter, Snapshot, Store, StoreError};
use rust_decimal::Decimal;
use serde_json::json;
use std::path::{Path, PathBuf};

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
        "wanderbooks-p2-compat-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TestDir(dir)
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn usd(s: &str) -> Money {
    Money::new(s.parse::<Decimal>().unwrap(), Currency::USD)
}

fn journal(dir: &Path) -> Vec<u8> {
    std::fs::read(dir.join(JOURNAL_FILE)).unwrap()
}

/// Books as P2 left them: format 2 (declared at their first setting), no
/// features, a chart.
fn p2_books(dir: &Path) -> Store {
    let mut s = Store::create_in_format(
        dir,
        Currency::USD,
        &JournalFormat {
            format: 2,
            features: Vec::new(),
        },
    )
    .unwrap();
    for (code, name, t) in [
        ("1000", "Bank", AccountType::Asset),
        ("1500", "Equipment", AccountType::Asset),
        ("1510", "Accumulated depreciation", AccountType::Asset),
        ("3000", "Capital", AccountType::Equity),
        ("4000", "Revenue", AccountType::Revenue),
        ("6200", "Depreciation", AccountType::Expense),
    ] {
        s.add_account(code, name, t, None, false).unwrap();
    }
    s
}

fn account(s: &Store, code: &str) -> u64 {
    s.ledger().chart().find_by_code(code).unwrap().id
}

/// INVARIANT 1's refusal: waiting for the upgrade, nothing journaled.
fn assert_waits<T: std::fmt::Debug>(dir: &Path, before: &[u8], got: Result<T, StoreError>) {
    match got {
        Err(StoreError::FormatUpgradePending { .. }) => {}
        other => panic!("expected FormatUpgradePending, got {other:?}"),
    }
    assert_eq!(journal(dir), before, "a refused write journaled nothing");
}

fn source() -> Source {
    Source {
        decided_by: "Avery Quill".into(),
        decided_on: d(2026, 10, 21),
        citation: None,
        note: None,
    }
}

#[test]
fn the_schedule_convention_setting_waits_for_the_upgrade() {
    let dir = test_dir("setting");
    let mut s = p2_books(&dir);
    let change = vec![Change {
        key: "books.schedule_convention".into(),
        value: Some(json!("monthly_even")),
    }];
    let before = journal(&dir);
    assert_waits(&dir, &before, s.change_settings(&change, &source()));
    s.upgrade_format().unwrap();
    s.change_settings(&change, &source()).unwrap();
}

#[test]
fn a_tax_category_p3_added_waits_for_the_upgrade() {
    let dir = test_dir("tax");
    let mut s = p2_books(&dir);
    let before = journal(&dir);
    for cat in ["form_4797_gain_loss", "tax_exempt_income"] {
        assert_waits(
            &dir,
            &before,
            s.add_account_with_tax(
                "4900",
                "Gain on sale",
                AccountType::Revenue,
                None,
                false,
                Some((cat, None)),
            ),
        );
        let revenue = account(&s, "4000");
        assert_waits(&dir, &before, s.set_account_tax(revenue, Some(cat), None));
    }
    // INVARIANT 2: a category P2 knew.
    s.add_account_with_tax(
        "4100",
        "Consulting",
        AccountType::Revenue,
        None,
        false,
        Some(("gross_receipts", None)),
    )
    .unwrap();
    s.upgrade_format().unwrap();
    s.add_account_with_tax(
        "4900",
        "Gain on sale",
        AccountType::Revenue,
        None,
        false,
        Some(("form_4797_gain_loss", None)),
    )
    .unwrap();
}

/// An asset bought on 2026-01-10 and placed in service on 2026-03-01.
fn camera(s: &mut Store) -> u64 {
    let (bank, equipment) = (account(s, "1000"), account(s, "1500"));
    let e = s.create_entry(d(2026, 1, 10), "Camera").unwrap();
    s.add_line(e, equipment, Side::Debit, usd("1200.00"))
        .unwrap();
    s.add_line(e, bank, Side::Credit, usd("1200.00")).unwrap();
    s.post_entry(e).unwrap();
    let id = s
        .register_asset(NewAsset {
            description: "Camera".into(),
            in_service_date: d(2026, 3, 1),
            cost: usd("1200.00"),
            salvage_value: usd("0"),
            useful_life_months: 24,
            cost_account: equipment,
            accum_depr_account: account(s, "1510"),
            expense_account: account(s, "6200"),
        })
        .unwrap();
    s.set_asset_acquisition(id, e).unwrap();
    id
}

#[test]
fn a_disposal_before_the_in_service_date_waits_for_the_upgrade() {
    let dir = test_dir("dispose");
    let mut s = p2_books(&dir);
    let id = camera(&mut s);
    let (bank, revenue) = (account(&s, "1000"), account(&s, "4000"));
    let before = journal(&dir);
    assert_waits(
        &dir,
        &before,
        s.dispose_asset(id, d(2026, 2, 1), usd("1000.00"), bank, revenue),
    );
    s.upgrade_format().unwrap();
    s.dispose_asset(id, d(2026, 2, 1), usd("1000.00"), bank, revenue)
        .unwrap();
}

#[test]
fn a_disposal_on_or_after_the_in_service_date_is_made_before_the_upgrade() {
    let dir = test_dir("dispose-late");
    let mut s = p2_books(&dir);
    let id = camera(&mut s);
    let (bank, revenue) = (account(&s, "1000"), account(&s, "4000"));
    s.dispose_asset(id, d(2026, 3, 1), usd("1000.00"), bank, revenue)
        .unwrap();
}

/// A printer in service from 2026-01-01, 1,200.00 over 12 months.
fn printer(s: &mut Store) -> u64 {
    s.register_asset(NewAsset {
        description: "Printer".into(),
        in_service_date: d(2026, 1, 1),
        cost: usd("1200.00"),
        salvage_value: usd("0"),
        useful_life_months: 12,
        cost_account: account(s, "1500"),
        accum_depr_account: account(s, "1510"),
        expense_account: account(s, "6200"),
    })
    .unwrap()
}

#[test]
fn a_depreciation_run_that_catches_up_another_month_waits_for_the_upgrade() {
    let dir = test_dir("catch-up");
    let mut s = p2_books(&dir);
    let id = printer(&mut s);
    let jan = s.run_depreciation(id, "2026-01").unwrap();
    assert_eq!(jan.amount, jan.scheduled, "its own month: made");
    // February skipped: March's run would carry it.
    let before = journal(&dir);
    assert_waits(&dir, &before, s.run_depreciation(id, "2026-03"));
    // INVARIANT 2: run in order, each its own month, as P2 ran them.
    for m in ["2026-02", "2026-03"] {
        let r = s.run_depreciation(id, m).unwrap();
        assert_eq!(r.amount, r.scheduled, "{m}");
    }
    // After the upgrade a skipped month is caught up.
    s.upgrade_format().unwrap();
    let may = s.run_depreciation(id, "2026-05").unwrap();
    assert!(may.amount > may.scheduled, "{may:?}");
}

#[test]
fn a_reversed_run_caught_up_later_waits_for_the_upgrade() {
    let dir = test_dir("reversed");
    let mut s = p2_books(&dir);
    let id = printer(&mut s);
    let jan = s.run_depreciation(id, "2026-01").unwrap();
    s.run_depreciation(id, "2026-02").unwrap();
    s.reverse_entry(jan.entry_id.unwrap(), d(2026, 2, 28), "wrong")
        .unwrap();
    let before = journal(&dir);
    assert_waits(&dir, &before, s.run_depreciation(id, "2026-03"));
}

/// INVARIANT 3: P2's detector found a full TIN only across ASCII
/// separators, the no-break space and a few dashes; P3's finds one across
/// every Unicode space and dash and invisible characters. A name P2 took
/// replays; the command refuses it now.
#[test]
fn a_name_p2_took_replays_though_the_command_now_refuses_it() {
    let dir = test_dir("tin-replay");
    drop(p2_books(&dir));
    let names = [
        "Studio 400\u{ff0d}12\u{ff0d}3456",
        "Studio 400\u{200b}12\u{200b}3456",
        "Studio 400\u{2009}12\u{2009}3456",
    ];
    {
        let mut w = JournalWriter::open(&dir.join(JOURNAL_FILE)).unwrap();
        for (i, name) in names.iter().enumerate() {
            w.append("vendor.added", json!({"id": i as u64 + 1, "name": name}))
                .unwrap();
        }
        w.append(
            "party.added",
            json!({"id": 4, "name": "Kai 400\u{301c}12\u{301c}3456", "roles": ["member"]}),
        )
        .unwrap();
    }
    let mut s = Store::open(&dir).expect("P2's books open");
    s.verify().unwrap();
    assert_eq!(s.ledger().list_vendors().len(), 3);
    Snapshot::load(&dir).expect("and the viewer reads them");
    for name in names {
        assert!(s.add_vendor(name).is_err(), "the command refuses {name:?}");
    }
    // Outside replay the detector is P3's again.
    assert!(ledger_core::parties::has_full_tin(names[0]));
}

/// After the upgrade, each write above is made, and the books replay to
/// the live ledger exactly, in the writer (reopened) and the viewer.
#[test]
fn after_the_upgrade_the_writes_replay_to_the_live_books() {
    let dir = test_dir("after");
    let mut s = p2_books(&dir);
    let camera = camera(&mut s);
    let printer = printer(&mut s);
    s.run_depreciation(printer, "2026-01").unwrap();
    assert!(s.upgrade_format().unwrap().is_some());
    let (bank, revenue) = (account(&s, "1000"), account(&s, "4000"));
    s.change_settings(
        &[Change {
            key: "books.schedule_convention".into(),
            value: Some(json!("daily")),
        }],
        &source(),
    )
    .unwrap();
    s.add_account_with_tax(
        "4910",
        "Exempt interest",
        AccountType::Revenue,
        None,
        false,
        Some(("tax_exempt_income", None)),
    )
    .unwrap();
    s.run_depreciation(printer, "2026-03").unwrap();
    s.dispose_asset(camera, d(2026, 2, 1), usd("1000.00"), bank, revenue)
        .unwrap();
    s.verify().unwrap();
    let live = s.ledger().clone();
    drop(s);
    let s = Store::open(&dir).unwrap();
    assert!(live.differs_from(s.ledger()).is_empty());
    let snap = Snapshot::load(&dir).unwrap();
    assert!(live.differs_from(snap.ledger()).is_empty());
}
