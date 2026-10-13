//! Kani proof harnesses for the accounting kernel.
//!
//! `cargo kani -p ledger-core` (or `just prove`) model-checks each
//! harness: CBMC explores *every* input within the stated bounds, not a
//! sample, and fails if any one of them breaks an assertion, panics, or
//! overflows. The code under proof is the engine's own [`crate::kernel`],
//! instantiated at `i64` (exact integer money, in the smallest unit); the
//! engine runs the same functions on `Decimal`, and the `kernel_bridge`
//! property test checks the two instantiations agree.
//!
//! Bounds: entries of up to [`N`] lines over up to [`K`] accounts, and
//! amounts up to 2^40 in magnitude. The Lean model in `verify/lean`
//! proves the same laws for any number of lines and accounts.

use crate::kernel::{self, Amount, LineError};
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
/// strictly larger remainder goes without (Stephen, 2026-10-13: no member
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
