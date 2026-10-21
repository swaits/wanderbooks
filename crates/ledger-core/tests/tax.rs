//! Tax categories and the tax summary. Expected numbers computed by hand.
//!
//! The fixture: a partnership's first quarter.
//!
//! | account               | category              | % | book    | deductible |
//! |-----------------------|-----------------------|---|---------|------------|
//! | 4000 Sales            | gross_receipts        |   | 10,000  | 10,000     |
//! | 4100 Refunds (exp.)   | returns_allowances    |   | 250     | (250)      |
//! | 6000 Platform fees    | commissions_fees      |   | 1,500   | 1,500      |
//! | 6100 Meals            | meals                 |50%| 333.33  | 166.67     |
//! | 6200 Penalties        | nondeductible         | 0%| 40      | 0          |
//! | 6300 Software         | other_deductions      |   | 600     | 600        |
//! | 6400 Partner pay      | guaranteed_payments   |   | 2,000   | 2,000      |
//! | 6500 Misc             | (none)                |   | 75      | uncategorized |

use chrono::NaiveDate;
use ledger_core::tax::{TaxForm, check_category, tax_summary};
use ledger_core::{AccountType, Currency, Ledger, LedgerError, Money, Side};
use rust_decimal::Decimal;
use std::str::FromStr;

fn d(m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, day).unwrap()
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn books() -> Ledger {
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let mut acct =
        |code: &str, name: &str, t: AccountType, cat: Option<&str>, pct: Option<&str>| {
            let id = l.add_account(code, name, t, None, false).unwrap();
            if let Some(c) = cat {
                l.set_account_tax(id, Some(c), pct.map(dec)).unwrap();
            }
            id
        };
    let sales = acct(
        "4000",
        "Sales",
        AccountType::Revenue,
        Some("gross_receipts"),
        None,
    );
    let refunds = acct(
        "4100",
        "Refunds",
        AccountType::Expense,
        Some("returns_allowances"),
        None,
    );
    let fees = acct(
        "6000",
        "Platform fees",
        AccountType::Expense,
        Some("commissions_fees"),
        None,
    );
    let meals = acct("6100", "Meals", AccountType::Expense, Some("meals"), None);
    let pen = acct(
        "6200",
        "Penalties",
        AccountType::Expense,
        Some("nondeductible"),
        None,
    );
    let soft = acct(
        "6300",
        "Software",
        AccountType::Expense,
        Some("other_deductions"),
        None,
    );
    let gp = acct(
        "6400",
        "Partner pay",
        AccountType::Expense,
        Some("guaranteed_payments"),
        None,
    );
    let misc = acct("6500", "Misc", AccountType::Expense, None, None);
    let mut post = |dr, cr, amt: &str| {
        let e = l.create_entry(d(2, 1), "x");
        l.add_line(e, dr, Side::Debit, Money::new(dec(amt), Currency::USD))
            .unwrap();
        l.add_line(e, cr, Side::Credit, Money::new(dec(amt), Currency::USD))
            .unwrap();
        l.post_entry(e).unwrap();
    };
    post(cash, sales, "10000");
    for (a, amt) in [
        (refunds, "250"),
        (fees, "1500"),
        (meals, "333.33"),
        (pen, "40"),
        (soft, "600"),
        (gp, "2000"),
        (misc, "75"),
    ] {
        post(a, cash, amt);
    }
    l
}

#[test]
fn the_summary_groups_by_category_and_applies_deductible_percents() {
    let l = books();
    let ts = tax_summary(&l, d(1, 1), d(3, 31), Some(TaxForm::Form1065)).unwrap();
    let rows = |lines: &[ledger_core::TaxSummaryLine]| -> Vec<(String, Decimal, Decimal)> {
        lines
            .iter()
            .map(|l| (l.category.clone(), l.book, l.tax))
            .collect()
    };
    // Refunds (an expense account) reduce receipts as negative income.
    assert_eq!(
        rows(&ts.income),
        [
            ("gross_receipts".into(), dec("10000"), dec("10000")),
            ("returns_allowances".into(), dec("-250"), dec("-250")),
        ]
    );
    // Catalog order; meals at 50% (333.33 → 166.665 → 166.67, half away
    // from zero); penalties at 0%.
    assert_eq!(
        rows(&ts.deductions),
        [
            ("commissions_fees".into(), dec("1500"), dec("1500")),
            ("guaranteed_payments".into(), dec("2000"), dec("2000")),
            ("meals".into(), dec("333.33"), dec("166.67")),
            ("other_deductions".into(), dec("600"), dec("600")),
            ("nondeductible".into(), dec("40"), dec("0")),
        ]
    );
    assert_eq!(ts.uncategorized.len(), 1);
    assert_eq!(ts.uncategorized[0].account, "6500 Misc");
    assert_eq!(ts.uncategorized[0].book, dec("75"));
    // Book: 9,750 − 4,473.33 = 5,276.67. Tax: 9,750 − 4,266.67 = 5,483.33.
    assert_eq!(ts.book_net(), dec("5276.67"));
    assert_eq!(ts.tax_net(), dec("5483.33"));
}

#[test]
fn an_override_beats_the_category_default() {
    let mut l = books();
    let meals = l.chart().find_by_code("6100").unwrap().id;
    // A company party is 100% deductible: override the 50% default.
    l.set_account_tax(meals, Some("meals"), Some(dec("100")))
        .unwrap();
    let ts = tax_summary(&l, d(1, 1), d(3, 31), None).unwrap();
    let m = ts
        .deductions
        .iter()
        .find(|x| x.category == "meals")
        .unwrap();
    assert_eq!(m.tax, dec("333.33"));
}

#[test]
fn categories_must_fit_the_account_and_the_return() {
    let mut l = books();
    let sales = l.chart().find_by_code("4000").unwrap().id;
    let meals = l.chart().find_by_code("6100").unwrap().id;
    let bad = |r: Result<(), LedgerError>| matches!(r, Err(LedgerError::InvalidTaxCategory(_)));
    assert!(
        bad(l.set_account_tax(sales, Some("meals"), None)),
        "deduction on revenue"
    );
    assert!(
        bad(l.set_account_tax(meals, Some("no_such"), None)),
        "unknown key"
    );
    assert!(
        bad(l.set_account_tax(meals, Some("meals"), Some(dec("101")))),
        "over 100%"
    );
    assert!(
        bad(l.set_account_tax(meals, None, Some(dec("50")))),
        "percent without category"
    );
    // Guaranteed payments exist only on a partnership return.
    assert!(
        check_category(
            "guaranteed_payments",
            AccountType::Expense,
            Some(TaxForm::Form1065)
        )
        .is_ok()
    );
    assert!(
        check_category(
            "guaranteed_payments",
            AccountType::Expense,
            Some(TaxForm::ScheduleC)
        )
        .is_err()
    );
    assert!(
        check_category(
            "officer_compensation",
            AccountType::Expense,
            Some(TaxForm::Form1120S)
        )
        .is_ok()
    );
    // Clearing works.
    l.set_account_tax(meals, None, None).unwrap();
    assert!(l.chart().get(meals).unwrap().tax_category.is_none());
}

#[test]
fn forms_parse_from_common_spellings() {
    for (s, f) in [
        ("schedule_c", TaxForm::ScheduleC),
        ("Schedule C", TaxForm::ScheduleC),
        ("1065", TaxForm::Form1065),
        ("Form 1065", TaxForm::Form1065),
        ("1120s", TaxForm::Form1120S),
        ("1120-S", TaxForm::Form1120S),
        ("1120", TaxForm::Form1120),
    ] {
        assert_eq!(TaxForm::parse(s), Some(f), "{s}");
    }
    assert_eq!(TaxForm::parse("990"), None);
}

#[test]
fn cost_of_goods_reduces_net_like_a_deduction() {
    let mut l = books();
    let cash = l.chart().find_by_code("1000").unwrap().id;
    let cogs = l
        .add_account("5000", "Cost of sales", AccountType::Expense, None, false)
        .unwrap();
    l.set_account_tax(cogs, Some("cost_of_goods_sold"), None)
        .unwrap();
    let e = l.create_entry(d(2, 1), "x");
    let m = Money::new(dec("1000"), Currency::USD);
    l.add_line(e, cogs, Side::Debit, m).unwrap();
    l.add_line(e, cash, Side::Credit, m).unwrap();
    l.post_entry(e).unwrap();
    let ts = tax_summary(&l, d(1, 1), d(3, 31), Some(TaxForm::Form1065)).unwrap();
    assert_eq!(ts.cost_of_goods.len(), 1);
    assert_eq!(ts.cost_of_goods[0].book, dec("1000"));
    assert_eq!(ts.cost_of_goods[0].tax, dec("1000"));
    // The fixture's nets, less 1,000 of cost of goods sold.
    assert_eq!(ts.book_net(), dec("4276.67"));
    assert_eq!(ts.tax_net(), dec("4483.33"));
}

#[test]
fn forms_have_their_irs_names() {
    let labels: Vec<&str> = TaxForm::ALL.iter().map(|f| f.label()).collect();
    assert_eq!(
        labels,
        ["Schedule C", "Form 1065", "Form 1120-S", "Form 1120"]
    );
    // A label parses back to its form.
    for f in TaxForm::ALL {
        assert_eq!(TaxForm::parse(f.label()), Some(f));
    }
}

/// A small book with portfolio interest (125) and a donation (300) besides
/// sales (1,000), for where each lands by return.
fn interest_and_gift() -> Ledger {
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let mut acct = |code: &str, t, cat: &str| {
        let id = l.add_account(code, code, t, None, false).unwrap();
        l.set_account_tax(id, Some(cat), None).unwrap();
        id
    };
    let sales = acct("4000", AccountType::Revenue, "gross_receipts");
    let interest = acct("4200", AccountType::Revenue, "interest_income");
    let gift = acct("6900", AccountType::Expense, "charitable_contributions");
    let usd = |s: &str| Money::new(dec(s), Currency::USD);
    for (dr, cr, amt) in [
        (cash, sales, "1000"),
        (cash, interest, "125"),
        (gift, cash, "300"),
    ] {
        let e = l.create_entry(d(2, 1), "x");
        l.add_line(e, dr, Side::Debit, usd(amt)).unwrap();
        l.add_line(e, cr, Side::Credit, usd(amt)).unwrap();
        l.post_entry(e).unwrap();
    }
    l
}

/// Oracle: docs/spec/sources/irs-tax-lines.md. On Forms 1065 and 1120-S,
/// Schedule K takes "only taxable portfolio interest", and charitable
/// contributions are among the "items ... must state separately": both
/// leave the ordinary-income net (1,000 here, not 825) and are listed
/// apart. On Form 1120 they stay in it (lines 5 and 19).
#[test]
fn pass_throughs_state_interest_and_gifts_separately() {
    let l = interest_and_gift();
    for form in [TaxForm::Form1065, TaxForm::Form1120S] {
        let ts = tax_summary(&l, d(1, 1), d(12, 31), Some(form)).unwrap();
        let keys: Vec<&str> = ts
            .separately_stated
            .iter()
            .map(|x| x.category.as_str())
            .collect();
        assert_eq!(
            keys,
            ["interest_income", "charitable_contributions"],
            "{form:?}"
        );
        assert_eq!(ts.book_net(), dec("1000"), "{form:?}");
    }
    let ts = tax_summary(&l, d(1, 1), d(12, 31), Some(TaxForm::Form1120)).unwrap();
    assert!(ts.separately_stated.is_empty());
    assert_eq!(ts.book_net(), dec("825"));
}

/// Oracle: Schedule C instructions, Part V ("Do not include charitable
/// contributions"): the category is refused for a Schedule C business.
#[test]
fn a_sole_proprietor_cannot_deduct_a_gift_on_schedule_c() {
    assert!(matches!(
        check_category(
            "charitable_contributions",
            AccountType::Expense,
            Some(TaxForm::ScheduleC)
        ),
        Err(LedgerError::InvalidTaxCategory(_))
    ));
    assert!(
        check_category(
            "charitable_contributions",
            AccountType::Expense,
            Some(TaxForm::Form1120)
        )
        .is_ok()
    );
}

// ---------------------------------------------------------------------------
// Which accounts need a category, and the policy (P3, defect 8; rules.md
// B-32).
// ---------------------------------------------------------------------------

fn set_policy(l: &mut Ledger, value: &str) {
    use ledger_core::settings::{Change, Source};
    let src = Source {
        decided_by: "Owner".into(),
        decided_on: d(10, 20),
        citation: None,
        note: None,
    };
    let ch = l
        .check_settings(
            &[Change {
                key: "policy.require_tax_category".into(),
                value: Some(serde_json::json!(value)),
            }],
            &src,
        )
        .unwrap();
    l.apply_settings(&ch, &src, 1).unwrap();
}

/// needs_category: revenue and expense accounts that aren't headers. A
/// header never reaches the return: no entry posts to one, a replayed one
/// included (review R13 supposed an old journal could; replay refuses it).
#[test]
fn revenue_and_expense_accounts_need_a_category_headers_dont() {
    use ledger_core::tax::needs_category;
    for (t, header, needs) in [
        (AccountType::Revenue, false, true),
        (AccountType::Expense, false, true),
        (AccountType::Revenue, true, false),
        (AccountType::Expense, true, false),
        (AccountType::Asset, false, false),
        (AccountType::Liability, false, false),
        (AccountType::Equity, false, false),
        (AccountType::Asset, true, false),
    ] {
        assert_eq!(needs_category(t, header), needs, "{t:?} header {header}");
    }
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let group = l
        .add_account("6000", "Operating", AccountType::Expense, None, true)
        .unwrap();
    let a = l.chart().get(group).unwrap().clone();
    assert!(!ledger_core::tax::account_needs_category(&a));
    let e = l.create_entry(d(2, 1), "a posting to a header");
    let m = Money::new(dec("5.00"), Currency::USD);
    l.replay_add_line(e, group, Side::Debit, m).unwrap();
    l.replay_add_line(e, cash, Side::Credit, m).unwrap();
    assert_eq!(
        l.replay_post_entry(e),
        Err(LedgerError::PlaceholderAccount("6000".into()))
    );
}

/// uncategorized_accounts (repro 1.4.2-1.4.3): every revenue and expense
/// account without a category, active or not, last year's included,
/// sorted by code; headers without postings and categorized accounts
/// aren't listed.
#[test]
fn every_uncategorized_pl_account_is_listed_active_or_not() {
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    l.add_account("6000", "Operating", AccountType::Expense, None, true)
        .unwrap();
    let software = l
        .add_account("6150", "Software", AccountType::Expense, None, false)
        .unwrap();
    let last_year = l
        .add_account("6160", "Hosting", AccountType::Expense, None, false)
        .unwrap();
    let sales = l
        .add_account("4000", "Sales", AccountType::Revenue, None, false)
        .unwrap();
    l.set_account_tax(sales, Some("gross_receipts"), None)
        .unwrap();
    l.add_account("4900", "Other income", AccountType::Revenue, None, false)
        .unwrap();
    let e = l.create_entry(NaiveDate::from_ymd_opt(2025, 6, 1).unwrap(), "last year");
    let m = Money::new(dec("12.00"), Currency::USD);
    l.add_line(e, last_year, Side::Debit, m).unwrap();
    l.add_line(e, cash, Side::Credit, m).unwrap();
    l.post_entry(e).unwrap();
    let listed: Vec<String> = ledger_core::tax::uncategorized_accounts(&l)
        .into_iter()
        .map(|a| format!("{} {}", a.code, a.name))
        .collect();
    assert_eq!(
        listed,
        ["4900 Other income", "6150 Software", "6160 Hosting"]
    );
    l.set_account_tax(software, Some("other_deductions"), None)
        .unwrap();
    assert_eq!(ledger_core::tax::uncategorized_accounts(&l).len(), 2);
}

/// check_tax_category_required's truth table: refused iff the policy is
/// refuse, the account needs a category (revenue or expense, not a
/// header) and none is given; the message names the account and where
/// the categories are.
#[test]
fn the_policy_refuses_exactly_a_needed_category_left_out() {
    use ledger_core::settings::TaxCategoryPolicy;
    let types = [
        AccountType::Asset,
        AccountType::Liability,
        AccountType::Equity,
        AccountType::Revenue,
        AccountType::Expense,
    ];
    for policy in ["warn", "refuse"] {
        let mut l = Ledger::new(Currency::USD);
        set_policy(&mut l, policy);
        let refuse = l.settings().require_tax_category() == TaxCategoryPolicy::Refuse;
        assert_eq!(refuse, policy == "refuse");
        for t in types {
            for header in [false, true] {
                for category in [None, Some("other_income")] {
                    let got =
                        l.check_tax_category_required("6150", "Software", t, header, category);
                    let needs = matches!(t, AccountType::Revenue | AccountType::Expense) && !header;
                    if refuse && needs && category.is_none() {
                        let e = got.unwrap_err();
                        assert_eq!(
                            e,
                            LedgerError::TaxCategoryRequired {
                                account: "6150 Software".into(),
                                clearing: false
                            }
                        );
                        assert!(e.to_string().contains("books.tax_categories"), "{e}");
                    } else {
                        assert_eq!(got, Ok(()), "{policy} {t:?} {header} {category:?}");
                    }
                }
            }
        }
    }
}

/// check_tax_category_kept: under refuse, an account that needs a
/// category can't be left without one (cleared, or "cleared" while it has
/// none); setting one, or any account under warn, passes.
#[test]
fn under_refuse_a_needed_category_is_never_cleared() {
    let mut l = Ledger::new(Currency::USD);
    let software = l
        .add_account("6150", "Software", AccountType::Expense, None, false)
        .unwrap();
    let bare = l
        .add_account("6160", "Hosting", AccountType::Expense, None, false)
        .unwrap();
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    l.set_account_tax(software, Some("other_deductions"), None)
        .unwrap();
    for id in [software, bare, cash] {
        assert_eq!(l.check_tax_category_kept(id, None), Ok(()));
    }
    set_policy(&mut l, "refuse");
    assert_eq!(
        l.check_tax_category_kept(software, None),
        Err(LedgerError::TaxCategoryRequired {
            account: "6150 Software".into(),
            clearing: true
        })
    );
    assert_eq!(
        l.check_tax_category_kept(bare, None),
        Err(LedgerError::TaxCategoryRequired {
            account: "6160 Hosting".into(),
            clearing: false
        })
    );
    assert_eq!(l.check_tax_category_kept(cash, None), Ok(()));
    assert_eq!(
        l.check_tax_category_kept(software, Some("supplies")),
        Ok(())
    );
    assert_eq!(
        l.check_tax_category_kept(999, None),
        Err(LedgerError::UnknownAccount(999))
    );
    // The engine's own set_account_tax (which replay calls) never checks
    // the policy (rules.md B-28): a journaled clear replays.
    l.set_account_tax(software, None, None).unwrap();
}

/// rules.md B-32 (review R3): every account a tool can require has a
/// category that fits it on every return: a disposal's gain or loss
/// account (revenue or expense: Form 4797), tax-exempt income on the
/// entity returns, and each control role's account.
#[test]
fn every_account_a_tool_requires_has_a_fitting_category_on_every_return() {
    for form in TaxForm::ALL {
        for t in [AccountType::Revenue, AccountType::Expense] {
            assert!(
                check_category("form_4797_gain_loss", t, Some(form)).is_ok(),
                "{form:?} {t:?}"
            );
        }
        for c in ledger_core::control_accounts()
            .filter(|c| matches!(c.account_type, AccountType::Revenue | AccountType::Expense))
        {
            let fits = ledger_core::tax::CATEGORIES
                .iter()
                .any(|cat| check_category(cat.key, c.account_type, Some(form)).is_ok());
            assert!(fits, "{} on {form:?}", c.code);
        }
        let exempt = check_category("tax_exempt_income", AccountType::Revenue, Some(form));
        assert_eq!(exempt.is_ok(), form != TaxForm::ScheduleC, "{form:?}");
    }
    let c = ledger_core::tax::category("tax_exempt_income").unwrap();
    assert_eq!(c.default_deductible_percent, 0);
    let c = ledger_core::tax::category("form_4797_gain_loss").unwrap();
    assert_eq!(c.default_deductible_percent, 100);
}

/// Tax-exempt income (Form 1065 Schedule K 18a-18b) is book income with
/// no taxable amount: 0% of 300.00 is 0.00, so book net and tax net
/// differ by it, the income twin of nondeductible.
#[test]
fn tax_exempt_income_is_book_income_with_no_tax_amount() {
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let muni = l
        .add_account(
            "4800",
            "Municipal bond interest",
            AccountType::Revenue,
            None,
            false,
        )
        .unwrap();
    l.set_account_tax(muni, Some("tax_exempt_income"), None)
        .unwrap();
    let e = l.create_entry(d(3, 1), "coupon");
    let m = Money::new(dec("300.00"), Currency::USD);
    l.add_line(e, cash, Side::Debit, m).unwrap();
    l.add_line(e, muni, Side::Credit, m).unwrap();
    l.post_entry(e).unwrap();
    let s = tax_summary(&l, d(1, 1), d(12, 31), Some(TaxForm::Form1065)).unwrap();
    assert_eq!(s.income.len(), 1);
    assert_eq!(s.income[0].category, "tax_exempt_income");
    assert_eq!(s.income[0].book, dec("300.00"));
    assert_eq!(s.income[0].tax, dec("0.00"));
    assert_eq!(s.book_net(), dec("300.00"));
    assert_eq!(s.tax_net(), dec("0.00"));
}

/// Oracle: docs/spec/sources/irs-tax-lines.md. A gain or loss on business
/// property is never ordinary income the summary can place: Form 1065
/// line 6 and Form 1120-S line 4 take "only ordinary gains or losses"
/// from Form 4797, net section 1231 gain is Schedule K line 10 (1065) or
/// line 9 (1120-S), and the Schedule C instructions send sales of
/// business property to "Form 4797", not Schedule C. Form 4797 figures
/// the gain from the tax basis, which the books don't keep. So on every
/// return (and with none set) the 150.00 gain and the 40.00 loss are
/// listed apart at their book amounts (110.00 net), outside income and
/// both nets: the nets are the 1,000.00 of sales alone.
#[test]
fn a_gain_on_business_property_is_figured_on_form_4797_on_every_return() {
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let mut acct = |code: &str, name: &str, t, cat: &str| {
        let id = l.add_account(code, name, t, None, false).unwrap();
        l.set_account_tax(id, Some(cat), None).unwrap();
        id
    };
    let sales = acct("4000", "Sales", AccountType::Revenue, "gross_receipts");
    let gain = acct(
        "4700",
        "Gain on disposal",
        AccountType::Revenue,
        "form_4797_gain_loss",
    );
    let loss = acct(
        "6700",
        "Loss on disposal",
        AccountType::Expense,
        "form_4797_gain_loss",
    );
    let usd = |s: &str| Money::new(dec(s), Currency::USD);
    for (dr, cr, amt) in [
        (cash, sales, "1000.00"),
        (cash, gain, "150.00"),
        (loss, cash, "40.00"),
    ] {
        let e = l.create_entry(d(4, 1), "x");
        l.add_line(e, dr, Side::Debit, usd(amt)).unwrap();
        l.add_line(e, cr, Side::Credit, usd(amt)).unwrap();
        l.post_entry(e).unwrap();
    }
    for form in [None].into_iter().chain(TaxForm::ALL.into_iter().map(Some)) {
        let ts = tax_summary(&l, d(1, 1), d(12, 31), form).unwrap();
        let keys = |ls: &[ledger_core::TaxSummaryLine]| -> Vec<String> {
            ls.iter().map(|x| x.category.clone()).collect()
        };
        assert_eq!(keys(&ts.income), ["gross_receipts"], "{form:?}");
        assert!(ts.deductions.is_empty(), "{form:?}");
        assert!(ts.separately_stated.is_empty(), "{form:?}");
        assert_eq!(
            ts.form_4797,
            [ledger_core::BookOnlyLine {
                category: "form_4797_gain_loss".into(),
                label: "Gain or loss on business property (Form 4797)".into(),
                accounts: vec![
                    "4700 Gain on disposal".into(),
                    "6700 Loss on disposal".into()
                ],
                book: dec("110.00"),
            }],
            "{form:?}"
        );
        assert_eq!(ts.book_net(), dec("1000.00"), "{form:?}");
        assert_eq!(ts.tax_net(), dec("1000.00"), "{form:?}");
    }
}
