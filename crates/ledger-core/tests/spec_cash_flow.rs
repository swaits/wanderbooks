//! The statement of cash flows (rules.md B-14), from the rule and its
//! sources, never from the implementation.
//!
//! Every expected figure is worked by hand in the test's doc comment from
//! the paragraph it cites (testing.md rank 1). The random-book properties
//! at the end check the statement against an independent model: the plain
//! sum of the generated cash lines.

use chrono::NaiveDate;
use ledger_core::reports::foot;
use ledger_core::{
    AccountId, AccountType, CashFlowLine, CashFlowParams, CashFlowSection, CashFlowStatement,
    CashFlowWarningKind, Currency, EntryId, Ledger, LedgerError, MemberCapital, Money, NewAsset,
    OperatingClass, ReconcilingKind, Side, cash_flow,
};
use rust_decimal::Decimal;

const DR: Side = Side::Debit;
const CR: Side = Side::Credit;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

/// An exact amount from text: "2340.00".
fn x(s: &str) -> Decimal {
    s.parse().unwrap()
}

/// A small company to post to by account code.
struct Books {
    l: Ledger,
    members: Vec<MemberCapital>,
}

impl Books {
    fn new() -> Self {
        Books {
            l: Ledger::new(Currency::USD),
            members: Vec::new(),
        }
    }

    fn acct(&mut self, code: &str, name: &str, t: AccountType) -> AccountId {
        self.l.add_account(code, name, t, None, false).unwrap()
    }

    fn id(&self, code: &str) -> AccountId {
        self.l.chart().find_by_code(code).unwrap().id
    }

    fn post(&mut self, date: NaiveDate, desc: &str, lines: &[(&str, Side, &str)]) -> EntryId {
        let e = self.l.create_entry(date, desc);
        for &(code, side, amount) in lines {
            let a = self.id(code);
            self.l
                .add_line(e, a, side, Money::new(x(amount), Currency::USD))
                .unwrap();
        }
        self.l.post_entry(e).unwrap();
        e
    }

    fn reverse(&mut self, e: EntryId, date: NaiveDate) -> EntryId {
        let desc = format!("Reverse entry {e}");
        self.l.reverse_entry(e, date, &desc).unwrap()
    }

    /// A registered asset on these accounts, so they are investing.
    fn asset(&mut self, cost: &str, accum: &str, expense: &str, amount: &str) {
        let (c, a, e) = (self.id(cost), self.id(accum), self.id(expense));
        self.l
            .register_asset(NewAsset {
                description: format!("Asset on {cost}"),
                in_service_date: d(2026, 1, 1),
                cost: Money::new(x(amount), Currency::USD),
                salvage_value: Money::new(Decimal::ZERO, Currency::USD),
                useful_life_months: 36,
                cost_account: c,
                accum_depr_account: a,
                expense_account: e,
            })
            .unwrap();
    }

    fn member(&mut self, name: &str, capital: &str) {
        let capital_account = self.id(capital);
        self.members.push(MemberCapital {
            name: name.to_string(),
            capital_account,
            distribution_account: None,
            share: Decimal::ZERO,
        });
    }

    fn params(
        &self,
        start: NaiveDate,
        end: NaiveDate,
        cash: &[&str],
        financing: &[&str],
        investing: &[&str],
    ) -> CashFlowParams {
        let ids = |codes: &[&str]| codes.iter().map(|c| self.id(c)).collect();
        CashFlowParams {
            start,
            end,
            cash_accounts: ids(cash),
            financing_accounts: ids(financing),
            investing_accounts: ids(investing),
            members: self.members.clone(),
        }
    }

    fn cf(
        &self,
        start: NaiveDate,
        end: NaiveDate,
        cash: &[&str],
        financing: &[&str],
        investing: &[&str],
    ) -> CashFlowStatement {
        cash_flow(
            &self.l,
            &self.params(start, end, cash, financing, investing),
        )
        .unwrap()
    }
}

/// (entry, amount) of some lines.
fn em(lines: &[CashFlowLine]) -> Vec<(EntryId, Decimal)> {
    lines.iter().map(|l| (l.entry, l.amount)).collect()
}

/// (kind, amount) of a section's kinds.
fn kinds(cf: &CashFlowStatement, s: CashFlowSection) -> Vec<(String, Decimal)> {
    cf.kinds(s)
        .into_iter()
        .map(|k| (k.kind, k.amount))
        .collect()
}

/// (entry, amount) of the disclosures.
fn disclosed(cf: &CashFlowStatement) -> Vec<(EntryId, Decimal)> {
    cf.noncash_disclosures
        .iter()
        .map(|n| (n.entry, n.amount))
        .collect()
}

/// (label, amount) of the reconciling items.
fn recon(cf: &CashFlowStatement) -> Vec<(String, Decimal)> {
    cf.operating_reconciliation
        .lines
        .iter()
        .map(|l| (l.label.clone(), l.amount))
        .collect()
}

fn s(t: &str) -> String {
    t.to_string()
}

/// The laws every statement keeps (rules.md B-14 rules 8 and 9): each
/// section is its lines, and its kinds; the sections are the change in
/// cash; net income and the reconciling items are operating.
fn foots(cf: &CashFlowStatement) {
    for sec in CashFlowSection::ALL {
        let lines: Decimal = cf.lines(sec).iter().map(|l| l.amount).sum();
        assert_eq!(lines, cf.total(sec), "{sec:?} lines");
        let kinds: Decimal = cf.kinds(sec).iter().map(|k| k.amount).sum();
        assert_eq!(kinds, cf.total(sec), "{sec:?} kinds");
    }
    let sum = cf.cash_from_operations + cf.cash_from_investing + cf.cash_from_financing;
    assert_eq!(sum, cf.net_change_in_cash);
    assert_eq!(cf.cash_ending - cf.cash_beginning, cf.net_change_in_cash);
    assert_eq!(cf.reconciles.sections_sum, sum);
    assert_eq!(cf.reconciles.net_change, cf.net_change_in_cash);
    assert_eq!(cf.reconciles.difference, Decimal::ZERO);
    let r = &cf.operating_reconciliation;
    let rec: Decimal = r.net_income + r.lines.iter().map(|l| l.amount).sum::<Decimal>();
    assert_eq!(rec, cf.cash_from_operations);
    assert_eq!(r.net_cash_from_operating, cf.cash_from_operations);
}

/// No label anywhere says "distribution" (D1-c): used where nothing was
/// distributed.
fn no_distribution(cf: &CashFlowStatement) {
    let text = format!("{cf:?}").to_lowercase();
    assert!(!text.contains("distribution"), "{cf:#?}");
}

// ---------------------------------------------------------------------------
// The named regressions (defect 1's reproductions), also bookspec examples
// (bookspec/examples/cash-flow.*.toml).
// ---------------------------------------------------------------------------

/// The minimal reproduction of defect 1. Property contributed (Dr
/// Equipment 2,340 / Cr Capital), the asset registered, the entry reversed
/// the same day. No cash moved, and the entry and its reversal cancel
/// (rules.md B-14 rules 1, 6): every section 0.00, no lines, no
/// disclosures, footing 0 / 0 / 0. Before the fix: investing −2,340.00
/// against a change in cash of 0.00 (the reversal's credit to the
/// equipment account belonged to no section).
#[test]
fn defect1_reversed_contributions_foot_regression() {
    let mut b = Books::new();
    b.acct("1000", "Cash", AccountType::Asset);
    b.acct("1500", "Equipment", AccountType::Asset);
    b.acct("1510", "Accumulated depreciation", AccountType::Asset);
    b.acct("6100", "Depreciation", AccountType::Expense);
    b.acct("3000", "Capital", AccountType::Equity);
    let e1 = b.post(
        d(2026, 1, 10),
        "Property contributed",
        &[("1500", DR, "2340.00"), ("3000", CR, "2340.00")],
    );
    b.asset("1500", "1510", "6100", "2340.00");
    b.reverse(e1, d(2026, 1, 10));
    let cf = b.cf(d(2026, 1, 1), d(2026, 12, 31), &["1000"], &[], &[]);
    foots(&cf);
    for sec in CashFlowSection::ALL {
        assert_eq!(cf.total(sec), Decimal::ZERO);
        assert!(cf.lines(sec).is_empty());
    }
    assert!(cf.noncash_disclosures.is_empty());
    assert_eq!(
        (
            cf.reconciles.sections_sum,
            cf.reconciles.net_change,
            cf.reconciles.difference
        ),
        (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO)
    );
    assert!(cf.operating_reconciliation.lines.is_empty());
    no_distribution(&cf);
}

/// A two-member LLC's first weeks, in the shape of the books defect 1 was
/// found in: the same 23 entries in the same order, on the same accounts
/// and roles, every amount and date this replica's own (members "Member
/// A" and "Member B", vendors generic). Cash is 1000; 2100 (due to
/// members) is financing; 1500/1510/6500 carry registered assets. Also
/// the bookspec example `cash-flow.two-member-llc-first-year`.
///
/// | #  | date  | entry                                   | Dr        | Cr              |
/// |----|-------|-----------------------------------------|-----------|-----------------|
/// | 1  | 03-02 | formation fee, member-paid              | 6000 85.00 | 2100           |
/// | 2  | 03-03 | domain, member-paid                     | 6050 12.97 | 2100           |
/// | 3  | 03-05 | developer fee, member-paid              | 6200 99.00 | 2100           |
/// | 4  | 03-09 | Member A contributes cash               | 1000 1,200 | 3000           |
/// | 5  | 03-10 | Member B contributes cash               | 1000 800  | 3001            |
/// | 6  | 03-11 | subscription top-up, member-paid        | 6300 7.49 | 2100            |
/// | 7  | 03-16 | developer membership, member-paid       | 6400 129.00 | 2100          |
/// | 8  | 03-23 | reimburse members (#1, #2, #3, #6, #7)  | 2100 333.46 | 1000          |
/// | 9  | 03-20 | 2-year domain, member-paid (expensed)   | 6050 46.38 | 2100           |
/// | 10 | 03-25 | API credits on the card                 | 6300 14.20 | 2050           |
/// | 11 | 03-26 | reimburse members (#20)                 | 2100 46.38 | 1000           |
/// | 12 | 03-27 | laptop contributed by Member A          | 1500 1,480 | 3000           |
/// | 13 | 03-27 | reverses #12                            |           |                 |
/// | 14 | 03-27 | laptop contributed jointly              | 1500 1,480 | 3000 740, 3001 740 |
/// | 15 | 03-27 | laptop contributed jointly              | 1500 1,190 | 3000 595, 3001 595 |
/// | 16 | 03-27 | desktop contributed jointly             | 1500 2,210 | 3000/3001 1,105 |
/// | 17 | 03-27 | tablet contributed jointly              | 1500 630  | 3000/3001 315   |
/// | 18 | 03-27 | reverses #15                            |           |                 |
/// | 19 | 03-27 | laptop 1,151 and its service plan 39    | 1500 1,151, 1300 39 | 3000/3001 595 |
/// | 20 | 03-20 | 2-year domain, member-paid, prepaid     | 1300 46.38 | 2100           |
/// | 21 | 03-12 | subscription, member-paid               | 6300 216.40 | 2100          |
/// | 22 | 03-20 | reverses #9                             |           |                 |
/// | 23 | 03-30 | API credits on the card                 | 6300 42.75 | 2050           |
///
/// By hand, the year to 2026-03-31 (rules.md B-14; D-42 for what the
/// members paid):
/// - Operating: the members paid #1 85.00, #2 12.97, #3 99.00, #6 7.49,
///   #21 216.40, #7 129.00 and #20 46.38 (the prepaid domain) for the
///   company through 2100, each borrowed and spent on its date (rule 11):
///   85.00 + 12.97 + 99.00 + 7.49 + 216.40 + 129.00 + 46.38 = 596.24, so
///   −596.24, all class (d). #9 and its reversal #22 net out (rule 6).
///   The card's #10 and #23 moved no cash and 2050 is not financing: no
///   line. Nothing else moved cash through an operating account.
/// - Investing: every asset was contributed as property the members
///   owned (no cash, ASC 230-10-50-4): 0.00; #12/#13 and #15/#18 net out.
/// - Financing: #4 +1,200.00 and #5 +800.00 (contributions, 45-14(a));
///   borrowed from the members through 2100 +596.24 (the seven above,
///   45-14(b)); #8 −333.46 (85.00 + 12.97 + 99.00 + 7.49 + 129.00) and
///   #11 −46.38 (repayments, 45-15(b)), −379.84 together: 1,200.00 +
///   800.00 + 596.24 − 379.84 = +2,216.40.
/// - Sum: −596.24 + 0 + 2,216.40 = +1,620.16, the change in cash (in
///   1,200.00 + 800.00, out 333.46 + 46.38: 0.00 to 1,620.16);
///   difference 0.00.
/// - Reconciliation (45-28 to 45-30): net income is minus the expenses,
///   85.00 + 12.97 + 99.00 + 7.49 + 129.00 + 46.38 − 46.38 + 14.20 +
///   216.40 + 42.75 = 606.81, so −606.81; other noncash +39.00 (#19's
///   service plan, contributed with the laptop: noncash); increase in
///   1300, 39.00 + 46.38, so −85.38; increase in 2050, 14.20 + 42.75, so
///   +56.95. Total −606.81 + 39.00 − 85.38 + 56.95 = −596.24, the
///   operating section.
/// - Disclosed (50-3, 50-4), by date: #14 1,480.00, #16 2,210.00, #17
///   630.00, #19 1,190.00 (5,510.00, the property contributed); #12/#13
///   and #15/#18 drop out.
///
/// Before the fix the statement didn't foot: the reversals' credits to
/// equipment (#13, #18) belonged to no section, and the contributed
/// devices read as purchases.
fn two_member_llc() -> Books {
    let mut b = Books::new();
    for (code, name, t) in [
        ("1000", "Checking", AccountType::Asset),
        ("1300", "Prepaid Expenses", AccountType::Asset),
        ("1500", "Equipment", AccountType::Asset),
        ("1510", "Accumulated Depreciation", AccountType::Asset),
        ("2050", "Business Card", AccountType::Liability),
        ("2100", "Due to Members", AccountType::Liability),
        ("3000", "Capital — Member A", AccountType::Equity),
        ("3001", "Capital — Member B", AccountType::Equity),
        ("6000", "Organizational Costs", AccountType::Expense),
        ("6050", "Domain & Hosting", AccountType::Expense),
        ("6200", "Developer Fees", AccountType::Expense),
        ("6300", "Software & Subscriptions", AccountType::Expense),
        ("6400", "Memberships & Licenses", AccountType::Expense),
        ("6500", "Depreciation Expense", AccountType::Expense),
    ] {
        b.acct(code, name, t);
    }
    b.member("Member A", "3000");
    b.member("Member B", "3001");
    let p = |b: &mut Books, m, day, desc: &str, lines: &[(&str, Side, &str)]| {
        b.post(d(2026, m, day), desc, lines)
    };
    let paid = |exp: &'static str, amount: &'static str| [(exp, DR, amount), ("2100", CR, amount)];
    p(&mut b, 3, 2, "Formation filing fee", &paid("6000", "85.00"));
    p(&mut b, 3, 3, "Domain, 1 year", &paid("6050", "12.97"));
    p(
        &mut b,
        3,
        5,
        "Developer registration",
        &paid("6200", "99.00"),
    );
    p(
        &mut b,
        3,
        9,
        "Contribution — Member A",
        &[("1000", DR, "1200.00"), ("3000", CR, "1200.00")],
    );
    p(
        &mut b,
        3,
        10,
        "Contribution — Member B",
        &[("1000", DR, "800.00"), ("3001", CR, "800.00")],
    );
    p(&mut b, 3, 11, "Subscription top-up", &paid("6300", "7.49"));
    p(
        &mut b,
        3,
        16,
        "Developer membership",
        &paid("6400", "129.00"),
    );
    p(
        &mut b,
        3,
        23,
        "Reimburse members",
        &[("2100", DR, "333.46"), ("1000", CR, "333.46")],
    );
    let e9 = p(&mut b, 3, 20, "Domain, 2 years", &paid("6050", "46.38"));
    p(
        &mut b,
        3,
        25,
        "API credits",
        &[("6300", DR, "14.20"), ("2050", CR, "14.20")],
    );
    p(
        &mut b,
        3,
        26,
        "Reimburse members",
        &[("2100", DR, "46.38"), ("1000", CR, "46.38")],
    );
    let e12 = p(
        &mut b,
        3,
        27,
        "Laptop contributed — Member A",
        &[("1500", DR, "1480.00"), ("3000", CR, "1480.00")],
    );
    assert_eq!(b.reverse(e12, d(2026, 3, 27)), 13);
    let joint = |amount: &'static str, half: &'static str| {
        [("1500", DR, amount), ("3000", CR, half), ("3001", CR, half)]
    };
    p(
        &mut b,
        3,
        27,
        "Laptop contributed jointly",
        &joint("1480.00", "740.00"),
    );
    let e15 = p(
        &mut b,
        3,
        27,
        "Laptop contributed jointly",
        &joint("1190.00", "595.00"),
    );
    p(
        &mut b,
        3,
        27,
        "Desktop contributed jointly",
        &joint("2210.00", "1105.00"),
    );
    p(
        &mut b,
        3,
        27,
        "Tablet contributed jointly",
        &joint("630.00", "315.00"),
    );
    assert_eq!(b.reverse(e15, d(2026, 3, 27)), 18);
    p(
        &mut b,
        3,
        27,
        "Laptop and its service plan contributed jointly",
        &[
            ("1500", DR, "1151.00"),
            ("1300", DR, "39.00"),
            ("3000", CR, "595.00"),
            ("3001", CR, "595.00"),
        ],
    );
    p(
        &mut b,
        3,
        20,
        "Domain, 2 years, prepaid",
        &paid("1300", "46.38"),
    );
    p(&mut b, 3, 12, "Subscription", &paid("6300", "216.40"));
    assert_eq!(b.reverse(e9, d(2026, 3, 20)), 22);
    let e23 = p(
        &mut b,
        3,
        30,
        "API credits",
        &[("6300", DR, "42.75"), ("2050", CR, "42.75")],
    );
    assert_eq!(e23, 23);
    for amount in ["1480.00", "1151.00", "2210.00", "630.00"] {
        b.asset("1500", "1510", "6500", amount);
    }
    b
}

#[test]
fn defect1_two_member_llc_first_year_regression() {
    let b = two_member_llc();
    let cf = b.cf(d(2026, 1, 1), d(2026, 3, 31), &["1000"], &["2100"], &[]);
    foots(&cf);
    no_distribution(&cf);
    // What the members paid, by date: borrowed and spent on its own date.
    let paid = [
        (1, x("85.00")),
        (2, x("12.97")),
        (3, x("99.00")),
        (6, x("7.49")),
        (21, x("216.40")),
        (7, x("129.00")),
        (20, x("46.38")),
    ];
    let neg = |v: &[(EntryId, Decimal)]| -> Vec<(EntryId, Decimal)> {
        v.iter().map(|&(e, a)| (e, -a)).collect()
    };
    assert_eq!(em(&cf.operating), neg(&paid));
    assert!(cf.operating.iter().all(|l| l.on_behalf == l.amount));
    assert_eq!(
        cf.operating_by_class(),
        [(OperatingClass::ToSuppliersAndEmployees, x("-596.24"))]
    );
    assert_eq!(cf.cash_from_operations, x("-596.24"));
    assert!(cf.investing.is_empty());
    assert_eq!(cf.cash_from_investing, Decimal::ZERO);
    let mut financing = vec![(4, x("1200.00")), (5, x("800.00"))];
    financing.extend(paid);
    financing.extend([(8, x("-333.46")), (11, x("-46.38"))]);
    assert_eq!(em(&cf.financing), financing);
    assert_eq!(
        kinds(&cf, CashFlowSection::Financing),
        [
            (s("Contributions from members — Member A"), x("1200.00")),
            (s("Contributions from members — Member B"), x("800.00")),
            (
                s("Proceeds from borrowing — 2100 Due to Members"),
                x("596.24")
            ),
            (
                s("Repayments of borrowing — 2100 Due to Members"),
                x("-379.84")
            ),
        ]
    );
    let on_behalf: Decimal = cf.financing.iter().map(|l| l.on_behalf).sum();
    assert_eq!(on_behalf, x("596.24"));
    assert_eq!(cf.cash_from_financing, x("2216.40"));
    assert_eq!(
        (cf.cash_beginning, cf.cash_ending, cf.net_change_in_cash),
        (x("0.00"), x("1620.16"), x("1620.16"))
    );
    assert_eq!(
        (
            cf.reconciles.sections_sum,
            cf.reconciles.net_change,
            cf.reconciles.difference
        ),
        (x("1620.16"), x("1620.16"), x("0.00"))
    );
    let r = &cf.operating_reconciliation;
    assert_eq!(r.net_income, x("-606.81"));
    assert_eq!(
        recon(&cf),
        [
            (s("Other noncash items"), x("39.00")),
            (s("Increase in 1300 Prepaid Expenses"), x("-85.38")),
            (s("Increase in 2050 Business Card"), x("56.95")),
        ]
    );
    assert_eq!(r.lines[0].kind, ReconcilingKind::OtherNoncash);
    assert_eq!(r.lines[0].entries, [19]);
    assert_eq!(r.net_cash_from_operating, x("-596.24"));
    assert_eq!(
        disclosed(&cf),
        [
            (14, x("1480.00")),
            (16, x("2210.00")),
            (17, x("630.00")),
            (19, x("1190.00")),
        ]
    );
    let total: Decimal = cf.noncash_disclosures.iter().map(|n| n.amount).sum();
    assert_eq!(total, x("5510.00"));
    // #19: 1,190 moved into the company without cash; 1,151 of it was an
    // exchange inside investing and financing (the laptop for capital).
    let e19 = &cf.noncash_disclosures[3];
    assert_eq!(
        (e19.operating, e19.investing, e19.financing, e19.exchanged),
        (x("39.00"), x("1151.00"), x("-1190.00"), x("1151.00"))
    );
    assert!(cf.warnings.is_empty());
}

/// The same books as of 2026-03-18. By hand: entries #1 to #7 and #21 are
/// dated by then; #9, #20 and #22 are dated 03-20, after it, and the rest
/// later. The members paid #1 85.00, #2 12.97, #3 99.00, #6 7.49, #21
/// 216.40 and #7 129.00 for the company (D-42), 549.86 in all: operating
/// −549.86, class (d); financing #4 +1,200.00, #5 +800.00 and +549.86
/// borrowed from the members, +2,549.86; the change in cash 2,000.00 (#4,
/// #5); net income −549.86 with no reconciling item (no operating balance
/// moved yet); nothing disclosed.
#[test]
fn defect1_two_member_llc_as_of_march_18_regression() {
    let b = two_member_llc();
    let cf = b.cf(d(2026, 1, 1), d(2026, 3, 18), &["1000"], &["2100"], &[]);
    foots(&cf);
    let paid = [
        (1, x("85.00")),
        (2, x("12.97")),
        (3, x("99.00")),
        (6, x("7.49")),
        (21, x("216.40")),
        (7, x("129.00")),
    ];
    let mut financing = vec![(4, x("1200.00")), (5, x("800.00"))];
    financing.extend(paid);
    assert_eq!(em(&cf.financing), financing);
    assert_eq!(
        em(&cf.operating),
        paid.iter().map(|&(e, a)| (e, -a)).collect::<Vec<_>>()
    );
    assert_eq!(cf.cash_from_operations, x("-549.86"));
    assert_eq!(cf.cash_from_financing, x("2549.86"));
    assert_eq!(cf.net_change_in_cash, x("2000.00"));
    assert_eq!(cf.operating_reconciliation.net_income, x("-549.86"));
    assert!(cf.operating_reconciliation.lines.is_empty());
    assert!(cf.noncash_disclosures.is_empty());
}

// ---------------------------------------------------------------------------
// The rule, case by case.
// ---------------------------------------------------------------------------

/// A company with every kind of account the rule names. Registered fixed
/// assets on 1500/1510 and 1520/1530 (so they are investing); interest
/// accounts carry their tax categories.
fn co() -> Books {
    let mut b = Books::new();
    for (code, name, t) in [
        ("1000", "Cash", AccountType::Asset),
        ("1010", "Savings", AccountType::Asset),
        ("1200", "Accounts Receivable", AccountType::Asset),
        ("1210", "Due from buyer", AccountType::Asset),
        ("1250", "Note receivable", AccountType::Asset),
        ("1300", "Prepaid", AccountType::Asset),
        ("1400", "Loan to member", AccountType::Asset),
        ("1500", "Equipment", AccountType::Asset),
        ("1510", "Accumulated depreciation", AccountType::Asset),
        ("1520", "Vehicles", AccountType::Asset),
        (
            "1530",
            "Accumulated depreciation, vehicles",
            AccountType::Asset,
        ),
        ("2000", "Accounts Payable", AccountType::Liability),
        ("2050", "Card", AccountType::Liability),
        ("2100", "Due to members", AccountType::Liability),
        ("2300", "Deferred revenue", AccountType::Liability),
        ("2500", "Bank loan", AccountType::Liability),
        ("3000", "Capital — Member A", AccountType::Equity),
        ("3001", "Capital — Member B", AccountType::Equity),
        ("4000", "Revenue", AccountType::Revenue),
        ("4100", "Interest income", AccountType::Revenue),
        ("4900", "Gain on disposal", AccountType::Revenue),
        ("6000", "Supplies", AccountType::Expense),
        ("6100", "Depreciation", AccountType::Expense),
        ("6200", "Interest expense", AccountType::Expense),
        ("6900", "Loss on disposal", AccountType::Expense),
    ] {
        b.acct(code, name, t);
    }
    let (ii, ie) = (b.id("4100"), b.id("6200"));
    b.l.set_account_tax(ii, Some("interest_income"), None)
        .unwrap();
    b.l.set_account_tax(ie, Some("interest_expense"), None)
        .unwrap();
    b.asset("1500", "1510", "6100", "1000.00");
    b.asset("1520", "1530", "6100", "2000.00");
    b.member("Member A", "3000");
    b.member("Member B", "3001");
    b
}

/// Opening cash: Member A contributes 10,000 on Jan 2 (before every
/// period the tests below look at).
fn funded() -> Books {
    let mut b = co();
    b.post(
        d(2026, 1, 2),
        "Opening contribution",
        &[("1000", DR, "10000.00"), ("3000", CR, "10000.00")],
    );
    b
}

const FEB: (NaiveDate, NaiveDate) = (
    NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
    NaiveDate::from_ymd_opt(2026, 2, 28).unwrap(),
);

fn feb(b: &Books, financing: &[&str], investing: &[&str]) -> CashFlowStatement {
    let cf = b.cf(FEB.0, FEB.1, &["1000", "1010"], financing, investing);
    foots(&cf);
    cf
}

/// ASC 230-10-45-12(c): "Receipts from sales of property, plant, and
/// equipment" are investing, so the whole proceeds are; 45-28(b): the
/// gain or loss is removed from net income in the reconciliation.
/// (Plan-named test.) Bought for 1,000 cash, 800 depreciated, then:
/// - sold for 500 (Dr cash 500, Dr accum 800 / Cr cost 1,000, Cr gain
///   300): investing +500, operating 0, net income +300, G −300;
/// - in other books, sold for 100 at a 100 loss: investing +100, G +100;
/// - in other books, sold for 600 before any depreciation (no accumulated
///   depreciation line, defect 1's other shape): investing +600, G +400.
#[test]
fn disposal_proceeds_all_investing() {
    let sale = |proceeds: &str, accum: &str, gain_or_loss: (&str, Side, &str)| {
        let mut b = funded();
        b.post(
            d(2026, 1, 10),
            "Buy equipment",
            &[("1500", DR, "1000.00"), ("1000", CR, "1000.00")],
        );
        if accum != "0" {
            b.post(
                d(2026, 1, 31),
                "Depreciation",
                &[("6100", DR, accum), ("1510", CR, accum)],
            );
        }
        let mut lines = vec![("1000", DR, proceeds), ("1500", CR, "1000.00")];
        if accum != "0" {
            lines.push(("1510", DR, accum));
        }
        lines.push(gain_or_loss);
        b.post(d(2026, 2, 15), "Sell equipment", &lines);
        feb(&b, &[], &[])
    };
    let gain = sale("500.00", "800.00", ("4900", CR, "300.00"));
    assert_eq!(
        kinds(&gain, CashFlowSection::Investing),
        [(
            s("Proceeds from sales of property and equipment"),
            x("500.00")
        )]
    );
    assert!(gain.operating.is_empty());
    assert_eq!(gain.operating_reconciliation.net_income, x("300.00"));
    assert_eq!(
        recon(&gain),
        [(
            s("(Gain) loss and other noncash changes in investing assets"),
            x("-300.00")
        )]
    );
    assert!(gain.noncash_disclosures.is_empty());

    let loss = sale("100.00", "800.00", ("6900", DR, "100.00"));
    assert_eq!(loss.cash_from_investing, x("100.00"));
    assert_eq!(loss.cash_from_operations, Decimal::ZERO);
    assert_eq!(recon(&loss)[0].1, x("100.00"));

    let early = sale("600.00", "0", ("6900", DR, "400.00"));
    assert_eq!(early.cash_from_investing, x("600.00"));
    assert_eq!(early.cash_from_operations, Decimal::ZERO);
    assert_eq!(early.operating_reconciliation.net_income, x("-400.00"));
    assert_eq!(recon(&early)[0].1, x("400.00"));
}

/// A loss on disposal is a loss whichever account type holds it (the
/// defect found on main: the old code counted only revenue credits and
/// expense debits, so a loss debited to a revenue-typed "Gain (loss) on
/// disposal" account tripped a debug assertion and came out wrong in
/// release). Bought for 1,000 cash, 800 depreciated, sold for 100: a loss
/// of 100 (Dr cash 100, Dr accum 800, Dr loss 100 / Cr cost 1,000).
/// - Booked to 4900, a revenue account (a debit to it): investing +100
///   (ASC 230-10-45-12(c), the whole proceeds), operating 0, net income
///   −100, and the reconciliation adds the loss back (45-28(b)): +100.
/// - Booked to 6900, an expense account: the same figures.
#[test]
fn a_loss_on_disposal_on_a_revenue_or_expense_account_is_investing() {
    for loss_account in ["4900", "6900"] {
        let mut b = funded();
        b.post(
            d(2026, 1, 10),
            "Buy equipment",
            &[("1500", DR, "1000.00"), ("1000", CR, "1000.00")],
        );
        b.post(
            d(2026, 1, 31),
            "Depreciation",
            &[("6100", DR, "800.00"), ("1510", CR, "800.00")],
        );
        let sale = b.post(
            d(2026, 2, 15),
            "Sell equipment at a loss",
            &[
                ("1000", DR, "100.00"),
                ("1510", DR, "800.00"),
                (loss_account, DR, "100.00"),
                ("1500", CR, "1000.00"),
            ],
        );
        let cf = feb(&b, &[], &[]);
        assert_eq!(em(&cf.investing), [(sale, x("100.00"))], "{loss_account}");
        assert_eq!(
            cf.investing[0].kind,
            "Proceeds from sales of property and equipment"
        );
        assert!(cf.operating.is_empty(), "{loss_account}");
        assert_eq!(cf.operating_reconciliation.net_income, x("-100.00"));
        assert_eq!(
            recon(&cf),
            [(
                s("(Gain) loss and other noncash changes in investing assets"),
                x("100.00")
            )],
            "{loss_account}"
        );
        assert!(cf.noncash_disclosures.is_empty());
    }
}

/// Equipment sold for an IOU (Dr receivable 500, Dr accum 800 / Cr cost
/// 1,000, Cr gain 300). No cash moved, so no line; the noncash investing
/// part is disclosed (ASC 230-10-50-3): 500. Its later collection is
/// operating by the partition, though 45-12(c) makes it investing: D-35
/// flags it, with the fix (review item 4: it is not "(a) from customers";
/// on a receivable that isn't the customers' control account it is (c)
/// other). With the fix (the receivable on a note listed as investing),
/// the sale is disclosed as an exchange of 500 and the collection is
/// investing.
#[test]
fn a_sale_on_account_is_a_noncash_disclosure() {
    let sell = |b: &mut Books, on: &'static str| {
        b.post(
            d(2026, 1, 31),
            "Depreciation",
            &[("6100", DR, "800.00"), ("1510", CR, "800.00")],
        );
        b.post(
            d(2026, 2, 10),
            "Sell equipment on account",
            &[
                (on, DR, "500.00"),
                ("1510", DR, "800.00"),
                ("1500", CR, "1000.00"),
                ("4900", CR, "300.00"),
            ],
        );
        b.post(
            d(2026, 2, 20),
            "Buyer pays",
            &[("1000", DR, "500.00"), (on, CR, "500.00")],
        )
    };
    let mut b = funded();
    let collected = sell(&mut b, "1210");
    let cf = feb(&b, &[], &[]);
    assert!(cf.investing.is_empty());
    assert_eq!(disclosed(&cf), [(collected - 1, x("500.00"))]);
    let n = &cf.noncash_disclosures[0];
    assert_eq!((n.operating, n.investing), (x("500.00"), x("-500.00")));
    assert_eq!(em(&cf.operating), [(collected, x("500.00"))]);
    assert_eq!(cf.operating[0].class, Some(OperatingClass::OtherReceipts));
    assert_eq!(cf.warnings.len(), 1);
    assert_eq!(
        cf.warnings[0].kind,
        CashFlowWarningKind::AssetSoldOnOperatingReceivable
    );
    assert!(cf.warnings[0].message.contains("books.investing_accounts"));

    let mut fixed = funded();
    let collected = sell(&mut fixed, "1250");
    let cf = feb(&fixed, &[], &["1250"]);
    assert!(cf.warnings.is_empty());
    assert_eq!(disclosed(&cf), [(collected - 1, x("500.00"))]);
    assert_eq!(cf.noncash_disclosures[0].exchanged, x("500.00"));
    assert!(cf.operating.is_empty());
    assert_eq!(
        kinds(&cf, CashFlowSection::Investing),
        [(s("Collections on 1250 Note receivable"), x("500.00"))]
    );
}

/// A member's contribution of property (Dr Equipment 1,340 / Cr
/// Capital): no cash, so no line in any section (230-10-45-1: the
/// statement reports cash effects), disclosed in full: 1,340 (50-3, 50-4).
#[test]
fn a_property_contribution_is_a_noncash_disclosure() {
    let mut b = funded();
    let e = b.post(
        d(2026, 2, 3),
        "Laptop contributed",
        &[("1500", DR, "1340.00"), ("3000", CR, "1340.00")],
    );
    let cf = feb(&b, &[], &[]);
    for sec in CashFlowSection::ALL {
        assert!(cf.lines(sec).is_empty());
    }
    assert_eq!(disclosed(&cf), [(e, x("1340.00"))]);
    let n = &cf.noncash_disclosures[0];
    assert_eq!(
        (n.operating, n.investing, n.financing, n.exchanged),
        (x("0"), x("1340.00"), x("-1340.00"), x("1340.00"))
    );
    assert_eq!(n.label, format!("Laptop contributed (#{e})"));
}

/// D-42, rules.md B-14 rule 11 (ASC 230 is silent on a payment a third
/// party makes on the entity's behalf; PwC FSP 6.9.3.1, DART Roadmap 7.2
/// and BDO Blueprint 3.4 present it by substance, as constructive receipt
/// and disbursement). An expense a member pays personally (Dr Supplies
/// 100 / Cr 2100, the members' financing account) is cash the company
/// borrowed from the member (45-14(b)) and spent (45-17(b)), both on the
/// entry's date: financing +100 under "Proceeds from borrowing — 2100",
/// operating −100, class (d); each line's `on_behalf` is all of it. No
/// disclosure, no reconciling item but net income (−100 = operating).
/// Replaces `an_expense_a_member_paid_is_a_noncash_financing_disclosure`
/// (D-35's first point, revised by D-42). With 2100 not financing it is an
/// operating payable: the expense and the payable net, nothing moves.
#[test]
fn an_expense_a_member_paid_is_borrowed_and_spent() {
    let mut b = funded();
    let e = b.post(
        d(2026, 2, 5),
        "Software, paid by Member A",
        &[("6000", DR, "100.00"), ("2100", CR, "100.00")],
    );
    let cf = feb(&b, &["2100"], &[]);
    assert_eq!(em(&cf.operating), [(e, x("-100.00"))]);
    assert_eq!(
        cf.operating[0].class,
        Some(OperatingClass::ToSuppliersAndEmployees)
    );
    assert_eq!(cf.operating[0].on_behalf, x("-100.00"));
    assert_eq!(cf.operating[0].accounts, [s("6000")]);
    assert_eq!(em(&cf.financing), [(e, x("100.00"))]);
    assert_eq!(
        cf.financing[0].kind,
        "Proceeds from borrowing — 2100 Due to members"
    );
    assert_eq!(cf.financing[0].on_behalf, x("100.00"));
    assert_eq!(cf.financing[0].date, d(2026, 2, 5));
    assert!(cf.investing.is_empty());
    assert_eq!(cf.net_change_in_cash, Decimal::ZERO);
    assert!(cf.noncash_disclosures.is_empty());
    assert_eq!(cf.operating_reconciliation.net_income, x("-100.00"));
    assert!(cf.operating_reconciliation.lines.is_empty());

    let cf = feb(&b, &[], &[]);
    assert!(cf.operating.is_empty() && cf.financing.is_empty());
    assert!(cf.noncash_disclosures.is_empty());
    assert_eq!(
        recon(&cf),
        [(s("Increase in 2100 Due to members"), x("100.00"))]
    );
}

/// D-42, ASC 230-10-45-15(b): reimbursing the member (Dr 2100 / Cr cash
/// 100) is a repayment of borrowing, −100, the only line of its own; the
/// expense the member paid was borrowed and spent on its own date (above).
/// February: operating −100 (#e), financing +100 (#e) −100 (#r), the
/// change in cash −100. With 2100 not financing the reimbursement is an
/// operating payment to a supplier, class (d), and the expense an accrual.
#[test]
fn reimbursing_a_member_is_a_repayment_of_borrowing() {
    let mut b = funded();
    let e = b.post(
        d(2026, 2, 5),
        "Software, paid by Member A",
        &[("6000", DR, "100.00"), ("2100", CR, "100.00")],
    );
    let r = b.post(
        d(2026, 2, 25),
        "Reimburse Member A",
        &[("2100", DR, "100.00"), ("1000", CR, "100.00")],
    );
    let cf = feb(&b, &["2100"], &[]);
    assert_eq!(em(&cf.financing), [(e, x("100.00")), (r, x("-100.00"))]);
    assert_eq!(
        kinds(&cf, CashFlowSection::Financing),
        [
            (
                s("Proceeds from borrowing — 2100 Due to members"),
                x("100.00")
            ),
            (
                s("Repayments of borrowing — 2100 Due to members"),
                x("-100.00")
            ),
        ]
    );
    assert_eq!(cf.financing[1].on_behalf, Decimal::ZERO);
    assert_eq!(em(&cf.operating), [(e, x("-100.00"))]);
    assert_eq!(cf.net_change_in_cash, x("-100.00"));
    let cf = feb(&b, &[], &[]);
    assert_eq!(em(&cf.operating), [(r, x("-100.00"))]);
    assert_eq!(
        cf.operating[0].class,
        Some(OperatingClass::ToSuppliersAndEmployees)
    );
    assert_eq!(cf.operating[0].on_behalf, Decimal::ZERO);
}

/// D-42 for a lender (DART Roadmap 7.2's third-party financing; BDO
/// Blueprint 3.4, Example 3-5: a bank remits the price of equipment to
/// the supplier): Dr Vehicles 2,000 / Cr Bank loan 2,000, the loan
/// financing, no cash. Investing −2,000 ("Purchases of property and
/// equipment", 45-13(c)), financing +2,000 ("Proceeds from borrowing —
/// 2500", 45-14(b)), both the entry's date; nothing disclosed (nothing was
/// exchanged once the constructive cash is counted); no warning. Its first
/// repayment of 500 is financing −500 (45-15(b)).
#[test]
fn equipment_a_lender_paid_for_is_borrowed_and_invested() {
    let mut b = funded();
    let e = b.post(
        d(2026, 2, 6),
        "Van, paid by the bank",
        &[("1520", DR, "2000.00"), ("2500", CR, "2000.00")],
    );
    let p = b.post(
        d(2026, 2, 27),
        "Loan payment",
        &[("2500", DR, "500.00"), ("1000", CR, "500.00")],
    );
    let cf = feb(&b, &["2500"], &[]);
    assert!(cf.operating.is_empty());
    assert_eq!(em(&cf.investing), [(e, x("-2000.00"))]);
    assert_eq!(cf.investing[0].kind, "Purchases of property and equipment");
    assert_eq!(cf.investing[0].on_behalf, x("-2000.00"));
    assert_eq!(em(&cf.financing), [(e, x("2000.00")), (p, x("-500.00"))]);
    assert_eq!(
        kinds(&cf, CashFlowSection::Financing),
        [
            (s("Proceeds from borrowing — 2500 Bank loan"), x("2000.00")),
            (s("Repayments of borrowing — 2500 Bank loan"), x("-500.00")),
        ]
    );
    assert!(cf.noncash_disclosures.is_empty());
    assert!(cf.warnings.is_empty());
    assert_eq!(cf.net_change_in_cash, x("-500.00"));
}

/// The expense-to-capital case (rules.md B-14 rule 11): a member pays a
/// vendor for the company and the books credit the member's capital (Dr
/// Supplies 80 / Cr Capital — Member A), no cash. The member paid cash to
/// a third party on the company's behalf, as through 2100; only the form
/// of what the company owes back differs (nothing: capital). BDO Blueprint
/// 3.4 names this case (a parent paying a subsidiary's expenses) among
/// those for constructive cash flows, and the books' own K-1 worksheet
/// already counts such an entry as a cash contribution (k1.rs INVARIANT
/// 3). So: financing +80 "Contributions from members — Member A"
/// (45-14(a)), operating −80, class (d); nothing disclosed. A guaranteed
/// payment credited to capital is the member's own charge (no one was
/// paid): it stays noncash, reconciled through 3000 and disclosed.
#[test]
fn an_expense_a_member_paid_into_capital_is_contributed_and_spent() {
    let mut b = funded();
    let gp = b.acct("6600", "Guaranteed payments", AccountType::Expense);
    b.l.set_account_tax(gp, Some("guaranteed_payments"), None)
        .unwrap();
    let e = b.post(
        d(2026, 2, 7),
        "Supplies, paid by Member A, contributed",
        &[("6000", DR, "80.00"), ("3000", CR, "80.00")],
    );
    let g = b.post(
        d(2026, 2, 28),
        "Guaranteed payment to Member A, credited to capital",
        &[("6600", DR, "300.00"), ("3000", CR, "300.00")],
    );
    let cf = feb(&b, &[], &[]);
    assert_eq!(em(&cf.operating), [(e, x("-80.00"))]);
    assert_eq!(
        cf.operating[0].class,
        Some(OperatingClass::ToSuppliersAndEmployees)
    );
    assert_eq!(em(&cf.financing), [(e, x("80.00"))]);
    assert_eq!(
        cf.financing[0].kind,
        "Contributions from members — Member A"
    );
    assert_eq!(cf.financing[0].on_behalf, x("80.00"));
    assert_eq!(disclosed(&cf), [(g, x("300.00"))]);
    assert_eq!(
        recon(&cf),
        [(s("Noncash: through 3000 Capital — Member A"), x("300.00"))]
    );
    assert_eq!(cf.operating_reconciliation.net_income, x("-380.00"));
}

/// D-42 keeps property a member already owned noncash (ASC 230-10-50-4:
/// no cash moved, to anyone): a laptop and its service plan contributed
/// (Dr Equipment 1,000, Dr Prepaid 60 / Cr Capital 1,060). Only revenue
/// and expense lines can a member pay for through capital (rule 11), so
/// nothing is constructive: no line; disclosed 1,060 (the laptop 1,000
/// exchanged for capital); the prepaid's 60 reconciled as other noncash.
#[test]
fn property_a_member_contributes_stays_noncash() {
    let mut b = funded();
    let e = b.post(
        d(2026, 2, 3),
        "Laptop and its service plan contributed",
        &[
            ("1500", DR, "1000.00"),
            ("1300", DR, "60.00"),
            ("3000", CR, "1060.00"),
        ],
    );
    let cf = feb(&b, &["2100"], &[]);
    for sec in CashFlowSection::ALL {
        assert!(cf.lines(sec).is_empty(), "{sec:?}");
    }
    assert_eq!(disclosed(&cf), [(e, x("1060.00"))]);
    let n = &cf.noncash_disclosures[0];
    assert_eq!(
        (n.operating, n.investing, n.financing, n.exchanged),
        (x("60.00"), x("1000.00"), x("-1060.00"), x("1000.00"))
    );
    assert_eq!(
        recon(&cf),
        [
            (s("Other noncash items"), x("60.00")),
            (s("Increase in 1300 Prepaid"), x("-60.00")),
        ]
    );

    // The same with no members configured (equity named by account, not
    // a member's): still property for capital, noncash; while an expense
    // an owner paid into that capital is contributed and spent.
    let mut b = funded();
    b.members.clear();
    let e = b.post(
        d(2026, 2, 3),
        "Laptop contributed",
        &[("1500", DR, "1000.00"), ("3000", CR, "1000.00")],
    );
    let x2 = b.post(
        d(2026, 2, 4),
        "Supplies, paid by the owner, contributed",
        &[("6000", DR, "30.00"), ("3000", CR, "30.00")],
    );
    let cf = feb(&b, &["2100"], &[]);
    assert!(cf.investing.is_empty());
    assert_eq!(disclosed(&cf), [(e, x("1000.00"))]);
    assert_eq!(em(&cf.operating), [(x2, x("-30.00"))]);
    assert_eq!(em(&cf.financing), [(x2, x("30.00"))]);
    assert_eq!(
        cf.financing[0].kind,
        "Contributions from owners — 3000 Capital — Member A"
    );
}

/// Seller financing stays noncash (ASC 230-10-50-4: "acquiring assets by
/// assuming directly related liabilities"; EY FRD 5.3.1): equipment
/// bought on the seller's own credit, an operating payable (Dr Equipment
/// 1,200 / Cr Accounts Payable 1,200). No line; disclosed 1,200; flagged
/// `asset_on_operating_payable` (D-35's IOU flag), whose text says the
/// purchase is noncash and how to list a lender's credit instead.
#[test]
fn equipment_on_the_sellers_credit_stays_noncash_and_flagged() {
    let mut b = funded();
    let e = b.post(
        d(2026, 2, 9),
        "Equipment on the seller's terms",
        &[("1500", DR, "1200.00"), ("2000", CR, "1200.00")],
    );
    let cf = feb(&b, &["2100", "2500"], &[]);
    for sec in CashFlowSection::ALL {
        assert!(cf.lines(sec).is_empty(), "{sec:?}");
    }
    assert_eq!(disclosed(&cf), [(e, x("1200.00"))]);
    assert_eq!(cf.warnings.len(), 1);
    let w = &cf.warnings[0];
    assert_eq!(
        (w.entry, w.kind),
        (e, CashFlowWarningKind::AssetOnOperatingPayable)
    );
    assert!(w.message.contains("seller's own credit"), "{}", w.message);
    assert!(w.message.contains("borrowed and spent"), "{}", w.message);
}

/// rules.md B-14 rule 6 with rule 11: a payment a member made for the
/// company and its reversal, both in February, drop out together (their
/// flows, constructive cash included, are exact negatives: Kani
/// `constructive_mirrors`); no line, no disclosure, no total moves. The
/// reversal alone (its original in January) is listed under the
/// original's kinds and class with its own sign: operating +100 class
/// (d), financing −100 under "Proceeds from borrowing — 2100", both
/// labelled "Reversal of #e", `on_behalf` too.
#[test]
fn a_payment_made_for_the_company_and_its_reversal_net() {
    let mut b = funded();
    let before = b.cf(FEB.0, FEB.1, &["1000", "1010"], &["2100"], &[]);
    let e = b.post(
        d(2026, 2, 5),
        "Software, paid by Member A",
        &[("6000", DR, "100.00"), ("2100", CR, "100.00")],
    );
    b.reverse(e, d(2026, 2, 6));
    let cf = feb(&b, &["2100"], &[]);
    for sec in CashFlowSection::ALL {
        assert!(cf.lines(sec).is_empty(), "{sec:?}");
        assert_eq!(cf.total(sec), before.total(sec));
    }
    assert!(cf.noncash_disclosures.is_empty());
    assert!(cf.operating_reconciliation.lines.is_empty());

    let mut b = funded();
    let e = b.post(
        d(2026, 1, 20),
        "Software, paid by Member A",
        &[("6000", DR, "100.00"), ("2100", CR, "100.00")],
    );
    let r = b.reverse(e, d(2026, 2, 6));
    let cf = feb(&b, &["2100"], &[]);
    assert_eq!(em(&cf.operating), [(r, x("100.00"))]);
    assert_eq!(
        cf.operating[0].class,
        Some(OperatingClass::ToSuppliersAndEmployees)
    );
    assert_eq!(cf.operating[0].on_behalf, x("100.00"));
    assert_eq!(em(&cf.financing), [(r, x("-100.00"))]);
    assert_eq!(
        cf.financing[0].kind,
        "Proceeds from borrowing — 2100 Due to members"
    );
    assert_eq!(cf.financing[0].on_behalf, x("-100.00"));
    assert_eq!(
        cf.financing[0].label,
        format!("Reversal of #{e} — Software, paid by Member A")
    );
    assert!(cf.noncash_disclosures.is_empty());
    no_distribution(&cf);

    // The same for what only a payment can settle (a prepaid, equipment a
    // lender paid for): the reversal alone receives back what its root
    // paid, under the root's kinds: operating +50, investing +2,000,
    // financing −2,050; nothing disclosed.
    let mut b = funded();
    let p = b.post(
        d(2026, 1, 20),
        "Insurance, prepaid by Member A",
        &[("1300", DR, "50.00"), ("2100", CR, "50.00")],
    );
    let v = b.post(
        d(2026, 1, 21),
        "Van, paid by the bank",
        &[("1520", DR, "2000.00"), ("2500", CR, "2000.00")],
    );
    let rp = b.reverse(p, d(2026, 2, 6));
    let rv = b.reverse(v, d(2026, 2, 7));
    let cf = feb(&b, &["2100", "2500"], &[]);
    assert_eq!(em(&cf.operating), [(rp, x("50.00"))]);
    assert_eq!(em(&cf.investing), [(rv, x("2000.00"))]);
    assert_eq!(cf.investing[0].kind, "Purchases of property and equipment");
    assert_eq!(em(&cf.financing), [(rp, x("-50.00")), (rv, x("-2000.00"))]);
    assert!(cf.noncash_disclosures.is_empty());
}

/// Rule 11, part paid by the company and part by a member, in one entry
/// (Dr Supplies 100 / Cr Cash 40 / Cr 2100 60): the cash settles 40
/// (50-5), the member the other 60, so operating −100 with 60 on the
/// company's behalf, financing +60: the same as the two entries apart.
/// And money a member received for the company (a vendor's refund of 30
/// to the member's card, kept against what the company owes them: Dr 2100
/// / Cr Supplies 30) is collected and repaid: operating +30, financing
/// −30 (PwC FSP 6.9.3.1: cash "received or paid" by a third party).
#[test]
fn part_paid_by_a_member_and_a_refund_a_member_received() {
    let mut b = funded();
    let e = b.post(
        d(2026, 2, 5),
        "Supplies, part paid by Member A",
        &[
            ("6000", DR, "100.00"),
            ("1000", CR, "40.00"),
            ("2100", CR, "60.00"),
        ],
    );
    let r = b.post(
        d(2026, 2, 12),
        "Refund of supplies, to Member A's card",
        &[("2100", DR, "30.00"), ("6000", CR, "30.00")],
    );
    let cf = feb(&b, &["2100"], &[]);
    // By class: the refund is (c), other receipts; the supplies (d).
    assert_eq!(em(&cf.operating), [(r, x("30.00")), (e, x("-100.00"))]);
    assert_eq!(cf.operating[0].class, Some(OperatingClass::OtherReceipts));
    assert_eq!(cf.operating[0].on_behalf, x("30.00"));
    assert_eq!(cf.operating[1].on_behalf, x("-60.00"));
    assert_eq!(em(&cf.financing), [(e, x("60.00")), (r, x("-30.00"))]);
    assert_eq!(cf.net_change_in_cash, x("-40.00"));
    assert!(cf.noncash_disclosures.is_empty());
}

/// Rule 11's CONVENTION, pinned: what a lender or the members' account
/// pays for goes to operating balance-sheet lines before revenue and
/// expense, so a member's capital (which can pay only for revenue and
/// expense) is left the expenses. Supplies 100 and a prepaid 50, paid 50
/// through 2100 and 100 into Member A's capital (Dr 6000 100, Dr 1300 50
/// / Cr 2100 50, Cr 3000 100): 2100 pays the prepaid, capital the
/// supplies; all of it constructive (operating −150, financing +150),
/// nothing disclosed. Taken the other way round, 2100 would pay 50 of the
/// supplies, capital the other 50, and the prepaid's 50 would be left a
/// noncash contribution. And a customer paying a member for an invoice
/// (Dr 2100 / Cr Accounts Receivable 70) is collected for the company:
/// operating +70, class (a), and a repayment −70.
#[test]
fn a_lender_pays_balance_sheet_lines_first_and_a_customer_pays_a_member() {
    let mut b = funded();
    let e = b.post(
        d(2026, 2, 5),
        "Supplies and a prepaid, paid by Member A",
        &[
            ("6000", DR, "100.00"),
            ("1300", DR, "50.00"),
            ("2100", CR, "50.00"),
            ("3000", CR, "100.00"),
        ],
    );
    let c = b.post(
        d(2026, 2, 9),
        "Customer paid Member A",
        &[("2100", DR, "70.00"), ("1200", CR, "70.00")],
    );
    let cf = feb(&b, &["2100"], &[]);
    assert_eq!(em(&cf.operating), [(c, x("70.00")), (e, x("-150.00"))]);
    assert_eq!(cf.operating[0].class, Some(OperatingClass::FromCustomers));
    assert_eq!(
        kinds(&cf, CashFlowSection::Financing),
        [
            (s("Contributions from members — Member A"), x("100.00")),
            (
                s("Proceeds from borrowing — 2100 Due to members"),
                x("50.00")
            ),
            (
                s("Repayments of borrowing — 2100 Due to members"),
                x("-70.00")
            ),
        ]
    );
    assert!(cf.noncash_disclosures.is_empty());
}

/// Rule 11's exceptions: interest charged into a loan (Dr Interest
/// expense 25 / Cr Bank loan 25) and a guaranteed payment credited to what
/// the company owes a member (Dr Guaranteed payments 300 / Cr 2100) are
/// the lender's and the member's own charges, paid to no one: noncash.
/// Each is disclosed as a noncash financing activity (50-3) and reconciled
/// through its account; no line. ("Interest paid", 45-25(e), is cash
/// paid.)
#[test]
fn interest_and_guaranteed_payments_charged_to_a_creditor_stay_noncash() {
    let mut b = funded();
    let gp = b.acct("6600", "Guaranteed payments", AccountType::Expense);
    b.l.set_account_tax(gp, Some("guaranteed_payments"), None)
        .unwrap();
    let i = b.post(
        d(2026, 2, 28),
        "Interest added to the loan",
        &[("6200", DR, "25.00"), ("2500", CR, "25.00")],
    );
    let g = b.post(
        d(2026, 2, 28),
        "Guaranteed payment owed to Member A",
        &[("6600", DR, "300.00"), ("2100", CR, "300.00")],
    );
    let cf = feb(&b, &["2100", "2500"], &[]);
    for sec in CashFlowSection::ALL {
        assert!(cf.lines(sec).is_empty(), "{sec:?}");
    }
    assert_eq!(disclosed(&cf), [(i, x("25.00")), (g, x("300.00"))]);
    assert_eq!(
        recon(&cf),
        [
            (s("Noncash: through 2100 Due to members"), x("300.00")),
            (s("Noncash: through 2500 Bank loan"), x("25.00")),
        ]
    );
}

/// rules.md B-14 rule 6 (B-9: a correction undoes what it corrects): an
/// entry and its reversal both in the period drop out of every line and
/// disclosure, and no total moves (their flows are exact negatives, Kani
/// `entry_flows_of_a_reversal_mirror`).
#[test]
fn an_entry_and_its_reversal_in_the_period_drop_out() {
    let mut b = funded();
    b.post(
        d(2026, 2, 3),
        "Supplies",
        &[("6000", DR, "40.00"), ("1000", CR, "40.00")],
    );
    let before = feb(&b, &["2100"], &[]);
    let mut after = funded();
    after.post(
        d(2026, 2, 3),
        "Supplies",
        &[("6000", DR, "40.00"), ("1000", CR, "40.00")],
    );
    let c = after.post(
        d(2026, 2, 4),
        "Contribution",
        &[("1000", DR, "700.00"), ("3001", CR, "700.00")],
    );
    let p = after.post(
        d(2026, 2, 5),
        "Laptop contributed",
        &[("1500", DR, "900.00"), ("3001", CR, "900.00")],
    );
    after.reverse(c, d(2026, 2, 20));
    after.reverse(p, d(2026, 2, 21));
    let cf = feb(&after, &["2100"], &[]);
    assert_eq!(em(&cf.operating), em(&before.operating));
    assert!(cf.financing.is_empty() && cf.investing.is_empty());
    assert!(cf.noncash_disclosures.is_empty());
    assert_eq!(cf.cash_from_financing, Decimal::ZERO);
    assert_eq!(cf.net_change_in_cash, before.net_change_in_cash);
}

/// rules.md B-14 rule 6: a reversal shown alone (its original in an
/// earlier period) is labelled "Reversal of #N — <N's description>" and
/// listed under its original's kind, never as a distribution (D1-c).
#[test]
fn a_reversal_alone_is_labelled_reversal_of() {
    let mut b = funded();
    let c = b.post(
        d(2026, 1, 20),
        "Contribution — Member B",
        &[("1000", DR, "700.00"), ("3001", CR, "700.00")],
    );
    let r = b.reverse(c, d(2026, 2, 10));
    let cf = feb(&b, &[], &[]);
    assert_eq!(em(&cf.financing), [(r, x("-700.00"))]);
    let line = &cf.financing[0];
    assert_eq!(
        line.label,
        format!("Reversal of #{c} — Contribution — Member B")
    );
    assert_eq!(line.kind, "Contributions from members — Member B");
    assert_eq!(line.reverses, Some(c));
    no_distribution(&cf);
}

/// rules.md B-14 rules 4 and 6: a reversal takes its chain root's
/// sections, class and kind, with its own sign. Reversing January's cash
/// sale at a 300 gain in February: investing −500 under "Proceeds from
/// sales of property and equipment", G +300 (the gain, reversed, is still
/// investing). Reversing January's supplier payment of 100: operating
/// +100 under (d).
#[test]
fn a_reversal_takes_its_originals_sections_and_class() {
    let mut b = funded();
    b.post(
        d(2026, 1, 3),
        "Buy equipment",
        &[("1500", DR, "1000.00"), ("1000", CR, "1000.00")],
    );
    b.post(
        d(2026, 1, 5),
        "Depreciation",
        &[("6100", DR, "800.00"), ("1510", CR, "800.00")],
    );
    let sale = b.post(
        d(2026, 1, 25),
        "Sell equipment",
        &[
            ("1000", DR, "500.00"),
            ("1510", DR, "800.00"),
            ("1500", CR, "1000.00"),
            ("4900", CR, "300.00"),
        ],
    );
    let pay = b.post(
        d(2026, 1, 26),
        "Supplies",
        &[("6000", DR, "100.00"), ("1000", CR, "100.00")],
    );
    let rs = b.reverse(sale, d(2026, 2, 2));
    let rp = b.reverse(pay, d(2026, 2, 3));
    let cf = feb(&b, &[], &[]);
    assert_eq!(em(&cf.investing), [(rs, x("-500.00"))]);
    assert_eq!(
        cf.investing[0].kind,
        "Proceeds from sales of property and equipment"
    );
    assert_eq!(cf.operating_reconciliation.net_income, x("-200.00"));
    assert_eq!(
        recon(&cf),
        [(
            s("(Gain) loss and other noncash changes in investing assets"),
            x("300.00")
        )]
    );
    assert_eq!(em(&cf.operating), [(rp, x("100.00"))]);
    assert_eq!(
        cf.operating[0].class,
        Some(OperatingClass::ToSuppliersAndEmployees)
    );
    assert!(cf.noncash_disclosures.is_empty());
}

/// Review item 1: a reversal of a reversal. A ← B ← C all in the period:
/// pairing walks the chain from its root, so (A, B) drop out and C is
/// shown, "Reinstatement of #A (reverses #B)", under A's kind and with
/// A's sections (a cash sale at a gain: C's gain is investing). With A
/// before the period, (B, C) pair and nothing is shown; with C after it,
/// B is shown alone.
#[test]
fn a_reversal_of_a_reversal_is_shown_and_foots() {
    let mut b = funded();
    let a = b.post(
        d(2026, 2, 2),
        "Contribution — Member B",
        &[("1000", DR, "300.00"), ("3001", CR, "300.00")],
    );
    let r = b.reverse(a, d(2026, 2, 3));
    let c = b.reverse(r, d(2026, 2, 4));
    let cf = feb(&b, &[], &[]);
    assert_eq!(em(&cf.financing), [(c, x("300.00"))]);
    assert_eq!(
        cf.financing[0].label,
        format!("Reinstatement of #{a} (reverses #{r})")
    );
    assert_eq!(
        cf.financing[0].kind,
        "Contributions from members — Member B"
    );

    // A disposal chain: C takes A's sections.
    let mut b = funded();
    b.post(
        d(2026, 1, 3),
        "Buy equipment",
        &[("1500", DR, "1000.00"), ("1000", CR, "1000.00")],
    );
    let a = b.post(
        d(2026, 2, 2),
        "Sell equipment",
        &[
            ("1000", DR, "1300.00"),
            ("1500", CR, "1000.00"),
            ("4900", CR, "300.00"),
        ],
    );
    let r = b.reverse(a, d(2026, 2, 3));
    let c = b.reverse(r, d(2026, 2, 4));
    let cf = feb(&b, &[], &[]);
    assert_eq!(em(&cf.investing), [(c, x("1300.00"))]);
    assert!(cf.operating.is_empty());
    assert_eq!(recon(&cf)[0].1, x("-300.00"));

    // The root before the period: B and C pair.
    let mut b = funded();
    let a = b.post(
        d(2026, 1, 20),
        "Contribution — Member B",
        &[("1000", DR, "300.00"), ("3001", CR, "300.00")],
    );
    let r = b.reverse(a, d(2026, 2, 3));
    b.reverse(r, d(2026, 2, 4));
    let cf = feb(&b, &[], &[]);
    assert!(cf.financing.is_empty());
    assert_eq!(cf.cash_from_financing, Decimal::ZERO);

    // The reinstatement after the period: B alone.
    let mut b = funded();
    let a = b.post(
        d(2026, 1, 20),
        "Contribution — Member B",
        &[("1000", DR, "300.00"), ("3001", CR, "300.00")],
    );
    let r = b.reverse(a, d(2026, 2, 3));
    b.reverse(r, d(2026, 3, 4));
    let cf = feb(&b, &[], &[]);
    assert_eq!(em(&cf.financing), [(r, x("-300.00"))]);
    assert!(cf.financing[0].label.starts_with("Reversal of #"));
}

/// ASC 230-10-50-5: "only the cash portion shall be reported". Equipment
/// 1,000 for 300 cash and 700 owed to the seller on its own terms (2000
/// Accounts Payable, operating): investing −300; the 700 is disclosed
/// (investing +700, operating −700: 50-4's "acquiring assets by assuming
/// directly related liabilities") and flagged. The same purchase with the
/// 700 lent by a bank that paid the seller (2500, financing) is D-42's:
/// investing −1,000 (700 of it on the company's behalf), financing +700,
/// nothing disclosed.
#[test]
fn part_cash_part_noncash_reports_only_the_cash() {
    let mut b = funded();
    let e = b.post(
        d(2026, 2, 8),
        "Equipment, part on the seller's terms",
        &[
            ("1520", DR, "1000.00"),
            ("1000", CR, "300.00"),
            ("2000", CR, "700.00"),
        ],
    );
    let cf = feb(&b, &["2500"], &[]);
    assert_eq!(em(&cf.investing), [(e, x("-300.00"))]);
    assert_eq!(cf.investing[0].kind, "Purchases of property and equipment");
    assert_eq!(cf.investing[0].on_behalf, Decimal::ZERO);
    assert!(cf.financing.is_empty() && cf.operating.is_empty());
    assert_eq!(disclosed(&cf), [(e, x("700.00"))]);
    let n = &cf.noncash_disclosures[0];
    assert_eq!(
        (n.operating, n.investing, n.financing, n.exchanged),
        (x("-700.00"), x("700.00"), x("0"), x("0"))
    );
    assert_eq!(cf.warnings.len(), 1);

    let mut b = funded();
    let e = b.post(
        d(2026, 2, 8),
        "Equipment, part on a bank loan",
        &[
            ("1520", DR, "1000.00"),
            ("1000", CR, "300.00"),
            ("2500", CR, "700.00"),
        ],
    );
    let cf = feb(&b, &["2500"], &[]);
    assert_eq!(em(&cf.investing), [(e, x("-1000.00"))]);
    assert_eq!(cf.investing[0].on_behalf, x("-700.00"));
    assert_eq!(em(&cf.financing), [(e, x("700.00"))]);
    assert_eq!(cf.financing[0].on_behalf, x("700.00"));
    assert!(cf.noncash_disclosures.is_empty());
    assert!(cf.warnings.is_empty());
}

/// A fee the bank withholds from a 10,000 loan (Dr cash 9,900, Dr fee 100
/// / Cr loan 10,000). Posted to an expense account, the fee is an
/// operating payment the lender made out of the loan for the company
/// (D-42): financing +10,000 (9,900 cash, 100 on behalf), operating −100.
/// Debt issue costs are financing (ASC 230-10-45-15(f)): posted to a
/// contra-liability listed in books.financing_accounts, the loan reads as
/// its net proceeds, financing +9,900, the 100 a noncash exchange inside
/// financing (disclosed 100).
#[test]
fn a_fee_withheld_from_a_loan_follows_its_account() {
    let mut b = funded();
    let e = b.post(
        d(2026, 2, 8),
        "Bank loan, fee withheld",
        &[
            ("1000", DR, "9900.00"),
            ("6000", DR, "100.00"),
            ("2500", CR, "10000.00"),
        ],
    );
    let cf = feb(&b, &["2500"], &[]);
    assert_eq!(em(&cf.financing), [(e, x("10000.00"))]);
    assert_eq!(cf.financing[0].on_behalf, x("100.00"));
    assert_eq!(
        cf.financing[0].kind,
        "Proceeds from borrowing — 2500 Bank loan"
    );
    assert_eq!(em(&cf.operating), [(e, x("-100.00"))]);
    assert!(cf.noncash_disclosures.is_empty());
    assert!(cf.operating_reconciliation.lines.is_empty());

    let mut b = funded();
    b.acct("2510", "Debt issuance costs", AccountType::Liability);
    let e = b.post(
        d(2026, 2, 8),
        "Bank loan, fee withheld",
        &[
            ("1000", DR, "9900.00"),
            ("2510", DR, "100.00"),
            ("2500", CR, "10000.00"),
        ],
    );
    let cf = feb(&b, &["2500", "2510"], &[]);
    assert_eq!(em(&cf.financing), [(e, x("9900.00"))]);
    assert_eq!(cf.financing[0].on_behalf, Decimal::ZERO);
    assert!(cf.operating.is_empty());
    assert_eq!(disclosed(&cf), [(e, x("100.00"))]);
    assert_eq!(cf.noncash_disclosures[0].exchanged, x("100.00"));
}

/// ASC 230-10-45-15(b) and 45-25(e): a loan payment of 1,000 with 100
/// interest is a repayment of 900 (financing) and interest paid of 100
/// (operating, class (e)).
#[test]
fn interest_paid_with_principal_splits_by_section() {
    let mut b = funded();
    let e = b.post(
        d(2026, 2, 8),
        "Loan payment",
        &[
            ("2500", DR, "900.00"),
            ("6200", DR, "100.00"),
            ("1000", CR, "1000.00"),
        ],
    );
    let cf = feb(&b, &["2500"], &[]);
    assert_eq!(em(&cf.financing), [(e, x("-900.00"))]);
    assert_eq!(em(&cf.operating), [(e, x("-100.00"))]);
    assert_eq!(cf.operating[0].class, Some(OperatingClass::InterestPaid));
}

/// ASC 230-10-45-12(a) and 45-16(b): collecting a 1,000 loan the company
/// made, with 40 interest: the principal is investing, the interest
/// (tax category interest_income) stays operating, class (b): rule 4's
/// exception.
#[test]
fn interest_collected_with_a_loan_stays_operating() {
    let mut b = funded();
    b.post(
        d(2026, 1, 10),
        "Lend to Member B",
        &[("1400", DR, "1000.00"), ("1000", CR, "1000.00")],
    );
    let e = b.post(
        d(2026, 2, 10),
        "Member B repays with interest",
        &[
            ("1000", DR, "1040.00"),
            ("1400", CR, "1000.00"),
            ("4100", CR, "40.00"),
        ],
    );
    let cf = feb(&b, &[], &["1400"]);
    assert_eq!(em(&cf.investing), [(e, x("1000.00"))]);
    assert_eq!(cf.investing[0].kind, "Collections on 1400 Loan to member");
    assert_eq!(em(&cf.operating), [(e, x("40.00"))]);
    assert_eq!(
        cf.operating[0].class,
        Some(OperatingClass::InterestAndDividendsReceived)
    );
    assert!(cf.operating_reconciliation.lines.is_empty());
}

/// ASC 230-10-45-25 (a)-(g), one entry per class, decided by structure:
/// (a) a sale (Cr revenue), and a deposit from a customer (Cr deferred
/// revenue, the customer named: review item 10); (b) interest received;
/// (c) a supplier's refund; (d) supplies paid; (e) interest paid; (g) a
/// refund to a customer (Dr revenue), and a government fee (tax category
/// `taxes_licenses`: payments to governments for taxes and fees are
/// 45-17(c), apart from suppliers' 45-17(b), and 45-25 names only income
/// taxes). The printed rows are the classes in order, each its sum.
#[test]
fn operating_classes_follow_asc_230_10_45_25() {
    let mut b = funded();
    let cash = |b: &mut Books, day, desc: &str, other: &'static str, side: Side, amount| {
        let cash_side = if side == DR { CR } else { DR };
        b.post(
            d(2026, 2, day),
            desc,
            &[("1000", cash_side, amount), (other, side, amount)],
        )
    };
    let a1 = cash(&mut b, 2, "Sale", "4000", CR, "300.00");
    let a2 = cash(&mut b, 3, "Deposit", "2300", CR, "200.00");
    let customer = b.l.add_customer("Acme LLC").unwrap();
    let party = b.l.party_of_customer(customer).unwrap();
    b.l.set_entry_party(a2, Some(party)).unwrap();
    let b1 = cash(&mut b, 4, "Bank interest", "4100", CR, "7.00");
    let c1 = cash(&mut b, 5, "Supplier refund", "6000", CR, "15.00");
    let d1 = cash(&mut b, 6, "Supplies", "6000", DR, "120.00");
    let e1 = cash(&mut b, 7, "Interest", "6200", DR, "30.00");
    let g1 = cash(&mut b, 8, "Customer refund", "4000", DR, "20.00");
    // A tax or a government fee is not a supplier's (45-17(c) apart from
    // 45-17(b)): (g), as 45-25 names only income taxes, (f).
    let fees = b.acct("6700", "Taxes and licenses", AccountType::Expense);
    b.l.set_account_tax(fees, Some("taxes_licenses"), None)
        .unwrap();
    let g2 = cash(&mut b, 9, "Annual report fee", "6700", DR, "25.00");
    let cf = feb(&b, &[], &[]);
    let class = |e| cf.operating.iter().find(|l| l.entry == e).unwrap().class;
    use OperatingClass::*;
    assert_eq!(class(a1), Some(FromCustomers));
    assert_eq!(class(a2), Some(FromCustomers));
    assert_eq!(class(b1), Some(InterestAndDividendsReceived));
    assert_eq!(class(c1), Some(OtherReceipts));
    assert_eq!(class(d1), Some(ToSuppliersAndEmployees));
    assert_eq!(class(e1), Some(InterestPaid));
    assert_eq!(class(g1), Some(OtherPayments));
    assert_eq!(class(g2), Some(OtherPayments));
    assert_eq!(
        cf.operating_by_class(),
        [
            (FromCustomers, x("500.00")),
            (InterestAndDividendsReceived, x("7.00")),
            (OtherReceipts, x("15.00")),
            (ToSuppliersAndEmployees, x("-120.00")),
            (InterestPaid, x("-30.00")),
            (OtherPayments, x("-45.00")),
        ]
    );
    assert_eq!(cf.cash_from_operations, x("327.00"));
}

/// ASC 230-10-45-28(b): depreciation is an item of net income with no
/// cash effect, a reconciling item; it is not an investing or financing
/// activity, so nothing is disclosed (rules.md B-14 rule 7).
#[test]
fn depreciation_is_reconciling_not_disclosed() {
    let mut b = funded();
    b.post(
        d(2026, 2, 28),
        "Depreciation",
        &[("6100", DR, "100.00"), ("1510", CR, "100.00")],
    );
    let cf = feb(&b, &[], &[]);
    assert!(cf.noncash_disclosures.is_empty());
    assert_eq!(recon(&cf), [(s("Depreciation"), x("100.00"))]);
    assert_eq!(
        cf.operating_reconciliation.lines[0].kind,
        ReconcilingKind::Depreciation
    );
}

/// Defect D1-e: cash spent through an account that is also an asset's
/// depreciation expense account is spending, not an add-back. 50 paid
/// from cash to 6100: operating −50, class (d), and no depreciation line.
#[test]
fn cash_spent_on_a_depreciation_expense_account_is_not_added_back() {
    let mut b = funded();
    let e = b.post(
        d(2026, 2, 3),
        "Misposted repair",
        &[("6100", DR, "50.00"), ("1000", CR, "50.00")],
    );
    let cf = feb(&b, &[], &[]);
    assert_eq!(em(&cf.operating), [(e, x("-50.00"))]);
    assert!(cf.operating_reconciliation.lines.is_empty());
    assert_eq!(cf.operating_reconciliation.net_income, x("-50.00"));
}

/// ASC 230-10-45-29 and 45-30: net income plus the reconciling items is
/// net cash from operating activities. A month of mixed activity: sales
/// on account 1,000 (500 collected), supplies on the card 200 (paid 150),
/// a prepaid 120, depreciation 50. By hand: operating cash 500 − 150 −
/// 120 = +230; net income 1,000 − 200 − 50 = 750; depreciation +50;
/// receivables −500; prepaid −120; card +50: 750 + 50 − 500 − 120 + 50 =
/// 230.
#[test]
fn operating_reconciliation_ties_to_operating_cash() {
    let mut b = funded();
    b.post(
        d(2026, 2, 2),
        "Invoice",
        &[("1200", DR, "1000.00"), ("4000", CR, "1000.00")],
    );
    b.post(
        d(2026, 2, 9),
        "Collect",
        &[("1000", DR, "500.00"), ("1200", CR, "500.00")],
    );
    b.post(
        d(2026, 2, 10),
        "Supplies",
        &[("6000", DR, "200.00"), ("2050", CR, "200.00")],
    );
    b.post(
        d(2026, 2, 20),
        "Pay card",
        &[("2050", DR, "150.00"), ("1000", CR, "150.00")],
    );
    b.post(
        d(2026, 2, 21),
        "Prepay",
        &[("1300", DR, "120.00"), ("1000", CR, "120.00")],
    );
    b.post(
        d(2026, 2, 28),
        "Depreciation",
        &[("6100", DR, "50.00"), ("1510", CR, "50.00")],
    );
    let cf = feb(&b, &[], &[]);
    assert_eq!(cf.cash_from_operations, x("230.00"));
    assert_eq!(cf.operating_reconciliation.net_income, x("750.00"));
    assert_eq!(
        recon(&cf),
        [
            (s("Depreciation"), x("50.00")),
            (s("Increase in 1200 Accounts Receivable"), x("-500.00")),
            (s("Increase in 1300 Prepaid"), x("-120.00")),
            (s("Increase in 2050 Card"), x("50.00")),
        ]
    );
    assert_eq!(
        cf.operating_by_class(),
        [
            (OperatingClass::FromCustomers, x("500.00")),
            (OperatingClass::ToSuppliersAndEmployees, x("-270.00")),
        ]
    );
}

/// rules.md B-11 and B-14 rule 10: a period close moves earnings into
/// equity; it earns and spends nothing, so it is never in the statement.
#[test]
fn closing_entries_never_appear() {
    let mut b = funded();
    b.post(
        d(2026, 2, 2),
        "Sale",
        &[("1000", DR, "300.00"), ("4000", CR, "300.00")],
    );
    let open = feb(&b, &[], &[]);
    let p = b.l.create_period("2026-02", FEB.0, FEB.1).unwrap();
    let close = b.l.close_period(p).unwrap().unwrap();
    let closed = feb(&b, &[], &[]);
    assert_eq!(closed, open);
    let text = format!("{closed:?}");
    assert!(!text.contains(&format!("entry: {close},")));
}

/// The period is [start, end] inclusive (rules.md B-14 rule 1): a
/// contribution on the start date is a flow, not opening cash.
#[test]
fn cash_flow_starts_counting_on_the_start_date() {
    let b = funded();
    let cf = b.cf(d(2026, 1, 2), d(2026, 1, 2), &["1000"], &[], &[]);
    foots(&cf);
    assert_eq!(cf.cash_beginning, Decimal::ZERO);
    assert_eq!(cf.cash_ending, x("10000.00"));
    assert_eq!(em(&cf.financing), [(1, x("10000.00"))]);
    let cf = b.cf(d(2026, 1, 3), d(2026, 1, 3), &["1000"], &[], &[]);
    assert_eq!(cf.cash_beginning, x("10000.00"));
    assert!(cf.financing.is_empty());
    assert!(matches!(
        cash_flow(
            &b.l,
            &b.params(d(2026, 1, 3), d(2026, 1, 2), &["1000"], &[], &[])
        ),
        Err(LedgerError::InvalidDateRange { .. })
    ));
}

/// ASC 230-10-45-24 (the statement explains the change in cash): moving
/// money between two cash accounts is no flow.
#[test]
fn a_transfer_between_cash_accounts_is_not_a_flow() {
    let mut b = funded();
    b.post(
        d(2026, 2, 2),
        "To savings",
        &[("1010", DR, "4000.00"), ("1000", CR, "4000.00")],
    );
    let cf = feb(&b, &[], &[]);
    for sec in CashFlowSection::ALL {
        assert!(cf.lines(sec).is_empty());
    }
    assert_eq!(cf.net_change_in_cash, Decimal::ZERO);
    assert!(cf.noncash_disclosures.is_empty());
}

/// Review round 2, footing in odd shapes (rules.md B-14 rules 1, 6, 9,
/// 11), by hand:
/// - two cash accounts in one entry, a transfer less its fee (Dr Savings
///   99, Dr Supplies 1 / Cr Cash 100): only the net, −1, moves cash,
///   operating (d); no investing or financing;
/// - one account on both sides (Dr Supplies 50, Dr 2100 20 / Cr 2100 70):
///   2100 nets to a 50 credit first, which pays the supplies: operating
///   −50, financing +50;
/// - a chain across the period start: a member's payment on Jan 20, its
///   reversal on Feb 3 and the reinstatement on Feb 4. February: the
///   reversal and reinstatement pair, nothing shown, no total moves;
///   January: the payment, borrowed and spent;
/// - closing February changes nothing.
#[test]
fn odd_shapes_foot_and_net() {
    let mut b = funded();
    let t = b.post(
        d(2026, 2, 2),
        "To savings, less the fee",
        &[
            ("1010", DR, "99.00"),
            ("6000", DR, "1.00"),
            ("1000", CR, "100.00"),
        ],
    );
    let n = b.post(
        d(2026, 2, 3),
        "Supplies, Member A, netted",
        &[
            ("6000", DR, "50.00"),
            ("2100", DR, "20.00"),
            ("2100", CR, "70.00"),
        ],
    );
    let a = b.post(
        d(2026, 1, 20),
        "Software, paid by Member A",
        &[("6000", DR, "40.00"), ("2100", CR, "40.00")],
    );
    let r = b.reverse(a, d(2026, 2, 3));
    b.reverse(r, d(2026, 2, 4));
    let cf = feb(&b, &["2100"], &[]);
    assert_eq!(em(&cf.operating), [(t, x("-1.00")), (n, x("-50.00"))]);
    assert!(cf.investing.is_empty());
    assert_eq!(em(&cf.financing), [(n, x("50.00"))]);
    assert_eq!(cf.net_change_in_cash, x("-1.00"));
    assert!(cf.noncash_disclosures.is_empty());
    let jan = b.cf(
        d(2026, 1, 1),
        d(2026, 1, 31),
        &["1000", "1010"],
        &["2100"],
        &[],
    );
    foots(&jan);
    assert_eq!(em(&jan.financing)[1..], [(a, x("40.00"))]);
    assert_eq!(em(&jan.operating), [(a, x("-40.00"))]);
    let p = b.l.create_period("2026-02", FEB.0, FEB.1).unwrap();
    b.l.close_period(p).unwrap();
    assert_eq!(feb(&b, &["2100"], &[]), cf);
}

/// Review item 2, ASC 230-10-50-4's first example, "converting debt to
/// equity": a member's 500 balance in 2100 turned into capital (Dr 2100 /
/// Cr 3000, both financing) is disclosed as an exchange of 500, though no
/// section's noncash part moves.
#[test]
fn converting_member_debt_to_capital_is_disclosed() {
    let mut b = funded();
    let e = b.post(
        d(2026, 2, 3),
        "Member A's advances to capital",
        &[("2100", DR, "500.00"), ("3000", CR, "500.00")],
    );
    let cf = feb(&b, &["2100"], &[]);
    assert_eq!(disclosed(&cf), [(e, x("500.00"))]);
    let n = &cf.noncash_disclosures[0];
    assert_eq!(
        (n.operating, n.investing, n.financing, n.exchanged),
        (x("0"), x("0"), x("0"), x("500.00"))
    );
}

/// ASC 230-10-50-4, "exchanging noncash assets": a 700 note received with
/// 300 cash for equipment (cost 1,000, accumulated 800, gain 800).
/// Investing +300; disclosed 700, the note for the equipment.
#[test]
fn a_note_received_for_equipment_is_disclosed() {
    let mut b = funded();
    b.post(
        d(2026, 1, 5),
        "Depreciation",
        &[("6100", DR, "800.00"), ("1510", CR, "800.00")],
    );
    let e = b.post(
        d(2026, 2, 3),
        "Sell equipment for a note and cash",
        &[
            ("1250", DR, "700.00"),
            ("1000", DR, "300.00"),
            ("1510", DR, "800.00"),
            ("1500", CR, "1000.00"),
            ("4900", CR, "800.00"),
        ],
    );
    let cf = feb(&b, &[], &["1250"]);
    assert_eq!(em(&cf.investing), [(e, x("300.00"))]);
    assert_eq!(disclosed(&cf), [(e, x("700.00"))]);
    assert_eq!(recon(&cf)[0].1, x("-800.00"));
}

/// ASC 230-10-50-4: a trade-in. A new vehicle (2,000, on 1520) for the
/// old equipment (cost 1,000 on 1500, accumulated 800, gain 300) and
/// 1,500 cash: investing −1,500 and an exchange of 500 disclosed. Within
/// one cost account a trade-in nets away (a limit B-14 records).
#[test]
fn a_trade_in_is_disclosed() {
    let mut b = funded();
    b.post(
        d(2026, 1, 5),
        "Depreciation",
        &[("6100", DR, "800.00"), ("1510", CR, "800.00")],
    );
    let e = b.post(
        d(2026, 2, 3),
        "Trade in",
        &[
            ("1520", DR, "2000.00"),
            ("1510", DR, "800.00"),
            ("1500", CR, "1000.00"),
            ("4900", CR, "300.00"),
            ("1000", CR, "1500.00"),
        ],
    );
    let cf = feb(&b, &[], &[]);
    assert_eq!(em(&cf.investing), [(e, x("-1500.00"))]);
    assert_eq!(disclosed(&cf), [(e, x("500.00"))]);
    assert_eq!(cf.noncash_disclosures[0].exchanged, x("500.00"));
}

/// D-35, ASC 230-10-45-13(c): a bill payment carries its bill's split,
/// equipment part first. A bill (from a later version, restored as the
/// journal holds it) for equipment 1,600 and supplies 100 on payables; it
/// is paid 1,000 in February and 700 in March. February: investing −1,000
/// (the equipment, paid soon after purchase), and the 600 still owed for
/// equipment disclosed (EY FRD 5.3.3: the part unpaid at the end). March:
/// investing −600 and operating −100. Payables' equipment part never
/// reaches the operating reconciliation (DART 6.1.3).
#[test]
fn a_bill_payment_carries_its_bills_split() {
    let mut b = funded();
    let entry = b.post(
        d(2026, 2, 2),
        "Bill 1",
        &[
            ("1500", DR, "1600.00"),
            ("6000", DR, "100.00"),
            ("2000", CR, "1700.00"),
        ],
    );
    let vendor = b.l.add_vendor("Acme LLC").unwrap();
    let bill = b.l.next_bill_id();
    b.l.replay_bill(
        bill,
        vendor,
        d(2026, 2, 2),
        d(2026, 3, 2),
        vec![ledger_core::DocumentLine {
            description: "Equipment and supplies".into(),
            amount: x("1700.00"),
            class_id: None,
        }],
        entry,
    )
    .unwrap();
    let cash = b.id("1000");
    let p1 =
        b.l.pay_bill(bill, d(2026, 2, 20), x("1000.00"), cash)
            .unwrap();
    let p2 =
        b.l.pay_bill(bill, d(2026, 3, 5), x("700.00"), cash)
            .unwrap();
    let cf = feb(&b, &[], &[]);
    assert_eq!(em(&cf.investing), [(p1, x("-1000.00"))]);
    assert!(cf.operating.is_empty());
    assert_eq!(disclosed(&cf), [(entry, x("600.00"))]);
    assert_eq!(
        recon(&cf),
        [(s("Increase in 2000 Accounts Payable"), x("100.00"))]
    );
    assert!(cf.warnings.is_empty());
    let march = b.cf(d(2026, 3, 1), d(2026, 3, 31), &["1000", "1010"], &[], &[]);
    foots(&march);
    assert_eq!(em(&march.investing), [(p2, x("-600.00"))]);
    assert_eq!(em(&march.operating), [(p2, x("-100.00"))]);
    // Paid in full within one period: no disclosure at all.
    let both = b.cf(d(2026, 2, 1), d(2026, 3, 31), &["1000", "1010"], &[], &[]);
    foots(&both);
    assert!(both.noncash_disclosures.is_empty());
    assert_eq!(both.cash_from_investing, x("-1600.00"));
}

/// D-35 (unchanged by D-42): a card purchase of equipment follows the
/// card's classification. On a card that is an operating payable:
/// disclosed (investing +1,000, operating −1,000), flagged, and its
/// payment is operating (d). On a card listed as financing, the issuer is
/// a lender that paid the seller (D-42): borrowed and invested on the
/// purchase's date (financing +1,000, investing −1,000), no flag, and its
/// payment a repayment of borrowing.
#[test]
fn a_card_purchase_follows_the_cards_classification() {
    let mut b = funded();
    let e = b.post(
        d(2026, 2, 3),
        "Equipment on the card",
        &[("1500", DR, "1000.00"), ("2050", CR, "1000.00")],
    );
    let pay = b.post(
        d(2026, 2, 25),
        "Pay the card",
        &[("2050", DR, "1000.00"), ("1000", CR, "1000.00")],
    );
    let cf = feb(&b, &[], &[]);
    assert_eq!(disclosed(&cf), [(e, x("1000.00"))]);
    assert_eq!(
        cf.warnings
            .iter()
            .map(|w| (w.entry, w.kind))
            .collect::<Vec<_>>(),
        [(e, CashFlowWarningKind::AssetOnOperatingPayable)]
    );
    assert_eq!(em(&cf.operating), [(pay, x("-1000.00"))]);
    let cf = feb(&b, &["2050"], &[]);
    assert!(cf.noncash_disclosures.is_empty());
    assert!(cf.warnings.is_empty());
    assert_eq!(em(&cf.investing), [(e, x("-1000.00"))]);
    assert_eq!(cf.investing[0].on_behalf, x("-1000.00"));
    assert_eq!(em(&cf.financing), [(e, x("1000.00")), (pay, x("-1000.00"))]);
    assert_eq!(
        kinds(&cf, CashFlowSection::Financing),
        [
            (s("Proceeds from borrowing — 2050 Card"), x("1000.00")),
            (s("Repayments of borrowing — 2050 Card"), x("-1000.00")),
        ]
    );
}

/// Kinds (D-35): each member's contributions and distributions are
/// listed under their own name, an entry crediting two members split
/// between them (each its own movement: no noncash part); equity not a
/// member's is named by account.
#[test]
fn financing_lists_each_members_contributions_and_distributions() {
    let mut b = funded();
    let both = b.post(
        d(2026, 2, 2),
        "Joint contribution",
        &[
            ("1000", DR, "600.00"),
            ("3000", CR, "400.00"),
            ("3001", CR, "200.00"),
        ],
    );
    let dist = b.post(
        d(2026, 2, 3),
        "Distribution to Member B",
        &[("3001", DR, "50.00"), ("1000", CR, "50.00")],
    );
    let cf = feb(&b, &[], &[]);
    assert_eq!(
        kinds(&cf, CashFlowSection::Financing),
        [
            (s("Contributions from members — Member A"), x("400.00")),
            (s("Contributions from members — Member B"), x("200.00")),
            (s("Distributions to members — Member B"), x("-50.00")),
        ]
    );
    assert_eq!(cf.kinds(CashFlowSection::Financing)[0].entries, [both]);
    assert_eq!(cf.kinds(CashFlowSection::Financing)[2].entries, [dist]);
    // No members configured: equity is named by account.
    let mut plain = b;
    plain.members.clear();
    let cf = feb(&plain, &[], &[]);
    assert_eq!(
        cf.financing[0].kind,
        "Contributions from owners — 3000 Capital — Member A"
    );
}

/// ASC 230-10-45-25 requires interest received (b) and interest paid (e)
/// "at a minimum" apart from the other classes, so an entry's operating
/// cash is classed kind by kind: its interest lines are (b) or (e)
/// whatever else it settles, the rest by its structure (rules.md B-14
/// rule 5). By hand:
/// - a payment of 900 on a loan the settings leave operating, with 100
///   interest (Dr 2500 900, Dr 6200 100 / Cr cash 1,000): (d) −900 and
///   (e) −100, not (e) −1,000;
/// - a customer's 1,000 collected with 10 interest (Dr cash 1,010 / Cr
///   1200 1,000, Cr 4100 10): (a) +1,000 and (b) +10, not (b) +1,010;
/// - the payment reversed in March: (d) +900 and (e) +100, its root's
///   classes with its own sign.
#[test]
fn operating_cash_is_classed_kind_by_kind() {
    let mut b = funded();
    let pay = b.post(
        d(2026, 2, 8),
        "Loan payment",
        &[
            ("2500", DR, "900.00"),
            ("6200", DR, "100.00"),
            ("1000", CR, "1000.00"),
        ],
    );
    let got = b.post(
        d(2026, 2, 9),
        "Customer pays, with interest",
        &[
            ("1000", DR, "1010.00"),
            ("1200", CR, "1000.00"),
            ("4100", CR, "10.00"),
        ],
    );
    let cf = feb(&b, &[], &[]);
    use OperatingClass::*;
    let lines: Vec<_> = cf
        .operating
        .iter()
        .map(|l| (l.entry, l.class, l.amount, l.accounts.clone()))
        .collect();
    assert_eq!(
        lines,
        [
            (got, Some(FromCustomers), x("1000.00"), vec![s("1200")]),
            (
                got,
                Some(InterestAndDividendsReceived),
                x("10.00"),
                vec![s("4100")]
            ),
            (
                pay,
                Some(ToSuppliersAndEmployees),
                x("-900.00"),
                vec![s("2500")]
            ),
            (pay, Some(InterestPaid), x("-100.00"), vec![s("6200")]),
        ]
    );
    assert!(cf.noncash_disclosures.is_empty());
    b.reverse(pay, d(2026, 3, 2));
    let march = b.cf(d(2026, 3, 1), d(2026, 3, 31), &["1000", "1010"], &[], &[]);
    foots(&march);
    assert_eq!(
        march.operating_by_class(),
        [
            (ToSuppliersAndEmployees, x("900.00")),
            (InterestPaid, x("100.00"))
        ]
    );
}

/// One line per entry, section and kind: two of an entry's operating
/// kinds that fall in one class are one line. A refund to a customer of
/// 20 with 10 of the interest it was charged (Dr 4000 20, Dr 4100 10 / Cr
/// cash 30): both parts are (g) other payments (cash refunds to customers,
/// 45-17; interest paid back out is neither (b) nor (e)), one line of −30
/// on both accounts.
#[test]
fn two_kinds_of_one_class_are_one_line() {
    let mut b = funded();
    let e = b.post(
        d(2026, 2, 14),
        "Refund, with the interest charged",
        &[
            ("4000", DR, "20.00"),
            ("4100", DR, "10.00"),
            ("1000", CR, "30.00"),
        ],
    );
    let cf = feb(&b, &[], &[]);
    let lines: Vec<_> = cf
        .operating
        .iter()
        .map(|l| (l.entry, l.class, l.amount, l.accounts.clone()))
        .collect();
    assert_eq!(
        lines,
        [(
            e,
            Some(OperatingClass::OtherPayments),
            x("-30.00"),
            vec![s("4000"), s("4100")]
        )]
    );
}

/// Interest is operating in any entry (ASC 230-10-45-17(d), 45-25(e)),
/// even one that disposes of equipment, where rule 4 makes the other
/// revenue and expense lines investing. Equipment (cost 1,000) handed to
/// the bank against 1,000 of the loan, with 50 interest paid in cash (Dr
/// 2500 1,000, Dr 6200 50 / Cr 1500 1,000, Cr cash 50): operating −50,
/// class (e); no investing cash; the equipment for the debt is an
/// exchange, disclosed (50-4): 1,000.
#[test]
fn interest_paid_in_a_disposal_stays_operating() {
    let mut b = funded();
    let e = b.post(
        d(2026, 2, 12),
        "Equipment to the bank, with interest",
        &[
            ("2500", DR, "1000.00"),
            ("6200", DR, "50.00"),
            ("1500", CR, "1000.00"),
            ("1000", CR, "50.00"),
        ],
    );
    let cf = feb(&b, &["2500"], &[]);
    assert_eq!(em(&cf.operating), [(e, x("-50.00"))]);
    assert_eq!(cf.operating[0].class, Some(OperatingClass::InterestPaid));
    assert!(cf.investing.is_empty() && cf.financing.is_empty());
    assert_eq!(disclosed(&cf), [(e, x("1000.00"))]);
    assert!(
        cf.operating_reconciliation
            .lines
            .iter()
            .all(|l| l.kind != ReconcilingKind::InvestingGainLoss)
    );
}

/// Rule 4 for an investing account: selling a 1,000 note receivable (an
/// `investing_accounts` account) for 1,200 (Dr cash 1,200 / Cr 1250 1,000,
/// Cr gain 200). Receipts from sales of other entities' debt instruments
/// are investing (ASC 230-10-45-12(a)), the whole proceeds: investing
/// +1,200, operating 0, the gain taken out of net income (45-28(b)): G
/// −200.
#[test]
fn an_investments_gain_is_investing() {
    let mut b = funded();
    b.post(
        d(2026, 1, 10),
        "Buy a note",
        &[("1250", DR, "1000.00"), ("1000", CR, "1000.00")],
    );
    let e = b.post(
        d(2026, 2, 10),
        "Sell the note",
        &[
            ("1000", DR, "1200.00"),
            ("1250", CR, "1000.00"),
            ("4900", CR, "200.00"),
        ],
    );
    let cf = feb(&b, &[], &["1250"]);
    assert_eq!(em(&cf.investing), [(e, x("1200.00"))]);
    assert_eq!(cf.investing[0].kind, "Collections on 1250 Note receivable");
    assert!(cf.operating.is_empty());
    assert_eq!(
        recon(&cf),
        [(
            s("(Gain) loss and other noncash changes in investing assets"),
            x("-200.00")
        )]
    );
}

/// Rule 6, three levels down: A ← B ← C ← D with A before February. B and
/// C pair (walking from the root); D, a reversal of the reinstatement, is
/// shown alone, labelled as the reversal it is ("Reversal of #C — …"),
/// never "Reinstatement", under A's kind with its own sign: −300.
#[test]
fn a_reversal_three_levels_down_is_labelled_a_reversal() {
    let mut b = funded();
    let a = b.post(
        d(2026, 1, 20),
        "Contribution — Member B",
        &[("1000", DR, "300.00"), ("3001", CR, "300.00")],
    );
    let r = b.reverse(a, d(2026, 2, 3));
    let c = b.reverse(r, d(2026, 2, 4));
    let dd = b.reverse(c, d(2026, 2, 5));
    let cf = feb(&b, &[], &[]);
    assert_eq!(em(&cf.financing), [(dd, x("-300.00"))]);
    let line = &cf.financing[0];
    assert_eq!(line.label, format!("Reversal of #{c} — Reverse entry {r}"));
    assert_eq!(line.kind, "Contributions from members — Member B");
    assert_eq!(line.reverses, Some(c));
}

/// Rule 3's CONVENTION, pinned: an entry's cash goes to the sections
/// moving against it in the order operating, investing, financing. Supplies
/// 100 and a vehicle 1,000 for 600 cash, Member A putting in the other 500
/// as capital (Dr 6000 100, Dr 1520 1,000 / Cr cash 600, Cr 3000 500):
/// operating takes its 100 first, investing the other 500; the vehicle's
/// 500 for capital is noncash (property for capital, rule 11: capital pays
/// constructively only for revenue and expense, and the supplies were
/// already paid in cash), disclosed (investing +500, financing −500). Had
/// the member's 500 been a loan through 2100 (financing), it would have
/// paid for the rest of the vehicle (D-42): investing −1,000, financing
/// +500.
#[test]
fn cash_goes_to_the_sections_in_order() {
    let mut b = funded();
    let e = b.post(
        d(2026, 2, 6),
        "Supplies and a vehicle, part contributed",
        &[
            ("6000", DR, "100.00"),
            ("1520", DR, "1000.00"),
            ("1000", CR, "600.00"),
            ("3000", CR, "500.00"),
        ],
    );
    let cf = feb(&b, &["2500"], &[]);
    assert_eq!(em(&cf.operating), [(e, x("-100.00"))]);
    assert_eq!(em(&cf.investing), [(e, x("-500.00"))]);
    assert!(cf.financing.is_empty());
    let n = &cf.noncash_disclosures[0];
    assert_eq!(
        (n.entry, n.operating, n.investing, n.financing, n.amount),
        (e, x("0"), x("500.00"), x("-500.00"), x("500.00"))
    );

    let mut b = funded();
    let e = b.post(
        d(2026, 2, 6),
        "Supplies and a vehicle, part lent by Member A",
        &[
            ("6000", DR, "100.00"),
            ("1520", DR, "1000.00"),
            ("1000", CR, "600.00"),
            ("2100", CR, "500.00"),
        ],
    );
    let cf = feb(&b, &["2100"], &[]);
    assert_eq!(em(&cf.operating), [(e, x("-100.00"))]);
    assert_eq!(em(&cf.investing), [(e, x("-1000.00"))]);
    assert_eq!(cf.investing[0].on_behalf, x("-500.00"));
    assert_eq!(em(&cf.financing), [(e, x("500.00"))]);
    assert!(cf.noncash_disclosures.is_empty());
}

/// Rule 5's CONVENTION, pinned: within a section the cash goes to its
/// kinds in kind order (members in their order, then other equity, then
/// borrowing). 500 cash and 100 of the loan repaid for 400 from Member A
/// and 200 from Member B (Dr cash 500, Dr 2500 100 / Cr 3000 400, Cr 3001
/// 200): Member A +400, Member B +100; Member B's other 100 went straight
/// to the bank, an exchange inside financing, disclosed (50-4): 100.
#[test]
fn cash_goes_to_a_sections_kinds_in_order() {
    let mut b = funded();
    let e = b.post(
        d(2026, 2, 6),
        "Contributions, part to the bank",
        &[
            ("1000", DR, "500.00"),
            ("2500", DR, "100.00"),
            ("3000", CR, "400.00"),
            ("3001", CR, "200.00"),
        ],
    );
    let cf = feb(&b, &["2500"], &[]);
    assert_eq!(
        kinds(&cf, CashFlowSection::Financing),
        [
            (s("Contributions from members — Member A"), x("400.00")),
            (s("Contributions from members — Member B"), x("100.00")),
        ]
    );
    assert_eq!(disclosed(&cf), [(e, x("100.00"))]);
    assert_eq!(cf.noncash_disclosures[0].exchanged, x("100.00"));
}

/// Rule 12: a bill payment reversed in the period unpays the bill again.
/// Equipment 1,000 on a bill (restored as a later version would journal
/// it), paid on Feb 20 and the payment reversed on Feb 21: the payment
/// and its reversal drop out, and the 1,000 still owed at the end of
/// February is disclosed (EY FRD 5.3.3: what remains unpaid).
#[test]
fn a_reversed_bill_payment_leaves_the_purchase_unpaid() {
    let mut b = funded();
    let entry = b.post(
        d(2026, 2, 2),
        "Bill 1",
        &[("1500", DR, "1000.00"), ("2000", CR, "1000.00")],
    );
    let vendor = b.l.add_vendor("Acme LLC").unwrap();
    let bill = b.l.next_bill_id();
    b.l.replay_bill(
        bill,
        vendor,
        d(2026, 2, 2),
        d(2026, 3, 2),
        vec![ledger_core::DocumentLine {
            description: "Equipment".into(),
            amount: x("1000.00"),
            class_id: None,
        }],
        entry,
    )
    .unwrap();
    let cash = b.id("1000");
    let pay =
        b.l.pay_bill(bill, d(2026, 2, 20), x("1000.00"), cash)
            .unwrap();
    let undo = b.reverse(pay, d(2026, 2, 21));
    let cf = feb(&b, &[], &[]);
    assert!(cf.investing.is_empty() && cf.operating.is_empty());
    assert_eq!(disclosed(&cf), [(entry, x("1000.00"))]);
    // Through the payment only (Feb 1-20): paid, nothing disclosed.
    let early = b.cf(FEB.0, d(2026, 2, 20), &["1000", "1010"], &[], &[]);
    foots(&early);
    assert_eq!(em(&early.investing), [(pay, x("-1000.00"))]);
    assert!(early.noncash_disclosures.is_empty());
    // The reversal alone (Feb 21 on): the payment's investing part, back.
    let late = b.cf(d(2026, 2, 21), FEB.1, &["1000", "1010"], &[], &[]);
    foots(&late);
    assert_eq!(em(&late.investing), [(undo, x("1000.00"))]);
}

// ---------------------------------------------------------------------------
// Role refusals (rules.md B-14 rule 2) and the footing check (rule 9).
// ---------------------------------------------------------------------------

fn refused(b: &Books, cash: &[&str], financing: &[&str], investing: &[&str]) -> String {
    match cash_flow(&b.l, &b.params(FEB.0, FEB.1, cash, financing, investing)) {
        Err(LedgerError::CashFlowRoles(m)) => m,
        other => panic!("expected a role refusal, got {other:?}"),
    }
}

/// An account belongs to one section: a code in two lists is refused,
/// naming both settings.
#[test]
fn role_lists_that_overlap_are_refused() {
    let b = funded();
    let m = refused(&b, &["1000"], &[], &["1000"]);
    assert!(m.contains("both cash_accounts"), "{m}");
    assert!(m.contains("books.investing_accounts"), "{m}");
    let m = refused(&b, &["1000", "1000"], &[], &[]);
    assert!(m.contains("1000 twice"), "{m}");
    let m = refused(&b, &[], &[], &[]);
    assert!(m.contains("no cash accounts"), "{m}");
}

/// ASC 230-10-45-14(b): financing liabilities are borrowing. An asset
/// listed as financing is refused, naming the setting; equity listed is
/// accepted (it is financing anyway).
#[test]
fn a_financing_account_must_be_a_liability() {
    let b = funded();
    let m = refused(&b, &["1000"], &["1500"], &[]);
    assert!(
        m.contains("financing_accounts (from the call or books.financing_accounts) lists 1500"),
        "{m}"
    );
    assert!(
        m.contains(
            "an asset account: financing accounts are liabilities (equity is always financing)"
        ),
        "{m}"
    );
    let with_equity = b.cf(FEB.0, FEB.1, &["1000"], &["3000", "2500"], &[]);
    foots(&with_equity);
}

/// ASC 230-10-45-12, -13: investing accounts are assets (loans made,
/// investments bought).
#[test]
fn an_investing_account_must_be_an_asset() {
    let m = refused(&funded(), &["1000"], &[], &["2500"]);
    assert!(
        m.contains("a liability account: investing accounts are asset accounts"),
        "{m}"
    );
}

/// Cash is an asset; the statement explains its change.
#[test]
fn a_cash_account_must_be_an_asset() {
    let m = refused(&funded(), &["2100"], &[], &[]);
    assert!(m.contains("cash accounts are asset accounts"), "{m}");
}

/// A registered fixed asset's accounts are investing, never cash.
#[test]
fn a_registered_asset_account_cannot_be_cash() {
    let m = refused(&funded(), &["1000", "1510"], &[], &[]);
    assert!(m.contains("a registered fixed asset's account"), "{m}");
}

/// rules.md B-14 rule 9: any difference between the sections and the
/// change in cash is an engine error, never a statement.
#[test]
fn foot_refuses_any_difference() {
    let ok = foot(x("1620.16"), x("1620.16")).unwrap();
    assert_eq!(ok.difference, Decimal::ZERO);
    for (sum, change) in [("-1980.32", "1620.16"), ("0.01", "0"), ("0", "-0.01")] {
        match foot(x(sum), x(change)) {
            Err(LedgerError::CashFlowDoesNotFoot {
                expected, found, ..
            }) => {
                assert_eq!((expected, found), (x(change), x(sum)));
            }
            other => panic!("expected an engine error, got {other:?}"),
        }
    }
    let msg = foot(x("1"), x("0")).unwrap_err().to_string();
    assert!(msg.starts_with("engine error"), "{msg}");
}

// ---------------------------------------------------------------------------
// Properties over random books (rules.md B-14; plan P3: 5,000 cases).
// ---------------------------------------------------------------------------

mod random {
    use super::*;
    use proptest::prelude::*;
    use std::collections::{BTreeMap, BTreeSet};

    /// One generated event: a posting from a template, or a reversal of an
    /// earlier entry (any earlier entry, reversals included: chains).
    #[derive(Clone, Debug)]
    pub enum Op {
        Post {
            day: u32,
            template: u8,
            amounts: [i64; 3],
            who: bool,
        },
        Reverse {
            of: usize,
            after: u32,
        },
    }

    /// The lines of a template, in cents. Every shape the rule names:
    /// contributions of cash and property, member-paid expenses and
    /// reimbursements, sales on account and collections, card spending
    /// and payments, asset purchases (cash, on a note, part cash),
    /// depreciation, disposals (cash, on account, gain, loss, before any
    /// depreciation, the loss on a revenue or an expense account), loans in
    /// and out with interest, transfers, debt to equity, a fee withheld;
    /// and what a member or lender pays or receives for the company (rule
    /// 11, D-42): an expense into capital, a prepaid through 2100, part
    /// cash and part a member's, a refund to a member, a customer paying a
    /// member, a member paying the card, revenue a member kept, and the
    /// shapes that stay noncash: interest charged into the loan, equipment
    /// on the seller's credit.
    pub fn template(t: u8, [a, b, c]: [i64; 3], who: bool) -> Vec<(&'static str, Side, i64)> {
        let cap = if who { "3000" } else { "3001" };
        let cash = if who { "1000" } else { "1010" };
        // A disposal: proceeds `a` to `to`, accumulated `b` (none if 0),
        // cost `c` + 1, the difference a gain or a loss.
        let dispose = |to: &'static str, accum: i64| {
            let cost = c + 1;
            let mut v = vec![(to, DR, a), ("1500", CR, cost)];
            if accum > 0 {
                v.push(("1510", DR, accum));
            }
            let gain = a + accum - cost;
            // A loss on a revenue-typed gain/loss account too (a debit
            // to revenue), as on an expense account.
            let loss_to = if who { "6900" } else { "4900" };
            if gain > 0 {
                v.push(("4900", CR, gain));
            } else if gain < 0 {
                v.push((loss_to, DR, -gain));
            }
            v
        };
        match t {
            0 => vec![(cash, DR, a), (cap, CR, a)],
            1 => vec![("1500", DR, a), (cap, CR, a)],
            2 => vec![("6000", DR, a), ("2100", CR, a)],
            3 => vec![("2100", DR, a), (cash, CR, a)],
            4 => vec![("1200", DR, a), ("4000", CR, a)],
            5 => vec![(cash, DR, a), ("1200", CR, a)],
            6 => vec![("6000", DR, a), ("2050", CR, a)],
            7 => vec![("2050", DR, a), (cash, CR, a)],
            8 => vec![("1520", DR, a), (cash, CR, a)],
            9 => vec![("1520", DR, a), ("2500", CR, a)],
            10 => vec![("1500", DR, a + b), (cash, CR, a), ("2500", CR, b)],
            11 => vec![("6100", DR, a), ("1510", CR, a)],
            12 => dispose(cash, b),
            13 => dispose("1210", b),
            14 => dispose(cash, 0),
            15 => vec![(cash, DR, a), ("2500", CR, a)],
            16 => vec![("2500", DR, a), ("6200", DR, b), (cash, CR, a + b)],
            17 => vec![("1400", DR, a), (cash, CR, a)],
            18 => vec![(cash, DR, a + b), ("1400", CR, a), ("4100", CR, b)],
            19 => vec![("1010", DR, a), ("1000", CR, a)],
            20 => vec![("2100", DR, a), (cap, CR, a)],
            21 => vec![(cash, DR, a), ("6000", DR, b), ("2500", CR, a + b)],
            22 => vec![(cash, DR, a), ("4000", CR, a)],
            24 => vec![("6000", DR, a), (cap, CR, a)],
            25 => vec![("1300", DR, a), ("2100", CR, a)],
            26 => vec![("6000", DR, a + b), (cash, CR, a), ("2100", CR, b)],
            27 => vec![("2100", DR, a), ("6000", CR, a)],
            28 => vec![("2100", DR, a), ("1200", CR, a)],
            29 => vec![("6200", DR, a), ("2500", CR, a)],
            30 => vec![("1500", DR, a), ("2000", CR, a)],
            31 => vec![("2050", DR, a), ("2100", CR, a)],
            32 => vec![(cap, DR, a), ("4000", CR, a)],
            _ => vec![("6000", DR, a), (cash, CR, a)],
        }
    }

    /// How many templates.
    pub const TEMPLATES: u8 = 33;

    /// Templates whose entries are pure: their cash settles every section
    /// they move (or they move no cash and no section), or a member or
    /// lender settles it all for the company (rule 11), whether 2100 is
    /// financing or not.
    const PURE: [u8; 21] = [
        0, 2, 3, 5, 7, 8, 12, 14, 15, 16, 17, 18, 19, 22, 24, 25, 26, 27, 28, 31, 32,
    ];

    fn arb_op(pure: bool) -> impl Strategy<Value = Op> {
        let template = if pure {
            prop::sample::select(PURE.to_vec()).boxed()
        } else {
            (0u8..TEMPLATES).boxed()
        };
        prop_oneof![
            4 => (0u32..365, template, [1i64..100_000, 1i64..100_000, 1i64..100_000], any::<bool>())
                .prop_map(|(day, template, amounts, who)| Op::Post { day, template, amounts, who }),
            1 => (any::<usize>(), 0u32..60).prop_map(|(of, after)| Op::Reverse { of, after }),
        ]
    }

    pub fn arb_ops(pure: bool) -> impl Strategy<Value = Vec<Op>> {
        prop::collection::vec(arb_op(pure), 1..30)
    }

    fn day(n: u32) -> NaiveDate {
        d(2026, 1, 1) + chrono::Days::new(u64::from(n.min(364)))
    }

    /// The model: each posted entry's date and lines, as generated (a
    /// reversal's are its original's, sides flipped), and whether it
    /// mirrors its chain's root (an odd number of reversals down).
    pub type Model = Vec<(EntryId, NaiveDate, Vec<(&'static str, Side, i64)>, bool)>;

    /// Post the ops to fresh books, amounts times `scale`.
    pub fn build(ops: &[Op], scale: i64) -> (Books, Model) {
        let mut b = co();
        let mut model: Model = Vec::new();
        let mut reversed = BTreeSet::new();
        for op in ops {
            match op {
                Op::Post {
                    day: n,
                    template: t,
                    amounts,
                    who,
                } => {
                    let lines = template(*t, *amounts, *who);
                    let text: Vec<(&str, Side, String)> = lines
                        .iter()
                        .map(|&(a, s, c)| (a, s, Decimal::new(c * scale, 2).to_string()))
                        .collect();
                    let refs: Vec<(&str, Side, &str)> =
                        text.iter().map(|(a, s, t)| (*a, *s, t.as_str())).collect();
                    let e = b.post(day(*n), "Event", &refs);
                    model.push((e, day(*n), lines, false));
                }
                Op::Reverse { of, after } => {
                    if model.is_empty() {
                        continue;
                    }
                    let (orig, date, lines, mirrored) = model[of % model.len()].clone();
                    if !reversed.insert(orig) {
                        continue;
                    }
                    let when = date + chrono::Days::new(u64::from(*after));
                    let when = when.min(d(2026, 12, 31));
                    let r = b.reverse(orig, when);
                    let flipped = lines.iter().map(|&(a, s, c)| (a, flip(s), c)).collect();
                    model.push((r, when, flipped, !mirrored));
                }
            }
        }
        (b, model)
    }

    pub const CASH: [&str; 2] = ["1000", "1010"];

    pub fn params(b: &Books, a: NaiveDate, e: NaiveDate, designate: bool) -> CashFlowParams {
        let fin: &[&str] = if designate { &["2100", "2500"] } else { &[] };
        b.params(a, e, &CASH, fin, &["1250", "1400"])
    }

    fn flip(s: Side) -> Side {
        if s == DR { CR } else { DR }
    }

    fn signed(s: Side, c: i64) -> i64 {
        if s == DR { c } else { -c }
    }

    /// The model's change in cash over [a, e]: the plain sum of the
    /// generated cash lines.
    pub fn model_cash(model: &Model, a: NaiveDate, e: NaiveDate) -> Decimal {
        let cents: i64 = model
            .iter()
            .filter(|(_, d, _, _)| *d >= a && *d <= e)
            .flat_map(|(_, _, ls, _)| ls)
            .filter(|(acct, _, _)| CASH.contains(acct))
            .map(|&(_, s, c)| signed(s, c))
            .sum();
        Decimal::new(cents, 2)
    }

    /// The model's section of an account, for pure books: gains and
    /// losses appear only in disposals, so they are investing.
    fn model_section(acct: &str, designate: bool) -> Option<usize> {
        match acct {
            "1000" | "1010" => None,
            "1500" | "1510" | "1520" | "1530" | "1250" | "1400" | "4900" | "6900" => Some(1),
            "3000" | "3001" => Some(2),
            "2100" | "2500" if designate => Some(2),
            _ => Some(0),
        }
    }

    /// Rule 11 for a whole entry, independently of the engine: no cash
    /// line, and every line either a source (2100 or 2500 when financing;
    /// a member's capital) or a use on the other side; capital pays or
    /// receives only revenue and expense; a liability pays for any
    /// operating or investing account and receives only revenue, expense
    /// or receivables; interest never.
    pub fn constructive_whole(lines: &[(&str, Side, i64)], designate: bool) -> bool {
        if lines.iter().any(|(acct, _, _)| CASH.contains(acct)) {
            return false;
        }
        let lender = |a: &str| designate && matches!(a, "2100" | "2500");
        let member = |a: &str| matches!(a, "3000" | "3001");
        type Lines<'a, 'b> = Vec<&'a (&'b str, Side, i64)>;
        let (src, uses): (Lines, Lines) =
            lines.iter().partition(|(a, _, _)| lender(a) || member(a));
        let (Some(first), false) = (src.first(), uses.is_empty()) else {
            return false;
        };
        let side = first.1;
        if src.iter().any(|l| l.1 != side) || uses.iter().any(|l| l.1 == side) {
            return false;
        }
        let pl = |a: &str| matches!(a, "4000" | "6000");
        if member(first.0) {
            src.iter().all(|l| member(l.0)) && uses.iter().all(|l| pl(l.0))
        } else if side == CR {
            uses.iter().all(|l| {
                model_section(l.0, designate) != Some(2) && !matches!(l.0, "6200" | "4100")
            })
        } else {
            uses.iter().all(|l| pl(l.0) || l.0 == "1200")
        }
    }

    /// For pure books, each section's total is minus the movement of its
    /// accounts over the entries that touch cash (plan P3's wording), or
    /// that a member or lender settled whole for the company (rule 11).
    pub fn model_sections(
        model: &Model,
        a: NaiveDate,
        e: NaiveDate,
        designate: bool,
    ) -> [Decimal; 3] {
        let mut out = [0i64; 3];
        for (_, d, lines, mirrored) in model.iter().filter(|(_, d, _, _)| *d >= a && *d <= e) {
            let _ = d;
            // A reversal mirrors its root (rule 6): judged by the root's
            // shape, its lines flipped back.
            let root: Vec<(&str, Side, i64)> = lines
                .iter()
                .map(|&(acct, side, c)| (acct, if *mirrored { flip(side) } else { side }, c))
                .collect();
            if !lines.iter().any(|(acct, _, _)| CASH.contains(acct))
                && !constructive_whole(&root, designate)
            {
                continue;
            }
            for &(acct, s, c) in lines {
                if let Some(k) = model_section(acct, designate) {
                    out[k] -= signed(s, c);
                }
            }
        }
        out.map(|c| Decimal::new(c, 2))
    }

    /// Every figure of a statement, scaled.
    pub fn scaled(cf: &CashFlowStatement, k: i64) -> CashFlowStatement {
        let k = Decimal::from(k);
        let mut out = cf.clone();
        for sec in [&mut out.operating, &mut out.investing, &mut out.financing] {
            for l in sec.iter_mut() {
                l.amount *= k;
                l.on_behalf *= k;
            }
        }
        out.cash_from_operations *= k;
        out.cash_from_investing *= k;
        out.cash_from_financing *= k;
        out.net_change_in_cash *= k;
        out.cash_beginning *= k;
        out.cash_ending *= k;
        for n in &mut out.noncash_disclosures {
            n.amount *= k;
            n.operating *= k;
            n.investing *= k;
            n.financing *= k;
            n.exchanged *= k;
        }
        let r = &mut out.operating_reconciliation;
        r.net_income *= k;
        r.net_cash_from_operating *= k;
        for l in &mut r.lines {
            l.amount *= k;
        }
        out.reconciles.sections_sum *= k;
        out.reconciles.net_change *= k;
        out.reconciles.difference *= k;
        out
    }

    /// The cash net of a model entry.
    pub fn entry_cash(model: &Model) -> BTreeMap<EntryId, i64> {
        model
            .iter()
            .map(|(id, _, ls, _)| {
                let c = ls
                    .iter()
                    .filter(|(a, _, _)| CASH.contains(a))
                    .map(|&(_, s, c)| signed(s, c))
                    .sum();
                (*id, c)
            })
            .collect()
    }

    /// Normalize amounts' scale so scaled statements compare equal.
    pub fn norm(cf: &CashFlowStatement) -> String {
        let text = format!("{cf:?}");
        // Decimals print with their scale; normalize "1.50" and "1.5".
        let mut out = String::new();
        let mut num = String::new();
        let flush = |num: &mut String, out: &mut String| {
            if !num.is_empty() {
                match num.parse::<Decimal>() {
                    Ok(d) if num.contains('.') => out.push_str(&d.normalize().to_string()),
                    _ => out.push_str(num),
                }
                num.clear();
            }
        };
        for ch in text.chars() {
            if ch.is_ascii_digit() || ch == '.' || (ch == '-' && num.is_empty()) {
                num.push(ch);
            } else {
                flush(&mut num, &mut out);
                out.push(ch);
            }
        }
        flush(&mut num, &mut out);
        out
    }
}

/// How many random books: `CF_CASES` (default 5,000, the plan's count).
fn cases() -> u32 {
    std::env::var("CF_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5000)
}

use proptest::prelude::*;
use random::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases()))]

    /// rules.md B-14 over random books, reversals of reversals included:
    /// 1. the sections sum to the change in cash, which is the model's;
    /// 2. each section is its lines and its kinds, and 3. net income and
    ///    the reconciling items are operating (`foots`);
    /// 4. every line's entry moved cash, and every disclosure's amount is
    ///    the larger of its noncash parts' positive sum and its exchange;
    /// 5. posting an entry and its reversal inside the period changes
    ///    nothing;
    /// 6. [a, m] and (m, e] add up to [a, e], section by section;
    /// 7. closing every month changes nothing;
    /// 8. scaling every amount scales every figure.
    #[test]
    fn prop_cash_flow_laws(
        ops in arb_ops(false),
        extra in (0u8..TEMPLATES, [1i64..100_000, 1i64..100_000, 1i64..100_000], any::<bool>()),
        (a, m, e) in (0u32..365, 0u32..365, 0u32..365),
        designate in any::<bool>(),
        k in 2i64..=5,
    ) {
        let mut days = [a, m, e];
        days.sort_unstable();
        let day = |n: u32| d(2026, 1, 1) + chrono::Days::new(u64::from(n));
        let (a, m, e) = (day(days[0]), day(days[1]), day(days[2]));
        let (b, model) = build(&ops, 1);
        let cf = cash_flow(&b.l, &params(&b, a, e, designate)).unwrap();
        foots(&cf);
        prop_assert_eq!(cf.net_change_in_cash, model_cash(&model, a, e));
        // 4: an entry's lines are its cash (what a member or lender
        // handled for it nets to zero), each line's cash part has the
        // entry's sign, and an entry no cash moved shows only what was
        // handled for it.
        let cash_of = entry_cash(&model);
        let mut shown: std::collections::BTreeMap<EntryId, (Decimal, Decimal)> =
            std::collections::BTreeMap::new();
        for sec in CashFlowSection::ALL {
            for l in cf.lines(sec) {
                prop_assert!(!l.amount.is_zero());
                let c = Decimal::new(cash_of[&l.entry], 2);
                let real = l.amount - l.on_behalf;
                prop_assert!(real.is_zero() || (real > Decimal::ZERO) == (c > Decimal::ZERO), "{:?}", l);
                prop_assert!(l.on_behalf.is_zero() || (l.on_behalf > Decimal::ZERO) == (l.amount > Decimal::ZERO), "{:?}", l);
                let t = shown.entry(l.entry).or_default();
                t.0 += l.amount;
                t.1 += l.on_behalf;
            }
        }
        for (entry, (amount, on_behalf)) in &shown {
            prop_assert_eq!(*amount, Decimal::new(cash_of[entry], 2), "entry #{}", entry);
            prop_assert_eq!(*on_behalf, Decimal::ZERO, "entry #{}", entry);
        }
        if !designate {
            // Only capital can then pay for the company, and only revenue
            // and expense: no investing line is handled for it.
            prop_assert!(cf.investing.iter().all(|l| l.on_behalf.is_zero()));
        }
        for n in &cf.noncash_disclosures {
            let moved: Decimal = [n.operating, n.investing, n.financing]
                .iter()
                .filter(|x| **x > Decimal::ZERO)
                .sum();
            prop_assert_eq!(n.amount, moved.max(n.exchanged));
            prop_assert!(n.amount > Decimal::ZERO);
            prop_assert_eq!(n.operating + n.investing + n.financing, Decimal::ZERO);
        }

        // 5: an entry and its reversal inside the period.
        let mut more = ops.clone();
        more.push(Op::Post { day: days[0], template: extra.0, amounts: extra.1, who: extra.2 });
        let (mut with, _) = build(&more, 1);
        let last = with.l.next_entry_id() - 1;
        with.reverse(last, a.max(with.l.get_entry(last).unwrap().date()).min(e));
        let cf5 = cash_flow(&with.l, &params(&with, a, e, designate)).unwrap();
        foots(&cf5);
        prop_assert_eq!(&cf5, &cf);

        // 6: split at m.
        let p1 = cash_flow(&b.l, &params(&b, a, m, designate)).unwrap();
        if m < e {
            let p2 = cash_flow(&b.l, &params(&b, m + chrono::Days::new(1), e, designate)).unwrap();
            for sec in CashFlowSection::ALL {
                prop_assert_eq!(p1.total(sec) + p2.total(sec), cf.total(sec));
            }
            prop_assert_eq!(p1.cash_beginning, cf.cash_beginning);
            prop_assert_eq!(p2.cash_ending, cf.cash_ending);
        }

        // 7: closing every month changes nothing.
        let (mut closed, _) = build(&ops, 1);
        for month in 1..=12u32 {
            let start = d(2026, month, 1);
            let end = if month == 12 { d(2026, 12, 31) } else { d(2026, month + 1, 1) - chrono::Days::new(1) };
            let p = closed.l.create_period(&format!("2026-{month:02}"), start, end).unwrap();
            closed.l.close_period(p).unwrap();
        }
        let cf7 = cash_flow(&closed.l, &params(&closed, a, e, designate)).unwrap();
        prop_assert_eq!(norm(&cf7), norm(&cf));

        // 8: scaling.
        let (big, _) = build(&ops, k);
        let cf8 = cash_flow(&big.l, &params(&big, a, e, designate)).unwrap();
        prop_assert_eq!(norm(&cf8), norm(&scaled(&cf, k)));
    }

    /// Plan P3's original wording, for books of pure entries only (each
    /// entry's cash settles every section it moves; review item 7): each
    /// section's total is minus the movement of the section's accounts
    /// over the entries that touch cash, by an independent model of the
    /// sections.
    #[test]
    fn prop_pure_entries_sections_are_movements(
        ops in arb_ops(true),
        (a, e) in (0u32..365, 0u32..365),
        designate in any::<bool>(),
    ) {
        let day = |n: u32| d(2026, 1, 1) + chrono::Days::new(u64::from(n));
        let (a, e) = (day(a.min(e)), day(a.max(e)));
        let (b, model) = build(&ops, 1);
        let cf = cash_flow(&b.l, &params(&b, a, e, designate)).unwrap();
        foots(&cf);
        let want = model_sections(&model, a, e, designate);
        for (sec, w) in CashFlowSection::ALL.iter().zip(want) {
            prop_assert_eq!(cf.total(*sec), w, "{:?}", sec);
        }
        // A pure entry's cash settles all it moves: nothing is noncash.
        prop_assert!(cf.noncash_disclosures.is_empty());
    }
}

/// Rule 2 (P3 seam review): a distribution declared to a liability that
/// holds only distributions (Dr capital / Cr 2400 Distributions Payable)
/// and paid later is a distribution paid, financing (ASC
/// 230-10-45-15(a)), under the payable's kind; the declaration is an
/// exchange inside financing, disclosed (50-3, 50-4). Before the fix the
/// payment was "Cash paid to suppliers and employees" and the declaration
/// an "Other noncash items" line beside an increase in the payable.
#[test]
fn a_distribution_declared_then_paid_is_a_distribution_paid() {
    let mut b = funded();
    b.acct("2400", "Distributions payable", AccountType::Liability);
    let declared = b.post(
        d(2026, 1, 31),
        "Distribution declared",
        &[
            ("3000", DR, "600.00"),
            ("3001", DR, "400.00"),
            ("2400", CR, "1000.00"),
        ],
    );
    let paid = b.post(
        d(2026, 2, 10),
        "Distribution paid",
        &[("2400", DR, "1000.00"), ("1000", CR, "1000.00")],
    );
    let cf = feb(&b, &[], &[]);
    assert!(cf.operating.is_empty(), "{:?}", cf.operating);
    assert_eq!(
        kinds(&cf, CashFlowSection::Financing),
        [(
            s("Distributions to members — 2400 Distributions payable"),
            x("-1000.00")
        )]
    );
    assert_eq!(em(&cf.financing), [(paid, x("-1000.00"))]);
    assert!(recon(&cf).is_empty(), "{:?}", recon(&cf));
    // The declaration's month: no cash, the exchange disclosed, and no
    // operating item.
    let jan = b.cf(d(2026, 1, 1), d(2026, 1, 31), &["1000", "1010"], &[], &[]);
    foots(&jan);
    assert_eq!(disclosed(&jan), [(declared, x("1000.00"))]);
    assert!(
        recon(&jan).is_empty(),
        "{:?}",
        jan.operating_reconciliation.lines
    );
    // A liability credited for anything else as well is no distributions
    // payable: an expense accrued to it keeps it operating.
    b.post(
        d(2026, 2, 20),
        "Accrued supplies",
        &[("6000", DR, "50.00"), ("2400", CR, "50.00")],
    );
    let cf = feb(&b, &[], &[]);
    assert_eq!(
        cf.operating.iter().map(|l| l.entry).collect::<Vec<_>>(),
        [paid]
    );
}

/// Rule 8 (P3 seam review): a disposal's lines on the asset's
/// depreciation expense account (the days a month's run posted past the
/// disposal, credited back) are depreciation on the reconciliation, not
/// gain or loss, so its depreciation line is the P&L's depreciation
/// expense.
#[test]
fn a_disposals_depreciation_credit_back_is_depreciation() {
    let mut b = funded();
    b.post(
        d(2026, 1, 10),
        "Buy equipment",
        &[("1500", DR, "1000.00"), ("1000", CR, "1000.00")],
    );
    b.post(
        d(2026, 2, 28),
        "Depreciation",
        &[("6100", DR, "30.00"), ("1510", CR, "30.00")],
    );
    // Sold Feb 14: 15 of the month's 30.00 taken back, a 10.00 loss on
    // the 985.00 book value.
    b.post(
        d(2026, 2, 14),
        "Sell equipment",
        &[
            ("1000", DR, "975.00"),
            ("1510", DR, "30.00"),
            ("6900", DR, "10.00"),
            ("1500", CR, "1000.00"),
            ("6100", CR, "15.00"),
        ],
    );
    let cf = feb(&b, &[], &[]);
    assert_eq!(
        recon(&cf),
        [
            (s("Depreciation"), x("15.00")),
            (
                s("(Gain) loss and other noncash changes in investing assets"),
                x("10.00")
            ),
        ]
    );
    assert_eq!(cf.cash_from_investing, x("975.00"));
}
