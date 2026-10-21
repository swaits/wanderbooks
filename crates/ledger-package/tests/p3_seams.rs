//! P3's seams: the same economic fact, computed by two pieces of the
//! engine, agrees (or rules.md says exactly why not).
//!
//! One year of fictional books for a two-member LLC (Ana Example and Ben
//! Example, 50/50), built through the real [`Store`]: cash and property
//! contributions, member-paid expenses through Due to Members and their
//! reimbursement, card purchases and card payments, a prepaid on a daily
//! schedule, a 1099 contractor paid by check, by card and by a member's
//! check, whose status changes mid-year, a vendor refund, a loan,
//! depreciation runs with one reversed and caught up, a disposal, a
//! distribution declared then paid, and monthly closes to the members with
//! odd cents. Every name and amount is made up.
//!
//! Then, on several dates, the pieces are compared with each other:
//! the members' equity statement, the K-1 worksheet (book and tax basis),
//! party totals on both bases, the cash flow statement, the 1099
//! worksheet, the P&L, the balance sheet, the fixed asset register, the
//! IIF and the CPA package as of each date.

#[path = "seams/books.rs"]
mod books;

use books::{Books, books, d, dec};
use chrono::NaiveDate;
use ledger_core::parties::Basis;
use ledger_core::tax_capital::Open;
use ledger_core::{AccountId, CashFlowSection, CashFlowStatement, Ledger, MemberCapital, PartyId};
use ledger_package::{PackageRequest, Settings, build_package};
use rust_decimal::Decimal;

fn capitals(b: &Books) -> Vec<MemberCapital> {
    ledger_package::member_capitals(b.store.ledger(), &b.company.members).unwrap()
}

fn settings(b: &Books) -> Settings<'_> {
    Settings {
        company: &b.company,
        cash_accounts: &b.cash,
        financing_accounts: &b.financing,
        investing_accounts: &[],
    }
}

fn ids(l: &Ledger, codes: &[String]) -> Vec<AccountId> {
    codes
        .iter()
        .map(|c| l.chart().find_by_code(c).unwrap().id)
        .collect()
}

/// The dates every check runs on: mid-year after a close with a
/// distribution declared and unpaid, mid-month with nothing closed for
/// the month, and the year's end.
const DATES: [&str; 3] = ["2026-06-30", "2026-09-15", "2026-12-31"];
const START: &str = "2026-01-01";

fn cash_flow(b: &Books, end: NaiveDate) -> CashFlowStatement {
    ledger_package::cash_flow_for(
        b.store.ledger(),
        &settings(b),
        Some(START),
        Some(&end.to_string()),
        &[],
        &[],
        &[],
        end,
    )
    .unwrap()
}

/// The total of the kinds in `section` whose caption starts with `prefix`.
fn kinds(cf: &CashFlowStatement, section: CashFlowSection, prefix: &str) -> Decimal {
    cf.kinds(section)
        .iter()
        .filter(|k| k.kind.starts_with(prefix))
        .map(|k| k.amount)
        .sum()
}

fn kind(cf: &CashFlowStatement, section: CashFlowSection, caption: &str) -> Decimal {
    cf.kinds(section)
        .iter()
        .filter(|k| k.kind == caption)
        .map(|k| k.amount)
        .sum()
}

fn activity(b: &Books, party: PartyId, end: NaiveDate) -> ledger_core::parties::Activity {
    let l = b.store.ledger();
    let money = ledger_core::parties::money_accounts(l, &ids(l, &b.cash), &ids(l, &b.financing));
    ledger_core::parties::activity_with(
        l,
        party,
        (d(START), end),
        &money,
        &b.company.party_members(l),
    )
}

/// An account's balance through `end`, credit-positive.
fn credit_balance(l: &Ledger, code: &str, end: NaiveDate) -> Decimal {
    let id = l.chart().find_by_code(code).unwrap().id;
    l.posted_entries()
        .filter(|e| e.date() <= end)
        .flat_map(|e| e.lines().iter())
        .filter(|x| x.account == id)
        .map(|x| match x.side {
            ledger_core::Side::Credit => x.amount.amount,
            ledger_core::Side::Debit => -x.amount.amount,
        })
        .sum()
}

/// What the P&L says an expense account was from START to `end`.
fn pnl_expense(l: &Ledger, code: &str, end: NaiveDate) -> Decimal {
    ledger_core::profit_and_loss_for_period(l, d(START), end, None)
        .unwrap()
        .expenses
        .iter()
        .filter(|x| x.code == code)
        .map(|x| x.amount)
        .sum()
}

/// The P&L's depreciation expense (6500) from START to `end`.
fn pnl_depreciation(l: &Ledger, end: NaiveDate) -> Decimal {
    pnl_expense(l, "6500", end)
}

/// The members' figures agree on every date as B-13, B-35 items 4 and 8
/// and B-14 rule 11 say: the statement of members' equity and the K-1
/// worksheet are one roll-forward; party totals on basis all are the
/// statement's contributions, on basis cash the K-1's cash
/// contributions; the cash flow statement's contributions are the K-1's
/// cash, the property is disclosed, and its distributions are the
/// statement's less what is declared and unpaid, which party totals
/// count when paid too; what the members advanced is borrowed and repaid
/// on the statement and the repayment is paid to them in party totals.
#[test]
fn the_members_figures_agree_on_every_date() {
    let dir = tempfile::tempdir().unwrap();
    let b = books(dir.path());
    let l = b.store.ledger();
    for end in DATES.map(d) {
        let me = ledger_core::members_equity(l, d(START), end, &capitals(&b)).unwrap();
        let k1 = ledger_core::k1::k1_worksheet(l, d(START), end, &capitals(&b), &ids(l, &b.cash))
            .unwrap();
        let cf = cash_flow(&b, end);
        let bs = ledger_core::balance_sheet_as_of(l, end).unwrap();
        assert_eq!(me.total.ending, bs.total_equity, "{end}");
        let unpaid = credit_balance(l, "2200", end);
        let mut distributions_paid = Decimal::ZERO;
        let mut paid_to_members = Decimal::ZERO;
        for (i, (party, name)) in [(b.ana, "Ana Example"), (b.ben, "Ben Example")]
            .into_iter()
            .enumerate()
        {
            let (m, k) = (&me.members[i], &k1[i]);
            // One roll-forward (k1 INVARIANT 1).
            assert_eq!(
                (
                    m.beginning,
                    m.net_income,
                    m.distributions,
                    m.other,
                    m.ending
                ),
                (
                    k.beginning,
                    k.net_income,
                    k.distributions,
                    k.other,
                    k.ending
                ),
                "{name} {end}"
            );
            assert_eq!(
                m.contributions,
                k.contributions_cash + k.contributions_property
            );
            // Party totals (B-35 item 4).
            let a = activity(&b, party, end);
            assert!(a.unresolved.is_empty(), "{name} {end}: {:?}", a.unresolved);
            assert_eq!(a.totals(Basis::All).1, m.contributions, "{name} {end}");
            assert_eq!(
                a.totals(Basis::Cash).1,
                k.contributions_cash,
                "{name} {end}"
            );
            assert_eq!(a.received_in_kind, k.contributions_property, "{name} {end}");
            // The cash flow (B-14 rule 11, B-35 item 8).
            assert_eq!(
                kind(
                    &cf,
                    CashFlowSection::Financing,
                    &format!("Contributions from members — {name}")
                ),
                k.contributions_cash,
                "{name} {end}"
            );
            distributions_paid -= kind(
                &cf,
                CashFlowSection::Financing,
                &format!("Distributions to members — {name}"),
            );
            paid_to_members += a.totals(Basis::Cash).0;
        }
        // A distribution declared to Distributions Payable is a
        // distribution on the statement of members' equity when declared
        // and on the cash flow statement and in party totals when paid
        // (B-14 rule 2, B-35 item 4).
        distributions_paid -= kind(
            &cf,
            CashFlowSection::Financing,
            "Distributions to members — 2200 Distributions Payable",
        );
        assert_eq!(
            distributions_paid,
            me.total.distributions - unpaid,
            "{end}: distributions paid"
        );
        assert!(
            cf.noncash_disclosures
                .iter()
                .any(|n| n.label.starts_with("Distribution declared") && n.amount == dec("2000.00")),
            "{end}: the declaration is disclosed"
        );
        assert!(
            cf.noncash_disclosures
                .iter()
                .any(|n| n.label.starts_with("Ana contributes her laptop")
                    && n.amount == dec("1800.00")),
            "{end}: the property contributed is disclosed"
        );
        // No distribution is ever cash paid to suppliers, and no noncash
        // operating item hides the declaration.
        assert!(
            !cf.operating_reconciliation
                .lines
                .iter()
                .any(|r| r.label.contains("2200") || r.label == "Other noncash items"),
            "{end}: {:?}",
            cf.operating_reconciliation.lines
        );
        // What the members advanced: borrowed when they paid, repaid when
        // reimbursed; the reimbursements are what party totals paid the
        // members beyond their distributions.
        let advanced = kind(
            &cf,
            CashFlowSection::Financing,
            "Proceeds from borrowing — 2100 Due to Members",
        );
        let repaid = -kind(
            &cf,
            CashFlowSection::Financing,
            "Repayments of borrowing — 2100 Due to Members",
        );
        let expect = if end < d("2026-05-20") {
            dec("120.00")
        } else {
            dec("1020.00")
        };
        assert_eq!(advanced, expect, "{end}");
        assert_eq!(repaid, expect, "{end}");
        assert_eq!(paid_to_members, distributions_paid + repaid, "{end}");
        assert_eq!(credit_balance(l, "2100", end), Decimal::ZERO, "{end}");
    }
    // Hand figures at the year's end: Ana 7,500 cash and the 1,800 laptop,
    // Ben 7,500; each took 1,000 declared and 500 directly.
    let end = d("2026-12-31");
    let k1 =
        ledger_core::k1::k1_worksheet(l, d(START), end, &capitals(&b), &ids(l, &b.cash)).unwrap();
    assert_eq!(k1[0].contributions_property, dec("1800.00"));
    assert_eq!(k1[0].distributions, dec("1500.00"));
    assert_eq!(
        activity(&b, b.ana, end).totals(Basis::Cash).0,
        dec("1620.00")
    );
    assert_eq!(
        activity(&b, b.ben, end).totals(Basis::Cash).0,
        dec("2400.00")
    );
    // Mid-year, the declaration is unpaid: the statement counts it, party
    // totals and the cash flow don't yet (B-35 item 4's second condition).
    let mid = d("2026-06-30");
    assert_eq!(
        activity(&b, b.ana, mid).totals(Basis::Cash).0,
        dec("120.00")
    );
    assert_eq!(
        ledger_core::members_equity(l, d(START), mid, &capitals(&b))
            .unwrap()
            .members[0]
            .distributions,
        dec("1000.00")
    );
}

/// B-14 rule 2, from the seam review: a distribution declared to a
/// liability and paid later is a financing outflow when paid (ASC
/// 230-10-45-15(a)), its declaration a noncash financing activity; before
/// the fix it was "Cash paid to suppliers and employees".
#[test]
fn a_distribution_declared_then_paid_is_financing_when_paid() {
    let dir = tempfile::tempdir().unwrap();
    let b = books(dir.path());
    let cf = cash_flow(&b, d("2026-07-31"));
    assert_eq!(
        kind(
            &cf,
            CashFlowSection::Financing,
            "Distributions to members — 2200 Distributions Payable"
        ),
        dec("-2000.00")
    );
    let paid = cf
        .lines(CashFlowSection::Operating)
        .iter()
        .filter(|x| x.label.starts_with("Distribution paid"))
        .count();
    assert_eq!(paid, 0, "{:?}", cf.operating);
    // A statement of July alone, after the declaration's period, reads
    // the payment the same way.
    let july = ledger_package::cash_flow_for(
        b.store.ledger(),
        &settings(&b),
        Some("2026-07-01"),
        Some("2026-07-31"),
        &[],
        &[],
        &[],
        d("2026-07-31"),
    )
    .unwrap();
    assert_eq!(
        kind(
            &july,
            CashFlowSection::Financing,
            "Distributions to members — 2200 Distributions Payable"
        ),
        dec("-2000.00")
    );
    // Through June the payment hasn't happened: nothing in the body, the
    // declaration disclosed.
    let june = cash_flow(&b, d("2026-06-30"));
    assert_eq!(
        kinds(&june, CashFlowSection::Financing, "Distributions"),
        Decimal::ZERO
    );
}

/// B-35 item 9 and B-19: for a payee every entry of which names it alone,
/// party totals' paid less received (basis cash) is the 1099 worksheet's
/// paid + excluded card + undetermined + not reportable, less what the
/// worksheet lists as received for something else; and the worksheet
/// ties to the ledger by hand.
#[test]
fn the_1099_worksheet_ties_to_party_totals_and_the_ledger() {
    let dir = tempfile::tempdir().unwrap();
    let b = books(dir.path());
    let l = b.store.ledger();
    for end in DATES.map(d) {
        let w =
            ledger_package::vendor_1099_report(l, &settings(&b), 2026, None, Some(end)).unwrap();
        assert!(w.unresolved.is_empty());
        for v in &w.vendors {
            let party = l.party_of_vendor(v.vendor_id).unwrap();
            let a = activity(&b, party, end);
            let (paid, received) = a.totals(Basis::Cash);
            let other: Decimal = w
                .other_receipts
                .iter()
                .filter(|p| p.vendor_id == v.vendor_id)
                .map(|p| p.amount)
                .sum();
            assert_eq!(
                paid - received,
                v.paid + v.excluded_card + v.undetermined + v.not_reportable - other,
                "{} {end}",
                v.name
            );
        }
        let delta = w.vendors.iter().find(|v| v.vendor_id == b.delta).unwrap();
        // By hand: 1,500 by check and 900 by Ben's check count; 700 on
        // the card is excluded (Form 1099-K); 1,200 paid after the
        // election is not reportable (D-40).
        let expect = if end < d("2026-09-10") {
            ("2400.00", "700.00", "0")
        } else {
            ("2400.00", "700.00", "1200.00")
        };
        assert_eq!(
            (delta.paid, delta.excluded_card, delta.not_reportable),
            (dec(expect.0), dec(expect.1), dec(expect.2)),
            "{end}"
        );
        assert_eq!(delta.status, ledger_core::form1099::Status1099::Reportable);
        // The contract labor expense (6400) is exactly Delta's payments:
        // the four buckets are its debits in the year.
        let labor = pnl_expense(l, "6400", end);
        assert_eq!(
            delta.paid + delta.excluded_card + delta.undetermined + delta.not_reportable,
            labor,
            "{end}"
        );
        // The supplies vendor: 300.25 on the card, a 40.00 refund netted
        // on the worksheet and received in party totals.
        let paper = w
            .vendors
            .iter()
            .find(|v| l.party_of_vendor(v.vendor_id) == Some(b.paper_party))
            .unwrap();
        assert_eq!(
            (paper.paid, paper.excluded_card),
            (dec("-40.00"), dec("300.25"))
        );
        assert_eq!(
            activity(&b, b.paper_party, end).totals(Basis::Cash),
            (dec("300.25"), dec("40.00"))
        );
    }
}

/// The IIF's amounts on one account, through as_of (B-34: every posted
/// entry, closes left out), debit-positive.
fn iif_total(iif: &str, account: &str) -> Decimal {
    let mut total = Decimal::ZERO;
    for line in iif.split("\r\n") {
        let f: Vec<&str> = line.split('\t').collect();
        if (f[0] == "TRNS" || f[0] == "SPL") && f.get(4) == Some(&account) {
            total += dec(f[6]);
        }
    }
    total
}

/// Every date in the IIF, as written (MM/DD/YYYY).
fn iif_dates(iif: &str) -> Vec<NaiveDate> {
    iif.split("\r\n")
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            (f[0] == "TRNS" || f[0] == "SPL")
                .then(|| NaiveDate::parse_from_str(f[3], "%m/%d/%Y").unwrap())
        })
        .collect()
}

/// Depreciation is one figure everywhere (B-15 rule 8, B-14 rule 8,
/// B-36): the P&L's expense, the cash flow reconciliation's add-back, the
/// tax-basis K-1's open item, the IIF's, and the runs plus what the
/// disposal posted; the balance sheet's accumulated depreciation is the
/// register's posted figure, and posted differs from scheduled by exactly
/// what is still to be caught up.
#[test]
fn depreciation_is_one_figure_everywhere() {
    let dir = tempfile::tempdir().unwrap();
    let b = books(dir.path());
    let l = b.store.ledger();
    // (date, scheduled, posted): May's camera run, reversed on June 3, is
    // caught up by July's run; mid-September the month isn't run yet.
    let expect = [
        ("2026-06-30", "440.00", "365.00"),
        ("2026-09-15", "752.50", "690.00"),
        ("2026-12-31", "550.00", "550.00"),
    ];
    for (end, scheduled, posted) in expect {
        let end = d(end);
        let pnl = pnl_depreciation(l, end);
        let cf = cash_flow(&b, end);
        let addback: Decimal = cf
            .operating_reconciliation
            .lines
            .iter()
            .filter(|r| r.kind == ledger_core::ReconcilingKind::Depreciation)
            .map(|r| r.amount)
            .sum();
        assert_eq!(
            addback, pnl,
            "{end}: the cash flow's depreciation is the P&L's"
        );
        let tc = ledger_core::tax_capital::tax_basis_capital(
            l,
            d(START),
            end,
            &capitals(&b),
            &ids(l, &b.cash),
        )
        .unwrap();
        let open: Vec<Decimal> = tc
            .open
            .iter()
            .filter_map(|o| match o {
                Open::TaxDepreciation { book, .. } => Some(*book),
                _ => None,
            })
            .collect();
        assert_eq!(
            open,
            vec![pnl],
            "{end}: the tax-basis K-1's book depreciation"
        );
        let iif = ledger_package::export_iif(l, &b.cash, end).unwrap();
        assert_eq!(
            iif_total(&iif.text, "Depreciation"),
            pnl,
            "{end}: the IIF's"
        );
        assert!(iif_dates(&iif.text).iter().all(|x| *x <= end));
        let reg = ledger_core::fixed_asset_register(l, end).unwrap();
        assert_eq!(reg.total_accumulated_scheduled, dec(scheduled), "{end}");
        assert_eq!(reg.total_accumulated_posted, dec(posted), "{end}");
        assert_eq!(
            reg.ledger_accumulated, reg.total_accumulated_posted,
            "{end}"
        );
        let bs_accum = -(credit_balance(l, "1510", end) + credit_balance(l, "1530", end));
        assert_eq!(
            -bs_accum, reg.ledger_accumulated,
            "{end}: the balance sheet's"
        );
        for r in &reg.rows {
            if r.disposed.is_none() {
                assert_eq!(r.book_value, r.cost - r.accumulated_posted);
                assert_eq!(
                    r.accumulated_posted,
                    l.accumulated_posted(r.asset_id, end).unwrap(),
                    "list_assets and the viewer read Ledger::accumulated_posted"
                );
            }
        }
        // The runs plus the disposal's own depreciation lines are the
        // expense.
        let runs: Decimal = [b.laptop, b.camera]
            .iter()
            .map(|&a| l.depreciation_posted_between(a, d(START), end).unwrap())
            .sum();
        let disposal = if end >= d("2026-11-20") {
            dec("-25.00")
        } else {
            Decimal::ZERO
        };
        assert_eq!(runs + disposal, pnl, "{end}");
    }
    // By hand: the laptop 50.00 a month, February through December; the
    // camera 75.00 a month from April 15 through November 20, by days:
    // 40.00 + 6 × 75.00 + 50.00 = 540.00.
    assert_eq!(pnl_depreciation(l, d("2026-12-31")), dec("1090.00"));
    let cf = cash_flow(&b, d("2026-12-31"));
    let loss: Decimal = cf
        .operating_reconciliation
        .lines
        .iter()
        .filter(|r| r.kind == ledger_core::ReconcilingKind::InvestingGainLoss)
        .map(|r| r.amount)
        .sum();
    // Book value 3,600.00 − 540.00 = 3,060.00, sold for 2,900.00.
    assert_eq!(loss, dec("160.00"));
}

/// Footing on every date: the trial balance, the balance sheet, the cash
/// flow (sections, change in cash, the reconciliation to operating).
#[test]
fn every_statement_foots_on_every_date() {
    let dir = tempfile::tempdir().unwrap();
    let b = books(dir.path());
    let l = b.store.ledger();
    for end in DATES.map(d) {
        assert!(
            ledger_core::reports::trial_balance_as_of(l, end).balanced(),
            "{end}"
        );
        let bs = ledger_core::balance_sheet_as_of(l, end).unwrap();
        assert!(bs.balanced(), "{end}");
        let cf = cash_flow(&b, end);
        assert!(cf.reconciles.difference.is_zero());
        assert_eq!(
            cf.cash_from_operations + cf.cash_from_investing + cf.cash_from_financing,
            cf.net_change_in_cash
        );
        assert_eq!(cf.cash_ending - cf.cash_beginning, cf.net_change_in_cash);
        assert_eq!(cf.cash_ending, -credit_balance(l, "1000", end));
        assert_eq!(
            cf.operating_reconciliation.net_cash_from_operating,
            cf.cash_from_operations
        );
        let pnl = ledger_core::profit_and_loss_for_period(l, d(START), end, None).unwrap();
        assert_eq!(cf.operating_reconciliation.net_income, pnl.net_income());
        for section in CashFlowSection::ALL {
            let lines: Decimal = cf.lines(section).iter().map(|x| x.amount).sum();
            assert_eq!(lines, cf.total(section));
        }
    }
}

fn file<'p>(p: &'p ledger_package::Package, name: &str) -> &'p str {
    let (_, bytes) = p
        .files
        .iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("{name}"));
    std::str::from_utf8(bytes).unwrap()
}

/// The CPA package as of each date: every file stops at as_of, and its
/// figures are the tools' for the same accounts and dates (B-33).
#[test]
fn the_cpa_package_as_of_each_date_is_the_tools_figures() {
    let dir = tempfile::tempdir().unwrap();
    let b = books(dir.path());
    let l = b.store.ledger();
    for end in DATES.map(d) {
        let p = build_package(
            &b.store,
            &settings(&b),
            &PackageRequest {
                as_of: end,
                prepared_on: d("2027-01-15"),
                cash_accounts: &[],
                financing_accounts: &[],
                investing_accounts: &[],
                include_receipts: false,
                tins: None,
            },
        )
        .unwrap();
        // The K-1 worksheet's rows are k1_worksheet's for the year to date.
        let k1 = ledger_core::k1::k1_worksheet(l, d(START), end, &capitals(&b), &ids(l, &b.cash))
            .unwrap();
        let csv = file(&p, "k1-members.csv");
        for (row, k) in csv.lines().skip(1).zip(&k1) {
            let cells: Vec<&str> = row.split(',').collect();
            let figures: Vec<Decimal> = cells[cells.len() - 7..].iter().map(|c| dec(c)).collect();
            assert_eq!(
                figures,
                vec![
                    k.beginning,
                    k.contributions_cash,
                    k.contributions_property,
                    k.net_income,
                    k.distributions,
                    k.other,
                    k.ending
                ],
                "{end}"
            );
        }
        // The tax-basis worksheet's depreciation is the P&L's.
        let tax = file(&p, "k1-tax-basis-capital.csv");
        let pnl = pnl_depreciation(l, end);
        assert!(
            tax.contains(&format!(
                "the book depreciation posted ({} in the span",
                ledger_package::money(pnl)
            )),
            "{end}: {tax}"
        );
        // The 1099 worksheet is books.vendor_1099's CSV through as_of.
        let w =
            ledger_package::vendor_1099_worksheet(l, &settings(&b), 2026, None, Some(end), None)
                .unwrap();
        assert_eq!(
            file(&p, "1099-worksheet-2026.csv"),
            ledger_package::vendor_1099_csv(&w, false)
        );
        // The IIF is books.export's as of the day, and holds nothing after.
        let iif = ledger_package::export_iif(l, &b.cash, end).unwrap();
        assert_eq!(file(&p, "general-journal.iif"), iif.text);
        assert!(iif_dates(&iif.text).iter().all(|x| *x <= end));
        // The fixed asset register is in it: both assets were bought by
        // every date.
        assert!(
            p.files
                .iter()
                .any(|(n, _)| n == "fixed-asset-register.xlsx")
        );
    }
}
