//! The audit's statements checks (plan P8; rules.md B-45): one test per
//! finding code, each seeding exactly the defect the code names.

use std::collections::BTreeSet;

use chrono::NaiveDate;
use ledger_core::arap::{ControlCheck, DocAttention, UnattachedLine};
use ledger_core::audit::{
    self, AttestationBar, AuditInput, CODES, Effort, Finding, Method, Severity,
};
use ledger_core::cashflow::CashFlowFooting;
use ledger_core::periods::MemberSplit;
use ledger_core::reports::{AssetRegister, TrialBalance};
use ledger_core::settings::Settings;
use ledger_core::{
    AccountType, Currency, DocRef, DocStatus, EntryId, Ledger, Money, NewAsset, NewSchedule,
    ScheduleConvention, ScheduleKind, Side,
};
use rust_decimal::Decimal;

fn d(m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, day).unwrap()
}

fn dec(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn usd(s: &str) -> Money {
    Money::new(dec(s), Currency::USD)
}

struct Books {
    l: Ledger,
    cash: u64,
    capital: u64,
    expense: u64,
    revenue: u64,
}

/// A bank, capital, an expense and a revenue account, all categorized.
fn books() -> Books {
    let mut l = Ledger::new(Currency::USD);
    let mut a = |code: &str, name: &str, t| l.add_account(code, name, t, None, false).unwrap();
    let cash = a("1000", "Bank", AccountType::Asset);
    let capital = a("3000", "Capital", AccountType::Equity);
    let revenue = a("4000", "Sales", AccountType::Revenue);
    let expense = a("6000", "Supplies", AccountType::Expense);
    l.set_account_tax(revenue, Some("gross_receipts"), None)
        .unwrap();
    l.set_account_tax(expense, Some("supplies"), None).unwrap();
    Books {
        l,
        cash,
        capital,
        expense,
        revenue,
    }
}

fn post(l: &mut Ledger, date: NaiveDate, dr: u64, cr: u64, amt: &str) -> EntryId {
    let e = l.create_entry(date, "x");
    l.add_line(e, dr, Side::Debit, usd(amt)).unwrap();
    l.add_line(e, cr, Side::Credit, usd(amt)).unwrap();
    l.post_entry(e).unwrap();
    e
}

fn run(l: &Ledger, receipts: &BTreeSet<EntryId>, through: NaiveDate) -> Vec<Finding> {
    let settings = Settings::default();
    audit::statements(
        l,
        &AuditInput {
            settings: &settings,
            receipts,
            cash_flow: None,
            settings_problems: &[],
            undecided_1099: &[],
        },
        through,
    )
}

/// Every posted entry has its receipt.
fn receipted(l: &Ledger) -> BTreeSet<EntryId> {
    l.posted_entries().map(|e| e.id()).collect()
}

fn codes(f: &[Finding]) -> Vec<&'static str> {
    f.iter().map(|f| f.code).collect()
}

fn control(difference: &str) -> ControlCheck {
    ControlCheck {
        role: "receivables".into(),
        account: 7,
        balance: dec("100.00"),
        documents_total: dec("100.00"),
        unattached: vec![],
        difference: dec(difference),
        attention: vec![],
    }
}

/// Every code's seed: the defect, run, and the codes it yields.
fn seeds() -> Vec<(&'static str, Vec<Finding>)> {
    let mut out = Vec::new();
    let none = BTreeSet::new();

    out.push((
        "trial_balance_unbalanced",
        audit::check_trial_balance(&TrialBalance {
            lines: vec![],
            total_debits: dec("10.00"),
            total_credits: dec("9.99"),
        })
        .into_iter()
        .collect(),
    ));

    let b = books();
    let mut bs = ledger_core::reports::balance_sheet_as_of(&b.l, d(12, 31)).unwrap();
    bs.total_assets = dec("1.00");
    out.push((
        "equation_broken",
        audit::check_equation(&bs).into_iter().collect(),
    ));

    out.push((
        "pnl_not_articulated",
        audit::check_articulation(dec("100.00"), dec("40.00"), dec("50.00"))
            .into_iter()
            .collect(),
    ));

    // A statement the engine refuses (a liability named as cash) and one
    // that does not foot are both unfooted.
    let settings = Settings::default();
    let refused: Result<CashFlowFooting, String> = Err("account 9 is not an asset".into());
    let off: Result<CashFlowFooting, String> = Ok(CashFlowFooting {
        sections_sum: dec("10.00"),
        net_change: dec("9.00"),
        difference: dec("1.00"),
    });
    let b = books();
    for made in [&refused, &off] {
        let f = audit::statements(
            &b.l,
            &AuditInput {
                settings: &settings,
                receipts: &none,
                cash_flow: Some(made),
                settings_problems: &[],
                undecided_1099: &[],
            },
            d(12, 31),
        );
        assert_eq!(codes(&f), vec!["cash_flow_unfooted"]);
    }
    out.push((
        "cash_flow_unfooted",
        audit::check_cash_flow(Some(&off)).into_iter().collect(),
    ));

    out.push((
        "control_difference",
        audit::check_control(&[control("0.01")]),
    ));
    let mut c = control("0.00");
    c.unattached = vec![UnattachedLine {
        entry: 3,
        line: 0,
        date: d(2, 1),
        amount: dec("5.00"),
    }];
    out.push(("unattached_control_line", audit::check_control(&[c])));
    let mut c = control("0.00");
    c.attention = vec![DocAttention {
        doc: DocRef::Invoice(1),
        status: DocStatus::Overpaid,
        total: dec("10.00"),
        outstanding: dec("-5.00"),
    }];
    out.push(("document_overpaid", audit::check_control(&[c])));

    out.push((
        "close_split_drifts",
        audit::check_split(
            2,
            "2026-10",
            &[MemberSplit {
                capital: 9,
                share: dec("0.5"),
                part: dec("1.00"),
                running: dec("1.00"),
                exact: dec("1.05"),
            }],
            dec("0.01"),
        ),
    ));

    // A prepaid plan with a month due and nothing posted.
    let mut b = books();
    let prepaid =
        b.l.add_account("1300", "Prepaid", AccountType::Asset, None, false)
            .unwrap();
    b.l.register_schedule(NewSchedule {
        description: "Insurance".into(),
        kind: ScheduleKind::PrepaidExpense,
        total: usd("1200.00"),
        start: d(1, 1),
        months: 12,
        balance_account: prepaid,
        pl_account: b.expense,
        convention: ScheduleConvention::MonthlyEven,
    })
    .unwrap();
    out.push(("schedule_behind", run(&b.l, &receipted(&b.l), d(3, 31))));

    // A registered asset, no depreciation run.
    let mut b = books();
    let (cost, accum, dep) = asset_accounts(&mut b);
    post(&mut b.l, d(1, 2), cost, b.capital, "6000.00");
    b.l.register_asset(NewAsset {
        description: "Van".into(),
        in_service_date: d(1, 15),
        cost: usd("6000.00"),
        salvage_value: usd("0"),
        useful_life_months: 60,
        cost_account: cost,
        accum_depr_account: accum,
        expense_account: dep,
    })
    .unwrap();
    out.push(("depreciation_behind", run(&b.l, &receipted(&b.l), d(6, 30))));

    // Cost posted to the asset's account that the register doesn't hold.
    post(&mut b.l, d(2, 1), cost, b.capital, "100.00");
    let f = run(&b.l, &receipted(&b.l), d(6, 30));
    out.push((
        "register_differs",
        f.into_iter()
            .filter(|f| f.code == "register_differs")
            .collect(),
    ));

    // A 40.00 hand entry (threshold 25.00) with no receipt, then reversed.
    let mut b = books();
    let e = post(&mut b.l, d(2, 1), b.expense, b.cash, "40.00");
    let f = run(&b.l, &none, d(12, 31));
    out.push(("evidence_missing", f));
    let r = b.l.reverse_entry(e, d(2, 2), "undo").unwrap();
    let mut with_original = BTreeSet::new();
    with_original.insert(e);
    let f = run(&b.l, &with_original, d(12, 31));
    assert!(f.iter().any(|f| f.subject == audit::Subject::Entry(r)));
    out.push(("legacy_reversal_exempt", f));

    let mut b = books();
    let e = b.l.create_entry(d(3, 3), "draft");
    b.l.add_line(e, b.expense, Side::Debit, usd("1.00"))
        .unwrap();
    b.l.add_line(e, b.cash, Side::Credit, usd("1.00")).unwrap();
    out.push(("draft_open", run(&b.l, &none, d(12, 31))));

    let mut b = books();
    b.l.add_account("4100", "Other sales", AccountType::Revenue, None, false)
        .unwrap();
    out.push(("tax_category_missing", run(&b.l, &none, d(12, 31))));

    let b = books();
    let settings = Settings::default();
    out.push((
        "vendor_1099_undecided",
        audit::statements(
            &b.l,
            &AuditInput {
                settings: &settings,
                receipts: &none,
                cash_flow: None,
                settings_problems: &[],
                undecided_1099: &[("Tobin Harlow Supply".into(), dec("900.00"))],
            },
            d(12, 31),
        ),
    ));
    out.push((
        "settings_problem",
        audit::statements(
            &b.l,
            &AuditInput {
                settings: &settings,
                receipts: &none,
                cash_flow: None,
                settings_problems: &["books.close_into names 9999".into()],
                undecided_1099: &[],
            },
            d(12, 31),
        ),
    ));
    let _ = (b.revenue, AccountType::Asset, Side::Debit);
    out
}

fn asset_accounts(b: &mut Books) -> (u64, u64, u64) {
    let cost =
        b.l.add_account("1500", "Equipment", AccountType::Asset, None, false)
            .unwrap();
    let accum =
        b.l.add_account(
            "1590",
            "Accumulated depreciation",
            AccountType::Asset,
            None,
            false,
        )
        .unwrap();
    let dep =
        b.l.add_account("6500", "Depreciation", AccountType::Expense, None, false)
            .unwrap();
    b.l.set_account_tax(dep, Some("supplies"), None).unwrap();
    (cost, accum, dep)
}

#[test]
fn audit_detects_each_seeded_defect() {
    let seeds = seeds();
    for row in CODES {
        let (_, found) = seeds
            .iter()
            .find(|(c, _)| *c == row.code)
            .unwrap_or_else(|| panic!("no seeding fixture for {}", row.code));
        let mine: Vec<&Finding> = found.iter().filter(|f| f.code == row.code).collect();
        assert!(!mine.is_empty(), "{} was not found", row.code);
        assert!(
            mine.iter().all(|f| f.severity == row.severity),
            "{}",
            row.code
        );
        // Exactly that finding: nothing else but what the seed implies.
        let others: Vec<_> = found.iter().filter(|f| f.code != row.code).collect();
        let implied = match row.code {
            // A 40.00 reversal chain, or a draft's unbalanced side, leaves
            // the neighbouring checks quiet.
            "legacy_reversal_exempt" => vec!["legacy_reversal_exempt", "evidence_missing"],
            _ => vec![],
        };
        for o in others {
            assert!(
                implied.contains(&o.code),
                "{}: also found {} ({})",
                row.code,
                o.code,
                o.message
            );
        }
    }
    for (c, _) in &seeds {
        assert!(CODES.iter().any(|r| r.code == *c), "{c} is not in CODES");
    }
}

#[test]
fn a_whole_book_gives_no_failure_or_warning() {
    let mut b = books();
    post(&mut b.l, d(1, 5), b.cash, b.capital, "1000.00");
    post(&mut b.l, d(1, 6), b.cash, b.revenue, "20.00");
    let f = run(&b.l, &receipted(&b.l), d(12, 31));
    assert!(
        f.iter().all(|f| f.severity == Severity::Info),
        "{:?}",
        codes(&f)
    );
}

#[test]
fn findings_come_in_one_order() {
    let mut b = books();
    post(&mut b.l, d(2, 1), b.expense, b.cash, "40.00");
    let e = b.l.create_entry(d(3, 3), "draft");
    b.l.add_line(e, b.expense, Side::Debit, usd("1.00"))
        .unwrap();
    b.l.add_line(e, b.cash, Side::Credit, usd("1.00")).unwrap();
    let a = run(&b.l, &BTreeSet::new(), d(12, 31));
    let c = run(&b.l, &BTreeSet::new(), d(12, 31));
    assert_eq!(a, c);
    assert_eq!(codes(&a), vec!["evidence_missing", "draft_open"]);
    let _ = AssetRegister::unposted_depreciation;
}

fn method(effort: Option<&str>, adversarial: Option<bool>, agents: Option<u32>) -> Method {
    Method {
        model: None,
        reasoning_effort: effort.map(|e| Effort::parse(e).unwrap()),
        adversarial,
        verifier_agents: agents,
    }
}

/// D-9: effort at the bar counts for a receipt or a month; a year-level
/// attestation also needs the verifiers and the adversarial check; less
/// than the bar, or no claim at all, never counts.
#[test]
fn below_bar_not_counted() {
    let bar = AttestationBar::default();
    assert_eq!(bar.describe(), "effort=high verifiers=2 adversarial=true");
    assert!(bar.counts(&method(Some("high"), None, None), false, false));
    assert!(bar.counts(&method(Some("max"), None, None), false, false));
    assert!(!bar.counts(&method(Some("medium"), None, None), false, false));
    assert!(!bar.counts(&method(None, Some(true), Some(3)), false, false));
    assert!(bar.counts(&method(Some("high"), Some(true), Some(2)), true, false));
    assert!(!bar.counts(&method(Some("high"), Some(true), Some(1)), true, false));
    assert!(!bar.counts(&method(Some("high"), Some(false), Some(2)), true, false));
    assert!(!bar.counts(&method(Some("high"), None, Some(2)), true, false));
    // A bar change changes what counts, earlier claims too (no
    // versioning, D-9).
    let lax = AttestationBar::parse("effort=low verifiers=1 adversarial=false").unwrap();
    assert!(lax.counts(&method(Some("low"), Some(false), Some(1)), true, false));
    assert!(AttestationBar::parse("effort=wild").is_err());
    assert!(AttestationBar::parse("verifiers=0").is_err());
    assert!(AttestationBar::parse("nonsense").is_err());
    assert_eq!(
        AttestationBar::parse("adversarial=false, effort=xhigh")
            .unwrap()
            .describe(),
        "effort=xhigh verifiers=2 adversarial=false"
    );
}

/// A reading that disagrees with the books never counts, however hard the
/// checker says it thought.
#[test]
fn amount_mismatch_never_counts() {
    let bar = AttestationBar::default();
    let top = method(Some("max"), Some(true), Some(9));
    assert!(bar.counts(&top, true, false));
    assert!(!bar.counts(&top, false, true));
    assert!(!bar.counts(&top, true, true));
}
