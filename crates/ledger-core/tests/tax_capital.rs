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
use ledger_core::tax_capital::{Open, Reconciliation, tax_basis_capital};
use ledger_core::{
    AccountId, AccountType, AssetBasis, AssetId, BasisRule, Currency, EntryId, Ledger, Money,
    NewAsset, Sec704cMethod, Side,
};
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
    let id =
        b.l.register_asset(NewAsset {
            description: name.into(),
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
        assert_eq!(row.net_income.open, vec![depreciation.clone()]);
        assert_eq!(row.other.tax(), Some(d("0")));
        assert_eq!(row.distributions.tax(), Some(d("300.00")));
        assert_eq!(row.ending.tax(), None);
        assert_eq!(row.ending.known, d("1699.20"));
        assert_eq!(row.ending.open, vec![depreciation.clone()]);
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
/// book figure.
#[test]
fn cash_only_books_are_complete() {
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
    for row in &tc.rows {
        assert_eq!(row.ending.tax(), Some(row.book.ending));
        assert!(row.ending.open.is_empty());
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
            (prepaid, Side::Debit, "25.00"),
            (tobin, Side::Credit, "12.50"),
            (morgan, Side::Credit, "12.50"),
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
    assert_eq!(t.book.contributions_property, d("412.50"));
    assert_eq!(t.contributions_property.known, d("0"));
    assert_eq!(
        t.contributions_property.open,
        vec![
            Open::PropertyBasis {
                entry: contract,
                book: d("12.50")
            },
            Open::PropertyBasis {
                entry: tool,
                book: d("400.00")
            },
        ]
    );
    assert_eq!(t.reconciliation.property_without_basis, d("412.50"));
    assert_eq!(m.contributions_property.known, d("0"));
    assert_eq!(m.reconciliation.property_without_basis, d("12.50"));
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
