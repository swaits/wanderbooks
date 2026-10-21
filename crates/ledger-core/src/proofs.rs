//! Kani proof harnesses for the accounting kernel.
//!
//! `cargo kani -p ledger-core` (or `just prove`) model-checks each
//! harness: CBMC explores *every* input within the stated bounds, not a
//! sample, and fails if any one of them breaks an assertion, panics, or
//! overflows. The code under proof is the engine's own [`crate::kernel`],
//! instantiated at `i64` (exact integer money, in the smallest unit; the
//! four charge harnesses of the proration by days at `i32`); the engine
//! runs the same functions on `Decimal` (proration on `i128`), and the
//! `kernel_bridge` property tests check the instantiations agree.
//!
//! Bounds: entries of up to [`N`] lines over up to [`K`] accounts, and
//! amounts up to 2^40 in magnitude; the statement of cash flows' entry
//! harnesses, up to [`NF`] lines, each in any of the four places;
//! proration over up to [`PR`] month parts, 1 to 3 months, totals up to
//! [`PR_TOTAL`] units (the charges: 1 or 2 months, totals to 2,048 units,
//! 64 in a span's last month). The Lean model in `verify/lean` proves the
//! same laws for any number of lines and accounts, and the proration's
//! for any number of parts and any sizes (docs/verification.md says
//! which, and how it ties to the kernel's formula).

use crate::kernel::{self, Amount, LineError, MONTH_L, MonthPart};
use crate::{AccountType, Side};

/// Most lines per entry the harnesses explore.
const N: usize = 4;
/// Accounts the harnesses' lines draw from.
const K: u8 = 3;

fn any_side() -> Side {
    if kani::any() {
        Side::Debit
    } else {
        Side::Credit
    }
}

fn any_type() -> AccountType {
    // An assumption, not `% 5`: modulo is costly for the SAT solver.
    let t: u8 = kani::any();
    kani::assume(t < 5);
    match t {
        0 => AccountType::Asset,
        1 => AccountType::Liability,
        2 => AccountType::Equity,
        3 => AccountType::Revenue,
        _ => AccountType::Expense,
    }
}

/// Any amount a journal line could carry, sign included.
fn any_amount() -> i64 {
    let a: i64 = kani::any();
    kani::assume(a.in_line_range());
    a
}

/// Any count up to `N`. Harnesses fill fixed arrays and take a prefix of
/// this length: fixed arrays keep the model free of allocation (which is
/// also why they call `movements_into` rather than `movements`).
fn any_len() -> usize {
    let n: usize = kani::any();
    kani::assume(n <= N);
    n
}

/// `N` lines, any sides, any in-range amounts (use a prefix).
fn any_lines() -> [(Side, i64); N] {
    core::array::from_fn(|_| (any_side(), any_amount()))
}

/// `N` lines over `K` accounts (use a prefix).
fn any_account_lines() -> [(u8, Side, i64); N] {
    core::array::from_fn(|_| {
        let k: u8 = kani::any();
        kani::assume(k < K);
        (k, any_side(), any_amount())
    })
}

/// The mathematical signed amount. Amounts are at most 2^40 and no
/// harness sums more than eight, so `i64` holds every spec sum exactly
/// (and Kani would flag it if not).
fn wide(side: Side, amount: i64) -> i64 {
    match side {
        Side::Debit => amount,
        Side::Credit => -(amount),
    }
}

fn wide_net(lines: &[(Side, i64)]) -> i64 {
    lines.iter().map(|&(s, a)| wide(s, a)).sum()
}

/// The sides and amounts of some account lines, into a fixed array.
fn sides(lines: &[(u8, Side, i64)]) -> [(Side, i64); N] {
    let mut out = [(Side::Debit, 0); N];
    for (o, &(_, s, a)) in out.iter_mut().zip(lines) {
        *o = (s, a);
    }
    out
}

// ---------------------------------------------------------------------------
// Lines and entries
// ---------------------------------------------------------------------------

/// Flipping a side negates its signed amount, and flipping twice is the
/// identity.
#[kani::proof]
fn flip_negates() {
    let side = any_side();
    let a = any_amount();
    assert_eq!(kernel::flip(kernel::flip(side)), side);
    assert_ne!(kernel::flip(side), side);
    assert_eq!(kernel::signed(kernel::flip(side), a), Some(-wide(side, a)));
}

/// In-range lines never overflow the kernel's sums: `totals` and `net`
/// always succeed, and agree with the mathematical sums. (`movements`
/// is covered by its own harnesses.)
#[kani::proof]
#[kani::unwind(6)]
fn sums_are_exact() {
    let all = any_account_lines();
    let lines = &all[..any_len()];
    let all_sides = sides(lines);
    let s = &all_sides[..lines.len()];
    let (debits, credits) = kernel::totals(s.iter().copied()).expect("no overflow");
    let net = kernel::net(s.iter().copied()).expect("no overflow");
    assert_eq!(net, wide_net(s));
    assert_eq!(debits - credits, wide_net(s));
}

/// The kernel accepts an entry exactly when it has two or more lines,
/// every amount is positive and in range, and debits equal credits.
#[kani::proof]
#[kani::unwind(6)]
fn check_lines_is_exact() {
    let all = any_lines();
    let lines = &all[..any_len()];
    let positive = lines.iter().all(|&(_, a)| a > 0);
    let debits: i64 = lines
        .iter()
        .filter(|l| l.0 == Side::Debit)
        .map(|l| l.1)
        .sum();
    let credits: i64 = lines
        .iter()
        .filter(|l| l.0 == Side::Credit)
        .map(|l| l.1)
        .sum();
    let spec = lines.len() >= 2 && positive && debits == credits;
    let got = kernel::check_lines(lines);
    assert_eq!(got.is_ok(), spec);
    if let Err(LineError::Unbalanced {
        debits: d,
        credits: c,
    }) = got
    {
        assert_eq!((d, c), (debits, credits));
    }
}

/// An out-of-range amount is always rejected.
#[kani::proof]
fn out_of_range_is_rejected() {
    let big: i64 = kani::any();
    kani::assume(big > 0 && !big.in_line_range());
    let side = any_side();
    let lines = [(side, big), (kernel::flip(side), big)];
    assert_eq!(kernel::check_lines(&lines), Err(LineError::OutOfRange));
}

/// An accepted entry nets to zero.
#[kani::proof]
#[kani::unwind(6)]
fn accepted_entry_nets_to_zero() {
    let all = any_lines();
    let lines = &all[..any_len()];
    if kernel::check_lines(lines).is_ok() {
        assert_eq!(kernel::net(lines.iter().copied()), Some(0));
        kani::cover!(lines.len() == N, "an accepted four-line entry");
    }
}

/// Movements group lines by account without losing or inventing money:
/// any account appears at most once, and its movement is the net of its
/// own lines (zero if it has none).
#[kani::proof]
#[kani::unwind(6)]
fn movements_are_per_account() {
    let all = any_account_lines();
    let lines = &all[..any_len()];
    let mut out = [(0u8, 0i64); N];
    let n = kernel::movements_into(lines, &mut out).expect("no overflow, room for all");
    let moves = &out[..n];
    let k: u8 = kani::any();
    kani::assume(k < K);
    let mut own = 0i64;
    let mut has = false;
    for &(a, s, x) in lines {
        if a == k {
            own += wide(s, x);
            has = true;
        }
    }
    let mut found = 0;
    let mut movement = 0i64;
    for &(a, m) in moves {
        if a == k {
            found += 1;
            movement = m;
        }
    }
    assert_eq!(found, usize::from(has));
    assert_eq!(movement, own);
    kani::cover!(n == 3, "three distinct accounts");
}

/// The movements sum to the lines' net: zero for an accepted entry.
#[kani::proof]
#[kani::unwind(6)]
fn movements_sum_to_the_net() {
    let all = any_account_lines();
    let lines = &all[..any_len()];
    let mut out = [(0u8, 0i64); N];
    let n = kernel::movements_into(lines, &mut out).expect("no overflow, room for all");
    let total: i64 = out[..n].iter().map(|&(_, m)| m).sum();
    let all_sides = sides(lines);
    let s = &all_sides[..lines.len()];
    assert_eq!(total, wide_net(s));
    if kernel::check_lines(s).is_ok() {
        assert_eq!(total, 0);
        kani::cover!(
            n >= 2 && lines.len() == N,
            "an accepted entry over several accounts"
        );
    }
}

/// A reversal flips every side and keeps every account and amount, so it
/// is accepted exactly when the original is. (`kernel::reversal` is
/// `reverse_line` mapped over the lines; proving it line by line keeps
/// the model free of allocation.)
#[kani::proof]
#[kani::unwind(6)]
fn reversal_mirrors() {
    let all = any_account_lines();
    let lines = &all[..any_len()];
    let mut rev = [(0u8, Side::Debit, 0i64); N];
    for i in 0..lines.len() {
        rev[i] = kernel::reverse_line(lines[i]);
        assert_eq!((rev[i].0, rev[i].2), (lines[i].0, lines[i].2));
        assert_eq!(rev[i].1, kernel::flip(lines[i].1));
    }
    let n = lines.len();
    assert_eq!(
        kernel::check_lines(&sides(&rev[..n])[..n]).is_ok(),
        kernel::check_lines(&sides(lines)[..n]).is_ok()
    );
    kani::cover!(
        n == N && kernel::check_lines(&sides(lines)[..n]).is_ok(),
        "an accepted four-line entry, reversed"
    );
}

/// An entry and its reversal together net every account to zero.
#[kani::proof]
#[kani::unwind(10)]
fn reversal_cancels() {
    let all = any_account_lines();
    let n = any_len();
    let lines = &all[..n];
    // Element by element, never `copy_from_slice`: Kani 0.68 mis-models a
    // bulk copy of symbolic length (see `kani_symbolic_copy_canary`).
    let mut both = [(0u8, Side::Debit, 0i64); 2 * N];
    for i in 0..n {
        let (k, s, a) = lines[i];
        both[i] = lines[i];
        both[n + i] = kernel::reverse_line((k, s, a));
    }
    let mut out = [(0u8, 0i64); 2 * N];
    let m = kernel::movements_into(&both[..2 * n], &mut out).expect("no overflow");
    for &(_, total) in &out[..m] {
        assert_eq!(total, 0);
    }
    // Not vacuous: an account with several lines on both sides is reached.
    kani::cover!(n == N && m == 1, "four lines on one account, reversed");
}

// ---------------------------------------------------------------------------
// Balances and statements
// ---------------------------------------------------------------------------

/// A normal balance is the movement for assets and expenses, and its
/// negation for liabilities, equity, and revenue. This is the fact the
/// accounting equation rests on: the Lean theorem `accounting_equation`
/// proves it gives Assets = Liabilities + Equity + (Revenue − Expenses)
/// for any number of accounts whose movements net to zero.
#[kani::proof]
fn normal_balance_is_signed() {
    let t = any_type();
    let m = any_amount();
    let b = kernel::normal_balance(t, m).expect("in range");
    match t {
        AccountType::Asset | AccountType::Expense => assert_eq!(b, m),
        AccountType::Liability | AccountType::Equity | AccountType::Revenue => {
            assert_eq!(b, -m)
        }
    }
}

/// The accounting equation end to end on the code, for up to three
/// accounts: whenever movements net to zero,
/// Assets = Liabilities + Equity + (Revenue − Expenses). (Regrouping a
/// sum of signed 64-bit values is hard for a SAT solver, so the bound is
/// small here; `normal_balance_is_signed` and the Lean proof cover the
/// rest.)
#[kani::proof]
#[kani::unwind(4)]
fn accounting_equation() {
    const EQ: usize = 3;
    let all: [(AccountType, i64); EQ] = core::array::from_fn(|_| (any_type(), any_amount()));
    let n: usize = kani::any();
    kani::assume(n <= EQ);
    let accounts = &all[..n];
    let (mut net, mut a, mut l, mut e, mut r, mut x) = (0i64, 0i64, 0i64, 0i64, 0i64, 0i64);
    for &(t, m) in accounts {
        net += m;
        let b = kernel::normal_balance(t, m).expect("in range");
        match t {
            AccountType::Asset => a += b,
            AccountType::Liability => l += b,
            AccountType::Equity => e += b,
            AccountType::Revenue => r += b,
            AccountType::Expense => x += b,
        }
    }
    assert_eq!(a - (l + e + (r - x)), net);
    if net == 0 {
        assert_eq!(a, l + e + (r - x));
        kani::cover!(n == EQ, "a balanced three-account chart");
    }
}

/// A normal-side line raises an account's balance by its amount; an
/// opposite-side line lowers it.
#[kani::proof]
fn normal_side_increases() {
    let t = any_type();
    let a = any_amount();
    let side = any_side();
    let movement = kernel::signed(side, a).expect("in range");
    let bal = kernel::normal_balance(t, movement).expect("in range");
    if side == t.normal_side() {
        assert_eq!(bal, a);
    } else {
        assert_eq!(bal, -a);
    }
}

/// A trial balance puts each movement in exactly one non-negative column,
/// so its columns balance exactly when movements net to zero.
#[kani::proof]
#[kani::unwind(6)]
fn trial_balance_columns() {
    let all: [i64; N] = core::array::from_fn(|_| any_amount());
    let moves = &all[..any_len()];
    let (mut debits, mut credits) = (0i64, 0i64);
    for &m in moves {
        let (d, c) = kernel::columns(m).expect("in range");
        assert!(d >= 0 && c >= 0);
        assert!(d == 0 || c == 0);
        assert_eq!(d - c, m);
        debits += d;
        credits += c;
    }
    let net: i64 = moves.iter().map(|&m| m).sum();
    assert_eq!(debits - credits, net);
    assert_eq!(debits == credits, net == 0);
}

// ---------------------------------------------------------------------------
// Closing
// ---------------------------------------------------------------------------

/// A closing line zeroes its account: opposite side, same amount, nothing
/// for a zero balance.
#[kani::proof]
fn closing_line_zeroes() {
    let m = any_amount();
    match kernel::closing_line(m) {
        None => assert_eq!(m, 0),
        Some((side, amount)) => {
            assert!(amount > 0);
            assert_eq!(m + wide(side, amount), 0);
        }
    }
}

/// `credit_line` credits exactly what it is asked to.
#[kani::proof]
fn credit_line_credits() {
    let c = any_amount();
    match kernel::credit_line(c) {
        None => assert_eq!(c, 0),
        Some((side, amount)) => {
            assert!(amount > 0);
            assert_eq!(wide(side, amount), -c);
        }
    }
}

/// A period close balances and zeroes every revenue and expense account:
/// one closing line per nonzero nominal account, plus the net credited to
/// equity (whole, or split among members): the entry's lines net to
/// zero, and each nominal account ends at zero.
#[kani::proof]
#[kani::unwind(6)]
fn closing_entry_balances() {
    const NOMINAL: usize = 3;
    let nominal: [i64; NOMINAL] = core::array::from_fn(|_| any_amount());
    // Each nominal account's closing line zeroes it.
    let mut entry = [(Side::Debit, 0i64); NOMINAL + 2];
    let mut len = 0;
    for &m in &nominal {
        if let Some((side, amount)) = kernel::closing_line(m) {
            assert_eq!(m + wide(side, amount), 0);
            entry[len] = (side, amount);
            len += 1;
        } else {
            assert_eq!(m, 0);
        }
    }
    let net = kernel::net(entry[..len].iter().copied()).expect("no overflow");
    // Split the net between two members, the first share rounded any way.
    let mut parts = [any_amount(), 0];
    kernel::plug_last(net, &mut parts).expect("no overflow");
    for p in parts {
        if let Some(line) = kernel::credit_line(p) {
            entry[len] = line;
            len += 1;
        }
    }
    assert_eq!(wide_net(&entry[..len]), 0);
    kani::cover!(len == NOMINAL + 2, "three accounts closed to two members");
}

// ---------------------------------------------------------------------------
// 1099 payments (rules.md B-19)
// ---------------------------------------------------------------------------

fn any_role() -> kernel::PayRole {
    let r: u8 = kani::any();
    kani::assume(r < 4);
    match r {
        0 => kernel::PayRole::Cash,
        1 => kernel::PayRole::Financing,
        2 => kernel::PayRole::CardAccount,
        _ => kernel::PayRole::Other,
    }
}

fn any_tag() -> kernel::Tag {
    let t: u8 = kani::any();
    kani::assume(t < 4);
    match t {
        0 => kernel::Tag::None,
        1 => kernel::Tag::ThisCard,
        2 => kernel::Tag::ThisOther,
        _ => kernel::Tag::Elsewhere,
    }
}

/// `kernel::payment_bucket` is the B-19 table, all 16 cases: a tag naming
/// the line's account decides (card or network: Card; otherwise Counts);
/// else the role does, and with no tag naming its account a financing line
/// never counts and a card account's line is always Card.
#[kani::proof]
fn payment_bucket_is_the_table() {
    use kernel::{Bucket, PayRole, Tag};
    let (role, tag) = (any_role(), any_tag());
    let got = kernel::payment_bucket(role, tag);
    let want = match tag {
        Tag::ThisCard => Some(Bucket::Card),
        Tag::ThisOther => Some(Bucket::Counts),
        Tag::None | Tag::Elsewhere => match role {
            PayRole::Cash => Some(Bucket::Counts),
            PayRole::Financing => Some(Bucket::Undetermined),
            PayRole::CardAccount => Some(Bucket::Card),
            PayRole::Other => None,
        },
    };
    assert_eq!(got, want);
    if matches!(tag, Tag::None | Tag::Elsewhere) {
        assert!(role != PayRole::Financing || got != Some(Bucket::Counts));
        assert!(role != PayRole::CardAccount || got == Some(Bucket::Card));
    }
}

/// `kernel::split_payment` never fails on journal-line amounts, and each
/// bucket is the exact credits-minus-debits of its lines: the buckets
/// together are the whole payment, so nothing is lost or counted twice.
#[kani::proof]
#[kani::unwind(6)]
fn payment_split_is_exact() {
    let n = any_len();
    let lines: [(Option<kernel::Bucket>, Side, i64); N] = core::array::from_fn(|_| {
        let b: u8 = kani::any();
        kani::assume(b < 4);
        let bucket = match b {
            0 => Some(kernel::Bucket::Counts),
            1 => Some(kernel::Bucket::Card),
            2 => Some(kernel::Bucket::Undetermined),
            _ => None,
        };
        (bucket, any_side(), any_amount())
    });
    let got = kernel::split_payment(&lines[..n]).expect("never fails in range");
    for b in kernel::Bucket::ALL {
        let mut want = 0i64;
        for &(lb, side, amount) in &lines[..n] {
            if lb == Some(b) {
                want -= wide(side, amount);
            }
        }
        assert_eq!(got[b.index()], want);
    }
    kani::cover!(n == N, "four lines");
}

/// `kernel::net_buckets` keeps the payment's total, never leaves a bucket
/// opposite the total, and leaves buckets of one sign alone (money moved
/// between the company's own sources pays no one: P3 review R1); with
/// mixed signs the whole net goes to the one bucket of its sign, or to
/// Undetermined when two share it (its SCENARIOS 1-3).
#[kani::proof]
fn net_buckets_keeps_the_total() {
    let b: [i64; 3] = core::array::from_fn(|_| any_amount());
    let p = b[0] + b[1] + b[2];
    let out = kernel::net_buckets(b).expect("never fails in range");
    assert_eq!(out[0] + out[1] + out[2], p);
    for x in out {
        assert!(x == 0 || (x > 0) == (p > 0));
    }
    let one_sign = b.iter().all(|&x| x >= 0) || b.iter().all(|&x| x <= 0);
    if one_sign {
        assert_eq!(out, b);
    } else {
        // Mixed signs: the whole net lands in one bucket, the one with its
        // sign when there is exactly one, else Undetermined.
        let with_sign: [bool; 3] = core::array::from_fn(|i| b[i] != 0 && (b[i] > 0) == (p > 0));
        let takers = with_sign.iter().filter(|&&s| s).count();
        let mut taker = kernel::Bucket::Undetermined.index();
        if takers == 1 {
            for (i, &s) in with_sign.iter().enumerate() {
                if s {
                    taker = i;
                }
            }
        }
        for (i, &x) in out.iter().enumerate() {
            assert_eq!(x, if i == taker { p } else { 0 });
        }
    }
    kani::cover!(b[0] > 0 && b[2] < 0 && p > 0, "a payment net of a refund");
    kani::cover!(
        b[0] > 0 && b[1] > 0 && b[2] < 0 && p > 0,
        "two buckets share the net's sign"
    );
    kani::cover!(!one_sign && p == 0, "money moved between own sources");
}

/// An allocation's parts sum to the whole exactly, whatever the rounding
/// of the others, and only the last part changes.
#[kani::proof]
#[kani::unwind(6)]
fn plug_last_sums_exactly() {
    let whole = any_amount();
    let n = any_len();
    let mut all: [i64; N] = core::array::from_fn(|_| any_amount());
    let before = all;
    let parts = &mut all[..n];
    kernel::plug_last(whole, parts).expect("no overflow");
    if n > 0 {
        let sum: i64 = parts.iter().map(|&p| p).sum();
        assert_eq!(sum, whole);
        for i in 0..n - 1 {
            assert_eq!(parts[i], before[i]);
        }
    }
}

/// Most parts the apportionment harnesses explore (its ranking is
/// quadratic, so the bound is smaller than `N`).
const AP: usize = 3;

/// Inputs for `kernel::apportion`: rounded-down parts, remainders, a unit,
/// and a whole some shortfall above the parts' sum.
fn any_apportionment() -> (usize, [i64; AP], [i64; AP], i64, i64, i64) {
    let n: usize = kani::any();
    kani::assume(n <= AP);
    let before: [i64; AP] = core::array::from_fn(|_| any_amount());
    let rems: [i64; AP] = core::array::from_fn(|_| any_amount());
    let unit: i64 = kani::any();
    kani::assume(unit > 0 && unit <= 1000);
    let shortfall: i64 = kani::any();
    kani::assume(shortfall.unsigned_abs() <= 4000);
    let mut whole = shortfall;
    for &p in &before[..n] {
        whole += p;
    }
    (n, before, rems, unit, shortfall, whole)
}

/// `kernel::apportion` succeeds exactly when the parts are short of the
/// whole by less than one unit per part; then the parts sum to the whole,
/// and each gains nothing or one unit, except that one part may also take
/// the sub-unit fraction.
#[kani::proof]
#[kani::unwind(5)]
fn apportion_is_exact() {
    let (n, before, rems, unit, shortfall, whole) = any_apportionment();
    let mut parts = before;
    let ok = kernel::apportion(whole, &mut parts[..n], &rems[..n], unit);
    let in_range = if n == 0 {
        whole == 0
    } else {
        shortfall >= 0 && shortfall < n as i64 * unit
    };
    assert_eq!(ok.is_some(), in_range);
    if ok.is_none() {
        return;
    }
    let mut sum = 0i64;
    let mut fraction_takers = 0;
    for i in 0..n {
        sum += parts[i];
        let gain = parts[i] - before[i];
        assert!(gain >= 0 && gain < 2 * unit);
        if gain != 0 && gain != unit {
            fraction_takers += 1;
        }
    }
    assert_eq!(sum, whole);
    assert!(fraction_takers <= 1);
    kani::cover!(
        n == AP && shortfall >= 2 * unit,
        "three parts, two units short"
    );
}

/// `kernel::apportion` never gives a unit to a part while a part with a
/// strictly larger remainder goes without (owner, 2026-10-13: no member
/// favored by position).
#[kani::proof]
#[kani::unwind(5)]
fn apportion_is_fair() {
    let (n, before, rems, unit, _, whole) = any_apportionment();
    let mut parts = before;
    if kernel::apportion(whole, &mut parts[..n], &rems[..n], unit).is_none() {
        return;
    }
    for i in 0..n {
        for j in 0..n {
            if rems[i] > rems[j] {
                assert!(parts[i] - before[i] >= unit || parts[j] - before[j] < unit);
            }
        }
    }
    kani::cover!(
        n == AP && rems[0] < rems[2],
        "a later part with a larger remainder"
    );
}

/// `kernel::apportion_cumulative` is `kernel::apportion` less what was
/// given: it succeeds exactly when `apportion` would on the same inputs and
/// the running total is in line range (the parts, unit and `given` are,
/// here), and then each part is apportion's less its `given`, so each
/// running total (given + part) is apportion's. With `apportion_is_exact`,
/// the parts sum to the running total less what was given, and each
/// running total is nothing or one unit above its rounded-down part (one
/// of them may also take the sub-unit fraction). A refusal leaves the
/// parts as they were.
#[kani::proof]
#[kani::unwind(5)]
fn apportion_cumulative_is_exact() {
    let (n, before, rems, unit, _, whole) = any_apportionment();
    let given: [i64; AP] = core::array::from_fn(|_| any_amount());
    let mut reference = before;
    let apportioned = kernel::apportion(whole, &mut reference[..n], &rems[..n], unit);
    let mut parts = before;
    let ok = kernel::apportion_cumulative(whole, &mut parts[..n], &rems[..n], unit, &given[..n]);
    assert_eq!(ok.is_some(), apportioned.is_some() && whole.in_line_range());
    let mut against = false;
    for i in 0..n {
        if ok.is_some() {
            assert_eq!(parts[i] + given[i], reference[i]);
            against |= parts[i] < 0;
        } else {
            assert_eq!(parts[i], before[i]);
        }
    }
    kani::cover!(
        ok.is_some() && n == AP && against && whole > given[0] + given[1] + given[2],
        "a true-up against the net's sign"
    );
}

/// Bound on the amounts the drift harnesses explore (symbolic multiply and
/// divide are costly for the solver; Lean's `allocW_drift` and
/// `cumulative_drift` carry any weights and any amounts).
const DRIFT: i64 = 1 << 12;

/// **Bounded drift** (rules.md B-13) for members with these integer
/// weights (shares w_i / W, W their sum): a cumulative members close,
/// modelled as `periods::allocate_cumulative` runs it at whole minor units
/// (unit 1), hands out exactly its net, and leaves every member's running
/// total, given + part, within one unit of their exact share of
/// everything closed, whatever earlier closes gave them (any `given`
/// summing to what they closed: legacy rounding included). Scaled by W:
/// |(given_i + part_i)·W − T·w_i| < W.
fn drift_is_under_a_unit(weights: &[i64]) {
    let n = weights.len();
    let mut total_weight = 0i64;
    for w in weights {
        total_weight += w;
    }
    let closed: i64 = kani::any();
    let net: i64 = kani::any();
    kani::assume((-DRIFT..=DRIFT).contains(&closed) && (-DRIFT..=DRIFT).contains(&net));
    // What earlier closes gave: anything, as long as it adds up.
    let mut given = [0i64; AP];
    let mut others = 0i64;
    for g in given.iter_mut().take(n - 1) {
        *g = kani::any();
        kani::assume((-DRIFT..=DRIFT).contains(g));
        others += *g;
    }
    given[n - 1] = closed - others;
    // allocate_cumulative: |T| split by exact share, rounded toward zero,
    // the remainders ranked; a loss runs on |T| against given negated.
    let total = closed + net;
    let magnitude = total.abs();
    let loss = total < 0;
    let mut parts = [0i64; AP];
    let mut rems = [0i64; AP];
    let mut against = [0i64; AP];
    for i in 0..n {
        parts[i] = magnitude * weights[i] / total_weight;
        rems[i] = magnitude * weights[i] % total_weight;
        against[i] = if loss { -given[i] } else { given[i] };
    }
    kernel::apportion_cumulative(magnitude, &mut parts[..n], &rems[..n], 1, &against[..n])
        .expect("exact shares are fewer units short than there are members");
    let mut sum = 0i64;
    for i in 0..n {
        if loss {
            parts[i] = -parts[i];
        }
        sum += parts[i];
        let drift = (given[i] + parts[i]) * total_weight - total * weights[i];
        assert!(-total_weight < drift && drift < total_weight);
    }
    assert_eq!(sum, net);
    kani::cover!(
        loss && given[0] * total_weight != closed * weights[0],
        "a loss, after an earlier split off the exact share"
    );
}

/// [`drift_is_under_a_unit`] for two members at half each: every odd cent
/// a tie.
#[kani::proof]
#[kani::unwind(4)]
fn cumulative_close_never_drifts_a_unit_at_halves() {
    drift_is_under_a_unit(&[1, 1]);
}

/// [`drift_is_under_a_unit`] for three members at 20% / 30% / 50%:
/// remainders that differ.
#[kani::proof]
#[kani::unwind(4)]
fn cumulative_close_never_drifts_a_unit_at_tenths() {
    drift_is_under_a_unit(&[2, 3, 5]);
}

/// [`drift_is_under_a_unit`] for three members at 33.33% / 33.33% /
/// 33.34%, the nearest decimal shares to a third each.
#[kani::proof]
#[kani::unwind(4)]
fn cumulative_close_never_drifts_a_unit_at_near_thirds() {
    drift_is_under_a_unit(&[3333, 3333, 3334]);
}

/// A cumulative members close balances: the nominal accounts' closing
/// lines and each member's part of the running split (any earlier split
/// of what was closed before) net to zero, whatever the true-up.
#[kani::proof]
#[kani::unwind(6)]
fn cumulative_closing_entry_balances() {
    const NOMINAL: usize = 3;
    let nominal: [i64; NOMINAL] = core::array::from_fn(|_| {
        let a = any_amount();
        kani::assume((-(1 << 38)..=1 << 38).contains(&a));
        a
    });
    let mut entry = [(Side::Debit, 0i64); NOMINAL + 2];
    let mut len = 0;
    for &m in &nominal {
        if let Some(line) = kernel::closing_line(m) {
            entry[len] = line;
            len += 1;
        }
    }
    let net = kernel::net(entry[..len].iter().copied()).expect("no overflow");
    // Two members at half each; earlier closes gave them any split of
    // what they closed.
    let closed: i64 = kani::any();
    kani::assume((-(1 << 38)..=1 << 38).contains(&closed));
    let first = any_amount();
    let given = [first, closed - first];
    let total = closed + net;
    let (magnitude, loss) = (total.abs(), total < 0);
    let mut parts = [magnitude / 2, magnitude / 2];
    let rems = [magnitude % 2, magnitude % 2];
    let against = if loss { [-given[0], -given[1]] } else { given };
    if kernel::apportion_cumulative(magnitude, &mut parts, &rems, 1, &against).is_none() {
        // Only an earlier split out of line range is refused here.
        assert!(!given[1].in_line_range());
        return;
    }
    let mut true_up = false;
    for p in parts {
        let p = if loss { -p } else { p };
        if let Some(line) = kernel::credit_line(p) {
            entry[len] = line;
            len += 1;
            true_up |= net != 0 && (p > 0) != (net > 0);
        }
    }
    assert_eq!(wide_net(&entry[..len]), 0);
    kani::cover!(true_up, "a true-up line on a member");
}

// ---------------------------------------------------------------------------
// The statement of cash flows
// ---------------------------------------------------------------------------

use kernel::Flow;

/// Most lines per entry the cash flow harnesses explore: three is the
/// fewest an entry needs to be mixed (cash and two sections), and each
/// line can land in any of the four places. (Four lines ran past half an
/// hour per harness, too long for `just prove`; the Lean model covers any
/// number.)
const NF: usize = 3;

/// Any count up to `NF`.
fn any_flow_len() -> usize {
    let n: usize = kani::any();
    kani::assume(n <= NF);
    n
}

fn any_flow() -> Flow {
    let t: u8 = kani::any();
    kani::assume(t < 4);
    match t {
        0 => Flow::Cash,
        1 => Flow::Operating,
        2 => Flow::Investing,
        _ => Flow::Financing,
    }
}

/// `NF` lines, each tagged with its place on the statement (use a
/// prefix).
fn any_flow_lines() -> [(Flow, Side, i64); NF] {
    core::array::from_fn(|_| (any_flow(), any_side(), any_amount()))
}

/// The spec's net movement of the lines tagged `f`.
fn tagged(lines: &[(Flow, Side, i64)], f: Flow) -> i64 {
    let mut total = 0i64;
    for &(g, s, a) in lines {
        if g == f {
            total += wide(s, a);
        }
    }
    total
}

/// The spec's section movements, `[operating, investing, financing]`.
fn sections(lines: &[(Flow, Side, i64)]) -> [i64; 3] {
    [
        tagged(lines, Flow::Operating),
        tagged(lines, Flow::Investing),
        tagged(lines, Flow::Financing),
    ]
}

/// Whether the lines would post: their sides and amounts pass
/// `check_lines`.
fn accepted(lines: &[(Flow, Side, i64)]) -> bool {
    let mut s = [(Side::Debit, 0i64); N];
    for (o, &(_, side, a)) in s.iter_mut().zip(lines) {
        *o = (side, a);
    }
    kernel::check_lines(&s[..lines.len()]).is_ok()
}

/// The building block of every cash flow harness below, for any two
/// amounts: what a section takes is odd (a reversal takes minus it), has
/// the cash's sign (or is zero), comes only from a movement against the
/// cash, and is never more than the cash left or the movement.
#[kani::proof]
fn take_is_odd_and_bounded() {
    let r = any_amount();
    let n = any_amount();
    let t = kernel::take(r, n).expect("in range");
    assert_eq!(kernel::take(-r, -n), Some(-t));
    assert!(t == 0 || (t > 0) == (r > 0));
    assert!(t == 0 || (t > 0) == (n < 0));
    assert!(t.abs() <= r.abs() && t.abs() <= n.abs());
    kani::cover!(
        t != 0 && t.abs() < r.abs(),
        "a movement smaller than the cash"
    );
}

/// Applying minus the cash to minus the movements gives minus the parts,
/// part by part, and succeeds exactly when the original does.
#[kani::proof]
#[kani::unwind(5)]
fn apply_cash_mirrors() {
    const M: usize = 3;
    let cash = any_amount();
    let moves: [i64; M] = core::array::from_fn(|_| any_amount());
    let neg: [i64; M] = core::array::from_fn(|k| -moves[k]);
    let mut a = [0i64; M];
    let mut b = [0i64; M];
    let ra = kernel::apply_cash(cash, &moves, &mut a);
    let rb = kernel::apply_cash(-cash, &neg, &mut b);
    assert_eq!(ra.is_some(), rb.is_some());
    if ra.is_some() {
        for k in 0..M {
            assert_eq!(b[k], -a[k]);
        }
        kani::cover!(a[0] != 0 && a[2] != 0, "two sections settled, mirrored");
    }
}

/// An accepted entry always has cash flows, and they explain its cash
/// exactly: the sections' cash sums to the entry's cash, and what no cash
/// settled (each section's movement plus its cash) sums to zero
/// (rules.md B-14 rules 3 and 9: the statement foots, entry by entry).
#[kani::proof]
#[kani::unwind(5)]
fn entry_flows_sum_to_cash() {
    let all = any_flow_lines();
    let lines = &all[..any_flow_len()];
    if !accepted(lines) {
        return;
    }
    let f = kernel::entry_flows(lines).expect("an accepted entry has cash flows");
    let n = sections(lines);
    assert_eq!(f.cash, tagged(lines, Flow::Cash));
    assert_eq!(f.flows[0] + f.flows[1] + f.flows[2], f.cash);
    assert_eq!(f.noncash[0] + f.noncash[1] + f.noncash[2], 0);
    for k in 0..3 {
        assert_eq!(f.noncash[k], n[k] + f.flows[k]);
    }
    kani::cover!(
        f.flows[1] != 0 && f.noncash[1] != 0,
        "a part-cash, part-noncash investing entry"
    );
}

/// When every section moves against the cash (or not at all), the cash
/// settles every movement: each section's cash is minus its movement and
/// nothing is noncash. (With no cash at all the premise would hold
/// vacuously; review item 7: it needs cash.)
#[kani::proof]
#[kani::unwind(5)]
fn entry_flows_pure_are_movements() {
    let all = any_flow_lines();
    let lines = &all[..any_flow_len()];
    if !accepted(lines) {
        return;
    }
    let c = tagged(lines, Flow::Cash);
    let n = sections(lines);
    let against = |m: i64| m == 0 || (c > 0 && m < 0) || (c < 0 && m > 0);
    if c == 0 || !(against(n[0]) && against(n[1]) && against(n[2])) {
        return;
    }
    let f = kernel::entry_flows(lines).expect("an accepted entry has cash flows");
    for k in 0..3 {
        assert_eq!(f.flows[k], -n[k]);
        assert_eq!(f.noncash[k], 0);
    }
    kani::cover!(n[0] != 0 && n[2] != 0, "cash settling two sections at once");
}

/// Each section's cash has the cash's sign (or is zero), is never larger
/// than the section's own movement, and comes only from a section moving
/// against the cash.
#[kani::proof]
#[kani::unwind(5)]
fn entry_flows_are_bounded() {
    let all = any_flow_lines();
    let lines = &all[..any_flow_len()];
    let Some(f) = kernel::entry_flows(lines) else {
        return;
    };
    let n = sections(lines);
    for k in 0..3 {
        let x = f.flows[k];
        assert!(x == 0 || (x > 0) == (f.cash > 0));
        assert!(x.abs() <= n[k].abs());
        assert!(x == 0 || (x > 0) == (n[k] < 0));
    }
    kani::cover!(
        f.flows[0] != 0 && f.flows[0].abs() < n[0].abs(),
        "a section only partly settled by cash"
    );
}

/// A reversal's cash flows are its original's, negated, part by part:
/// what lets an entry and its reversal in one period drop out of the
/// statement together (rules.md B-14 rule 6).
#[kani::proof]
#[kani::unwind(5)]
fn entry_flows_of_a_reversal_mirror() {
    let all = any_flow_lines();
    let n = any_flow_len();
    let lines = &all[..n];
    // Element by element (see `kani_symbolic_copy_canary`).
    let mut rev = [(Flow::Cash, Side::Debit, 0i64); NF];
    for i in 0..n {
        rev[i] = kernel::reverse_line(lines[i]);
    }
    let a = kernel::entry_flows(lines);
    let b = kernel::entry_flows(&rev[..n]);
    assert_eq!(a.is_some(), b.is_some());
    if let (Some(a), Some(b)) = (a, b) {
        assert_eq!(b.cash, -a.cash);
        for k in 0..3 {
            assert_eq!(b.flows[k], -a.flows[k]);
            assert_eq!(b.noncash[k], -a.noncash[k]);
        }
        kani::cover!(
            a.flows[1] != 0 && a.noncash[2] != 0,
            "a mixed entry, reversed"
        );
    }
}

/// An entry with no cash line has no cash flows: every movement is
/// noncash.
#[kani::proof]
#[kani::unwind(5)]
fn no_cash_no_flow() {
    let all = any_flow_lines();
    let lines = &all[..any_flow_len()];
    for &(f, _, _) in lines {
        kani::assume(f != Flow::Cash);
    }
    if !accepted(lines) {
        return;
    }
    let f = kernel::entry_flows(lines).expect("an accepted entry has cash flows");
    let n = sections(lines);
    assert_eq!(f.cash, 0);
    for k in 0..3 {
        assert_eq!(f.flows[k], 0);
        assert_eq!(f.noncash[k], n[k]);
    }
    kani::cover!(n[1] != 0 && n[2] != 0, "property contributed for capital");
}

/// How a section's cash splits among its kinds (contributions per member,
/// borrowing per lender): `apply_cash` succeeds exactly when the
/// movements against the cash cover it, and then the parts sum to the
/// cash, each with its sign and within its own movement.
#[kani::proof]
#[kani::unwind(5)]
fn apply_cash_spends_exactly() {
    const M: usize = 3;
    let cash = any_amount();
    let moves: [i64; M] = core::array::from_fn(|_| any_amount());
    let n: usize = kani::any();
    kani::assume(n <= M);
    let mut against = 0i64;
    for &m in &moves[..n] {
        if (cash > 0 && m < 0) || (cash < 0 && m > 0) {
            against += m.abs();
        }
    }
    let mut out = [0i64; M];
    let ok = kernel::apply_cash(cash, &moves[..n], &mut out[..n]);
    assert_eq!(ok.is_some(), cash.abs() <= against);
    if ok.is_some() {
        let mut sum = 0i64;
        for k in 0..n {
            let x = out[k];
            sum += x;
            assert!(x == 0 || (x > 0) == (cash > 0));
            assert!(x.abs() <= moves[k].abs());
        }
        assert_eq!(sum, cash);
        kani::cover!(n == M && out[2] != 0, "the third kind takes the rest");
    }
}

/// Any amount a line may carry, not negative.
fn any_magnitude() -> i64 {
    let a = any_amount();
    kani::assume(a >= 0);
    a
}

/// The exchange inside investing and financing is the smaller side left
/// once cash in has settled credits and cash out debits, never negative
/// and never more than either side.
#[kani::proof]
fn exchanged_is_bounded() {
    let (d, c) = (any_magnitude(), any_magnitude());
    let (inflow, outflow) = (any_magnitude(), any_magnitude());
    let x = kernel::exchanged(d, c, inflow, outflow).expect("no overflow");
    let spec = (d - outflow).min(c - inflow).max(0);
    assert_eq!(x, spec);
    assert!(0 <= x && x <= d && x <= c);
    kani::cover!(x > 0 && outflow != 0, "a part-cash exchange");
    kani::cover!(
        x > 0 && inflow != 0 && outflow != 0,
        "an exchange beside cash in and out"
    );
}

fn any_way() -> kernel::OnBehalf {
    if kani::any() {
        kernel::OnBehalf::Paid
    } else {
        kernel::OnBehalf::Received
    }
}

/// What a member or a lender paid or received for the company (rules.md
/// B-14 rule 11, D-42) invents and loses nothing: the parts net to zero,
/// so an entry's flows still sum to its cash; each part settles its own
/// movement toward zero, never past it; and as much is settled as can be.
#[kani::proof]
#[kani::unwind(4)]
fn constructive_nets_to_zero() {
    const M: usize = 2;
    let way = any_way();
    let s: [i64; M] = core::array::from_fn(|_| any_amount());
    let u: [i64; M] = core::array::from_fn(|_| any_amount());
    let (mut from, mut to) = ([0i64; M], [0i64; M]);
    kernel::constructive(way, &s, &u, &mut from, &mut to).expect("amounts in range always settle");
    assert_eq!(from[0] + from[1] + to[0] + to[1], 0);
    let settles = |x: i64, m: i64| (x == 0 || (x > 0) == (m < 0)) && x.abs() <= m.abs();
    for k in 0..M {
        assert!(settles(from[k], s[k]));
        assert!(settles(to[k], u[k]));
    }
    let ls = [s[0] + from[0], s[1] + from[1]];
    let lu = [u[0] + to[0], u[1] + to[1]];
    let any = |xs: [i64; M], f: fn(i64) -> bool| f(xs[0]) || f(xs[1]);
    match way {
        kernel::OnBehalf::Paid => {
            assert!(!(any(ls, |x| x < 0) && any(lu, |x| x > 0)));
            // Paying never touches a source debit or a use credit.
            for k in 0..M {
                assert!(s[k] < 0 || from[k] == 0);
                assert!(u[k] > 0 || to[k] == 0);
            }
        }
        kernel::OnBehalf::Received => {
            assert!(!(any(ls, |x| x > 0) && any(lu, |x| x < 0)));
            for k in 0..M {
                assert!(s[k] > 0 || from[k] == 0);
                assert!(u[k] < 0 || to[k] == 0);
            }
        }
    }
    kani::cover!(from[0] > 0 && from[1] > 0, "two sources pay");
    kani::cover!(from[0] < 0 && to[1] > 0, "received for the company");
    kani::cover!(
        ls[0] < 0 && to[0] < 0,
        "a source larger than what it paid for"
    );
}

/// The other way, minus the sources and minus the uses give minus the
/// parts: a reversal of a payment made on the company's behalf mirrors it
/// (rules.md B-14 rule 6), so the two drop out together.
#[kani::proof]
#[kani::unwind(4)]
fn constructive_mirrors() {
    const M: usize = 2;
    let way = any_way();
    let s: [i64; M] = core::array::from_fn(|_| any_amount());
    let u: [i64; M] = core::array::from_fn(|_| any_amount());
    let (ns, nu) = ([-s[0], -s[1]], [-u[0], -u[1]]);
    let (mut fa, mut ta) = ([0i64; M], [0i64; M]);
    let (mut fb, mut tb) = ([0i64; M], [0i64; M]);
    let a = kernel::constructive(way, &s, &u, &mut fa, &mut ta);
    let b = kernel::constructive(way.mirror(), &ns, &nu, &mut fb, &mut tb);
    assert_eq!(a.is_some(), b.is_some());
    if a.is_some() {
        for k in 0..M {
            assert_eq!(fb[k], -fa[k]);
            assert_eq!(tb[k], -ta[k]);
        }
        kani::cover!(fa[0] != 0 && ta[1] != 0, "a payment, mirrored");
    }
}

// ---------------------------------------------------------------------------
// A party's part of an entry (rules.md B-35)
// ---------------------------------------------------------------------------

/// The inputs `kernel::party_target` reads, any in range: a capital
/// movement (when every party named is a member) with the property beside
/// it, the money net, the own-account movement, and the debits (a
/// magnitude).
fn any_party_inputs() -> (Option<(i64, i64)>, i64, i64, i64) {
    let capital = if kani::any() {
        let property = any_amount();
        kani::assume(property >= 0);
        Some((any_amount(), property))
    } else {
        None
    };
    let debits = any_amount();
    kani::assume(debits >= 0);
    (capital, any_amount(), any_amount(), debits)
}

/// B-35 items 1-4: the direction is the sign of what counts (paid above
/// zero, received below), neither exactly when nothing counts; cash and
/// in kind each carry the whole's sign and together never exceed it; a
/// money target is all cash; a contribution counts all of itself; a
/// distribution's cash is never more than the money it paid, nor its in
/// kind more than the property it gave (the rest is still owed).
#[kani::proof]
fn party_target_direction_is_its_sign() {
    let (capital, net, own, debits) = any_party_inputs();
    let t = kernel::party_target(capital, net, own, debits).expect("in range");
    let sign = |a: i64| a.signum();
    assert!(t.cash == 0 || sign(t.cash) == sign(t.amount));
    assert!(t.in_kind == 0 || sign(t.in_kind) == sign(t.amount));
    assert!(t.cash.abs() + t.in_kind.abs() <= t.amount.abs());
    match t.direction {
        kernel::Direction::Neither => {
            assert!(t.cash == 0 && t.in_kind == 0);
        }
        kernel::Direction::Paid => {
            assert!(t.amount > 0 && t.cash + t.in_kind > 0);
        }
        kernel::Direction::Received => {
            assert!(t.amount < 0 && t.cash + t.in_kind < 0);
        }
    }
    let capital_case = matches!(capital, Some((e, _)) if e != 0);
    if !capital_case && net != 0 {
        assert!(t.amount == net && t.cash == net && t.in_kind == 0);
    }
    if let Some((e, property)) = capital {
        if e < 0 {
            // A contribution counts all of itself, money or property.
            assert_eq!(t.cash + t.in_kind, e);
            assert!(-t.in_kind <= property);
        }
        if e > 0 {
            // A distribution: what it paid, then what it gave; no more.
            assert!(t.cash <= net.max(0) && t.in_kind <= property);
        }
    }
    kani::cover!(
        t.direction == kernel::Direction::Paid && t.in_kind != 0 && t.cash != 0,
        "a distribution partly in kind"
    );
    kani::cover!(
        capital_case && t.direction == kernel::Direction::Neither,
        "a distribution all still owed"
    );
    kani::cover!(
        t.direction == kernel::Direction::Paid && t.cash + t.in_kind < t.amount,
        "a distribution partly still owed"
    );
}

/// B-35 item 3 and B-9: the target is odd. Negating the lines negates the
/// capital movement, the money net and the own-account movement, and the
/// property comes from the other side, so it keeps its size: the result
/// is the original's reverse_target, whenever it isn't the debits fallback
/// (a sign means nothing there) and a member's entry counts exactly its
/// money as cash (not an expense booked straight to capital, a debt
/// converted, or a distribution still owed). The reversal arm, not the
/// lines, gives every reversal its target, so these exceptions never reach
/// one.
#[kani::proof]
fn party_target_is_odd() {
    let (capital, net, own, debits) = any_party_inputs();
    if let Some((e, property)) = capital
        && e != 0
    {
        // The money the member's entry moved their way, signed as the
        // distribution side reads it, is exactly what isn't property.
        let side_net = if e > 0 { net } else { -net };
        let whole = e.abs();
        kani::assume(side_net.max(0).min(whole) == whole - property.min(whole));
    }
    let t = kernel::party_target(capital, net, own, debits).expect("in range");
    let negated = capital.map(|(e, p)| (-e, p));
    let n = kernel::party_target(negated, -net, -own, debits).expect("in range");
    let fallback = !matches!(capital, Some((e, _)) if e != 0) && net == 0 && own == 0;
    if !fallback {
        assert_eq!(Some(n), kernel::reverse_target(t));
    }
    kani::cover!(!fallback && capital.is_some(), "a member's entry, negated");
    kani::cover!(
        matches!(capital, Some((e, p)) if e > 0 && p > 0 && net > 0 && net < e),
        "a distribution of money and property, negated"
    );
}

/// B-9: reversing a reversal gives the original back, and reverse_target
/// never panics: past the range it refuses.
#[kani::proof]
fn reverse_target_is_an_involution() {
    let direction = match kani::any::<u8>() {
        0 => kernel::Direction::Paid,
        1 => kernel::Direction::Received,
        _ => kernel::Direction::Neither,
    };
    let t = kernel::PartyTarget {
        amount: kani::any::<i64>(),
        direction,
        cash: kani::any::<i64>(),
        in_kind: kani::any::<i64>(),
    };
    match kernel::reverse_target(t) {
        Some(r) => {
            assert_eq!(r.direction, t.direction.flip());
            assert_eq!(r.amount, -t.amount);
            assert_eq!(kernel::reverse_target(r), Some(t));
        }
        None => {
            assert!(t.amount == i64::MIN || t.cash == i64::MIN || t.in_kind == i64::MIN);
        }
    }
    kani::cover!(t.amount == i64::MIN, "the range's edge");
}

// ---------------------------------------------------------------------------
// Proration by days (depreciation, B-15; daily schedules, B-16)
// ---------------------------------------------------------------------------

/// Most month parts the proration harnesses explore.
const PR: usize = 3;

/// Any share a calendar month can have: 28 to 31 days in the month, 1 to
/// all of them.
fn any_part() -> MonthPart {
    let month_days: u32 = kani::any();
    kani::assume((28..=31).contains(&month_days));
    let days: u32 = kani::any();
    kani::assume(days >= 1 && days <= month_days);
    MonthPart { days, month_days }
}

/// The largest total the proration harnesses explore, in units: 2^16
/// (655.36 in cents). The kernel divides a symbolic 64-bit product by a
/// symbolic divisor, which the SAT solver finds costly; narrowed from 2^20
/// first, as the P3 brief says, before anything else (docs/verification.md
/// says what no bound made tractable, and what proves it instead).
const PR_TOTAL: i64 = 1 << 16;

/// A total up to `PR_TOTAL` units, 1 to 3 months, 1 to `PR` parts, a unit
/// of 1 or 10 (cents, or cents of an amount in tenths of a cent).
fn any_proration() -> (i64, u32, [MonthPart; PR], usize, i64) {
    let total: i64 = kani::any();
    kani::assume(total >= 0 && total <= PR_TOTAL);
    let months: u32 = kani::any();
    kani::assume(months >= 1 && months <= 3);
    let parts: [MonthPart; PR] = core::array::from_fn(|_| any_part());
    let n: usize = kani::any();
    kani::assume(n >= 1 && n <= PR);
    let unit = if kani::any() { 1 } else { 10 };
    (total, months, parts, n, unit)
}

/// The weight of parts, in MONTH_L-ths of a month.
fn weight(parts: &[MonthPart]) -> i64 {
    parts
        .iter()
        .map(|p| i64::from(p.days) * i64::from(MONTH_L / p.month_days))
        .sum()
}

/// The parts of a total prorated by days sum to it exactly, and none is
/// negative; and every split whose parts but the last fit in the span is
/// computed (only symbolic parts that are no calendar's are refused).
#[kani::proof]
#[kani::unwind(5)]
fn prorate_parts_sum_to_total() {
    let (total, months, parts, n, unit) = any_proration();
    let mut out = [0i64; PR];
    let got = kernel::prorate_by_days(total, months, &parts[..n], unit, &mut out[..n]);
    let fits = weight(&parts[..n - 1]) <= i64::from(months) * i64::from(MONTH_L);
    assert_eq!(got.is_some(), fits);
    if got.is_some() {
        let mut sum = 0i64;
        for &o in &out[..n] {
            assert!(o >= 0);
            sum += o;
        }
        assert_eq!(sum, total);
        kani::cover!(n == PR && total > 0, "three parts of a positive total");
    }
}

/// The last part is the total less all the others: it absorbs the
/// remainder, the final month's drift included, and is never negative.
/// A corollary of [`prorate_parts_sum_to_total`] (the parts sum to the
/// total and none is negative), kept as its own harness because the plan
/// names the law (plan/served.md P3 defect 9).
#[kani::proof]
#[kani::unwind(5)]
fn prorate_last_part_absorbs_the_remainder() {
    let (total, months, parts, n, unit) = any_proration();
    let mut out = [0i64; PR];
    if kernel::prorate_by_days(total, months, &parts[..n], unit, &mut out[..n]).is_none() {
        return;
    }
    let mut others = 0i64;
    for &o in &out[..n - 1] {
        others += o;
    }
    assert_eq!(out[n - 1], total - others);
    assert!(out[n - 1] >= 0);
    kani::cover!(
        n == PR && parts[0].days < parts[0].month_days,
        "a partial first month, then the last absorbing the drift"
    );
}

/// A cumulative charge is its exact share rounded half up to a multiple of
/// the unit (or the total itself, when that rounds above it): a multiple of
/// the unit or the total, with −unit × den < 2 × (charge × den − num) <=
/// unit × den (a tie goes up, never down), where num/den is the exact
/// share of the total the kernel rounds (a month but the last: total ×
/// (before + k × MONTH_L / days in month) / W; the last month: total ×
/// (before × d + (W − before) × k) / (W × d)); never negative, never above
/// the total; and the total exactly on the last month's last day. Each
/// part but the last, a difference of two such charges over W, is then
/// its days times the monthly rate within one unit (plan/served.md P3
/// defect 9: "each part equals days × rate within 0.01", for any minor
/// unit): Lean `Prorate.part_within_a_unit` does that arithmetic, for any
/// parts.
///
/// At `i32` (the kernel is generic over [`kernel::Units`]), 1 or 2 months,
/// totals to `max_total`, a unit fixed per harness: a property relating a
/// product to a quotient is costly for the solver, and at `i64` these ran
/// past twenty minutes (docs/verification.md). The bridge checks the `i32`
/// kernel against the engine's `i128` one. Returns (charge, total, num,
/// den) for the callers' covers; `None` on the last month's last day.
fn charge_is_within_half_a_unit(
    last: bool,
    unit: i32,
    max_total: i32,
) -> Option<(i64, i64, i64, i64)> {
    let total: i32 = kani::any();
    kani::assume(total >= 0 && total <= max_total);
    let months: u32 = kani::any();
    kani::assume(months >= 1 && months <= 2);
    let part = any_part();
    let whole = months as i32 * MONTH_L as i32;
    let before: i32 = kani::any();
    kani::assume(before >= 0 && before <= whole);
    let k: u32 = kani::any();
    kani::assume(k <= part.days);
    let rate = (MONTH_L / part.month_days) as i32;
    // A month but the last ends inside the span.
    kani::assume(last || before + k as i32 * rate <= whole);
    let c = kernel::prorated_through(total, months, before, part, k, last, unit)
        .expect("well formed, in the span, and no overflow at these sizes");
    assert!(0 <= c && c <= total);
    if last && k == part.days {
        assert_eq!(c, total);
        return None;
    }
    assert!(c % unit == 0 || c == total);
    let (total, whole, before, k, unit) = (
        i64::from(total),
        i64::from(whole),
        i64::from(before),
        i64::from(k),
        i64::from(unit),
    );
    let (num, den) = if last {
        let d = i64::from(part.days);
        (total * (before * d + (whole - before) * k), whole * d)
    } else {
        (total * (before + k * i64::from(rate)), whole)
    };
    let c = i64::from(c);
    let e = 2 * (c * den - num);
    assert!(e <= unit * den && e > -(unit * den));
    kani::cover!(c > 0 && c < total, "a charge strictly inside the total");
    kani::cover!(e == unit * den, "a half-unit tie rounds up");
    Some((c, total, num, den))
}

/// The cap reached: a total that isn't a whole number of units, whose
/// exact share rounds above it before the last day, is charged itself.
fn cover_the_cap(got: Option<(i64, i64, i64, i64)>) {
    kani::cover!(
        got.is_some_and(|(c, total, num, den)| c == total && total % 10 != 0 && num < total * den),
        "the charge capped at a total finer than its unit"
    );
}

/// [`charge_is_within_half_a_unit`] for a month but the last, in cents
/// (totals to 2,048 units).
#[kani::proof]
fn a_months_charge_is_within_half_a_cent() {
    charge_is_within_half_a_unit(false, 1, 2_048);
}

/// [`charge_is_within_half_a_unit`] for a month but the last, in cents of
/// an amount kept in tenths of a cent (an asset an earlier version took).
#[kani::proof]
fn a_months_charge_in_tenths_is_within_half_a_cent() {
    cover_the_cap(charge_is_within_half_a_unit(false, 10, 2_048));
}

/// [`charge_is_within_half_a_unit`] for the span's last month, in cents
/// (totals to 64 units: its numerator is the total times up to 31 months'
/// worth of MONTH_L-ths).
#[kani::proof]
fn the_last_months_charge_is_within_half_a_cent() {
    charge_is_within_half_a_unit(true, 1, 64);
}

/// [`charge_is_within_half_a_unit`] for the span's last month, in cents of
/// an amount kept in tenths of a cent.
#[kani::proof]
fn the_last_months_charge_in_tenths_is_within_half_a_cent() {
    cover_the_cap(charge_is_within_half_a_unit(true, 10, 64));
}

/// Malformed input is refused, never a panic: a negative total, no months,
/// a unit that isn't positive, a part no month has, a day past the part,
/// a weight before it outside the span, or a part but the last ending past
/// it.
#[kani::proof]
fn prorated_through_refuses_malformed_input() {
    let total: i64 = kani::any();
    kani::assume(total.unsigned_abs() <= 1 << 20);
    let months: u32 = kani::any();
    kani::assume(months <= 3);
    let before: i64 = kani::any();
    kani::assume(before.unsigned_abs() <= 4 * u64::from(MONTH_L));
    let part = MonthPart {
        days: kani::any(),
        month_days: kani::any(),
    };
    kani::assume(part.days <= 40 && part.month_days <= 40);
    let k: u32 = kani::any();
    kani::assume(k <= 40);
    let last: bool = kani::any();
    let unit: i64 = kani::any();
    kani::assume(unit.unsigned_abs() <= 10);
    let got = kernel::prorated_through(total, months, before, part, k, last, unit);
    let whole = i64::from(months) * i64::from(MONTH_L);
    let malformed = total < 0
        || months == 0
        || unit <= 0
        || !(28..=31).contains(&part.month_days)
        || part.days == 0
        || part.days > part.month_days
        || k > part.days
        || before < 0
        || before > whole;
    kani::cover!(malformed, "malformed input");
    kani::cover!(
        !malformed && got.is_none(),
        "a part but the last past the span"
    );
    if malformed {
        assert!(got.is_none());
    }
    if !malformed {
        // At these sizes nothing overflows: only a part but the last that
        // ends past the span is refused.
        let ends_past =
            !last && before + i64::from(k) * i64::from(MONTH_L / part.month_days) > whole;
        assert_eq!(got.is_none(), ends_past);
    }
    kani::cover!(got.is_some(), "well-formed input is computed");
}

// ---------------------------------------------------------------------------
// Catching up and truing up (B-15)
// ---------------------------------------------------------------------------

/// A catch-up run posts max(0, min gap), never negative, and never carries
/// the books past the schedule at a month end already posted.
#[kani::proof]
#[kani::unwind(5)]
fn catch_up_never_overtakes_the_schedule() {
    let n = any_len();
    kani::assume(n >= 1 && n <= PR);
    let scheduled: [i64; PR] = core::array::from_fn(|_| any_amount());
    let posted: [i64; PR] = core::array::from_fn(|_| any_amount());
    let got = kernel::catch_up(&scheduled[..n], &posted[..n]).expect("in range");
    assert!(got >= 0);
    let mut least = scheduled[0] - posted[0];
    for i in 0..n {
        least = least.min(scheduled[i] - posted[i]);
    }
    assert_eq!(got, least.max(0));
    // When the books carry no more than the schedule anywhere, the run
    // keeps it so at every month end.
    if least >= 0 {
        for i in 0..n {
            assert!(posted[i] + got <= scheduled[i]);
        }
    }
    kani::cover!(n == PR && got > 0, "a catch-up across three month ends");
}

/// What a run of every month posts at once (a viewer's one pass) is the
/// catch-up of every suffix, month by month.
#[kani::proof]
#[kani::unwind(5)]
fn catch_ups_is_every_suffixs_catch_up() {
    let n = any_len();
    kani::assume(n >= 1 && n <= PR);
    let scheduled: [i64; PR] = core::array::from_fn(|_| any_amount());
    let posted: [i64; PR] = core::array::from_fn(|_| any_amount());
    let mut out = [0i64; PR];
    kernel::catch_ups(&scheduled[..n], &posted[..n], &mut out[..n]).expect("in range");
    for i in 0..n {
        assert_eq!(
            Some(out[i]),
            kernel::catch_up(&scheduled[i..n], &posted[i..n])
        );
    }
    kani::cover!(
        n == PR && out[0] > 0 && out[0] < out[1],
        "distinct positive suffixes"
    );
}

/// A disposal's true-up reaches the target from what the books carry: up
/// or down, never both, both non-negative.
#[kani::proof]
fn true_up_reaches_the_target() {
    let target = any_amount();
    let posted = any_amount();
    let (up, down) = kernel::true_up(target, posted).expect("in range");
    assert!(up >= 0 && down >= 0);
    assert!(up == 0 || down == 0);
    assert_eq!(posted + up - down, target);
    kani::cover!(up > 0, "depreciation still due");
    kani::cover!(down > 0, "depreciation past the day, credited back");
}

/// `kernel::basis_figures` (rules.md B-25), for every rule and every
/// cost, adjusted basis and value within the bounds: a purchase
/// depreciates from cost with no §723 basis or built-in gain; a
/// contribution's §723 basis is the adjusted basis and its built-in gain
/// the value less it (negative exactly when the basis exceeds the value);
/// carryover depreciates from the basis, personal use from the lesser of
/// value and basis (at most each, equal to one); no overflow is reachable.
#[kani::proof]
fn basis_figures_laws() {
    use crate::assets::BasisRule;
    let r: u8 = kani::any();
    kani::assume(r < 3);
    let rule = match r {
        0 => BasisRule::Purchase,
        1 => BasisRule::Carryover723,
        _ => BasisRule::PersonalUseConversion,
    };
    let (cost, adjusted, fmv) = (any_amount(), any_amount(), any_amount());
    kani::assume(cost > 0 && adjusted >= 0 && fmv >= 0);
    let f = kernel::basis_figures(rule, cost, adjusted, fmv).expect("no overflow");
    match rule {
        BasisRule::Purchase => {
            assert_eq!(f.depreciable_basis, cost);
            assert!(f.tax_basis_723.is_none() && f.built_in_gain_704c.is_none());
        }
        _ => {
            assert_eq!(f.tax_basis_723, Some(adjusted));
            let g = f.built_in_gain_704c.expect("a contribution's gain");
            assert_eq!(g, fmv - adjusted);
            assert_eq!(g < 0, adjusted > fmv);
            if rule == BasisRule::Carryover723 {
                assert_eq!(f.depreciable_basis, adjusted);
            } else {
                assert!(f.depreciable_basis <= adjusted && f.depreciable_basis <= fmv);
                assert!(f.depreciable_basis == adjusted || f.depreciable_basis == fmv);
            }
        }
    }
    kani::cover!(
        rule == BasisRule::PersonalUseConversion && fmv < adjusted,
        "personal use worth less than its basis: a built-in loss"
    );
}

// ---------------------------------------------------------------------------
// The checker itself
// ---------------------------------------------------------------------------

/// A canary for a known Kani bug: Kani 0.68 (CBMC 6.11) mis-models a bulk
/// copy whose length is symbolic, so this correct copy "fails"
/// (model-checking/kani#4904, closed as a duplicate, unfixed in 0.68). The
/// bug is unsound in both directions, so no harness above copies a slice
/// of symbolic length. This harness is expected to fail; when a Kani
/// upgrade fixes the bug it will start passing, `should_panic` will then
/// report it, and this canary and the comment in `reversal_cancels` can go.
#[kani::proof]
#[kani::unwind(10)]
#[kani::should_panic]
fn kani_symbolic_copy_canary() {
    let src: [i64; N] = kani::any();
    let n = any_len();
    let mut dst = [0i64; 2 * N];
    dst[..n].copy_from_slice(&src[..n]);
    for i in 0..n {
        assert_eq!(dst[i], src[i]);
    }
}
