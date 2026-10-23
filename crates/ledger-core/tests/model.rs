//! Model-based stateful test (rank 3: a peer).
//!
//! The peer is the Lean state machine (verify/lean/Wanderbooks/Ledger.lean)
//! transcribed into the simplest Rust that has the same observable
//! behavior: a list of accounts, a list of drafts, an append-only list of
//! posted entries, a list of periods, and loops. It is written from the
//! spec (rules.md, the Ledger doc comments), not from ledger.rs.
//!
//! Random command sequences, weighted toward commands that interact (post
//! a draft, reverse a posted entry, close a period with drafts in it,
//! reverse a closing entry, reverse twice), run against both. After every
//! command: the same accept/refuse decision, and the same posted log,
//! chart, drafts, periods, and every account's balance.
//!
//! `MODEL_STATS=1 cargo test --test model -- --nocapture` prints every
//! accepted command: check that posts, reversals, and closes really
//! happen. (The first version never posted anything, so it agreed with the
//! model vacuously; the balanced `Pair` command fixed that.) Shown to have
//! teeth: letting the model reverse an entry twice fails in 0.03s.

use chrono::NaiveDate;
use ledger_core::reports::trial_balance;
use ledger_core::{AccountType, Currency, EntryStatus, Ledger, Money, Side};
use proptest::prelude::*;
use rust_decimal::Decimal;

// ---------------------------------------------------------------------------
// The model
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
struct MLine {
    account: u64,
    side: Side,
    amount: Decimal,
}

#[derive(Clone, Debug)]
struct MEntry {
    id: u64,
    date: NaiveDate,
    lines: Vec<MLine>,
    reversal_of: Option<u64>,
    closes: bool,
}

#[derive(Clone, Debug)]
struct MPeriod {
    start: NaiveDate,
    end: NaiveDate,
    closed: bool,
}

#[derive(Clone, Debug, Default)]
struct Model {
    /// (id, code, type, placeholder, name). The name is outside the Lean
    /// model (its chart is code and type): kept here so the two agree on
    /// renames' refusals, and a rename moves nothing else (rules.md B-31).
    accounts: Vec<(u64, String, AccountType, bool, String)>,
    drafts: Vec<MEntry>,
    posted: Vec<MEntry>,
    periods: Vec<MPeriod>,
    next_account: u64,
    next_entry: u64,
}

fn signed(l: &MLine) -> Decimal {
    if l.side == Side::Debit {
        l.amount
    } else {
        -l.amount
    }
}

impl Model {
    fn new() -> Self {
        Model {
            next_account: 1,
            next_entry: 1,
            ..Default::default()
        }
    }

    fn account(&self, id: u64) -> Option<&(u64, String, AccountType, bool, String)> {
        self.accounts.iter().find(|a| a.0 == id)
    }

    fn closed_on(&self, d: NaiveDate) -> bool {
        self.periods
            .iter()
            .any(|p| p.closed && p.start <= d && d <= p.end)
    }

    /// Debits minus credits on an account, over posted entries (through
    /// `end` if given).
    fn movement(&self, account: u64, end: Option<NaiveDate>) -> Decimal {
        self.posted
            .iter()
            .filter(|e| end.is_none_or(|end| e.date <= end))
            .flat_map(|e| &e.lines)
            .filter(|l| l.account == account)
            .map(signed)
            .sum()
    }

    fn add_account(&mut self, code: &str, name: &str, t: AccountType, placeholder: bool) -> bool {
        if code.is_empty()
            || self.accounts.iter().any(|a| a.1 == code)
            || !self.name_free(None, name)
        {
            return false;
        }
        self.add_engine_account(code, name, t, placeholder);
        true
    }

    /// An account the engine makes for a control role: its fixed name,
    /// never refused for one.
    fn add_engine_account(&mut self, code: &str, name: &str, t: AccountType, placeholder: bool) {
        self.accounts
            .push((self.next_account, code.into(), t, placeholder, name.into()));
        self.next_account += 1;
    }

    /// rules.md B-31: a name is free for `account` (None: a new one) when
    /// it isn't blank, holds no nine digits in a row, and no other account
    /// (all are top-level here, so all are siblings) has it, ignoring
    /// capitals and surrounding space.
    fn name_free(&self, account: Option<u64>, name: &str) -> bool {
        let key = name.trim().to_lowercase();
        let nine_digits = name
            .as_bytes()
            .windows(9)
            .any(|w| w.iter().all(u8::is_ascii_digit));
        !key.is_empty()
            && !nine_digits
            && !self
                .accounts
                .iter()
                .any(|a| Some(a.0) != account && a.4.trim().to_lowercase() == key)
    }

    /// rules.md B-31: rename an account that exists to a free name other
    /// than its own; only the name changes.
    fn rename(&mut self, id: u64, name: &str) -> bool {
        let Some(i) = self.accounts.iter().position(|a| a.0 == id) else {
            return false;
        };
        if name.trim() == self.accounts[i].4 || !self.name_free(Some(id), name) {
            return false;
        }
        self.accounts[i].4 = name.trim().to_string();
        true
    }

    fn draft(&mut self, date: NaiveDate) -> Option<u64> {
        if self.closed_on(date) {
            return None;
        }
        let id = self.next_entry;
        self.next_entry += 1;
        self.drafts.push(MEntry {
            id,
            date,
            lines: vec![],
            reversal_of: None,
            closes: false,
        });
        Some(id)
    }

    fn add_line(&mut self, entry: u64, l: MLine) -> bool {
        let postable = self.account(l.account).is_some_and(|a| !a.3);
        let Some(d) = self.drafts.iter_mut().find(|d| d.id == entry) else {
            return false;
        };
        if !postable {
            return false;
        }
        d.lines.push(l);
        true
    }

    fn acceptable(&self, lines: &[MLine]) -> bool {
        let debits: Decimal = lines
            .iter()
            .filter(|l| l.side == Side::Debit)
            .map(|l| l.amount)
            .sum();
        let credits: Decimal = lines
            .iter()
            .filter(|l| l.side == Side::Credit)
            .map(|l| l.amount)
            .sum();
        lines.len() >= 2
            && lines.iter().all(|l| l.amount > Decimal::ZERO)
            && lines
                .iter()
                .all(|l| self.account(l.account).is_some_and(|a| !a.3))
            && debits == credits
    }

    fn post(&mut self, entry: u64) -> bool {
        let Some(i) = self.drafts.iter().position(|d| d.id == entry) else {
            return false;
        };
        let d = &self.drafts[i];
        if self.closed_on(d.date) || !self.acceptable(&d.lines) {
            return false;
        }
        let d = self.drafts.remove(i);
        self.posted.push(d);
        true
    }

    fn reverse(&mut self, entry: u64, date: NaiveDate) -> bool {
        let Some(orig) = self.posted.iter().find(|e| e.id == entry).cloned() else {
            return false;
        };
        let reversed_already = self.posted.iter().any(|e| e.reversal_of == Some(entry));
        if reversed_already || orig.closes || self.closed_on(date) {
            return false;
        }
        let lines = orig
            .lines
            .iter()
            .map(|l| MLine {
                side: if l.side == Side::Debit {
                    Side::Credit
                } else {
                    Side::Debit
                },
                ..l.clone()
            })
            .collect();
        self.posted.push(MEntry {
            id: self.next_entry,
            date,
            lines,
            reversal_of: Some(entry),
            closes: false,
        });
        self.next_entry += 1;
        true
    }

    fn create_period(&mut self, start: NaiveDate, end: NaiveDate) -> bool {
        if end < start {
            return false;
        }
        let nests =
            |p: &MPeriod| (start >= p.start && end <= p.end) || (p.start >= start && p.end <= end);
        if self
            .periods
            .iter()
            .any(|p| start <= p.end && p.start <= end && !nests(p))
        {
            return false;
        }
        self.periods.push(MPeriod {
            start,
            end,
            closed: false,
        });
        true
    }

    /// Close into retained earnings (3999).
    fn close(&mut self, index: usize) -> bool {
        let Some(p) = self.periods.get(index).cloned() else {
            return false;
        };
        if p.closed
            || self
                .drafts
                .iter()
                .any(|d| p.start <= d.date && d.date <= p.end)
            || self.periods.iter().any(|q| q.closed && q.end > p.end)
            || self
                .periods
                .iter()
                .any(|q| q.closed && q.start <= p.end && p.end <= q.end)
        {
            return false;
        }
        // Retained earnings must be a postable equity account if it exists.
        if let Some(re) = self.accounts.iter().find(|a| a.1 == "3999")
            && (re.2 != AccountType::Equity || re.3)
        {
            return false;
        }
        let mut lines = Vec::new();
        let nominal: Vec<u64> = {
            let mut v: Vec<_> = self
                .accounts
                .iter()
                .filter(|a| matches!(a.2, AccountType::Revenue | AccountType::Expense))
                .collect();
            v.sort_by(|a, b| a.1.cmp(&b.1));
            v.iter().map(|a| a.0).collect()
        };
        for a in nominal {
            let m = self.movement(a, Some(p.end));
            if m > Decimal::ZERO {
                lines.push(MLine {
                    account: a,
                    side: Side::Credit,
                    amount: m,
                });
            } else if m < Decimal::ZERO {
                lines.push(MLine {
                    account: a,
                    side: Side::Debit,
                    amount: -m,
                });
            }
        }
        if !lines.is_empty() {
            let net: Decimal = lines.iter().map(signed).sum();
            let re = match self.accounts.iter().find(|a| a.1 == "3999") {
                Some(a) => a.0,
                None => {
                    self.add_engine_account(
                        "3999",
                        "Retained Earnings",
                        AccountType::Equity,
                        false,
                    );
                    self.next_account - 1
                }
            };
            if net > Decimal::ZERO {
                lines.push(MLine {
                    account: re,
                    side: Side::Credit,
                    amount: net,
                });
            } else if net < Decimal::ZERO {
                lines.push(MLine {
                    account: re,
                    side: Side::Debit,
                    amount: -net,
                });
            }
            self.posted.push(MEntry {
                id: self.next_entry,
                date: p.end,
                lines,
                reversal_of: None,
                closes: true,
            });
            self.next_entry += 1;
        }
        self.periods[index].closed = true;
        true
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

// No 1200 or 2000: a line by hand on a receivables or payables account is
// refused (the control guard, rules.md B-38), which this model leaves to
// spec_arap.rs.
const CODES: [&str; 7] = ["1000", "2100", "3000", "3999", "4000", "6000", "1900"];
/// New names: a free one, its own in other capitals and spacing, a
/// sibling's (the accounts are named for their codes), blank, nine
/// digits, the engine's own name for retained earnings.
const NAMES: [&str; 10] = [
    "Software",
    "SOFTWARE",
    " software ",
    "Hosting",
    "",
    "   ",
    "Acct 123456789",
    "1000",
    "Retained Earnings",
    "Rent",
];
const TYPES: [AccountType; 5] = [
    AccountType::Asset,
    AccountType::Liability,
    AccountType::Equity,
    AccountType::Revenue,
    AccountType::Expense,
];

#[derive(Clone, Debug)]
enum Cmd {
    AddAccount(usize, usize, bool),
    Draft(u32),
    AddLine(usize, usize, bool, i64),
    /// A balanced pair: debit one account, credit another, same amount.
    Pair(usize, usize, usize, i64),
    Post(usize),
    Reverse(usize, u32),
    CreatePeriod(u32, u32),
    Close(usize),
    /// Rename an account (rules.md B-31): `Step.record` in the Lean model,
    /// whose chart has no names.
    Rename(usize, usize),
}

fn day(n: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 1, 1).unwrap() + chrono::Days::new(u64::from(n % 180))
}

fn cmd() -> impl Strategy<Value = Cmd> {
    prop_oneof![
        2 => (0..CODES.len(), 0..TYPES.len(), prop::bool::weighted(0.1))
            .prop_map(|(c, t, h)| Cmd::AddAccount(c, t, h)),
        3 => (0u32..180).prop_map(Cmd::Draft),
        6 => (0usize..8, 0usize..8, any::<bool>(),
              prop_oneof![8 => 1i64..100_000, 1 => Just(0i64), 1 => -500i64..0])
            .prop_map(|(e, a, s, c)| Cmd::AddLine(e, a, s, c)),
        6 => (0usize..8, 0usize..8, 0usize..8, 1i64..100_000)
            .prop_map(|(e, a, b, c)| Cmd::Pair(e, a, b, c)),
        5 => (0usize..8).prop_map(Cmd::Post),
        2 => (0usize..8, 0u32..180).prop_map(|(e, d)| Cmd::Reverse(e, d)),
        2 => (0u32..180, 0u32..60).prop_map(|(s, len)| Cmd::CreatePeriod(s, s + len)),
        2 => (0usize..4).prop_map(Cmd::Close),
        2 => (0usize..8, 0..NAMES.len()).prop_map(|(a, n)| Cmd::Rename(a, n)),
    ]
}

/// Apply one command to both and return (engine accepted, model accepted).
/// Indices pick among existing drafts / posted entries / accounts / periods,
/// so most commands hit something real.
fn apply(l: &mut Ledger, m: &mut Model, c: &Cmd) -> (bool, bool) {
    let pick = |v: &[u64], i: usize| v.get(i % v.len().max(1)).copied().unwrap_or(999);
    let accounts: Vec<u64> = m.accounts.iter().map(|a| a.0).collect();
    let drafts: Vec<u64> = m.drafts.iter().map(|d| d.id).collect();
    let posted: Vec<u64> = m.posted.iter().map(|e| e.id).collect();
    match *c {
        Cmd::AddAccount(code, t, h) => (
            l.add_account(CODES[code], CODES[code], TYPES[t], None, h)
                .is_ok(),
            m.add_account(CODES[code], CODES[code], TYPES[t], h),
        ),
        Cmd::Draft(d) => {
            let got = l.try_create_entry(day(d), "x").ok();
            let want = m.draft(day(d));
            assert_eq!(got, want, "draft ids");
            (got.is_some(), want.is_some())
        }
        Cmd::AddLine(e, a, debit, cents) => {
            let (e, a) = (pick(&drafts, e), pick(&accounts, a));
            let side = if debit { Side::Debit } else { Side::Credit };
            let amount = Decimal::new(cents, 2);
            (
                l.add_line(e, a, side, Money::new(amount, Currency::USD))
                    .is_ok(),
                m.add_line(
                    e,
                    MLine {
                        account: a,
                        side,
                        amount,
                    },
                ),
            )
        }
        Cmd::Pair(e, a, b, cents) => {
            let (e, a, b) = (pick(&drafts, e), pick(&accounts, a), pick(&accounts, b));
            let amount = Decimal::new(cents, 2);
            let money = Money::new(amount, Currency::USD);
            let got = l.add_line(e, a, Side::Debit, money).is_ok()
                && l.add_line(e, b, Side::Credit, money).is_ok();
            let want = m.add_line(
                e,
                MLine {
                    account: a,
                    side: Side::Debit,
                    amount,
                },
            ) && m.add_line(
                e,
                MLine {
                    account: b,
                    side: Side::Credit,
                    amount,
                },
            );
            (got, want)
        }
        Cmd::Post(e) => {
            let e = pick(&drafts, e);
            (l.post_entry(e).is_ok(), m.post(e))
        }
        Cmd::Reverse(e, d) => {
            let e = pick(&posted, e);
            (
                l.reverse_entry(e, day(d), "undo").is_ok(),
                m.reverse(e, day(d)),
            )
        }
        Cmd::CreatePeriod(s, e) => (
            l.create_period("p", day(s), day(e)).is_ok(),
            m.create_period(day(s), day(e)),
        ),
        Cmd::Close(i) => {
            let ids: Vec<u64> = l.list_periods().iter().map(|p| p.id).collect();
            let i = if ids.is_empty() { 0 } else { i % ids.len() };
            let id = ids.get(i).copied().unwrap_or(999);
            (l.close_period(id).is_ok(), m.close(i))
        }
        Cmd::Rename(a, n) => {
            let a = pick(&accounts, a);
            (l.rename_account(a, NAMES[n]).is_ok(), m.rename(a, NAMES[n]))
        }
    }
}

/// Every observable agrees.
fn agree(l: &Ledger, m: &Model) -> Result<(), TestCaseError> {
    // Chart.
    prop_assert_eq!(l.chart().len(), m.accounts.len());
    for (id, code, t, h, name) in &m.accounts {
        let a = l.chart().get(*id).expect("account");
        prop_assert_eq!(
            (&a.code, a.account_type, a.placeholder, &a.name),
            (code, *t, *h, name)
        );
    }
    // The posted log, in order, line for line.
    let log: Vec<_> = l.posted_entries().collect();
    prop_assert_eq!(log.len(), m.posted.len());
    for (e, me) in log.iter().zip(&m.posted) {
        prop_assert_eq!(
            (e.id(), e.date(), e.reversal_of()),
            (me.id, me.date, me.reversal_of)
        );
        let lines: Vec<MLine> = e
            .lines()
            .iter()
            .map(|l| MLine {
                account: l.account,
                side: l.side,
                amount: l.amount.amount,
            })
            .collect();
        prop_assert_eq!(&lines, &me.lines, "entry {}", e.id());
    }
    // Drafts, with their lines.
    for d in &m.drafts {
        let e = l.get_entry(d.id).expect("draft exists");
        prop_assert_eq!(e.status(), EntryStatus::Draft);
        prop_assert_eq!(e.lines().len(), d.lines.len());
    }
    prop_assert_eq!(l.next_entry_id(), m.next_entry);
    prop_assert_eq!(l.next_account_id(), m.next_account);
    // Periods.
    let ps = l.list_periods();
    prop_assert_eq!(ps.len(), m.periods.len());
    for (p, mp) in ps.iter().zip(&m.periods) {
        prop_assert_eq!(
            (
                p.start,
                p.end,
                p.status == ledger_core::PeriodStatus::Closed
            ),
            (mp.start, mp.end, mp.closed)
        );
    }
    // Every account's balance, and the trial balance balances.
    for (id, ..) in &m.accounts {
        prop_assert_eq!(
            l.account_movement(*id),
            m.movement(*id, None),
            "account {}",
            id
        );
    }
    let tb = trial_balance(l);
    prop_assert_eq!(tb.total_debits, tb.total_credits);
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]

    /// Oracle: the model (the Lean state machine, transcribed). Same
    /// decision on every command, same books after every command.
    #[test]
    fn the_ledger_matches_its_model(cmds in prop::collection::vec(cmd(), 1..60)) {
        let mut l = Ledger::new(Currency::USD);
        let mut m = Model::new();
        for c in &cmds {
            let (got, want) = apply(&mut l, &mut m, c);
            if std::env::var("MODEL_STATS").is_ok() && got {
                eprintln!("ACCEPTED {}", format!("{c:?}").split('(').next().unwrap());
            }
            prop_assert_eq!(got, want, "{:?}: engine accepted {}, model {}", c, got, want);
            agree(&l, &m)?;
        }
    }
}
