//! The Rust `Ledger` and the Lean model of it, differentially (P14 L-G5).
//!
//! `docs/verification.md` proves the accounting laws of a *model* of the
//! ledger (bookspec's `Bookspec.Step`, and its computable twin
//! `Bookspec.Exec.step`, proved sound and complete against it). That the
//! model is the engine was checked by review. This test makes it a
//! machine check: random command sequences are run through the Rust
//! `Ledger`, written as bookspec examples whose expected values are the
//! Rust engine's own answers, and handed to the Lean model
//! (`lake exe bookspec-check`, which runs every event through `step` and
//! compares each figure and each refusal). Any `mismatch` is the engine
//! and the model disagreeing; the test shrinks the sequence, prints it with
//! both answers and the command that diverged, and fails.
//!
//! What the example format lets a sequence say (and so what is compared):
//!
//! * an entry: drafted and posted whole (1 to 4 lines, balanced or not,
//!   positive, zero, negative or finer-than-a-cent amounts, an unknown
//!   account, a control account), on dates spread over two years;
//! * a reversal of any earlier event that posted an entry (a reversal, a
//!   closing entry, one already reversed, dated before, on or after it);
//! * a close to retained earnings, which is a period opened and closed in
//!   one event (nested, disjoint, partly overlapping, inverted, around
//!   nothing at all, around something, inside or after a closed one).
//!
//! What it cannot say, and the model's other commands are therefore not
//! reached: a standalone `openPeriod`, a draft left unposted (so
//! `drafts_in_period` and `discard`), `addAccount` after the first event
//! (the chart is the setup's), a reversal of an event that posted nothing.
//! Those stay by review (docs/verification.md).
//!
//! The figures compared: after every event the trial balance; at the end
//! the trial balance and the balances on several days, the profit and
//! loss over several ranges, and every posted entry's lines (each
//! account's debit and credit, zero included, so an extra line shows).
//!
//! How a refusal is compared. The checker reads `refused = "<word>"` and
//! checks the model answers that word, but only for the words it lists
//! (`refusalWords` in `Exec.lean`); an event refused with another word is
//! skipped, never run. Two kinds of those:
//!
//! * words the model has but the checker does not list
//!   (`period_overlaps`, `period_end_before_start`, `close_out_of_order`,
//!   `closed_period_contains_end`, ...): the event is written as refused
//!   with a *probe* word no model refusal uses, and the model's actual
//!   word is read from the mismatch it reports ("expected refusal probe,
//!   got X") and compared with the engine's. A model that accepts the
//!   event reports that instead, and it is a disagreement;
//! * words the model does not have ([`UNCHECKED_BY_THE_MODEL`]): written
//!   as they are, the checker skips them, and the skip is allowed because
//!   it is in that list.
//!
//! Each refused command is tried on a clone and kept only if accepted, as
//! an example's events are applied: a close that is refused leaves no open
//! period behind, which is how the model's event reads it.
//!
//! Run it: `just lean-model` (`PROPTEST_CASES` sequences, one `lake` call
//! for all of them, one more for each round of shrinking). It skips
//! without `lake`, unless `WANDERBOOKS_REQUIRE_LEAN` is set (then it
//! fails); `WANDERBOOKS_SEED` replays a run; `WANDERBOOKS_BOOKSPEC_LEAN`
//! names a bookspec `lean/` directory to use instead of the pinned
//! checkout.

use crate::*;
use ledger_core::AccountId;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::Command;
use std::str::FromStr;

// ---- what the sequences are made of --------------------------------------

/// A line as the example writes it: the account's code, the side, and the
/// amount's text (so a negative, zero or finer-than-a-cent amount is
/// expressible).
#[derive(Clone, Debug, PartialEq)]
struct L {
    code: u32,
    debit: bool,
    amount: String,
}

#[derive(Clone, Debug, PartialEq)]
enum Cmd {
    /// An entry, drafted and posted whole.
    Entry { date: NaiveDate, lines: Vec<L> },
    /// A reversal of the entry event `of` (0-based), dated `date`.
    Reverse { of: usize, date: NaiveDate },
    /// A period `start..=end` opened and closed to retained earnings.
    Close { start: NaiveDate, end: NaiveDate },
    /// An invoice for `cents`: the books post 1200 debit, 4000 credit (the
    /// model's fixed document accounts), and a hand line on 1200 is
    /// refused from then on (the control guard).
    Invoice { date: NaiveDate, cents: i64 },
}

/// A sequence: the chart (the setup's), its commands, and the days the
/// final queries ask about.
#[derive(Clone, Debug)]
struct Seq {
    chart: Vec<(u32, AccountType)>,
    cmds: Vec<Cmd>,
    tb_days: Vec<NaiveDate>,
    balance_days: Vec<NaiveDate>,
    pl_ranges: Vec<(NaiveDate, NaiveDate)>,
}

/// The five accounts the checker's books always have (`roleAccounts` in
/// `Exec.lean`): declared in every chart so both sides have them from the
/// start (the engine makes 3999 only when a close needs it).
const ROLE_ACCOUNTS: [(u32, AccountType); 5] = [
    (1200, AccountType::Asset),
    (2000, AccountType::Liability),
    (3999, AccountType::Equity),
    (4000, AccountType::Revenue),
    (6999, AccountType::Expense),
];

/// Accounts a chart may add (1000 and 3000 always).
const OPTIONAL_ACCOUNTS: [(u32, AccountType); 6] = [
    (1100, AccountType::Asset),
    (2100, AccountType::Liability),
    (4100, AccountType::Revenue),
    (4200, AccountType::Revenue),
    (5000, AccountType::Expense),
    (6000, AccountType::Expense),
];

/// A code no chart has.
const UNKNOWN_CODE: u32 = 9999;

/// The engine's id for an account it does not have.
const UNKNOWN_ID: AccountId = 999_999;

fn account_name(code: u32) -> String {
    format!("Account {code}")
}

fn type_word(t: AccountType) -> &'static str {
    match t {
        AccountType::Asset => "asset",
        AccountType::Liability => "liability",
        AccountType::Equity => "equity",
        AccountType::Revenue => "revenue",
        AccountType::Expense => "expense",
    }
}

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("a real day")
}

/// A day later than anything a sequence dates.
fn far() -> NaiveDate {
    day(2030, 12, 31)
}

/// Cents as the example writes an amount.
fn cents(c: i64) -> String {
    let sign = if c < 0 { "-" } else { "" };
    format!("{sign}{}.{:02}", c.abs() / 100, c.abs() % 100)
}

// ---- the Rust engine, one command at a time ------------------------------

/// What the engine did with one command.
#[derive(Clone, Debug, PartialEq)]
enum Outcome {
    Posted,
    /// Refused, in the examples' words.
    Refused(String),
    /// Accepted by the engine, but a gap the owner has ruled on and the
    /// engine does not enforce yet ([`KNOWN_GAPS`]): the command is rolled
    /// back, as if refused, and written as refused with this word.
    KnownGap(&'static str),
}

#[derive(Clone)]
struct Step {
    outcome: Outcome,
    /// The entry the command posted, if any (a close with nothing to
    /// close posts none).
    entry: Option<EntryId>,
}

/// The books as the engine keeps them, and what each event did.
#[derive(Clone)]
struct Runner {
    l: Ledger,
    ids: BTreeMap<u32, AccountId>,
    customer: ledger_core::CustomerId,
    steps: Vec<Step>,
    /// Entry events the engine accepted a reversal of.
    reversed: BTreeSet<usize>,
    /// Events whose entry is a closing entry.
    closes: BTreeSet<usize>,
    /// The trial balance after each event: (code, debit minus credit).
    tb_after: Vec<Vec<(u32, Decimal)>>,
}

/// An engine refusal in the model's words. The adapter's own mapping
/// ([`refusal`]) covers the examples' words; the model has more
/// (`Refusal` in `Exec/Step.lean`), mapped here.
fn lean_word(e: &LedgerError) -> String {
    let debug = format!("{e:?}");
    match e {
        // The adapter's `end_before_start` (`InvalidDateRange`) is BS-period-3's,
        // a query asked for an end before its start; a period opened that
        // way is BS-period-2's.
        LedgerError::InvalidPeriodRange { .. } => "period_end_before_start".to_string(),
        LedgerError::PeriodsOverlap { .. } => "period_overlaps".to_string(),
        LedgerError::UnknownAccount(_) => "unknown_account".to_string(),
        LedgerError::ClosingEntryIrreversible(_) => "closing_entry_irreversible".to_string(),
        LedgerError::CloseOutOfOrder(..) => "close_out_of_order".to_string(),
        LedgerError::DraftsInPeriod(_) => "drafts_in_period".to_string(),
        LedgerError::PeriodAlreadyClosed(_) => "period_already_closed".to_string(),
        // D-73: a reversal comes after what it reverses (the engine's
        // variant for it, whatever it is called, when it lands).
        _ if debug.contains("ReversalBeforeOriginal") => "reversal_before_original".to_string(),
        other => refusal(other),
    }
}

/// Behaviors the owner has ruled on that the engine does not enforce yet:
/// a command the engine accepts that the model refuses with this word.
/// (D-73: every reversal is dated on or after its original; another unit
/// is adding it to the engine. Once it lands the engine refuses first and
/// this is never reached, then it can go.)
const KNOWN_GAPS: [&str; 1] = ["reversal_before_original"];

impl Runner {
    fn new(chart: &[(u32, AccountType)]) -> Runner {
        let mut l = Ledger::new(Currency::new("USD").unwrap());
        let mut ids = BTreeMap::new();
        for &(code, t) in chart {
            let id = l
                .add_account(&code.to_string(), &account_name(code), t, None, false)
                .unwrap_or_else(|e| panic!("account {code}: {e}"));
            ids.insert(code, id);
        }
        let customer = l.add_customer("Ridgeway Co").expect("a customer");
        Runner {
            l,
            ids,
            customer,
            steps: Vec::new(),
            reversed: BTreeSet::new(),
            closes: BTreeSet::new(),
            tb_after: Vec::new(),
        }
    }

    fn chart_codes(&self) -> Vec<u32> {
        self.ids.keys().copied().collect()
    }

    fn tb(&self, as_of: NaiveDate) -> Vec<(u32, Decimal)> {
        let tb = trial_balance_as_of(&self.l, as_of);
        assert!(tb.balanced(), "the trial balance balances");
        self.chart_codes()
            .into_iter()
            .map(|c| {
                let code = c.to_string();
                let v: Decimal = tb
                    .lines
                    .iter()
                    .filter(|t| t.code == code)
                    .map(|t| t.debit - t.credit)
                    .sum();
                (c, v)
            })
            .collect()
    }

    /// Apply one command; `Err` if it names an event that posted no entry
    /// (a sequence the example format cannot say).
    fn apply(&mut self, cmd: &Cmd) -> Result<(), String> {
        let n = self.steps.len();
        let mut probe = self.l.clone();
        let usd = Currency::new("USD").unwrap();
        let result: Result<Option<EntryId>, LedgerError> = match cmd {
            Cmd::Entry { date, lines } => (|| {
                let e = probe.try_create_entry(*date, &format!("event {}", n + 1))?;
                for line in lines {
                    let account = self.ids.get(&line.code).copied().unwrap_or(UNKNOWN_ID);
                    let side = if line.debit {
                        Side::Debit
                    } else {
                        Side::Credit
                    };
                    let amount = Money::new(Decimal::from_str(&line.amount).unwrap(), usd);
                    probe.add_line(e, account, side, amount)?;
                }
                probe.post_entry(e)?;
                Ok(Some(e))
            })(),
            Cmd::Reverse { of, date } => {
                let original = self
                    .steps
                    .get(*of)
                    .and_then(|s| s.entry)
                    .ok_or_else(|| format!("event {} posted no entry to reverse", of + 1))?;
                probe
                    .reverse_entry(original, *date, &format!("event {}", n + 1))
                    .map(Some)
            }
            Cmd::Close { start, end } => (|| {
                let p = probe.create_period(&format!("event {}", n + 1), *start, *end)?;
                probe.close_period(p)
            })(),
            Cmd::Invoice { date, cents } => (|| {
                let line = DocumentLine {
                    description: format!("event {}", n + 1),
                    amount: Decimal::new(*cents, 2),
                    class_id: None,
                    account: None,
                    quantity: None,
                    unit_price: None,
                };
                let id = probe.create_invoice(self.customer, *date, *date, vec![line])?;
                Ok(probe
                    .invoices()
                    .into_iter()
                    .find(|i| i.id == id)
                    .map(|i| i.entry_id))
            })(),
        };
        let step = match result {
            Err(e) => Step {
                outcome: Outcome::Refused(lean_word(&e)),
                entry: None,
            },
            Ok(entry) => {
                // A known gap: accepted here, refused by the model; roll
                // back and say so.
                let gap = match (cmd, entry) {
                    (Cmd::Reverse { of, date }, Some(_)) => {
                        let original = self.steps[*of].entry.unwrap();
                        (*date < self.l.get_entry(original).unwrap().date())
                            .then_some(KNOWN_GAPS[0])
                    }
                    _ => None,
                };
                match gap {
                    Some(word) => Step {
                        outcome: Outcome::KnownGap(word),
                        entry: None,
                    },
                    None => {
                        self.l = probe;
                        match cmd {
                            Cmd::Reverse { of, .. } => {
                                self.reversed.insert(*of);
                            }
                            Cmd::Close { .. } if entry.is_some() => {
                                self.closes.insert(n);
                            }
                            _ => {}
                        }
                        Step {
                            outcome: Outcome::Posted,
                            entry,
                        }
                    }
                }
            }
        };
        self.steps.push(step);
        let tb = self.tb(far());
        self.tb_after.push(tb);
        Ok(())
    }
}

/// A sequence run through the engine from the start, or why it cannot be
/// (a shrunk candidate whose reversal lost its original).
fn run_rust(seq: &Seq) -> Result<Runner, String> {
    let mut r = Runner::new(&seq.chart);
    for c in &seq.cmds {
        r.apply(c)?;
    }
    Ok(r)
}

// ---- generating sequences ------------------------------------------------

/// A small deterministic generator (SplitMix64): the run is a function of
/// its seed, so a failure replays, and no dependency is added for it.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// 0..n
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    /// lo..=hi
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next() % (hi - lo + 1) as u64) as i64
    }
    /// True `pct` times in a hundred.
    fn chance(&mut self, pct: usize) -> bool {
        self.below(100) < pct
    }
    fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[self.below(xs.len())]
    }
}

/// The days sequences like to use: month starts and ends over two years,
/// so entries, periods and reversals meet on them.
fn key_days() -> Vec<NaiveDate> {
    let mut out = Vec::new();
    for y in [2025, 2026] {
        for m in 1..=12u32 {
            out.push(day(y, m, 1));
            let next = if m == 12 {
                day(y + 1, 1, 1)
            } else {
                day(y, m + 1, 1)
            };
            out.push(next.pred_opt().unwrap());
        }
    }
    out
}

fn any_day(rng: &mut Rng) -> NaiveDate {
    if rng.chance(60) {
        let days = key_days();
        rng.pick(&days)
    } else {
        day(2025, 1, 1) + chrono::Duration::days(rng.range(0, 729))
    }
}

/// A period: a calendar span the books like, any two days (sorted, or
/// inverted), or one day.
fn any_period(rng: &mut Rng) -> (NaiveDate, NaiveDate) {
    let palette = [
        (day(2025, 1, 1), day(2025, 12, 31)),
        (day(2026, 1, 1), day(2026, 12, 31)),
        (day(2025, 1, 1), day(2025, 6, 30)),
        (day(2025, 7, 1), day(2025, 12, 31)),
        (day(2025, 7, 1), day(2026, 6, 30)),
        (day(2026, 1, 1), day(2026, 3, 31)),
        (day(2026, 4, 1), day(2026, 6, 30)),
        (day(2026, 7, 1), day(2026, 9, 30)),
        (day(2025, 1, 1), day(2025, 1, 31)),
        (day(2025, 2, 1), day(2025, 2, 28)),
        (day(2025, 12, 1), day(2025, 12, 31)),
        (day(2026, 1, 1), day(2026, 1, 31)),
        (day(2026, 2, 1), day(2026, 2, 28)),
        (day(2026, 3, 1), day(2026, 3, 31)),
        (day(2026, 12, 1), day(2026, 12, 31)),
    ];
    match rng.below(100) {
        0..=54 => rng.pick(&palette),
        55..=84 => {
            let (a, b) = (any_day(rng), any_day(rng));
            (a.min(b), a.max(b))
        }
        85..=91 => (any_day(rng), any_day(rng)), // maybe inverted
        _ => {
            let d = any_day(rng);
            (d, d)
        }
    }
}

fn gen_chart(rng: &mut Rng) -> Vec<(u32, AccountType)> {
    let mut chart = ROLE_ACCOUNTS.to_vec();
    chart.push((1000, AccountType::Asset));
    chart.push((3000, AccountType::Equity));
    for a in OPTIONAL_ACCOUNTS {
        if rng.chance(60) {
            chart.push(a);
        }
    }
    chart.sort_by_key(|a| a.0);
    chart
}

/// An amount in cents, of any size from a cent to some thousands of
/// dollars.
fn any_cents(rng: &mut Rng) -> i64 {
    match rng.below(3) {
        0 => rng.range(1, 100),
        1 => rng.range(1, 10_000),
        _ => rng.range(1, 500_000),
    }
}

fn gen_entry(rng: &mut Rng, chart: &[(u32, AccountType)]) -> Cmd {
    let postable: Vec<u32> = chart
        .iter()
        .map(|a| a.0)
        .filter(|c| *c != 1200 && *c != 2000)
        .collect();
    // The control accounts (receivable and payable) refuse a hand line
    // the model has no guard for ([`UNCHECKED_BY_THE_MODEL`]): rarely.
    let account = |rng: &mut Rng| {
        if rng.chance(3) {
            rng.pick(&[1200, 2000])
        } else {
            rng.pick(&postable)
        }
    };
    // A balanced entry: debits and credits each split from one total.
    let total = any_cents(rng);
    let split = |rng: &mut Rng, parts: usize| -> Vec<i64> {
        let mut left = total;
        let mut out = Vec::new();
        for i in 0..parts {
            let part = if i + 1 == parts || left <= 1 {
                left
            } else {
                rng.range(1, left - 1)
            };
            if part > 0 {
                out.push(part);
            }
            left -= part;
        }
        out
    };
    let (nd, nc) = (rng.range(1, 2) as usize, rng.range(1, 2) as usize);
    let mut lines: Vec<(u32, bool, i64)> = Vec::new();
    for c in split(rng, nd) {
        lines.push((account(rng), true, c));
    }
    for c in split(rng, nc) {
        lines.push((account(rng), false, c));
    }
    // One fault at most, now and then.
    let mut texts: Vec<L> = Vec::new();
    for &(code, debit, c) in &lines {
        texts.push(L {
            code,
            debit,
            amount: cents(c),
        });
    }
    // A fault now and then, and now and then a second one, so that a
    // command failing two checks shows which the engine and the model
    // reach first.
    let faults = if rng.chance(22) {
        1 + usize::from(rng.chance(30))
    } else {
        0
    };
    for _ in 0..faults {
        let i = rng.below(texts.len());
        match rng.below(8) {
            0 => texts[i].amount = cents(rng.range(1, 500_000)), // likely unbalanced
            1 => {
                if !texts[i].amount.starts_with('-') {
                    texts[i].amount = format!("-{}", texts[i].amount);
                }
            }
            2 => texts[i].amount = "0.00".into(),
            3 => texts[i].amount.push('5'), // a tenth of a cent too fine
            4 => texts[i].code = UNKNOWN_CODE,
            5 => texts.truncate(1),
            _ => {
                let extra = L {
                    code: account(rng),
                    debit: rng.chance(50),
                    amount: cents(any_cents(rng)),
                };
                texts.push(extra);
            }
        }
    }
    Cmd::Entry {
        date: any_day(rng),
        lines: texts,
    }
}

fn gen_cmd(rng: &mut Rng, chart: &[(u32, AccountType)], run: &Runner) -> Cmd {
    let posted: Vec<usize> = run
        .steps
        .iter()
        .enumerate()
        .filter(|(_, s)| s.entry.is_some())
        .map(|(i, _)| i)
        .collect();
    match rng.below(100) {
        0..=63 => gen_entry(rng, chart),
        64..=69 => Cmd::Invoice {
            date: any_day(rng),
            cents: any_cents(rng),
        },
        70..=87 if !posted.is_empty() => {
            // A reversal: of anything posted, now and then of what is
            // already reversed or of a closing entry.
            let reversed: Vec<usize> = run.reversed.iter().copied().collect();
            let closes: Vec<usize> = run.closes.iter().copied().collect();
            let of = if !reversed.is_empty() && rng.chance(20) {
                rng.pick(&reversed)
            } else if !closes.is_empty() && rng.chance(20) {
                rng.pick(&closes)
            } else {
                rng.pick(&posted)
            };
            let original = run
                .l
                .get_entry(run.steps[of].entry.unwrap())
                .unwrap()
                .date();
            let date = match rng.below(100) {
                0..=49 => original + chrono::Duration::days(rng.range(1, 60)),
                50..=69 => original,
                70..=84 => original - chrono::Duration::days(rng.range(1, 400)),
                _ => any_day(rng),
            };
            Cmd::Reverse { of, date }
        }
        _ => {
            let (start, end) = any_period(rng);
            Cmd::Close { start, end }
        }
    }
}

fn gen_seq(rng: &mut Rng) -> Seq {
    let chart = gen_chart(rng);
    let mut run = Runner::new(&chart);
    let mut cmds = Vec::new();
    for _ in 0..rng.range(3, 14) {
        let c = gen_cmd(rng, &chart, &run);
        run.apply(&c).expect("a generated command applies");
        cmds.push(c);
    }
    let tb_days = (0..rng.range(1, 3))
        .map(|_| any_day(rng))
        .chain([far()])
        .collect();
    let balance_days = (0..rng.range(1, 3)).map(|_| any_day(rng)).collect();
    let mut pl_ranges = vec![(day(2025, 1, 1), day(2026, 12, 31))];
    for _ in 0..rng.range(1, 3) {
        let (a, b) = (any_day(rng), any_day(rng));
        pl_ranges.push((a.min(b), a.max(b)));
    }
    Seq {
        chart,
        cmds,
        tb_days,
        balance_days,
        pl_ranges,
    }
}

// ---- writing the example --------------------------------------------------

/// An engine figure as an example writes it: exact cents.
fn fig(d: Decimal) -> String {
    assert_eq!(d.round_dp(2), d, "an engine figure finer than a cent: {d}");
    if d.is_zero() {
        "0.00".to_string()
    } else {
        format!("{:.2}", d.round_dp(2))
    }
}

/// The word an event is written as refused with. A word the checker
/// lists is written as it is; one the model has but the checker does not
/// list is probed ([`PROBE`]); one the model does not have is written as
/// it is and skipped ([`UNCHECKED_BY_THE_MODEL`]).
const PROBE: &str = "finer_than_minor_unit";

/// The words the checker answers (`refusalWords` in `Exec.lean`).
const CHECKED_WORDS: [&str; 10] = [
    "too_few_lines",
    "negative_amount",
    "zero_amount",
    "unbalanced",
    "unknown_account",
    "already_reversed",
    "closing_entry_irreversible",
    "reversal_before_original",
    "period_closed",
    "finer_than_minor_unit",
];

/// Refusals the engine makes that the model has no branch for, so the
/// checker skips an event refused with one (`the model does not check
/// {word}`): the allow-list of skips.
const UNCHECKED_BY_THE_MODEL: [&str; 1] = [
    // BS-arap-2, the control guard: a hand line on a receivable or payable
    // account is refused. `Step.lean`: "No command here can post a hand
    // line on a control account ... so `untagged_control_line` has no
    // branch in this file."
    "untagged_control_line",
];

/// Disagreements of *word* only: the engine and the model both refuse the
/// command (so the books are the same after it), and name different
/// checks, because a command that fails two checks is refused for the
/// first the implementation reaches and the spec orders none. Each is a
/// finding for the owner to rule on (the order of checks); pinned as an
/// `#[ignore]`d test below, which asserts the words are equal and fails
/// until one side is changed. (engine word, model word)
const KNOWN_WORD_DIFFERENCES: [(&str, &str); 10] = [
    // D-A. Reversing a closing entry on a day inside the closed period:
    // the engine checks that a closing entry is never reversed (BS-ledger-6)
    // first, the model checks the closed period (BS-period-2) first.
    ("closing_entry_irreversible", "period_closed"),
    // D-D. Reversing an entry that is already reversed, on a day inside a
    // closed period: the same order, the entry's checks before the date's
    // in the engine, the closed period first in the model.
    ("already_reversed", "period_closed"),
    // D-E. An entry with an unknown account and a second fault: the engine
    // checks each line's account as the line is added, before the entry
    // is posted, so it names the account; the model checks an entry's
    // line count, amounts and balance first (`postEntry`), then its
    // accounts. (The last pair is D-B's reader again: the amount is read
    // before any command.)
    ("unknown_account", "too_few_lines"),
    ("unknown_account", "negative_amount"),
    ("unknown_account", "unbalanced"),
    ("unknown_account", "finer_than_minor_unit"),
    ("unknown_account", "zero_amount"),
    // D-F. An entry with a zero line and a negative one: the engine names
    // the first offending line, in the order the lines were added; the
    // model names a negative amount whenever there is one (`postEntry`).
    ("zero_amount", "negative_amount"),
    // D-B. An entry with an amount finer than the minor unit, dated in a
    // closed period: the engine looks at the date when the entry is begun,
    // the example reader reads every amount before any command runs, so
    // the model's refusal is the amount's.
    ("period_closed", "finer_than_minor_unit"),
    // D-C. A close whose last day is inside another closed period: the
    // engine's closing entry is refused as dated in a closed period
    // (BS-period-2's word); the model names its own check, with no entry
    // involved (`closed_period_contains_end`, a word the spec does not
    // have).
    ("period_closed", "closed_period_contains_end"),
];

/// The word an event is written as refused with, and whether it is a probe.
fn written_word(word: &str) -> (String, bool) {
    if CHECKED_WORDS.contains(&word) || UNCHECKED_BY_THE_MODEL.contains(&word) {
        (word.to_string(), false)
    } else {
        (PROBE.to_string(), true)
    }
}

fn example_id(case: usize) -> String {
    format!("lean-model.case-{case:04}")
}

fn side_figures(out: &mut String, keys: impl IntoIterator<Item = (String, String)>) {
    for (k, v) in keys {
        writeln!(out, "\"{k}\" = \"{v}\"").unwrap();
    }
}

fn toml_of(case: usize, seq: &Seq, run: &Runner) -> String {
    let mut s = String::new();
    let id = example_id(case);
    writeln!(s, "id = \"{id}\"\ntitle = \"random sequence {case}\"").unwrap();
    writeln!(
        s,
        "rule = \"The Rust ledger and the Lean model answer a command sequence alike.\""
    )
    .unwrap();
    writeln!(
        s,
        "status = \"CONVENTION\"\nsources = [\"lean_model.rs\"]\ntags = [\"accrual\"]"
    )
    .unwrap();
    writeln!(
        s,
        "\n[setup]\ncurrency = \"USD\"\nfiscal_year_start = \"01-01\"\nbasis = \"accrual\"\nrounding = \"half-even\""
    )
    .unwrap();
    for &(code, t) in &seq.chart {
        writeln!(
            s,
            "\n[[setup.accounts]]\ncode = \"{code}\"\nname = \"{}\"\ntype = \"{}\"",
            account_name(code),
            type_word(t)
        )
        .unwrap();
    }
    for (i, (cmd, step)) in seq.cmds.iter().zip(&run.steps).enumerate() {
        writeln!(s, "\n[[events]]  # event {}", i + 1).unwrap();
        let refused = match &step.outcome {
            Outcome::Posted => None,
            Outcome::Refused(w) => Some(written_word(w)),
            Outcome::KnownGap(w) => Some((w.to_string(), false)),
        };
        match cmd {
            Cmd::Entry { date, lines } => {
                writeln!(s, "date = \"{date}\"\ndescription = \"event {}\"", i + 1).unwrap();
                if let Some((w, _)) = &refused {
                    writeln!(s, "refused = \"{w}\"").unwrap();
                }
                writeln!(s, "lines = [").unwrap();
                for l in lines {
                    writeln!(
                        s,
                        "  {{ account = \"{}\", side = \"{}\", amount = \"{}\" }},",
                        l.code,
                        if l.debit { "debit" } else { "credit" },
                        l.amount
                    )
                    .unwrap();
                }
                writeln!(s, "]").unwrap();
            }
            Cmd::Reverse { of, date } => {
                writeln!(s, "date = \"{date}\"\ndescription = \"event {}\"", i + 1).unwrap();
                if let Some((w, _)) = &refused {
                    writeln!(s, "refused = \"{w}\"").unwrap();
                }
                writeln!(s, "reverses = {}", of + 1).unwrap();
            }
            Cmd::Invoice {
                date,
                cents: amount,
            } => {
                writeln!(s, "date = \"{date}\"\ndescription = \"event {}\"", i + 1).unwrap();
                if let Some((w, _)) = &refused {
                    writeln!(s, "refused = \"{w}\"").unwrap();
                }
                writeln!(
                    s,
                    "invoice = {{ customer = \"Ridgeway Co\", amount = \"{}\" }}",
                    cents(*amount)
                )
                .unwrap();
            }
            Cmd::Close { start, end } => {
                writeln!(s, "date = \"{end}\"\ndescription = \"event {}\"", i + 1).unwrap();
                if let Some((w, _)) = &refused {
                    writeln!(s, "refused = \"{w}\"").unwrap();
                }
                writeln!(
                    s,
                    "close = {{ start = \"{start}\", to = \"retained_earnings\" }}"
                )
                .unwrap();
            }
        }
        // The trial balance after the event, every account.
        writeln!(
            s,
            "\n[[expect]]\nquery = \"trial_balance\"\nas_of = \"{}\"\nafter = {}\n[expect.values]",
            far(),
            i + 1
        )
        .unwrap();
        side_figures(
            &mut s,
            run.tb_after[i]
                .iter()
                .map(|(c, v)| (c.to_string(), fig(*v))),
        );
    }
    // The final books.
    let codes = run.chart_codes();
    for &d in &seq.tb_days {
        writeln!(
            s,
            "\n[[expect]]\nquery = \"trial_balance\"\nas_of = \"{d}\"\n[expect.values]"
        )
        .unwrap();
        side_figures(
            &mut s,
            run.tb(d).into_iter().map(|(c, v)| (c.to_string(), fig(v))),
        );
    }
    for &d in &seq.balance_days {
        writeln!(
            s,
            "\n[[expect]]\nquery = \"balances\"\nas_of = \"{d}\"\n[expect.values]"
        )
        .unwrap();
        let tb = trial_balance_as_of(&run.l, d);
        let figures = seq.chart.iter().map(|&(c, t)| {
            let code = c.to_string();
            let (dr, cr) = tb
                .lines
                .iter()
                .find(|l| l.code == code)
                .map_or((Decimal::ZERO, Decimal::ZERO), |l| (l.debit, l.credit));
            let v = match t.normal_side() {
                Side::Debit => dr - cr,
                Side::Credit => cr - dr,
            };
            (code, fig(v))
        });
        side_figures(&mut s, figures);
    }
    for &(from, to) in &seq.pl_ranges {
        writeln!(
            s,
            "\n[[expect]]\nquery = \"profit_and_loss\"\nfrom = \"{from}\"\nto = \"{to}\"\n[expect.values]"
        )
        .unwrap();
        let pl = profit_and_loss_for_period(&run.l, from, to, None)
            .unwrap_or_else(|e| panic!("profit and loss {from}..{to}: {e}"));
        let mut figures = vec![("net_income".to_string(), fig(pl.net_income()))];
        for &(c, t) in &seq.chart {
            if matches!(t, AccountType::Revenue | AccountType::Expense) {
                let code = c.to_string();
                let v: Decimal = pl
                    .revenue
                    .iter()
                    .chain(&pl.expenses)
                    .filter(|l| l.code == code)
                    .map(|l| l.amount)
                    .sum();
                figures.push((code, fig(v)));
            }
        }
        side_figures(&mut s, figures);
    }
    for (i, step) in run.steps.iter().enumerate() {
        let Some(entry) = step.entry else { continue };
        writeln!(
            s,
            "\n[[expect]]\nquery = \"entry\"\nentry = {}\n[expect.values]",
            i + 1
        )
        .unwrap();
        let mut got: BTreeMap<String, Decimal> = BTreeMap::new();
        for c in &codes {
            got.insert(format!("{c}.debit"), Decimal::ZERO);
            got.insert(format!("{c}.credit"), Decimal::ZERO);
        }
        for line in run.l.get_entry(entry).unwrap().lines() {
            let code = &run.l.chart().get(line.account).unwrap().code;
            let side = match line.side {
                Side::Debit => "debit",
                Side::Credit => "credit",
            };
            *got.get_mut(&format!("{code}.{side}")).unwrap() += line.amount.amount;
        }
        side_figures(&mut s, got.into_iter().map(|(k, v)| (k, fig(v))));
    }
    s
}

// ---- the Lean side --------------------------------------------------------

/// A `bookspec-check` ready to run: a copy of the pinned bookspec's
/// `lean/` directory, built.
struct Lean {
    dir: PathBuf,
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if e.file_name() == ".lake" {
            continue;
        }
        if e.file_type().unwrap().is_dir() {
            copy_dir(&src, &dst);
        } else {
            std::fs::copy(&src, &dst).unwrap();
        }
    }
}

/// The pinned bookspec rev, from this crate's `Cargo.toml`.
fn pinned_rev() -> String {
    let manifest =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
            .expect("Cargo.toml");
    let line = manifest
        .lines()
        .find(|l| l.starts_with("bookspec-checker"))
        .expect("the bookspec-checker pin");
    let rev = line
        .split("rev = \"")
        .nth(1)
        .expect("a rev")
        .split('"')
        .next()
        .unwrap();
    rev.to_string()
}

fn require_lean() -> bool {
    std::env::var_os("WANDERBOOKS_REQUIRE_LEAN").is_some_and(|v| !v.is_empty() && v != "0")
}

/// The Lean model, or `None` (skipped) when `lake` is not installed.
fn lean() -> Option<&'static Lean> {
    static LEAN: std::sync::OnceLock<Option<Lean>> = std::sync::OnceLock::new();
    LEAN.get_or_init(|| {
        match Command::new("lake").arg("--version").output() {
            Err(_) => {
                assert!(
                    !require_lean(),
                    "WANDERBOOKS_REQUIRE_LEAN is set, so Lean must run here, but lake is not installed"
                );
                eprintln!("lake is not installed: the Lean model is not run here");
                return None;
            }
            Ok(v) => assert!(
                v.status.success(),
                "`lake` is on the path but cannot run (exit {:?}): `mise install` in the repository root",
                v.status.code()
            ),
        }
        let dir = match std::env::var_os("WANDERBOOKS_BOOKSPEC_LEAN") {
            Some(d) => PathBuf::from(d),
            None => {
                let rev = pinned_rev();
                let home = std::env::var_os("CARGO_HOME")
                    .map(PathBuf::from)
                    .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo")))
                    .expect("CARGO_HOME or HOME");
                let checkouts = home.join("git/checkouts");
                let source = std::fs::read_dir(&checkouts)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|repo| repo.path().join(&rev[..7]).join("lean"))
                    .find(|p| p.join("lakefile.toml").exists())
                    .unwrap_or_else(|| {
                        panic!(
                            "the pinned bookspec {rev} is not checked out under {} (cargo fetches it with the dev-dependencies)",
                            checkouts.display()
                        )
                    });
                // Built in a copy under target/, not in cargo's checkout.
                let target = std::env::var_os("CARGO_TARGET_DIR")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"));
                let dir = target.join("lean-model").join(&rev[..7]).join("lean");
                if !dir.join("lakefile.toml").exists() {
                    copy_dir(&source, &dir);
                }
                dir
            }
        };
        let build = Command::new("lake")
            .args(["build", "bookspec-check"])
            .current_dir(&dir)
            .output()
            .expect("lake runs");
        assert!(
            build.status.success(),
            "lake build bookspec-check failed in {}:\n{}{}",
            dir.display(),
            String::from_utf8_lossy(&build.stdout),
            String::from_utf8_lossy(&build.stderr)
        );
        Some(Lean { dir })
    })
    .as_ref()
}

/// One item the model reported (`I` lines of `--machine`).
#[derive(Debug)]
struct Item {
    id: String,
    kind: String,
    what: String,
    status: String,
    detail: String,
}

struct Report {
    /// Per example: its outcome word.
    examples: BTreeMap<String, String>,
    items: Vec<Item>,
}

impl Lean {
    /// Run the model on `cases` (id and text), all in one directory, in
    /// one `lake` call.
    fn run(&self, cases: &[(String, String)]) -> Report {
        let tmp = tempfile::tempdir().unwrap();
        for (id, text) in cases {
            std::fs::write(tmp.path().join(format!("{id}.toml")), text).unwrap();
        }
        let out = Command::new("lake")
            .args(["exe", "bookspec-check", "--machine"])
            .arg(tmp.path())
            .current_dir(&self.dir)
            .output()
            .expect("lake runs");
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.code().is_some_and(|c| c <= 1),
            "bookspec-check failed ({:?}):\n{text}\n{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
        let mut r = Report {
            examples: BTreeMap::new(),
            items: Vec::new(),
        };
        for line in text.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            match f.as_slice() {
                ["E", id, status, _] => {
                    r.examples.insert(id.to_string(), status.to_string());
                }
                ["I", id, kind, what, status, detail] => r.items.push(Item {
                    id: id.to_string(),
                    kind: kind.to_string(),
                    what: what.to_string(),
                    status: status.to_string(),
                    detail: detail.to_string(),
                }),
                _ => {}
            }
        }
        assert_eq!(
            r.examples.len(),
            cases.len(),
            "the model reports every example:\n{text}"
        );
        r
    }
}

// ---- reading the model's answers ------------------------------------------

/// How one example came out.
#[derive(Default)]
struct Verdict {
    /// Disagreements: what was asked, the engine's answer, the model's.
    disagreements: Vec<String>,
    ok: usize,
    /// Refusals the model gave in the words the engine did (probes).
    probes_agreed: usize,
    /// Refusals that differ in word only, in [`KNOWN_WORD_DIFFERENCES`].
    known_words: Vec<(String, String)>,
    skipped: Vec<String>,
}

/// Judge the model's items for one example against what the engine did.
fn judge(seq: &Seq, run: &Runner, id: &str, items: &[&Item]) -> Verdict {
    let mut v = Verdict::default();
    for it in items {
        let event: Option<usize> = (it.kind == "event")
            .then(|| {
                it.what
                    .split_whitespace()
                    .nth(1)
                    .and_then(|n| n.parse().ok())
            })
            .flatten();
        match (it.status.as_str(), event) {
            ("ok", _) => v.ok += 1,
            ("skipped", Some(n)) => {
                let reason = &it.detail;
                let word = match &run.steps[n - 1].outcome {
                    Outcome::Refused(w) => w.clone(),
                    _ => String::new(),
                };
                if UNCHECKED_BY_THE_MODEL.contains(&word.as_str())
                    && *reason == format!("the model does not check {word}")
                {
                    v.skipped.push(reason.clone());
                } else {
                    v.disagreements.push(format!(
                        "{id} {}: an unexpected skip ({reason}); the engine: {:?}",
                        it.what,
                        run.steps[n - 1].outcome
                    ));
                }
            }
            ("mismatch", Some(n)) => {
                let step = &run.steps[n - 1];
                let engine = match &step.outcome {
                    Outcome::Refused(w) => Some(w.clone()),
                    _ => None,
                };
                // "expected refusal W, got G": the model refused, in its
                // own word G. For a probe W is the probe; otherwise the
                // engine's word.
                let got = it
                    .detail
                    .strip_prefix("expected refusal ")
                    .and_then(|r| r.split_once(", got "))
                    .map(|(_, g)| g.split(' ').next().unwrap().to_string());
                match (&engine, &got) {
                    (Some(e), Some(g))
                        if it.what.ends_with(&format!("refused {PROBE}")) && e == g =>
                    {
                        v.probes_agreed += 1
                    }
                    (Some(e), Some(g))
                        if KNOWN_WORD_DIFFERENCES.contains(&(e.as_str(), g.as_str())) =>
                    {
                        v.known_words.push((e.clone(), g.clone()))
                    }
                    _ => v.disagreements.push(format!(
                        "{id} {}: the engine {:?}; the model: {}",
                        it.what, step.outcome, it.detail
                    )),
                }
            }
            _ => v
                .disagreements
                .push(format!("{id} {}: {} ({})", it.what, it.status, it.detail)),
        }
    }
    let _ = seq;
    v
}

/// The engine's account of a sequence, for a failure report.
fn describe(seq: &Seq, run: &Runner) -> String {
    let mut s = String::new();
    writeln!(
        s,
        "chart: {:?}",
        seq.chart.iter().map(|a| a.0).collect::<Vec<_>>()
    )
    .unwrap();
    for (i, (c, st)) in seq.cmds.iter().zip(&run.steps).enumerate() {
        writeln!(s, "  event {}: {c:?}\n     engine: {:?}", i + 1, st.outcome).unwrap();
    }
    s
}

// ---- shrinking -------------------------------------------------------------

/// Smaller sequences: one event gone (and the reversals that named it,
/// the rest renumbered), or one line of an entry gone.
fn smaller(seq: &Seq) -> Vec<Seq> {
    let mut out = Vec::new();
    for i in 0..seq.cmds.len() {
        let mut gone: BTreeSet<usize> = [i].into();
        loop {
            let more: Vec<usize> = seq
                .cmds
                .iter()
                .enumerate()
                .filter_map(|(j, c)| match c {
                    Cmd::Reverse { of, .. } if gone.contains(of) && !gone.contains(&j) => Some(j),
                    _ => None,
                })
                .collect();
            if more.is_empty() {
                break;
            }
            gone.extend(more);
        }
        let renumber = |of: usize| of - gone.iter().filter(|g| **g < of).count();
        let cmds = seq
            .cmds
            .iter()
            .enumerate()
            .filter(|(j, _)| !gone.contains(j))
            .map(|(_, c)| match c {
                Cmd::Reverse { of, date } => Cmd::Reverse {
                    of: renumber(*of),
                    date: *date,
                },
                other => other.clone(),
            })
            .collect::<Vec<_>>();
        if !cmds.is_empty() {
            out.push(Seq {
                cmds,
                ..seq.clone()
            });
        }
        if let Cmd::Entry { date, lines } = &seq.cmds[i] {
            for j in 0..lines.len() {
                if lines.len() > 1 {
                    let mut ls = lines.clone();
                    ls.remove(j);
                    let mut cmds = seq.cmds.clone();
                    cmds[i] = Cmd::Entry {
                        date: *date,
                        lines: ls,
                    };
                    out.push(Seq {
                        cmds,
                        ..seq.clone()
                    });
                }
            }
        }
    }
    out
}

/// Check a batch of sequences against the model: each one's verdict.
fn check_batch(lean: &Lean, seqs: &[Seq]) -> Vec<(Runner, Verdict)> {
    let runs: Vec<Runner> = seqs
        .iter()
        .map(|s| run_rust(s).expect("a runnable sequence"))
        .collect();
    let cases: Vec<(String, String)> = seqs
        .iter()
        .zip(&runs)
        .enumerate()
        .map(|(i, (s, r))| (example_id(i), toml_of(i, s, r)))
        .collect();
    let report = lean.run(&cases);
    runs.into_iter()
        .enumerate()
        .map(|(i, run)| {
            let id = example_id(i);
            let items: Vec<&Item> = report.items.iter().filter(|it| it.id == id).collect();
            let verdict = judge(&seqs[i], &run, &id, &items);
            (run, verdict)
        })
        .collect()
}

/// The smallest sequence (by greedy removal, a round of candidates per
/// `lake` call) that still disagrees.
fn shrink(lean: &Lean, mut seq: Seq) -> Seq {
    loop {
        let candidates: Vec<Seq> = smaller(&seq)
            .into_iter()
            .filter(|c| run_rust(c).is_ok())
            .collect();
        if candidates.is_empty() {
            return seq;
        }
        let verdicts = check_batch(lean, &candidates);
        let mut best: Option<usize> = None;
        for (i, (_, v)) in verdicts.iter().enumerate() {
            if !v.disagreements.is_empty()
                && best.is_none_or(|b| candidates[i].cmds.len() < candidates[b].cmds.len())
            {
                best = Some(i);
            }
        }
        match best {
            Some(b) => seq = candidates[b].clone(),
            None => return seq,
        }
    }
}

/// The failure report: the minimal sequence, as an example, with both
/// answers.
fn report_failure(lean: &Lean, seq: &Seq) -> String {
    let (run, verdict) = check_batch(lean, std::slice::from_ref(seq)).remove(0);
    let toml = toml_of(0, seq, &run);
    let mut s = format!(
        "the Rust ledger and the Lean model disagree.\nMinimal sequence:\n{}\n",
        describe(seq, &run)
    );
    for d in &verdict.disagreements {
        writeln!(s, "DISAGREEMENT: {d}").unwrap();
    }
    writeln!(
        s,
        "\nThe example both were given (expected values are the engine's):\n{toml}"
    )
    .unwrap();
    s
}

// ---- the test --------------------------------------------------------------

fn cases() -> usize {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|c| c.parse().ok())
        .unwrap_or(64)
}

fn seed() -> u64 {
    std::env::var("WANDERBOOKS_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0x5EED_1AB5)
}

/// Random command sequences run through the Rust ledger and the Lean
/// model agree on every refusal and every figure.
#[test]
fn the_rust_ledger_and_the_lean_model_agree_on_random_command_sequences() {
    let Some(lean) = lean() else { return };
    let (n, seed) = (cases(), seed());
    let mut rng = Rng(seed);
    let seqs: Vec<Seq> = (0..n).map(|_| gen_seq(&mut rng)).collect();
    let results = check_batch(lean, &seqs);

    // Counts, and the coverage the generator owes.
    let (mut commands, mut entries, mut reversals, mut closes) = (0, 0, 0, 0);
    let mut accepted: BTreeMap<&str, usize> = BTreeMap::new();
    let mut refused: BTreeMap<String, usize> = BTreeMap::new();
    let (mut ok, mut probes, mut gaps) = (0, 0, 0);
    let mut skips: BTreeMap<String, usize> = BTreeMap::new();
    let mut known: BTreeMap<String, usize> = BTreeMap::new();
    for (seq, (run, v)) in seqs.iter().zip(&results) {
        ok += v.ok;
        probes += v.probes_agreed;
        for k in &v.known_words {
            *known
                .entry(format!("engine {} / model {}", k.0, k.1))
                .or_default() += 1;
        }
        for s in &v.skipped {
            *skips.entry(s.clone()).or_default() += 1;
        }
        for (i, (cmd, step)) in seq.cmds.iter().zip(&run.steps).enumerate() {
            commands += 1;
            let kind = match cmd {
                Cmd::Entry { .. } => {
                    entries += 1;
                    "entry"
                }
                Cmd::Reverse { of, .. } => {
                    reversals += 1;
                    if matches!(seq.cmds[*of], Cmd::Reverse { .. }) {
                        *accepted.entry("reversal of a reversal").or_default() +=
                            usize::from(step.outcome == Outcome::Posted);
                    }
                    "reversal"
                }
                Cmd::Invoice { .. } => "invoice",
                Cmd::Close { .. } => {
                    closes += 1;
                    if step.outcome == Outcome::Posted {
                        *accepted
                            .entry(if step.entry.is_some() {
                                "close with an entry"
                            } else {
                                "close with nothing to close"
                            })
                            .or_default() += 1;
                    }
                    "close"
                }
            };
            let _ = i;
            match &step.outcome {
                Outcome::Posted => *accepted.entry(kind).or_default() += 1,
                Outcome::Refused(w) => *refused.entry(w.clone()).or_default() += 1,
                Outcome::KnownGap(w) => {
                    gaps += 1;
                    *refused
                        .entry(format!("{w} (known gap, rolled back)"))
                        .or_default() += 1;
                }
            }
        }
    }
    let bad = results
        .iter()
        .filter(|(_, v)| !v.disagreements.is_empty())
        .count();
    eprintln!(
        "lean_model: seed {seed}: {n} sequences, {commands} commands ({entries} entries, {reversals} reversals, {closes} closes)\n  \
         the model, per item: {ok} ok, {probes} refusals matched by probe, {} skipped (allow-listed), {} mismatched sequences\n  \
         known gaps rolled back: {gaps}\n  refused in different words (known, see KNOWN_WORD_DIFFERENCES): {known:?}\n  accepted: {accepted:?}\n  refused: {refused:?}\n  skips: {skips:?}",
        skips.values().sum::<usize>(),
        bad
    );

    if bad > 0 {
        // Up to six failing sequences, each shrunk; the first in full.
        let failing: Vec<usize> = results
            .iter()
            .enumerate()
            .filter(|(_, (_, v))| !v.disagreements.is_empty())
            .map(|(i, _)| i)
            .collect();
        let mut text = String::new();
        let mut seen = BTreeSet::new();
        // Every disagreement, by kind, before shrinking any.
        let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
        for (_, v) in &results {
            for d in &v.disagreements {
                let kind = d.split_once(" refused ").map_or(d.as_str(), |(_, r)| r);
                let kind = kind.split_once(": ").map_or(kind, |(_, r)| r);
                *kinds.entry(kind.to_string()).or_default() += 1;
            }
        }
        writeln!(text, "all disagreements, by kind: {kinds:#?}").unwrap();
        for (k, &i) in failing.iter().take(6).enumerate() {
            let minimal = shrink(lean, seqs[i].clone());
            let (run, verdict) = check_batch(lean, std::slice::from_ref(&minimal)).remove(0);
            let signature = verdict
                .disagreements
                .join("|")
                .replace("lean-model.case-0000", "");
            if !seen.insert(signature) {
                continue;
            }
            if k == 0 || std::env::var_os("WANDERBOOKS_FULL").is_some() {
                text.push_str(&report_failure(lean, &minimal));
            } else {
                writeln!(text, "\nAnother, shrunk:\n{}", describe(&minimal, &run)).unwrap();
                for d in &verdict.disagreements {
                    writeln!(text, "DISAGREEMENT: {d}").unwrap();
                }
            }
        }
        let dir = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"))
            .join("lean-model");
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("last-failure.txt"), &text);
        panic!(
            "{bad} of {n} sequences (seed {seed}) disagree (sequence numbers {:?}); shrunk:\n{text}",
            failing.iter().take(20).collect::<Vec<_>>()
        );
    }

    // The generator reaches what it claims to, once the run is large
    // enough to be expected to.
    if n >= 64 {
        for word in [
            "too_few_lines",
            "negative_amount",
            "zero_amount",
            "unbalanced",
            "unknown_account",
            "finer_than_minor_unit",
            "already_reversed",
            "closing_entry_irreversible",
            "period_closed",
            "period_overlaps",
            "period_end_before_start",
            "untagged_control_line",
        ] {
            assert!(
                refused.get(word).copied().unwrap_or(0) > 0,
                "the generator never drew {word} in {n} sequences (seed {seed}): {refused:?}"
            );
        }
        for kind in [
            "entry",
            "invoice",
            "reversal",
            "close with an entry",
            "close with nothing to close",
            "reversal of a reversal",
        ] {
            assert!(
                accepted.get(kind).copied().unwrap_or(0) > 0,
                "the generator never had a {kind} accepted in {n} sequences (seed {seed}): {accepted:?}"
            );
        }
    }
}

/// The harness would notice: an expectation the engine did not give is a
/// mismatch, a probe of a word the model does not use is read, and the
/// skip allow-list is enforced.
#[test]
fn the_harness_notices_a_wrong_answer() {
    let Some(lean) = lean() else { return };
    let seq = Seq {
        chart: {
            let mut c = ROLE_ACCOUNTS.to_vec();
            c.push((1000, AccountType::Asset));
            c.push((3000, AccountType::Equity));
            c.sort_by_key(|a| a.0);
            c
        },
        cmds: vec![
            Cmd::Entry {
                date: day(2026, 1, 5),
                lines: vec![
                    L {
                        code: 1000,
                        debit: true,
                        amount: "10.00".into(),
                    },
                    L {
                        code: 3000,
                        debit: false,
                        amount: "10.00".into(),
                    },
                ],
            },
            // A refusal only the probe can read.
            Cmd::Close {
                start: day(2026, 2, 1),
                end: day(2026, 1, 1),
            },
            Cmd::Close {
                start: day(2026, 1, 1),
                end: day(2026, 1, 31),
            },
        ],
        tb_days: vec![far()],
        balance_days: vec![far()],
        pl_ranges: vec![(day(2026, 1, 1), day(2026, 12, 31))],
    };
    let run = run_rust(&seq).unwrap();
    assert_eq!(
        run.steps[1].outcome,
        Outcome::Refused("period_end_before_start".into())
    );
    let good = toml_of(0, &seq, &run);
    // The same example with one figure changed: a cent.
    let wrong = good.replacen("\"1000\" = \"10.00\"", "\"1000\" = \"10.01\"", 1);
    assert_ne!(good, wrong);
    let report = lean.run(&[
        (example_id(0), good),
        (example_id(1), wrong.replace("case-0000", "case-0001")),
    ]);
    let verdicts: Vec<Verdict> = [0, 1]
        .iter()
        .map(|&i| {
            let id = example_id(i);
            let items: Vec<&Item> = report.items.iter().filter(|it| it.id == id).collect();
            judge(&seq, &run, &id, &items)
        })
        .collect();
    assert!(
        verdicts[0].disagreements.is_empty(),
        "{:?}",
        verdicts[0].disagreements
    );
    assert_eq!(verdicts[0].probes_agreed, 1, "the probe was read");
    assert!(
        !verdicts[1].disagreements.is_empty(),
        "a changed figure must be a disagreement"
    );
}

// ---- the disagreements found, pinned ---------------------------------------

fn minimal_chart() -> Vec<(u32, AccountType)> {
    let mut c = ROLE_ACCOUNTS.to_vec();
    c.push((1000, AccountType::Asset));
    c.push((3000, AccountType::Equity));
    c.sort_by_key(|a| a.0);
    c
}

fn entry(date: NaiveDate, lines: &[(u32, bool, &str)]) -> Cmd {
    Cmd::Entry {
        date,
        lines: lines
            .iter()
            .map(|&(code, debit, amount)| L {
                code,
                debit,
                amount: amount.to_string(),
            })
            .collect(),
    }
}

fn pinned_seq(cmds: Vec<Cmd>) -> Seq {
    Seq {
        chart: minimal_chart(),
        cmds,
        tb_days: vec![far()],
        balance_days: vec![far()],
        pl_ranges: vec![(day(2025, 1, 1), day(2026, 12, 31))],
    }
}

/// Each disagreement of word, as the smallest sequence that shows it,
/// with the pair (engine word, model word) it is allow-listed under.
fn pinned() -> Vec<(&'static str, Seq, (&'static str, &'static str))> {
    let spend = |d| entry(d, &[(6999, true, "10.00"), (1000, false, "10.00")]);
    vec![
        (
            "D-A",
            pinned_seq(vec![
                spend(day(2025, 5, 7)),
                Cmd::Close {
                    start: day(2025, 1, 1),
                    end: day(2025, 6, 30),
                },
                Cmd::Reverse {
                    of: 1,
                    date: day(2025, 6, 30),
                },
            ]),
            ("closing_entry_irreversible", "period_closed"),
        ),
        (
            "D-B",
            pinned_seq(vec![
                Cmd::Close {
                    start: day(2026, 1, 1),
                    end: day(2026, 3, 31),
                },
                entry(
                    day(2026, 1, 6),
                    &[(1000, true, "1.005"), (3000, false, "1.005")],
                ),
            ]),
            ("period_closed", "finer_than_minor_unit"),
        ),
        (
            "D-C",
            pinned_seq(vec![
                Cmd::Close {
                    start: day(2026, 2, 1),
                    end: day(2026, 2, 28),
                },
                Cmd::Close {
                    start: day(2025, 10, 13),
                    end: day(2026, 2, 28),
                },
            ]),
            ("period_closed", "closed_period_contains_end"),
        ),
        (
            "D-D",
            pinned_seq(vec![
                spend(day(2026, 1, 10)),
                Cmd::Reverse {
                    of: 0,
                    date: day(2026, 1, 20),
                },
                Cmd::Close {
                    start: day(2026, 1, 1),
                    end: day(2026, 1, 31),
                },
                Cmd::Reverse {
                    of: 0,
                    date: day(2026, 1, 25),
                },
            ]),
            ("already_reversed", "period_closed"),
        ),
        (
            "D-E",
            pinned_seq(vec![entry(
                day(2026, 3, 5),
                &[(UNKNOWN_CODE, true, "0.01")],
            )]),
            ("unknown_account", "too_few_lines"),
        ),
        (
            "D-E",
            pinned_seq(vec![entry(
                day(2026, 3, 5),
                &[(UNKNOWN_CODE, true, "-1.00"), (1000, false, "1.00")],
            )]),
            ("unknown_account", "negative_amount"),
        ),
        (
            "D-E",
            pinned_seq(vec![entry(
                day(2026, 3, 5),
                &[(UNKNOWN_CODE, true, "1.00"), (1000, false, "2.00")],
            )]),
            ("unknown_account", "unbalanced"),
        ),
        (
            "D-E",
            pinned_seq(vec![entry(
                day(2026, 3, 5),
                &[(1000, true, "0.00"), (UNKNOWN_CODE, false, "1.00")],
            )]),
            ("unknown_account", "zero_amount"),
        ),
        (
            "D-F",
            pinned_seq(vec![entry(
                day(2026, 3, 5),
                &[(1000, true, "0.00"), (3000, false, "-1.00")],
            )]),
            ("zero_amount", "negative_amount"),
        ),
    ]
}

/// What the model says of a pinned sequence.
fn pinned_verdict(seq: &Seq) -> Verdict {
    let lean = lean().expect("checked by the caller");
    check_batch(lean, std::slice::from_ref(seq)).remove(0).1
}

/// Every pinned disagreement is still exactly what the allow-list says:
/// both refuse, in the two words, and nothing else differs. When one side
/// is changed to agree, this fails, and the pair comes out of
/// [`KNOWN_WORD_DIFFERENCES`] and the pin out of [`pinned`].
#[test]
fn the_known_differences_are_each_still_what_they_were() {
    if lean().is_none() {
        return;
    }
    for (name, seq, (engine, model)) in pinned() {
        let v = pinned_verdict(&seq);
        assert!(v.disagreements.is_empty(), "{name}: {:?}", v.disagreements);
        assert_eq!(
            v.known_words,
            vec![(engine.to_string(), model.to_string())],
            "{name}: the engine and the model refuse in the same words now?"
        );
    }
}

/// The strict form of a pinned difference: the engine and the model name
/// the same check. Fails until the order of checks is settled.
fn strict(name: &str) {
    if lean().is_none() {
        return;
    }
    for (n, seq, (engine, model)) in pinned().into_iter().filter(|p| p.0 == name) {
        let v = pinned_verdict(&seq);
        assert!(
            v.known_words.is_empty() && v.disagreements.is_empty(),
            "{n}: the engine says {engine}, the model says {model}: {:?} {:?}",
            v.known_words,
            v.disagreements
        );
    }
}

#[test]
#[ignore = "D-A: reversing a closing entry inside its closed period: engine closing_entry_irreversible (BS-ledger-6), model period_closed (BS-period-2); the spec orders neither"]
fn d_a_a_closing_entry_is_irreversible_before_the_period_is_closed() {
    strict("D-A");
}

#[test]
#[ignore = "D-B: a finer-than-a-cent amount in a closed period: engine period_closed, the example reader finer_than_minor_unit; an artifact of reading amounts before any command"]
fn d_b_the_date_is_checked_before_the_amount() {
    strict("D-B");
}

#[test]
#[ignore = "D-C: closing a period whose last day is in a closed one: engine period_closed (BS-period-2's word), model closed_period_contains_end (a word the spec does not have)"]
fn d_c_a_close_inside_a_closed_period_is_period_closed() {
    strict("D-C");
}

#[test]
#[ignore = "D-D: reversing an already reversed entry on a closed day: engine already_reversed, model period_closed; the spec orders neither"]
fn d_d_already_reversed_before_the_period_is_closed() {
    strict("D-D");
}

#[test]
#[ignore = "D-E: an unknown account with a second fault: engine unknown_account first (at add_line), model its line count, amounts and balance first; the spec orders none"]
fn d_e_the_account_is_checked_before_the_entry() {
    strict("D-E");
}

#[test]
#[ignore = "D-F: a zero line and a negative line in one entry: engine zero_amount (the first offending line), model negative_amount (a negative one anywhere); the spec orders neither"]
fn d_f_the_first_offending_line_is_named() {
    strict("D-F");
}
