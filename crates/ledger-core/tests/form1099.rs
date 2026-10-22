//! 1099 tracking. Numbers computed by hand. Payments count out of the
//! books' cash account (1000) and, where the books name one, their
//! financing liability (Due to Members); every name is fictional.
//!
//! `books()`, 2026 payments:
//! - Dana (1099-NEC, W-9): bill 1,500 paid by check + 700 direct = 2,200,
//!   over the 2,000 threshold. A 300 payment on 2025-12-30 counts in 2025.
//! - Ace Rentals (1099-MISC): 1,200 twice, one reversed = 1,200, under.
//! - Sam (1099-NEC): 5,000 charged to a card (2100: a liability no payment
//!   comes out of, and no payment method names it) = nothing counts.
//! - Bigco (never decided): 9,000 from the bank: undetermined, a decision
//!   is needed, never reportable until one is made (defect 18).

use chrono::NaiveDate;
use ledger_core::arap::DocumentLine;
use ledger_core::form1099::{Applied, default_threshold};
use ledger_core::kernel::{self, Bucket, PayRole, Tag};
use ledger_core::settings::Source;
use ledger_core::{
    AccountId, AccountType, Currency, Dated1099, Decision1099, EntryId, ExemptReason, Form1099,
    InstrumentKind, Ledger, LedgerError, Money, NewAsset, PaymentAccounts, Side, Status1099,
    Vendor1099Line, Vendor1099Report, VendorId, vendor_1099_report,
};
use proptest::prelude::*;
use rust_decimal::Decimal;
use std::str::FromStr;

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

/// Who decided every decision here (a fictional member).
fn src() -> Source {
    Source {
        decided_by: "Riley Chen".into(),
        decided_on: d(2026, 1, 5),
        citation: None,
        note: None,
    }
}

/// `decision` for `vendor` from the beginning, decided by Riley Chen.
fn decide(l: &mut Ledger, vendor: VendorId, decision: Decision1099, tin_on_file: bool) {
    l.set_vendor_1099(
        vendor,
        Dated1099::from_beginning(decision, Some(src())),
        tin_on_file,
    )
    .unwrap();
}

/// `decision` for `vendor` from `effective` on (keeping the W-9 flag).
fn decide_from(l: &mut Ledger, vendor: VendorId, effective: NaiveDate, decision: Decision1099) {
    let tin = l.get_vendor(vendor).unwrap().tin_on_file;
    l.set_vendor_1099(
        vendor,
        Dated1099 {
            effective: Some(effective),
            decision,
            source: Some(src()),
        },
        tin,
    )
    .unwrap();
}

fn nec() -> Decision1099 {
    Decision1099::Files(Form1099::Nec)
}

fn corp(reason: &str) -> Decision1099 {
    Decision1099::not_reportable(ExemptReason::Corporation, reason).unwrap()
}

/// The report out of the code 1000 cash account and `financing` codes.
fn report_for(
    l: &Ledger,
    year: i32,
    threshold: Option<Decimal>,
    financing: &[&str],
) -> Result<Vendor1099Report, LedgerError> {
    let id = |c: &str| l.chart().find_by_code(c).unwrap().id;
    vendor_1099_report(
        l,
        year,
        threshold,
        &PaymentAccounts {
            cash: vec![id("1000")],
            financing: financing.iter().map(|c| id(c)).collect(),
        },
        &[],
        None,
    )
}

fn line<'a>(r: &'a Vendor1099Report, name: &str) -> &'a Vendor1099Line {
    r.vendors
        .iter()
        .find(|v| v.name == name)
        .unwrap_or_else(|| panic!("{name} not on the worksheet: {r:?}"))
}

/// Post `amt` debited to `dr`, credited to `cr`, on `date`, tagged with
/// `vendor` when given.
fn pay(
    l: &mut Ledger,
    date: NaiveDate,
    vendor: Option<VendorId>,
    dr: AccountId,
    cr: AccountId,
    amt: &str,
) -> EntryId {
    let e = l.create_entry(date, "payment");
    let m = Money::new(dec(amt), Currency::USD);
    l.add_line(e, dr, Side::Debit, m).unwrap();
    l.add_line(e, cr, Side::Credit, m).unwrap();
    l.post_entry(e).unwrap();
    if let Some(v) = vendor {
        l.set_entry_vendor(e, Some(v)).unwrap();
    }
    e
}

fn books() -> Ledger {
    let mut l = Ledger::new(Currency::USD);
    let bank = l
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let card = l
        .add_account("2100", "Credit Card", AccountType::Liability, None, false)
        .unwrap();
    let contract = l
        .add_account("6000", "Contract Labor", AccountType::Expense, None, false)
        .unwrap();
    let rent = l
        .add_account("6600", "Rent", AccountType::Expense, None, false)
        .unwrap();
    let dana = l.add_vendor("Dana").unwrap();
    let ace = l.add_vendor("Ace Rentals").unwrap();
    let sam = l.add_vendor("Sam").unwrap();
    let bigco = l.add_vendor("Bigco").unwrap();
    decide(&mut l, dana, nec(), true);
    decide(&mut l, ace, Decision1099::Files(Form1099::Misc), false);
    decide(&mut l, sam, nec(), true);
    let bill = l
        .create_bill(
            dana,
            d(2026, 3, 1),
            d(2026, 3, 31),
            vec![DocumentLine {
                description: "Design work".into(),
                amount: dec("1500"),
                class_id: None,
            }],
        )
        .unwrap();
    l.pay_bill(bill, d(2026, 3, 20), dec("1500"), bank).unwrap();
    pay(&mut l, d(2026, 6, 1), Some(dana), contract, bank, "700");
    pay(&mut l, d(2025, 12, 30), Some(dana), contract, bank, "300");
    pay(&mut l, d(2026, 1, 1), Some(ace), rent, bank, "1200");
    let wrong = pay(&mut l, d(2026, 2, 1), Some(ace), rent, bank, "1200");
    l.reverse_entry(wrong, d(2026, 2, 2), "duplicate").unwrap();
    pay(&mut l, d(2026, 4, 1), Some(sam), contract, card, "5000");
    pay(&mut l, d(2026, 5, 1), Some(bigco), contract, bank, "9000");
    // Untagged spending is not anyone's 1099.
    pay(&mut l, d(2026, 7, 1), None, contract, bank, "50");
    l
}

#[test]
fn cash_paid_per_vendor_and_who_is_reportable() {
    let l = books();
    let r = report_for(&l, 2026, None, &[]).unwrap();
    assert_eq!(r.threshold, dec("2000"));
    let rows: Vec<(&str, Decimal, Status1099, bool)> = r
        .vendors
        .iter()
        .map(|v| (v.name.as_str(), v.paid, v.status, v.reportable))
        .collect();
    assert_eq!(
        rows,
        [
            (
                "Ace Rentals",
                dec("1200"),
                Status1099::UnderThreshold,
                false
            ),
            ("Bigco", dec("9000"), Status1099::Undetermined, false),
            ("Dana", dec("2200"), Status1099::Reportable, true),
            ("Sam", dec("0"), Status1099::UnderThreshold, false),
        ]
    );
    let ace = &r.vendors[0];
    assert_eq!(ace.form(), Some(Form1099::Misc));
    assert_eq!(ace.decided, Some(src()));
    assert!(!ace.tin_on_file);
    // The untagged 50 paid out for contract labor: the quieter note's one.
    assert_eq!(r.untagged_payments.len(), 1);
}

#[test]
fn years_and_thresholds() {
    let l = books();
    // SOURCE: rules.md B-19 (sources/irs-1099-thresholds.md; Pub. L.
    // 119-21 §70433(f): payments made after December 31, 2025).
    assert_eq!(default_threshold(2025), Ok(dec("600")));
    assert_eq!(default_threshold(2026), Ok(dec("2000")));
    let y2025 = report_for(&l, 2025, None, &[]).unwrap();
    let dana = line(&y2025, "Dana");
    assert_eq!(dana.paid, dec("300"));
    assert_eq!(dana.status, Status1099::UnderThreshold, "300 < 600");
    // An override: at 1,000, Ace's 1,200 is reportable.
    let r = report_for(&l, 2026, Some(dec("1000")), &[]).unwrap();
    assert!(line(&r, "Ace Rentals").reportable);
}

/// The threshold is met at exactly the amount ("of $2,000 or more",
/// §6041(a); "equals or exceeds", §6041A(a)(2)), each year's own (bookspec
/// 1099-the-threshold-is-the-years), and so is every bound on an unknown:
/// - 2025, $600: Riley 600.00 from the bank: reportable.
/// - 2026, $2,000: Riley 1,999.99: under; Cedar 2,000.00: reportable;
///   Bigco (never decided) 2,000.00: undetermined (a decision is needed);
///   Tobin 1,500.00 from the bank and 500.00 by a member, method not
///   recorded: 2,000.00 if it counts, so undetermined; 1,499.99 and
///   500.00: never more than 1,999.99, under.
#[test]
fn the_threshold_is_met_at_exactly_the_amount() {
    let mut m = members_books();
    let vendor = |l: &mut Ledger, name: &str, decision: Option<Decision1099>| {
        let v = l.add_vendor(name).unwrap();
        if let Some(d) = decision {
            decide(l, v, d, true);
        }
        v
    };
    let riley = vendor(&mut m.l, "Riley", Some(nec()));
    let cedar = vendor(&mut m.l, "Cedar", Some(nec()));
    let bigco = vendor(&mut m.l, "Bigco", None);
    let tobin = vendor(&mut m.l, "Tobin", Some(nec()));
    let ash = vendor(&mut m.l, "Ash", Some(nec()));
    let (labor, bank, due) = (m.labor, m.bank, m.due);
    for (date, v, cr, amt) in [
        (d(2025, 11, 1), riley, bank, "600.00"),
        (d(2026, 2, 1), riley, bank, "1999.99"),
        (d(2026, 2, 1), cedar, bank, "2000.00"),
        (d(2026, 2, 1), bigco, bank, "2000.00"),
        (d(2026, 2, 1), tobin, bank, "1500.00"),
        (d(2026, 3, 1), tobin, due, "500.00"),
        (d(2026, 2, 1), ash, bank, "1499.99"),
        (d(2026, 3, 1), ash, due, "500.00"),
    ] {
        pay(&mut m.l, date, Some(v), labor, cr, amt);
    }
    let r = report_for(&m.l, 2025, None, &["2100"]).unwrap();
    assert_eq!(line(&r, "Riley").status, Status1099::Reportable);
    let r = report_for(&m.l, 2026, None, &["2100"]).unwrap();
    let status = |n: &str| line(&r, n).status;
    assert_eq!(status("Riley"), Status1099::UnderThreshold);
    assert_eq!(status("Cedar"), Status1099::Reportable);
    assert_eq!(status("Bigco"), Status1099::Undetermined);
    assert_eq!(status("Tobin"), Status1099::Undetermined);
    assert_eq!(status("Ash"), Status1099::UnderThreshold);
}

/// The status never settles an undecided part of the year (review R3): a
/// vendor decided 1099-NEC, then undetermined from 2026-07-01 (its status
/// in question), paid 2,100.00 in March and refunded 200.00 in August.
/// Decided 1099-NEC, the August refund makes it 1,900.00, under; decided
/// not reportable, March's 2,100.00 stands, reportable: undetermined, and
/// no decision is asked for money that only came back.
#[test]
fn an_undecided_refund_never_settles_the_status() {
    let mut m = members_books();
    let v = m.l.add_vendor("Harbor Design LLC").unwrap();
    decide(&mut m.l, v, nec(), true);
    decide_from(&mut m.l, v, d(2026, 7, 1), Decision1099::Undetermined);
    let (labor, bank) = (m.labor, m.bank);
    pay(&mut m.l, d(2026, 3, 1), Some(v), labor, bank, "2100.00");
    pay(&mut m.l, d(2026, 8, 1), Some(v), bank, labor, "200.00");
    let r = report_for(&m.l, 2026, None, &["2100"]).unwrap();
    let h = line(&r, "Harbor Design LLC");
    assert_eq!(
        (h.paid, h.status, h.decision_needed),
        (dec("1900.00"), Status1099::Undetermined, false)
    );
}

#[test]
fn tags_need_real_entries_and_vendors() {
    let mut l = books();
    assert!(matches!(
        l.set_entry_vendor(999, Some(1)),
        Err(LedgerError::UnknownEntry(999))
    ));
    assert!(matches!(
        l.set_entry_vendor(1, Some(999)),
        Err(LedgerError::UnknownVendor(999))
    ));
    // The bill and its payment were tagged automatically.
    assert_eq!(l.entry_vendor(1), Some(1));
    assert_eq!(l.entry_vendor(2), Some(1));
    l.set_entry_vendor(2, None).unwrap();
    assert_eq!(l.entry_vendor(2), None);
    assert_eq!(Form1099::parse("1099-NEC"), Some(Form1099::Nec));
    assert_eq!(Form1099::parse("misc"), Some(Form1099::Misc));
    assert_eq!(Form1099::parse("K"), None);
}

/// Spec (owner, 2026-10-13): a rule set by law that wanderbooks doesn't
/// know for a year is never guessed. 26 U.S.C. §6041(h) indexes the $2,000
/// threshold for calendar years after 2026; until the indexed amount is in
/// the rule table, asking for it is an error that says what to do.
#[test]
fn an_unknown_year_threshold_is_never_guessed() {
    let err = default_threshold(2027).unwrap_err();
    let LedgerError::RuleNotKnown {
        rule,
        year,
        known_through,
        authority,
    } = &err
    else {
        panic!("expected RuleNotKnown, got {err:?}");
    };
    assert_eq!(
        (rule.as_str(), *year, *known_through),
        ("1099 reporting threshold", 2027, 2026)
    );
    assert!(authority.contains("https://www.irs.gov/"), "{authority}");
    let why = err.to_string();
    for needle in [
        "2027",
        "official source",
        "CPA",
        "irs.gov",
        "tell the human",
    ] {
        assert!(why.contains(needle), "missing {needle:?} in: {why}");
    }
    // It reaches agents: it names the authority, never this repo.
    for never in ["github", "pull request", "docs/", ".md"] {
        assert!(!why.to_lowercase().contains(never), "{never:?} in: {why}");
    }
    let l = books();
    assert!(matches!(
        report_for(&l, 2027, None, &[]),
        Err(LedgerError::RuleNotKnown { .. })
    ));
    // An explicit threshold (from the IRS, or the owner or CPA) works.
    let r = report_for(&l, 2027, Some(dec("2100")), &[]).unwrap();
    assert_eq!(r.threshold, dec("2100"));
}

/// P3 review R5 and R16: the worksheet never guesses where money is paid
/// from (every asset was a guess that counted a prepayment as nothing),
/// and never compares another currency against dollars.
#[test]
fn the_report_needs_the_cash_accounts_and_dollars() {
    let l = books();
    let err =
        vendor_1099_report(&l, 2026, None, &PaymentAccounts::default(), &[], None).unwrap_err();
    assert!(matches!(err, LedgerError::Invalid1099(_)), "{err:?}");
    assert!(err.to_string().contains("cash accounts"), "{err}");
    let mut eur = Ledger::new(Currency::new("EUR").unwrap());
    let bank = eur
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let sources = PaymentAccounts {
        cash: vec![bank],
        financing: vec![],
    };
    let err = vendor_1099_report(&eur, 2026, None, &sources, &[], None).unwrap_err();
    assert!(err.to_string().contains("US dollars"), "{err}");
    assert!(vendor_1099_report(&eur, 2026, Some(dec("1800")), &sources, &[], None).is_ok());
}

/// A report through a day (an as-of package, P3 review R9) counts the
/// year's payments on or before it only, and says so; through the year's
/// end or later it is the whole year. Dana's 2026: 1,500.00 on 03-20 and
/// 700.00 on 06-01 (books()).
#[test]
fn a_report_through_a_day_stops_there() {
    let l = books();
    let id = l.chart().find_by_code("1000").unwrap().id;
    let sources = PaymentAccounts {
        cash: vec![id],
        financing: vec![],
    };
    let through =
        |t: NaiveDate| vendor_1099_report(&l, 2026, None, &sources, &[], Some(t)).unwrap();
    let dana = |r: &Vendor1099Report| line(r, "Dana").paid;
    let r = through(d(2026, 5, 31));
    assert_eq!((dana(&r), r.through), (dec("1500"), Some(d(2026, 5, 31))));
    let r = through(d(2026, 6, 1));
    assert_eq!(dana(&r), dec("2200"), "a payment on the day counts");
    for t in [d(2026, 12, 31), d(2027, 3, 1)] {
        let r = through(t);
        assert_eq!((dana(&r), r.through), (dec("2200"), None));
    }
}

/// Dated rules are due for review from their review date, and stay due;
/// before it, nothing is stale. The engine has no clock: callers pass
/// today.
#[test]
fn dated_rules_go_stale_on_their_review_date() {
    use ledger_core::rules::{THRESHOLD_1099, stale_rules};
    assert!(stale_rules(d(2026, 10, 13)).is_empty());
    let due = THRESHOLD_1099.review_after();
    assert!(stale_rules(due.pred_opt().unwrap()).is_empty());
    assert_eq!(stale_rules(due).len(), 1);
    assert_eq!(
        stale_rules(d(2027, 1, 1))[0].name,
        "1099 reporting threshold"
    );
    assert_eq!(stale_rules(d(2030, 6, 1)).len(), 1);
}

#[test]
fn forms_have_their_irs_names() {
    assert_eq!(Form1099::Nec.label(), "1099-NEC");
    assert_eq!(Form1099::Misc.label(), "1099-MISC");
    // A label (and a key) parses back to its form.
    for f in [Form1099::Nec, Form1099::Misc] {
        assert_eq!(Form1099::parse(f.label()), Some(f));
        assert_eq!(Form1099::parse(f.key()), Some(f));
    }
}

/// vendor_1099_report SCENARIOS 1-2, worked by hand. Payment sources:
/// cash 1000 and the members' financing liability 2200. Kim (1099-MISC,
/// landlord) in 2026:
/// - 1,000 rent from the bank (Dr rent, Cr 1000): +1,000
/// - 2,400 prepaid rent from the bank (Dr 1300, Cr 1000): +2,400 (paid
///   when made; the prepaid asset is not a source, so it doesn't offset)
/// - 800 rent paid by a member (Dr rent, Cr 2200): undetermined until its
///   payment method is recorded
/// - 500 rent on the card (Dr rent, Cr 2100): nothing (no payment method
///   names 2100, and it is no payment source: the processor's 1099-K)
/// - 300 refunded to the bank (Dr 1000, Cr rent): −300
///
/// Counts 3,100, undetermined 800: reportable (3,100 is over 2,000 even
/// if the 800 doesn't count). The member's check: 3,900; their card:
/// 3,100 with 800 excluded.
#[test]
fn payments_come_from_cash_and_financing_only() {
    let mut l = Ledger::new(Currency::USD);
    let acct = |l: &mut Ledger, code: &str, t| l.add_account(code, code, t, None, false).unwrap();
    let bank = acct(&mut l, "1000", AccountType::Asset);
    let prepaid = acct(&mut l, "1300", AccountType::Asset);
    let card = acct(&mut l, "2100", AccountType::Liability);
    let members = acct(&mut l, "2200", AccountType::Liability);
    let rent = acct(&mut l, "6600", AccountType::Expense);
    let kim = l.add_vendor("Kim").unwrap();
    decide(&mut l, kim, Decision1099::Files(Form1099::Misc), true);
    let mut member_paid = 0;
    for (dr, cr, amt) in [
        (rent, bank, "1000"),
        (prepaid, bank, "2400"),
        (rent, members, "800"),
        (rent, card, "500"),
        (bank, rent, "300"),
    ] {
        let e = pay(&mut l, d(2026, 5, 1), Some(kim), dr, cr, amt);
        if cr == members {
            member_paid = e;
        }
    }
    let kim_line = |l: &Ledger| line(&report_for(l, 2026, None, &["2200"]).unwrap(), "Kim").clone();
    let k = kim_line(&l);
    assert_eq!((k.paid, k.undetermined), (dec("3100"), dec("800")));
    assert_eq!(k.status, Status1099::Reportable);
    let check = l
        .add_instrument(
            InstrumentKind::MemberCashOrCheck,
            "Member checks",
            None,
            None,
            members,
        )
        .unwrap();
    l.set_entry_instrument(member_paid, Some(check)).unwrap();
    let k = kim_line(&l);
    assert_eq!((k.paid, k.undetermined), (dec("3900"), dec("0")));
    let visa = l
        .add_instrument(
            InstrumentKind::MemberCard,
            "Member Visa",
            None,
            None,
            members,
        )
        .unwrap();
    l.set_entry_instrument(member_paid, Some(visa)).unwrap();
    let k = kim_line(&l);
    assert_eq!(
        (k.paid, k.excluded_card, k.undetermined),
        (dec("3100"), dec("800"), dec("0"))
    );
}

/// D-40's cutover, with made-up amounts: a vendor decided not reportable
/// (a corporation, from the public record) and one left undecided until a
/// W-9 shows what it is, each paid only with a member's personal card.
/// The first is not reportable, the second under the threshold, and
/// Needs attention asks about neither (nothing it could be decided
/// changes the year). Untagged, the undecided one's payment is
/// undetermined, so a decision is needed.
#[test]
fn card_only_vendors_need_no_decision() {
    let mut m = members_books();
    let visa =
        m.l.add_instrument(
            InstrumentKind::MemberCard,
            "Riley's Visa",
            None,
            None,
            m.due,
        )
        .unwrap();
    let anthropic = m.l.add_vendor("Anthropic, PBC").unwrap();
    decide(
        &mut m.l,
        anthropic,
        corp("a corporation (Delaware public benefit corporation, from the public record)"),
        false,
    );
    let google = m.l.add_vendor("Google LLC").unwrap();
    let (labor, due) = (m.labor, m.due);
    let a = pay(
        &mut m.l,
        d(2026, 4, 1),
        Some(anthropic),
        labor,
        due,
        "40.00",
    );
    let g = pay(&mut m.l, d(2026, 4, 2), Some(google), labor, due, "15.00");
    let rows = |l: &Ledger| -> Vec<(Decimal, Decimal, Status1099, bool)> {
        let r = report_for(l, 2026, None, &["2100"]).unwrap();
        ["Anthropic, PBC", "Google LLC"]
            .into_iter()
            .map(|n| {
                let v = line(&r, n);
                (v.excluded_card, v.undetermined, v.status, v.decision_needed)
            })
            .collect()
    };
    assert_eq!(
        rows(&m.l),
        [
            (dec("0"), dec("0"), Status1099::NotReportable, false),
            (dec("0"), dec("15.00"), Status1099::UnderThreshold, true),
        ]
    );
    m.l.set_entry_instrument(a, Some(visa)).unwrap();
    m.l.set_entry_instrument(g, Some(visa)).unwrap();
    assert_eq!(
        rows(&m.l),
        [
            (dec("40.00"), dec("0"), Status1099::NotReportable, false),
            (dec("15.00"), dec("0"), Status1099::UnderThreshold, false),
        ]
    );
}

/// Defect 18, from its reproduction: Bigco, never decided and paid 9,000,
/// and Bigco decided "not reportable: a corporation" are different lines.
/// reportable is false both times; the status (and where the 9,000 goes)
/// is what tells them apart.
#[test]
fn undecided_is_not_not_reportable() {
    let mut l = books();
    let r = report_for(&l, 2026, None, &[]).unwrap();
    let b = line(&r, "Bigco");
    assert_eq!(b.decision, Decision1099::Undetermined);
    assert_eq!(b.decided, None);
    assert_eq!(b.status, Status1099::Undetermined);
    assert!(b.decision_needed);
    assert_eq!((b.paid, b.not_reportable), (dec("9000"), dec("0")));
    assert!(!b.reportable);
    let bigco = b.vendor_id;
    decide(&mut l, bigco, corp("a corporation by name"), false);
    let r = report_for(&l, 2026, None, &[]).unwrap();
    let b = line(&r, "Bigco");
    assert_eq!(b.decision, corp("a corporation by name"));
    assert_eq!(b.decided, Some(src()));
    assert_eq!(b.status, Status1099::NotReportable);
    assert!(!b.decision_needed);
    assert_eq!((b.paid, b.not_reportable), (dec("0"), dec("9000")));
    assert!(!b.reportable);
}

/// The defect 12 books: bank 1000 (cash), Due to Members 2100
/// (financing), a card 2050, contract labor 6000; Dana (1099-NEC, W-9).
struct Members {
    l: Ledger,
    bank: AccountId,
    due: AccountId,
    card: AccountId,
    labor: AccountId,
    dana: VendorId,
}

fn members_books() -> Members {
    let mut l = Ledger::new(Currency::USD);
    let bank = l
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let card = l
        .add_account("2050", "Harbor Visa", AccountType::Liability, None, false)
        .unwrap();
    let due = l
        .add_account(
            "2100",
            "Due to Members",
            AccountType::Liability,
            None,
            false,
        )
        .unwrap();
    let labor = l
        .add_account("6000", "Contract Labor", AccountType::Expense, None, false)
        .unwrap();
    let dana = l.add_vendor("Dana").unwrap();
    decide(&mut l, dana, nec(), true);
    Members {
        l,
        bank,
        due,
        card,
        labor,
        dana,
    }
}

fn dana(m: &Members) -> Vendor1099Line {
    line(&report_for(&m.l, 2026, None, &["2100"]).unwrap(), "Dana").clone()
}

/// Defect 12, from its reproduction: a member paid Dana 2,500.00 for the
/// company (Dr 6000 / Cr 2100). With no payment method recorded it is
/// undetermined (their card would be excluded, their check would count);
/// tagged with the member's own card it is excluded, a card payment on
/// Form 1099-K whoever's card it was (Treas. Reg. §1.6041-1(a)(1)(iv));
/// tagged with their check it counts, the company being the payor
/// (§1.6041-1(e)(1)): reportable at 2,500.00 ≥ 2,000.
#[test]
fn a_members_personal_card_payment_is_not_a_1099_payment() {
    let mut m = members_books();
    let e1 = pay(
        &mut m.l,
        d(2026, 5, 1),
        Some(m.dana),
        m.labor,
        m.due,
        "2500.00",
    );
    let v = dana(&m);
    assert_eq!(
        (v.paid, v.excluded_card, v.undetermined),
        (dec("0"), dec("0"), dec("2500.00"))
    );
    assert_eq!(v.status, Status1099::Undetermined);
    let r = report_for(&m.l, 2026, None, &["2100"]).unwrap();
    assert_eq!(r.undetermined_payments.len(), 1);
    assert_eq!(
        (
            r.undetermined_payments[0].entry_id,
            r.undetermined_payments[0].amount
        ),
        (e1, dec("2500.00"))
    );
    let visa =
        m.l.add_instrument(
            InstrumentKind::MemberCard,
            "Riley's Visa",
            Some("4242"),
            None,
            m.due,
        )
        .unwrap();
    m.l.set_entry_instrument(e1, Some(visa)).unwrap();
    let v = dana(&m);
    assert_eq!(
        (v.paid, v.excluded_card, v.undetermined),
        (dec("0"), dec("2500.00"), dec("0"))
    );
    assert_eq!(v.status, Status1099::UnderThreshold);
    let r = report_for(&m.l, 2026, None, &["2100"]).unwrap();
    assert!(r.undetermined_payments.is_empty());
    assert_eq!(r.excluded_card_payments.len(), 1);
    assert_eq!(r.excluded_card_payments[0].instrument, Some(visa));
    let check =
        m.l.add_instrument(
            InstrumentKind::MemberCashOrCheck,
            "Riley's checks",
            None,
            None,
            m.due,
        )
        .unwrap();
    m.l.set_entry_instrument(e1, Some(check)).unwrap();
    let v = dana(&m);
    assert_eq!((v.paid, v.excluded_card), (dec("2500.00"), dec("0")));
    assert_eq!(v.status, Status1099::Reportable);
}

/// Defect 12, the card account: Sam's 5,000.00 charged to the company card
/// (Dr 6000 / Cr 2050). With the card recorded as a payment method, its
/// account's lines are card payments: excluded and listed, not dropped
/// silently. With no payment method naming 2050 it is no payment line at
/// all, as before.
#[test]
fn card_account_payments_are_listed_not_dropped() {
    let mut m = members_books();
    let sam = m.l.add_vendor("Sam").unwrap();
    decide(&mut m.l, sam, nec(), true);
    let e2 = pay(
        &mut m.l,
        d(2026, 6, 1),
        Some(sam),
        m.labor,
        m.card,
        "5000.00",
    );
    let sam_line =
        |m: &Members| line(&report_for(&m.l, 2026, None, &["2100"]).unwrap(), "Sam").clone();
    let s = sam_line(&m);
    assert_eq!((s.paid, s.excluded_card), (dec("0"), dec("0")));
    m.l.add_instrument(
        InstrumentKind::BusinessCard,
        "Harbor Visa",
        Some("0042"),
        None,
        m.card,
    )
    .unwrap();
    let s = sam_line(&m);
    assert_eq!((s.paid, s.excluded_card), (dec("0"), dec("5000.00")));
    let r = report_for(&m.l, 2026, None, &["2100"]).unwrap();
    assert_eq!(r.excluded_card_payments.len(), 1);
    let p = &r.excluded_card_payments[0];
    assert_eq!(
        (p.entry_id, p.amount, p.instrument),
        (e2, dec("5000.00"), None)
    );
}

/// A debit card is a payment card (IRS, "Understanding your Form 1099-K":
/// "Credit, debit or stored value cards"): a bank payment tagged with a
/// debit card drawn on that account is excluded. Untagged, the same
/// payment counts (presumed check, ACH or transfer: D-40), and so does
/// every other untagged bank payment, the card notwithstanding.
#[test]
fn a_debit_card_payment_from_the_bank_is_excluded_when_tagged() {
    let mut m = members_books();
    let e = pay(
        &mut m.l,
        d(2026, 2, 1),
        Some(m.dana),
        m.labor,
        m.bank,
        "1200.00",
    );
    pay(
        &mut m.l,
        d(2026, 3, 1),
        Some(m.dana),
        m.labor,
        m.bank,
        "300.00",
    );
    assert_eq!(dana(&m).paid, dec("1500.00"));
    let debit =
        m.l.add_instrument(
            InstrumentKind::BusinessCard,
            "Harbor debit card",
            None,
            None,
            m.bank,
        )
        .unwrap();
    assert_eq!(dana(&m).paid, dec("1500.00"), "untagged still counts");
    m.l.set_entry_instrument(e, Some(debit)).unwrap();
    let v = dana(&m);
    assert_eq!((v.paid, v.excluded_card), (dec("300.00"), dec("1200.00")));
}

/// Never guessed (B-19): only a company card's account presumes a card.
/// The members' account 2100, left out of the financing accounts, carries
/// a member's card; a platform's balance 1050 carries a payment platform.
/// Each also pays by check or friends-and-family transfer (counting), so
/// an untagged payment through either is undetermined, never excluded as
/// a card's: 2,500.00 and 300.00 undetermined. Tagged with the platform,
/// the 300.00 is excluded; with the member's card, the 2,500.00.
#[test]
fn a_members_or_a_platforms_account_is_never_presumed_a_card() {
    let mut m = members_books();
    let balance =
        m.l.add_account("1050", "Platform balance", AccountType::Asset, None, false)
            .unwrap();
    let visa =
        m.l.add_instrument(
            InstrumentKind::MemberCard,
            "Riley's Visa",
            None,
            None,
            m.due,
        )
        .unwrap();
    let pay_platform =
        m.l.add_instrument(InstrumentKind::Platform, "Harbor Pay", None, None, balance)
            .unwrap();
    let (labor, due, dana) = (m.labor, m.due, Some(m.dana));
    let member = pay(&mut m.l, d(2026, 5, 1), dana, labor, due, "2500.00");
    let network = pay(&mut m.l, d(2026, 6, 1), dana, labor, balance, "300.00");
    // No financing accounts named: 2100 is known only through the method.
    let line_of = |l: &Ledger| line(&report_for(l, 2026, None, &[]).unwrap(), "Dana").clone();
    let v = line_of(&m.l);
    assert_eq!(
        (v.paid, v.excluded_card, v.undetermined, v.status),
        (dec("0"), dec("0"), dec("2800.00"), Status1099::Undetermined)
    );
    m.l.set_entry_instrument(network, Some(pay_platform))
        .unwrap();
    m.l.set_entry_instrument(member, Some(visa)).unwrap();
    let v = line_of(&m.l);
    assert_eq!(
        (v.paid, v.excluded_card, v.undetermined, v.status),
        (
            dec("0"),
            dec("2800.00"),
            dec("0"),
            Status1099::UnderThreshold
        )
    );
}

/// P3 review R2: a tag names one account. A member's payment (Cr 2100)
/// tagged with the company card (whose account is 2050) isn't described
/// by the tag: undetermined.
#[test]
fn a_tag_naming_another_account_is_undetermined() {
    let mut m = members_books();
    let e = pay(
        &mut m.l,
        d(2026, 4, 1),
        Some(m.dana),
        m.labor,
        m.due,
        "700.00",
    );
    let visa =
        m.l.add_instrument(
            InstrumentKind::BusinessCard,
            "Harbor Visa",
            None,
            None,
            m.card,
        )
        .unwrap();
    m.l.set_entry_instrument(e, Some(visa)).unwrap();
    let v = dana(&m);
    assert_eq!(
        (v.paid, v.excluded_card, v.undetermined),
        (dec("0"), dec("0"), dec("700.00"))
    );
}

/// P3 review R1: money moved between the company's own sources pays no
/// one. A loan's principal paid to the lender (Dr 2500 Loan, a financing
/// account / Cr 1000, tagged with the lender) and a member reimbursed (Dr
/// 2100 / Cr 1000, tagged with Dana) each net to zero in every bucket,
/// though each line alone would count or be undetermined.
#[test]
fn paying_down_a_financing_liability_pays_no_vendor() {
    let mut m = members_books();
    let loan =
        m.l.add_account(
            "2500",
            "Equipment Loan",
            AccountType::Liability,
            None,
            false,
        )
        .unwrap();
    let lender = m.l.add_vendor("Harbor Equipment Finance").unwrap();
    decide(&mut m.l, lender, nec(), false);
    pay(
        &mut m.l,
        d(2026, 3, 1),
        Some(lender),
        loan,
        m.bank,
        "3000.00",
    );
    pay(
        &mut m.l,
        d(2026, 3, 2),
        Some(m.dana),
        m.due,
        m.bank,
        "80.00",
    );
    pay(
        &mut m.l,
        d(2026, 3, 3),
        Some(m.dana),
        m.labor,
        m.bank,
        "100.00",
    );
    let r = report_for(&m.l, 2026, None, &["2100", "2500"]).unwrap();
    let f = line(&r, "Harbor Equipment Finance");
    assert_eq!(
        (f.paid, f.excluded_card, f.undetermined),
        (dec("0"), dec("0"), dec("0"))
    );
    assert_eq!(f.status, Status1099::UnderThreshold);
    let v = line(&r, "Dana");
    assert_eq!((v.paid, v.undetermined), (dec("100.00"), dec("0")));
    assert!(r.undetermined_payments.is_empty());
}

/// P3 review R1: the card bill (Dr 2050, a card's account / Cr 1000),
/// tagged with the card's issuer, pays the issuer nothing toward a 1099.
#[test]
fn paying_the_card_bill_pays_no_vendor() {
    let mut m = members_books();
    m.l.add_instrument(
        InstrumentKind::BusinessCard,
        "Harbor Visa",
        None,
        None,
        m.card,
    )
    .unwrap();
    let issuer = m.l.add_vendor("Harbor Card Services").unwrap();
    decide(&mut m.l, issuer, nec(), false);
    pay(
        &mut m.l,
        d(2026, 5, 25),
        Some(issuer),
        m.card,
        m.bank,
        "1200.00",
    );
    let r = report_for(&m.l, 2026, None, &["2100"]).unwrap();
    let f = line(&r, "Harbor Card Services");
    assert_eq!(
        (f.paid, f.excluded_card, f.undetermined),
        (dec("0"), dec("0"), dec("0"))
    );
    assert!(r.excluded_card_payments.is_empty());
}

/// P3 review R3: an unknown that could go either way never settles the
/// status. Dana paid 3,000.00 from the bank, then 1,500.00 refunded to a
/// member (Dr 2100 / Cr 6000), how it was paid unknown: what counts is
/// between 1,500.00 and 3,000.00, so neither reportable nor under the
/// threshold. Refunded to the member's check: 1,500.00 counts, under;
/// to their card: 3,000.00 counts, reportable.
#[test]
fn a_negative_unknown_keeps_a_reportable_vendor_undetermined() {
    let mut m = members_books();
    pay(
        &mut m.l,
        d(2026, 2, 1),
        Some(m.dana),
        m.labor,
        m.bank,
        "3000.00",
    );
    let refund = pay(
        &mut m.l,
        d(2026, 3, 1),
        Some(m.dana),
        m.due,
        m.labor,
        "1500.00",
    );
    let v = dana(&m);
    assert_eq!((v.paid, v.undetermined), (dec("3000.00"), dec("-1500.00")));
    assert_eq!(v.status, Status1099::Undetermined);
    let check =
        m.l.add_instrument(
            InstrumentKind::MemberCashOrCheck,
            "Riley's checks",
            None,
            None,
            m.due,
        )
        .unwrap();
    m.l.set_entry_instrument(refund, Some(check)).unwrap();
    let v = dana(&m);
    assert_eq!(
        (v.paid, v.status),
        (dec("1500.00"), Status1099::UnderThreshold)
    );
    let visa =
        m.l.add_instrument(
            InstrumentKind::MemberCard,
            "Riley's Visa",
            None,
            None,
            m.due,
        )
        .unwrap();
    m.l.set_entry_instrument(refund, Some(visa)).unwrap();
    let v = dana(&m);
    assert_eq!(
        (v.paid, v.excluded_card, v.status),
        (dec("3000.00"), dec("-1500.00"), Status1099::Reportable)
    );
}

/// P3 review R6: money received from a contractor for something else
/// doesn't lower its 1099. Dana paid 3,000.00; a laptop sold to her for
/// 500.00 (Dr 1000 / Cr 4900 Other Income) and its reversal, and an old
/// monitor disposed of to her for 400.00 (Dr 1000 400, Dr 1510 600 / Cr
/// 1500 1,000, the asset's cost account), are left out and listed. A
/// refund of 200.00 (Dr 1000 / Cr 6000) nets: 2,800.00.
#[test]
fn a_sale_to_a_contractor_does_not_lower_its_1099() {
    let mut m = members_books();
    let acct = |l: &mut Ledger, code: &str, t| l.add_account(code, code, t, None, false).unwrap();
    let income = acct(&mut m.l, "4900", AccountType::Revenue);
    let equipment = acct(&mut m.l, "1500", AccountType::Asset);
    let accum = acct(&mut m.l, "1510", AccountType::Asset);
    let depr = acct(&mut m.l, "6800", AccountType::Expense);
    m.l.register_asset(NewAsset {
        description: "Monitor".into(),
        in_service_date: d(2024, 1, 1),
        cost: Money::new(dec("1000"), Currency::USD),
        salvage_value: Money::new(dec("0"), Currency::USD),
        useful_life_months: 36,
        cost_account: equipment,
        accum_depr_account: accum,
        expense_account: depr,
    })
    .unwrap();
    pay(
        &mut m.l,
        d(2026, 2, 1),
        Some(m.dana),
        m.labor,
        m.bank,
        "3000.00",
    );
    let sale = pay(
        &mut m.l,
        d(2026, 4, 1),
        Some(m.dana),
        m.bank,
        income,
        "500.00",
    );
    let undo =
        m.l.reverse_entry(sale, d(2026, 4, 2), "wrong buyer")
            .unwrap();
    let disposal = m.l.create_entry(d(2026, 5, 1), "monitor sold");
    for (a, side, amt) in [
        (m.bank, Side::Debit, "400.00"),
        (accum, Side::Debit, "600.00"),
        (equipment, Side::Credit, "1000.00"),
    ] {
        m.l.add_line(disposal, a, side, Money::new(dec(amt), Currency::USD))
            .unwrap();
    }
    m.l.post_entry(disposal).unwrap();
    m.l.set_entry_vendor(disposal, Some(m.dana)).unwrap();
    pay(
        &mut m.l,
        d(2026, 6, 1),
        Some(m.dana),
        m.bank,
        m.labor,
        "200.00",
    );
    let r = report_for(&m.l, 2026, None, &["2100"]).unwrap();
    let v = line(&r, "Dana");
    assert_eq!(v.paid, dec("2800.00"));
    assert_eq!(v.status, Status1099::Reportable);
    let other: Vec<(EntryId, Decimal)> = r
        .other_receipts
        .iter()
        .map(|p| (p.entry_id, p.amount))
        .collect();
    assert_eq!(
        other,
        [
            (sale, dec("-500.00")),
            (undo, dec("500.00")),
            (disposal, dec("-400.00"))
        ]
    );
}

/// P3 review R6, money in that is no refund: only what gives back what a
/// payment paid for nets. Dana, a contractor who is also a customer,
/// paid 3,000.00 from the bank. Her payment of an invoice (Dr 1000 / Cr
/// 1200 Accounts Receivable, 450.00) and a loan she made the company (Dr
/// 1000 / Cr 2600 Loan payable, 1,000.00) are money received for
/// something else: left out and listed. A 500.00 deposit paid her (Dr 1400
/// / Cr 1000) counts when paid, and its return (Dr 1000 / Cr 1400) nets as
/// a refund. 3,000.00 counts: reportable (netting the receipts would have
/// left 1,550.00, under the threshold).
#[test]
fn money_from_a_vendor_that_is_no_refund_does_not_lower_its_1099() {
    let mut m = members_books();
    let acct = |l: &mut Ledger, code: &str, t| l.add_account(code, code, t, None, false).unwrap();
    let ar = acct(&mut m.l, "1200", AccountType::Asset);
    let deposit = acct(&mut m.l, "1400", AccountType::Asset);
    let loan = acct(&mut m.l, "2600", AccountType::Liability);
    assert_eq!(m.l.role_account(&ledger_core::RECEIVABLES), Some(ar));
    let (bank, labor, dana) = (m.bank, m.labor, Some(m.dana));
    pay(&mut m.l, d(2026, 2, 1), dana, labor, bank, "3000.00");
    let invoice = pay(&mut m.l, d(2026, 3, 1), dana, bank, ar, "450.00");
    let lent = pay(&mut m.l, d(2026, 4, 1), dana, bank, loan, "1000.00");
    pay(&mut m.l, d(2026, 5, 1), dana, deposit, bank, "500.00");
    pay(&mut m.l, d(2026, 6, 1), dana, bank, deposit, "500.00");
    let r = report_for(&m.l, 2026, None, &["2100"]).unwrap();
    let v = line(&r, "Dana");
    assert_eq!((v.paid, v.status), (dec("3000.00"), Status1099::Reportable));
    let other: Vec<(EntryId, Decimal)> = r
        .other_receipts
        .iter()
        .map(|p| (p.entry_id, p.amount))
        .collect();
    assert_eq!(other, [(invoice, dec("-450.00")), (lent, dec("-1000.00"))]);
}

/// D-40, a status change mid-year: Harbor Design LLC, 1099-NEC from the
/// beginning, elects to be taxed as a corporation effective 2026-07-01
/// (same EIN, same vendor). Paid from the bank 1,500.00 on 03-01 and
/// 1,000.00 on 06-30 (while NEC: 2,500.00, reportable) and 800.00 on 07-01
/// and 1,200.00 on 09-15 (while not reportable: 2,000.00).
#[test]
fn a_status_change_mid_year_applies_by_payment_date() {
    let mut m = members_books();
    let harbor = m.l.add_vendor("Harbor Design LLC").unwrap();
    decide(&mut m.l, harbor, nec(), true);
    let elected = corp("elected to be taxed as a corporation (Form 8832)");
    decide_from(&mut m.l, harbor, d(2026, 7, 1), elected.clone());
    for (date, amt) in [
        (d(2026, 3, 1), "1500.00"),
        (d(2026, 6, 30), "1000.00"),
        (d(2026, 7, 1), "800.00"),
        (d(2026, 9, 15), "1200.00"),
    ] {
        pay(&mut m.l, date, Some(harbor), m.labor, m.bank, amt);
    }
    let r = report_for(&m.l, 2026, None, &["2100"]).unwrap();
    let h = line(&r, "Harbor Design LLC");
    assert_eq!((h.paid, h.not_reportable), (dec("2500.00"), dec("2000.00")));
    assert_eq!(h.status, Status1099::Reportable);
    assert_eq!(h.decision, elected);
    assert_eq!(h.effective, Some(d(2026, 7, 1)));
    let changes: Vec<_> = h
        .decisions
        .iter()
        .map(|x| (x.effective, x.decision.clone()))
        .collect();
    assert_eq!(changes, [(None, nec()), (Some(d(2026, 7, 1)), elected)]);
    // 2027 (the threshold passed in): all of it not reportable.
    pay(
        &mut m.l,
        d(2027, 2, 1),
        Some(harbor),
        m.labor,
        m.bank,
        "5000.00",
    );
    let r = report_for(&m.l, 2027, Some(dec("2000")), &["2100"]).unwrap();
    let h = line(&r, "Harbor Design LLC");
    assert_eq!((h.paid, h.not_reportable), (dec("0"), dec("5000.00")));
    assert_eq!(h.status, Status1099::NotReportable);
    assert_eq!(h.decisions.len(), 1);
}

/// D-40, two changes in one year, and a reversal crossing them: Tidepool
/// Studio, 1099-NEC from the beginning, 1099-MISC from 2026-03-01 (it
/// rents the company space), not reportable from 2026-09-01 (a corporate
/// election). Paid 2,500.00 on 02-01, 700.00 on 04-01 and 900.00 on 10-01
/// (each in force on its date: NEC, MISC, not reportable). The February
/// payment, reversed on 10-15 (after both changes), takes February's
/// decision and nets to nothing: 700.00 counts, under the threshold;
/// 900.00 not reportable. The year shows all three decisions in order.
#[test]
fn two_status_changes_in_a_year_and_a_reversal_across_them() {
    let mut m = members_books();
    let v = m.l.add_vendor("Tidepool Studio").unwrap();
    decide(&mut m.l, v, nec(), true);
    let misc = Decision1099::Files(Form1099::Misc);
    decide_from(&mut m.l, v, d(2026, 3, 1), misc.clone());
    let elected = corp("elected to be taxed as a corporation");
    decide_from(&mut m.l, v, d(2026, 9, 1), elected.clone());
    let (labor, bank) = (m.labor, m.bank);
    let feb = pay(&mut m.l, d(2026, 2, 1), Some(v), labor, bank, "2500.00");
    pay(&mut m.l, d(2026, 4, 1), Some(v), labor, bank, "700.00");
    pay(&mut m.l, d(2026, 10, 1), Some(v), labor, bank, "900.00");
    let r = report_for(&m.l, 2026, None, &["2100"]).unwrap();
    assert_eq!(line(&r, "Tidepool Studio").status, Status1099::Reportable);
    m.l.reverse_entry(feb, d(2026, 10, 15), "paid in error")
        .unwrap();
    let r = report_for(&m.l, 2026, None, &["2100"]).unwrap();
    let t = line(&r, "Tidepool Studio");
    assert_eq!(
        (t.paid, t.not_reportable, t.status),
        (dec("700.00"), dec("900.00"), Status1099::UnderThreshold)
    );
    let changes: Vec<_> = t
        .decisions
        .iter()
        .map(|x| (x.effective, x.decision.clone()))
        .collect();
    assert_eq!(
        changes,
        [
            (None, nec()),
            (Some(d(2026, 3, 1)), misc),
            (Some(d(2026, 9, 1)), elected.clone())
        ]
    );
    assert_eq!(
        (t.decision.clone(), t.effective),
        (elected, Some(d(2026, 9, 1)))
    );
    // A change on the year's last day is one of the year's, and its
    // decision the year's last; the next year opens with it.
    let back = Decision1099::Files(Form1099::Nec);
    decide_from(&mut m.l, v, d(2026, 12, 31), back.clone());
    let r = report_for(&m.l, 2026, None, &["2100"]).unwrap();
    let t = line(&r, "Tidepool Studio");
    assert_eq!(t.decisions.len(), 4);
    assert_eq!(
        (t.decision.clone(), t.effective),
        (back.clone(), Some(d(2026, 12, 31)))
    );
    let r = report_for(&m.l, 2027, Some(dec("2000")), &["2100"]).unwrap();
    let t = line(&r, "Tidepool Studio");
    assert_eq!(
        t.decisions
            .iter()
            .map(|x| x.decision.clone())
            .collect::<Vec<_>>(),
        [back]
    );
}

/// D-40, a payee that incorporates on December 30 is a new legal person:
/// a new vendor record with its own W-9 and decision. Riley Chen (sole
/// proprietor, 1099-NEC) was paid 1,900.00 on 12-29, under the threshold;
/// Riley Chen Design, Inc. (a corporation by name) 500.00 on 12-30 and
/// 300.00 on 12-31: 800.00 not reportable. Neither is reportable, and the
/// one doesn't add to the other.
#[test]
fn an_incorporation_on_december_30_is_a_new_vendor() {
    let mut m = members_books();
    let riley = m.l.add_vendor("Riley Chen").unwrap();
    decide(&mut m.l, riley, nec(), true);
    let inc = m.l.add_vendor("Riley Chen Design, Inc.").unwrap();
    decide(&mut m.l, inc, corp("a corporation by name"), false);
    pay(
        &mut m.l,
        d(2026, 12, 29),
        Some(riley),
        m.labor,
        m.bank,
        "1900.00",
    );
    pay(
        &mut m.l,
        d(2026, 12, 30),
        Some(inc),
        m.labor,
        m.bank,
        "500.00",
    );
    pay(
        &mut m.l,
        d(2026, 12, 31),
        Some(inc),
        m.labor,
        m.bank,
        "300.00",
    );
    let r = report_for(&m.l, 2026, None, &["2100"]).unwrap();
    let a = line(&r, "Riley Chen");
    assert_eq!(
        (a.paid, a.status),
        (dec("1900.00"), Status1099::UnderThreshold)
    );
    assert!(a.tin_on_file);
    let b = line(&r, "Riley Chen Design, Inc.");
    assert_eq!(
        (b.paid, b.not_reportable, b.status),
        (dec("0"), dec("800.00"), Status1099::NotReportable)
    );
    assert!(!b.tin_on_file);
}

/// D-40, an LLC electing corporate taxation keeps its EIN and its vendor
/// record: its status changes on the election's effective date
/// (§301.7701-3(c)(1)(iii)), here 2026-12-30. 2,100.00 on 12-15 counts
/// (reportable); 400.00 on 12-30 doesn't. Its 12-15 payment reversed on
/// 2027-01-05 takes the decision in force on the payment's own date
/// (1099-NEC), so 2027 counts −2,100.00, not "not reportable".
#[test]
fn an_llc_electing_corporate_taxation_keeps_its_vendor() {
    let mut m = members_books();
    let harbor = m.l.add_vendor("Harbor Design LLC").unwrap();
    decide(&mut m.l, harbor, nec(), true);
    decide_from(
        &mut m.l,
        harbor,
        d(2026, 12, 30),
        corp("elected S corporation status"),
    );
    let paid = pay(
        &mut m.l,
        d(2026, 12, 15),
        Some(harbor),
        m.labor,
        m.bank,
        "2100.00",
    );
    pay(
        &mut m.l,
        d(2026, 12, 30),
        Some(harbor),
        m.labor,
        m.bank,
        "400.00",
    );
    let r = report_for(&m.l, 2026, None, &["2100"]).unwrap();
    let h = line(&r, "Harbor Design LLC");
    assert_eq!(
        (h.paid, h.not_reportable, h.status),
        (dec("2100.00"), dec("400.00"), Status1099::Reportable)
    );
    m.l.reverse_entry(paid, d(2027, 1, 5), "paid twice")
        .unwrap();
    let r = report_for(&m.l, 2027, Some(dec("2000")), &["2100"]).unwrap();
    let h = line(&r, "Harbor Design LLC");
    assert_eq!(
        (h.paid, h.not_reportable, h.status),
        (dec("-2100.00"), dec("0"), Status1099::UnderThreshold)
    );
}

/// D-40: a dated decision records a change. With nothing decided before
/// its date it is refused (it would leave every earlier payment
/// undetermined); the same as the decision before its date changes
/// nothing; undoing a change takes it out.
#[test]
fn a_dated_decision_records_a_change() {
    let mut m = members_books();
    let v = m.l.add_vendor("Tidepool Studio").unwrap();
    let dated = |decision| Dated1099 {
        effective: Some(d(2026, 6, 1)),
        decision,
        source: Some(src()),
    };
    let err = m.l.set_vendor_1099(v, dated(nec()), false).unwrap_err();
    assert!(matches!(err, LedgerError::Invalid1099(_)), "{err:?}");
    assert!(err.to_string().contains("from the beginning"), "{err}");
    assert!(m.l.get_vendor(v).unwrap().decisions_1099.is_empty());
    decide(&mut m.l, v, nec(), false);
    assert_eq!(
        m.l.set_vendor_1099(v, dated(nec()), false),
        Ok(Applied::Unchanged)
    );
    assert_eq!(
        m.l.set_vendor_1099(v, dated(corp("elected")), false),
        Ok(Applied::Set)
    );
    assert_eq!(m.l.get_vendor(v).unwrap().decisions_1099.len(), 2);
    assert_eq!(
        m.l.set_vendor_1099(v, dated(nec()), false),
        Ok(Applied::Removed)
    );
    assert_eq!(m.l.get_vendor(v).unwrap().decisions_1099.len(), 1);
    // A decision for a date counts nothing decided before it, its own
    // date's included: with the status from the beginning back to
    // undetermined, the 06-01 change can't be replaced either.
    m.l.set_vendor_1099(v, dated(corp("elected")), false)
        .unwrap();
    decide(&mut m.l, v, Decision1099::Undetermined, false);
    assert!(m.l.set_vendor_1099(v, dated(nec()), false).is_err());
    // A decision needs sound words and a sound source.
    let bad = Dated1099::from_beginning(
        Decision1099::NotReportable {
            kind: ExemptReason::Other,
            reason: "".into(),
        },
        Some(src()),
    );
    assert!(m.l.set_vendor_1099(v, bad, false).is_err());
    let unnamed = Dated1099::from_beginning(
        nec(),
        Some(Source {
            decided_by: " ".into(),
            ..src()
        }),
    );
    assert!(m.l.set_vendor_1099(v, unnamed, false).is_err());
    assert!(matches!(
        m.l.set_vendor_1099(999, Dated1099::from_beginning(nec(), Some(src())), false),
        Err(LedgerError::UnknownVendor(999))
    ));
}

/// D-40's warning names what a change reaches: recording "not reportable
/// from 2026-07-01" changes the decision for the July and September
/// payments, not March's; from the beginning, all three; recorded again
/// exactly so, none.
#[test]
fn a_decision_change_names_the_payments_it_reaches() {
    let mut m = members_books();
    let harbor = m.l.add_vendor("Harbor Design LLC").unwrap();
    decide(&mut m.l, harbor, nec(), true);
    let e: Vec<EntryId> = [d(2026, 3, 1), d(2026, 7, 1), d(2026, 9, 15)]
        .into_iter()
        .map(|date| pay(&mut m.l, date, Some(harbor), m.labor, m.bank, "100.00"))
        .collect();
    let from_july = Dated1099 {
        effective: Some(d(2026, 7, 1)),
        decision: corp("elected"),
        source: Some(src()),
    };
    let reached = |l: &Ledger, new: &Dated1099| -> Vec<EntryId> {
        ledger_core::form1099::decision_changes(l, harbor, new)
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect()
    };
    assert_eq!(reached(&m.l, &from_july), [e[1], e[2]]);
    let always = Dated1099::from_beginning(corp("elected"), Some(src()));
    assert_eq!(reached(&m.l, &always), e);
    m.l.set_vendor_1099(harbor, from_july.clone(), true)
        .unwrap();
    assert!(reached(&m.l, &from_july).is_empty());
}

/// Plan 5.5 / D-40: a decision written before decisions were kept (no
/// date, no source) is in force from the beginning: a 2020 payment to a
/// vendor whose old line says 1099-NEC counts; an old null reads as
/// undetermined, never as "not reportable".
#[test]
fn an_undated_old_decision_is_in_force_from_the_beginning() {
    let mut m = members_books();
    let old = m.l.add_vendor("Ledgerline Bookkeeping").unwrap();
    let blank = m.l.add_vendor("Pinecrest Supply").unwrap();
    m.l.replay_vendor_1099(old, Dated1099::from_beginning(nec(), None), true)
        .unwrap();
    m.l.replay_vendor_1099(
        blank,
        Dated1099::from_beginning(Decision1099::Undetermined, None),
        false,
    )
    .unwrap();
    pay(
        &mut m.l,
        d(2020, 5, 1),
        Some(old),
        m.labor,
        m.bank,
        "700.00",
    );
    pay(
        &mut m.l,
        d(2020, 5, 2),
        Some(blank),
        m.labor,
        m.bank,
        "900.00",
    );
    let r = report_for(&m.l, 2020, None, &["2100"]).unwrap();
    let a = line(&r, "Ledgerline Bookkeeping");
    assert_eq!(
        (a.paid, a.status, a.decided.clone()),
        (dec("700.00"), Status1099::Reportable, None)
    );
    let b = line(&r, "Pinecrest Supply");
    assert_eq!(
        b.status,
        Status1099::Undetermined,
        "900 ≥ 600 and undecided"
    );
    assert!(b.decision_needed);
}

/// kernel::payment_bucket, every one of its 16 cases against the B-19
/// table (a complete check, not a sample).
#[test]
fn payment_bucket_is_the_table_in_all_16_cases() {
    use Bucket::*;
    let roles = [
        PayRole::Cash,
        PayRole::Financing,
        PayRole::CardAccount,
        PayRole::Other,
    ];
    let tags = [Tag::None, Tag::ThisCard, Tag::ThisOther, Tag::Elsewhere];
    let want = |role: PayRole, tag: Tag| match (tag, role) {
        (Tag::ThisCard, _) => Some(Card),
        (Tag::ThisOther, _) => Some(Counts),
        (_, PayRole::Cash) => Some(Counts),
        (_, PayRole::Financing) => Some(Undetermined),
        (_, PayRole::CardAccount) => Some(Card),
        (_, PayRole::Other) => None,
    };
    let mut n = 0;
    for role in roles {
        for tag in tags {
            assert_eq!(
                kernel::payment_bucket(role, tag),
                want(role, tag),
                "{role:?} {tag:?}"
            );
            n += 1;
        }
    }
    assert_eq!(n, 16);
    // Spelled out, the cases that decide money: an untagged member payment
    // is undetermined, a card account's line is a card's, a bank line
    // counts, and a tag elsewhere changes none of them.
    assert_eq!(
        kernel::payment_bucket(PayRole::Financing, Tag::None),
        Some(Undetermined)
    );
    assert_eq!(
        kernel::payment_bucket(PayRole::Financing, Tag::Elsewhere),
        Some(Undetermined)
    );
    assert_eq!(
        kernel::payment_bucket(PayRole::CardAccount, Tag::None),
        Some(Card)
    );
    assert_eq!(
        kernel::payment_bucket(PayRole::Cash, Tag::Elsewhere),
        Some(Counts)
    );
    assert_eq!(kernel::payment_bucket(PayRole::Other, Tag::Elsewhere), None);
}

/// kernel::split_payment and net_buckets, by hand.
#[test]
fn buckets_net_money_moved_between_the_companys_sources() {
    use Bucket::*;
    let l = [
        (Some(Counts), Side::Credit, 1000i64),
        (Some(Card), Side::Debit, 1000),
        (None, Side::Debit, 5),
    ];
    assert_eq!(kernel::split_payment(&l), Some([1000, -1000, 0]));
    assert_eq!(kernel::net_buckets([1000, -1000, 0]), Some([0, 0, 0]));
    // One bucket with the net's sign takes it.
    assert_eq!(kernel::net_buckets([1000, -300, 0]), Some([700, 0, 0]));
    assert_eq!(kernel::net_buckets([-200, 0, 500]), Some([0, 0, 300]));
    // Two with it: unknown which the other offsets.
    assert_eq!(kernel::net_buckets([600, 300, -100]), Some([0, 0, 800]));
    // One sign: unchanged.
    assert_eq!(kernel::net_buckets([600, 300, 0]), Some([600, 300, 0]));
    assert_eq!(kernel::net_buckets([-6, 0, -1]), Some([-6, 0, -1]));
    assert_eq!(kernel::net_buckets([i64::MAX, i64::MAX, -1]), None);
}

/// crate::instruments INVARIANTS 1-4: each refusal, one at a time, and
/// nothing added by any of them.
#[test]
fn a_payment_method_is_refused_unless_sound() {
    let mut m = members_books();
    let ar =
        m.l.add_account(
            "1200",
            "Accounts Receivable",
            AccountType::Asset,
            None,
            false,
        )
        .unwrap();
    let ap =
        m.l.add_account(
            "2000",
            "Accounts Payable",
            AccountType::Liability,
            None,
            false,
        )
        .unwrap();
    let header =
        m.l.add_account("1001", "Cash accounts", AccountType::Asset, None, true)
            .unwrap();
    let equity =
        m.l.add_account("3000", "Capital", AccountType::Equity, None, false)
            .unwrap();
    let riley =
        m.l.add_party("Riley Chen", &[ledger_core::Role::Member])
            .unwrap();
    let k = InstrumentKind::MemberCard;
    m.l.add_instrument(k, "Riley's Visa", Some("4242"), Some(riley), m.due)
        .unwrap();
    let long = "x".repeat(ledger_core::settings::MAX_TEXT + 1);
    let refused: Vec<(&str, Option<&str>, Option<u64>, AccountId)> = vec![
        ("", None, None, m.due),
        ("two\nlines", None, None, m.due),
        ("TIN 123-45-6789", None, None, m.due),
        (long.as_str(), None, None, m.due),
        ("  riley's visa ", None, None, m.due),
        ("Other Visa", Some("123"), None, m.due),
        ("Other Visa", Some("4111111111111111"), None, m.due),
        ("Other Visa", None, Some(999), m.due),
        ("Other Visa", None, None, 999),
        ("Other Visa", None, None, header),
        ("Other Visa", None, None, equity),
        ("Other Visa", None, None, m.labor),
        ("Other Visa", None, None, ap),
        ("Other Visa", None, None, ar),
    ];
    for (label, last4, owner, account) in refused {
        let r = m.l.add_instrument(k, label, last4, owner, account);
        assert!(r.is_err(), "{label:?} {last4:?} {owner:?} {account}");
    }
    assert_eq!(m.l.instruments().count(), 1);
    assert_eq!(m.l.next_instrument_id(), 2);
    assert_eq!(m.l.instrument_by_label("RILEY'S VISA").unwrap().id, 1);
}

/// set_entry_instrument SCENARIOS 1-3: the tag is metadata on drafts and
/// posted entries alike, carried through the reversal chain both ways,
/// copied to a reversal made later, cleared the same way; an unknown
/// entry or method is refused.
#[test]
fn a_payment_method_tags_the_whole_reversal_chain() {
    let mut m = members_books();
    let visa =
        m.l.add_instrument(
            InstrumentKind::MemberCard,
            "Riley's Visa",
            None,
            None,
            m.due,
        )
        .unwrap();
    let e = pay(
        &mut m.l,
        d(2026, 5, 1),
        Some(m.dana),
        m.labor,
        m.due,
        "250.00",
    );
    let rev = m.l.reverse_entry(e, d(2026, 5, 2), "oops").unwrap();
    let figures = ledger_core::reports::snapshot(&m.l).unwrap();
    m.l.set_entry_instrument(rev, Some(visa)).unwrap();
    assert_eq!(m.l.entry_instrument(e), Some(visa));
    assert_eq!(m.l.entry_instrument(rev), Some(visa));
    assert_eq!(
        ledger_core::reports::snapshot(&m.l).unwrap(),
        figures,
        "no amount moved"
    );
    let re =
        m.l.reverse_entry(rev, d(2026, 5, 3), "it was right")
            .unwrap();
    assert_eq!(
        m.l.entry_instrument(re),
        Some(visa),
        "the reversal step copies it"
    );
    m.l.set_entry_instrument(e, None).unwrap();
    assert_eq!(
        [
            m.l.entry_instrument(e),
            m.l.entry_instrument(rev),
            m.l.entry_instrument(re)
        ],
        [None, None, None]
    );
    let draft = m.l.create_entry(d(2026, 6, 1), "draft");
    m.l.set_entry_instrument(draft, Some(visa)).unwrap();
    assert_eq!(m.l.entry_instrument(draft), Some(visa));
    assert!(matches!(
        m.l.set_entry_instrument(999, Some(visa)),
        Err(LedgerError::UnknownEntry(999))
    ));
    assert!(matches!(
        m.l.set_entry_instrument(e, Some(99)),
        Err(LedgerError::UnknownInstrument(99))
    ));
}

/// Books shared by the shared-payment tests: bank 1000, card 2050,
/// contract labor 6000; Dana (1099-NEC) and member Tobin as parties.
struct Shared {
    l: Ledger,
    bank: AccountId,
    card: AccountId,
    labor: AccountId,
    dana: VendorId,
    dana_p: ledger_core::PartyId,
    tobin: ledger_core::PartyId,
}

fn shared_books() -> Shared {
    let mut l = Ledger::new(Currency::USD);
    let bank = l
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let card = l
        .add_account("2050", "Card", AccountType::Liability, None, false)
        .unwrap();
    let labor = l
        .add_account("6000", "Contract Labor", AccountType::Expense, None, false)
        .unwrap();
    let dana = l.add_vendor("Dana").unwrap();
    decide(&mut l, dana, nec(), true);
    let dana_p = l.party_of_vendor(dana).unwrap();
    let tobin = l.add_party("Tobin", &[ledger_core::Role::Member]).unwrap();
    Shared {
        l,
        bank,
        card,
        labor,
        dana,
        dana_p,
        tobin,
    }
}

/// Post `lines` on `date` and name `shares` on it.
fn post_shared(
    s: &mut Shared,
    date: NaiveDate,
    lines: &[(AccountId, Side, &str)],
    shares: &[(ledger_core::PartyId, ledger_core::Share)],
) -> EntryId {
    let e = s.l.create_entry(date, "shared payment");
    for (a, side, amt) in lines {
        s.l.add_line(e, *a, *side, Money::new(dec(amt), Currency::USD))
            .unwrap();
    }
    s.l.post_entry(e).unwrap();
    let shares: Vec<ledger_core::PartyShare> = shares
        .iter()
        .map(|&(party, share)| ledger_core::PartyShare { party, share })
        .collect();
    s.l.set_entry_parties(e, &shares).unwrap();
    e
}

fn dana_paid(s: &Shared) -> (Decimal, Vec<EntryId>) {
    let r = report_for(&s.l, 2026, None, &[]).unwrap();
    let line = r.vendors.iter().find(|v| v.vendor_id == s.dana).unwrap();
    (line.paid, r.unresolved)
}

/// vendor_1099_report SCENARIO 3, by hand: $1,000.00 from the bank (Dr
/// 6000 / Cr 1000), Dana $600.00 and Tobin $400.00 by amount. Payment =
/// target = 1,000.00, so Dana's 1099 counts her share unchanged: 600.00
/// (not the whole 1,000.00).
#[test]
fn a_shared_payment_counts_the_vendors_share() {
    use ledger_core::Share::Amount;
    let mut s = shared_books();
    let (bank, labor, dana_p, tobin) = (s.bank, s.labor, s.dana_p, s.tobin);
    let e = post_shared(
        &mut s,
        d(2026, 3, 1),
        &[
            (labor, Side::Debit, "1000.00"),
            (bank, Side::Credit, "1000.00"),
        ],
        &[
            (dana_p, Amount(dec("600.00"))),
            (tobin, Amount(dec("400.00"))),
        ],
    );
    assert_eq!(
        s.l.entry_vendor(e),
        Some(s.dana),
        "the vendor tag follows Dana"
    );
    assert_eq!(dana_paid(&s), (dec("600.00"), vec![]));
    let r = report_for(&s.l, 2026, None, &[]).unwrap();
    assert_eq!(r.vendors[0].paid, dec("600.00"));
    assert!(!r.vendors[0].reportable, "600.00 is under 2,000");
}

/// SCENARIO 4, by hand: $100.01 from the bank split 50% / 50%. Each part
/// is 100.01 × 50 / 100 = 50.005, rounded toward zero to 50.00; the cent
/// left over goes to the party named first (shares INVARIANT 8). Dana
/// first: 50.01. A second payment with Dana named second: 50.00. Total
/// 100.01.
#[test]
fn a_percent_split_gives_the_leftover_cent_to_the_first() {
    use ledger_core::Share::Percent;
    let mut s = shared_books();
    let (bank, labor, dana_p, tobin) = (s.bank, s.labor, s.dana_p, s.tobin);
    let lines = [
        (labor, Side::Debit, "100.01"),
        (bank, Side::Credit, "100.01"),
    ];
    post_shared(
        &mut s,
        d(2026, 4, 1),
        &lines,
        &[(dana_p, Percent(dec("50"))), (tobin, Percent(dec("50")))],
    );
    assert_eq!(dana_paid(&s).0, dec("50.01"));
    post_shared(
        &mut s,
        d(2026, 4, 2),
        &lines,
        &[(tobin, Percent(dec("50"))), (dana_p, Percent(dec("50")))],
    );
    assert_eq!(dana_paid(&s).0, dec("100.01"), "50.01 + 50.00");
}

/// SCENARIO 6, by hand: SCENARIO 3's payment reversed the same year. The
/// reversal carries the shares; its target is −1,000.00, Dana's amount
/// resolves to −600.00, and the payment is −1,000.00 = target, so she
/// nets 600.00 − 600.00 = 0.00 (still listed: she is decided for a 1099).
#[test]
fn a_reversed_shared_payment_nets_to_zero() {
    use ledger_core::Share::Amount;
    let mut s = shared_books();
    let (bank, labor, dana_p, tobin) = (s.bank, s.labor, s.dana_p, s.tobin);
    let e = post_shared(
        &mut s,
        d(2026, 3, 1),
        &[
            (labor, Side::Debit, "1000.00"),
            (bank, Side::Credit, "1000.00"),
        ],
        &[
            (dana_p, Amount(dec("600.00"))),
            (tobin, Amount(dec("400.00"))),
        ],
    );
    let rev = s.l.reverse_entry(e, d(2026, 3, 2), "wrong payee").unwrap();
    assert_eq!(s.l.entry_parties(rev), s.l.entry_parties(e));
    assert_eq!(dana_paid(&s), (dec("0.00"), vec![]));
}

/// SCENARIO 5, by hand: $1,000.00 of contract labor, 600.00 from the
/// bank and 400.00 on a card no payment method names, Dana 60% / Tobin
/// 40%. Target (the flow net, the card counted as money) 1,000.00, Dana's
/// share 600.00, the 1099 payment 600.00 (the card is no payment line):
/// 600.00 × 600.00 / 1,000.00 = 360.00. Then $100.00 with 33.33 from the
/// bank and 66.67 on the card, 50/50: share 50.00, 33.33 × 50.00 / 100.00
/// = 16.665, rounded half away from zero to 16.67. Total 376.67.
#[test]
fn a_shared_payment_partly_on_a_card_scales_to_the_payment() {
    use ledger_core::Share::Percent;
    let mut s = shared_books();
    let (bank, card, labor, dana_p, tobin) = (s.bank, s.card, s.labor, s.dana_p, s.tobin);
    post_shared(
        &mut s,
        d(2026, 5, 1),
        &[
            (labor, Side::Debit, "1000.00"),
            (bank, Side::Credit, "600.00"),
            (card, Side::Credit, "400.00"),
        ],
        &[(dana_p, Percent(dec("60"))), (tobin, Percent(dec("40")))],
    );
    assert_eq!(dana_paid(&s).0, dec("360.00"));
    post_shared(
        &mut s,
        d(2026, 5, 2),
        &[
            (labor, Side::Debit, "100.00"),
            (bank, Side::Credit, "33.33"),
            (card, Side::Credit, "66.67"),
        ],
        &[(dana_p, Percent(dec("50"))), (tobin, Percent(dec("50")))],
    );
    assert_eq!(dana_paid(&s).0, dec("376.67"));
}

/// P3 review R12, by hand: SCENARIO 5's first payment with the card
/// recorded as a payment method. Netted, the entry paid 600.00 (counts)
/// and 400.00 (card); Dana's 60% is 600.00 of it, split in proportion:
/// 360.00 counts and 240.00 excluded, summing to her share exactly.
/// 100.00 with 33.33 from the bank and 66.67 on the card, 50/50: her
/// share 50.00 splits into 16.665 and 33.335, down to 16.66 and 33.33,
/// the cent short going to the larger remainder (equal: the first), so
/// 16.67 and 33.33.
#[test]
fn a_shared_payment_splits_across_buckets_to_the_cent() {
    use ledger_core::Share::Percent;
    let mut s = shared_books();
    let card = s.card;
    s.l.add_instrument(
        InstrumentKind::BusinessCard,
        "Harbor Visa",
        None,
        None,
        card,
    )
    .unwrap();
    let (bank, labor, dana_p, tobin) = (s.bank, s.labor, s.dana_p, s.tobin);
    post_shared(
        &mut s,
        d(2026, 5, 1),
        &[
            (labor, Side::Debit, "1000.00"),
            (bank, Side::Credit, "600.00"),
            (card, Side::Credit, "400.00"),
        ],
        &[(dana_p, Percent(dec("60"))), (tobin, Percent(dec("40")))],
    );
    let get = |s: &Shared| {
        let r = report_for(&s.l, 2026, None, &[]).unwrap();
        let v = line(&r, "Dana");
        (v.paid, v.excluded_card)
    };
    assert_eq!(get(&s), (dec("360.00"), dec("240.00")));
    post_shared(
        &mut s,
        d(2026, 5, 2),
        &[
            (labor, Side::Debit, "100.00"),
            (bank, Side::Credit, "33.33"),
            (card, Side::Credit, "66.67"),
        ],
        &[(dana_p, Percent(dec("50"))), (tobin, Percent(dec("50")))],
    );
    assert_eq!(get(&s), (dec("376.67"), dec("273.33")));
}

/// INVARIANT 5, by hand: amounts of 600.00 and 300.00 on a 1,000.00
/// payment don't resolve (shares SCENARIO 5), so the whole 1,000.00
/// counts toward Dana and the entry is listed. A single-party entry
/// (300.00, Dana alone) counts in full, as before shares: 1,300.00.
#[test]
fn unresolved_shares_count_in_full_and_are_listed() {
    use ledger_core::Share::{Amount, Derived};
    let mut s = shared_books();
    let (bank, labor, dana_p, tobin) = (s.bank, s.labor, s.dana_p, s.tobin);
    let e = post_shared(
        &mut s,
        d(2026, 6, 1),
        &[
            (labor, Side::Debit, "1000.00"),
            (bank, Side::Credit, "1000.00"),
        ],
        &[
            (dana_p, Amount(dec("600.00"))),
            (tobin, Amount(dec("300.00"))),
        ],
    );
    post_shared(
        &mut s,
        d(2026, 6, 2),
        &[
            (labor, Side::Debit, "300.00"),
            (bank, Side::Credit, "300.00"),
        ],
        &[(dana_p, Derived)],
    );
    assert_eq!(dana_paid(&s), (dec("1300.00"), vec![e]));
}

/// The SCENARIO 3 payment: $1,000.00 from the bank, Dana 600.00 and
/// Tobin 400.00 by amount.
fn dana_and_tobin(s: &mut Shared) -> EntryId {
    use ledger_core::Share::Amount;
    let (bank, labor, dana_p, tobin) = (s.bank, s.labor, s.dana_p, s.tobin);
    post_shared(
        s,
        d(2026, 3, 1),
        &[
            (labor, Side::Debit, "1000.00"),
            (bank, Side::Credit, "1000.00"),
        ],
        &[
            (dana_p, Amount(dec("600.00"))),
            (tobin, Amount(dec("400.00"))),
        ],
    )
}

/// set_entry_vendor SCENARIO 4, by hand: re-tagging Dana's vendor on the
/// shared payment keeps both shares, so her 1099 stays 600.00 (not the
/// whole 1,000.00) and Tobin keeps his part. Tagging a vendor whose
/// party isn't named is refused, changing nothing.
#[test]
fn tagging_the_vendor_keeps_a_shared_entrys_parties() {
    let mut s = shared_books();
    let e = dana_and_tobin(&mut s);
    let before = s.l.entry_parties(e).to_vec();
    s.l.set_entry_vendor(e, Some(s.dana)).unwrap();
    assert_eq!(s.l.entry_parties(e), before.as_slice());
    assert!(s.l.entry_names_party(e, s.tobin));
    assert_eq!(dana_paid(&s), (dec("600.00"), vec![]));
    let other = s.l.add_vendor("Other Co").unwrap();
    let err = s.l.set_entry_vendor(e, Some(other)).unwrap_err();
    assert!(matches!(err, LedgerError::InvalidParty(_)), "{err:?}");
    assert_eq!(s.l.entry_parties(e), before.as_slice());
    assert_eq!(s.l.entry_vendor(e), Some(s.dana));
    // One party or none: the vendor's party becomes the entry's one party.
    let (labor, bank, tobin) = (s.labor, s.bank, s.tobin);
    let lone = post_shared(
        &mut s,
        d(2026, 3, 5),
        &[(labor, Side::Debit, "50.00"), (bank, Side::Credit, "50.00")],
        &[(tobin, ledger_core::Share::Derived)],
    );
    s.l.set_entry_vendor(lone, Some(s.dana)).unwrap();
    assert_eq!(s.l.entry_party(lone), Some(s.dana_p));
}

/// set_entry_vendor SCENARIO 5: clearing the tag on the shared payment,
/// whose vendor's party (Dana) is named, is refused and changes nothing,
/// so the 1099 (600.00) and Dana's party activity never disagree.
#[test]
fn clearing_the_vendor_on_a_shared_entry_is_refused() {
    let mut s = shared_books();
    let e = dana_and_tobin(&mut s);
    let err = s.l.set_entry_vendor(e, None).unwrap_err();
    assert!(matches!(err, LedgerError::InvalidParty(_)), "{err:?}");
    assert_eq!(s.l.entry_vendor(e), Some(s.dana));
    assert!(s.l.entry_names_party(e, s.dana_p));
    assert_eq!(dana_paid(&s), (dec("600.00"), vec![]));
    // Dropping Dana from the parties clears the tag with her.
    s.l.set_entry_parties(e, &[ledger_core::PartyShare::whole(s.tobin)])
        .unwrap();
    assert_eq!(s.l.entry_vendor(e), None);
}

/// set_entry_parties SCENARIO 3, by hand: the shared payment reversed,
/// then re-apportioned 700.00 / 300.00. The reversal is re-apportioned
/// too, so Dana nets 700.00 − 700.00 = 0.00 (not 700.00 − 600.00 =
/// 100.00). Re-apportioning the reversal itself does the same for the
/// original.
#[test]
fn re_apportioning_a_reversed_entry_re_apportions_its_reversal() {
    use ledger_core::Share::Amount;
    let mut s = shared_books();
    let e = dana_and_tobin(&mut s);
    let rev = s.l.reverse_entry(e, d(2026, 3, 2), "wrong payee").unwrap();
    assert_eq!(s.l.reversal_chain(e), vec![e, rev]);
    assert_eq!(s.l.reversal_chain(rev), vec![e, rev]);
    let shares = |a: &str, b: &str| {
        vec![
            ledger_core::PartyShare {
                party: s.dana_p,
                share: Amount(dec(a)),
            },
            ledger_core::PartyShare {
                party: s.tobin,
                share: Amount(dec(b)),
            },
        ]
    };
    let new = shares("700.00", "300.00");
    s.l.set_entry_parties(e, &new).unwrap();
    assert_eq!(s.l.entry_parties(rev), new.as_slice());
    assert_eq!(dana_paid(&s), (dec("0.00"), vec![]));
    let back = shares("550.00", "450.00");
    s.l.set_entry_parties(rev, &back).unwrap();
    assert_eq!(s.l.entry_parties(e), back.as_slice());
    assert_eq!(dana_paid(&s), (dec("0.00"), vec![]));
    // Re-tagging the vendor on a reversed one-party entry tags both.
    let (labor, bank, tobin) = (s.labor, s.bank, s.tobin);
    let lone = post_shared(
        &mut s,
        d(2026, 4, 1),
        &[(labor, Side::Debit, "80.00"), (bank, Side::Credit, "80.00")],
        &[(tobin, ledger_core::Share::Derived)],
    );
    let lone_rev = s.l.reverse_entry(lone, d(2026, 4, 2), "oops").unwrap();
    s.l.set_entry_vendor(lone, Some(s.dana)).unwrap();
    assert_eq!(s.l.entry_vendor(lone_rev), Some(s.dana));
    assert_eq!(dana_paid(&s), (dec("0.00"), vec![]));
}

/// add_vendor INVARIANTS 2-3: a party named on a shared entry beside
/// Dana (who has a vendor record) can't gain a vendor record, which
/// would leave the entry naming two vendors (shares INVARIANT 5); a party
/// named with no vendor party gains the tag with its new record.
#[test]
fn a_named_party_gaining_a_vendor_record_keeps_one_vendor_per_entry() {
    use ledger_core::Share::Percent;
    let mut s = shared_books();
    let beta = s.l.add_party("Beta", &[]).unwrap();
    let (bank, labor, dana_p) = (s.bank, s.labor, s.dana_p);
    let e = post_shared(
        &mut s,
        d(2026, 7, 1),
        &[
            (labor, Side::Debit, "100.00"),
            (bank, Side::Credit, "100.00"),
        ],
        &[(dana_p, Percent(dec("60"))), (beta, Percent(dec("40")))],
    );
    let vendors_before = s.l.list_vendors().len();
    let err = s.l.add_vendor("Beta").unwrap_err();
    assert!(matches!(err, LedgerError::InvalidParty(_)), "{err:?}");
    assert_eq!(s.l.list_vendors().len(), vendors_before, "nothing changed");
    assert_eq!(s.l.get_party(beta).unwrap().vendor, None);
    // Re-issuing the same shares is still accepted.
    let same = s.l.entry_parties(e).to_vec();
    s.l.set_entry_parties(e, &same).unwrap();
    // Gamma, named only beside Tobin (a member, no vendor record).
    let gamma = s.l.add_party("Gamma", &[]).unwrap();
    let tobin = s.tobin;
    let g = post_shared(
        &mut s,
        d(2026, 7, 2),
        &[
            (labor, Side::Debit, "100.00"),
            (bank, Side::Credit, "100.00"),
        ],
        &[(tobin, Percent(dec("50"))), (gamma, Percent(dec("50")))],
    );
    assert_eq!(s.l.entry_vendor(g), None);
    let gv = s.l.add_vendor("Gamma").unwrap();
    assert_eq!(s.l.get_party(gamma).unwrap().vendor, Some(gv));
    assert_eq!(s.l.entry_vendor(g), Some(gv), "the tag follows the party");
}

// ---------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------

/// One generated payment: debit and credit account indexes (into the
/// property books' accounts), cents, month, vendor index, payment method
/// index (0: none), reversed or not.
type Gen = (usize, usize, u32, u32, usize, usize, bool);

/// Books for the properties: bank 1000 (cash), Due to Members 2100
/// (financing), a card 2050 (a business card's), contract labor 6000,
/// prepaid 1300, another liability 2001; three vendors decided for a
/// 1099; with methods, the card, a member's card and a member's check.
fn property_books(gens: &[Gen], with_methods: bool) -> (Ledger, Vec<AccountId>, Vec<VendorId>) {
    let mut l = Ledger::new(Currency::USD);
    let accounts: Vec<AccountId> = [
        ("1000", AccountType::Asset),
        ("2100", AccountType::Liability),
        ("2050", AccountType::Liability),
        ("6000", AccountType::Expense),
        ("1300", AccountType::Asset),
        ("2001", AccountType::Liability),
    ]
    .into_iter()
    .map(|(c, t)| l.add_account(c, c, t, None, false).unwrap())
    .collect();
    let vendors: Vec<VendorId> = ["Ash", "Birch", "Cedar"]
        .into_iter()
        .map(|n| {
            let v = l.add_vendor(n).unwrap();
            decide(&mut l, v, nec(), false);
            v
        })
        .collect();
    let mut methods = vec![None];
    if with_methods {
        for (k, label, a) in [
            (InstrumentKind::BusinessCard, "Card", accounts[2]),
            (InstrumentKind::MemberCard, "Member card", accounts[1]),
            (
                InstrumentKind::MemberCashOrCheck,
                "Member check",
                accounts[1],
            ),
        ] {
            methods.push(Some(l.add_instrument(k, label, None, None, a).unwrap()));
        }
    }
    for &(dr, cr, cents, month, v, method, reversed) in gens {
        if dr == cr {
            continue;
        }
        let e = l.create_entry(d(2026, month, 10), "generated");
        let m = Money::new(Decimal::new(i64::from(cents), 2), Currency::USD);
        l.add_line(e, accounts[dr], Side::Debit, m).unwrap();
        l.add_line(e, accounts[cr], Side::Credit, m).unwrap();
        l.post_entry(e).unwrap();
        l.set_entry_vendor(e, Some(vendors[v])).unwrap();
        if let Some(i) = methods[method % methods.len()] {
            l.set_entry_instrument(e, Some(i)).unwrap();
        }
        if reversed {
            l.reverse_entry(e, d(2026, month, 20), "undo").unwrap();
        }
    }
    (l, accounts, vendors)
}

fn arb_gen(accounts: usize) -> impl Strategy<Value = Gen> {
    (
        0..accounts,
        0..accounts,
        1u32..500_000,
        1u32..=12,
        0usize..3,
        0usize..4,
        any::<bool>(),
    )
}

/// Credits minus debits to `on` over `vendor`'s posted entries.
fn signed_on(l: &Ledger, vendor: VendorId, on: &[AccountId]) -> Decimal {
    l.posted_entries()
        .filter(|e| l.entry_vendor(e.id()) == Some(vendor))
        .flat_map(|e| e.lines().iter())
        .filter(|ln| on.contains(&ln.account))
        .map(|ln| match ln.side {
            Side::Credit => ln.amount.amount,
            Side::Debit => -ln.amount.amount,
        })
        .sum()
}

proptest! {
    /// The regression property (P3 brief 3.3, with review R5): for books
    /// with their cash accounts named and no payment methods or financing
    /// accounts, what counts is the old figure exactly (credits minus
    /// debits to the cash accounts on each vendor's tagged entries), but
    /// for money received from the vendor for something else (review R6:
    /// here a credit to a liability, a loan from it), which is left out
    /// and listed, each chain's first entry taking money in.
    #[test]
    fn no_instruments_no_financing_is_the_old_report(
        gens in prop::collection::vec(arb_gen(6), 0..12)
    ) {
        let (l, accounts, vendors) = property_books(&gens, false);
        let r = vendor_1099_report(
            &l, 2026, None,
            &PaymentAccounts { cash: vec![accounts[0]], financing: vec![] },
            &[], None,
        ).unwrap();
        for v in &vendors {
            let line = r.vendors.iter().find(|x| x.vendor_id == *v).unwrap();
            let other: Decimal = r
                .other_receipts
                .iter()
                .filter(|p| p.vendor_id == *v)
                .map(|p| p.amount)
                .sum();
            prop_assert_eq!(line.paid + other, signed_on(&l, *v, &accounts[..1]));
            prop_assert_eq!(line.excluded_card, Decimal::ZERO);
            prop_assert_eq!(line.undetermined, Decimal::ZERO);
        }
        for p in &r.other_receipts {
            let e = l.get_entry(p.entry_id).unwrap();
            prop_assert!(e.reversal_of().is_some() || p.amount < Decimal::ZERO, "{:?}", p);
            // Here no account is a financing one or a card's: only a
            // credit to a liability (2100, 2050, 2001) is money in for
            // something else.
            let root = match e.reversal_of() {
                Some(o) => l.get_entry(o).unwrap(),
                None => e,
            };
            prop_assert!(
                root.lines().iter().any(|ln| ln.side == Side::Credit
                    && [accounts[1], accounts[2], accounts[5]].contains(&ln.account)),
                "{:?}",
                p
            );
        }
    }

    /// The buckets partition each payment (P3 brief 8.2): for every
    /// vendor, counts + card + undetermined, and what it paid the company
    /// for something else, is the signed sum of its entries' payment lines
    /// (cash, financing, and the card's account), whatever the methods,
    /// netting included.
    #[test]
    fn buckets_partition_each_payment(
        gens in prop::collection::vec(arb_gen(6), 0..12)
    ) {
        let (l, accounts, vendors) = property_books(&gens, true);
        let r = vendor_1099_report(
            &l, 2026, None,
            &PaymentAccounts { cash: vec![accounts[0]], financing: vec![accounts[1]] },
            &[], None,
        ).unwrap();
        for v in &vendors {
            let line = r.vendors.iter().find(|x| x.vendor_id == *v).unwrap();
            let other: Decimal = r
                .other_receipts
                .iter()
                .filter(|p| p.vendor_id == *v)
                .map(|p| p.amount)
                .sum();
            prop_assert_eq!(
                line.paid + line.excluded_card + line.undetermined + other,
                signed_on(&l, *v, &accounts[..3])
            );
            prop_assert_eq!(line.not_reportable, Decimal::ZERO);
        }
    }

    /// rules.md B-35 item 9: party totals and the 1099 worksheet never
    /// contradict. For books whose cash accounts are named, whose company
    /// card is recorded as a business-card payment method (or that have
    /// no card at all: without the method, 2050's name says nothing, so
    /// neither reads it as a card), and whose entries each name one
    /// vendor alone and pay it (or take money from it) between one
    /// source (cash, the card, the members' financing) and something that
    /// isn't one, every vendor's party totals' paid less received (basis
    /// cash) is its worksheet's paid + excluded_card + undetermined +
    /// not_reportable, plus what the worksheet lists as received from it
    /// for something else (those amounts are negative), whatever the
    /// methods, refunds and reversals. (Money moved between two sources,
    /// a card bill or a reimbursement, is paid to the creditor in party
    /// totals and pays no payee on the worksheet: left out here.)
    #[test]
    fn party_totals_are_the_1099s_buckets(
        gens in prop::collection::vec(arb_gen(6), 0..12),
        with_methods in any::<bool>(),
    ) {
        let one_source: Vec<Gen> = gens
            .into_iter()
            .filter(|g| (g.0 < 3) != (g.1 < 3))
            .collect();
        let (l, accounts, vendors) = property_books(&one_source, with_methods);
        let r = vendor_1099_report(
            &l, 2026, None,
            &PaymentAccounts { cash: vec![accounts[0]], financing: vec![accounts[1]] },
            &[], None,
        ).unwrap();
        let money = ledger_core::parties::money_accounts(&l, &[accounts[0]], &[accounts[1]]);
        let year = (d(2026, 1, 1), d(2026, 12, 31));
        for v in &vendors {
            let party = l.party_of_vendor(*v).unwrap();
            let a = ledger_core::parties::activity_with(&l, party, year, &money, &[]);
            prop_assert!(a.unresolved.is_empty(), "{:?}", a.unresolved);
            let (paid, received) = a.totals(ledger_core::parties::Basis::Cash);
            let other: Decimal = r
                .other_receipts
                .iter()
                .filter(|p| p.vendor_id == *v)
                .map(|p| p.amount)
                .sum();
            let line = r.vendors.iter().find(|x| x.vendor_id == *v).unwrap();
            prop_assert_eq!(
                paid - received,
                line.paid + line.excluded_card + line.undetermined + line.not_reportable + other,
                "{:?}",
                line
            );
        }
    }

    /// A payment and its reversal sit in the same bucket and net to zero
    /// in each (the reversal carries the method): books whose every
    /// payment is reversed in the year show nothing anywhere.
    #[test]
    fn a_reversal_nets_every_bucket_to_zero(
        gens in prop::collection::vec(arb_gen(6), 0..10)
    ) {
        let reversed: Vec<Gen> = gens
            .into_iter()
            .map(|g| (g.0, g.1, g.2, g.3, g.4, g.5, true))
            .collect();
        let (l, accounts, _) = property_books(&reversed, true);
        let r = vendor_1099_report(
            &l, 2026, None,
            &PaymentAccounts { cash: vec![accounts[0]], financing: vec![accounts[1]] },
            &[], None,
        ).unwrap();
        for line in &r.vendors {
            prop_assert_eq!(
                (line.paid, line.excluded_card, line.undetermined),
                (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO)
            );
            prop_assert_eq!(line.status, Status1099::UnderThreshold);
        }
    }
}

/// A generated decision: 1099-NEC (2), a corporation (3), else
/// undetermined.
fn gen_decision(k: u8) -> Decision1099 {
    match k {
        2 => nec(),
        3 => corp("a corporation by name"),
        _ => Decision1099::Undetermined,
    }
}

proptest! {
    /// The worksheet ties to the ledger: for every vendor, what counts,
    /// what a card paid, what is undetermined, what was paid while not
    /// reportable and what was received for something else sum to the
    /// signed total of its entries' payment lines in the year (cash,
    /// financing and the card's account), whatever its decisions (dated
    /// ones included), methods, reversals, sales and receipts; the
    /// payment lists sum to their columns; and a vendor left off the
    /// worksheet has nothing but receipts for something else.
    #[test]
    fn the_worksheet_ties_to_the_ledger(
        gens in prop::collection::vec(arb_gen(8), 0..14),
        // Per vendor: its decision from the beginning, and a change on
        // the first of a month (recorded over a decision only).
        decisions in prop::collection::vec(
            (1u8..4, prop::option::of((1u32..=12, 1u8..4))),
            3,
        ),
    ) {
        // property_books' accounts, then revenue 4000 and receivables
        // 1200 (money from a vendor for something else).
        let (mut l, mut accounts, vendors) = property_books(&[], true);
        for (code, t) in [("4000", AccountType::Revenue), ("1200", AccountType::Asset)] {
            accounts.push(l.add_account(code, code, t, None, false).unwrap());
        }
        for (v, &(from_start, change)) in vendors.iter().zip(&decisions) {
            decide(&mut l, *v, gen_decision(from_start), false);
            if from_start >= 2
                && let Some((month, k)) = change
            {
                decide_from(&mut l, *v, d(2026, month, 1), gen_decision(k));
            }
        }
        let methods: Vec<Option<u64>> = std::iter::once(None)
            .chain(l.instruments().map(|i| Some(i.id)))
            .collect();
        for &(dr, cr, cents, month, v, method, reversed) in &gens {
            if dr == cr {
                continue;
            }
            let e = l.create_entry(d(2026, month, 10), "generated");
            let m = Money::new(Decimal::new(i64::from(cents), 2), Currency::USD);
            l.add_line(e, accounts[dr], Side::Debit, m).unwrap();
            l.add_line(e, accounts[cr], Side::Credit, m).unwrap();
            l.post_entry(e).unwrap();
            l.set_entry_vendor(e, Some(vendors[v])).unwrap();
            if let Some(i) = methods[method % methods.len()] {
                l.set_entry_instrument(e, Some(i)).unwrap();
            }
            if reversed {
                l.reverse_entry(e, d(2026, month, 20), "undo").unwrap();
            }
        }
        let r = vendor_1099_report(
            &l, 2026, None,
            &PaymentAccounts { cash: vec![accounts[0]], financing: vec![accounts[1]] },
            &[], None,
        ).unwrap();
        let of = |list: &[ledger_core::Payment1099], v: VendorId| -> Decimal {
            list.iter().filter(|p| p.vendor_id == v).map(|p| p.amount).sum()
        };
        for v in &vendors {
            let ledger_total = signed_on(&l, *v, &accounts[..3]);
            let other = of(&r.other_receipts, *v);
            match r.vendors.iter().find(|x| x.vendor_id == *v) {
                Some(line) => {
                    prop_assert_eq!(
                        line.paid + line.excluded_card + line.undetermined + line.not_reportable + other,
                        ledger_total
                    );
                    prop_assert_eq!(of(&r.excluded_card_payments, *v), line.excluded_card);
                    prop_assert_eq!(of(&r.undetermined_payments, *v), line.undetermined);
                    prop_assert_eq!(line.reportable, line.status == Status1099::Reportable);
                }
                None => prop_assert_eq!(other, ledger_total),
            }
        }
    }
}

/// Ledger::card_accounts: the liability accounts company cards pay
/// through, from the payment methods recorded (never an account's name):
/// not a debit card's bank account, a member's card's Due to Members, or a
/// platform's balance.
#[test]
fn card_accounts_are_the_company_cards_liabilities() {
    let mut m = members_books();
    assert!(m.l.card_accounts().is_empty(), "no methods recorded: none");
    let paypal =
        m.l.add_account("1050", "Payment platform", AccountType::Asset, None, false)
            .unwrap();
    for (kind, label, account) in [
        (InstrumentKind::BusinessCard, "Harbor Visa", m.card),
        (InstrumentKind::BusinessCard, "Harbor debit card", m.bank),
        (InstrumentKind::MemberCard, "Riley's Visa", m.due),
        (InstrumentKind::Platform, "Harbor Pay", paypal),
    ] {
        m.l.add_instrument(kind, label, None, None, account)
            .unwrap();
    }
    assert_eq!(
        m.l.card_accounts().into_iter().collect::<Vec<_>>(),
        [m.card]
    );
}
