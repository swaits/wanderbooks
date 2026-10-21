//! Fixed-asset store tests: journaling, idempotency, disposal, and verify.

use chrono::NaiveDate;
use ledger_core::{AccountType, Currency, Money, NewAsset, Side};
use ledger_store::Store;
use rust_decimal::Decimal;
use tempfile::tempdir;

fn usd(amount: &str) -> Money {
    Money::new(amount.parse().unwrap(), Currency::USD)
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn open_store() -> (tempfile::TempDir, Store) {
    let dir = tempdir().unwrap();
    let store = Store::create(dir.path(), Currency::USD).unwrap();
    (dir, store)
}

fn asset_accounts(store: &mut Store) -> (u64, u64, u64, u64, u64) {
    let cost = store
        .add_account("1500", "Equipment", AccountType::Asset, None, false)
        .unwrap();
    let accum = store
        .add_account(
            "1501",
            "Accumulated Depreciation",
            AccountType::Asset,
            None,
            false,
        )
        .unwrap();
    let expense = store
        .add_account(
            "6100",
            "Depreciation Expense",
            AccountType::Expense,
            None,
            false,
        )
        .unwrap();
    let bank = store
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let gain = store
        .add_account(
            "4900",
            "Gain on Disposal",
            AccountType::Revenue,
            None,
            false,
        )
        .unwrap();
    (cost, accum, expense, bank, gain)
}

fn van(store: &mut Store, dollars: u64, cost_acct: u64, accum_acct: u64, expense_acct: u64) -> u64 {
    store
        .register_asset(NewAsset {
            description: "Sprinter van".to_string(),
            purchase_date: date(2026, 1, 15),
            cost: usd(&dollars.to_string()),
            salvage_value: usd("0"),
            useful_life_months: 60,
            cost_account: cost_acct,
            accum_depr_account: accum_acct,
            expense_account: expense_acct,
        })
        .unwrap()
}

#[test]
fn asset_survives_reopen_with_stable_ids() {
    let (dir, mut store) = open_store();
    let (cost, accum, expense, _, _) = asset_accounts(&mut store);
    let id = van(&mut store, 60_000, cost, accum, expense);
    drop(store);

    let mut store = Store::open(dir.path()).unwrap();
    let asset = store.ledger().get_asset(id).expect("asset must survive");
    assert_eq!(asset.description, "Sprinter van");
    assert_eq!(asset.cost.amount, Decimal::from(60_000));
    // Allocator stability: the next registration continues the sequence.
    let id2 = van(&mut store, 1_000, cost, accum, expense);
    assert_eq!(id2, id + 1);
    store.verify().unwrap();
}

#[test]
fn depreciation_run_is_idempotent() {
    let (_dir, mut store) = open_store();
    let (cost, accum, expense, _, _) = asset_accounts(&mut store);
    let id = van(&mut store, 60_000, cost, accum, expense);

    let run1 = store.run_depreciation(id, "2026-01").unwrap();
    assert!(!run1.already_posted);
    assert_eq!(run1.amount, Decimal::new(54839, 2)); // 1000 * 17/31

    // Second run of the same period is a no-op returning the same entry.
    let run2 = store.run_depreciation(id, "2026-01").unwrap();
    assert!(run2.already_posted);
    assert_eq!(run2.entry_id, run1.entry_id);
    assert_eq!(run2.amount, run1.amount);

    // A later period posts its own entry.
    let run3 = store.run_depreciation(id, "2026-02").unwrap();
    assert!(!run3.already_posted);
    assert_eq!(run3.amount, Decimal::from(1_000));

    // The depreciation entries hit the right accounts.
    let entry = store.ledger().get_entry(run3.entry_id.unwrap()).unwrap();
    assert!(entry.lines().iter().any(|l| l.account == expense
        && l.side == Side::Debit
        && l.amount.amount == Decimal::from(1_000)));
    store.verify().unwrap();
}

#[test]
fn depreciation_rejects_period_outside_life() {
    let (_dir, mut store) = open_store();
    let (cost, accum, expense, _, _) = asset_accounts(&mut store);
    let id = van(&mut store, 60_000, cost, accum, expense);
    // Before purchase.
    assert!(store.run_depreciation(id, "2025-12").is_err());
    // Malformed period.
    assert!(store.run_depreciation(id, "2026-13").is_err());
    assert!(store.run_depreciation(id, "january").is_err());
    // Unknown asset.
    assert!(store.run_depreciation(999, "2026-01").is_err());
    // After the 60-month life ends (2031-01): the run catches up every
    // month skipped, here all of them (plan/served.md P3, settled in
    // design: a month skipped is caught up in the month the run is made).
    let late = store.run_depreciation(id, "2031-06").unwrap();
    assert_eq!(late.amount, Decimal::from(60_000));
    assert_eq!(late.scheduled, Decimal::ZERO);
    assert!(late.entry_id.is_some());
    // Then nothing is left: a run after the life posts nothing, and
    // journals nothing.
    let head = store.head();
    let after = store.run_depreciation(id, "2031-07").unwrap();
    assert_eq!((after.amount, after.entry_id), (Decimal::ZERO, None));
    assert!(after.nothing_due.unwrap().contains("life is over"));
    assert_eq!(store.head(), head);
    store.verify().unwrap();
}

#[test]
fn disposal_posts_balanced_entry_and_marks_asset() {
    let (_dir, mut store) = open_store();
    let (cost, accum, expense, bank, gain) = asset_accounts(&mut store);
    let id = van(&mut store, 60_000, cost, accum, expense);
    store.run_depreciation(id, "2026-01").unwrap();
    store.run_depreciation(id, "2026-02").unwrap();

    // Book value after two months: 60000 − (548.39 + 1000) = 58451.61.
    // Sell for 60,000 → gain of 1548.39.
    let entry_id = store
        .dispose_asset(id, date(2026, 2, 28), usd("60000"), bank, gain)
        .unwrap();
    let entry = store.ledger().get_entry(entry_id).unwrap();
    let debits: Decimal = entry
        .lines()
        .iter()
        .filter(|l| l.side == Side::Debit)
        .map(|l| l.amount.amount)
        .sum();
    let credits: Decimal = entry
        .lines()
        .iter()
        .filter(|l| l.side == Side::Credit)
        .map(|l| l.amount.amount)
        .sum();
    assert_eq!(debits, credits, "disposal entry must balance");
    assert_eq!(debits, Decimal::new(154839, 2) + Decimal::from(60_000));

    // Asset is marked disposed; further depreciation is refused.
    assert!(matches!(
        store.ledger().get_asset(id).unwrap().status,
        ledger_core::AssetStatus::Disposed { .. }
    ));
    assert!(store.run_depreciation(id, "2026-03").is_err());
    assert!(
        store
            .dispose_asset(id, date(2026, 3, 1), usd("1"), bank, gain)
            .is_err()
    );
    store.verify().unwrap();
}

#[test]
fn disposal_at_loss_uses_debit_side() {
    let (_dir, mut store) = open_store();
    let (cost, accum, expense, bank, _) = asset_accounts(&mut store);
    let loss = store
        .add_account(
            "6900",
            "Loss on Disposal",
            AccountType::Expense,
            None,
            false,
        )
        .unwrap();
    let id = van(&mut store, 60_000, cost, accum, expense);
    // Dispose 2026-01-31 with no prior depreciation runs: the final-period
    // depreciation (Jan, 548.39) is auto-posted first. Book value 59451.61,
    // sell for 50,000 → loss 9451.61.
    let entry_id = store
        .dispose_asset(id, date(2026, 1, 31), usd("50000"), bank, loss)
        .unwrap();
    let entry = store.ledger().get_entry(entry_id).unwrap();
    let loss_line = entry.lines().iter().find(|l| l.account == loss).unwrap();
    assert_eq!(loss_line.side, Side::Debit);
    assert_eq!(loss_line.amount.amount, Decimal::new(945161, 2));
    // The auto-posted depreciation entry exists for the disposal month.
    let posted = store
        .ledger()
        .posted_entries()
        .filter(|e| e.description().contains("(final)"))
        .count();
    assert_eq!(posted, 1);
    store.verify().unwrap();
}

#[test]
fn rejected_asset_registration_journals_nothing() {
    let (_dir, mut store) = open_store();
    let (cost, accum, expense, _, _) = asset_accounts(&mut store);
    let bad = NewAsset {
        description: "Bad".to_string(),
        purchase_date: date(2026, 1, 1),
        cost: usd("1000"),
        salvage_value: usd("1000"), // salvage >= cost: invalid
        useful_life_months: 12,
        cost_account: cost,
        accum_depr_account: accum,
        expense_account: expense,
    };
    assert!(store.register_asset(bad).is_err());
    // Journal holds only the setup lines; no asset.registered.
    let events = store.events_since(0, 100).unwrap();
    assert!(events.iter().all(|e| e.event_type != "asset.registered"));
    // And the allocator did not advance.
    let id = van(&mut store, 1_000, cost, accum, expense);
    assert_eq!(id, 1);
}

// ---------------------------------------------- P3: defect 22, catch-up, R2, R3

/// Asset X of the P3 depreciation brief: 12,000.00, no salvage, 12 months
/// from 2026-01-01, so 1,000.00 a month and December its final month.
fn asset_x(store: &mut Store, cost: u64, accum: u64, expense: u64) -> u64 {
    store
        .register_asset(NewAsset {
            description: "Asset X".to_string(),
            purchase_date: date(2026, 1, 1),
            cost: usd("12000.00"),
            salvage_value: usd("0"),
            useful_life_months: 12,
            cost_account: cost,
            accum_depr_account: accum,
            expense_account: expense,
        })
        .unwrap()
}

/// An account's balance in the books (debit positive) on a day.
fn balance(store: &Store, account: u64, on: NaiveDate) -> Decimal {
    let code = &store.ledger().chart().get(account).unwrap().code;
    ledger_core::reports::trial_balance_as_of(store.ledger(), on)
        .lines
        .iter()
        .find(|l| &l.code == code)
        .map_or(Decimal::ZERO, |l| l.debit - l.credit)
}

fn dec(s: &str) -> Decimal {
    s.parse().unwrap()
}

/// Defect 22, reproduction C (wrong posted figures before P3): January
/// through November run (11,000.00), then a disposal on 2026-12-10 for
/// 1,000.00. Hand check: December holds 10 of its 31 days, 1,000.00 ×
/// 10/31 = 322.58; accumulated 11,322.58, book value 677.42, gain 1,000.00
/// − 677.42 = 322.58. Before P3: 1,000.00 of depreciation, book value 0.00,
/// gain 1,000.00 (expense and gain each 677.42 too high).
#[test]
fn defect22_a_disposal_in_the_final_month_posts_depreciation_by_days() {
    let (_dir, mut store) = open_store();
    let (cost, accum, expense, bank, gain) = asset_accounts(&mut store);
    let x = asset_x(&mut store, cost, accum, expense);
    for m in 1..=11 {
        store.run_depreciation(x, &format!("2026-{m:02}")).unwrap();
    }
    let p = store
        .dispose_preview(x, date(2026, 12, 10), usd("1000.00"))
        .unwrap();
    assert_eq!(p.final_depreciation, dec("322.58"));
    assert_eq!(p.accumulated, dec("11322.58"));
    assert_eq!(p.book_value, dec("677.42"));
    assert_eq!(p.gain_loss, dec("322.58"));
    assert_eq!(p.excess_depreciation, Decimal::ZERO);
    store
        .dispose_asset(x, date(2026, 12, 10), usd("1000.00"), bank, gain)
        .unwrap();
    let on = date(2026, 12, 31);
    assert_eq!(balance(&store, expense, on), dec("11322.58"));
    assert_eq!(balance(&store, gain, on), dec("-322.58"));
    assert_eq!(balance(&store, accum, on), Decimal::ZERO);
    assert_eq!(balance(&store, cost, on), dec("-12000.00"));
    // The books' accumulated equals the disposed schedule's.
    let a = store.ledger().get_asset(x).unwrap();
    assert_eq!(
        ledger_core::accumulated_depreciation(a, NaiveDate::MAX).unwrap(),
        dec("11322.58")
    );
    store.verify().unwrap();
}

/// P3 depreciation review R2: a disposal dated inside a month already run
/// takes back the days after it. Hand check: January through June run
/// (June's 1,000.00 dated Jun 30), then a disposal on 2026-06-15 for
/// 7,000.00. Held through Jun 15: 5,500.00 of depreciation, book value
/// 6,500.00, gain 500.00. The entry: Dr accumulated depreciation 6,000.00
/// (all the runs posted), Dr bank 7,000.00 / Cr equipment 12,000.00, Cr
/// depreciation expense 500.00, Cr gain 500.00: 13,000.00 a side. Before
/// P3: a book value of 6,000.00 and a gain of 1,000.00.
#[test]
fn defect22_a_disposal_inside_a_posted_month_takes_back_the_extra_days() {
    let (_dir, mut store) = open_store();
    let (cost, accum, expense, bank, gain) = asset_accounts(&mut store);
    let x = asset_x(&mut store, cost, accum, expense);
    for m in 1..=6 {
        store.run_depreciation(x, &format!("2026-{m:02}")).unwrap();
    }
    let p = store
        .dispose_preview(x, date(2026, 6, 15), usd("7000.00"))
        .unwrap();
    assert_eq!(
        (
            p.final_depreciation,
            p.excess_depreciation,
            p.catch_up_depreciation
        ),
        (Decimal::ZERO, dec("500.00"), Decimal::ZERO)
    );
    assert_eq!(p.accumulated, dec("6000.00"));
    assert_eq!(p.book_value, dec("6500.00"));
    assert_eq!(p.gain_loss, dec("500.00"));
    let a = store.ledger().get_asset(x).unwrap().clone();
    let lines = Store::disposal_lines(&a, &p, dec("7000.00"), bank, gain);
    assert_eq!(
        lines,
        vec![
            (accum, Side::Debit, dec("6000.00")),
            (bank, Side::Debit, dec("7000.00")),
            (cost, Side::Credit, dec("12000.00")),
            (expense, Side::Credit, dec("500.00")),
            (gain, Side::Credit, dec("500.00")),
        ]
    );
    store
        .dispose_asset(x, date(2026, 6, 15), usd("7000.00"), bank, gain)
        .unwrap();
    let on = date(2026, 6, 30);
    assert_eq!(balance(&store, expense, on), dec("5500.00"));
    assert_eq!(balance(&store, gain, on), dec("-500.00"));
    assert_eq!(balance(&store, accum, on), Decimal::ZERO);
    store.verify().unwrap();
}

/// The catch-up rule (plan/served.md P3, settled in design: a month of
/// depreciation skipped is caught up in the month the run is made): run
/// January, skip February, run March: March posts 2,000.00 (caught up
/// 1,000.00); a February run after it posts nothing and journals nothing.
#[test]
fn a_skipped_month_is_caught_up_in_the_month_run() {
    let (_dir, mut store) = open_store();
    let (cost, accum, expense, _, _) = asset_accounts(&mut store);
    let x = asset_x(&mut store, cost, accum, expense);
    store.run_depreciation(x, "2026-01").unwrap();
    let mar = store.run_depreciation(x, "2026-03").unwrap();
    assert_eq!(
        (mar.amount, mar.scheduled),
        (dec("2000.00"), dec("1000.00"))
    );
    let head = store.head();
    let feb = store.run_depreciation(x, "2026-02").unwrap();
    assert_eq!((feb.amount, feb.entry_id), (Decimal::ZERO, None));
    assert!(feb.nothing_due.unwrap().contains("later run"));
    assert_eq!(store.head(), head);
    assert_eq!(balance(&store, accum, date(2026, 3, 31)), dec("-3000.00"));
    store.verify().unwrap();
}

/// P3 depreciation review R3: a run reversed within its month is
/// un-posted, so its month is due again; running it again needs journal
/// feature month_reposting (an older version would ignore the second line
/// for the month), which new books declare as they are created, so it
/// journals no declaration of its own (books an older version wrote wait
/// for their format upgrade: format.rs
/// a_write_waiting_for_the_upgrade_journals_nothing).
#[test]
fn a_reversed_depreciation_month_can_be_run_again() {
    let (dir, mut store) = open_store();
    let (cost, accum, expense, _, _) = asset_accounts(&mut store);
    let x = asset_x(&mut store, cost, accum, expense);
    let jan = store.run_depreciation(x, "2026-01").unwrap();
    let jan_entry = jan.entry_id.unwrap();
    store
        .reverse_entry(jan_entry, date(2026, 1, 31), "posted to the wrong asset")
        .unwrap();
    assert!(
        store
            .journal_format()
            .has_feature(ledger_store::format::MONTH_REPOSTING)
    );
    let declarations = |dir: &std::path::Path| {
        std::fs::read_to_string(dir.join(ledger_store::JOURNAL_FILE))
            .unwrap()
            .lines()
            .filter(|l| l.contains("journal.format_set"))
            .count()
    };
    let before = declarations(dir.path());
    let again = store.run_depreciation(x, "2026-01").unwrap();
    assert!(!again.already_posted);
    assert_eq!(again.amount, dec("1000.00"));
    assert_ne!(again.entry_id, Some(jan_entry));
    assert_eq!(*store.journal_format(), ledger_store::format::current());
    assert_eq!(
        declarations(dir.path()),
        before,
        "no declaration of its own"
    );
    // Idempotent again: the new run is the month's.
    let twice = store.run_depreciation(x, "2026-01").unwrap();
    assert!(twice.already_posted);
    assert_eq!(twice.entry_id, again.entry_id);
    // January's end carries January once: the first run, its reversal and
    // the run again are all dated Jan 31.
    assert_eq!(balance(&store, accum, date(2026, 1, 31)), dec("-1000.00"));
    assert_eq!(balance(&store, accum, date(2026, 2, 28)), dec("-1000.00"));
    store.verify().unwrap();
    // Reopened (the projection rebuilt from the journal, then caught up),
    // the same.
    drop(store);
    std::fs::remove_file(dir.path().join(ledger_store::db_file())).unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.verify().unwrap();
    let mut store = store;
    assert!(store.run_depreciation(x, "2026-01").unwrap().already_posted);
}

/// R3's example: January through May run, May reversed, then a disposal on
/// Jun 15 for 7,000.00. The runs in effect carry 4,000.00; the schedule
/// through Jun 15 is 5,500.00, so the final depreciation is 1,500.00 (May
/// again, and half of June), and the disposal clears 5,500.00: the
/// accumulated depreciation account ends at 0.00, never a 1,000.00 debit.
#[test]
fn a_reversed_run_before_a_disposal_leaves_nothing_behind() {
    let (_dir, mut store) = open_store();
    let (cost, accum, expense, bank, gain) = asset_accounts(&mut store);
    let x = asset_x(&mut store, cost, accum, expense);
    let mut may = 0;
    for m in 1..=5 {
        may = store
            .run_depreciation(x, &format!("2026-{m:02}"))
            .unwrap()
            .entry_id
            .unwrap();
    }
    store
        .reverse_entry(may, date(2026, 6, 1), "reversed")
        .unwrap();
    let p = store
        .dispose_preview(x, date(2026, 6, 15), usd("7000.00"))
        .unwrap();
    assert_eq!(p.final_depreciation, dec("1500.00"));
    assert_eq!(p.accumulated, dec("5500.00"));
    assert_eq!(p.gain_loss, dec("500.00"));
    store
        .dispose_asset(x, date(2026, 6, 15), usd("7000.00"), bank, gain)
        .unwrap();
    let on = date(2026, 6, 30);
    assert_eq!(balance(&store, accum, on), Decimal::ZERO);
    assert_eq!(balance(&store, expense, on), dec("5500.00"));
    store.verify().unwrap();
}

/// R3: a disposal can't be reversed (the register would still say
/// disposed), nor a run its disposal took as final; nor can a reversal be
/// reversed when that would post a month a second time. The ledger and
/// journal are untouched by each refusal.
#[test]
fn a_disposal_entry_cannot_be_reversed() {
    let (_dir, mut store) = open_store();
    let (cost, accum, expense, bank, gain) = asset_accounts(&mut store);
    let x = asset_x(&mut store, cost, accum, expense);
    let jan = store
        .run_depreciation(x, "2026-01")
        .unwrap()
        .entry_id
        .unwrap();
    // Re-instating a run whose month ran again would post it twice.
    let rev = store.reverse_entry(jan, date(2026, 1, 31), "oops").unwrap();
    let again = store
        .run_depreciation(x, "2026-01")
        .unwrap()
        .entry_id
        .unwrap();
    let head = store.head();
    match store.reverse_entry(rev, date(2026, 2, 2), "re-instate") {
        Err(ledger_store::StoreError::Ledger(
            ledger_core::LedgerError::AssetEntryIrreversible(e, why),
        )) => {
            assert_eq!(e, rev);
            assert!(why.contains(&format!("entry {again}")), "{why}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(store.head(), head);
    let disposal = store
        .dispose_asset(x, date(2026, 2, 15), usd("11000.00"), bank, gain)
        .unwrap();
    let head = store.head();
    for (entry, what) in [(disposal, "disposal"), (again, "disposed")] {
        match store.reverse_entry(entry, date(2026, 3, 1), "undo") {
            Err(ledger_store::StoreError::Ledger(
                ledger_core::LedgerError::AssetEntryIrreversible(_, why),
            )) => assert!(why.contains(what), "{why}"),
            other => panic!("{entry}: {other:?}"),
        }
        assert!(store.check_reverse_entry(entry, date(2026, 3, 1)).is_err());
    }
    assert_eq!(store.head(), head);
    store.verify().unwrap();
}

/// P3 depreciation review R10: a month whose depreciation rounds to 0.00
/// posts nothing and journals nothing (an entry can't carry a zero line);
/// the cent posts in the month it rounds into. Hand check: 0.01 over 2
/// months from 2026-01-01: half a cent after January rounds up, so
/// January posts 0.01 and February nothing. 0.01 over 3 months: January
/// nothing, February 0.01, March nothing.
#[test]
fn run_depreciation_posts_nothing_for_a_month_that_rounds_to_nothing() {
    let (_dir, mut store) = open_store();
    let (cost, accum, expense, _, _) = asset_accounts(&mut store);
    let tiny = store
        .register_asset(NewAsset {
            description: "Cable".to_string(),
            purchase_date: date(2026, 1, 1),
            cost: usd("0.01"),
            salvage_value: usd("0"),
            useful_life_months: 3,
            cost_account: cost,
            accum_depr_account: accum,
            expense_account: expense,
        })
        .unwrap();
    let head = store.head();
    let jan = store.run_depreciation(tiny, "2026-01").unwrap();
    assert_eq!((jan.amount, jan.entry_id), (Decimal::ZERO, None));
    assert!(jan.nothing_due.unwrap().contains("rounds to nothing"));
    assert_eq!(store.head(), head);
    let feb = store.run_depreciation(tiny, "2026-02").unwrap();
    assert_eq!(feb.amount, dec("0.01"));
    let mar = store.run_depreciation(tiny, "2026-03").unwrap();
    assert_eq!((mar.amount, mar.entry_id), (Decimal::ZERO, None));
    store.verify().unwrap();
}

/// R1: in yen, each run posts whole yen (¥100,000 over 12 months: ¥8,333,
/// ¥8,334, ¥8,333, ...), so every month can post.
#[test]
fn jpy_depreciation_runs_post_whole_yen() {
    let dir = tempdir().unwrap();
    let mut store = Store::create(dir.path(), Currency::JPY).unwrap();
    let add = |s: &mut Store, code: &str, t| s.add_account(code, code, t, None, false).unwrap();
    let cost = add(&mut store, "1500", AccountType::Asset);
    let accum = add(&mut store, "1501", AccountType::Asset);
    let expense = add(&mut store, "6100", AccountType::Expense);
    let yen = |a: &str| Money::new(dec(a), Currency::JPY);
    let id = store
        .register_asset(NewAsset {
            description: "Lathe".to_string(),
            purchase_date: date(2026, 1, 1),
            cost: yen("100000"),
            salvage_value: yen("0"),
            useful_life_months: 12,
            cost_account: cost,
            accum_depr_account: accum,
            expense_account: expense,
        })
        .unwrap();
    let got: Vec<Decimal> = (1..=3)
        .map(|m| {
            store
                .run_depreciation(id, &format!("2026-{m:02}"))
                .unwrap()
                .amount
        })
        .collect();
    assert_eq!(got, [dec("8333"), dec("8334"), dec("8333")]);
    store.verify().unwrap();
}

/// P3 review of the catch-up rule: a run reversed after its month ended
/// still carried that month at its end, so running the month again would
/// carry it twice there; the run of the reversal's month catches it up
/// instead. Hand check: January run (1,000.00, Jan 31), reversed Feb 2.
/// January again: nothing (Jan 31 carries 1,000.00, the schedule). February:
/// 2,000.00 (the schedule through Feb 28, 2,000.00, less the 0.00 carried
/// after the reversal). Jan 31 carries 1,000.00, Feb 28 2,000.00: each the
/// schedule, never 2,000.00 on Jan 31 (before this review it was).
#[test]
fn a_run_reversed_after_its_month_is_caught_up_in_the_reversals_month() {
    let (_dir, mut store) = open_store();
    let (cost, accum, expense, _, _) = asset_accounts(&mut store);
    let x = asset_x(&mut store, cost, accum, expense);
    let jan = store
        .run_depreciation(x, "2026-01")
        .unwrap()
        .entry_id
        .unwrap();
    store
        .reverse_entry(jan, date(2026, 2, 2), "posted twice by hand")
        .unwrap();
    let head = store.head();
    let again = store.run_depreciation(x, "2026-01").unwrap();
    assert_eq!((again.amount, again.entry_id), (Decimal::ZERO, None));
    assert!(
        again
            .nothing_due
            .as_deref()
            .unwrap()
            .contains("reversed after"),
        "{again:?}"
    );
    assert_eq!(store.head(), head, "nothing journaled");
    let feb = store.run_depreciation(x, "2026-02").unwrap();
    assert_eq!(
        (feb.amount, feb.scheduled),
        (dec("2000.00"), dec("1000.00"))
    );
    assert_eq!(balance(&store, accum, date(2026, 1, 31)), dec("-1000.00"));
    assert_eq!(balance(&store, accum, date(2026, 2, 28)), dec("-2000.00"));
    assert_eq!(*store.journal_format(), ledger_store::format::current());
    store.verify().unwrap();
}

/// P3 review: re-instating a reversed run (reversing its reversal) after a
/// later run caught its month up would carry the books past the schedule,
/// here past cost less salvage. Hand check: January through November run
/// (11,000.00); November reversed on Nov 30; December's run posts 2,000.00
/// (the whole 12,000.00 less the 10,000.00 carried). Re-instating November
/// on Dec 31 would carry 13,000.00 of a 12,000.00 asset: refused, journaling
/// nothing. A run re-instated with nothing run since (January, reversed and
/// re-instated on Jan 31, room 1,000.00) is taken.
#[test]
fn reinstating_a_run_a_later_run_caught_up_is_refused() {
    let (_dir, mut store) = open_store();
    let (cost, accum, expense, _, _) = asset_accounts(&mut store);
    let x = asset_x(&mut store, cost, accum, expense);
    let mut nov = 0;
    for m in 1..=11 {
        nov = store
            .run_depreciation(x, &format!("2026-{m:02}"))
            .unwrap()
            .entry_id
            .unwrap();
    }
    let rev = store
        .reverse_entry(nov, date(2026, 11, 30), "check the count")
        .unwrap();
    let dec_run = store.run_depreciation(x, "2026-12").unwrap();
    assert_eq!(dec_run.amount, dec("2000.00"));
    let head = store.head();
    match store.reverse_entry(rev, date(2026, 12, 31), "it was right") {
        Err(ledger_store::StoreError::Ledger(
            ledger_core::LedgerError::AssetEntryIrreversible(e, why),
        )) => {
            assert_eq!(e, rev);
            assert!(why.contains("more than its schedule"), "{why}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(store.head(), head);
    assert_eq!(balance(&store, accum, date(2026, 12, 31)), dec("-12000.00"));
    store.verify().unwrap();

    // Before a later run, re-instating is room the schedule has: taken.
    let (_dir, mut store) = open_store();
    let (cost, accum, expense, _, _) = asset_accounts(&mut store);
    let x = asset_x(&mut store, cost, accum, expense);
    let jan = store
        .run_depreciation(x, "2026-01")
        .unwrap()
        .entry_id
        .unwrap();
    let rev = store.reverse_entry(jan, date(2026, 1, 31), "oops").unwrap();
    store
        .reverse_entry(rev, date(2026, 1, 31), "it was right")
        .unwrap();
    assert_eq!(balance(&store, accum, date(2026, 1, 31)), dec("-1000.00"));
    assert!(store.run_depreciation(x, "2026-01").unwrap().already_posted);
    store.verify().unwrap();
}

/// P3 review: a disposal dated before a later month's run would credit
/// that run back at the disposal day, leaving the disposal month's
/// accumulated depreciation a debit and its expense short at its end; it is
/// refused, naming the run to reverse first. Hand check: January through
/// July run (7,000.00); a disposal on Jun 15 is refused. July reversed on
/// Jul 31: the disposal takes 5,500.00 (June's last 15 days, 500.00,
/// credited back), and on Jun 30 the books carry accumulated depreciation
/// 0.00 and expense 5,500.00; before this review June ended with a
/// 1,000.00 debit in accumulated depreciation and 4,500.00 of expense.
#[test]
fn a_disposal_before_a_later_months_run_is_refused_until_it_is_reversed() {
    let (_dir, mut store) = open_store();
    let (cost, accum, expense, bank, gain) = asset_accounts(&mut store);
    let x = asset_x(&mut store, cost, accum, expense);
    let mut jul = 0;
    for m in 1..=7 {
        jul = store
            .run_depreciation(x, &format!("2026-{m:02}"))
            .unwrap()
            .entry_id
            .unwrap();
    }
    let head = store.head();
    for refused in [
        store
            .dispose_preview(x, date(2026, 6, 15), usd("7000.00"))
            .map(|_| ()),
        store
            .dispose_asset(x, date(2026, 6, 15), usd("7000.00"), bank, gain)
            .map(|_| ()),
    ] {
        match refused {
            Err(ledger_store::StoreError::Ledger(
                ledger_core::LedgerError::DepreciationPastDisposal(why),
            )) => assert!(why.contains(&format!("2026-07 (entry {jul})")), "{why}"),
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(store.head(), head);
    store
        .reverse_entry(jul, date(2026, 7, 31), "sold in June")
        .unwrap();
    let p = store
        .dispose_preview(x, date(2026, 6, 15), usd("7000.00"))
        .unwrap();
    assert_eq!(
        (p.excess_depreciation, p.book_value, p.gain_loss),
        (dec("500.00"), dec("6500.00"), dec("500.00"))
    );
    store
        .dispose_asset(x, date(2026, 6, 15), usd("7000.00"), bank, gain)
        .unwrap();
    for on in [date(2026, 6, 30), date(2026, 7, 31)] {
        assert_eq!(balance(&store, accum, on), Decimal::ZERO, "{on}");
        assert_eq!(balance(&store, expense, on), dec("5500.00"), "{on}");
    }
    store.verify().unwrap();
}

/// P3 review: a run of the disposal's month or before reversed after the
/// disposal month's end would be charged again at the disposal and taken
/// away only later: refused until that change is undone within its month.
/// Hand check: January through May run; May reversed on Aug 1; a disposal
/// on Jun 15 would charge 1,500.00 (May again, and half of June) and so
/// carry 6,500.00 on Jun 30. Refused; the reversal reversed on Aug 3 (May
/// stands again, what it carries the same at every month end from June),
/// the disposal charges 500.00 and June ends at 0.00 and 5,500.00.
#[test]
fn a_disposal_before_a_later_reversal_of_an_earlier_run_is_refused() {
    let (_dir, mut store) = open_store();
    let (cost, accum, expense, bank, gain) = asset_accounts(&mut store);
    let x = asset_x(&mut store, cost, accum, expense);
    let mut may = 0;
    for m in 1..=5 {
        may = store
            .run_depreciation(x, &format!("2026-{m:02}"))
            .unwrap()
            .entry_id
            .unwrap();
    }
    let rev = store
        .reverse_entry(may, date(2026, 8, 1), "doubted")
        .unwrap();
    match store.dispose_preview(x, date(2026, 6, 15), usd("7000.00")) {
        Err(ledger_store::StoreError::Ledger(
            ledger_core::LedgerError::DepreciationPastDisposal(why),
        )) => assert!(
            why.contains(&format!("by entry {rev} on 2026-08-01")),
            "{why}"
        ),
        other => panic!("{other:?}"),
    }
    store
        .reverse_entry(rev, date(2026, 8, 3), "it was right")
        .unwrap();
    let p = store
        .dispose_preview(x, date(2026, 6, 15), usd("7000.00"))
        .unwrap();
    assert_eq!(p.final_depreciation, dec("500.00"));
    store
        .dispose_asset(x, date(2026, 6, 15), usd("7000.00"), bank, gain)
        .unwrap();
    let on = date(2026, 6, 30);
    assert_eq!(balance(&store, accum, on), Decimal::ZERO);
    assert_eq!(balance(&store, expense, on), dec("5500.00"));
    store.verify().unwrap();
}

/// P3 review round 2: a run re-instated after its month was run again and
/// that second run reversed still counts. Hand check: January run (A, Jan
/// 31), reversed on Jan 31 (B); January run again (A2), reversed on Jan 31
/// (B2); then B reversed on Jan 31, re-instating A: the room is the whole
/// 1,000.00 (nothing else carries January). The books carry A, so January
/// is posted (A's run), and a disposal on Feb 15 for 11,000.00 charges
/// only February's 15 of 28 days, 535.71, clearing 1,535.71 (book value
/// 10,464.29, gain 535.71): the accumulated depreciation account ends at
/// 0.00 and the expense at 1,535.71. Before this review the projection's
/// January row still named A2, so A was left out: January ran as nothing
/// due, and the disposal charged January again (1,535.71 final), leaving
/// 1,000.00 in accumulated depreciation.
#[test]
fn a_run_reinstated_after_its_rerun_was_reversed_still_counts() {
    let (dir, mut store) = open_store();
    let (cost, accum, expense, bank, gain) = asset_accounts(&mut store);
    let x = asset_x(&mut store, cost, accum, expense);
    let jan31 = date(2026, 1, 31);
    let a = store
        .run_depreciation(x, "2026-01")
        .unwrap()
        .entry_id
        .unwrap();
    let b = store.reverse_entry(a, jan31, "first doubt").unwrap();
    let a2 = store
        .run_depreciation(x, "2026-01")
        .unwrap()
        .entry_id
        .unwrap();
    store.reverse_entry(a2, jan31, "second doubt").unwrap();
    store
        .reverse_entry(b, jan31, "the first was right")
        .unwrap();
    assert_eq!(balance(&store, accum, jan31), dec("-1000.00"));
    let again = store.run_depreciation(x, "2026-01").unwrap();
    assert!(again.already_posted, "{again:?}");
    assert_eq!((again.entry_id, again.amount), (Some(a), dec("1000.00")));
    let p = store
        .dispose_preview(x, date(2026, 2, 15), usd("11000.00"))
        .unwrap();
    assert_eq!(
        (p.final_depreciation, p.accumulated, p.gain_loss),
        (dec("535.71"), dec("1535.71"), dec("535.71"))
    );
    store
        .dispose_asset(x, date(2026, 2, 15), usd("11000.00"), bank, gain)
        .unwrap();
    let on = date(2026, 2, 28);
    assert_eq!(balance(&store, accum, on), Decimal::ZERO);
    assert_eq!(balance(&store, expense, on), dec("1535.71"));
    store.verify().unwrap();
    // The same after the projection is rebuilt from the journal.
    drop(store);
    std::fs::remove_file(dir.path().join(ledger_store::db_file())).unwrap();
    Store::open(dir.path()).unwrap().verify().unwrap();
}

/// P3 review round 2: a correction comes after what it corrects (B-9). A
/// reversal of a depreciation run dated before the run (March's run, dated
/// Mar 31, reversed on Feb 10) would leave January and February with
/// negative depreciation, and a disposal on Feb 15 clearing what February
/// doesn't carry (before this review it was taken: February ended with
/// accumulated depreciation a 1,000.00 debit and 535.71 of expense, not
/// 1,535.71). So is a re-instatement dated before the reversal it reverses,
/// and a schedule month's reversal dated before its posting. A reversal on
/// the entry's own day is taken. Each refusal journals nothing.
#[test]
fn a_reversal_dated_before_the_entry_it_reverses_is_refused() {
    let (_dir, mut store) = open_store();
    let (cost, accum, expense, _, _) = asset_accounts(&mut store);
    let x = asset_x(&mut store, cost, accum, expense);
    let mut mar = 0;
    for m in 1..=3 {
        mar = store
            .run_depreciation(x, &format!("2026-{m:02}"))
            .unwrap()
            .entry_id
            .unwrap();
    }
    let head = store.head();
    for on in [date(2026, 2, 10), date(2026, 3, 30)] {
        match store.reverse_entry(mar, on, "backdated") {
            Err(ledger_store::StoreError::Ledger(
                ledger_core::LedgerError::AssetEntryIrreversible(e, why),
            )) => {
                assert_eq!(e, mar);
                assert!(why.contains("can't be dated before it"), "{why}");
            }
            other => panic!("{on}: {other:?}"),
        }
    }
    assert_eq!(store.head(), head);
    let rev = store
        .reverse_entry(mar, date(2026, 3, 31), "doubted")
        .unwrap();
    let head = store.head();
    assert!(matches!(
        store.reverse_entry(rev, date(2026, 3, 30), "it was right"),
        Err(ledger_store::StoreError::Ledger(
            ledger_core::LedgerError::AssetEntryIrreversible(..)
        ))
    ));
    assert_eq!(store.head(), head);
    store
        .reverse_entry(rev, date(2026, 3, 31), "it was right")
        .unwrap();
    assert_eq!(balance(&store, accum, date(2026, 2, 28)), dec("-2000.00"));
    assert_eq!(balance(&store, accum, date(2026, 3, 31)), dec("-3000.00"));

    // A schedule month's posting, the same.
    let prepaid = store
        .add_account("1300", "Prepaid", AccountType::Asset, None, false)
        .unwrap();
    let insurance = store
        .add_account("6300", "Insurance", AccountType::Expense, None, false)
        .unwrap();
    let s = store
        .register_schedule(ledger_core::NewSchedule {
            description: "Insurance".into(),
            kind: ledger_core::ScheduleKind::PrepaidExpense,
            total: usd("1200.00"),
            start: date(2026, 1, 1),
            months: 12,
            balance_account: prepaid,
            pl_account: insurance,
            convention: ledger_core::ScheduleConvention::MonthlyEven,
        })
        .unwrap();
    let feb = store.post_schedule_month(s, date(2026, 2, 28)).unwrap();
    let head = store.head();
    assert!(matches!(
        store.reverse_entry(feb, date(2026, 1, 31), "backdated"),
        Err(ledger_store::StoreError::Ledger(
            ledger_core::LedgerError::InvalidSchedule(_)
        ))
    ));
    assert_eq!(store.head(), head);
    store
        .reverse_entry(feb, date(2026, 2, 28), "doubted")
        .unwrap();
    store.verify().unwrap();
}

/// P3 review round 2: a disposal waits only until what the runs carry at
/// the end of its month is what they carry now, so a late change can be
/// undone on any open day. Hand check: January through May run (5,000.00);
/// May reversed on Aug 1, and August closed. A disposal on Jun 15 for
/// 7,000.00 is refused (it would charge May again: June would end carrying
/// 6,500.00). May re-instated on Sep 2 (August is closed, so it can't be
/// undone in August): the disposal is taken, charging June's 15 days,
/// 500.00, and clearing 5,500.00. June ends at accumulated depreciation
/// 0.00 and expense 5,500.00, gain 500.00; August's end carries the change
/// made in August (May's 1,000.00 taken away, a 1,000.00 debit) and
/// September's end its correction (0.00), as without the disposal. Before
/// this review the disposal was refused for ever: round 1 required what May
/// carries at every month end after June to be the same.
#[test]
fn a_disposal_is_taken_once_a_late_change_is_undone_after_its_month_closed() {
    let (_dir, mut store) = open_store();
    let (cost, accum, expense, bank, gain) = asset_accounts(&mut store);
    let x = asset_x(&mut store, cost, accum, expense);
    let mut may = 0;
    for m in 1..=5 {
        may = store
            .run_depreciation(x, &format!("2026-{m:02}"))
            .unwrap()
            .entry_id
            .unwrap();
    }
    let rev = store
        .reverse_entry(may, date(2026, 8, 1), "doubted")
        .unwrap();
    let aug = store
        .create_period("2026-08", date(2026, 8, 1), date(2026, 8, 31))
        .unwrap();
    store.close_period(aug).unwrap();
    assert!(matches!(
        store.dispose_preview(x, date(2026, 6, 15), usd("7000.00")),
        Err(ledger_store::StoreError::Ledger(
            ledger_core::LedgerError::DepreciationPastDisposal(_)
        ))
    ));
    store
        .reverse_entry(rev, date(2026, 9, 2), "it was right")
        .unwrap();
    let p = store
        .dispose_preview(x, date(2026, 6, 15), usd("7000.00"))
        .unwrap();
    assert_eq!(
        (p.final_depreciation, p.accumulated, p.gain_loss),
        (dec("500.00"), dec("5500.00"), dec("500.00"))
    );
    store
        .dispose_asset(x, date(2026, 6, 15), usd("7000.00"), bank, gain)
        .unwrap();
    let jun = date(2026, 6, 30);
    assert_eq!(balance(&store, accum, jun), Decimal::ZERO);
    assert_eq!(balance(&store, expense, jun), dec("5500.00"));
    assert_eq!(balance(&store, gain, jun), dec("-500.00"));
    assert_eq!(balance(&store, accum, date(2026, 8, 31)), dec("1000.00"));
    assert_eq!(balance(&store, accum, date(2026, 9, 30)), Decimal::ZERO);
    store.verify().unwrap();
}

/// P3 review round 2 (no store test held it): depreciation still due when
/// the disposal's month already has a run is charged in the disposal entry
/// (catch_up_depreciation), never as a second run of that month. Hand
/// check: January through June run (June's 1,000.00 dated Jun 30); March's
/// run reversed on Jun 20 (5,000.00 carried at June's end, and now). A
/// disposal on Jun 15 for 7,000.00 takes 5,500.00 (held through Jun 15):
/// 500.00 still due, debited to depreciation expense in the disposal entry
/// (March's 1,000.00 again, less June's last 15 days already in June's
/// run). The entry: Dr accumulated depreciation 5,000.00, Dr bank
/// 7,000.00, Dr depreciation expense 500.00 / Cr equipment 12,000.00, Cr
/// gain 500.00: 12,500.00 a side. No final depreciation entry, June keeps
/// one run, no journal feature; June ends at 0.00 and 5,500.00.
#[test]
fn a_disposal_whose_month_has_run_charges_what_is_still_due_in_its_entry() {
    let (dir, mut store) = open_store();
    let (cost, accum, expense, bank, gain) = asset_accounts(&mut store);
    let x = asset_x(&mut store, cost, accum, expense);
    let mut mar = 0;
    for m in 1..=6 {
        let r = store.run_depreciation(x, &format!("2026-{m:02}")).unwrap();
        if m == 3 {
            mar = r.entry_id.unwrap();
        }
    }
    store
        .reverse_entry(mar, date(2026, 6, 20), "doubted")
        .unwrap();
    let p = store
        .dispose_preview(x, date(2026, 6, 15), usd("7000.00"))
        .unwrap();
    assert_eq!(
        (
            p.final_depreciation,
            p.catch_up_depreciation,
            p.excess_depreciation
        ),
        (Decimal::ZERO, dec("500.00"), Decimal::ZERO)
    );
    assert_eq!(
        (p.accumulated, p.book_value, p.gain_loss),
        (dec("5000.00"), dec("6500.00"), dec("500.00"))
    );
    let a = store.ledger().get_asset(x).unwrap().clone();
    assert_eq!(
        Store::disposal_lines(&a, &p, dec("7000.00"), bank, gain),
        vec![
            (accum, Side::Debit, dec("5000.00")),
            (bank, Side::Debit, dec("7000.00")),
            (expense, Side::Debit, dec("500.00")),
            (cost, Side::Credit, dec("12000.00")),
            (gain, Side::Credit, dec("500.00")),
        ]
    );
    let next = store.ledger().next_entry_id();
    let disposal = store
        .dispose_asset(x, date(2026, 6, 15), usd("7000.00"), bank, gain)
        .unwrap();
    assert_eq!(disposal, next, "the disposal entry alone");
    let journal = std::fs::read_to_string(dir.path().join(ledger_store::JOURNAL_FILE)).unwrap();
    let junes = journal
        .lines()
        .filter(|l| l.contains("asset.depreciation_posted") && l.contains("\"2026-06\""))
        .count();
    assert_eq!(junes, 1, "June keeps one run");
    assert_eq!(*store.journal_format(), ledger_store::format::current());
    let on = date(2026, 6, 30);
    assert_eq!(balance(&store, accum, on), Decimal::ZERO);
    assert_eq!(balance(&store, expense, on), dec("5500.00"));
    assert_eq!(balance(&store, gain, on), dec("-500.00"));
    store.verify().unwrap();
}
