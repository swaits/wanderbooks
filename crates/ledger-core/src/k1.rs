//! The members' K-1 worksheet: each member's capital account for the
//! year, as Schedule K-1 (Form 1065) Item L reports it, with property
//! contributed apart from cash.
//!
//! ```text
//! GOAL: hand the CPA, per member, what the K-1 needs from the books:
//!       beginning capital, contributions (cash and property separately),
//!       their share of income, distributions, ending capital.
//! SOURCE: Form 1065 Schedule K-1 instructions, Item L ("Partner's capital
//!         account analysis", on the partnership's books); B-13;
//!         DECISION (Stephen, 2026-10-15): the CPA package carries a K-1
//!         worksheet per member.
//! INVARIANTS:
//!   1. Every figure but the contribution split is the statement of
//!      changes in members' equity's (reports::members_equity) for the
//!      same span, so the two never disagree.
//!   2. contributions_cash + contributions_property = contributions,
//!      exactly, per member.
//!   3. Property is what an entry crediting a member's capital account
//!      debits to asset accounts that are neither cash (`cash`) nor
//!      receivables: equipment, a vehicle, software, or the unexpired part
//!      of a service contract (AppleCare+, a prepaid subscription) that a
//!      member owned and hands the company, each with its own basis
//!      (§723). The rest of a contribution is cash: deposits, and what a
//!      member paid for the company (an expense booked straight to
//!      capital, or a debt they took on, e.g. Dr 2100 / Cr capital). A
//!      member who pays a vendor for the company's prepaid service is
//!      booked through Due to Shareholders (Dr prepaid / Cr 2100), then
//!      converted (Dr 2100 / Cr capital): cash. A prepaid credited
//!      straight to capital is a contract they handed over: property.
//!      (DECISION, Stephen, 2026-10-15, after the AppleCare+ contributed
//!      with the MacBook read as cash.)
//!   4. Within one entry, the property (at most the members' credits in
//!      it) is shared among the members credited in proportion to their
//!      credits, to the cent, leftover cents by largest remainder
//!      (kernel::apportion, ties to the earlier member): each takes at
//!      most its own credit, and the split depends on the order the
//!      members are given only for a tied leftover cent (never on line
//!      order). Joint property is joint (B-13's
//!      shares, the house convention that joint funds are 50/50).
//!   5. k1_detail's lines explain each row: per member, its cash lines
//!      sum to contributions_cash, its property lines to
//!      contributions_property, and its distribution lines to
//!      −distributions, exactly (the same entries members_equity reads:
//!      posted, in the span, not a period's closing entry, not cancelled
//!      by its reversal in the span).
//!   6. An entry and its reversal both dated in the span cancel
//!      (reports::cancelled_in_span; members_equity INVARIANT 6): neither
//!      gives a line, so a reversed contribution is neither a contribution
//!      nor a distribution, and a reversed distribution neither a
//!      distribution nor a contribution.
//! SCENARIOS:
//!   1. A joint 1,000.00 contribution (Dr 1000 / Cr 3000 500, Cr 3001
//!      500) reversed the next day: each member 0.00 contributed, 0.00
//!      distributed.
//!   2. Members pay a vendor ahead for the company's annual service from
//!      joint funds, through Due to Shareholders (Dr 1300 Prepaid 3,000 /
//!      Cr 2100 3,000), later converted to capital (Dr 2100 3,000 / Cr
//!      3000 1,500, Cr 3001 1,500): 1,500.00 cash each, no property.
//!   3. A joint mixed contribution (Dr 1000 600, Dr 1500 400 / Cr 3000
//!      500, Cr 3001 500): each member 300.00 cash and 200.00 property,
//!      whichever member is listed first.
//!   4. Members hand the company a service contract they already owned,
//!      straight to capital (Dr 1300 Prepaid 3,000 / Cr 3000 1,500, Cr
//!      3001 1,500): 1,500.00 property each, no cash.
//! ```

use crate::reports::{MemberCapital, members_equity};
use crate::{AccountId, AccountType, EntryId, Ledger, LedgerError, PartyId, Side};
use chrono::NaiveDate;
use rust_decimal::Decimal;

/// One member's row on the K-1 worksheet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct K1Row {
    /// The member's name, as in wanderbooks.toml.
    pub name: String,
    /// Share of income, e.g. 0.5.
    pub share: Decimal,
    /// Capital at the start of the span.
    pub beginning: Decimal,
    /// Contributed in cash (or paid for the company) in the span.
    pub contributions_cash: Decimal,
    /// Contributed as property (book value credited) in the span.
    pub contributions_property: Decimal,
    /// Their share of the span's net income (loss).
    pub net_income: Decimal,
    /// Distributions in the span (positive, subtracted).
    pub distributions: Decimal,
    /// Any other change, as on the statement of members' equity.
    pub other: Decimal,
    /// Capital at the end of the span.
    pub ending: Decimal,
}

/// How one entry moved one member's capital ([`K1Line`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum K1Kind {
    /// The cash part of a contribution (INVARIANT 3).
    CashContribution,
    /// The property part of a contribution (INVARIANTS 3-4).
    PropertyContribution,
    /// Taken out: debits to capital, net debits to distributions.
    Distribution,
    /// A period close allocating income into capital. Informational:
    /// the row's `net_income` is the roll-forward's (INVARIANT 1), which
    /// also counts income not yet closed.
    IncomeClose,
}

/// One entry's effect on one member's capital in the span, by kind
/// ([`k1_detail`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct K1Line {
    /// The member's index in the `members` given.
    pub member: usize,
    /// The entry.
    pub entry: EntryId,
    /// What it was.
    pub kind: K1Kind,
    /// Signed: positive raises their capital, negative lowers it.
    pub amount: Decimal,
}

/// The K-1 worksheet with the entries behind it ([`k1_detail`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct K1Detail {
    /// One row per member, in order: [`k1_worksheet`]'s.
    pub rows: Vec<K1Row>,
    /// Every entry dated in the span that moved a member's capital
    /// account (or distributions account), in journal order, a kind per
    /// line (an entry with more than one kind for a member is more than
    /// one line). Per member (INVARIANT 5):
    /// Σ CashContribution = contributions_cash,
    /// Σ PropertyContribution = contributions_property, and
    /// −Σ Distribution = distributions, exactly.
    pub lines: Vec<K1Line>,
}

/// The K-1 worksheet for `start..=end`: one row per member, in order
/// (INVARIANTS 1-4). `cash` are the company's cash accounts; it must not
/// be empty (without it a bank deposit would read as property).
pub fn k1_worksheet(
    ledger: &Ledger,
    start: NaiveDate,
    end: NaiveDate,
    members: &[MemberCapital],
    cash: &[AccountId],
) -> Result<Vec<K1Row>, LedgerError> {
    k1_detail(ledger, start, end, members, cash).map(|d| d.rows)
}

/// [`k1_worksheet`] with the per-entry lines behind each row's
/// contributions and distributions (INVARIANT 5): the one source the
/// viewer's member capital statement reads, so it never disagrees with
/// the CPA package.
pub fn k1_detail(
    ledger: &Ledger,
    start: NaiveDate,
    end: NaiveDate,
    members: &[MemberCapital],
    cash: &[AccountId],
) -> Result<K1Detail, LedgerError> {
    if cash.is_empty() {
        return Err(LedgerError::InvalidMembers(
            "the K-1 worksheet needs the cash accounts".into(),
        ));
    }
    let me = members_equity(ledger, start, end, members)?;
    let receivables = ledger.role_account(&crate::RECEIVABLES);
    let closings = ledger.closing_entry_ids();
    let cancelled = crate::reports::cancelled_in_span(ledger, start, end);
    let property_account = |id: AccountId| {
        !cash.contains(&id)
            && Some(id) != receivables
            && ledger
                .chart()
                .get(id)
                .is_some_and(|a| a.account_type == AccountType::Asset)
    };
    let sum = |e: &crate::JournalEntry, side: Side, account: AccountId| -> Decimal {
        e.lines()
            .iter()
            .filter(|l| l.side == side && l.account == account)
            .map(|l| l.amount.amount)
            .sum()
    };
    let mut lines = Vec::new();
    for e in ledger
        .posted_entries()
        .filter(|e| e.date() >= start && e.date() <= end && !cancelled.contains(&e.id()))
    {
        let line = |member, kind, amount| K1Line {
            member,
            entry: e.id(),
            kind,
            amount,
        };
        if closings.contains(&e.id()) {
            for (i, m) in members.iter().enumerate() {
                let effect: Decimal = std::iter::once(m.capital_account)
                    .chain(m.distribution_account)
                    .map(|a| sum(e, Side::Credit, a) - sum(e, Side::Debit, a))
                    .sum();
                if !effect.is_zero() {
                    lines.push(line(i, K1Kind::IncomeClose, effect));
                }
            }
            continue;
        }
        let pool: Decimal = e
            .lines()
            .iter()
            .filter(|l| l.side == Side::Debit && property_account(l.account))
            .map(|l| l.amount.amount)
            .sum();
        // Contributions are credits to capital (members_equity's).
        let credits: Vec<Decimal> = members
            .iter()
            .map(|m| sum(e, Side::Credit, m.capital_account))
            .collect();
        let takes = share_property(pool, &credits)?;
        for (i, m) in members.iter().enumerate() {
            let (credit, take) = (credits[i], takes[i]);
            if !(credit - take).is_zero() {
                lines.push(line(i, K1Kind::CashContribution, credit - take));
            }
            if !take.is_zero() {
                lines.push(line(i, K1Kind::PropertyContribution, take));
            }
            // Distributions: debits to capital, net debits to the
            // distributions account (members_equity's).
            let taken = sum(e, Side::Debit, m.capital_account)
                + m.distribution_account.map_or(Decimal::ZERO, |d| {
                    sum(e, Side::Debit, d) - sum(e, Side::Credit, d)
                });
            if !taken.is_zero() {
                lines.push(line(i, K1Kind::Distribution, -taken));
            }
        }
    }
    let total = |i: usize, k: K1Kind| -> Decimal {
        lines
            .iter()
            .filter(|l| l.member == i && l.kind == k)
            .map(|l| l.amount)
            .sum()
    };
    let rows = me
        .members
        .into_iter()
        .zip(members)
        .enumerate()
        .map(|(i, (col, m))| {
            // Each entry's property is at most that member's credit in it,
            // and contributions are those credits' sum: property never
            // exceeds contributions, and is never negative.
            let prop = total(i, K1Kind::PropertyContribution);
            K1Row {
                name: col.name,
                share: m.share,
                beginning: col.beginning,
                contributions_cash: col.contributions - prop,
                contributions_property: prop,
                net_income: col.net_income,
                distributions: col.distributions,
                other: col.other,
                ending: col.ending,
            }
        })
        .collect();
    Ok(K1Detail { rows, lines })
}

/// One entry's property shared among the members' credits in it
/// (INVARIANT 4): at most their total credit, in proportion to each
/// credit, rounded toward zero to the cent, the leftover cents by largest
/// remainder. Each part is at most its own credit.
/// Refused when whole × credit is beyond exact arithmetic (credits near
/// the line limit; P0 review round 3, S1).
fn share_property(pool: Decimal, credits: &[Decimal]) -> Result<Vec<Decimal>, LedgerError> {
    use rust_decimal::RoundingStrategy::ToZero;
    let too_large =
        || LedgerError::TooLargeToCompute("a member's share of contributed property".into());
    let total = credits
        .iter()
        .try_fold(Decimal::ZERO, |t, c| t.checked_add(*c))
        .ok_or_else(too_large)?;
    let whole = pool.min(total);
    if whole <= Decimal::ZERO || total <= Decimal::ZERO {
        return Ok(vec![Decimal::ZERO; credits.len()]);
    }
    // whole × credit is exact; ÷ total is exact whenever the share is a
    // whole number of cents, so truncating never loses a cent it is owed.
    let exact: Vec<Decimal> = credits
        .iter()
        .map(|c| whole.checked_mul(*c).and_then(|x| x.checked_div(total)))
        .collect::<Option<_>>()
        .ok_or_else(too_large)?;
    let mut parts: Vec<Decimal> = exact
        .iter()
        .map(|x| x.round_dp_with_strategy(2, ToZero))
        .collect();
    let remainders: Vec<Decimal> = exact.iter().zip(&parts).map(|(x, p)| x - p).collect();
    crate::kernel::apportion(whole, &mut parts, &remainders, Decimal::new(1, 2))
        .expect("each part is short by under a cent, so the shortfall is under a cent per member");
    Ok(parts)
}

/// The party a member (by name, as in wanderbooks.toml) is in the
/// directory: the party not archived with that name, ignoring case and
/// surrounding space ([`Ledger::party_by_name`]), and only when it has
/// the member role. The same rule as ledger-config's
/// `Company::party_members`, so the K-1 worksheet and the viewer's
/// capital statement always name the same party. A display name never
/// matches, and a party without the member role is never a member's: a
/// vendor whose display name is a member's name must never put its TIN
/// on that member's K-1 (the caller reports the member's TIN as a gap).
pub fn member_party(ledger: &Ledger, name: &str) -> Option<PartyId> {
    ledger
        .party_by_name(name)
        .filter(|p| p.roles.contains(&crate::Role::Member))
        .map(|p| p.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AccountType, Currency, Money};
    use std::str::FromStr;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    /// Hand-worked: A contributes 300 cash and a 700 laptop in one entry
    /// (Dr 1000 300, Dr 1500 700 / Cr 3000 1000); B contributes a 200
    /// receivable-backed promise (Dr 1200 200 / Cr 3001 200, cash by
    /// INVARIANT 3) and pays a 50 expense (Dr 6999 50 / Cr 3001 50).
    #[test]
    fn contributions_split_into_cash_and_property() {
        let mut l = Ledger::new(Currency::USD);
        let add =
            |l: &mut Ledger, code: &str, t| l.add_account(code, code, t, None, false).unwrap();
        let cash = add(&mut l, "1000", AccountType::Asset);
        let ar = add(&mut l, "1200", AccountType::Asset);
        let equip = add(&mut l, "1500", AccountType::Asset);
        let a = add(&mut l, "3000", AccountType::Equity);
        let b = add(&mut l, "3001", AccountType::Equity);
        let exp = add(&mut l, "6999", AccountType::Expense);
        assert_eq!(
            l.role_account(&crate::RECEIVABLES),
            Some(ar),
            "1200 by default"
        );
        let day = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap();
        let mut post = |lines: &[(AccountId, Side, &str)]| {
            let e = l.create_entry(day, "x");
            for (acct, side, amt) in lines {
                l.add_line(e, *acct, *side, Money::new(d(amt), Currency::USD))
                    .unwrap();
            }
            l.post_entry(e).unwrap();
        };
        post(&[
            (cash, Side::Debit, "300"),
            (equip, Side::Debit, "700"),
            (a, Side::Credit, "1000"),
        ]);
        post(&[(ar, Side::Debit, "200"), (b, Side::Credit, "200")]);
        post(&[(exp, Side::Debit, "50"), (b, Side::Credit, "50")]);
        let members = [
            MemberCapital {
                name: "A".into(),
                capital_account: a,
                distribution_account: None,
                share: d("0.5"),
            },
            MemberCapital {
                name: "B".into(),
                capital_account: b,
                distribution_account: None,
                share: d("0.5"),
            },
        ];
        let start = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 12, 31).unwrap();
        let rows = k1_worksheet(&l, start, end, &members, &[cash]).unwrap();
        assert_eq!(
            (rows[0].contributions_cash, rows[0].contributions_property),
            (d("300"), d("700"))
        );
        assert_eq!(
            (rows[1].contributions_cash, rows[1].contributions_property),
            (d("250"), d("0"))
        );
        // Net income -50, split -25 each.
        assert_eq!(rows[0].ending, d("975"));
        assert_eq!(rows[1].ending, d("225"));
        assert!(k1_worksheet(&l, start, end, &members, &[]).is_err());
    }

    /// Hand-worked: A (capital 3000, distributions 3100) contributes 100
    /// cash and 40 of equipment in one entry and takes 30 through 3100; B
    /// takes 10 straight from 3001; revenue 60; Jan closed 50/50 (30
    /// each, A's 3100 zeroed into 3000). A: 140 in (100 cash, 40
    /// property), 30 out, 30 income, ending 140. B: 10 out, 30 income,
    /// ending 20. The lines explain the rows (INVARIANT 5).
    #[test]
    fn detail_lines_explain_each_row() {
        let mut l = Ledger::new(Currency::USD);
        let add =
            |l: &mut Ledger, code: &str, t| l.add_account(code, code, t, None, false).unwrap();
        let cash = add(&mut l, "1000", AccountType::Asset);
        let equip = add(&mut l, "1500", AccountType::Asset);
        let a = add(&mut l, "3000", AccountType::Equity);
        let b = add(&mut l, "3001", AccountType::Equity);
        let da = add(&mut l, "3100", AccountType::Equity);
        let rev = add(&mut l, "4000", AccountType::Revenue);
        let day = |m, d| NaiveDate::from_ymd_opt(2026, m, d).unwrap();
        let mut post = |date, lines: &[(AccountId, Side, &str)]| {
            let e = l.create_entry(date, "x");
            for (acct, side, amt) in lines {
                l.add_line(e, *acct, *side, Money::new(d(amt), Currency::USD))
                    .unwrap();
            }
            l.post_entry(e).unwrap();
            e
        };
        let put_in = post(
            day(1, 2),
            &[
                (cash, Side::Debit, "100"),
                (equip, Side::Debit, "40"),
                (a, Side::Credit, "140"),
            ],
        );
        let a_out = post(
            day(1, 3),
            &[(da, Side::Debit, "30"), (cash, Side::Credit, "30")],
        );
        let b_out = post(
            day(1, 4),
            &[(b, Side::Debit, "10"), (cash, Side::Credit, "10")],
        );
        post(
            day(1, 5),
            &[(cash, Side::Debit, "60"), (rev, Side::Credit, "60")],
        );
        let jan = l.create_period("Jan", day(1, 1), day(1, 31)).unwrap();
        let close = l
            .close_period_to(
                jan,
                &crate::periods::CloseTo::Members(vec![
                    crate::MemberClose {
                        capital: a,
                        share: d("0.5"),
                        distributions: Some(da),
                    },
                    crate::MemberClose {
                        capital: b,
                        share: d("0.5"),
                        distributions: None,
                    },
                ]),
            )
            .unwrap()
            .unwrap();
        let members = [
            MemberCapital {
                name: "A".into(),
                capital_account: a,
                distribution_account: Some(da),
                share: d("0.5"),
            },
            MemberCapital {
                name: "B".into(),
                capital_account: b,
                distribution_account: None,
                share: d("0.5"),
            },
        ];
        let (start, end) = (day(1, 1), day(12, 31));
        let detail = k1_detail(&l, start, end, &members, &[cash]).unwrap();
        assert_eq!(
            detail.rows,
            k1_worksheet(&l, start, end, &members, &[cash]).unwrap()
        );
        let line = |member, entry, kind, amount: &str| K1Line {
            member,
            entry,
            kind,
            amount: d(amount),
        };
        assert_eq!(
            detail.lines,
            vec![
                line(0, put_in, K1Kind::CashContribution, "100"),
                line(0, put_in, K1Kind::PropertyContribution, "40"),
                line(0, a_out, K1Kind::Distribution, "-30"),
                line(1, b_out, K1Kind::Distribution, "-10"),
                line(0, close, K1Kind::IncomeClose, "30"),
                line(1, close, K1Kind::IncomeClose, "30"),
            ]
        );
        let ra = &detail.rows[0];
        assert_eq!(
            (
                ra.beginning,
                ra.contributions_cash,
                ra.contributions_property
            ),
            (d("0"), d("100"), d("40"))
        );
        assert_eq!(
            (ra.net_income, ra.distributions, ra.ending),
            (d("30"), d("30"), d("140"))
        );
        let rb = &detail.rows[1];
        assert_eq!(
            (rb.net_income, rb.distributions, rb.ending),
            (d("30"), d("10"), d("20"))
        );
        for (i, row) in detail.rows.iter().enumerate() {
            let sum = |k| -> Decimal {
                detail
                    .lines
                    .iter()
                    .filter(|x| x.member == i && x.kind == k)
                    .map(|x| x.amount)
                    .sum()
            };
            assert_eq!(sum(K1Kind::CashContribution), row.contributions_cash);
            assert_eq!(
                sum(K1Kind::PropertyContribution),
                row.contributions_property
            );
            assert_eq!(-sum(K1Kind::Distribution), row.distributions);
            assert_eq!(
                row.beginning
                    + row.contributions_cash
                    + row.contributions_property
                    + row.net_income
                    - row.distributions
                    + row.other,
                row.ending
            );
        }
        // February alone: nothing moved; the capital carries over.
        let feb = k1_detail(&l, day(2, 1), day(2, 28), &members, &[cash]).unwrap();
        assert!(feb.lines.is_empty());
        assert_eq!(
            (feb.rows[0].beginning, feb.rows[0].ending),
            (d("140"), d("140"))
        );
    }

    /// Two members A (3000) and B (3001), cash 1000, equipment 1500,
    /// prepaid 1300, and a `post` that returns the entry.
    struct Books {
        l: Ledger,
        cash: AccountId,
        equip: AccountId,
        prepaid: AccountId,
        a: AccountId,
        b: AccountId,
    }

    fn books() -> Books {
        let mut l = Ledger::new(Currency::USD);
        let mut add =
            |code: &str, name: &str, t| l.add_account(code, name, t, None, false).unwrap();
        let cash = add("1000", "Bluevine", AccountType::Asset);
        let prepaid = add("1300", "Prepaid Expenses", AccountType::Asset);
        let equip = add("1500", "Equipment", AccountType::Asset);
        let a = add("3000", "Capital A", AccountType::Equity);
        let b = add("3001", "Capital B", AccountType::Equity);
        Books {
            l,
            cash,
            equip,
            prepaid,
            a,
            b,
        }
    }

    fn post(l: &mut Ledger, m: u32, day: u32, lines: &[(AccountId, Side, &str)]) -> EntryId {
        let e = l.create_entry(NaiveDate::from_ymd_opt(2026, m, day).unwrap(), "x");
        for (acct, side, amt) in lines {
            l.add_line(e, *acct, *side, Money::new(d(amt), Currency::USD))
                .unwrap();
        }
        l.post_entry(e).unwrap();
        e
    }

    fn members(bk: &Books) -> [MemberCapital; 2] {
        [
            MemberCapital {
                name: "A".into(),
                capital_account: bk.a,
                distribution_account: None,
                share: d("0.5"),
            },
            MemberCapital {
                name: "B".into(),
                capital_account: bk.b,
                distribution_account: None,
                share: d("0.5"),
            },
        ]
    }

    /// (cash, property, distributions) per member for 2026.
    fn split(bk: &Books, members: &[MemberCapital]) -> Vec<(Decimal, Decimal, Decimal)> {
        let year = |m, day| NaiveDate::from_ymd_opt(2026, m, day).unwrap();
        let detail = k1_detail(&bk.l, year(1, 1), year(12, 31), members, &[bk.cash]).unwrap();
        detail
            .rows
            .iter()
            .map(|r| {
                (
                    r.contributions_cash,
                    r.contributions_property,
                    r.distributions,
                )
            })
            .collect()
    }

    /// INVARIANT 6 and SCENARIO 1, by hand. Case 1: a joint 1,000 cash
    /// contribution reversed the next day: 0 in, 0 out each (not 500 of
    /// each). Case 2: a 700 laptop contribution reversed: A 0 property, 0
    /// out. Case 3: a real 500 + 500 contribution, then a 100 + 100
    /// distribution reversed: 500 cash each, 0 out (not 600 in, 100 out).
    #[test]
    fn a_reversed_contribution_or_distribution_cancels() {
        let mut bk = books();
        let (cash, equip, a, b) = (bk.cash, bk.equip, bk.a, bk.b);
        let joint = post(
            &mut bk.l,
            1,
            2,
            &[
                (cash, Side::Debit, "1000"),
                (a, Side::Credit, "500"),
                (b, Side::Credit, "500"),
            ],
        );
        let r =
            bk.l.reverse_entry(joint, NaiveDate::from_ymd_opt(2026, 1, 3).unwrap(), "x")
                .unwrap();
        let ms = members(&bk);
        let zero = (d("0"), d("0"), d("0"));
        assert_eq!(split(&bk, &ms), vec![zero, zero]);
        let year = |m, day| NaiveDate::from_ymd_opt(2026, m, day).unwrap();
        let detail = k1_detail(&bk.l, year(1, 1), year(12, 31), &ms, &[cash]).unwrap();
        assert!(
            detail
                .lines
                .iter()
                .all(|l| l.entry != joint && l.entry != r),
            "{:?}",
            detail.lines
        );
        // The statement of members' equity agrees (INVARIANT 1).
        let me = members_equity(&bk.l, year(1, 1), year(12, 31), &ms).unwrap();
        assert_eq!(
            (me.members[0].contributions, me.members[0].distributions),
            (d("0"), d("0"))
        );
        // Only the reversal in the span: by side, a distribution.
        let feb_on = members_equity(&bk.l, year(1, 3), year(12, 31), &ms).unwrap();
        assert_eq!(feb_on.members[0].distributions, d("500"));

        let laptop = post(
            &mut bk.l,
            2,
            1,
            &[(equip, Side::Debit, "700"), (a, Side::Credit, "700")],
        );
        bk.l.reverse_entry(laptop, year(2, 2), "x").unwrap();
        assert_eq!(split(&bk, &ms), vec![zero, zero]);

        post(
            &mut bk.l,
            3,
            1,
            &[
                (cash, Side::Debit, "1000"),
                (a, Side::Credit, "500"),
                (b, Side::Credit, "500"),
            ],
        );
        let out = post(
            &mut bk.l,
            3,
            2,
            &[
                (a, Side::Debit, "100"),
                (b, Side::Debit, "100"),
                (cash, Side::Credit, "200"),
            ],
        );
        let undo = bk.l.reverse_entry(out, year(3, 3), "x").unwrap();
        let five = (d("500"), d("0"), d("0"));
        assert_eq!(split(&bk, &ms), vec![five, five]);
        // Reversing the reversal reinstates the distribution.
        bk.l.reverse_entry(undo, year(3, 4), "x").unwrap();
        let back = (d("500"), d("0"), d("100"));
        assert_eq!(split(&bk, &ms), vec![back, back]);
    }

    /// INVARIANT 3, by hand: a contract the members hand over (Dr 1300
    /// Prepaid 25 / Cr 3000 12.50, Cr 3001 12.50, the AppleCare+ with the
    /// MacBook) is property, 12.50 each. Members paying a 3,000 annual
    /// service for the company go through 2100: Dr 1300 / Cr 2100 3,000,
    /// then Dr 2100 / Cr 3000 1,500, Cr 3001 1,500: 1,500 cash each.
    #[test]
    fn a_contract_handed_over_is_property_a_service_paid_for_is_cash() {
        let mut bk = books();
        let (prepaid, a, b) = (bk.prepaid, bk.a, bk.b);
        post(
            &mut bk.l,
            1,
            5,
            &[
                (prepaid, Side::Debit, "25"),
                (a, Side::Credit, "12.50"),
                (b, Side::Credit, "12.50"),
            ],
        );
        let ms = members(&bk);
        let handed = (d("0"), d("12.50"), d("0"));
        assert_eq!(split(&bk, &ms), vec![handed, handed]);
        let due =
            bk.l.add_account(
                "2100",
                "Due to Members",
                AccountType::Liability,
                None,
                false,
            )
            .unwrap();
        post(
            &mut bk.l,
            2,
            1,
            &[(prepaid, Side::Debit, "3000"), (due, Side::Credit, "3000")],
        );
        post(
            &mut bk.l,
            2,
            2,
            &[
                (due, Side::Debit, "3000"),
                (a, Side::Credit, "1500"),
                (b, Side::Credit, "1500"),
            ],
        );
        let each = (d("1500"), d("12.50"), d("0"));
        assert_eq!(split(&bk, &ms), vec![each, each]);
    }

    /// SCENARIOS 2 and 4 exactly as the module doc states them: 3,000 of
    /// prepaid service through 2100 is 1,500.00 cash each; the same 3,000
    /// straight to capital is 1,500.00 property each.
    #[test]
    fn scenarios_2_and_4_prepaid_through_2100_is_cash_straight_to_capital_is_property() {
        let mut bk = books();
        let (prepaid, a, b) = (bk.prepaid, bk.a, bk.b);
        let due =
            bk.l.add_account(
                "2100",
                "Due to Shareholders",
                AccountType::Liability,
                None,
                false,
            )
            .unwrap();
        post(
            &mut bk.l,
            1,
            5,
            &[(prepaid, Side::Debit, "3000"), (due, Side::Credit, "3000")],
        );
        post(
            &mut bk.l,
            1,
            6,
            &[
                (due, Side::Debit, "3000"),
                (a, Side::Credit, "1500"),
                (b, Side::Credit, "1500"),
            ],
        );
        let ms = members(&bk);
        let cash = (d("1500"), d("0"), d("0"));
        assert_eq!(split(&bk, &ms), vec![cash, cash]);
        post(
            &mut bk.l,
            2,
            1,
            &[
                (prepaid, Side::Debit, "3000"),
                (a, Side::Credit, "1500"),
                (b, Side::Credit, "1500"),
            ],
        );
        let both = (d("1500"), d("1500"), d("0"));
        assert_eq!(split(&bk, &ms), vec![both, both]);
    }

    /// INVARIANT 4 and SCENARIO 3, by hand: Dr 1000 600, Dr 1500 400 /
    /// Cr 3000 500, Cr 3001 500. Property 400 split by credit (500:500):
    /// 200 each; cash 300 each, whichever member is listed first. Unequal
    /// credits (Cr 3000 750, Cr 3001 250, property 401): 401 × 0.75 =
    /// 300.75, 401 × 0.25 = 100.25, exact. Odd cents: property 0.01 on
    /// equal credits goes to the earlier member on a tie.
    #[test]
    fn joint_property_is_shared_by_credit() {
        let mut bk = books();
        let (cash, equip, a, b) = (bk.cash, bk.equip, bk.a, bk.b);
        post(
            &mut bk.l,
            1,
            5,
            &[
                (cash, Side::Debit, "600"),
                (equip, Side::Debit, "400"),
                (a, Side::Credit, "500"),
                (b, Side::Credit, "500"),
            ],
        );
        let ms = members(&bk);
        let each = (d("300"), d("200"), d("0"));
        assert_eq!(split(&bk, &ms), vec![each, each]);
        let mut flipped = ms.clone();
        flipped.reverse();
        assert_eq!(split(&bk, &flipped), vec![each, each]);
        assert_eq!(
            share_property(d("401"), &[d("750"), d("250")]).unwrap(),
            vec![d("300.75"), d("100.25")]
        );
        assert_eq!(
            share_property(d("0.01"), &[d("5"), d("5")]).unwrap(),
            vec![d("0.01"), d("0.00")]
        );
        // More property than credit (part financed): at most the credits.
        assert_eq!(
            share_property(d("900"), &[d("300"), d("100")]).unwrap(),
            vec![d("300"), d("100")]
        );
        assert_eq!(
            share_property(d("50"), &[d("0"), d("0")]).unwrap(),
            vec![d("0"), d("0")]
        );
        // Credits near the line limit: whole x credit is past exact
        // arithmetic, so the split is refused, never a panic (P0 review
        // round 3, S1); at a quarter of it, it still splits exactly.
        let near = d("900000000000000");
        assert!(matches!(
            share_property(near, &[near, near]),
            Err(LedgerError::TooLargeToCompute(_))
        ));
        assert!(matches!(
            share_property(near, &[Decimal::MAX, Decimal::MAX]),
            Err(LedgerError::TooLargeToCompute(_))
        ));
        let quarter = d("250000000000");
        assert_eq!(
            share_property(quarter, &[quarter, quarter]).unwrap(),
            vec![d("125000000000"), d("125000000000")]
        );
    }

    /// member_party doc: the party of that name with the member role only.
    /// A contractor whose display name is the member's name, or a party
    /// of the name without the member role, is never the member's.
    #[test]
    fn member_party_needs_the_member_role_and_the_name() {
        let mut l = Ledger::new(Currency::USD);
        let ke = l
            .add_party("KE Consulting LLC", &[crate::Role::Contractor])
            .unwrap();
        l.update_party(
            ke,
            crate::PartyField::DisplayName(Some("Kat Example".into())),
        )
        .unwrap();
        assert_eq!(member_party(&l, "Kat Example"), None);
        let plain = l.add_party("Kat Example", &[]).unwrap();
        assert_eq!(member_party(&l, "kat example "), None, "no member role");
        l.set_party_role(plain, crate::Role::Member, true).unwrap();
        assert_eq!(member_party(&l, "kat example "), Some(plain));
    }
}
