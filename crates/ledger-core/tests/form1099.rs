//! 1099 tracking. Numbers computed by hand.
//!
//! 2026 payments:
//! - Dana (1099-NEC, W-9): bill 1,500 paid by check + 700 direct = 2,200,
//!   over the 2,000 threshold. A 300 payment on 2025-12-30 counts in 2025.
//! - Ace Rentals (1099-MISC): 1,200 twice, one reversed = 1,200, under.
//! - Sam (1099-NEC): 5,000 by credit card = 0 cash: the card processor's
//!   1099-K covers it.
//! - Bigco (no 1099): 9,000, listed but never reportable.

use chrono::NaiveDate;
use ledger_core::arap::DocumentLine;
use ledger_core::form1099::default_threshold;
use ledger_core::{
    AccountType, Currency, Form1099, Ledger, LedgerError, Money, Side, vendor_1099_report,
};
use rust_decimal::Decimal;
use std::str::FromStr;

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
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
    l.set_vendor_1099(dana, Some(Form1099::Nec), true).unwrap();
    l.set_vendor_1099(ace, Some(Form1099::Misc), false).unwrap();
    l.set_vendor_1099(sam, Some(Form1099::Nec), true).unwrap();
    let pay = |l: &mut Ledger, date, vendor, dr, cr, amt: &str| {
        let e = l.create_entry(date, "payment");
        let m = Money::new(dec(amt), Currency::USD);
        l.add_line(e, dr, Side::Debit, m).unwrap();
        l.add_line(e, cr, Side::Credit, m).unwrap();
        l.post_entry(e).unwrap();
        l.set_entry_vendor(e, Some(vendor)).unwrap();
        e
    };
    let bill = l
        .create_bill(
            dana,
            d(2026, 3, 1),
            d(2026, 3, 31),
            vec![DocumentLine {
                description: "Design work".into(),
                amount: dec("1500"),
            }],
        )
        .unwrap();
    l.pay_bill(bill, d(2026, 3, 20), dec("1500"), bank).unwrap();
    pay(&mut l, d(2026, 6, 1), dana, contract, bank, "700");
    pay(&mut l, d(2025, 12, 30), dana, contract, bank, "300");
    pay(&mut l, d(2026, 1, 1), ace, rent, bank, "1200");
    let wrong = pay(&mut l, d(2026, 2, 1), ace, rent, bank, "1200");
    l.reverse_entry(wrong, d(2026, 2, 2), "duplicate").unwrap();
    pay(&mut l, d(2026, 4, 1), sam, contract, card, "5000");
    pay(&mut l, d(2026, 5, 1), bigco, contract, bank, "9000");
    // Untagged spending is not anyone's 1099.
    let e = l.create_entry(d(2026, 7, 1), "untagged");
    let m = Money::new(dec("50"), Currency::USD);
    l.add_line(e, contract, Side::Debit, m).unwrap();
    l.add_line(e, bank, Side::Credit, m).unwrap();
    l.post_entry(e).unwrap();
    l
}

#[test]
fn cash_paid_per_vendor_and_who_is_reportable() {
    let l = books();
    let r = vendor_1099_report(&l, 2026, None).unwrap();
    assert_eq!(r.threshold, dec("2000"));
    let rows: Vec<(&str, Decimal, bool)> = r
        .vendors
        .iter()
        .map(|v| (v.name.as_str(), v.paid, v.reportable))
        .collect();
    assert_eq!(
        rows,
        [
            ("Ace Rentals", dec("1200"), false),
            ("Bigco", dec("9000"), false),
            ("Dana", dec("2200"), true),
            ("Sam", dec("0"), false),
        ]
    );
    let ace = &r.vendors[0];
    assert_eq!(ace.form, Some(Form1099::Misc));
    assert!(!ace.tin_on_file);
}

#[test]
fn years_and_thresholds() {
    let l = books();
    // SOURCE: rules.md B-19 (sources/irs-1099-thresholds.md).
    assert_eq!(default_threshold(2025), Ok(dec("600")));
    assert_eq!(default_threshold(2026), Ok(dec("2000")));
    let y2025 = vendor_1099_report(&l, 2025, None).unwrap();
    let dana = y2025.vendors.iter().find(|v| v.name == "Dana").unwrap();
    assert_eq!(dana.paid, dec("300"));
    assert!(!dana.reportable, "300 < 600");
    // An override: at 1,000, Ace's 1,200 is reportable.
    let r = vendor_1099_report(&l, 2026, Some(dec("1000"))).unwrap();
    assert!(
        r.vendors
            .iter()
            .any(|v| v.name == "Ace Rentals" && v.reportable)
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
        spec,
    } = &err
    else {
        panic!("expected RuleNotKnown, got {err:?}");
    };
    assert_eq!(
        (rule.as_str(), *year, *known_through),
        ("1099 reporting threshold", 2027, 2026)
    );
    assert!(spec.ends_with("irs-1099-thresholds.md"), "{spec}");
    let why = err.to_string();
    for needle in [
        "2027",
        "official source",
        "CPA",
        "github.com/swaits/wanderbooks",
    ] {
        assert!(why.contains(needle), "missing {needle:?} in: {why}");
    }
    let l = books();
    assert!(matches!(
        vendor_1099_report(&l, 2027, None),
        Err(LedgerError::RuleNotKnown { .. })
    ));
    // An explicit threshold (from the IRS, or the owner or CPA) works.
    let r = vendor_1099_report(&l, 2027, Some(dec("2100"))).unwrap();
    assert_eq!(r.threshold, dec("2100"));
}

/// Spec: a dated rule is due for review from its review date, and stays
/// due; before it, nothing is stale. The engine has no clock: callers pass
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
    // A label parses back to its form.
    for f in [Form1099::Nec, Form1099::Misc] {
        assert_eq!(Form1099::parse(f.label()), Some(f));
    }
}

/// vendor_1099_report_with SCENARIOS 1–2, worked by hand. Payment sources:
/// cash 1000 and the members' financing liability 2200. Kim (1099-MISC,
/// landlord) in 2026:
/// - 1,000 rent from the bank (Dr rent, Cr 1000): +1,000
/// - 2,400 prepaid rent from the bank (Dr 1300, Cr 1000): +2,400 (paid
///   when made; the prepaid asset is not a source, so it doesn't offset)
/// - 800 rent paid by a member (Dr rent, Cr 2200): +800
/// - 500 rent on the card (Dr rent, Cr 2100): 0 (1099-K)
/// - 300 refunded to the bank (Dr 1000, Cr rent): −300
///
/// Total 3,900. The plain report (every asset a source, no financing)
/// sees 1,000 + 0 + 0 + 0 − 300 = 700: the understatement this fixes.
#[test]
fn payments_come_from_cash_and_financing_only() {
    use ledger_core::{PaymentAccounts, vendor_1099_report_with};
    let mut l = Ledger::new(Currency::USD);
    let acct = |l: &mut Ledger, code: &str, t| l.add_account(code, code, t, None, false).unwrap();
    let bank = acct(&mut l, "1000", AccountType::Asset);
    let prepaid = acct(&mut l, "1300", AccountType::Asset);
    let card = acct(&mut l, "2100", AccountType::Liability);
    let members = acct(&mut l, "2200", AccountType::Liability);
    let rent = acct(&mut l, "6600", AccountType::Expense);
    let kim = l.add_vendor("Kim").unwrap();
    l.set_vendor_1099(kim, Some(Form1099::Misc), true).unwrap();
    for (dr, cr, amt) in [
        (rent, bank, "1000"),
        (prepaid, bank, "2400"),
        (rent, members, "800"),
        (rent, card, "500"),
        (bank, rent, "300"),
    ] {
        let e = l.create_entry(d(2026, 5, 1), "rent");
        let m = Money::new(dec(amt), Currency::USD);
        l.add_line(e, dr, Side::Debit, m).unwrap();
        l.add_line(e, cr, Side::Credit, m).unwrap();
        l.post_entry(e).unwrap();
        l.set_entry_vendor(e, Some(kim)).unwrap();
    }
    let sources = PaymentAccounts {
        cash: vec![bank],
        financing: vec![members],
    };
    let r = vendor_1099_report_with(&l, 2026, None, &sources).unwrap();
    assert_eq!(r.vendors[0].paid, dec("3900"));
    assert!(r.vendors[0].reportable, "3,900 is over 2,000");
    let plain = vendor_1099_report(&l, 2026, None).unwrap();
    assert_eq!(plain.vendors[0].paid, dec("700"));
}

/// Books for the shared-payment tests: bank 1000, card 2050, contract
/// labor 6000; Dana (1099-NEC) and member Tobin as parties.
struct Shared {
    l: Ledger,
    bank: ledger_core::AccountId,
    card: ledger_core::AccountId,
    labor: ledger_core::AccountId,
    dana: ledger_core::VendorId,
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
    l.set_vendor_1099(dana, Some(Form1099::Nec), true).unwrap();
    let dana_p = l.party_of_vendor(dana).unwrap();
    let tobin = l
        .add_party("Tobin", &[ledger_core::Role::Member])
        .unwrap();
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
    lines: &[(ledger_core::AccountId, Side, &str)],
    shares: &[(ledger_core::PartyId, ledger_core::Share)],
) -> ledger_core::EntryId {
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

fn dana_paid(s: &Shared) -> (Decimal, Vec<ledger_core::EntryId>) {
    let r = vendor_1099_report(&s.l, 2026, None).unwrap();
    let line = r.vendors.iter().find(|v| v.vendor_id == s.dana).unwrap();
    (line.paid, r.unresolved)
}

/// vendor_1099_report_with_members SCENARIO 3, by hand: $1,000.00 from
/// the bank (Dr 6000 / Cr 1000), Dana $600.00 and Tobin $400.00 by
/// amount. Payment = target = 1,000.00, so Dana's 1099 counts her share
/// unchanged: 600.00 (not the whole 1,000.00).
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
    // With sources and members named, the same.
    let r = ledger_core::vendor_1099_report_with_members(
        &s.l,
        2026,
        None,
        &ledger_core::PaymentAccounts {
            cash: vec![bank],
            financing: vec![],
        },
        &[],
    )
    .unwrap();
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
/// nets 600.00 − 600.00 = 0.00 (still listed: she is marked for a 1099).
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
/// bank and 400.00 on the card, Dana 60% / Tobin 40%. Target (the flow
/// net, the card counted as money) 1,000.00, Dana's share 600.00, the
/// 1099 payment 600.00 (the card is not a payment source): 600.00 ×
/// 600.00 / 1,000.00 = 360.00. Then $100.00 with 33.33 from the bank and
/// 66.67 on the card, 50/50: share 50.00, 33.33 × 50.00 / 100.00 =
/// 16.665, rounded half away from zero to 16.67. Total 376.67.
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

/// INVARIANT 4, by hand: amounts of 600.00 and 300.00 on a 1,000.00
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
fn dana_and_tobin(s: &mut Shared) -> ledger_core::EntryId {
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
