//! The odd cent (P3 defect 17; rules.md B-13; D-10, D-38): members closes
//! split earnings cumulatively, each member's share of everything closed
//! to these members and shares so far less what earlier closes gave them,
//! and the statement of members' equity, the K-1 worksheet and the viewer
//! split earnings not yet closed the same running way.
//!
//! Every expected figure is worked by hand in the comment beside it, from
//! the rule: a close gives member i allocate(C + net)_i − given_i, where C
//! is what the count's earlier closes closed, given_i what they gave i,
//! and allocate splits by largest remainder to the currency's minor unit,
//! ties to the member listed first (DECISION, owner 2026-10-13).

use chrono::{Datelike, NaiveDate};
use ledger_core::{
    AccountId, AccountType, CloseTo, Currency, EntryId, Ledger, LedgerError, MemberCapital,
    MemberClose, MemberRule, Money, PeriodId, Side, members_equity,
};
use proptest::prelude::*;
use rust_decimal::Decimal;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn d(m: u32, dd: u32) -> NaiveDate {
    day(2026, m, dd)
}

fn dec(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn decs(v: &[&str]) -> Vec<Decimal> {
    v.iter().map(|s| dec(s)).collect()
}

fn month_end(y: i32, m: u32) -> NaiveDate {
    if m == 12 {
        day(y, 12, 31)
    } else {
        day(y, m + 1, 1).pred_opt().unwrap()
    }
}

/// A partnership: cash 1000; member i's capital 300i and distributions
/// 310i; other equity 3500; sales 4000; fees 6000.
struct Co {
    l: Ledger,
    currency: Currency,
    cash: AccountId,
    caps: Vec<AccountId>,
    dists: Vec<AccountId>,
    other_equity: AccountId,
    sales: AccountId,
    fees: AccountId,
}

fn company(currency: Currency, members: usize) -> Co {
    let mut l = Ledger::new(currency);
    let mut add = |code: &str, t| l.add_account(code, code, t, None, false).unwrap();
    let cash = add("1000", AccountType::Asset);
    let caps = (0..members)
        .map(|i| add(&format!("300{i}"), AccountType::Equity))
        .collect();
    let dists = (0..members)
        .map(|i| add(&format!("310{i}"), AccountType::Equity))
        .collect();
    let other_equity = add("3500", AccountType::Equity);
    let sales = add("4000", AccountType::Revenue);
    let fees = add("6000", AccountType::Expense);
    Co {
        l,
        currency,
        cash,
        caps,
        dists,
        other_equity,
        sales,
        fees,
    }
}

impl Co {
    fn post(&mut self, date: NaiveDate, dr: AccountId, cr: AccountId, amount: Decimal) {
        let e = self.l.create_entry(date, "x");
        let m = Money::new(amount, self.currency);
        self.l.add_line(e, dr, Side::Debit, m).unwrap();
        self.l.add_line(e, cr, Side::Credit, m).unwrap();
        self.l.post_entry(e).unwrap();
    }

    /// Earn `net` on `date`: a sale for a gain, a fee for a loss, nothing
    /// for zero.
    fn earn(&mut self, date: NaiveDate, net: Decimal) {
        if net > Decimal::ZERO {
            self.post(date, self.cash, self.sales, net);
        } else if net < Decimal::ZERO {
            self.post(date, self.fees, self.cash, -net);
        }
    }

    /// Create month `m` of `y` as a period and close it to `to`.
    fn close(
        &mut self,
        y: i32,
        m: u32,
        to: &CloseTo,
    ) -> (PeriodId, Result<Option<EntryId>, LedgerError>) {
        let p = self
            .l
            .create_period(&format!("{y}-{m:02}"), day(y, m, 1), month_end(y, m))
            .unwrap();
        (p, self.l.close_period_to(p, to))
    }

    /// Earn `net` on the 10th of month `m` of 2026 and close the month.
    fn month(&mut self, m: u32, net: &str, to: &CloseTo) -> PeriodId {
        self.earn(d(m, 10), dec(net));
        let (p, r) = self.close(2026, m, to);
        r.unwrap();
        p
    }

    /// The members in this order (indexes into `caps`), member i at
    /// `shares[i]`, without distribution accounts.
    fn members(&self, order: &[usize], shares: &[&str]) -> Vec<MemberClose> {
        order
            .iter()
            .map(|&i| MemberClose {
                capital: self.caps[i],
                share: dec(shares[i]),
                distributions: None,
            })
            .collect()
    }

    fn capitals(&self) -> Vec<Decimal> {
        self.caps
            .iter()
            .map(|&c| self.l.account_balance(c).unwrap())
            .collect()
    }

    /// The members as the statement of members' equity takes them, each
    /// with their distributions account.
    fn member_capitals(&self, shares: &[&str]) -> Vec<MemberCapital> {
        shares
            .iter()
            .enumerate()
            .map(|(i, s)| MemberCapital {
                name: format!("M{i}"),
                capital_account: self.caps[i],
                distribution_account: Some(self.dists[i]),
                share: dec(s),
            })
            .collect()
    }

    /// Each member's capital on the statement of members' equity as of
    /// `as_of` (its ending, the statement from Jan 1, 2026).
    fn statement(&self, shares: &[&str], as_of: NaiveDate) -> Vec<Decimal> {
        self.statement_listing(&(0..shares.len()).collect::<Vec<_>>(), shares, as_of)
    }

    /// [`Co::statement`] listing the members in this order (indexes into
    /// `caps`), as settings that list them so would.
    fn statement_listing(
        &self,
        order: &[usize],
        shares: &[&str],
        as_of: NaiveDate,
    ) -> Vec<Decimal> {
        let all = self.member_capitals(shares);
        let members: Vec<MemberCapital> = order.iter().map(|&i| all[i].clone()).collect();
        members_equity(&self.l, d(1, 1), as_of, &members)
            .unwrap()
            .members
            .iter()
            .map(|m| m.ending)
            .collect()
    }

    fn lines(&self, id: EntryId) -> Vec<(String, Side, Decimal)> {
        self.l
            .get_entry(id)
            .unwrap()
            .lines()
            .iter()
            .map(|x| {
                let code = self.l.chart().get(x.account).unwrap().code.clone();
                (code, x.side, x.amount.amount)
            })
            .collect()
    }
}

fn line(code: &str, side: Side, amount: &str) -> (String, Side, Decimal) {
    (code.to_string(), side, dec(amount))
}

const HALF: [&str; 2] = ["0.5", "0.5"];
const THIRDS: [&str; 3] = ["0.3333", "0.3333", "0.3334"];

// ---------------------------------------------------------------------------
// The regression (audit F52, 2026-10-16, anonymized)
// ---------------------------------------------------------------------------

/// Regression (P3 defect 17; audit F52's −399.23, anonymized): members A
/// and B at half each. January's net loss of 399.23 closes to them: exact
/// −199.615 each, so A −199.62, B −199.61 (the tie to the member listed
/// first). February's 10.00 loss closes to retained earnings. March's
/// 399.23 loss closes to them again. The run rule (2026-10-13 to P3)
/// started over at March, handing A the tied cent a second time: A
/// −399.24, B −399.22 for an exact −399.23 each. Now February's close is
/// skipped and March continues January's count: the running −798.46 splits
/// −399.23 each, less January's −199.62 / −199.61, so March gives A
/// −199.61 and B −199.62. The March entry: Cr 6000 399.23, Dr 3000 199.61,
/// Dr 3001 199.62.
#[test]
fn the_audit_odd_cent_never_piles_up_across_closes() {
    type Rule = fn(Vec<MemberClose>) -> CloseTo;
    let (run, cumulative): (Rule, Rule) = (CloseTo::MembersRun, CloseTo::Members);
    for (rule, a, b) in [
        (run, "-399.24", "-399.22"),
        (cumulative, "-399.23", "-399.23"),
    ] {
        let mut co = company(Currency::USD, 2);
        let to = rule(co.members(&[0, 1], &HALF));
        let jan = co.month(1, "-399.23", &to);
        let jan_entry = co.l.get_period(jan).unwrap().closing_entry.unwrap();
        assert_eq!(
            co.lines(jan_entry),
            [
                line("6000", Side::Credit, "399.23"),
                line("3000", Side::Debit, "199.62"),
                line("3001", Side::Debit, "199.61"),
            ]
        );
        co.month(2, "-10.00", &CloseTo::RetainedEarnings { sweep: vec![] });
        let mar = co.month(3, "-399.23", &to);
        assert_eq!(co.capitals(), decs(&[a, b]));
        let retained = co.l.chart().find_by_code("3999").unwrap().id;
        assert_eq!(co.l.account_balance(retained).unwrap(), dec("-10.00"));
        if a == "-399.23" {
            let mar_entry = co.l.get_period(mar).unwrap().closing_entry.unwrap();
            assert_eq!(
                co.lines(mar_entry),
                [
                    line("6000", Side::Credit, "399.23"),
                    line("3000", Side::Debit, "199.61"),
                    line("3001", Side::Debit, "199.62"),
                ]
            );
            let (since, split) = co.l.member_close_split(mar).unwrap();
            assert_eq!(since, 1, "the count starts at January's close");
            let rows: Vec<_> = split.iter().map(|m| (m.part, m.running, m.exact)).collect();
            assert_eq!(
                rows,
                [
                    (dec("-199.61"), dec("-399.23"), dec("-399.23")),
                    (dec("-199.62"), dec("-399.23"), dec("-399.23")),
                ]
            );
        }
    }
}

// ---------------------------------------------------------------------------
// From inception (D-10): earlier rules trued up by the next close that posts
// ---------------------------------------------------------------------------

/// Three cents, 50/50, closed one at a time under the per-close rule:
/// each tie to A, so A 0.03, B 0.00.
fn three_per_close_cents(co: &mut Co) {
    let legacy = CloseTo::MembersPerClose(co.members(&[0, 1], &HALF));
    for m in 1..=3 {
        co.month(m, "0.01", &legacy);
    }
    assert_eq!(co.capitals(), decs(&["0.03", "0.00"]));
}

/// D-10 (from inception; P3 brief R2): after three per-close cent months
/// (A 0.03, B 0.00), April's cent closes under the cumulative rule from
/// the first close: the running 0.04 splits 0.02 / 0.02, less 0.03 / 0.00
/// given, so A −0.01 and B +0.02. The entry: Dr 4000 0.01, Dr 3000 0.01,
/// Cr 3001 0.02. Both stand at their exact 0.02.
#[test]
fn members_closes_true_up_from_inception() {
    let mut co = company(Currency::USD, 2);
    three_per_close_cents(&mut co);
    let apr = co.month(4, "0.01", &CloseTo::Members(co.members(&[0, 1], &HALF)));
    let entry = co.l.get_period(apr).unwrap().closing_entry.unwrap();
    assert_eq!(
        co.lines(entry),
        [
            line("4000", Side::Debit, "0.01"),
            line("3000", Side::Debit, "0.01"),
            line("3001", Side::Credit, "0.02"),
        ]
    );
    assert_eq!(co.capitals(), decs(&["0.02", "0.02"]));
    assert_eq!(co.l.member_close_split(apr).unwrap().0, 1);
}

/// INVARIANT 6 and rule 4 (a true-up rides only on a close that posts):
/// after the three per-close cents (A 0.03, B 0.00), April has no
/// activity, so its close posts nothing and credits no one; the count
/// still includes it (its net is zero). May's cent posts and carries the
/// true-up: the running 0.04 splits 0.02 / 0.02, so A −0.01, B +0.02.
#[test]
fn a_close_that_posts_nothing_carries_no_true_up() {
    let mut co = company(Currency::USD, 2);
    three_per_close_cents(&mut co);
    let to = CloseTo::Members(co.members(&[0, 1], &HALF));
    let (apr, posted) = co.close(2026, 4, &to);
    assert_eq!(posted.unwrap(), None);
    let record =
        co.l.get_period(apr)
            .unwrap()
            .members_closed
            .clone()
            .unwrap();
    assert_eq!(record.parts, decs(&["0", "0"]));
    assert_eq!(record.rule, MemberRule::Cumulative { since_seq: 1 });
    assert_eq!(co.capitals(), decs(&["0.03", "0.00"]));
    co.month(5, "0.01", &to);
    assert_eq!(co.capitals(), decs(&["0.02", "0.02"]));
}

/// P3 review R-3: a close whose only lines would be a distribution sweep
/// still posts, so it carries the pending true-up. After the three
/// per-close cents (A 0.03, B 0.00), A takes 5.00 through their
/// distributions account in April and nothing is earned: the running 0.03
/// splits 0.02 / 0.01, so A −0.01 and B +0.01, beside the sweep. The
/// entry: Dr 3000 0.01, Cr 3001 0.01, Cr 3100 5.00, Dr 3000 5.00.
#[test]
fn a_true_up_rides_on_a_distribution_sweep() {
    let mut co = company(Currency::USD, 2);
    three_per_close_cents(&mut co);
    co.post(d(4, 20), co.dists[0], co.cash, dec("5.00"));
    let mut members = co.members(&[0, 1], &HALF);
    members[0].distributions = Some(co.dists[0]);
    let (apr, posted) = co.close(2026, 4, &CloseTo::Members(members));
    let entry = posted.unwrap().unwrap();
    assert_eq!(
        co.lines(entry),
        [
            line("3000", Side::Debit, "0.01"),
            line("3001", Side::Credit, "0.01"),
            line("3100", Side::Credit, "5.00"),
            line("3000", Side::Debit, "5.00"),
        ]
    );
    assert_eq!(co.capitals(), decs(&["-4.98", "0.01"]));
    let (_, split) = co.l.member_close_split(apr).unwrap();
    let running: Vec<_> = split.iter().map(|m| m.running).collect();
    assert_eq!(running, decs(&["0.02", "0.01"]));
}

// ---------------------------------------------------------------------------
// Where a count starts: a set of members and shares (B-13; §706(d))
// ---------------------------------------------------------------------------

/// A reorder of the members is not a change: [A, B] closes January's cent
/// (A 0.01); [B, A] closes February's and March's. February: the running
/// 0.02 splits 0.01 / 0.01, less A 0.01 / B 0.00, so B +0.01. March: 0.03
/// is 0.015 each, and the tied cent goes to the member the count's first
/// close listed first, A, whatever order March lists them in: 0.02 /
/// 0.01, less 0.01 / 0.01, so A +0.01. A 0.02, B 0.01; every close counts
/// from close 1. The statement of members' equity, listing them [A, B]
/// or [B, A], shows the same before and after March closes.
#[test]
fn reordering_members_keeps_counting_from_the_first_close() {
    let mut co = company(Currency::USD, 2);
    co.month(1, "0.01", &CloseTo::Members(co.members(&[0, 1], &HALF)));
    let reversed = CloseTo::Members(co.members(&[1, 0], &HALF));
    let feb = co.month(2, "0.01", &reversed);
    assert_eq!(co.capitals(), decs(&["0.01", "0.01"]));
    co.earn(d(3, 10), dec("0.01"));
    let listed_backwards = |co: &Co| {
        let mut members = co.member_capitals(&HALF);
        members.reverse();
        let st = members_equity(&co.l, d(1, 1), d(3, 31), &members).unwrap();
        st.members.iter().map(|m| m.ending).collect::<Vec<_>>()
    };
    assert_eq!(co.statement(&HALF, d(3, 31)), decs(&["0.02", "0.01"]));
    assert_eq!(listed_backwards(&co), decs(&["0.01", "0.02"]));
    let (mar, r) = co.close(2026, 3, &reversed);
    r.unwrap();
    assert_eq!(co.capitals(), decs(&["0.02", "0.01"]));
    assert_eq!(co.statement(&HALF, d(3, 31)), decs(&["0.02", "0.01"]));
    assert_eq!(listed_backwards(&co), decs(&["0.01", "0.02"]));
    for p in [feb, mar] {
        assert_eq!(co.l.member_close_split(p).unwrap().0, 1);
    }
}

/// A change of shares starts a new count (26 U.S.C. §706(d)(1); Treas.
/// Reg. §1.706-4(a)(3)(vii)): what was closed at the old shares is never
/// split again at the new ones. January's cent at 50/50 goes to A (close
/// 1). February at 60/40 starts a count at close 2: 0.006 / 0.004, the
/// cent to A's larger remainder (re-splitting January's cent too would
/// have given B February's). March continues from close 2: 0.02 at 60/40
/// is 0.012 / 0.008, so 0.01 / 0.01, less 0.01 / 0.00: B +0.01. A return
/// to 50/50 in April is a new count again (from close 4), not January's:
/// April's cent to A. A 0.03, B 0.01.
#[test]
fn a_share_change_starts_a_new_count() {
    let mut co = company(Currency::USD, 2);
    let jan = co.month(1, "0.01", &CloseTo::Members(co.members(&[0, 1], &HALF)));
    let sixty = CloseTo::Members(co.members(&[0, 1], &["0.6", "0.4"]));
    let feb = co.month(2, "0.01", &sixty);
    assert_eq!(co.capitals(), decs(&["0.02", "0.00"]));
    let mar = co.month(3, "0.01", &sixty);
    assert_eq!(co.capitals(), decs(&["0.02", "0.01"]));
    let apr = co.month(4, "0.01", &CloseTo::Members(co.members(&[0, 1], &HALF)));
    assert_eq!(co.capitals(), decs(&["0.03", "0.01"]));
    let since: Vec<u64> = [jan, feb, mar, apr]
        .iter()
        .map(|&p| co.l.member_close_split(p).unwrap().0)
        .collect();
    assert_eq!(since, [1, 2, 2, 4]);
}

/// P3 review R-8: a member joining mid-year starts a new count, so the
/// under-a-cent bound holds per set of members and shares, and a member's
/// year can be off by up to a cent per count. January to June, A and B at
/// 50/50 close 0.01, 0.02, 0.01, 0.01, 0.01, 0.01: the count's 0.07 ends
/// A 0.04 / B 0.03 (0.035 each, the tie to A). From July C joins, all at
/// 33.33% / 33.33% / 33.34%, closing 1.00 then 0.01 a month to December:
/// the count's 1.05 is exactly 0.349965 / 0.349965 / 0.35007; floors 0.34 /
/// 0.34 / 0.35 leave two cents for the largest remainders (A's and B's
/// 0.009965): 0.35 each. A 0.39, B 0.38, C 0.35.
#[test]
fn a_member_joining_mid_year_starts_a_new_count() {
    let mut co = company(Currency::USD, 3);
    let two = CloseTo::Members(co.members(&[0, 1], &HALF));
    let three = CloseTo::Members(co.members(&[0, 1, 2], &THIRDS));
    let mut periods = Vec::new();
    for (m, net) in ["0.01", "0.02", "0.01", "0.01", "0.01", "0.01"]
        .iter()
        .enumerate()
    {
        periods.push(co.month(m as u32 + 1, net, &two));
    }
    for (m, net) in ["1.00", "0.01", "0.01", "0.01", "0.01", "0.01"]
        .iter()
        .enumerate()
    {
        periods.push(co.month(m as u32 + 7, net, &three));
    }
    assert_eq!(co.capitals(), decs(&["0.39", "0.38", "0.35"]));
    let since: Vec<u64> = periods
        .iter()
        .map(|&p| co.l.member_close_split(p).unwrap().0)
        .collect();
    assert_eq!(since, [1, 1, 1, 1, 1, 1, 7, 7, 7, 7, 7, 7]);
    // Within each count, every running total is under a cent from exact.
    for &p in &periods {
        for m in co.l.member_close_split(p).unwrap().1 {
            assert!((m.running - m.exact).abs() < dec("0.01"), "{m:?}");
        }
    }
    // Over the year, A is under two cents (one per count) from its exact
    // 0.035 + 0.349965 = 0.384965.
    assert!((co.capitals()[0] - dec("0.384965")).abs() < dec("0.02"));
}

/// Scenario 6 (a journaled start must be the derived one): with January
/// closed to the members (close 1), February journaled as counting from
/// close 2 is refused and changes nothing; from close 0, refused as
/// malformed; from close 1, closed.
#[test]
fn a_journaled_start_other_than_the_derived_one_is_refused() {
    let mut co = company(Currency::USD, 2);
    co.month(1, "0.01", &CloseTo::Members(co.members(&[0, 1], &HALF)));
    co.earn(d(2, 10), dec("0.01"));
    let feb = co.l.create_period("2026-02", d(2, 1), d(2, 28)).unwrap();
    let members = co.members(&[0, 1], &HALF);
    let before = co.l.clone();
    for since_seq in [2, 0] {
        let to = CloseTo::MembersCumulative {
            members: members.clone(),
            since_seq,
        };
        assert!(
            matches!(
                co.l.preview_close_period_to(feb, &to),
                Err(LedgerError::InvalidMembers(_))
            ),
            "{since_seq}"
        );
        assert!(
            matches!(
                co.l.close_period_to(feb, &to),
                Err(LedgerError::InvalidMembers(_))
            ),
            "{since_seq}"
        );
        assert_eq!(co.l.list_periods(), before.list_periods());
        assert_eq!(co.l.next_entry_id(), before.next_entry_id());
    }
    let to = CloseTo::MembersCumulative {
        members,
        since_seq: 1,
    };
    co.l.close_period_to(feb, &to).unwrap();
    assert_eq!(co.capitals(), decs(&["0.01", "0.01"]));
}

// ---------------------------------------------------------------------------
// The minor unit (B-3): whole yen, thousandths of a dinar
// ---------------------------------------------------------------------------

/// P3 brief item 6: a yen book closes to its members in whole yen. 101
/// yen at 0.5 / 0.25 / 0.25 is exactly 50.5 / 25.25 / 25.25; floors 50 /
/// 25 / 25 leave one yen for the largest remainder (0.5): 51 / 25 / 25.
/// February's yen: the running 102 is 51 / 25.5 / 25.5, floors 51 / 25 /
/// 25, the tied yen to B: B +1. The run rule split to cents (50.50 …) and
/// the post refused it (FinerThanMinorUnit). The statement splits in whole
/// yen too, its "other" included (P3 review R-10.2): one yen put into
/// other equity in March is 0.5 / 0.25 / 0.25 exactly, so A's.
#[test]
fn a_yen_book_closes_to_members_in_whole_yen() {
    let shares = ["0.5", "0.25", "0.25"];
    let mut legacy = company(Currency::JPY, 3);
    legacy.earn(d(1, 10), dec("101"));
    let run = CloseTo::MembersRun(legacy.members(&[0, 1, 2], &shares));
    assert!(matches!(
        legacy.close(2026, 1, &run).1,
        Err(LedgerError::FinerThanMinorUnit { .. })
    ));

    let mut co = company(Currency::JPY, 3);
    let to = CloseTo::Members(co.members(&[0, 1, 2], &shares));
    co.earn(d(1, 10), dec("101"));
    assert_eq!(co.statement(&shares, d(1, 31)), decs(&["51", "25", "25"]));
    let (jan, r) = co.close(2026, 1, &to);
    r.unwrap();
    let entry = co.l.get_period(jan).unwrap().closing_entry.unwrap();
    assert_eq!(
        co.lines(entry),
        [
            line("4000", Side::Debit, "101"),
            line("3000", Side::Credit, "51"),
            line("3001", Side::Credit, "25"),
            line("3002", Side::Credit, "25"),
        ]
    );
    co.earn(d(2, 10), dec("1"));
    assert_eq!(co.statement(&shares, d(2, 28)), decs(&["51", "26", "25"]));
    co.close(2026, 2, &to).1.unwrap();
    assert_eq!(co.capitals(), decs(&["51", "26", "25"]));
    assert_eq!(co.statement(&shares, d(2, 28)), decs(&["51", "26", "25"]));
    // One yen into other equity: unexplained by flows or income, so
    // "other", split in whole yen.
    co.post(d(3, 5), co.cash, co.other_equity, dec("1"));
    let st = members_equity(&co.l, d(3, 1), d(3, 31), &co.member_capitals(&shares)).unwrap();
    let other: Vec<_> = st.members.iter().map(|m| m.other).collect();
    assert_eq!(other, decs(&["1", "0", "0"]));
}

/// A dinar book splits to thousandths: 0.101 at 50/50 is 0.0505 each, the
/// tied thousandth to A (0.051 / 0.050); February's 0.001 makes the
/// running 0.102, 0.051 each: B +0.001.
#[test]
fn a_dinar_book_closes_to_members_in_thousandths() {
    let kwd = Currency::new("KWD").unwrap();
    let mut co = company(kwd, 2);
    let to = CloseTo::Members(co.members(&[0, 1], &HALF));
    co.month(1, "0.101", &to);
    assert_eq!(co.capitals(), decs(&["0.051", "0.050"]));
    co.month(2, "0.001", &to);
    assert_eq!(co.capitals(), decs(&["0.051", "0.051"]));
}

/// A currency ISO 4217 gives no minor unit (gold, XAU) splits to the finest
/// amount a line may carry (kernel::MAX_LINE_SCALE places), so every amount
/// is a whole number of the unit it splits to and no member is ever one
/// from exact (B-13's precondition). 0.019 at 33.33% / 33.33% / 33.34% is
/// exactly 0.0063327 / 0.0063327 / 0.0063346: each member takes theirs.
/// Split to cents instead, the floors (0.00 each) left 0.019, a cent and
/// 0.009 of one, all of it to the largest remainder: C 0.019, A and B
/// nothing, C 1.27 cents from exact.
#[test]
fn a_gold_book_splits_to_the_finest_unit_a_line_carries() {
    let xau = Currency::new("XAU").unwrap();
    assert_eq!(xau.minor_units(), None);
    assert_eq!(xau.allocation_places(), ledger_core::kernel::MAX_LINE_SCALE);
    let mut co = company(xau, 3);
    let to = CloseTo::Members(co.members(&[0, 1, 2], &THIRDS));
    co.earn(d(1, 10), dec("0.019"));
    let exact = decs(&["0.0063327", "0.0063327", "0.0063346"]);
    assert_eq!(co.statement(&THIRDS, d(1, 31)), exact);
    co.close(2026, 1, &to).1.unwrap();
    assert_eq!(co.capitals(), exact);
    assert_eq!(co.statement(&THIRDS, d(1, 31)), exact);
}

/// K-1 INVARIANT 4 at the minor unit: property contributed jointly in a
/// yen book is shared in whole yen. Dr Equipment 101, Dr Cash 1 / Cr A's
/// capital 51, Cr B's 51: the 101 of property is 50.5 each by their
/// credits; floors 50 / 50 leave a yen for the tie, the member listed
/// first: A 51 property and no cash, B 50 property and 1 cash. Split to
/// cents it was 50.50 each: half a yen of property, which no yen line can
/// hold.
#[test]
fn a_yen_books_k1_shares_property_in_whole_yen() {
    let mut co = company(Currency::JPY, 2);
    let equipment =
        co.l.add_account("1500", "Equipment", AccountType::Asset, None, false)
            .unwrap();
    let e = co.l.create_entry(d(3, 1), "contributed together");
    let yen = |n: &str| Money::new(dec(n), Currency::JPY);
    co.l.add_line(e, equipment, Side::Debit, yen("101"))
        .unwrap();
    co.l.add_line(e, co.cash, Side::Debit, yen("1")).unwrap();
    co.l.add_line(e, co.caps[0], Side::Credit, yen("51"))
        .unwrap();
    co.l.add_line(e, co.caps[1], Side::Credit, yen("51"))
        .unwrap();
    co.l.post_entry(e).unwrap();
    let members = co.member_capitals(&HALF);
    let rows =
        ledger_core::k1::k1_worksheet(&co.l, d(1, 1), d(12, 31), &members, &[co.cash]).unwrap();
    let split: Vec<_> = rows
        .iter()
        .map(|r| (r.contributions_property, r.contributions_cash))
        .collect();
    assert_eq!(split, [(dec("51"), dec("0")), (dec("50"), dec("1"))]);
}

// ---------------------------------------------------------------------------
// The statement of members' equity (D-38; P3 review R-1)
// ---------------------------------------------------------------------------

/// Each member's whole statement column (beginning, contributions, income,
/// distributions, other, ending) for `start..=end`, the members listed in
/// `order` (indexes into `caps`), keyed back to that index.
fn columns(co: &Co, order: &[usize], start: NaiveDate, end: NaiveDate) -> Vec<[Decimal; 6]> {
    let all = co.member_capitals(&HALF);
    let listed: Vec<MemberCapital> = order.iter().map(|&i| all[i].clone()).collect();
    let st = members_equity(&co.l, start, end, &listed).unwrap();
    let mut out = vec![[Decimal::ZERO; 6]; order.len()];
    for (&i, m) in order.iter().zip(&st.members) {
        out[i] = [
            m.beginning,
            m.contributions,
            m.net_income,
            m.distributions,
            m.other,
            m.ending,
        ];
    }
    out
}

/// Regime::allocate's promise, on days before a count's first close too: a
/// reorder of the members in the settings moves no cent. January's cent,
/// earned on the 10th, closes to [A, B] (A's, the tie). As of Jan 15 no
/// close counts yet, but the cent will be closed into the count January's
/// close begins, so its order breaks the tie: A's whether the settings list
/// [A, B] or [B, A], as on Jan 31. In February a cent goes into other
/// equity (unexplained: "other"); its split follows the count's order too,
/// so neither a member's other nor their income moves with the listing.
#[test]
fn a_reorder_moves_no_cent_before_the_counts_first_close() {
    let mut co = company(Currency::USD, 2);
    co.month(1, "0.01", &CloseTo::Members(co.members(&[0, 1], &HALF)));
    co.post(d(2, 5), co.cash, co.other_equity, dec("0.01"));
    for (start, end) in [
        (d(1, 1), d(1, 15)),
        (d(1, 1), d(1, 31)),
        (d(2, 1), d(2, 28)),
    ] {
        assert_eq!(
            columns(&co, &[1, 0], start, end),
            columns(&co, &[0, 1], start, end),
            "{start} to {end}"
        );
    }
    assert_eq!(co.statement(&HALF, d(1, 15)), decs(&["0.01", "0.00"]));
}

/// D-38 (P3 brief R3): a member's capital on a date doesn't change when
/// its period is closed. 50/50; January's cent closes to A. February
/// earns a cent, not yet closed: as of Feb 28 each member's share of it is
/// what February's close will give, allocate(0.02) − allocate(0.01) = 0.00
/// / 0.01, so A 0.01, B 0.01 (the split of February's cent alone gave A
/// 0.02 / B 0.00, and the close then moved it to 0.01 / 0.01). After the
/// close: the same.
#[test]
fn a_members_capital_on_a_date_does_not_change_when_its_period_closes() {
    let mut co = company(Currency::USD, 2);
    let to = CloseTo::Members(co.members(&[0, 1], &HALF));
    co.month(1, "0.01", &to);
    co.earn(d(2, 10), dec("0.01"));
    assert_eq!(co.statement(&HALF, d(2, 28)), decs(&["0.01", "0.01"]));
    co.close(2026, 2, &to).1.unwrap();
    assert_eq!(co.capitals(), decs(&["0.01", "0.01"]));
    assert_eq!(co.statement(&HALF, d(2, 28)), decs(&["0.01", "0.01"]));
}

/// P3 review R-1: a true-up of rounding by closes made under an earlier
/// rule shows on the statement only once a close posts it. After three
/// per-close cents (A 0.03, B 0.00, nothing undistributed), the statement
/// as of Mar 31 is the capital accounts: A 0.03, B 0.00. April's cent,
/// not yet closed: allocate(0.04) − allocate(0.03) = 0.02 − 0.02 / 0.02 −
/// 0.01, so A 0.03, B 0.01 as of Apr 30. April's close posts Dr 4000 0.01,
/// Dr 3000 0.01, Cr 3001 0.02: as of Apr 30, A 0.02, B 0.02 (moved by
/// the true-up, dated at the close); as of Mar 31, still A 0.03, B 0.00.
#[test]
fn a_pending_true_up_shows_only_once_posted() {
    let mut co = company(Currency::USD, 2);
    three_per_close_cents(&mut co);
    assert_eq!(co.statement(&HALF, d(3, 31)), decs(&["0.03", "0.00"]));
    co.earn(d(4, 10), dec("0.01"));
    assert_eq!(co.statement(&HALF, d(4, 30)), decs(&["0.03", "0.01"]));
    co.close(2026, 4, &CloseTo::Members(co.members(&[0, 1], &HALF)))
        .1
        .unwrap();
    assert_eq!(co.statement(&HALF, d(4, 30)), decs(&["0.02", "0.02"]));
    assert_eq!(co.statement(&HALF, d(3, 31)), decs(&["0.03", "0.00"]));
}

/// Three members at 33.33% / 33.33% / 33.34% (a third each can't be
/// written as decimal shares summing to 1; written to 28 places, the
/// product with an amount can't be computed exactly and the close is
/// refused, P3 review R-5), twelve months each closed to them: 100.00,
/// 0.01, 0.02, 33.33, −0.05, 0.10, 1.00, 0.07, 0.01, 12.34, −5.00, 0.02.
///
/// By hand: January 33.33 / 33.33 / 33.34 (exact). February's running
/// 100.01 is 33.333333 / 33.333333 / 33.343334; floors 33.33 / 33.33 /
/// 33.34 leave a cent for C's remainder (.003334): C +0.01. March's
/// 100.03 is 33.339999 / 33.339999 / 33.350002; floors 33.33 / 33.33 /
/// 33.35 leave two cents for A's and B's .009999: A +0.01, B +0.01, C
/// 0.00. December's running 141.85 is 47.278605 / 47.278605 / 47.29279;
/// floors 47.27 / 47.27 / 47.29 leave two cents for A and B: 47.28 /
/// 47.28 / 47.29.
///
/// D-38 on every day: the statement as of each day of the year, the
/// books closed month by month, equals the same statement for the same
/// entries never closed (everything still undistributed); and the K-1
/// worksheet ends each month where the statement does.
#[test]
fn three_members_at_a_third_each_over_twelve_months() {
    let nets = [
        "100.00", "0.01", "0.02", "33.33", "-0.05", "0.10", "1.00", "0.07", "0.01", "12.34",
        "-5.00", "0.02",
    ];
    let mut co = company(Currency::USD, 3);
    let mut open = company(Currency::USD, 3);
    let to = CloseTo::Members(co.members(&[0, 1, 2], &THIRDS));
    let mut closed = Decimal::ZERO;
    let mut parts = Vec::new();
    for (i, net) in nets.iter().enumerate() {
        let m = i as u32 + 1;
        open.earn(d(m, 10), dec(net));
        let before = co.capitals();
        let p = co.month(m, net, &to);
        closed += dec(net);
        let after = co.capitals();
        let part: Vec<Decimal> = after.iter().zip(&before).map(|(a, b)| a - b).collect();
        assert_eq!(part.iter().sum::<Decimal>(), dec(net), "month {m}");
        for (cap, share) in after.iter().zip(THIRDS) {
            assert!((cap - closed * dec(share)).abs() < dec("0.01"), "month {m}");
        }
        assert_eq!(co.l.member_close_split(p).unwrap().0, 1);
        parts.push(part);
    }
    assert_eq!(parts[0], decs(&["33.33", "33.33", "33.34"]));
    assert_eq!(parts[1], decs(&["0.00", "0.00", "0.01"]));
    assert_eq!(parts[2], decs(&["0.01", "0.01", "0.00"]));
    assert_eq!(co.capitals(), decs(&["47.28", "47.28", "47.29"]));

    let members = co.member_capitals(&THIRDS);
    let mut date = d(1, 1);
    while date <= d(12, 31) {
        assert_eq!(
            co.statement(&THIRDS, date),
            open.statement(&THIRDS, date),
            "{date}"
        );
        if date.succ_opt().unwrap().day() == 1 {
            let rows =
                ledger_core::k1::k1_worksheet(&co.l, d(1, 1), date, &members, &[co.cash]).unwrap();
            let k1: Vec<Decimal> = rows.iter().map(|r| r.ending).collect();
            assert_eq!(k1, co.statement(&THIRDS, date), "{date}");
        }
        date = date.succ_opt().unwrap();
    }
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

/// How a close of the property below is made.
#[derive(Clone, Copy, Debug)]
enum Close {
    /// The cumulative rule, the members in their order.
    Members,
    /// The cumulative rule, the members in reverse order.
    Reversed,
    /// To retained earnings.
    Retained,
    /// Not closed this month.
    Open,
}

fn close_kind() -> impl Strategy<Value = Close> {
    prop_oneof![
        Just(Close::Members),
        Just(Close::Reversed),
        Just(Close::Retained),
        Just(Close::Open),
    ]
}

/// A legacy close (replay only): 0 per close, 1 the last absorbs, 2 the
/// run rule.
fn legacy_to(kind: usize, members: Vec<MemberClose>) -> CloseTo {
    match kind {
        0 => CloseTo::MembersPerClose(members),
        1 => CloseTo::MembersLastAbsorbs(members),
        _ => CloseTo::MembersRun(members),
    }
}

/// Month `i` from January 2026.
fn nth_month(i: usize) -> (i32, u32) {
    (2026 + (i / 12) as i32, (i % 12) as u32 + 1)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Oracle: rules.md B-13 and close INVARIANT 6, hand-stated: after
    /// any history (closes under the legacy rules, then months closed to
    /// the members in either order, to retained earnings, or left open),
    /// every members close hands out exactly its net; once a cumulative
    /// close has posted, each member's capital is under a cent from their
    /// share of everything closed to the members; a close that posts
    /// nothing changes no capital (P3 review R-3); and member_close_split
    /// agrees with the balances. Fails on the run rule (the audit's
    /// pattern: a close to retained earnings restarts it).
    #[test]
    fn members_capital_never_drifts_a_cent_from_their_share(
        thirds in any::<bool>(),
        legacy in prop::collection::vec((0usize..3, -500_000i64..500_000), 0..4),
        months in prop::collection::vec((close_kind(), -500_000i64..500_000), 1..13),
    ) {
        let shares: &[&str] = if thirds { &THIRDS } else { &HALF };
        let n = shares.len();
        let mut co = company(Currency::USD, n);
        let order: Vec<usize> = (0..n).collect();
        let reversed: Vec<usize> = (0..n).rev().collect();
        // Everything closed to the members (any rule), and the close the
        // count started at.
        let mut closed = Decimal::ZERO;
        let mut first: Option<u64> = None;
        let mut month = 0;
        for (kind, net) in legacy {
            let (y, m) = nth_month(month);
            month += 1;
            co.earn(day(y, m, 10), Decimal::new(net, 2));
            let to = legacy_to(kind, co.members(&order, shares));
            let (p, r) = co.close(y, m, &to);
            r.unwrap();
            closed += Decimal::new(net, 2);
            first = first.or(co.l.get_period(p).unwrap().close_seq);
        }
        let mut pending = Decimal::ZERO;
        let mut posted = false;
        for (kind, net) in months {
            let (y, m) = nth_month(month);
            month += 1;
            co.earn(day(y, m, 10), Decimal::new(net, 2));
            pending += Decimal::new(net, 2);
            let to = match kind {
                Close::Open => continue,
                Close::Retained => {
                    co.close(y, m, &CloseTo::RetainedEarnings { sweep: vec![] }).1.unwrap();
                    pending = Decimal::ZERO;
                    continue;
                }
                Close::Members => CloseTo::Members(co.members(&order, shares)),
                Close::Reversed => CloseTo::Members(co.members(&reversed, shares)),
            };
            let before = co.capitals();
            let (p, r) = co.close(y, m, &to);
            let entry = r.unwrap();
            let period = co.l.get_period(p).unwrap().clone();
            first = first.or(period.close_seq);
            closed += pending;
            let record = period.members_closed.unwrap();
            prop_assert_eq!(record.net, pending);
            pending = Decimal::ZERO;
            prop_assert_eq!(record.parts.iter().sum::<Decimal>(), record.net);
            prop_assert_eq!(record.rule, MemberRule::Cumulative { since_seq: first.unwrap() });
            posted |= entry.is_some();
            let after = co.capitals();
            if entry.is_none() {
                // Posting nothing, it credits no one: any true-up waits.
                prop_assert_eq!(&after, &before);
            }
            if !posted {
                continue;
            }
            for (cap, share) in after.iter().zip(shares) {
                prop_assert!((cap - closed * dec(share)).abs() < dec("0.01"), "{} vs {}", cap, closed * dec(share));
            }
            let (since, split) = co.l.member_close_split(p).unwrap();
            prop_assert_eq!(since, first.unwrap());
            for s in split {
                let i = co.caps.iter().position(|&c| c == s.capital).unwrap();
                prop_assert_eq!(s.running, after[i]);
                prop_assert_eq!(s.exact, closed * s.share);
            }
        }
        prop_assert!(ledger_core::reports::trial_balance(&co.l).balanced());
    }

    /// Oracle: members' equity INVARIANTS 3a and 3c (D-38): for random
    /// books with contributions, distributions, profits and losses, and
    /// months closed to the members, to retained earnings or not at all,
    /// with the settings listing the members in either order from month to
    /// month (the close and the statement list them as the settings do),
    /// each member's capital on the statement as of a close's period end
    /// is the same before and after the close; and each member's ending on
    /// one statement is their beginning on the next, whatever the split
    /// date.
    #[test]
    fn each_members_capital_chains_and_holds_through_closes(
        months in prop::collection::vec((0usize..3, 0i64..50_000, 0i64..20_000, 0i64..100_000, 0i64..80_000), 1..12),
        kinds in prop::collection::vec(close_kind(), 12),
        split in 1u32..330,
    ) {
        let mut co = company(Currency::USD, 3);
        let cents = |n: i64| Decimal::new(n, 2);
        let order = [0, 1, 2];
        let reversed = [2, 1, 0];
        for (i, (who, contrib, dist, sales, costs)) in months.iter().enumerate() {
            let m = i as u32 + 1;
            let cap = co.caps[*who];
            if *contrib > 0 {
                co.post(d(m, 3), co.cash, cap, cents(*contrib));
            }
            co.earn(d(m, 10), cents(*sales));
            co.earn(d(m, 15), -cents(*costs));
            if *dist > 0 {
                co.post(d(m, 20), co.dists[*who], co.cash, cents(*dist));
            }
            let with_dists = |co: &Co, order: &[usize]| -> Vec<MemberClose> {
                let mut ms = co.members(order, &THIRDS);
                for (m, &i) in ms.iter_mut().zip(order) {
                    m.distributions = Some(co.dists[i]);
                }
                ms
            };
            let listed: &[usize] = match kinds[i] {
                Close::Reversed => &reversed,
                _ => &order,
            };
            let to = match kinds[i] {
                Close::Open => continue,
                Close::Retained => CloseTo::RetainedEarnings { sweep: vec![] },
                Close::Members | Close::Reversed => CloseTo::Members(with_dists(&co, listed)),
            };
            let end = month_end(2026, m);
            let before = co.statement_listing(listed, &THIRDS, end);
            co.close(2026, m, &to).1.unwrap();
            prop_assert_eq!(co.statement_listing(listed, &THIRDS, end), before, "month {}", m);
        }
        let members = co.member_capitals(&THIRDS);
        let mid = d(1, 1) + chrono::Days::new(u64::from(split));
        let first = members_equity(&co.l, d(1, 1), mid, &members).unwrap();
        let second = members_equity(&co.l, mid.succ_opt().unwrap(), d(12, 31), &members).unwrap();
        let whole = members_equity(&co.l, d(1, 1), d(12, 31), &members).unwrap();
        for i in 0..3 {
            prop_assert_eq!(first.members[i].ending, second.members[i].beginning);
            prop_assert_eq!(second.members[i].ending, whole.members[i].ending);
        }
        for st in [&first, &second, &whole] {
            let incomes: Decimal = st.members.iter().map(|m| m.net_income).sum();
            prop_assert_eq!(incomes, st.total.net_income);
        }
    }

    /// Oracle: members_regime's doc (P3 review R-10.6): whatever order
    /// nested periods (months, quarters, the year) are tried in, the
    /// periods that close end strictly later, close after close, so "the
    /// closes dated on or before a day" is always the first closes in
    /// order.
    #[test]
    fn period_ends_rise_with_close_order(
        attempts in Just((0usize..17).collect::<Vec<_>>()).prop_shuffle(),
    ) {
        let mut co = company(Currency::USD, 2);
        let mut periods = Vec::new();
        for m in 1..=12 {
            periods.push(co.l.create_period(&format!("m{m}"), d(m, 1), month_end(2026, m)).unwrap());
        }
        for q in 0..4u32 {
            periods.push(co.l.create_period(&format!("q{q}"), d(3 * q + 1, 1), month_end(2026, 3 * q + 3)).unwrap());
        }
        periods.push(co.l.create_period("year", d(1, 1), d(12, 31)).unwrap());
        for m in 1..=12 {
            co.earn(d(m, 10), dec("0.01"));
        }
        let to = CloseTo::Members(co.members(&[0, 1], &HALF));
        for i in attempts {
            let _ = co.l.close_period_to(periods[i], &to);
        }
        let mut closed: Vec<_> = co.l.list_periods().into_iter().filter(|p| p.close_seq.is_some()).collect();
        closed.sort_by_key(|p| p.close_seq);
        for w in closed.windows(2) {
            prop_assert!(w[0].end < w[1].end, "{} then {}", w[0].name, w[1].name);
        }
    }
}
