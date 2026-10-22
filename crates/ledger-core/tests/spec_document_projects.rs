//! An invoice's or bill's lines name their projects (rules.md B-37 item
//! 8): the posted entry's revenue or expense side is one line per project,
//! the untagged lines' as one; the receivable or payable line, and the
//! payments, never take one.
//!
//! ```text
//! GOAL: a client project's invoiced revenue and a contractor's billed
//!       cost count toward the project, every other figure as before.
//! SOURCE: docs/spec/rules.md B-37 items 3 and 8 (job costing), B-14 (the
//!         projects with the untagged lines partition the P&L), B-19 and
//!         B-35 (1099 and party figures follow the money, not the tags).
//! ```

use chrono::NaiveDate;
use ledger_core::arap::{DocumentLine, document_parts};
use ledger_core::form1099::{PaymentAccounts, vendor_1099_report};
use ledger_core::parties::{MoneyAccounts, activity};
use ledger_core::reports::profit_and_loss_by_class;
use ledger_core::{
    AccountType, ClassId, Currency, Ledger, LedgerError, ProjectFacts, ProjectStatus, Side,
    project_entries, project_to_date,
};
use proptest::prelude::*;
use rust_decimal::Decimal;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn cents(c: i64) -> Decimal {
    Decimal::new(c, 2)
}

fn line(what: &str, c: i64, class: Option<ClassId>) -> DocumentLine {
    DocumentLine {
        description: what.into(),
        amount: cents(c),
        class_id: class,
    }
}

/// Cash 1000 and two projects, Zeta (client) and Harbor.
struct Books {
    l: Ledger,
    cash: u64,
    zeta: ClassId,
    harbor: ClassId,
}

fn books() -> Books {
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let zeta = l.add_class("Zeta").unwrap();
    let harbor = l.add_class("Harbor").unwrap();
    Books {
        l,
        cash,
        zeta,
        harbor,
    }
}

/// Each line of `entry`: (code, side, amount, class).
fn lines_of(l: &Ledger, entry: u64) -> Vec<(String, Side, Decimal, Option<ClassId>)> {
    l.get_entry(entry)
        .unwrap()
        .lines()
        .iter()
        .map(|x| {
            (
                l.chart().get(x.account).unwrap().code.clone(),
                x.side,
                x.amount.amount,
                x.class_id,
            )
        })
        .collect()
}

#[test]
fn an_invoices_revenue_is_one_line_per_project_and_the_receivable_none() {
    let mut b = books();
    let (z, h) = (b.zeta, b.harbor);
    let c = b.l.add_customer("Cedar").unwrap();
    let inv =
        b.l.create_invoice(
            c,
            d(2026, 3, 2),
            d(2026, 4, 1),
            vec![
                line("Design", 300_000, Some(z)),
                line("Hosting", 20_000, Some(h)),
                line("Rush fee", 10_000, None),
                line("Revisions", 50_000, Some(z)),
            ],
        )
        .unwrap();
    let e = b.l.get_invoice(inv).unwrap().entry_id;
    assert_eq!(
        lines_of(&b.l, e),
        vec![
            ("1200".into(), Side::Debit, cents(380_000), None),
            ("4000".into(), Side::Credit, cents(350_000), Some(z)),
            ("4000".into(), Side::Credit, cents(20_000), Some(h)),
            ("4000".into(), Side::Credit, cents(10_000), None),
        ]
    );
    // The invoice keeps its lines as given, projects and all.
    let kept = &b.l.get_invoice(inv).unwrap().lines;
    assert_eq!(kept[3].class_id, Some(z));
    assert_eq!(
        b.l.get_invoice(inv).unwrap().total().unwrap(),
        cents(380_000)
    );
    // Its revenue counts toward each project, to the cent.
    let zt = project_to_date(&b.l, z, d(2026, 12, 31)).unwrap();
    assert_eq!((zt.revenue, zt.expenses), (cents(350_000), Decimal::ZERO));
    assert_eq!(
        project_to_date(&b.l, h, d(2026, 12, 31)).unwrap().revenue,
        cents(20_000)
    );
    assert_eq!(project_entries(&b.l, z), vec![(e, cents(350_000))]);
    // The payment is balance-sheet only: no project, no project figure.
    let paid =
        b.l.receive_payment(inv, d(2026, 3, 20), cents(380_000), b.cash)
            .unwrap();
    assert!(lines_of(&b.l, paid).iter().all(|l| l.3.is_none()));
    assert_eq!(project_to_date(&b.l, z, d(2026, 12, 31)).unwrap(), zt);
}

#[test]
fn a_bills_cost_is_one_line_per_project_and_the_payable_none() {
    let mut b = books();
    let (z, h) = (b.zeta, b.harbor);
    let v = b.l.add_vendor("Dana Illustration").unwrap();
    let bill =
        b.l.create_bill(
            v,
            d(2026, 3, 9),
            d(2026, 4, 8),
            vec![
                line("Office supplies", 5_000, None),
                line("Illustration", 120_000, Some(z)),
                line("Icons", 30_000, Some(h)),
            ],
        )
        .unwrap();
    let e = b.l.get_bill(bill).unwrap().entry_id;
    assert_eq!(
        lines_of(&b.l, e),
        vec![
            ("6999".into(), Side::Debit, cents(5_000), None),
            ("6999".into(), Side::Debit, cents(120_000), Some(z)),
            ("6999".into(), Side::Debit, cents(30_000), Some(h)),
            ("2000".into(), Side::Credit, cents(155_000), None),
        ]
    );
    let zt = project_to_date(&b.l, z, d(2026, 12, 31)).unwrap();
    assert_eq!((zt.revenue, zt.expenses), (Decimal::ZERO, cents(120_000)));
    let paid =
        b.l.pay_bill(bill, d(2026, 3, 30), cents(155_000), b.cash)
            .unwrap();
    assert!(lines_of(&b.l, paid).iter().all(|l| l.3.is_none()));
    assert_eq!(project_to_date(&b.l, z, d(2026, 12, 31)).unwrap(), zt);
}

/// Reversing a tagged invoice's or bill's entry carries each line's
/// project to the reversal (B-37 item 3: each line on its normal side), so
/// every project nets back to zero; a partial payment, cash against the
/// receivable, moves no project figure.
#[test]
fn reversing_a_tagged_document_takes_its_projects_to_zero() {
    let mut b = books();
    let (z, h) = (b.zeta, b.harbor);
    let c = b.l.add_customer("Cedar").unwrap();
    let v = b.l.add_vendor("Dana").unwrap();
    let inv =
        b.l.create_invoice(
            c,
            d(2026, 3, 2),
            d(2026, 4, 1),
            vec![
                line("Design", 300_000, Some(z)),
                line("Hosting", 20_000, Some(h)),
                line("Fee", 10_000, None),
            ],
        )
        .unwrap();
    let bill =
        b.l.create_bill(
            v,
            d(2026, 3, 9),
            d(2026, 4, 8),
            vec![line("Illustration", 120_000, Some(z))],
        )
        .unwrap();
    let to_date = |l: &Ledger, p| {
        let t = project_to_date(l, p, d(2026, 12, 31)).unwrap();
        (t.revenue, t.expenses)
    };
    assert_eq!(to_date(&b.l, z), (cents(300_000), cents(120_000)));
    b.l.receive_payment(inv, d(2026, 3, 20), cents(100_000), b.cash)
        .unwrap();
    assert_eq!(to_date(&b.l, z), (cents(300_000), cents(120_000)));
    assert_eq!(to_date(&b.l, h), (cents(20_000), Decimal::ZERO));
    for e in [
        b.l.get_invoice(inv).unwrap().entry_id,
        b.l.get_bill(bill).unwrap().entry_id,
    ] {
        let r = b.l.reverse_entry(e, d(2026, 3, 31), "Reverse").unwrap();
        let original: Vec<_> = lines_of(&b.l, e).into_iter().map(|l| l.3).collect();
        let reversed: Vec<_> = lines_of(&b.l, r).into_iter().map(|l| l.3).collect();
        assert_eq!(reversed, original, "entry {e}");
    }
    // Each line on its normal side: a reversed revenue line is a debit to
    // revenue, so the project's revenue and expenses are both zero.
    assert_eq!(to_date(&b.l, z), (Decimal::ZERO, Decimal::ZERO));
    assert_eq!(to_date(&b.l, h), (Decimal::ZERO, Decimal::ZERO));
    let by_class = profit_and_loss_by_class(&b.l).unwrap();
    assert!(!by_class.is_empty());
    for r in &by_class {
        assert_eq!(r.profit_and_loss.net_income(), Decimal::ZERO, "{r:?}");
    }
}

/// A document naming no project posts exactly what every document posted
/// before projects: one revenue (or expense) line for its total.
#[test]
fn an_untagged_document_posts_as_before() {
    let mut b = books();
    let c = b.l.add_customer("Cedar").unwrap();
    let v = b.l.add_vendor("Dana").unwrap();
    let inv =
        b.l.create_invoice(
            c,
            d(2026, 1, 5),
            d(2026, 2, 4),
            vec![line("A", 10_000, None), line("B", 2_550, None)],
        )
        .unwrap();
    let bill =
        b.l.create_bill(v, d(2026, 1, 6), d(2026, 2, 5), vec![line("C", 999, None)])
            .unwrap();
    assert_eq!(
        lines_of(&b.l, b.l.get_invoice(inv).unwrap().entry_id),
        vec![
            ("1200".into(), Side::Debit, cents(12_550), None),
            ("4000".into(), Side::Credit, cents(12_550), None),
        ]
    );
    assert_eq!(
        lines_of(&b.l, b.l.get_bill(bill).unwrap().entry_id),
        vec![
            ("6999".into(), Side::Debit, cents(999), None),
            ("2000".into(), Side::Credit, cents(999), None),
        ]
    );
}

#[test]
fn a_line_naming_an_unknown_project_is_refused_whole() {
    let mut b = books();
    let c = b.l.add_customer("Cedar").unwrap();
    let v = b.l.add_vendor("Dana").unwrap();
    let before = b.l.clone();
    assert_eq!(
        b.l.create_invoice(
            c,
            d(2026, 1, 5),
            d(2026, 2, 4),
            vec![line("A", 100, Some(b.zeta)), line("B", 100, Some(77))],
        ),
        Err(LedgerError::UnknownClass(77))
    );
    assert_eq!(
        b.l.create_bill(
            v,
            d(2026, 1, 5),
            d(2026, 2, 4),
            vec![line("A", 100, Some(78))]
        ),
        Err(LedgerError::UnknownClass(78))
    );
    assert!(b.l.differs_from(&before).is_empty(), "nothing changed");
}

/// Finished and abandoned projects still take a document's lines (B-37
/// item 5: a late cost, a final invoice).
#[test]
fn a_finished_project_still_takes_an_invoice() {
    let mut b = books();
    b.l.set_project(
        b.zeta,
        "Zeta",
        ProjectFacts {
            status: ProjectStatus::Finished,
            started_on: Some(d(2026, 1, 1)),
            ended_on: Some(d(2026, 2, 28)),
            ..Default::default()
        },
    )
    .unwrap();
    let c = b.l.add_customer("Cedar").unwrap();
    b.l.create_invoice(
        c,
        d(2026, 3, 15),
        d(2026, 4, 14),
        vec![line("Final", 40_000, Some(b.zeta))],
    )
    .unwrap();
    assert_eq!(
        project_to_date(&b.l, b.zeta, d(2026, 12, 31))
            .unwrap()
            .revenue,
        cents(40_000)
    );
}

/// The same books twice, one with its documents' lines tagged: every
/// figure that isn't a project's is the same (the P&L, the 1099
/// worksheet, a party's totals), and the tagged books' projects with the
/// untagged lines partition the P&L.
#[test]
fn tags_move_only_the_project_figures() {
    let build = |tagged: bool| {
        let mut b = books();
        let tag = |c: ClassId| tagged.then_some(c);
        let (z, h, cash) = (b.zeta, b.harbor, b.cash);
        let c = b.l.add_customer("Cedar").unwrap();
        let v = b.l.add_vendor("Dana").unwrap();
        b.l.set_vendor_1099(
            v,
            ledger_core::form1099::Dated1099::from_beginning(
                ledger_core::form1099::Decision1099::Files(ledger_core::form1099::Form1099::Nec),
                None,
            ),
            false,
        )
        .unwrap();
        let inv =
            b.l.create_invoice(
                c,
                d(2026, 2, 1),
                d(2026, 3, 3),
                vec![
                    line("Design", 500_000, tag(z)),
                    line("App", 80_000, tag(h)),
                    line("Fee", 1_000, None),
                ],
            )
            .unwrap();
        b.l.receive_payment(inv, d(2026, 2, 20), cents(581_000), cash)
            .unwrap();
        let bill =
            b.l.create_bill(
                v,
                d(2026, 2, 5),
                d(2026, 3, 7),
                vec![
                    line("Illustration", 180_000, tag(z)),
                    line("Misc", 2_500, None),
                ],
            )
            .unwrap();
        b.l.pay_bill(bill, d(2026, 2, 25), cents(182_500), cash)
            .unwrap();
        (b, c, v)
    };
    let (plain, _, v) = build(false);
    let (tagged, _, _) = build(true);
    let year = (d(2026, 1, 1), d(2026, 12, 31));
    let pnl = |l: &Ledger| ledger_core::profit_and_loss_for_period(l, year.0, year.1, None);
    assert_eq!(
        format!("{:?}", pnl(&plain.l).unwrap()),
        format!("{:?}", pnl(&tagged.l).unwrap())
    );
    let sources = PaymentAccounts {
        cash: vec![plain.cash],
        financing: vec![],
    };
    let w = |l: &Ledger| {
        format!(
            "{:?}",
            vendor_1099_report(l, 2026, None, &sources, &[], None)
        )
    };
    assert_eq!(w(&plain.l), w(&tagged.l));
    assert!(w(&tagged.l).contains("1825.00"), "{}", w(&tagged.l));
    let money = MoneyAccounts {
        cash: [plain.cash].into_iter().collect(),
        liabilities: Default::default(),
    };
    let party = plain.l.party_of_vendor(v).unwrap();
    let a = |l: &Ledger| format!("{:?}", activity(l, party, year, &money));
    assert_eq!(a(&plain.l), a(&tagged.l));
    // The tagged books: Zeta 5,000.00 - 1,800.00, Harbor 800.00, and the
    // untagged lines 10.00 - 25.00; together the P&L's net.
    let rows = profit_and_loss_by_class(&tagged.l).unwrap();
    let net = |name: Option<&str>| {
        let r = rows
            .iter()
            .find(|r| r.class.as_ref().map(|c| c.name.as_str()) == name)
            .unwrap();
        r.profit_and_loss.net_income()
    };
    assert_eq!(net(Some("Zeta")), cents(320_000));
    assert_eq!(net(Some("Harbor")), cents(80_000));
    assert_eq!(net(None), cents(-1_500));
    let whole = pnl(&tagged.l).unwrap();
    assert_eq!(cents(320_000 + 80_000 - 1_500), whole.net_income());
    let z = project_to_date(&tagged.l, tagged.zeta, year.1).unwrap();
    assert_eq!((z.revenue, z.expenses), (cents(500_000), cents(180_000)));
    let none = project_to_date(&plain.l, plain.zeta, year.1).unwrap();
    assert!(!none.any_tagged());
}

proptest! {
    /// arap::document_parts INVARIANTS 1-3: the parts sum to the total,
    /// each project once and in the order first named, the untagged lines
    /// one part, and a document naming none one part.
    #[test]
    fn prop_document_parts_partition_the_total(
        raw in prop::collection::vec((1i64..=10_000_000, prop::option::of(0u32..4)), 1..12)
    ) {
        let lines: Vec<DocumentLine> = raw
            .iter()
            .map(|&(c, k)| line("x", c, k))
            .collect();
        let parts = document_parts(&lines, "doc").unwrap();
        let total: Decimal = lines.iter().map(|l| l.amount).sum();
        prop_assert_eq!(parts.iter().map(|p| p.1).sum::<Decimal>(), total);
        let mut first: Vec<Option<ClassId>> = Vec::new();
        for l in &lines {
            if !first.contains(&l.class_id) {
                first.push(l.class_id);
            }
        }
        prop_assert_eq!(parts.iter().map(|p| p.0).collect::<Vec<_>>(), first);
        for (k, amount) in &parts {
            let sum: Decimal = lines.iter().filter(|l| l.class_id == *k).map(|l| l.amount).sum();
            prop_assert_eq!(*amount, sum);
            prop_assert!(*amount > Decimal::ZERO);
        }
        if lines.iter().all(|l| l.class_id.is_none()) {
            prop_assert_eq!(parts, vec![(None, total)]);
        }
    }
}

/// document_parts INVARIANT 4: a sum past exact arithmetic is refused,
/// never a panic.
#[test]
fn document_parts_refuse_past_exact_arithmetic() {
    let big = DocumentLine {
        description: "x".into(),
        amount: Decimal::MAX,
        class_id: Some(1),
    };
    assert!(matches!(
        document_parts(&[big.clone(), big], "doc"),
        Err(LedgerError::TooLargeToCompute(_))
    ));
}
