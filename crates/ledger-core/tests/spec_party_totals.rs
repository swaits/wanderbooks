//! P3 defects 14, 24 and 11: a party's part of an entry, a party's
//! totals, and the directory's TIN last four.
//!
//! Oracles: rules.md B-35 (a party's part, signed as a line on its own
//! account, with its direction and its money and property; party totals
//! on basis cash or all; a member's entry counts what moved their capital;
//! money is cash, cards, the members' financing and loans; a liability
//! credit never counts as paying that liability's own creditor), B-9 (a
//! correction is the opposite entry), B-13 and the K-1 worksheet (k1
//! INVARIANT 3: what is property), B-26 (a TIN's last four), and the spec
//! blocks of `ledger_core::shares`, `ledger_core::parties` (`flow`,
//! `activity_with`, `Party::tin_last4`) and `ledger_core::kernel`
//! (`party_target`, `reverse_target`). Every expected value is worked by
//! hand from those texts and the fixture's lines; nothing is read back
//! from the code under test.
//!
//! The fixture is the shape of the first company's books at the 2026-10-16
//! audit (findings F36, F37), anonymized, its names and every amount made
//! up: two members, Tobin Harlow and Morgan Ellis, 50/50, a bank account, a
//! card, the members' financing account, equipment, prepaid expenses.

use std::str::FromStr;

use chrono::NaiveDate;
use ledger_core::k1::k1_detail;
use ledger_core::kernel::{self, Direction, PartyTarget};
use ledger_core::parties::{Basis, MoneyAccounts, activity_with, entry_flow, flow, money_accounts};
use ledger_core::reports::{MemberCapital, members_equity};
use ledger_core::shares::{Part, PartyShare, Share, Target, resolve, target};
use ledger_core::{
    AccountId, AccountType, Currency, DocumentLine, EntryId, Ledger, LedgerError, Money,
    PartyField, PartyId, PartyMember, Role, Side, TinFacts, TinType, W9Facts,
};
use proptest::prelude::*;
use rust_decimal::Decimal;

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("decimal literal")
}

fn date(m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, day).expect("a 2026 date")
}

fn year() -> (NaiveDate, NaiveDate) {
    (date(1, 1), date(12, 31))
}

/// Fernhill Labs LLC's chart and directory.
struct Books {
    l: Ledger,
    cash: AccountId,
    prepaid: AccountId,
    equip: AccountId,
    card: AccountId,
    due: AccountId,
    accrued: AccountId,
    deferred: AccountId,
    note: AccountId,
    /// Distributions declared and not yet paid: a liability that isn't
    /// money (not a card, not a financing account).
    dist_payable: AccountId,
    /// Gift cards the company sold: deferred revenue, though its name says
    /// "card".
    gift: AccountId,
    accum: AccountId,
    cap_t: AccountId,
    cap_m: AccountId,
    dist_t: AccountId,
    dist_m: AccountId,
    revenue: AccountId,
    org: AccountId,
    hosting: AccountId,
    fees: AccountId,
    tobin: PartyId,
    morgan: PartyId,
    /// A vendor (it has a vendor record): Northwind Filings.
    vendor: PartyId,
    vendor_id: u64,
    /// The card's issuer: Harbor Bank.
    issuer: PartyId,
    /// A customer (it has a customer record): Riley Chen.
    customer: PartyId,
    customer_id: u64,
}

fn books() -> Books {
    let mut l = Ledger::new(Currency::USD);
    let mut add = |code: &str, name: &str, t| l.add_account(code, name, t, None, false).unwrap();
    let cash = add("1000", "Checking", AccountType::Asset);
    let prepaid = add("1300", "Prepaid Expenses", AccountType::Asset);
    let equip = add("1500", "Equipment", AccountType::Asset);
    let card = add("2050", "Business Card", AccountType::Liability);
    let due = add("2100", "Due to Members", AccountType::Liability);
    let accrued = add("2200", "Accrued Expenses", AccountType::Liability);
    let deferred = add("2400", "Deferred Revenue", AccountType::Liability);
    let note = add("2500", "Note Payable", AccountType::Liability);
    let dist_payable = add("2150", "Distributions Payable", AccountType::Liability);
    let gift = add("2410", "Gift Card Liability", AccountType::Liability);
    let accum = add(
        "1510",
        "Accumulated Depreciation — Equipment",
        AccountType::Asset,
    );
    let cap_t = add("3000", "Capital — Tobin Harlow", AccountType::Equity);
    let cap_m = add("3001", "Capital — Morgan Ellis", AccountType::Equity);
    let dist_t = add("3100", "Distributions — Tobin Harlow", AccountType::Equity);
    let dist_m = add("3101", "Distributions — Morgan Ellis", AccountType::Equity);
    let revenue = add("4000", "Revenue", AccountType::Revenue);
    let org = add("6000", "Organizational Costs", AccountType::Expense);
    let hosting = add("6050", "Domain & Hosting", AccountType::Expense);
    let fees = add("6400", "Bank Fees", AccountType::Expense);
    let tobin = l.add_party("Tobin Harlow", &[Role::Member]).unwrap();
    let morgan = l.add_party("Morgan Ellis", &[Role::Member]).unwrap();
    let vendor_id = l.add_vendor("Northwind Filings").unwrap();
    let vendor = l.party_of_vendor(vendor_id).unwrap();
    let issuer = l
        .add_party("Harbor Bank", &[Role::FinancialInstitution])
        .unwrap();
    let customer_id = l.add_customer("Riley Chen").unwrap();
    let customer = l.party_of_customer(customer_id).unwrap();
    Books {
        l,
        cash,
        prepaid,
        equip,
        card,
        due,
        accrued,
        deferred,
        note,
        dist_payable,
        gift,
        accum,
        cap_t,
        cap_m,
        dist_t,
        dist_m,
        revenue,
        org,
        hosting,
        fees,
        tobin,
        morgan,
        vendor,
        vendor_id,
        issuer,
        customer,
        customer_id,
    }
}

impl Books {
    /// The money accounts: the bank, the card (by its name), and the
    /// members' financing and the note (`books.financing_accounts`).
    fn money(&self) -> MoneyAccounts {
        money_accounts(&self.l, &[self.cash], &[self.due, self.note])
    }

    /// The company's members, as the settings give them.
    fn members(&self) -> Vec<PartyMember> {
        [
            (self.tobin, self.cap_t, self.dist_t),
            (self.morgan, self.cap_m, self.dist_m),
        ]
        .map(|(party, capital, distributions)| PartyMember {
            party,
            share: d("0.5"),
            capital,
            distributions: Some(distributions),
        })
        .to_vec()
    }

    /// The members as the statement of members' equity and the K-1
    /// worksheet take them.
    fn capitals(&self) -> Vec<MemberCapital> {
        [
            ("Tobin Harlow", self.cap_t, self.dist_t),
            ("Morgan Ellis", self.cap_m, self.dist_m),
        ]
        .map(|(name, capital_account, distributions)| MemberCapital {
            name: name.into(),
            capital_account,
            distribution_account: Some(distributions),
            share: d("0.5"),
        })
        .to_vec()
    }

    /// Post `lines` on `day`, tagged with `shares`.
    fn post(
        &mut self,
        day: NaiveDate,
        lines: &[(AccountId, Side, &str)],
        shares: &[PartyShare],
    ) -> EntryId {
        let e = self.l.try_create_entry(day, "x").unwrap();
        for (a, side, v) in lines {
            self.l
                .add_line(e, *a, *side, Money::new(d(v), Currency::USD))
                .unwrap();
        }
        self.l.post_entry(e).unwrap();
        if !shares.is_empty() {
            self.l.set_entry_parties(e, shares).unwrap();
        }
        e
    }

    fn reverse(&mut self, e: EntryId, day: NaiveDate) -> EntryId {
        self.l.reverse_entry(e, day, "reversed").unwrap()
    }

    /// Entry `e`'s target and parts, as books.get_entry resolves them.
    fn resolved(&self, e: EntryId) -> (Target, Vec<Part>) {
        let r = resolve(
            &self.l,
            e,
            self.l.entry_parties(e),
            &self.money(),
            &self.members(),
        )
        .unwrap_or_else(|why| panic!("entry {e}: {why}"));
        (r.target, r.parts)
    }

    /// `party`'s (paid, received) over 2026 on `basis`.
    fn totals(&self, party: PartyId, basis: Basis) -> (Decimal, Decimal) {
        activity_with(&self.l, party, year(), &self.money(), &self.members()).totals(basis)
    }
}

fn whole(p: PartyId) -> PartyShare {
    PartyShare::whole(p)
}

fn amounts(ps: &[(PartyId, &str)]) -> Vec<PartyShare> {
    ps.iter()
        .map(|&(party, v)| PartyShare {
            party,
            share: Share::Amount(d(v)),
        })
        .collect()
}

/// A part: (party, amount, cash, in kind).
fn part(party: PartyId, amount: &str, cash: &str, in_kind: &str) -> Part {
    Part {
        party,
        amount: d(amount),
        cash: d(cash),
        in_kind: d(in_kind),
    }
}

use Side::{Credit as Cr, Debit as Dr};

/// The audit's reproduction (F36, F37), anonymized, with what each row
/// should say (rules.md B-35): `resolved` signed as a line on the party's
/// own account, a direction, and what of it was money or property; a
/// reversal the exact opposite of what it reverses; totals on both bases,
/// labelled.
struct Audit {
    b: Books,
    /// (entry, the parts it should resolve to, its direction).
    rows: Vec<(EntryId, Vec<Part>, Direction)>,
}

fn audit() -> Audit {
    let mut b = books();
    let (t, m, v) = (b.tobin, b.morgan, b.vendor);
    let mut rows = Vec::new();
    // 1. A member pays the filing fee for the company (through 2100),
    // tagged with the vendor: paid to the vendor, in money.
    let e = b.post(
        date(10, 4),
        &[(b.org, Dr, "64.00"), (b.due, Cr, "64.00")],
        &[whole(v)],
    );
    rows.push((e, vec![part(v, "64.00", "64.00", "0")], Direction::Paid));
    // 4. Each member's cash contribution: received, money.
    let e = b.post(
        date(10, 5),
        &[(b.cash, Dr, "400.00"), (b.cap_t, Cr, "400.00")],
        &[whole(t)],
    );
    rows.push((
        e,
        vec![part(t, "-400.00", "-400.00", "0")],
        Direction::Received,
    ));
    let e = b.post(
        date(10, 5),
        &[(b.cash, Dr, "400.00"), (b.cap_m, Cr, "400.00")],
        &[whole(m)],
    );
    rows.push((
        e,
        vec![part(m, "-400.00", "-400.00", "0")],
        Direction::Received,
    ));
    // 8. Reimbursing both members: paid, money.
    let e = b.post(
        date(10, 8),
        &[(b.due, Dr, "186.50"), (b.cash, Cr, "186.50")],
        &amounts(&[(t, "93.25"), (m, "93.25")]),
    );
    rows.push((
        e,
        vec![
            part(t, "93.25", "93.25", "0"),
            part(m, "93.25", "93.25", "0"),
        ],
        Direction::Paid,
    ));
    // 9. Hosting a member paid for: paid to the vendor.
    let e = b.post(
        date(10, 9),
        &[(b.hosting, Dr, "142.30"), (b.due, Cr, "142.30")],
        &[whole(v)],
    );
    rows.push((e, vec![part(v, "142.30", "142.30", "0")], Direction::Paid));
    // 11. Reimbursing it.
    let e = b.post(
        date(10, 11),
        &[(b.due, Dr, "142.30"), (b.cash, Cr, "142.30")],
        &amounts(&[(t, "71.15"), (m, "71.15")]),
    );
    rows.push((
        e,
        vec![
            part(t, "71.15", "71.15", "0"),
            part(m, "71.15", "71.15", "0"),
        ],
        Direction::Paid,
    ));
    // 12. A laptop Tobin contributes: received, in kind (it read +1480.00,
    // the sign of a payment, before P3).
    let e12 = b.post(
        date(10, 12),
        &[(b.equip, Dr, "1480.00"), (b.cap_t, Cr, "1480.00")],
        &[whole(t)],
    );
    rows.push((
        e12,
        vec![part(t, "-1480.00", "0", "-1480.00")],
        Direction::Received,
    ));
    // 13. Its reversal: the exact opposite, paid, in kind (it read
    // +1480.00, the same as entry 12, before P3).
    let e13 = b.reverse(e12, date(10, 13));
    rows.push((
        e13,
        vec![part(t, "1480.00", "0", "1480.00")],
        Direction::Paid,
    ));
    // 14. The laptop again, half each by amount: −740.00 each.
    let e = b.post(
        date(10, 14),
        &[
            (b.equip, Dr, "1480.00"),
            (b.cap_t, Cr, "740.00"),
            (b.cap_m, Cr, "740.00"),
        ],
        &amounts(&[(t, "740.00"), (m, "740.00")]),
    );
    rows.push((
        e,
        vec![
            part(t, "-740.00", "0", "-740.00"),
            part(m, "-740.00", "0", "-740.00"),
        ],
        Direction::Received,
    ));
    // 15 and 18. A 960.00 device and its reversal.
    let e15 = b.post(
        date(10, 15),
        &[
            (b.equip, Dr, "960.00"),
            (b.cap_t, Cr, "480.00"),
            (b.cap_m, Cr, "480.00"),
        ],
        &amounts(&[(t, "480.00"), (m, "480.00")]),
    );
    rows.push((
        e15,
        vec![
            part(t, "-480.00", "0", "-480.00"),
            part(m, "-480.00", "0", "-480.00"),
        ],
        Direction::Received,
    ));
    // 16 and 17. Two more devices, derived from the capital lines.
    let e = b.post(
        date(10, 16),
        &[
            (b.equip, Dr, "1530.00"),
            (b.cap_t, Cr, "765.00"),
            (b.cap_m, Cr, "765.00"),
        ],
        &[whole(t), whole(m)],
    );
    rows.push((
        e,
        vec![
            part(t, "-765.00", "0", "-765.00"),
            part(m, "-765.00", "0", "-765.00"),
        ],
        Direction::Received,
    ));
    let e = b.post(
        date(10, 17),
        &[
            (b.equip, Dr, "690.00"),
            (b.cap_t, Cr, "345.00"),
            (b.cap_m, Cr, "345.00"),
        ],
        &[whole(t), whole(m)],
    );
    rows.push((
        e,
        vec![
            part(t, "-345.00", "0", "-345.00"),
            part(m, "-345.00", "0", "-345.00"),
        ],
        Direction::Received,
    ));
    let e18 = b.reverse(e15, date(10, 18));
    rows.push((
        e18,
        vec![
            part(t, "480.00", "0", "480.00"),
            part(m, "480.00", "0", "480.00"),
        ],
        Direction::Paid,
    ));
    // 19. A device with its service contract (prepaid: property, k1
    // INVARIANT 3), derived.
    let e = b.post(
        date(10, 19),
        &[
            (b.equip, Dr, "1010.00"),
            (b.prepaid, Dr, "30.00"),
            (b.cap_t, Cr, "520.00"),
            (b.cap_m, Cr, "520.00"),
        ],
        &[whole(t), whole(m)],
    );
    rows.push((
        e,
        vec![
            part(t, "-520.00", "0", "-520.00"),
            part(m, "-520.00", "0", "-520.00"),
        ],
        Direction::Received,
    ));
    // A bill (R1): Cr payables, the vendor's own account: −80.00, neither;
    // its reversal +80.00 (both read +80.00 before P3).
    let bill =
        b.l.create_bill(
            b.vendor_id,
            date(10, 20),
            date(11, 19),
            vec![DocumentLine {
                description: "filing".into(),
                amount: d("80.00"),
            }],
        )
        .unwrap();
    let e = b.l.get_bill(bill).unwrap().entry_id;
    rows.push((e, vec![part(v, "-80.00", "0", "0")], Direction::Neither));
    let r = b.reverse(e, date(10, 21));
    rows.push((r, vec![part(v, "80.00", "0", "0")], Direction::Neither));
    // An invoice: Dr receivables, the customer's own account: +120.00,
    // neither.
    let inv =
        b.l.create_invoice(
            b.customer_id,
            date(10, 20),
            date(11, 19),
            vec![DocumentLine {
                description: "consulting".into(),
                amount: d("120.00"),
            }],
        )
        .unwrap();
    let e = b.l.get_invoice(inv).unwrap().entry_id;
    rows.push((
        e,
        vec![part(b.customer, "120.00", "0", "0")],
        Direction::Neither,
    ));
    Audit { b, rows }
}

/// Defect 14, the audit's table row by row: each part signed as a line on
/// the party's own account, with its direction and its money and
/// property; every reversal the exact opposite of what it reverses.
#[test]
fn defect14_shares_take_the_partys_side_and_reversals_flip() {
    let Audit { b, rows } = audit();
    for (e, want, direction) in &rows {
        let (t, parts) = b.resolved(*e);
        assert_eq!(&parts, want, "entry {e}");
        assert_eq!(t.direction, *direction, "entry {e}");
        let sum: Decimal = parts.iter().map(|p| p.amount).sum();
        assert_eq!(t.amount, sum, "entry {e}: the parts sum to the target");
        // A reversal counts on its original's side.
        let original = b.l.get_entry(*e).unwrap().reversal_of();
        let expect_side = match original {
            Some(_) => direction.flip(),
            None => *direction,
        };
        assert_eq!(t.counts_as, expect_side, "entry {e}");
        // Never a negative zero (INVARIANT 6).
        for p in &parts {
            for v in [p.amount, p.cash, p.in_kind] {
                assert!(!(v.is_zero() && v.is_sign_negative()), "entry {e}: -0");
            }
        }
    }
    // Totals: the members' money is unchanged by P3 (paid 93.25 + 71.15,
    // received 400.00 each); basis all adds their property: 740.00 +
    // 765.00 + 345.00 + 520.00 = 2,370.00, the 1,480.00 and 480.00
    // contributions reversed in the year net to nothing.
    for who in [b.tobin, b.morgan] {
        assert_eq!(b.totals(who, Basis::Cash), (d("164.40"), d("400.00")));
        assert_eq!(b.totals(who, Basis::All), (d("164.40"), d("2770.00")));
    }
    // The vendor: the two payments members made for the company; the bill
    // and its reversal count nothing on either basis.
    assert_eq!(
        b.totals(b.vendor, Basis::Cash),
        (d("206.30"), Decimal::ZERO)
    );
    assert_eq!(b.totals(b.vendor, Basis::All), (d("206.30"), Decimal::ZERO));
    // The customer: invoiced, nothing received yet.
    assert_eq!(
        b.totals(b.customer, Basis::All),
        (Decimal::ZERO, Decimal::ZERO)
    );
}

/// B-35 item 4: on the member-only entries, basis all is the statement of
/// members' equity (contributions and distributions, the same dates), and
/// basis cash is the K-1 worksheet's cash contributions.
#[test]
fn basis_all_is_the_members_equity_statement_and_cash_the_k1s() {
    let Audit { b, .. } = audit();
    let (start, end) = year();
    let me = members_equity(&b.l, start, end, &b.capitals()).unwrap();
    let k1 = k1_detail(&b.l, start, end, &b.capitals(), &[b.cash]).unwrap();
    for (i, who) in [b.tobin, b.morgan].into_iter().enumerate() {
        // The reimbursements (164.40 paid) are money the member was paid
        // back, not equity: they are the only entries naming the member
        // that don't move their capital.
        let (paid_all, received_all) = b.totals(who, Basis::All);
        assert_eq!(received_all, me.members[i].contributions);
        assert_eq!(paid_all - d("164.40"), me.members[i].distributions);
        let (_, received_cash) = b.totals(who, Basis::Cash);
        assert_eq!(received_cash, k1.rows[i].contributions_cash);
        assert_eq!(
            received_all - received_cash,
            k1.rows[i].contributions_property
        );
        assert_eq!(me.members[i].contributions, d("2770.00"));
        assert_eq!(k1.rows[i].contributions_cash, d("400.00"));
    }
}

/// Defect 14, second bug (brief §1.4): a member pays the card from their
/// own funds (Dr 2050 / Cr 2100), tagged with the issuer: paid 100.00. Its
/// reversal read as another payment (paid 200.00); now it nets to 0.00,
/// and reversing the reversal brings 100.00 back.
#[test]
fn defect14_a_reversed_member_card_payment_nets_to_zero() {
    let mut b = books();
    let issuer = b.issuer;
    let e1 = b.post(
        date(6, 1),
        &[(b.card, Dr, "100.00"), (b.due, Cr, "100.00")],
        &[whole(issuer)],
    );
    assert_eq!(b.totals(issuer, Basis::Cash), (d("100.00"), Decimal::ZERO));
    let e2 = b.reverse(e1, date(6, 2));
    assert_eq!(
        b.totals(issuer, Basis::Cash),
        (Decimal::ZERO, Decimal::ZERO)
    );
    // Why parties::flow exists: entry_flow reads the reversal's own lines
    // as another payment (its `across` term is the same either way).
    let money = b.money();
    assert_eq!(
        entry_flow(b.l.get_entry(e2).unwrap(), &money).unwrap().net,
        d("100.00")
    );
    let f1 = flow(&b.l, e1, &[whole(issuer)], &money).unwrap();
    let f2 = flow(&b.l, e2, &[whole(issuer)], &money).unwrap();
    assert_eq!(
        (f1.net, f1.what_for.clone()),
        (d("100.00"), vec![(b.card, d("100.00"))])
    );
    assert_eq!(
        (f2.net, f2.what_for),
        (d("-100.00"), vec![(b.card, d("-100.00"))])
    );
    let e3 = b.reverse(e2, date(6, 3));
    assert_eq!(b.totals(issuer, Basis::Cash), (d("100.00"), Decimal::ZERO));
    assert_eq!(flow(&b.l, e3, &[whole(issuer)], &money).unwrap(), f1);
}

/// Defect 24: an accrual through a liability that is not money (accrued
/// expenses, deferred revenue) counts once, when it is settled, not also
/// when it is booked. Before P3 every liability but payables and payroll
/// was money: the vendor read paid 600.00 and the customer received
/// 1,300.00.
#[test]
fn defect24_an_accrual_through_another_liability_counts_once() {
    let mut b = books();
    let (v, c) = (b.vendor, b.customer);
    let accrue = b.post(
        date(3, 31),
        &[(b.hosting, Dr, "300.00"), (b.accrued, Cr, "300.00")],
        &[whole(v)],
    );
    let (t, _) = b.resolved(accrue);
    assert_eq!((t.direction, t.cash), (Direction::Neither, Decimal::ZERO));
    b.post(
        date(4, 15),
        &[(b.accrued, Dr, "300.00"), (b.cash, Cr, "300.00")],
        &[whole(v)],
    );
    assert_eq!(b.totals(v, Basis::All), (d("300.00"), Decimal::ZERO));
    // Deferred revenue: 1,200.00 received ahead, 100.00 earned.
    b.post(
        date(1, 2),
        &[(b.cash, Dr, "1200.00"), (b.deferred, Cr, "1200.00")],
        &[whole(c)],
    );
    let earn = b.post(
        date(1, 31),
        &[(b.deferred, Dr, "100.00"), (b.revenue, Cr, "100.00")],
        &[whole(c)],
    );
    assert_eq!(b.resolved(earn).0.direction, Direction::Neither);
    assert_eq!(b.totals(c, Basis::All), (Decimal::ZERO, d("1200.00")));
}

/// Defect 24 and B-35 item 6: a credit to what the company owes a party
/// itself never counts as paying it. A fee on the issuer's own card is a
/// bill from the issuer (counted when the card is paid); a distribution
/// still owed to a member counts when it is paid; equipment bought on the
/// seller's own note is not paid for until the note is (Helvering v.
/// Price); property contributed subject to a loan is the member's
/// property net of the loan, all in kind (Treas. Reg.
/// §1.704-1(b)(2)(iv)(b)(2)).
#[test]
fn defect24_a_credit_to_a_partys_own_liability_never_pays_it() {
    let mut b = books();
    let (issuer, t, m) = (b.issuer, b.tobin, b.morgan);
    // The card fee, and the card paid off: until the books know the card
    // is the issuer's, the fee reads as paid twice (150.00).
    let fee = b.post(
        date(5, 1),
        &[(b.fees, Dr, "25.00"), (b.card, Cr, "25.00")],
        &[whole(issuer)],
    );
    b.post(
        date(5, 20),
        &[(b.card, Dr, "125.00"), (b.cash, Cr, "125.00")],
        &[whole(issuer)],
    );
    assert_eq!(b.totals(issuer, Basis::Cash), (d("150.00"), Decimal::ZERO));
    b.l.set_party_creditor(issuer, b.card, true).unwrap();
    assert_eq!(b.totals(issuer, Basis::Cash), (d("125.00"), Decimal::ZERO));
    assert_eq!(b.totals(issuer, Basis::All), (d("125.00"), Decimal::ZERO));
    let (ft, fp) = b.resolved(fee);
    assert_eq!(
        (ft.direction, fp[0].amount),
        (Direction::Neither, d("-25.00"))
    );

    // A distribution declared through 2100, the members' own: owed, so it
    // counts as nothing until it is paid; then 200.00, once.
    b.l.set_party_creditor(t, b.due, true).unwrap();
    b.l.set_party_creditor(m, b.due, true).unwrap();
    let declared = b.post(
        date(6, 30),
        &[(b.cap_t, Dr, "200.00"), (b.due, Cr, "200.00")],
        &[whole(t)],
    );
    let (dt, dp) = b.resolved(declared);
    assert_eq!(dt.direction, Direction::Neither);
    assert_eq!(dp, vec![part(t, "200.00", "0", "0")]);
    assert_eq!(b.totals(t, Basis::All), (Decimal::ZERO, Decimal::ZERO));
    b.post(
        date(7, 15),
        &[(b.due, Dr, "200.00"), (b.cash, Cr, "200.00")],
        &[whole(t)],
    );
    assert_eq!(b.totals(t, Basis::Cash), (d("200.00"), Decimal::ZERO));
    assert_eq!(b.totals(t, Basis::All), (d("200.00"), Decimal::ZERO));

    // Equipment on the seller's own note: nothing paid until the note is.
    let seller = b.l.add_party("Lakeside Supply", &[]).unwrap();
    b.l.set_party_creditor(seller, b.note, true).unwrap();
    let bought = b.post(
        date(8, 1),
        &[(b.equip, Dr, "900.00"), (b.note, Cr, "900.00")],
        &[whole(seller)],
    );
    assert_eq!(
        b.resolved(bought).1,
        vec![part(seller, "-900.00", "0", "0")]
    );
    b.post(
        date(9, 1),
        &[(b.note, Dr, "300.00"), (b.cash, Cr, "300.00")],
        &[whole(seller)],
    );
    assert_eq!(b.totals(seller, Basis::All), (d("300.00"), Decimal::ZERO));

    // Property worth 1,000.00 contributed subject to a 200.00 loan: 800.00
    // received, in kind (before P3: "paid 200.00").
    let lender = b.l.add_party("Harbor Bank Loans", &[]).unwrap();
    b.l.set_party_creditor(lender, b.note, true).unwrap();
    let e = b.post(
        date(9, 15),
        &[
            (b.equip, Dr, "1000.00"),
            (b.note, Cr, "200.00"),
            (b.cap_m, Cr, "800.00"),
        ],
        &[whole(m)],
    );
    let (pt, pp) = b.resolved(e);
    assert_eq!(pt.direction, Direction::Received);
    assert_eq!(pp, vec![part(m, "-800.00", "0", "-800.00")]);
    assert_eq!(b.totals(m, Basis::Cash), (Decimal::ZERO, Decimal::ZERO));
    assert_eq!(b.totals(m, Basis::All), (Decimal::ZERO, d("800.00")));
}

/// The known limit B-35 states (and the tool text says): when the
/// members aren't named the creditors of their financing account, a
/// distribution declared through it counts when declared and again when
/// paid. Pinned so a change to it is seen.
#[test]
fn a_distribution_owed_counts_twice_until_the_members_are_named_its_creditors() {
    let mut b = books();
    let t = b.tobin;
    b.post(
        date(6, 30),
        &[(b.cap_t, Dr, "200.00"), (b.due, Cr, "200.00")],
        &[whole(t)],
    );
    b.post(
        date(7, 15),
        &[(b.due, Dr, "200.00"), (b.cash, Cr, "200.00")],
        &[whole(t)],
    );
    assert_eq!(b.totals(t, Basis::Cash), (d("400.00"), Decimal::ZERO));
    b.l.set_party_creditor(t, b.due, true).unwrap();
    assert_eq!(b.totals(t, Basis::Cash), (d("200.00"), Decimal::ZERO));
}

/// Defect 24 in a member's entry (found in review, 2026-10-21): a
/// distribution declared to a liability that isn't money (distributions
/// payable) is a promise. It counted as 200.00 paid when declared and
/// again when paid (400.00); now it counts nothing until it is paid, then
/// 200.00, once, on both bases (Helvering v. Price; party_target
/// INVARIANT 2). The statement of members' equity counts it when declared:
/// party totals catch up when it is paid (B-35 item 4's conditions).
#[test]
fn defect24_a_distribution_declared_to_a_payable_counts_once_when_paid() {
    let mut b = books();
    let t = b.tobin;
    let declared = b.post(
        date(6, 30),
        &[(b.dist_t, Dr, "200.00"), (b.dist_payable, Cr, "200.00")],
        &[whole(t)],
    );
    let (dt, dp) = b.resolved(declared);
    assert_eq!(dt.direction, Direction::Neither);
    assert_eq!(dp, vec![part(t, "200.00", "0", "0")]);
    for basis in Basis::ALL {
        assert_eq!(b.totals(t, basis), (Decimal::ZERO, Decimal::ZERO));
    }
    b.post(
        date(7, 15),
        &[(b.dist_payable, Dr, "200.00"), (b.cash, Cr, "200.00")],
        &[whole(t)],
    );
    for basis in Basis::ALL {
        assert_eq!(b.totals(t, basis), (d("200.00"), Decimal::ZERO));
    }
    // Declared and half paid at once: the money counts now, the rest when
    // paid.
    let half = b.post(
        date(8, 31),
        &[
            (b.dist_t, Dr, "300.00"),
            (b.cash, Cr, "100.00"),
            (b.dist_payable, Cr, "200.00"),
        ],
        &[whole(t)],
    );
    assert_eq!(
        b.resolved(half),
        (
            Target {
                amount: d("300.00"),
                direction: Direction::Paid,
                cash: d("100.00"),
                in_kind: Decimal::ZERO,
                counts_as: Direction::Paid,
            },
            vec![part(t, "300.00", "100.00", "0")]
        )
    );
    assert_eq!(b.totals(t, Basis::Cash), (d("300.00"), Decimal::ZERO));
}

/// B-35 item 3: a distribution's cash is the money it paid and its in
/// kind the property it gave. Equipment carried at 1,000.00 less 300.00
/// depreciated, and 100.00 in cash, distributed: 100.00 cash and 700.00
/// in kind (it read 800.00 in kind and no cash, the equipment's cost
/// standing in for the money). Basis all is the statement's 800.00.
#[test]
fn a_distribution_of_money_and_depreciated_property_splits_by_what_moved() {
    let mut b = books();
    let t = b.tobin;
    let e = b.post(
        date(9, 1),
        &[
            (b.dist_t, Dr, "800.00"),
            (b.accum, Dr, "300.00"),
            (b.equip, Cr, "1000.00"),
            (b.cash, Cr, "100.00"),
        ],
        &[whole(t)],
    );
    assert_eq!(b.resolved(e).1, vec![part(t, "800.00", "100.00", "700.00")]);
    assert_eq!(b.totals(t, Basis::Cash), (d("100.00"), Decimal::ZERO));
    assert_eq!(b.totals(t, Basis::All), (d("800.00"), Decimal::ZERO));
    let (start, end) = year();
    let me = members_equity(&b.l, start, end, &b.capitals()).unwrap();
    assert_eq!(me.members[0].distributions, d("800.00"));
    // At fair market value with a gain on it, and no money: all in kind.
    let gain = b.post(
        date(9, 2),
        &[
            (b.dist_t, Dr, "900.00"),
            (b.accum, Dr, "300.00"),
            (b.equip, Cr, "1000.00"),
            (b.revenue, Cr, "200.00"),
        ],
        &[whole(t)],
    );
    assert_eq!(b.resolved(gain).1, vec![part(t, "900.00", "0", "900.00")]);
}

/// Defect 24 and the card rule (B-35 item 5): a gift card the company
/// sold is deferred revenue, a promise, though its account's name says
/// "card". It counted as received when sold and again when redeemed
/// (200.00); now once, when the money came in. A card the company pays
/// with is still a card.
#[test]
fn defect24_a_gift_card_sold_is_received_once() {
    let mut b = books();
    let c = b.customer;
    assert_eq!(ledger_core::parties::card_accounts(&b.l), vec![b.card]);
    b.post(
        date(3, 1),
        &[(b.cash, Dr, "100.00"), (b.gift, Cr, "100.00")],
        &[whole(c)],
    );
    let redeemed = b.post(
        date(3, 5),
        &[(b.gift, Dr, "100.00"), (b.revenue, Cr, "100.00")],
        &[whole(c)],
    );
    assert_eq!(b.resolved(redeemed).0.direction, Direction::Neither);
    assert_eq!(b.totals(c, Basis::All), (Decimal::ZERO, d("100.00")));
}

/// B-35 item 5 and `parties::card_accounts` INVARIANTS 1-4: one rule for
/// what a card is. Before any business card is recorded, the liabilities
/// whose names say "card" but not "gift" (2050 Business Card; never 2410
/// Gift Card Liability), less a member's card's account; once one is
/// recorded, exactly the business cards' liabilities, whatever their
/// names, and a debit card on the bank (an asset) is none. A member's
/// card or a platform is never the company's card, and a card read as a
/// card is money: a charge to it is paid.
#[test]
fn a_card_is_a_recorded_business_card_else_a_liability_named_card() {
    use ledger_core::InstrumentKind as K;
    let mut b = books();
    let cards = |b: &Books| ledger_core::parties::card_accounts(&b.l);
    assert_eq!(cards(&b), vec![b.card]);
    // A member's own card, on a liability named for it: not the
    // company's, before or after a business card is recorded.
    let personal =
        b.l.add_account("2060", "Tobin's Card", AccountType::Liability, None, false)
            .unwrap();
    assert_eq!(cards(&b), vec![b.card, personal]);
    b.l.add_instrument(K::MemberCard, "Tobin's card", None, Some(b.tobin), personal)
        .unwrap();
    assert_eq!(
        cards(&b),
        vec![b.card],
        "a member's card is not a card account"
    );
    // A platform's balance is no card either.
    b.l.add_instrument(K::Platform, "Payment platform", None, None, b.prepaid)
        .unwrap();
    assert_eq!(cards(&b), vec![b.card]);
    // A business card on the bank (a debit card, an asset): no card
    // account, so the name rule still holds.
    b.l.add_instrument(K::BusinessCard, "Debit card", None, None, b.cash)
        .unwrap();
    assert_eq!(cards(&b), vec![b.card]);
    // A business card recorded on a liability not named "card": now
    // exactly the recorded cards' liabilities; 2050, named "card" but not
    // recorded, no longer is.
    let visa =
        b.l.add_account("2070", "Harbor Visa", AccountType::Liability, None, false)
            .unwrap();
    b.l.add_instrument(K::BusinessCard, "Harbor Visa", None, None, visa)
        .unwrap();
    assert_eq!(cards(&b), vec![visa]);
    assert_eq!(
        b.l.card_accounts().into_iter().collect::<Vec<_>>(),
        vec![visa]
    );
    // Money follows the rule: a charge to the recorded card is paid; one
    // to 2050, a liability like any other now, is a promise.
    let v = b.vendor;
    let on_visa = b.post(
        date(4, 1),
        &[(b.hosting, Dr, "30.00"), (visa, Cr, "30.00")],
        &[whole(v)],
    );
    let on_2050 = b.post(
        date(4, 2),
        &[(b.hosting, Dr, "20.00"), (b.card, Cr, "20.00")],
        &[whole(v)],
    );
    assert_eq!(b.resolved(on_visa).0.direction, Direction::Paid);
    assert_eq!(b.resolved(on_2050).0.direction, Direction::Neither);
}

/// B-35 item 3, case 3: E is the movement of equity other than retained
/// earnings, so a members' entry reads as the statement of members' equity
/// reads it. Retained earnings given up for equipment moves no member's
/// capital: neither, as the statement shows no distribution. Retained
/// earnings moved into the members' capital outside a close is, on the
/// statement and the K-1 worksheet, a contribution (credits to capital),
/// and party totals agree: received, cash (an open question for the
/// owner: a CPA would call it neither).
#[test]
fn retained_earnings_is_not_a_members_capital() {
    let mut b = books();
    let (t, m) = (b.tobin, b.morgan);
    let re =
        b.l.add_account(
            "3999",
            "Retained Earnings",
            AccountType::Equity,
            None,
            false,
        )
        .unwrap();
    assert_eq!(b.l.role_account(&ledger_core::RETAINED_EARNINGS), Some(re));
    let written_off = b.post(
        date(4, 1),
        &[(re, Dr, "100.00"), (b.equip, Cr, "100.00")],
        &[whole(t)],
    );
    assert_eq!(b.resolved(written_off).0.direction, Direction::Neither);
    let allocated = b.post(
        date(4, 2),
        &[
            (re, Dr, "300.00"),
            (b.cap_t, Cr, "150.00"),
            (b.cap_m, Cr, "150.00"),
        ],
        &[whole(t), whole(m)],
    );
    assert_eq!(
        b.resolved(allocated).1,
        vec![
            part(t, "-150.00", "-150.00", "0"),
            part(m, "-150.00", "-150.00", "0")
        ]
    );
    let (start, end) = year();
    let me = members_equity(&b.l, start, end, &b.capitals()).unwrap();
    let k1 = k1_detail(&b.l, start, end, &b.capitals(), &[b.cash]).unwrap();
    for (i, who) in [t, m].into_iter().enumerate() {
        let (paid, received) = b.totals(who, Basis::All);
        assert_eq!(
            (received, paid),
            (me.members[i].contributions, me.members[i].distributions)
        );
        assert_eq!(b.totals(who, Basis::Cash).1, k1.rows[i].contributions_cash);
    }
}

/// Activity's paid is net of reversals of payments; a refund recorded as
/// its own entry (money back from the vendor) is received from the vendor,
/// as it is: Paid out 100.00, Received in 20.00, net 80.00.
#[test]
fn a_refund_is_received_from_the_vendor() {
    let mut b = books();
    let v = b.vendor;
    b.post(
        date(3, 1),
        &[(b.hosting, Dr, "100.00"), (b.cash, Cr, "100.00")],
        &[whole(v)],
    );
    b.post(
        date(3, 5),
        &[(b.cash, Dr, "20.00"), (b.hosting, Cr, "20.00")],
        &[whole(v)],
    );
    assert_eq!(b.totals(v, Basis::Cash), (d("100.00"), d("20.00")));
}

/// B-35 item 3 (capital first): a mixed contribution is split into money
/// and property exactly as the K-1 worksheet splits it (its SCENARIO 3),
/// whichever member the shares list first; an expense a member pays
/// straight to capital is cash, as on the K-1 (k1 INVARIANT 3); a debt
/// converted to capital is cash.
#[test]
fn a_members_mixed_entry_splits_as_the_k1_does() {
    let mut b = books();
    let (t, m) = (b.tobin, b.morgan);
    let lines = [
        (b.cash, Dr, "600.00"),
        (b.equip, Dr, "400.00"),
        (b.cap_t, Cr, "500.00"),
        (b.cap_m, Cr, "500.00"),
    ];
    let e = b.post(date(2, 1), &lines, &[whole(t), whole(m)]);
    assert_eq!(
        b.resolved(e).1,
        vec![
            part(t, "-500.00", "-300.00", "-200.00"),
            part(m, "-500.00", "-300.00", "-200.00")
        ]
    );
    // Listed the other way round: the same split.
    b.l.set_entry_parties(e, &[whole(m), whole(t)]).unwrap();
    assert_eq!(
        b.resolved(e).1,
        vec![
            part(m, "-500.00", "-300.00", "-200.00"),
            part(t, "-500.00", "-300.00", "-200.00")
        ]
    );
    // A tied odd cent goes to the member listed first in the company's
    // members (Tobin), as on the K-1 worksheet, whatever the shares' order.
    let odd = b.post(
        date(2, 2),
        &[
            (b.cash, Dr, "0.99"),
            (b.equip, Dr, "0.01"),
            (b.cap_t, Cr, "0.50"),
            (b.cap_m, Cr, "0.50"),
        ],
        &[whole(m), whole(t)],
    );
    assert_eq!(
        b.resolved(odd).1,
        vec![
            part(m, "-0.50", "-0.50", "0"),
            part(t, "-0.50", "-0.49", "-0.01")
        ]
    );
    let expense = b.post(
        date(2, 3),
        &[(b.hosting, Dr, "50.00"), (b.cap_m, Cr, "50.00")],
        &[whole(m)],
    );
    assert_eq!(
        b.resolved(expense).1,
        vec![part(m, "-50.00", "-50.00", "0")]
    );
    let converted = b.post(
        date(2, 4),
        &[(b.due, Dr, "70.00"), (b.cap_t, Cr, "70.00")],
        &[whole(t)],
    );
    assert_eq!(
        b.resolved(converted).1,
        vec![part(t, "-70.00", "-70.00", "0")]
    );
    // Against the K-1 worksheet, member by member.
    let (start, end) = year();
    let k1 = k1_detail(&b.l, start, end, &b.capitals(), &[b.cash]).unwrap();
    for (i, who) in [t, m].into_iter().enumerate() {
        let (_, cash) = b.totals(who, Basis::Cash);
        let (_, all) = b.totals(who, Basis::All);
        assert_eq!(cash, k1.rows[i].contributions_cash, "member {i}");
        assert_eq!(all - cash, k1.rows[i].contributions_property, "member {i}");
    }
    // Tobin: 300 + 0.49 + 70 cash, 200 + 0.01 property; Morgan: 300 +
    // 0.50 + 50 cash, 200 property.
    assert_eq!(b.totals(t, Basis::Cash), (Decimal::ZERO, d("370.49")));
    assert_eq!(b.totals(m, Basis::Cash), (Decimal::ZERO, d("350.50")));
    assert_eq!(b.totals(t, Basis::All), (Decimal::ZERO, d("570.50")));
}

/// R2 of the design review: a member's entry is read from journaled facts
/// (the member role, the chart), not today's settings: it keeps its sign,
/// direction and property after the member leaves the company's members
/// and is archived; and a one-member company's property contribution
/// reads received, in kind, with no members in the settings at all.
#[test]
fn a_members_entry_keeps_its_reading_after_they_leave() {
    let mut b = books();
    let t = b.tobin;
    let e = b.post(
        date(3, 1),
        &[(b.equip, Dr, "400.00"), (b.cap_t, Cr, "400.00")],
        &[whole(t)],
    );
    let before = b.resolved(e);
    b.l.update_party(t, PartyField::Archived(true)).unwrap();
    let after = resolve(&b.l, e, b.l.entry_parties(e), &b.money(), &[]).unwrap();
    assert_eq!((after.target, after.parts), before);
    assert_eq!(before.0.direction, Direction::Received);
    // One member, no members in the settings.
    let mut l = Ledger::new(Currency::USD);
    let equip = l
        .add_account("1500", "Equipment", AccountType::Asset, None, false)
        .unwrap();
    let cash = l
        .add_account("1000", "Checking", AccountType::Asset, None, false)
        .unwrap();
    let capital = l
        .add_account("3000", "Owner's Capital", AccountType::Equity, None, false)
        .unwrap();
    let owner = l.add_party("Morgan Ellis", &[Role::Member]).unwrap();
    let e = l.try_create_entry(date(3, 1), "laptop").unwrap();
    l.add_line(e, equip, Dr, Money::new(d("900.00"), Currency::USD))
        .unwrap();
    l.add_line(e, capital, Cr, Money::new(d("900.00"), Currency::USD))
        .unwrap();
    l.post_entry(e).unwrap();
    l.set_entry_party(e, Some(owner)).unwrap();
    let money = money_accounts(&l, &[cash], &[]);
    let r = resolve(&l, e, l.entry_parties(e), &money, &[]).unwrap();
    assert_eq!(r.target.direction, Direction::Received);
    assert_eq!(r.parts, vec![part(owner, "-900.00", "0", "-900.00")]);
    assert_eq!(
        activity_with(&l, owner, year(), &money, &[]).totals(Basis::All),
        (Decimal::ZERO, d("900.00"))
    );
}

/// B-35 item 3, case 1: a period's closing entry moves nothing with
/// anyone: direction neither, its members' part their income allocated
/// (credited to their capital: negative), counted in no total.
#[test]
fn a_close_tagged_with_its_members_is_neither() {
    let mut b = books();
    let (t, m) = (b.tobin, b.morgan);
    b.post(
        date(1, 10),
        &[(b.cash, Dr, "100.00"), (b.revenue, Cr, "100.00")],
        &[whole(b.customer)],
    );
    let jan = b.l.create_period("Jan", date(1, 1), date(1, 31)).unwrap();
    let close =
        b.l.close_period_to(
            jan,
            &ledger_core::CloseTo::Members(vec![
                ledger_core::MemberClose {
                    capital: b.cap_t,
                    share: d("0.5"),
                    distributions: None,
                },
                ledger_core::MemberClose {
                    capital: b.cap_m,
                    share: d("0.5"),
                    distributions: None,
                },
            ]),
        )
        .unwrap()
        .unwrap();
    b.l.set_entry_parties(close, &[whole(t), whole(m)]).unwrap();
    let (ct, cp) = b.resolved(close);
    assert_eq!(ct.direction, Direction::Neither);
    assert_eq!(ct.counts_as, Direction::Neither);
    assert_eq!(
        cp,
        vec![part(t, "-50.00", "0", "0"), part(m, "-50.00", "0", "0")]
    );
    assert_eq!(b.totals(t, Basis::All), (Decimal::ZERO, Decimal::ZERO));
}

/// Basis::parse and key: "cash" and "all", nothing else (surrounding space
/// ignored); cash is the default.
#[test]
fn a_basis_is_cash_or_all() {
    assert_eq!(Basis::parse("cash"), Some(Basis::Cash));
    assert_eq!(Basis::parse(" all "), Some(Basis::All));
    for bad in ["", "accrual", "Cash", "ALL", "both"] {
        assert_eq!(Basis::parse(bad), None, "{bad:?}");
    }
    assert_eq!(Basis::default(), Basis::Cash);
    for b in Basis::ALL {
        assert_eq!(Basis::parse(b.key()), Some(b));
    }
}

/// Ledger::set_party_creditor SCENARIOS: only a liability that takes
/// postings and has one creditor; a refusal changes nothing; naming twice
/// or stopping one never named changes nothing; an account may have
/// several creditors (both members, for what the members advanced).
#[test]
fn a_creditor_is_named_only_on_a_liability_with_one_owner() {
    let mut b = books();
    let (t, m, issuer) = (b.tobin, b.morgan, b.issuer);
    let header =
        b.l.add_account(
            "2900",
            "Other Liabilities",
            AccountType::Liability,
            None,
            true,
        )
        .unwrap();
    let payables =
        b.l.add_account(
            "2000",
            "Accounts Payable",
            AccountType::Liability,
            None,
            false,
        )
        .unwrap();
    assert_eq!(b.l.role_account(&ledger_core::PAYABLES), Some(payables));
    let federal =
        b.l.add_account(
            "2110",
            "Federal Withholding Payable",
            AccountType::Liability,
            None,
            false,
        )
        .unwrap();
    let before = b.l.clone();
    for (what, party, account) in [
        ("an asset", issuer, b.cash),
        ("equity", issuer, b.cap_t),
        ("an unknown party", 999, b.card),
        ("an unknown account", issuer, 999),
        ("a placeholder", issuer, header),
        ("payables", issuer, payables),
        ("a payroll withholding", issuer, federal),
    ] {
        assert!(
            b.l.set_party_creditor(party, account, true).is_err(),
            "{what}"
        );
        assert!(b.l == before, "{what}: a refusal changed the ledger");
    }
    b.l.set_party_creditor(issuer, b.card, true).unwrap();
    let once = b.l.clone();
    b.l.set_party_creditor(issuer, b.card, true).unwrap();
    assert!(b.l == once, "naming twice changes nothing");
    assert_eq!(b.l.creditors_of(b.card), vec![issuer]);
    b.l.set_party_creditor(t, b.due, true).unwrap();
    b.l.set_party_creditor(m, b.due, true).unwrap();
    assert_eq!(b.l.creditors_of(b.due), vec![t, m]);
    b.l.set_party_creditor(m, b.due, false).unwrap();
    let stopped = b.l.clone();
    b.l.set_party_creditor(m, b.due, false).unwrap();
    assert!(b.l == stopped, "stopping one never named changes nothing");
    assert_eq!(b.l.creditors_of(b.due), vec![t]);
    assert!(b.l.get_party(t).unwrap().creditor_of.contains(&b.due));
}

/// Defect 11: Party::tin_last4 SCENARIOS 1-5.
#[test]
fn defect11_a_partys_tin_last4_is_its_tin_facts_else_its_w9s() {
    let mut l = Ledger::new(Currency::USD);
    let member = l.add_party("Tobin Harlow", &[Role::Member]).unwrap();
    let contractor = l.add_party("Riley Chen", &[Role::Contractor]).unwrap();
    let both = l.add_party("Morgan Ellis", &[Role::Contractor]).unwrap();
    let neither = l.add_party("Lakeside Supply", &[]).unwrap();
    let tin = |last4: &str| TinFacts {
        tin_type: TinType::Ssn,
        last4: last4.into(),
        vault_record: Some("a".repeat(64)),
        purged: None,
    };
    let w9 = |last4: &str| W9Facts {
        received: date(10, 15),
        entity_type: "individual/sole proprietor".into(),
        exempt_payee_code: None,
        fatca_code: None,
        tin_last4: last4.into(),
        kept_at: "wanderbooks vault".into(),
        vault_record: None,
    };
    l.set_party_tin(member, Some(tin("4321"))).unwrap();
    l.set_party_w9(contractor, Some(w9("8765"))).unwrap();
    l.set_party_tin(both, Some(tin("1111"))).unwrap();
    l.set_party_w9(both, Some(w9("1111"))).unwrap();
    let last4 = |l: &Ledger, p| l.get_party(p).unwrap().tin_last4().map(str::to_string);
    assert_eq!(last4(&l, member).as_deref(), Some("4321"));
    assert_eq!(last4(&l, contractor).as_deref(), Some("8765"));
    assert_eq!(last4(&l, both).as_deref(), Some("1111"));
    assert_eq!(last4(&l, neither), None);
    // Purged from the vault: the facts keep the last four.
    let mut purged = tin("4321");
    purged.vault_record = None;
    purged.purged = Some("purged on 2026-10-20: retention ended".into());
    l.set_party_tin(member, Some(purged)).unwrap();
    assert_eq!(last4(&l, member).as_deref(), Some("4321"));
    // SCENARIO 6: never more than four digits, however a party was built
    // (here by hand, past the checking setters).
    let mut by_hand = l.get_party(member).unwrap().clone();
    by_hand.tin.as_mut().unwrap().last4 = "123456789".into();
    assert_eq!(by_hand.tin_last4(), None);
    let mut by_hand = l.get_party(both).unwrap().clone();
    by_hand.tin.as_mut().unwrap().last4 = "11111".into();
    assert_eq!(by_hand.tin_last4(), Some("1111"), "the W-9's four");
    by_hand.w9.as_mut().unwrap().tin_last4 = "12-3456789".into();
    assert_eq!(by_hand.tin_last4(), None);
}

/// The kernel's B-35 cases, by hand (party_target INVARIANTS 1-6) and
/// reverse_target an involution.
#[test]
fn party_target_cases_by_hand() {
    let t = |amount, direction, cash, in_kind| PartyTarget {
        amount,
        direction,
        cash,
        in_kind,
    };
    // A contribution of 800, 1000 of property given (more than |e|): all
    // in kind, whatever money the entry moved.
    assert_eq!(
        kernel::party_target(Some((-800i64, 1000)), 200, 0, 1000),
        Some(t(-800, Direction::Received, 0, -800))
    );
    // A contribution: the property in it, the rest cash.
    assert_eq!(
        kernel::party_target(Some((-500i64, 200)), -500, 0, 500),
        Some(t(-500, Direction::Received, -300, -200))
    );
    // A contribution with no money at all (an expense booked straight to
    // capital): cash, as the K-1 worksheet counts it.
    assert_eq!(
        kernel::party_target(Some((-50i64, 0)), 0, 0, 50),
        Some(t(-50, Direction::Received, -50, 0))
    );
    // A distribution of 150: 20 paid in money, 100 in property, 30 still
    // owed.
    assert_eq!(
        kernel::party_target(Some((150i64, 100)), 20, 0, 150),
        Some(t(150, Direction::Paid, 20, 100))
    );
    // Money first, then property, each at most what is left.
    assert_eq!(
        kernel::party_target(Some((800i64, 1000)), 100, 0, 1100),
        Some(t(800, Direction::Paid, 100, 700))
    );
    assert_eq!(
        kernel::party_target(Some((100i64, 0)), 150, 0, 150),
        Some(t(100, Direction::Paid, 100, 0))
    );
    // A distribution all owed (declared, no money, no property): nothing
    // counts, neither. Money received on it doesn't count either.
    assert_eq!(
        kernel::party_target(Some((200i64, 0)), 0, 200, 200),
        Some(t(200, Direction::Neither, 0, 0))
    );
    assert_eq!(
        kernel::party_target(Some((200i64, 0)), -50, 0, 250),
        Some(t(200, Direction::Neither, 0, 0))
    );
    // Not every party a member, or no equity moved: money, then own, then
    // the debits.
    assert_eq!(
        kernel::party_target(Some((0i64, 0)), -70, 0, 70),
        Some(t(-70, Direction::Received, -70, 0))
    );
    assert_eq!(
        kernel::party_target(None, 0i64, -80, 80),
        Some(t(-80, Direction::Neither, 0, 0))
    );
    assert_eq!(
        kernel::party_target(None, 0i64, 0, 80),
        Some(t(80, Direction::Neither, 0, 0))
    );
    let x = t(-800i64, Direction::Received, -300, -500);
    let r = kernel::reverse_target(x).unwrap();
    assert_eq!(r, t(800, Direction::Paid, 300, 500));
    assert_eq!(kernel::reverse_target(r), Some(x));
    // Past the range: refused, never a panic.
    assert_eq!(
        kernel::reverse_target(t(i64::MIN, Direction::Received, 0, 0)),
        None
    );
    assert_eq!(kernel::party_target(Some((i64::MIN, 0)), 0, 0, 0), None);
    // Never a negative zero on Decimal: a contribution all in kind leaves
    // a plain zero of cash (a distribution all in kind read "-0" before
    // the review), and so does a reversal of zero.
    let all_in_kind = kernel::party_target(
        Some((d("-800.00"), d("1000.00"))),
        d("0"),
        d("0"),
        d("1000.00"),
    )
    .unwrap();
    assert_eq!(all_in_kind.cash, Decimal::ZERO);
    assert!(!all_in_kind.cash.is_sign_negative());
    let given = kernel::party_target(
        Some((d("800.00"), d("1000.00"))),
        d("0"),
        d("0"),
        d("1000.00"),
    )
    .unwrap();
    assert_eq!((given.cash, given.in_kind), (Decimal::ZERO, d("800.00")));
    assert!(!given.cash.is_sign_negative());
    let z = kernel::reverse_target(PartyTarget {
        amount: Decimal::ZERO,
        direction: Direction::Neither,
        cash: Decimal::ZERO,
        in_kind: Decimal::ZERO,
    })
    .unwrap();
    assert!(!z.amount.is_sign_negative() && !z.cash.is_sign_negative());
}

// ---------------------------------------------------------- properties

fn cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(256)
}

/// Who an entry names, in the properties below.
#[derive(Clone, Copy, Debug)]
enum Who {
    Vendor,
    Issuer,
    Tobin,
    Members,
    MembersByPercent,
    VendorAndTobinByPercent,
}

fn arb_who() -> impl Strategy<Value = Who> {
    prop_oneof![
        Just(Who::Vendor),
        Just(Who::Issuer),
        Just(Who::Tobin),
        Just(Who::Members),
        Just(Who::MembersByPercent),
        Just(Who::VendorAndTobinByPercent),
    ]
}

fn shares_for(b: &Books, who: Who) -> Vec<PartyShare> {
    let pct = |ps: &[(PartyId, &str)]| -> Vec<PartyShare> {
        ps.iter()
            .map(|&(party, p)| PartyShare {
                party,
                share: Share::Percent(d(p)),
            })
            .collect()
    };
    match who {
        Who::Vendor => vec![whole(b.vendor)],
        Who::Issuer => vec![whole(b.issuer)],
        Who::Tobin => vec![whole(b.tobin)],
        Who::Members => vec![whole(b.tobin), whole(b.morgan)],
        Who::MembersByPercent => pct(&[(b.tobin, "60"), (b.morgan, "40")]),
        Who::VendorAndTobinByPercent => pct(&[(b.vendor, "70"), (b.tobin, "30")]),
    }
}

/// A balanced entry over the fixture's accounts (by index into
/// `Books::pool`): one or two debits, one or two credits, the last credit
/// plugging the debits' total.
fn arb_lines() -> impl Strategy<Value = Vec<(usize, Side, i64)>> {
    (
        prop::collection::vec((0usize..POOL, 1i64..500_000), 1..3),
        prop::collection::vec((0usize..POOL, 1i64..500_000), 0..2),
        0usize..POOL,
    )
        .prop_filter_map("a positive plug", |(debits, credits, last)| {
            let total: i64 = debits.iter().map(|(_, c)| c).sum();
            let rest: i64 = credits.iter().map(|(_, c)| c).sum();
            (total > rest).then(|| {
                let mut out: Vec<(usize, Side, i64)> =
                    debits.into_iter().map(|(a, c)| (a, Dr, c)).collect();
                out.extend(credits.into_iter().map(|(a, c)| (a, Cr, c)));
                out.push((last, Cr, total - rest));
                out
            })
        })
}

const POOL: usize = 11;

impl Books {
    fn pool(&self) -> [AccountId; POOL] {
        [
            self.cash,
            self.card,
            self.due,
            self.note,
            self.accrued,
            self.equip,
            self.prepaid,
            self.cap_t,
            self.cap_m,
            self.hosting,
            self.revenue,
        ]
    }

    fn post_cents(&mut self, day: NaiveDate, lines: &[(usize, Side, i64)]) -> EntryId {
        let pool = self.pool();
        let e = self.l.try_create_entry(day, "x").unwrap();
        for &(a, side, c) in lines {
            self.l
                .add_line(
                    e,
                    pool[a],
                    side,
                    Money::new(Decimal::new(c, 2), Currency::USD),
                )
                .unwrap();
        }
        self.l.post_entry(e).unwrap();
        e
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases()))]

    /// Properties 1, 2 and 6 of the brief (shares INVARIANTS 6, 11, 12;
    /// parties::flow INVARIANT 2): for any posted entry and any parties,
    /// with any of them named its creditors, the parts sum to the target
    /// (and their cash and in_kind to its), each has the target's sign and
    /// never exceeds its part; a reversal's target and parts are its
    /// original's negated, the direction flipped, on the same side; and
    /// its flow is the original's negated.
    #[test]
    fn a_reversal_is_the_exact_opposite(
        lines in arb_lines(),
        who in arb_who(),
        owed in prop::collection::vec((0usize..3, 0usize..4), 0..3),
    ) {
        let mut b = books();
        let liabilities = [b.card, b.due, b.note, b.accrued];
        for (p, a) in owed {
            let party = [b.issuer, b.tobin, b.vendor][p];
            b.l.set_party_creditor(party, liabilities[a], true).unwrap();
        }
        let e = b.post_cents(date(4, 1), &lines);
        let shares = shares_for(&b, who);
        b.l.set_entry_parties(e, &shares).unwrap();
        let r = b.reverse(e, date(4, 2));
        let (money, members) = (b.money(), b.members());
        let orig = resolve(&b.l, e, &shares, &money, &members);
        let rev = resolve(&b.l, r, &shares, &money, &members);
        prop_assert_eq!(orig.is_ok(), rev.is_ok());
        if let (Ok(o), Ok(v)) = (orig, rev) {
            let neg = |x: Decimal| if x.is_zero() { Decimal::ZERO } else { -x };
            prop_assert_eq!(v.target.amount, neg(o.target.amount));
            prop_assert_eq!(v.target.cash, neg(o.target.cash));
            prop_assert_eq!(v.target.in_kind, neg(o.target.in_kind));
            prop_assert_eq!(v.target.direction, o.target.direction.flip());
            prop_assert_eq!(v.target.counts_as, o.target.counts_as);
            prop_assert_eq!(o.target.counts_as, o.target.direction);
            for (po, pv) in o.parts.iter().zip(&v.parts) {
                prop_assert_eq!(pv.party, po.party);
                prop_assert_eq!(pv.amount, neg(po.amount));
                prop_assert_eq!(pv.cash, neg(po.cash));
                prop_assert_eq!(pv.in_kind, neg(po.in_kind));
            }
            for res in [&o, &v] {
                let t = res.target;
                for x in [t.amount, t.cash, t.in_kind] {
                    prop_assert!(!(x.is_zero() && x.is_sign_negative()), "the target: {:?}", t);
                }
                let sum = |f: fn(&Part) -> Decimal| res.parts.iter().map(f).sum::<Decimal>();
                prop_assert_eq!(sum(|p| p.amount), t.amount);
                prop_assert_eq!(sum(|p| p.cash), t.cash);
                prop_assert_eq!(sum(|p| p.in_kind), t.in_kind);
                prop_assert_eq!(
                    t.direction == Direction::Neither,
                    t.cash.is_zero() && t.in_kind.is_zero()
                );
                for p in &res.parts {
                    for x in [p.amount, p.cash, p.in_kind] {
                        prop_assert!(x.is_zero() || (x < Decimal::ZERO) == (t.amount < Decimal::ZERO));
                        prop_assert!(!(x.is_zero() && x.is_sign_negative()));
                    }
                    prop_assert!(p.cash.abs() + p.in_kind.abs() <= p.amount.abs());
                }
            }
        }
        let fo = flow(&b.l, e, &shares, &money).unwrap();
        let fr = flow(&b.l, r, &shares, &money).unwrap();
        prop_assert_eq!(fr.net, -fo.net);
        prop_assert_eq!(
            fr.what_for,
            fo.what_for.iter().map(|(a, x)| (*a, -*x)).collect::<Vec<_>>()
        );
    }

    /// Property 3 of the brief: for an entry naming one party who isn't a
    /// member, whose lines move money only through cash, cards and the
    /// financing accounts, the party's part is entry_flow's net exactly, as
    /// before P3: no money figure changed.
    #[test]
    fn a_sole_partys_money_part_is_the_flow_as_before(lines in arb_lines()) {
        let mut b = books();
        let e = b.post_cents(date(4, 1), &lines);
        b.l.set_entry_party(e, Some(b.vendor)).unwrap();
        let money = b.money();
        let net = entry_flow(b.l.get_entry(e).unwrap(), &money).unwrap().net;
        let (t, parts) = b.resolved(e);
        if !net.is_zero() {
            prop_assert_eq!(t.amount, net);
            prop_assert_eq!(parts[0].cash, net);
            prop_assert_eq!(t.direction, if net > Decimal::ZERO { Direction::Paid } else { Direction::Received });
        } else {
            prop_assert_eq!(t.direction, Direction::Neither);
        }
    }

    /// Property 5 of the brief (metamorphic, member-only books): members
    /// contribute and take out money and property, alone and jointly, some
    /// of it subject to a loan, some through the members' financing
    /// account converted to capital, some through their distributions
    /// accounts, some declared to distributions payable and paid later,
    /// and reverse some of it later in the year. For each member, over
    /// every span from the year's start to any date (B-35 item 4's
    /// conditions hold: a reversal in such a span always has its original
    /// in it): basis all is the statement of members' equity's
    /// contributions and distributions, but for a distribution declared
    /// and not yet paid, which party totals count when it is paid; and
    /// basis cash's received is the K-1 worksheet's cash contributions.
    /// Over any span at all, received less paid on basis all is the
    /// statement's contributions less distributions (the same exception).
    #[test]
    fn member_totals_are_the_members_equity_statement(
        ops in prop::collection::vec(
            (0u8..11, 0usize..2, 1i64..200_000, 1i64..200_000, any::<bool>(), 1u32..30),
            1..12,
        ),
    ) {
        let mut b = books();
        let caps = [b.cap_t, b.cap_m];
        let dists = [b.dist_t, b.dist_m];
        let who = [b.tobin, b.morgan];
        // (member, declared on, paid on, amount): distributions declared
        // to distributions payable.
        let mut declared: Vec<(usize, NaiveDate, NaiveDate, Decimal)> = Vec::new();
        let mut dates = std::collections::BTreeSet::new();
        for (i, &(op, mem, x, y, undo, later)) in ops.iter().enumerate() {
            let day = date(1 + (i as u32 % 11), 1 + (i as u32 % 27));
            let undo_on = day + chrono::Days::new(u64::from(later));
            let (cap, dist, p) = (caps[mem], dists[mem], who[mem]);
            let c = |v: i64| Decimal::new(v, 2).to_string();
            let (sx, sy, sxy, s2y) = (c(x), c(y), c(x + y), c(2 * y));
            let lines: Vec<(AccountId, Side, String)> = match op {
                // Cash in.
                0 => vec![(b.cash, Dr, sx.clone()), (cap, Cr, sx.clone())],
                // Property in.
                1 => vec![(b.equip, Dr, sx.clone()), (cap, Cr, sx.clone())],
                // Cash and property in, one entry.
                2 => vec![(b.cash, Dr, sx.clone()), (b.equip, Dr, sy.clone()), (cap, Cr, sxy.clone())],
                // Property subject to a loan: x + y of it, y the loan.
                3 => vec![(b.equip, Dr, sxy.clone()), (b.note, Cr, sy.clone()), (cap, Cr, sx.clone())],
                // Cash out.
                4 => vec![(cap, Dr, sx.clone()), (b.cash, Cr, sx.clone())],
                // Property out.
                5 => vec![(cap, Dr, sx.clone()), (b.equip, Cr, sx.clone())],
                // What the member advanced (tagged with the vendor), then
                // converted to capital.
                6 => {
                    b.post(day, &[(b.hosting, Dr, &sx), (b.due, Cr, &sx)], &[whole(b.vendor)]);
                    vec![(b.due, Dr, sx.clone()), (cap, Cr, sx.clone())]
                }
                // A joint contribution of cash and property.
                7 => vec![
                    (b.cash, Dr, sx.clone()),
                    (b.prepaid, Dr, sy.clone()),
                    (b.cap_t, Cr, sx.clone()),
                    (b.cap_m, Cr, sy.clone()),
                ],
                // Money and depreciated property out, through the
                // distributions account: x of cash, and equipment of cost
                // 2y with y depreciated (y carried).
                8 => vec![
                    (dist, Dr, sxy.clone()),
                    (b.accum, Dr, sy.clone()),
                    (b.equip, Cr, s2y.clone()),
                    (b.cash, Cr, sx.clone()),
                ],
                // Declared to distributions payable, paid at the year's
                // end.
                9 => {
                    let paid_on = date(12, 31);
                    declared.push((mem, day, paid_on, Decimal::new(x, 2)));
                    let pay = b.post(paid_on, &[(b.dist_payable, Dr, &sx), (b.cash, Cr, &sx)], &[whole(p)]);
                    dates.insert(b.l.get_entry(pay).unwrap().date());
                    vec![(dist, Dr, sx.clone()), (b.dist_payable, Cr, sx.clone())]
                }
                // An expense paid straight to capital.
                _ => vec![(b.hosting, Dr, sx.clone()), (cap, Cr, sx.clone())],
            };
            let shares = if op == 7 { vec![whole(b.tobin), whole(b.morgan)] } else { vec![whole(p)] };
            let lines: Vec<(AccountId, Side, &str)> =
                lines.iter().map(|(a, s, v)| (*a, *s, v.as_str())).collect();
            let e = b.post(day, &lines, &shares);
            dates.insert(day);
            if undo && op != 9 {
                b.reverse(e, undo_on);
                dates.insert(undo_on);
            }
        }
        let (start, end) = year();
        dates.insert(end);
        let check = |span: (NaiveDate, NaiveDate), prefix: bool| -> Result<(), TestCaseError> {
            let me = members_equity(&b.l, span.0, span.1, &b.capitals()).unwrap();
            let k1 = k1_detail(&b.l, span.0, span.1, &b.capitals(), &[b.cash]).unwrap();
            let within = |d: NaiveDate| d >= span.0 && d <= span.1;
            for (i, p) in [b.tobin, b.morgan].into_iter().enumerate() {
                let (paid_all, received_all) = activity_with(&b.l, p, span, &b.money(), &b.members()).totals(Basis::All);
                let (_, received_cash) = activity_with(&b.l, p, span, &b.money(), &b.members()).totals(Basis::Cash);
                // Declared in the span: in the statement, not yet in party
                // totals; paid in the span: in party totals only.
                let owed: Decimal = declared.iter().filter(|x| x.0 == i && within(x.1)).map(|x| x.3).sum();
                let paid: Decimal = declared.iter().filter(|x| x.0 == i && within(x.2)).map(|x| x.3).sum();
                let col = &me.members[i];
                prop_assert_eq!(
                    received_all - paid_all,
                    col.contributions - col.distributions + owed - paid,
                    "member {} over {:?}", i, span
                );
                if prefix {
                    prop_assert_eq!(received_all, col.contributions, "member {} through {}", i, span.1);
                    prop_assert_eq!(paid_all, col.distributions - owed + paid, "member {} through {}", i, span.1);
                    prop_assert_eq!(received_cash, k1.rows[i].contributions_cash, "member {} through {}", i, span.1);
                }
            }
            Ok(())
        };
        for &through in &dates {
            check((start, through), true)?;
        }
        let ds: Vec<NaiveDate> = dates.iter().copied().collect();
        for (n, &from) in ds.iter().enumerate() {
            for &to in &ds[n..] {
                check((from, to), false)?;
            }
        }
    }

    /// Defect 11 property: for a party built by any sequence of TIN and
    /// W-9 facts the ledger accepts, tin_last4 is every source present.
    #[test]
    fn tin_last4_is_every_source_present(
        steps in prop::collection::vec((any::<bool>(), 0u16..3, any::<bool>()), 0..8),
    ) {
        let mut l = Ledger::new(Currency::USD);
        let p = l.add_party("Riley Chen", &[Role::Contractor]).unwrap();
        let fours = ["1234", "5678", "9012"];
        for (tin, which, clear) in steps {
            let last4 = fours[usize::from(which)].to_string();
            let _ = if tin {
                l.set_party_tin(p, (!clear).then_some(TinFacts {
                    tin_type: TinType::Ssn,
                    last4,
                    vault_record: None,
                    purged: None,
                }))
            } else {
                l.set_party_w9(p, (!clear).then_some(W9Facts {
                    received: date(10, 15),
                    entity_type: "individual/sole proprietor".into(),
                    exempt_payee_code: None,
                    fatca_code: None,
                    tin_last4: last4,
                    kept_at: "CPA portal".into(),
                    vault_record: None,
                }))
            };
            let party = l.get_party(p).unwrap();
            let sources: Vec<&str> = party
                .tin
                .iter()
                .map(|t| t.last4.as_str())
                .chain(party.w9.iter().map(|w| w.tin_last4.as_str()))
                .collect();
            match party.tin_last4() {
                None => prop_assert!(sources.is_empty()),
                Some(l4) => prop_assert!(!sources.is_empty() && sources.iter().all(|s| *s == l4)),
            }
        }
    }
}

/// shares::target refuses an unknown entry; target on a draft works (a
/// preview).
#[test]
fn a_target_needs_an_entry() {
    let b = books();
    assert_eq!(
        target(&b.l, 999, &[], &b.money()),
        Err(LedgerError::UnknownEntry(999))
    );
}
