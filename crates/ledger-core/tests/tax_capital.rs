//! Spec tests for the K-1 worksheet's tax-basis capital
//! (`ledger_core::tax_capital`, rules.md B-36): Item L on the tax basis
//! beside the book figures, every figure the books can't know left open
//! and named, and the reconciliation from book to tax capital.
//!
//! Expected values are worked by hand from the Form 1065 instructions'
//! Item L (`docs/spec/sources/irs-partnerships.md`): capital contributed is
//! "the amount of cash plus the adjusted tax basis of all property
//! contributed"; withdrawals and distributions "the amount of cash plus
//! the adjusted tax basis of all property distributed". Names and figures
//! are invented.

use std::str::FromStr;

use chrono::NaiveDate;
use ledger_core::reports::MemberCapital;
use ledger_core::tax_capital::{Note, Open, Reconciliation, RunningSplit, tax_basis_capital};
use ledger_core::{
    AccountId, AccountType, AssetBasis, AssetId, BasisRule, Currency, EntryId, Ledger, Money,
    NewAsset, NewSchedule, ScheduleConvention, ScheduleKind, Sec704cMethod, Side,
};
use proptest::prelude::*;
use rust_decimal::Decimal;

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn date(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn usd(s: &str) -> Money {
    Money::new(d(s), Currency::USD)
}

struct Books {
    l: Ledger,
    cash: AccountId,
    prepaid: AccountId,
    equipment: AccountId,
    accum: AccountId,
    tobin: AccountId,
    morgan: AccountId,
    revenue: AccountId,
    depreciation: AccountId,
}

fn books() -> Books {
    let mut l = Ledger::new(Currency::USD);
    let mut add = |code: &str, name: &str, t| l.add_account(code, name, t, None, false).unwrap();
    let cash = add("1000", "Bank", AccountType::Asset);
    let prepaid = add("1300", "Prepaid Expenses", AccountType::Asset);
    let equipment = add("1500", "Equipment", AccountType::Asset);
    let accum = add("1510", "Accumulated Depreciation", AccountType::Asset);
    let tobin = add("3000", "Capital, Tobin Harlow", AccountType::Equity);
    let morgan = add("3001", "Capital, Morgan Ellis", AccountType::Equity);
    let revenue = add("4000", "Revenue", AccountType::Revenue);
    let depreciation = add("6200", "Depreciation", AccountType::Expense);
    Books {
        l,
        cash,
        prepaid,
        equipment,
        accum,
        tobin,
        morgan,
        revenue,
        depreciation,
    }
}

fn post(l: &mut Ledger, on: NaiveDate, lines: &[(AccountId, Side, &str)]) -> EntryId {
    let e = l.try_create_entry(on, "entry").unwrap();
    for (a, side, amt) in lines {
        l.add_line(e, *a, *side, usd(amt)).unwrap();
    }
    l.post_entry(e).unwrap();
    e
}

fn members(b: &Books) -> [MemberCapital; 2] {
    [
        MemberCapital {
            name: "Tobin Harlow".into(),
            capital_account: b.tobin,
            distribution_account: None,
            share: d("0.5"),
        },
        MemberCapital {
            name: "Morgan Ellis".into(),
            capital_account: b.morgan,
            distribution_account: None,
            share: d("0.5"),
        },
    ]
}

/// A device two members contribute, in service 2026-01-01 over 24 months.
fn device(b: &mut Books, name: &str, fmv: &str, adjusted: &str, entry: EntryId) -> AssetId {
    device_on(b, name, fmv, adjusted, entry, date(2026, 1, 1))
}

/// A device two members contribute, in service `on` over 24 months.
fn device_on(
    b: &mut Books,
    name: &str,
    fmv: &str,
    adjusted: &str,
    entry: EntryId,
    on: NaiveDate,
) -> AssetId {
    let id =
        b.l.register_asset(NewAsset {
            description: name.into(),
            in_service_date: on,
            cost: usd(fmv),
            salvage_value: usd("0.00"),
            useful_life_months: 24,
            cost_account: b.equipment,
            accum_depr_account: b.accum,
            expense_account: b.depreciation,
        })
        .unwrap();
    b.l.set_asset_acquisition(id, entry).unwrap();
    let mut basis = AssetBasis::contribution(BasisRule::PersonalUseConversion, d(adjusted), d(fmv));
    basis.sec704c_method = Some(Sec704cMethod::Traditional);
    b.l.set_asset_basis(id, basis).unwrap();
    id
}

/// Every month's run Jan..=Dec 2026, as the schedule gives it.
fn a_year_of_runs(b: &mut Books, id: AssetId) {
    let a = b.l.get_asset(id).unwrap().clone();
    for l in ledger_core::depreciation_schedule(&a, date(2026, 12, 31)).unwrap() {
        let (y, m) = (
            l.period[..4].parse().unwrap(),
            l.period[5..].parse().unwrap(),
        );
        let end = date(y, m, 1)
            .checked_add_months(chrono::Months::new(1))
            .unwrap()
            .pred_opt()
            .unwrap();
        let amt = l.amount.to_string();
        let e = post(
            &mut b.l,
            end,
            &[
                (b.depreciation, Side::Debit, &amt),
                (b.accum, Side::Credit, &amt),
            ],
        );
        b.l.check_depreciation_run(id, &l.period, e, l.amount)
            .unwrap();
        b.l.record_depreciation_run(id, &l.period, e, l.amount)
            .unwrap();
    }
}

/// The year, by hand. Tobin Harlow and Morgan Ellis, 50/50:
/// - 2026-01-01 cash in: Dr 1000 2,000.00 / Cr 3000 1,000.00, Cr 3001
///   1,000.00;
/// - 2026-01-01 two devices in, jointly, from personal use: a laptop worth
///   1,440.00 (adjusted basis 1,688.40: built-in loss 248.40) and a camera
///   worth 1,300.00 (basis 1,080.00: built-in gain 220.00); Dr 1500
///   2,740.00 / Cr 3000 1,370.00, Cr 3001 1,370.00;
/// - revenue 600.00; a year of depreciation, 24-month lives: 1,440/2 +
///   1,300/2 = 720.00 + 650.00 = 1,370.00; net income 600.00 − 1,370.00 =
///   −770.00, −385.00 each;
/// - 2026-12-15 cash out: Dr 3000 300.00, Dr 3001 300.00 / Cr 1000 600.00.
///
/// Book: 1,000.00 + 1,370.00 − 385.00 − 300.00 = 1,685.00 each.
/// Tax: property at §723 basis, (1,688.40 + 1,080.00) / 2 = 1,384.20 each
/// (book 1,370.00 less the built-in gain share (−248.40 + 220.00) / 2 =
/// −14.20); the year's income is open (tax depreciation is the CPA's), so
/// the ending the books know is 1,000.00 + 1,384.20 − 385.00 − 300.00 =
/// 1,699.20; book less tax = −14.20, the built-in gain share.
fn the_year() -> (Books, EntryId, AssetId, AssetId) {
    let mut b = books();
    let (cash, tobin, morgan, equipment) = (b.cash, b.tobin, b.morgan, b.equipment);
    post(
        &mut b.l,
        date(2026, 1, 1),
        &[
            (cash, Side::Debit, "2000.00"),
            (tobin, Side::Credit, "1000.00"),
            (morgan, Side::Credit, "1000.00"),
        ],
    );
    let devices = post(
        &mut b.l,
        date(2026, 1, 1),
        &[
            (equipment, Side::Debit, "1440.00"),
            (equipment, Side::Debit, "1300.00"),
            (tobin, Side::Credit, "1370.00"),
            (morgan, Side::Credit, "1370.00"),
        ],
    );
    let laptop = device(&mut b, "Laptop", "1440.00", "1688.40", devices);
    let camera = device(&mut b, "Camera", "1300.00", "1080.00", devices);
    let revenue = b.revenue;
    post(
        &mut b.l,
        date(2026, 2, 1),
        &[
            (cash, Side::Debit, "600.00"),
            (revenue, Side::Credit, "600.00"),
        ],
    );
    a_year_of_runs(&mut b, laptop);
    a_year_of_runs(&mut b, camera);
    post(
        &mut b.l,
        date(2026, 12, 15),
        &[
            (tobin, Side::Debit, "300.00"),
            (morgan, Side::Debit, "300.00"),
            (cash, Side::Credit, "600.00"),
        ],
    );
    (b, devices, laptop, camera)
}

/// Item L on the tax basis, per member, for the year (B-36): a cash
/// contribution, a contribution of property, a distribution, a year of
/// depreciation.
#[test]
fn tax_basis_capital_per_member() {
    let (b, _, _, _) = the_year();
    let tc = tax_basis_capital(
        &b.l,
        date(2026, 1, 1),
        date(2026, 12, 31),
        &members(&b),
        &[b.cash],
    )
    .unwrap();
    let depreciation = Open::TaxDepreciation {
        book: d("1370.00"),
        contributed_book: d("1370.00"),
    };
    assert_eq!(tc.open, vec![depreciation.clone()]);
    for row in &tc.rows {
        // The book column is the K-1 worksheet's row.
        assert_eq!(row.book.beginning, d("0"));
        assert_eq!(row.book.contributions_cash, d("1000.00"));
        assert_eq!(row.book.contributions_property, d("1370.00"));
        assert_eq!(row.book.net_income, d("-385.00"));
        assert_eq!(row.book.distributions, d("300.00"));
        assert_eq!(row.book.ending, d("1685.00"));
        // The tax column.
        assert_eq!(row.beginning.tax(), Some(d("0")));
        assert_eq!(row.contributions_cash, d("1000.00"));
        assert_eq!(row.contributions_property.tax(), Some(d("1384.20")));
        assert_eq!(row.net_income.tax(), None, "tax depreciation is the CPA's");
        assert_eq!(row.net_income.known, d("-385.00"));
        let income = Open::TaxNetIncome { book: d("-385.00") };
        assert_eq!(
            row.net_income.open,
            vec![income.clone(), depreciation.clone()]
        );
        assert_eq!(row.other.tax(), Some(d("0")));
        assert_eq!(row.distributions.tax(), Some(d("300.00")));
        assert_eq!(row.ending.tax(), None);
        assert_eq!(row.ending.known, d("1699.20"));
        assert_eq!(row.ending.open, vec![income, depreciation.clone()]);
        // Book less tax, by cause.
        assert_eq!(
            row.reconciliation,
            Reconciliation {
                beginning: d("0"),
                built_in_gain_704c: d("-14.20"),
                property_without_basis: d("0"),
                property_distributed: d("0"),
            }
        );
        assert_eq!(
            row.reconciliation.total(),
            Some(row.book.ending - row.ending.known)
        );
    }
}

/// Beginning capital the next year is this year's ending as the books
/// know it, left open while an earlier year left a figure open.
#[test]
fn next_years_beginning_carries_what_is_known_and_says_what_isnt() {
    let (b, _, _, _) = the_year();
    let tc = tax_basis_capital(
        &b.l,
        date(2027, 1, 1),
        date(2027, 12, 31),
        &members(&b),
        &[b.cash],
    )
    .unwrap();
    for row in &tc.rows {
        assert_eq!(row.book.beginning, d("1685.00"));
        assert_eq!(row.beginning.known, d("1699.20"));
        assert_eq!(row.beginning.open, vec![Open::PriorYears]);
        assert_eq!(row.reconciliation.beginning, d("-14.20"));
        assert_eq!(
            row.reconciliation.total(),
            Some(row.book.ending - row.ending.known)
        );
    }
}

/// Cash only, no fixed assets: every line is the tax figure, equal to the
/// book figure, but net income (and so the ending), which is the return's
/// whatever the books hold (Item L: "as figured for tax purposes").
#[test]
fn cash_only_books_leave_only_the_tax_income_open() {
    let mut b = books();
    let (cash, tobin, morgan, revenue) = (b.cash, b.tobin, b.morgan, b.revenue);
    post(
        &mut b.l,
        date(2026, 3, 1),
        &[
            (cash, Side::Debit, "500.00"),
            (tobin, Side::Credit, "250.00"),
            (morgan, Side::Credit, "250.00"),
        ],
    );
    post(
        &mut b.l,
        date(2026, 4, 1),
        &[
            (cash, Side::Debit, "100.01"),
            (revenue, Side::Credit, "100.01"),
        ],
    );
    let tc = tax_basis_capital(
        &b.l,
        date(2026, 1, 1),
        date(2026, 12, 31),
        &members(&b),
        &[cash],
    )
    .unwrap();
    assert!(tc.open.is_empty());
    for (row, income) in tc.rows.iter().zip(["50.01", "50.00"]) {
        let income = vec![Open::TaxNetIncome { book: d(income) }];
        assert_eq!(row.beginning.tax(), Some(d("0")));
        assert_eq!(row.contributions_cash, d("250.00"));
        assert_eq!(row.contributions_property.tax(), Some(d("0")));
        assert_eq!(row.distributions.tax(), Some(d("0")));
        assert_eq!(row.net_income.tax(), None, "the return's, never the books'");
        assert_eq!(row.net_income.open, income);
        assert_eq!(row.ending.tax(), None);
        assert_eq!(row.ending.known, row.book.ending);
        assert_eq!(row.ending.open, income);
        assert_eq!(row.reconciliation, Reconciliation::default());
    }
}

/// Property contributed with no basis in the books (a prepaid contract
/// handed over, no registered asset) is open with its book amount; a
/// linked asset carrying a purchase's basis on a contribution is open
/// too. Neither is guessed.
#[test]
fn property_without_a_basis_is_open_never_guessed() {
    let mut b = books();
    let (prepaid, equipment, tobin, morgan, cash) =
        (b.prepaid, b.equipment, b.tobin, b.morgan, b.cash);
    let contract = post(
        &mut b.l,
        date(2026, 1, 5),
        &[
            (prepaid, Side::Debit, "36.00"),
            (tobin, Side::Credit, "18.00"),
            (morgan, Side::Credit, "18.00"),
        ],
    );
    let tool = post(
        &mut b.l,
        date(2026, 1, 6),
        &[
            (equipment, Side::Debit, "400.00"),
            (tobin, Side::Credit, "400.00"),
        ],
    );
    let id =
        b.l.register_asset(NewAsset {
            description: "Drill".into(),
            in_service_date: date(2026, 1, 6),
            cost: usd("400.00"),
            salvage_value: usd("0.00"),
            useful_life_months: 24,
            cost_account: b.equipment,
            accum_depr_account: b.accum,
            expense_account: b.depreciation,
        })
        .unwrap();
    b.l.set_asset_acquisition(id, tool).unwrap();
    b.l.set_asset_basis(id, AssetBasis::purchase()).unwrap();
    let tc = tax_basis_capital(
        &b.l,
        date(2026, 1, 1),
        date(2026, 12, 31),
        &members(&b),
        &[cash],
    )
    .unwrap();
    let (t, m) = (&tc.rows[0], &tc.rows[1]);
    assert_eq!(t.book.contributions_property, d("418.00"));
    assert_eq!(t.contributions_property.known, d("0"));
    assert_eq!(
        t.contributions_property.open,
        vec![
            Open::PropertyBasis {
                entry: contract,
                book: d("18.00")
            },
            Open::PropertyBasis {
                entry: tool,
                book: d("400.00")
            },
        ]
    );
    assert_eq!(t.reconciliation.property_without_basis, d("418.00"));
    assert_eq!(m.contributions_property.known, d("0"));
    assert_eq!(m.reconciliation.property_without_basis, d("18.00"));
    for row in &tc.rows {
        assert_eq!(
            row.reconciliation.total(),
            Some(row.book.ending - row.ending.known)
        );
    }
}

/// Property distributed: its adjusted tax basis is the CPA's (it needs
/// tax depreciation), so the line is open with its book amount.
#[test]
fn property_distributed_is_open_at_its_book_amount() {
    let mut b = books();
    let (equipment, tobin, morgan, cash) = (b.equipment, b.tobin, b.morgan, b.cash);
    post(
        &mut b.l,
        date(2026, 1, 1),
        &[
            (cash, Side::Debit, "1000.00"),
            (tobin, Side::Credit, "500.00"),
            (morgan, Side::Credit, "500.00"),
        ],
    );
    post(
        &mut b.l,
        date(2026, 1, 2),
        &[
            (equipment, Side::Debit, "100.00"),
            (cash, Side::Credit, "100.00"),
        ],
    );
    let out = post(
        &mut b.l,
        date(2026, 6, 1),
        &[
            (tobin, Side::Debit, "100.00"),
            (equipment, Side::Credit, "100.00"),
        ],
    );
    let tc = tax_basis_capital(
        &b.l,
        date(2026, 1, 1),
        date(2026, 12, 31),
        &members(&b),
        &[cash],
    )
    .unwrap();
    let t = &tc.rows[0];
    assert_eq!(t.book.distributions, d("100.00"));
    assert_eq!(t.distributions.known, d("0"));
    assert_eq!(
        t.distributions.open,
        vec![Open::PropertyDistribution {
            entry: out,
            book: d("100.00")
        }]
    );
    assert_eq!(t.reconciliation.property_distributed, d("100.00"));
    assert_eq!(
        t.reconciliation.total(),
        Some(t.book.ending - t.ending.known)
    );
    assert_eq!(tc.rows[1].distributions.tax(), Some(d("0")));
}

/// tax_capital INVARIANTS 4 and 5 hold for every row of the year and of
/// every span within it.
#[test]
fn every_span_rolls_forward_and_reconciles() {
    let (b, _, _, _) = the_year();
    for (start, end) in [
        (date(2026, 1, 1), date(2026, 6, 30)),
        (date(2026, 7, 1), date(2026, 12, 31)),
        (date(2026, 12, 15), date(2026, 12, 15)),
    ] {
        let tc = tax_basis_capital(&b.l, start, end, &members(&b), &[b.cash]).unwrap();
        for row in &tc.rows {
            assert_eq!(
                row.ending.known,
                row.beginning.known
                    + row.contributions_cash
                    + row.contributions_property.known
                    + row.net_income.known
                    + row.other.known
                    - row.distributions.known,
                "{start}..{end}"
            );
            assert_eq!(
                row.reconciliation.total(),
                Some(row.book.ending - row.ending.known),
                "{start}..{end}"
            );
        }
    }
}

/// A contributed asset with `rule`, in service 2026-01-01 over 24 months.
fn contributed(
    b: &mut Books,
    rule: BasisRule,
    fmv: &str,
    adjusted: &str,
    entry: EntryId,
) -> AssetId {
    let id =
        b.l.register_asset(NewAsset {
            description: format!("Device {fmv}"),
            in_service_date: date(2026, 1, 1),
            cost: usd(fmv),
            salvage_value: usd("0.00"),
            useful_life_months: 24,
            cost_account: b.equipment,
            accum_depr_account: b.accum,
            expense_account: b.depreciation,
        })
        .unwrap();
    b.l.set_asset_acquisition(id, entry).unwrap();
    b.l.set_asset_basis(id, AssetBasis::contribution(rule, d(adjusted), d(fmv)))
        .unwrap();
    id
}

/// Four devices and a prepaid service plan two members hand over jointly
/// from personal use, in one entry (P3 review, by hand; invented figures):
/// - D1 worth 1,200.00, basis 1,500.00; D2 worth 800.00, basis 1,100.00
///   (both worth less than basis); D3 worth 1,000.00, basis 700.00 (worth
///   more); D4 worth 600.00, basis 600.00 (equal); the plan 40.00, a
///   prepaid-expense schedule's account. Dr 1500 3,600.00, Dr 1300 40.00
///   / Cr 3000 1,820.00, Cr 3001 1,820.00.
/// - Depreciable basis, the lower of value and basis: 1,200.00 + 800.00 +
///   700.00 + 600.00 = 3,300.00. §723: 1,500.00 + 1,100.00 + 700.00 +
///   600.00 = 3,900.00. Built-in: −300.00, −300.00, +300.00, 0.00 =
///   −300.00, never netted per property (D1 and D2 §704(c)(1)(C)).
/// - Item L property, each: book 1,820.00; tax (3,900.00 + 40.00) / 2 =
///   1,970.00 = 1,820.00 − (−300.00 / 2), the plan at its book 20.00 each
///   with a note, never open.
/// - D3 sold 2026-06-30 for 1,100.00: its tax gain is the CPA's, flagged
///   with its built-in gain 300.00 (§704(c)(1)(A)).
#[test]
fn four_devices_and_a_plan_by_hand() {
    let mut b = books();
    let (equipment, prepaid, tobin, morgan, cash) =
        (b.equipment, b.prepaid, b.tobin, b.morgan, b.cash);
    let plan_expense =
        b.l.add_account("6300", "Service plans", AccountType::Expense, None, false)
            .unwrap();
    b.l.register_schedule(NewSchedule {
        description: "Service plan".into(),
        kind: ScheduleKind::PrepaidExpense,
        total: usd("40.00"),
        start: date(2026, 1, 1),
        months: 12,
        balance_account: prepaid,
        pl_account: plan_expense,
        convention: ScheduleConvention::Daily,
    })
    .unwrap();
    let e = post(
        &mut b.l,
        date(2026, 1, 1),
        &[
            (equipment, Side::Debit, "1200.00"),
            (equipment, Side::Debit, "800.00"),
            (equipment, Side::Debit, "1000.00"),
            (equipment, Side::Debit, "600.00"),
            (prepaid, Side::Debit, "40.00"),
            (tobin, Side::Credit, "1820.00"),
            (morgan, Side::Credit, "1820.00"),
        ],
    );
    let p = BasisRule::PersonalUseConversion;
    let ids = [
        contributed(&mut b, p, "1200.00", "1500.00", e),
        contributed(&mut b, p, "800.00", "1100.00", e),
        contributed(&mut b, p, "1000.00", "700.00", e),
        contributed(&mut b, p, "600.00", "600.00", e),
    ];
    let a = |i: usize| b.l.get_asset(ids[i]).unwrap();
    let want = [
        ("1200.00", "1500.00", "-300.00", true),
        ("800.00", "1100.00", "-300.00", true),
        ("700.00", "700.00", "300.00", false),
        ("600.00", "600.00", "0.00", false),
    ];
    for (i, (dep, t723, big, loss)) in want.into_iter().enumerate() {
        assert_eq!(a(i).depreciable_basis(), Some(d(dep)), "D{}", i + 1);
        assert_eq!(a(i).tax_basis_723(), Some(d(t723)), "D{}", i + 1);
        assert_eq!(a(i).built_in_gain_704c(), Some(d(big)), "D{}", i + 1);
        assert_eq!(a(i).sec704c_1c(), loss, "D{}", i + 1);
    }
    b.l.dispose_asset(ids[2], date(2026, 6, 30), usd("1100.00"))
        .unwrap();
    let tc = tax_basis_capital(
        &b.l,
        date(2026, 1, 1),
        date(2026, 12, 31),
        &members(&b),
        &[cash],
    )
    .unwrap();
    assert!(tc.open.contains(&Open::Disposal {
        asset: ids[2],
        proceeds: d("1100.00"),
        built_in_gain_704c: Some(d("300.00")),
    }));
    for row in &tc.rows {
        assert_eq!(row.book.contributions_property, d("1820.00"));
        assert_eq!(row.contributions_property.tax(), Some(d("1970.00")));
        assert_eq!(
            row.contributions_property.notes,
            vec![Note::PrepaidAtBook {
                entry: e,
                book: d("20.00")
            }]
        );
        assert_eq!(row.reconciliation.built_in_gain_704c, d("-150.00"));
        assert_eq!(row.reconciliation.property_without_basis, d("0"));
        assert_eq!(
            row.reconciliation.total(),
            Some(row.book.ending - row.ending.known)
        );
    }
}

/// Property contributed subject to a liability (Item L: "reduced by any
/// liabilities ... to which the property is subject"), by hand: worth
/// 500.00, basis 200.00, a 300.00 loan on it; Dr 1500 500.00 / Cr 2200
/// 300.00, Cr 3000 200.00. Book property 200.00 (value net of the loan);
/// tax 200.00 − 300.00 = −100.00 ("This amount might be negative"); the
/// other member nothing.
#[test]
fn property_subject_to_a_liability_enters_net_of_it() {
    let mut b = books();
    let (equipment, tobin, cash) = (b.equipment, b.tobin, b.cash);
    let loan =
        b.l.add_account("2200", "Loan", AccountType::Liability, None, false)
            .unwrap();
    let e = post(
        &mut b.l,
        date(2026, 1, 1),
        &[
            (equipment, Side::Debit, "500.00"),
            (loan, Side::Credit, "300.00"),
            (tobin, Side::Credit, "200.00"),
        ],
    );
    contributed(&mut b, BasisRule::Carryover723, "500.00", "200.00", e);
    let tc = tax_basis_capital(
        &b.l,
        date(2026, 1, 1),
        date(2026, 12, 31),
        &members(&b),
        &[cash],
    )
    .unwrap();
    let (t, m) = (&tc.rows[0], &tc.rows[1]);
    assert_eq!(t.book.contributions_property, d("200.00"));
    assert_eq!(t.contributions_property.tax(), Some(d("-100.00")));
    assert_eq!(t.reconciliation.built_in_gain_704c, d("300.00"));
    assert_eq!(m.contributions_property.tax(), Some(d("0")));
    assert_eq!(m.reconciliation, Reconciliation::default());
}

/// Three members with unequal shares, one contributing nothing, by hand:
/// a 1,000.00 machine (basis 400.00, business use) handed over 600.00 by
/// the first and 400.00 by the second; its 600.00 built-in gain is shared
/// as its value is, 360.00 and 240.00, so tax property 240.00 and 160.00
/// (the basis, 0.6 and 0.4); the third member has every line 0.00.
#[test]
fn unequal_contributors_and_a_member_with_none() {
    let mut b = books();
    let (equipment, tobin, morgan, cash) = (b.equipment, b.tobin, b.morgan, b.cash);
    let third =
        b.l.add_account(
            "3002",
            "Capital, Avery Quinn",
            AccountType::Equity,
            None,
            false,
        )
        .unwrap();
    let e = post(
        &mut b.l,
        date(2026, 1, 1),
        &[
            (equipment, Side::Debit, "1000.00"),
            (tobin, Side::Credit, "600.00"),
            (morgan, Side::Credit, "400.00"),
        ],
    );
    contributed(&mut b, BasisRule::Carryover723, "1000.00", "400.00", e);
    let [t, m] = members(&b);
    let ms = [
        MemberCapital {
            share: d("0.5"),
            ..t
        },
        MemberCapital {
            share: d("0.3"),
            ..m
        },
        MemberCapital {
            name: "Avery Quinn".into(),
            capital_account: third,
            distribution_account: None,
            share: d("0.2"),
        },
    ];
    let tc = tax_basis_capital(&b.l, date(2026, 1, 1), date(2026, 12, 31), &ms, &[cash]).unwrap();
    let want = [
        ("600.00", "240.00", "360.00"),
        ("400.00", "160.00", "240.00"),
    ];
    for (row, (book, tax, big)) in tc.rows.iter().zip(want) {
        assert_eq!(row.book.contributions_property, d(book));
        assert_eq!(row.contributions_property.tax(), Some(d(tax)));
        assert_eq!(row.reconciliation.built_in_gain_704c, d(big));
    }
    let none = &tc.rows[2];
    assert_eq!(none.book.ending, d("0"));
    assert_eq!(none.contributions_property.tax(), Some(d("0")));
    assert_eq!(none.ending.known, d("0"));
    assert_eq!(none.reconciliation, Reconciliation::default());
}

/// The odd cent on the tax basis runs (B-36 with D-38; owner, 2026-10-22):
/// three devices contributed 50/50, each worth less than its basis by an
/// odd number of cents, the built-in gain shared cumulatively, so the
/// members' tax property ends one cent apart, never three. By hand:
/// - 2026-01-01 a monitor, 400.00 (basis 412.37, gain −12.37): exact
///   −6.185 each, the tied cent to Tobin: −6.19 / −6.18;
/// - 2026-06-01 a chair, 250.00 (basis 263.41, gain −13.41): running
///   −25.78, exact −12.89 each: −6.70 / −6.71;
/// - 2026-11-01 a desk, 600.00 (basis 655.91, gain −55.91): running
///   −81.69, exact −40.845 each, the tied cent to Tobin: −27.96 / −27.95.
///
/// Running built-in gain −40.85 / −40.84; book property 625.00 each, tax
/// 665.85 / 665.84. Shared entry by entry, Tobin took all three tied
/// cents: 665.86 / 665.83. From June: book 425.00 each; gain −34.66 each
/// (−6.70 − 27.96, −6.71 − 27.95), tax 459.66 each, on a beginning of
/// 206.19 / 206.18, ending at the year's figures.
#[test]
fn tied_odd_cents_of_property_basis_run_cumulatively() {
    let mut b = books();
    let (equipment, tobin, morgan, cash) = (b.equipment, b.tobin, b.morgan, b.cash);
    for (on, name, fmv, adjusted, half) in [
        (date(2026, 1, 1), "Monitor", "400.00", "412.37", "200.00"),
        (date(2026, 6, 1), "Chair", "250.00", "263.41", "125.00"),
        (date(2026, 11, 1), "Desk", "600.00", "655.91", "300.00"),
    ] {
        let e = post(
            &mut b.l,
            on,
            &[
                (equipment, Side::Debit, fmv),
                (tobin, Side::Credit, half),
                (morgan, Side::Credit, half),
            ],
        );
        device_on(&mut b, name, fmv, adjusted, e, on);
    }
    let ms = members(&b);
    let year = tax_basis_capital(&b.l, date(2026, 1, 1), date(2026, 12, 31), &ms, &[cash]).unwrap();
    let from_june =
        tax_basis_capital(&b.l, date(2026, 6, 1), date(2026, 12, 31), &ms, &[cash]).unwrap();
    for (i, (tax, big, beginning)) in [
        ("665.85", "-40.85", "206.19"),
        ("665.84", "-40.84", "206.18"),
    ]
    .into_iter()
    .enumerate()
    {
        let row = &year.rows[i];
        assert_eq!(row.book.contributions_property, d("625.00"));
        assert_eq!(row.contributions_property.tax(), Some(d(tax)));
        assert_eq!(row.reconciliation.built_in_gain_704c, d(big));
        assert_eq!(row.ending.known, d(tax));
        let june = &from_june.rows[i];
        assert_eq!(june.beginning.known, d(beginning));
        assert_eq!(june.book.contributions_property, d("425.00"));
        assert_eq!(june.contributions_property.tax(), Some(d("459.66")));
        assert_eq!(june.reconciliation.built_in_gain_704c, d("-34.66"));
        assert_eq!(june.ending.known, row.ending.known, "the spans chain");
    }
}

/// A third member, Avery Quinn, beside Tobin and Morgan (made up), with
/// shares 0.5 / 0.3 / 0.2.
fn three_members(b: &mut Books) -> (AccountId, [MemberCapital; 3]) {
    let third =
        b.l.add_account(
            "3002",
            "Capital, Avery Quinn",
            AccountType::Equity,
            None,
            false,
        )
        .unwrap();
    let [t, m] = members(b);
    (
        third,
        [
            MemberCapital {
                share: d("0.5"),
                ..t
            },
            MemberCapital {
                share: d("0.3"),
                ..m
            },
            MemberCapital {
                name: "Avery Quinn".into(),
                capital_account: third,
                distribution_account: None,
                share: d("0.2"),
            },
        ],
    )
}

/// A member never takes a cent of an entry they had no part in (B-36,
/// Form 1065 Item L: capital contributed is the basis of the property
/// that member contributed; P4 review, 2026-10-22). Three members; by
/// hand, each set of contributing members its own running split:
/// - 2026-01-01 Tobin and Morgan, 50.00 each of a 100.00 device (basis
///   100.01, gain −0.01): −0.01 / 0.00, the tied cent to Tobin;
/// - 2026-02-01 Morgan and Avery, 25.00 / 75.00 of a 100.00 device
///   (basis 100.02, gain −0.02): exact −0.005 / −0.015, the tied cent to
///   Morgan: −0.01 / −0.01;
/// - 2027-01-01 Morgan and Avery, 60.00 / 30.00 of a 90.00 device (basis
///   90.01, gain −0.01): Morgan and Avery's count runs −0.03, exact
///   −0.011667 / −0.018333, rounded −0.01 / −0.02: 0.00 / −0.01.
///
/// 2027: Tobin nothing at all; Morgan 60.00 at tax, Avery 30.01, together
/// the device's §723 basis 90.01. One running split for all three
/// members instead (the first cut) gives Tobin, who put in nothing in
/// 2027, −0.01 of tax property and +0.01 of built-in gain, Morgan and
/// Avery 60.01 / 30.01, more than the basis.
#[test]
fn a_member_outside_an_entry_never_takes_its_cents() {
    let mut b = books();
    let (equipment, tobin, morgan, cash) = (b.equipment, b.tobin, b.morgan, b.cash);
    let (avery, ms) = three_members(&mut b);
    for (on, value, basis, credits) in [
        (
            date(2026, 1, 1),
            "100.00",
            "100.01",
            [(tobin, "50.00"), (morgan, "50.00")],
        ),
        (
            date(2026, 2, 1),
            "100.00",
            "100.02",
            [(morgan, "25.00"), (avery, "75.00")],
        ),
        (
            date(2027, 1, 1),
            "90.00",
            "90.01",
            [(morgan, "60.00"), (avery, "30.00")],
        ),
    ] {
        let e = post(
            &mut b.l,
            on,
            &[
                (equipment, Side::Debit, value),
                (credits[0].0, Side::Credit, credits[0].1),
                (credits[1].0, Side::Credit, credits[1].1),
            ],
        );
        device_on(&mut b, "Device", value, basis, e, on);
    }
    let year = |y| tax_basis_capital(&b.l, date(y, 1, 1), date(y, 12, 31), &ms, &[cash]).unwrap();
    let (y26, y27) = (year(2026), year(2027));
    // Each year's built-in shares sum to its entries' gains, and the tax
    // property to their bases.
    for (tc, gain, basis) in [(&y26, "-0.03", "200.03"), (&y27, "-0.01", "90.01")] {
        let big: Decimal = tc
            .rows
            .iter()
            .map(|r| r.reconciliation.built_in_gain_704c)
            .sum();
        assert_eq!(big, d(gain));
        let tax: Decimal = tc
            .rows
            .iter()
            .map(|r| r.contributions_property.tax().unwrap())
            .sum();
        assert_eq!(tax, d(basis));
    }
    for (row, (book, tax, big)) in y26.rows.iter().zip([
        ("50.00", "50.01", "-0.01"),
        ("75.00", "75.01", "-0.01"),
        ("75.00", "75.01", "-0.01"),
    ]) {
        assert_eq!(
            row.book.contributions_property,
            d(book),
            "{}",
            row.book.name
        );
        assert_eq!(row.contributions_property.tax(), Some(d(tax)));
        assert_eq!(row.reconciliation.built_in_gain_704c, d(big));
    }
    for (row, (book, tax, big)) in y27.rows.iter().zip([
        ("0", "0", "0"),
        ("60.00", "60.00", "0"),
        ("30.00", "30.01", "-0.01"),
    ]) {
        assert_eq!(
            row.book.contributions_property,
            d(book),
            "{}",
            row.book.name
        );
        assert_eq!(row.contributions_property.tax(), Some(d(tax)));
        assert_eq!(row.reconciliation.built_in_gain_704c, d(big));
    }
    let t = &y27.rows[0].contributions_property;
    assert!(t.open.is_empty() && t.notes.is_empty(), "{t:?}");
}

/// An entry's odd cent is the same whatever span the worksheet is asked
/// for (B-36; P4 review, 2026-10-22). Tobin and Morgan, 50/50: a monitor
/// contributed 2026-01-01 (400.00, basis 412.37, gain −12.37), reversed
/// 2026-07-01; a desk contributed 2026-11-01 (600.00, basis 655.91, gain
/// −55.91). The reversal takes the monitor's split back out of the count,
/// so the desk is split as if the monitor never was: exact −27.955 each,
/// the tied cent to Tobin, −27.96 / −27.95, in every span. The first cut
/// carried the count span by span: from June the monitor was split in the
/// span before and its reversal read as a distribution, so the desk's
/// cent went to Morgan there and to Tobin in the others.
#[test]
fn an_entrys_odd_cent_is_the_same_in_every_span() {
    let mut b = books();
    let (equipment, tobin, morgan, cash) = (b.equipment, b.tobin, b.morgan, b.cash);
    let mut entries = Vec::new();
    for (on, name, fmv, adjusted, half) in [
        (date(2026, 1, 1), "Monitor", "400.00", "412.37", "200.00"),
        (date(2026, 11, 1), "Desk", "600.00", "655.91", "300.00"),
    ] {
        let e = post(
            &mut b.l,
            on,
            &[
                (equipment, Side::Debit, fmv),
                (tobin, Side::Credit, half),
                (morgan, Side::Credit, half),
            ],
        );
        device_on(&mut b, name, fmv, adjusted, e, on);
        entries.push(e);
    }
    b.l.reverse_entry(entries[0], date(2026, 7, 1), "monitor back")
        .unwrap();
    let ms = members(&b);
    for start in [
        date(2026, 1, 1),
        date(2026, 6, 1),
        date(2026, 8, 1),
        date(2026, 10, 1),
    ] {
        let tc = tax_basis_capital(&b.l, start, date(2026, 12, 31), &ms, &[cash]).unwrap();
        let big: Vec<Decimal> = tc
            .rows
            .iter()
            .map(|r| r.reconciliation.built_in_gain_704c)
            .collect();
        assert_eq!(big, vec![d("-27.96"), d("-27.95")], "from {start}");
    }
}

/// The running split takes the entries in date order, whatever order
/// they were posted in, so next year's beginning is this year's ending
/// to the cent (B-36). Tobin and Morgan, 50/50: a chair dated 2027-03-01
/// (250.00, basis 263.41) posted first, then a monitor dated 2026-01-01
/// (400.00, basis 412.37). The monitor comes first: −6.19 / −6.18, then
/// the chair −6.70 / −6.71. Taken in posting order the 2027 worksheet
/// would split the chair first and the monitor −6.18 / −6.19 in its
/// beginning, a cent from the 2026 worksheet's ending.
#[test]
fn the_split_runs_in_date_order() {
    let mut b = books();
    let (equipment, tobin, morgan, cash) = (b.equipment, b.tobin, b.morgan, b.cash);
    for (on, name, fmv, adjusted, half) in [
        (date(2027, 3, 1), "Chair", "250.00", "263.41", "125.00"),
        (date(2026, 1, 1), "Monitor", "400.00", "412.37", "200.00"),
    ] {
        let e = post(
            &mut b.l,
            on,
            &[
                (equipment, Side::Debit, fmv),
                (tobin, Side::Credit, half),
                (morgan, Side::Credit, half),
            ],
        );
        device_on(&mut b, name, fmv, adjusted, e, on);
    }
    let ms = members(&b);
    let year = |y| tax_basis_capital(&b.l, date(y, 1, 1), date(y, 12, 31), &ms, &[cash]).unwrap();
    let (y26, y27) = (year(2026), year(2027));
    for (i, (big26, big27)) in [("-6.19", "-6.70"), ("-6.18", "-6.71")]
        .into_iter()
        .enumerate()
    {
        assert_eq!(y26.rows[i].reconciliation.built_in_gain_704c, d(big26));
        assert_eq!(y27.rows[i].reconciliation.built_in_gain_704c, d(big27));
        assert_eq!(y27.rows[i].beginning.known, y26.rows[i].ending.known);
    }
}

/// lcm(1..=16): every sum of up to four weights of 0..=4 divides it. It
/// is under 10^8, so two exact running totals that differ at all differ
/// by more than RunningSplit's ten places can blur (INVARIANT 4).
const LCM: i128 = 720_720;

/// The largest-remainder rounding to the cent of exact running totals
/// `exact` (in cents × LCM) summing to `total` cents, ties to the earlier
/// member, a negative total (or a zero one after a negative whole,
/// `negative`) rounded as its magnitude and negated: RunningSplit
/// INVARIANT 2 in integers.
fn largest_remainder(exact: &[i128], total: i128, negative: bool) -> Vec<i128> {
    let sign = if total < 0 || (total == 0 && negative) {
        -1
    } else {
        1
    };
    let floors: Vec<i128> = exact.iter().map(|x| (sign * x).div_euclid(LCM)).collect();
    let rems: Vec<i128> = exact.iter().map(|x| (sign * x).rem_euclid(LCM)).collect();
    let short = sign * total - floors.iter().sum::<i128>();
    assert!((0..=exact.len() as i128).contains(&short));
    let rank = |i: usize| {
        (0..rems.len())
            .filter(|&j| rems[j] > rems[i] || (rems[j] == rems[i] && j < i))
            .count() as i128
    };
    (0..exact.len())
        .map(|i| sign * (floors[i] + i128::from(rank(i) < short)))
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(5000))]

    /// Law: RunningSplit INVARIANTS 1 and 2 (B-36 with D-38): for any run
    /// of splits, wholes of either sign to the cent (a third of them a few
    /// cents, so the running whole often comes back to zero) and each its own
    /// weights among up to four members (small ratios, scaled by a cent
    /// amount up to ten million, so the quotients are a cent amount's),
    /// each split's parts sum to its whole, the running totals sum to
    /// everything split, every member's running total is the
    /// largest-remainder rounding of the exact running totals, ties to the
    /// earlier member (so never a cent from exact), checked in exact
    /// integers, and the run with every whole negated gives every part
    /// negated.
    #[test]
    fn a_running_split_never_drifts_a_cent(
        n in 1usize..=4,
        splits in prop::collection::vec(
            (
                prop_oneof![-3i64..=3, -100_000i64..=100_000, -1_000_000_000_000i64..=1_000_000_000_000],
                prop::collection::vec(0u8..=4, 4),
                prop_oneof![Just(1i64), 1i64..=1_000_000_000],
            ),
            1..24,
        ),
    ) {
        let mut r = RunningSplit::new(n);
        let mut mirror = RunningSplit::new(n);
        let mut exact = vec![0i128; n];
        let mut total = 0i128;
        for (whole, weights, scale) in &splits {
            let w = &weights[..n];
            let sum: i128 = w.iter().map(|&x| i128::from(x)).sum();
            let ws: Vec<Decimal> = w
                .iter()
                .map(|&x| Decimal::new(i64::from(x) * scale, 2))
                .collect();
            let parts = r.split(Decimal::new(*whole, 2), &ws).unwrap();
            let negated = mirror.split(Decimal::new(-*whole, 2), &ws).unwrap();
            prop_assert_eq!(parts.len(), n);
            prop_assert_eq!(
                negated,
                parts.iter().map(|p| -p).collect::<Vec<_>>(),
                "a loss mirrors a gain"
            );
            let split: Decimal = parts.iter().sum();
            if sum == 0 || *whole == 0 {
                prop_assert_eq!(split, Decimal::ZERO);
                continue;
            }
            prop_assert_eq!(split, Decimal::new(*whole, 2));
            total += i128::from(*whole);
            for (x, &wi) in exact.iter_mut().zip(w) {
                *x += i128::from(*whole) * i128::from(wi) * (LCM / sum);
            }
            let running: Decimal = r.running().iter().sum();
            prop_assert_eq!(running, Decimal::from_i128_with_scale(total, 2));
            let want = largest_remainder(&exact, total, *whole < 0);
            for (i, (&running, &exact)) in r.running().iter().zip(&exact).enumerate() {
                let cents = (running * Decimal::ONE_HUNDRED).normalize();
                prop_assert_eq!(cents.scale(), 0, "a running total to the cent");
                let cents = i128::try_from(cents).unwrap();
                prop_assert_eq!(cents, want[i], "member {}: the largest-remainder rounding", i);
                prop_assert!(
                    (cents * LCM - exact).abs() < LCM,
                    "member {} at {} against exact {}/{}",
                    i, cents, exact, LCM
                );
            }
        }
    }
}
