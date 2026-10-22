//! No command reaches an overflow or a date chrono can't hold: every amount
//! a new command takes is at most a journal line's
//! (kernel::MAX_LINE_AMOUNT), every date is in the calendar the books keep
//! (ledger_core::calendar), and what can't be computed exactly is refused,
//! never wrapped or panicked on (P0 review round 3, finding S1: a panic
//! stops the books server). The bounds are the commands' only: replay
//! accepts every line an earlier version accepted (rules.md B-28, posted
//! is forever; P0 review round 4), so each bound here has its pair, a
//! replay of an old line past it, accepted.
//!
//! ```text
//! GOAL: a command the books can't hold is refused with a reason, the
//!       ledger unchanged; a replayed line an earlier version wrote past
//!       the same bound is accepted exactly as written.
//! SOURCE: DECISION (P0 review round 3, 2026-10-16, S1; round 4,
//!         2026-10-17); kernel.rs MAX_LINE_AMOUNT (rules.md: a line over a
//!         quadrillion is a mistake); calendar.rs (four-digit years).
//! ```

use chrono::{Datelike, NaiveDate};
use ledger_core::kernel::MAX_LINE_AMOUNT;
use ledger_core::{
    AccountId, AccountType, Currency, DocumentLine, Frequency, Ledger, LedgerError, Money,
    NewRecurring, NewSchedule, PayrollLine, Side,
};
use rust_decimal::Decimal;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn usd(a: Decimal) -> Money {
    Money::new(a, Currency::USD)
}

/// One past the largest a line may be.
fn too_big() -> Decimal {
    MAX_LINE_AMOUNT + Decimal::new(1, 2)
}

struct Books {
    ledger: Ledger,
    cash: AccountId,
    prepaid: AccountId,
    revenue: AccountId,
    expense: AccountId,
}

fn books() -> Books {
    let mut ledger = Ledger::new(Currency::USD);
    let mut add = |code: &str, ty| ledger.add_account(code, code, ty, None, false).unwrap();
    let cash = add("1000", AccountType::Asset);
    let prepaid = add("1300", AccountType::Asset);
    let revenue = add("4000", AccountType::Revenue);
    let expense = add("6000", AccountType::Expense);
    add("3000", AccountType::Equity);
    Books {
        ledger,
        cash,
        prepaid,
        revenue,
        expense,
    }
}

/// A date outside the calendar is refused wherever a command takes one:
/// a draft, a period, a schedule's months, a recurring entry, a document.
#[test]
fn dates_outside_the_calendar_are_refused_everywhere() {
    let mut b = books();
    for far in [NaiveDate::MAX, NaiveDate::MIN, d(10000, 1, 1), d(0, 12, 31)] {
        assert_eq!(
            b.ledger.try_create_entry(far, "far").unwrap_err(),
            LedgerError::DateOutOfRange(far)
        );
        assert_eq!(
            b.ledger.create_period("far", far, far).unwrap_err(),
            LedgerError::DateOutOfRange(far)
        );
    }
    assert_eq!(
        b.ledger
            .create_period("far", d(2026, 1, 1), d(10000, 1, 1))
            .unwrap_err(),
        LedgerError::DateOutOfRange(d(10000, 1, 1))
    );
    b.ledger.try_create_entry(d(1, 1, 1), "first day").unwrap();
    b.ledger
        .try_create_entry(d(9999, 12, 31), "last day")
        .unwrap();
    b.ledger
        .create_period("last", d(9999, 12, 1), d(9999, 12, 31))
        .unwrap();
    // A schedule whose months run past the calendar's last day.
    let schedule = |start: NaiveDate, months: u32| NewSchedule {
        description: "Insurance".into(),
        kind: ledger_core::schedules::ScheduleKind::PrepaidExpense,
        total: usd(Decimal::new(30000, 2)),
        start,
        months,
        balance_account: b.prepaid,
        pl_account: b.expense,
        convention: ledger_core::ScheduleConvention::MonthlyEven,
    };
    for (start, months) in [
        (d(9999, 12, 15), 2),
        (d(9999, 1, 1), 13),
        (NaiveDate::MAX, 1),
        (d(0, 6, 1), 1),
    ] {
        assert!(
            b.ledger.register_schedule(schedule(start, months)).is_err(),
            "{start} + {months}"
        );
    }
    b.ledger
        .register_schedule(schedule(d(9999, 1, 1), 12))
        .unwrap();
    // A recurring entry starting or ending outside it.
    let recurring = |start: NaiveDate, end: Option<NaiveDate>| NewRecurring {
        description: "Rent".into(),
        lines: vec![
            (b.expense, Side::Debit, usd(Decimal::ONE)),
            (b.cash, Side::Credit, usd(Decimal::ONE)),
        ],
        frequency: Frequency::Monthly,
        start,
        end,
    };
    for (start, end) in [
        (NaiveDate::MIN, None),
        (d(2026, 1, 1), Some(NaiveDate::MAX)),
    ] {
        assert!(matches!(
            b.ledger.register_recurring(recurring(start, end)),
            Err(LedgerError::DateOutOfRange(_))
        ));
    }
    // An invoice due outside it.
    let customer = b.ledger.add_customer("Client").unwrap();
    let line = vec![DocumentLine {
        description: "work".into(),
        amount: Decimal::ONE,
        class_id: None,
    }];
    assert_eq!(
        b.ledger
            .create_invoice(customer, d(2026, 1, 1), NaiveDate::MAX, line.clone())
            .unwrap_err(),
        LedgerError::DateOutOfRange(NaiveDate::MAX)
    );
    let _ = b.revenue;
}

/// The pairs of dates_outside_the_calendar_are_refused_everywhere: each
/// replay of an old line dated past the calendar is accepted as written.
#[test]
fn replayed_dates_outside_the_calendar_are_accepted() {
    let mut b = books();
    for far in [NaiveDate::MAX, NaiveDate::MIN, d(10000, 1, 1), d(0, 12, 31)] {
        let e = b.ledger.create_entry(far, "far");
        assert_eq!(b.ledger.get_entry(e).unwrap().date(), far);
        let p = b.ledger.replay_create_period("far", far, far).unwrap();
        assert_eq!(b.ledger.get_period(p).unwrap().start, far);
    }
    // Only the order is checked on replay.
    assert!(matches!(
        b.ledger
            .replay_create_period("backwards", d(2026, 2, 1), d(2026, 1, 1)),
        Err(LedgerError::InvalidPeriodRange { .. })
    ));
    let schedule = |start: NaiveDate, months: u32| NewSchedule {
        description: "Insurance".into(),
        kind: ledger_core::schedules::ScheduleKind::PrepaidExpense,
        total: usd(Decimal::new(30000, 2)),
        start,
        months,
        balance_account: b.prepaid,
        pl_account: b.expense,
        convention: ledger_core::ScheduleConvention::MonthlyEven,
    };
    for (start, months) in [
        (d(9999, 12, 15), 2),
        (d(9999, 6, 1), 12),
        (NaiveDate::MAX, 1),
        (d(0, 6, 1), 1),
    ] {
        let id = b
            .ledger
            .replay_register_schedule(schedule(start, months))
            .unwrap_or_else(|e| panic!("{start} + {months}: {e}"));
        assert_eq!(b.ledger.get_schedule(id).unwrap().start, start);
    }
    // The month bounds are main's: 1 to 600.
    assert!(
        b.ledger
            .replay_register_schedule(schedule(d(2026, 1, 1), 601))
            .is_err()
    );
    let recurring = |start: NaiveDate, end: Option<NaiveDate>| NewRecurring {
        description: "Rent".into(),
        lines: vec![
            (b.expense, Side::Debit, usd(Decimal::ONE)),
            (b.cash, Side::Credit, usd(Decimal::ONE)),
        ],
        frequency: Frequency::Monthly,
        start,
        end,
    };
    for (start, end) in [
        (NaiveDate::MIN, None),
        (d(2026, 1, 1), Some(NaiveDate::MAX)),
    ] {
        b.ledger
            .replay_register_recurring(recurring(start, end))
            .unwrap();
    }
    let customer = b.ledger.add_customer("Client").unwrap();
    let vendor = b.ledger.add_vendor("Supplier").unwrap();
    let line = vec![DocumentLine {
        description: "work".into(),
        amount: Decimal::ONE,
        class_id: None,
    }];
    b.ledger
        .replay_invoice(1, customer, NaiveDate::MIN, NaiveDate::MAX, line.clone(), 1)
        .unwrap();
    b.ledger
        .replay_bill(1, vendor, d(0, 1, 1), d(10000, 1, 1), line, 1)
        .unwrap();
    let mut add = |code: &str, ty| b.ledger.add_account(code, code, ty, None, false).unwrap();
    let equipment = add("1500", AccountType::Asset);
    let accum = add("1501", AccountType::Asset);
    let depreciation = add("6100", AccountType::Expense);
    let asset = |in_service_date: NaiveDate, useful_life_months: u32| ledger_core::NewAsset {
        description: "Late".into(),
        in_service_date,
        cost: usd(Decimal::new(120000, 2)),
        salvage_value: usd(Decimal::ZERO),
        useful_life_months,
        cost_account: equipment,
        accum_depr_account: accum,
        expense_account: depreciation,
    };
    for (date, life) in [
        (d(9999, 1, 1), 60),
        (d(2026, 1, 1), 120_000),
        (d(0, 1, 1), 1),
    ] {
        assert_eq!(
            b.ledger.register_asset(asset(date, life)).unwrap_err(),
            if date.year() == 0 {
                LedgerError::DateOutOfRange(date)
            } else {
                LedgerError::AssetDateOverflow
            }
        );
        b.ledger.replay_register_asset(asset(date, life)).unwrap();
    }
    // Replay keeps main's one date rule: the life's end is a date chrono
    // holds.
    assert_eq!(
        b.ledger
            .replay_register_asset(asset(d(262_142, 1, 1), 12))
            .unwrap_err(),
        LedgerError::AssetDateOverflow
    );
    let id = b
        .ledger
        .replay_register_asset(asset(d(2026, 1, 1), 12))
        .unwrap();
    let fix = |in_service_date: NaiveDate| ledger_core::AssetCorrection {
        cost: usd(Decimal::new(120000, 2)),
        salvage_value: usd(Decimal::ZERO),
        useful_life_months: 60,
        in_service_date,
    };
    assert_eq!(
        b.ledger.correct_asset(id, fix(d(9999, 1, 1))).unwrap_err(),
        LedgerError::AssetDateOverflow
    );
    b.ledger
        .replay_correct_asset(id, fix(d(9999, 1, 1)))
        .unwrap();
    assert_eq!(
        b.ledger.get_asset(id).unwrap().in_service_date,
        d(9999, 1, 1)
    );
    // A disposal dated past the calendar.
    let proceeds = usd(Decimal::ZERO);
    assert_eq!(
        b.ledger
            .dispose_asset(id, d(10000, 1, 1), proceeds)
            .unwrap_err(),
        LedgerError::DateOutOfRange(d(10000, 1, 1))
    );
    b.ledger
        .replay_dispose_asset(id, d(10000, 1, 1), proceeds)
        .unwrap();
}

/// An amount larger than a line may be is refused wherever a command takes
/// one, so every sum the books make of them is exact.
#[test]
fn amounts_larger_than_a_line_are_refused_everywhere() {
    let mut b = books();
    let big = too_big();
    // A draft line, as it is added; replayed as an older version wrote it.
    let e = b.ledger.try_create_entry(d(2026, 1, 1), "big").unwrap();
    assert_eq!(
        b.ledger
            .add_line(e, b.expense, Side::Debit, usd(big))
            .unwrap_err(),
        LedgerError::AmountOutOfRange
    );
    for a in [big, Decimal::MAX, -Decimal::MAX] {
        b.ledger
            .replay_add_line(e, b.expense, Side::Debit, usd(a))
            .unwrap();
    }
    assert_eq!(b.ledger.get_entry(e).unwrap().lines().len(), 3);
    // Its post refuses it, never a panic.
    assert!(b.ledger.post_entry(e).is_err());
    // Document lines, and their replay.
    let customer = b.ledger.add_customer("Client").unwrap();
    let lines = |a: Decimal| {
        vec![
            DocumentLine {
                description: "a".into(),
                amount: a,
                class_id: None,
            },
            DocumentLine {
                description: "b".into(),
                amount: Decimal::ONE,
                class_id: None,
            },
        ]
    };
    for a in [big, Decimal::MAX, Decimal::new(1, 11)] {
        assert!(matches!(
            b.ledger
                .create_invoice(customer, d(2026, 1, 1), d(2026, 2, 1), lines(a)),
            Err(LedgerError::ValueOutOfRange(_))
        ));
    }
    // Replayed as journaled: whatever its lines, a document is restored;
    // a total past exact arithmetic is refused where it is computed.
    for (id, a) in [(1, big), (2, Decimal::new(1, 11)), (3, Decimal::MAX)] {
        b.ledger
            .replay_invoice(id, customer, d(2026, 1, 1), d(2026, 2, 1), lines(a), id)
            .unwrap();
    }
    assert_eq!(
        b.ledger.get_invoice(2).unwrap().total(),
        Ok(Decimal::new(100_000_000_001, 11))
    );
    assert!(matches!(
        b.ledger.get_invoice(3).unwrap().total(),
        Err(LedgerError::TooLargeToCompute(_))
    ));
    assert!(matches!(
        b.ledger.ar_aging(d(2026, 3, 1)),
        Err(LedgerError::TooLargeToCompute(_))
    ));
    // A recurring template's lines.
    let recurring = NewRecurring {
        description: "x".into(),
        lines: vec![
            (b.expense, Side::Debit, usd(Decimal::MAX)),
            (b.expense, Side::Debit, usd(Decimal::ONE)),
            (b.cash, Side::Credit, usd(Decimal::ONE)),
        ],
        frequency: Frequency::Monthly,
        start: d(2026, 1, 1),
        end: None,
    };
    assert_eq!(
        b.ledger.register_recurring(recurring.clone()).unwrap_err(),
        LedgerError::AmountOutOfRange
    );
    // On replay, one whose debits overflow Decimal is refused (no version
    // could have written it: main panicked on it), never a panic; one past
    // the line limit that balances is accepted.
    assert!(matches!(
        b.ledger.replay_register_recurring(recurring),
        Err(LedgerError::TooLargeToCompute(_))
    ));
    let half = Decimal::MAX / Decimal::TWO;
    b.ledger
        .replay_register_recurring(NewRecurring {
            description: "x".into(),
            lines: vec![
                (b.expense, Side::Debit, usd(half)),
                (b.cash, Side::Credit, usd(half)),
            ],
            frequency: Frequency::Monthly,
            start: d(2026, 1, 1),
            end: None,
        })
        .unwrap();
    // Payroll figures, run and replayed.
    let payroll = |fed: Decimal, ss: Decimal| PayrollLine {
        employee_name: "Dana".into(),
        employee: None,
        gross_pay: Decimal::ONE,
        federal_withholding: fed,
        state_withholding: fed,
        ss_employee: Decimal::ZERO,
        medicare_employee: Decimal::ZERO,
        ss_employer: ss,
        medicare_employer: Decimal::ONE,
        other_deductions: Decimal::ZERO,
    };
    for line in [
        payroll(Decimal::MAX, Decimal::ONE),
        payroll(Decimal::ZERO, big),
    ] {
        assert!(matches!(
            b.ledger.run_payroll(d(2026, 1, 31), vec![line], b.cash),
            Err(LedgerError::ValueOutOfRange(_))
        ));
    }
    // Replayed as journaled, sub-cent and past the limit alike; a net
    // past exact arithmetic is refused where it is computed.
    let fine = payroll(Decimal::new(1, 12), Decimal::ONE);
    let huge = payroll(Decimal::ZERO, big);
    b.ledger
        .replay_payroll_run(1, d(2026, 1, 31), vec![fine, huge], 1)
        .unwrap();
    let mut over = payroll(Decimal::ZERO, Decimal::ONE);
    over.gross_pay = -Decimal::MAX;
    over.federal_withholding = Decimal::MAX;
    assert!(matches!(
        over.net_pay(),
        Err(LedgerError::TooLargeToCompute(_))
    ));
    assert!(
        ledger_core::payroll::check_payroll_ranges(&[payroll(MAX_LINE_AMOUNT, MAX_LINE_AMOUNT)])
            .is_ok()
    );
    // A statement balance in a reconciliation.
    for balance in [-Decimal::MAX, big, -big] {
        assert!(matches!(
            b.ledger
                .reconciliation_report(b.cash, d(2026, 1, 31), balance),
            Err(LedgerError::ValueOutOfRange(_))
        ));
    }
    let report = b
        .ledger
        .reconciliation_report(b.cash, d(2026, 1, 31), -MAX_LINE_AMOUNT)
        .unwrap();
    assert_eq!(report.difference, MAX_LINE_AMOUNT);
    // A payout's lines and its deposit.
    let payout = |revenue: Decimal, deposit: Decimal| ledger_core::Payout {
        deposit_account: b.cash,
        deposit,
        revenue: vec![ledger_core::PayoutLine {
            account: b.revenue,
            amount: revenue,
            class: None,
        }],
        fees: vec![],
        adjustments: vec![],
    };
    for p in [payout(big, big), payout(Decimal::ONE, -Decimal::MAX)] {
        assert!(matches!(
            ledger_core::payout_plan(&p, b.ledger.chart()),
            Err(LedgerError::ValueOutOfRange(_))
        ));
    }
    assert!(
        ledger_core::payout_plan(&payout(MAX_LINE_AMOUNT, MAX_LINE_AMOUNT), b.ledger.chart())
            .is_ok()
    );
    // A contributor's basis and a value at contribution.
    let basis = |b: Decimal, v: Decimal| {
        ledger_core::AssetBasis::contribution(ledger_core::BasisRule::Carryover723, b, v)
    };
    assert!(basis(big, Decimal::ONE).check().is_err());
    assert!(basis(Decimal::ONE, big).check().is_err());
    assert!(basis(MAX_LINE_AMOUNT, MAX_LINE_AMOUNT).check().is_ok());
    // On replay: every journaled basis passed check_journaled (a pre-P3
    // one check_legacy_journaled).
    let cents_max = Decimal::MAX.round_dp(2) / Decimal::ONE_HUNDRED;
    assert!(
        basis(big.round_dp(2), big.round_dp(2))
            .check_journaled()
            .is_ok()
    );
    assert!(
        basis(cents_max.round_dp(2), Decimal::ONE)
            .check_journaled()
            .is_ok()
    );
    assert!(
        basis(-Decimal::ONE, Decimal::ONE)
            .check_journaled()
            .is_err()
    );
    let mut legacy = basis(big.round_dp(2), Decimal::ONE);
    legacy.fmv_at_contribution = None;
    assert!(legacy.check_legacy_journaled().is_ok());
    legacy.contributor_adjusted_basis = Some(-Decimal::ONE);
    assert!(legacy.check_legacy_journaled().is_err());
    // A party's share of an entry.
    let share = |a: Decimal| {
        vec![ledger_core::PartyShare {
            party: 1,
            share: ledger_core::shares::Share::Amount(a),
        }]
    };
    let known = |_| true;
    assert!(ledger_core::shares::check_shares(&share(big.round_dp(2)), known).is_err());
    assert!(ledger_core::shares::check_shares(&share(MAX_LINE_AMOUNT), known).is_ok());
    // On replay: every journaled share passed check_shares_journaled.
    assert!(ledger_core::shares::check_shares_journaled(&share(big.round_dp(2)), known).is_ok());
    assert!(ledger_core::shares::check_shares_journaled(&share(Decimal::MAX), known).is_ok());
    assert!(ledger_core::shares::check_shares_journaled(&share(Decimal::ZERO), known).is_err());
}

/// The ledger-level pairs for a basis and shares: the command refuses past
/// the limit; replay of the old line is accepted, and what it feeds (the
/// register, the shares' resolution) refuses what it can't compute.
#[test]
fn replayed_bases_and_shares_past_the_limit_are_accepted() {
    let mut b = books();
    let mut add = |code: &str, ty| b.ledger.add_account(code, code, ty, None, false).unwrap();
    let equipment = add("1500", AccountType::Asset);
    let accum = add("1501", AccountType::Asset);
    let depreciation = add("6100", AccountType::Expense);
    // Decimal's largest is a whole number of cents, as every version
    // required of a basis and a share.
    let cents_max = Decimal::MAX;
    let basis = ledger_core::AssetBasis {
        rule: ledger_core::BasisRule::Carryover723,
        contributor_adjusted_basis: Some(cents_max),
        fmv_at_contribution: None,
        sec704c_method: None,
        source: None,
    };
    for _ in 0..2 {
        let id = b
            .ledger
            .register_asset(ledger_core::NewAsset {
                description: "Laptop".into(),
                in_service_date: d(2026, 1, 1),
                cost: usd(Decimal::new(240000, 2)),
                salvage_value: usd(Decimal::ZERO),
                useful_life_months: 24,
                cost_account: equipment,
                accum_depr_account: accum,
                expense_account: depreciation,
            })
            .unwrap();
        let mut new = basis.clone();
        new.fmv_at_contribution = Some(Decimal::new(240000, 2));
        assert!(matches!(
            b.ledger.set_asset_basis(id, new),
            Err(LedgerError::InvalidTaxBasis(_))
        ));
        b.ledger
            .replay_set_asset_tax_basis(id, basis.rule, basis.contributor_adjusted_basis)
            .unwrap();
        assert_eq!(b.ledger.get_asset(id).unwrap().basis, Some(basis.clone()));
    }
    assert!(matches!(
        ledger_core::fixed_asset_register(&b.ledger, d(2026, 12, 31)),
        Err(LedgerError::TooLargeToCompute(_))
    ));
    let a = b
        .ledger
        .add_party("A", &[ledger_core::Role::Member])
        .unwrap();
    let c = b
        .ledger
        .add_party("C", &[ledger_core::Role::Member])
        .unwrap();
    let e = b.ledger.try_create_entry(d(2026, 3, 1), "rent").unwrap();
    b.ledger
        .add_line(e, b.expense, Side::Debit, usd(Decimal::TEN))
        .unwrap();
    b.ledger
        .add_line(e, b.cash, Side::Credit, usd(Decimal::TEN))
        .unwrap();
    b.ledger.post_entry(e).unwrap();
    let shares = [a, c].map(|party| ledger_core::PartyShare {
        party,
        share: ledger_core::shares::Share::Amount(cents_max),
    });
    assert!(matches!(
        b.ledger.set_entry_parties(e, &shares),
        Err(LedgerError::InvalidParty(_))
    ));
    b.ledger.replay_set_entry_parties(e, &shares).unwrap();
    assert_eq!(b.ledger.entry_parties(e), &shares);
    let money = ledger_core::parties::money_accounts(&b.ledger, &[b.cash], &[]);
    let why = ledger_core::shares::resolve(&b.ledger, e, &shares, &money, &[]).unwrap_err();
    assert!(why.contains("too large to compute"), "{why}");
    // Replay still checks what every version checked.
    assert_eq!(
        b.ledger.replay_set_entry_parties(99, &shares).unwrap_err(),
        LedgerError::UnknownEntry(99)
    );
}

/// A replayed payment is as journaled: no bound but exact arithmetic (a
/// paid amount past it is refused, never wrapped). The command's bound is
/// the balance (Overpayment).
#[test]
fn replayed_payments_are_as_journaled() {
    let mut b = books();
    let customer = b.ledger.add_customer("Client").unwrap();
    let invoice = b
        .ledger
        .create_invoice(
            customer,
            d(2026, 1, 1),
            d(2026, 2, 1),
            vec![DocumentLine {
                description: "work".into(),
                amount: Decimal::new(10000, 2),
                class_id: None,
            }],
        )
        .unwrap();
    assert!(matches!(
        b.ledger
            .receive_payment(invoice, d(2026, 1, 15), too_big(), b.cash),
        Err(LedgerError::Overpayment(..))
    ));
    b.ledger
        .replay_invoice_payment(invoice, too_big(), 98)
        .unwrap();
    b.ledger
        .replay_invoice_payment(invoice, Decimal::MAX - too_big(), 99)
        .unwrap();
    assert_eq!(
        b.ledger.get_invoice(invoice).unwrap().amount_paid,
        Decimal::MAX
    );
    assert!(matches!(
        b.ledger.replay_invoice_payment(invoice, Decimal::ONE, 100),
        Err(LedgerError::TooLargeToCompute(_))
    ));
    // Refused before anything changed.
    assert_eq!(
        b.ledger.get_invoice(invoice).unwrap().amount_paid,
        Decimal::MAX
    );
}

/// A disposal's proceeds are a line of its entry, and its date a day of
/// the calendar, for a new disposal; a replayed one is as journaled, and
/// its amounts (disposal_amounts, pure) are computed, never a panic.
#[test]
fn disposal_proceeds_and_dates_are_bounded() {
    let mut b = books();
    let mut add = |code: &str, ty| b.ledger.add_account(code, code, ty, None, false).unwrap();
    let equipment = add("1500", AccountType::Asset);
    let accum = add("1501", AccountType::Asset);
    let depreciation = add("6100", AccountType::Expense);
    let laptop = || ledger_core::NewAsset {
        description: "Laptop".into(),
        in_service_date: d(2026, 1, 1),
        cost: usd(Decimal::new(240000, 2)),
        salvage_value: usd(Decimal::ZERO),
        useful_life_months: 24,
        cost_account: equipment,
        accum_depr_account: accum,
        expense_account: depreciation,
    };
    let id = b.ledger.register_asset(laptop()).unwrap();
    for proceeds in [Decimal::MAX, too_big()] {
        assert_eq!(
            b.ledger
                .dispose_asset(id, d(2026, 6, 30), usd(proceeds))
                .unwrap_err(),
            LedgerError::AmountOutOfRange
        );
        let asset = b.ledger.get_asset(id).unwrap().clone();
        let amounts = ledger_core::disposal_amounts(&asset, d(2026, 6, 30), usd(proceeds)).unwrap();
        assert_eq!(amounts.gain_loss, proceeds - amounts.book_value);
    }
    assert_eq!(
        b.ledger
            .dispose_asset(id, NaiveDate::MAX, usd(Decimal::ONE))
            .unwrap_err(),
        LedgerError::DateOutOfRange(NaiveDate::MAX)
    );
    // Disposal amounts at chrono's last day: computed.
    let asset = b.ledger.get_asset(id).unwrap().clone();
    let amounts = ledger_core::disposal_amounts(&asset, NaiveDate::MAX, usd(Decimal::ONE)).unwrap();
    assert_eq!(amounts.book_value, Decimal::ZERO);
    b.ledger
        .dispose_asset(id, d(2026, 6, 30), usd(MAX_LINE_AMOUNT))
        .unwrap();
    // Replayed past both bounds: accepted as journaled.
    let old = b.ledger.register_asset(laptop()).unwrap();
    b.ledger
        .replay_dispose_asset(old, NaiveDate::MAX, usd(Decimal::MAX))
        .unwrap();
    assert!(matches!(
        b.ledger.get_asset(old).unwrap().status,
        ledger_core::AssetStatus::Disposed { date, .. } if date == NaiveDate::MAX
    ));
    // Replay keeps what every version checked: no disposal twice, no
    // negative proceeds. The date floor (the acquired date, P3) is a
    // command's: replay accepts a disposal before it, which every earlier
    // version's commands refused anyway (rules.md B-28: replay accepts
    // more, never less).
    assert_eq!(
        b.ledger
            .replay_dispose_asset(old, NaiveDate::MAX, usd(Decimal::ONE))
            .unwrap_err(),
        LedgerError::AssetAlreadyDisposed(old)
    );
    let early = b.ledger.register_asset(laptop()).unwrap();
    assert!(matches!(
        b.ledger
            .replay_dispose_asset(early, d(2026, 2, 1), usd(-Decimal::ONE)),
        Err(LedgerError::NegativeProceeds(_))
    ));
    assert!(matches!(
        b.ledger
            .dispose_asset(early, d(2025, 1, 1), usd(Decimal::ONE)),
        Err(LedgerError::DisposalBeforeAcquisition { .. })
    ));
    b.ledger
        .replay_dispose_asset(early, d(2025, 1, 1), usd(Decimal::ONE))
        .unwrap();
}

/// The bounds are the line limit exactly: a new draft line (base or
/// foreign) and a recurring template's line may be as large as a line, and
/// no larger; a replayed one past it is accepted as journaled.
#[test]
fn the_bound_is_the_line_limit_itself() {
    let mut b = books();
    let e = b.ledger.try_create_entry(d(2026, 1, 1), "max").unwrap();
    for amount in [MAX_LINE_AMOUNT, -MAX_LINE_AMOUNT] {
        b.ledger
            .add_line(e, b.cash, Side::Debit, usd(amount))
            .unwrap();
    }
    for amount in [too_big().round_dp(2), -too_big().round_dp(2)] {
        assert_eq!(
            b.ledger
                .add_line(e, b.cash, Side::Debit, usd(amount))
                .unwrap_err(),
            LedgerError::AmountOutOfRange
        );
    }
    // A foreign line: its base amount, foreign x rate, is what's bounded.
    b.ledger
        .add_line_fx(
            e,
            b.cash,
            Side::Debit,
            Currency::EUR,
            MAX_LINE_AMOUNT,
            Decimal::ONE,
        )
        .unwrap();
    assert_eq!(
        b.ledger
            .add_line_fx(
                e,
                b.cash,
                Side::Debit,
                Currency::EUR,
                MAX_LINE_AMOUNT,
                Decimal::TWO
            )
            .unwrap_err(),
        LedgerError::AmountOutOfRange
    );
    // Replayed as an earlier version booked them: past the limit too.
    for amount in [too_big(), -too_big(), Decimal::MAX] {
        b.ledger
            .replay_add_line_fx(
                e,
                b.cash,
                Side::Debit,
                usd(amount),
                Currency::EUR,
                Decimal::ONE,
            )
            .unwrap();
        b.ledger
            .replay_add_line(e, b.cash, Side::Debit, usd(amount))
            .unwrap();
    }
    assert_eq!(b.ledger.get_entry(e).unwrap().lines().len(), 9);
    let recurring = |a: Decimal| NewRecurring {
        description: "max".into(),
        lines: vec![
            (b.expense, Side::Debit, usd(a)),
            (b.cash, Side::Credit, usd(a)),
        ],
        frequency: Frequency::Monthly,
        start: d(2026, 1, 1),
        end: None,
    };
    b.ledger
        .register_recurring(recurring(MAX_LINE_AMOUNT))
        .unwrap();
    assert_eq!(
        b.ledger
            .register_recurring(recurring(too_big()))
            .unwrap_err(),
        LedgerError::AmountOutOfRange
    );
    let old = b
        .ledger
        .replay_register_recurring(recurring(too_big()))
        .unwrap();
    // Its occurrences generate drafts as journaled; their post refuses.
    let draft = b
        .ledger
        .replay_generate_recurring(old, d(2026, 1, 1))
        .unwrap();
    assert_eq!(
        b.ledger.get_entry(draft).unwrap().lines()[0].amount.amount,
        too_big()
    );
    assert_eq!(
        b.ledger.post_entry(draft).unwrap_err(),
        LedgerError::AmountOutOfRange
    );
}

// ------------------------------------------------- P3: assets, schedules

/// The equipment accounts an asset needs, on these books.
fn asset_accounts(b: &mut Books) -> (AccountId, AccountId, AccountId) {
    let mut add = |code: &str, ty| b.ledger.add_account(code, code, ty, None, false).unwrap();
    (
        add("1500", AccountType::Asset),
        add("1501", AccountType::Asset),
        add("6100", AccountType::Expense),
    )
}

fn new_asset(
    accounts: (AccountId, AccountId, AccountId),
    cost: &str,
    salvage: &str,
    in_service_date: NaiveDate,
    life: u32,
) -> ledger_core::NewAsset {
    ledger_core::NewAsset {
        description: "Machine".into(),
        in_service_date,
        cost: usd(cost.parse().unwrap()),
        salvage_value: usd(salvage.parse().unwrap()),
        useful_life_months: life,
        cost_account: accounts.0,
        accum_depr_account: accounts.1,
        expense_account: accounts.2,
    }
}

/// B-3 (P3): a new asset's cost and salvage are in the currency's minor
/// unit (every depreciation and disposal line derives from them, and a
/// line finer than a cent never posts). Its replay pair: an asset an
/// earlier version took in tenths of a cent opens, and its schedule is
/// computed, never a panic: whole cents until its last day, which is
/// exactly its cost (1,200.005 over 12 months from 2026-01-01: 100.00 a
/// month, the cumulatives 100.0004 × m rounding to whole cents, and
/// December the rest, 100.005).
#[test]
fn an_asset_cost_finer_than_a_cent_is_refused() {
    let mut b = books();
    let acc = asset_accounts(&mut b);
    for (cost, salvage) in [("1200.005", "0"), ("1200.00", "0.001")] {
        let before = b.ledger.clone();
        assert!(matches!(
            b.ledger
                .register_asset(new_asset(acc, cost, salvage, d(2026, 1, 1), 12)),
            Err(LedgerError::FinerThanMinorUnit { .. })
        ));
        assert!(b.ledger.differs_from(&before).is_empty());
    }
    let id = b
        .ledger
        .replay_register_asset(new_asset(acc, "1200.005", "0", d(2026, 1, 1), 12))
        .unwrap();
    let s = ledger_core::depreciation_schedule(b.ledger.get_asset(id).unwrap(), NaiveDate::MAX)
        .unwrap();
    assert_eq!(s.len(), 12);
    for line in &s[..11] {
        assert_eq!(line.amount, Decimal::new(10000, 2), "{}", line.period);
    }
    assert_eq!(s[11].amount, "100.005".parse::<Decimal>().unwrap());
    assert_eq!(s[11].accumulated, "1200.005".parse::<Decimal>().unwrap());
}

/// B-28 (P3: lives past 9999, plan/served.md defect 22): a new asset's
/// last day of depreciation is at most 9999-12-31, the calendar's last
/// day, so every month of its life is one the books can post. A life
/// ending on that day is taken (P0 bounded the life's exclusive end, one
/// day too strict); one a day later is refused. Replay takes either.
#[test]
fn a_life_ending_on_9999_12_31_is_accepted() {
    let mut b = books();
    let acc = asset_accounts(&mut b);
    // 9999-01-01 + 12 months = 10000-01-01, exclusive: last day 9999-12-31.
    b.ledger
        .register_asset(new_asset(acc, "1200.00", "0", d(9999, 1, 1), 12))
        .unwrap();
    // 9999-12-01 + 1 month = 10000-01-01: last day 9999-12-31.
    b.ledger
        .register_asset(new_asset(acc, "1200.00", "0", d(9999, 12, 1), 1))
        .unwrap();
}

/// The pair of a_life_ending_on_9999_12_31_is_accepted.
#[test]
fn a_life_ending_after_it_is_refused() {
    let mut b = books();
    let acc = asset_accounts(&mut b);
    for (start, life) in [
        (d(9999, 1, 2), 12),
        (d(9999, 12, 2), 1),
        (d(9999, 12, 31), 1),
    ] {
        assert_eq!(
            b.ledger
                .register_asset(new_asset(acc, "1200.00", "0", start, life))
                .unwrap_err(),
            LedgerError::AssetDateOverflow,
            "{start} + {life}"
        );
        b.ledger
            .replay_register_asset(new_asset(acc, "1200.00", "0", start, life))
            .unwrap();
    }
}

/// B-28 (P3): a daily schedule's last day is in the calendar: one month
/// from 9999-12-15 covers through 10000-01-14, refused; one month from
/// 9999-12-01 ends on 9999-12-31, taken. Replay takes either.
#[test]
fn a_daily_schedule_whose_last_day_passes_9999_is_refused() {
    let mut b = books();
    let schedule = |start: NaiveDate| NewSchedule {
        description: "Insurance".into(),
        kind: ledger_core::schedules::ScheduleKind::PrepaidExpense,
        total: usd(Decimal::new(30000, 2)),
        start,
        months: 1,
        balance_account: b.prepaid,
        pl_account: b.expense,
        convention: ledger_core::ScheduleConvention::Daily,
    };
    let (late, ok) = (schedule(d(9999, 12, 15)), schedule(d(9999, 12, 1)));
    assert!(matches!(
        b.ledger.register_schedule(late.clone()),
        Err(LedgerError::InvalidSchedule(m)) if m.contains("9999-12-31")
    ));
    b.ledger.register_schedule(ok).unwrap();
    let id = b.ledger.replay_register_schedule(late).unwrap();
    // Its plan is computed: December 9999, then January 10000.
    let plan = b.ledger.get_schedule(id).unwrap().plan().unwrap();
    assert_eq!(plan.len(), 2);
    assert_eq!(plan[1].0, d(10000, 1, 31));
}
