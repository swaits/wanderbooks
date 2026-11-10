//! K-U10: what ranking the leftover cents by the exact remainder moves.
//!
//! `split_by_weights_into` used to rank its largest remainders by
//! `Decimal`'s rounded quotient (28 significant digits), so an exact tie
//! was decided by the quotient's last digit; it now ranks the exact
//! integer remainder `whole x weight - part x total`, ties to the earlier
//! part. This file keeps the old ranking as a reference (`old_split`,
//! built from the kernel's own public `share_of`, `round_places` and
//! `apportion`, exactly as the kernel built it) and measures, over
//! realistic inputs, how often the two differ and by how much. Run it
//! with `--nocapture` to see the table the owner's decision rests on:
//!
//! ```text
//! cargo test -p ledger-core --test kernel_bridge ties -- --nocapture
//! ```
//!
//! Realistic inputs: whole-cent amounts up to 1,000,000.00, two to five
//! parties, weights that are whole percentages or small integers.

use ledger_core::kernel::{
    Leftover, Rounding, apportion, round_places, share_of, split_by_weights_with,
};
use rust_decimal::Decimal;

fn cents(c: i64) -> Decimal {
    Decimal::new(c, 2)
}

/// The ranking the kernel had before K-U10: each remainder is the rounded
/// quotient less its rounded-down part.
fn old_split(whole: Decimal, weights: &[Decimal], total: Decimal) -> Vec<Decimal> {
    let shares: Vec<Decimal> = weights
        .iter()
        .map(|&w| share_of(whole, w, total).unwrap())
        .collect();
    let mut parts: Vec<Decimal> = shares
        .iter()
        .map(|&s| round_places(s, 2, Rounding::TowardZero).unwrap())
        .collect();
    let rems: Vec<Decimal> = shares.iter().zip(&parts).map(|(s, p)| s - p).collect();
    apportion(whole, &mut parts, &rems, cents(1)).unwrap();
    parts
}

fn new_split(whole: Decimal, weights: &[Decimal], total: Decimal) -> Vec<Decimal> {
    split_by_weights_with(
        whole,
        weights,
        total,
        cents(1),
        |x| round_places(x, 2, Rounding::TowardZero),
        Leftover::LargestRemainder,
    )
    .unwrap()
}

/// True when two parts have an equal exact remainder `whole x w - part x
/// total` (before the leftover is handed out): the only place the two
/// rankings can part.
fn has_exact_tie(whole: Decimal, weights: &[Decimal], total: Decimal) -> bool {
    let rems: Vec<Decimal> = weights
        .iter()
        .map(|&w| {
            let share =
                round_places(share_of(whole, w, total).unwrap(), 2, Rounding::TowardZero).unwrap();
            // The exact floor: a quotient that rounded up across a cent is
            // taken back, as `share_floor_with` does.
            let mut part = share;
            while part * total > whole * w {
                part -= cents(1);
            }
            whole * w - part * total
        })
        .collect();
    (0..rems.len()).any(|i| (i + 1..rems.len()).any(|j| rems[i] == rems[j]))
}

/// A small deterministic generator (xorshift64*): the sweep is the same on
/// every machine and every run.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[derive(Default)]
struct Tally {
    runs: u64,
    differ: u64,
    max_moved_cents: i64,
    examples: Vec<String>,
}

impl Tally {
    fn run(&mut self, whole: Decimal, weights: &[Decimal]) {
        let total: Decimal = weights.iter().copied().sum();
        let old = old_split(whole, weights, total);
        let new = new_split(whole, weights, total);
        self.runs += 1;
        // Both sum to the whole, exactly, whatever the ranking.
        assert_eq!(old.iter().copied().sum::<Decimal>(), whole);
        assert_eq!(new.iter().copied().sum::<Decimal>(), whole);
        if old == new {
            return;
        }
        self.differ += 1;
        // They differ only where an exact tie gave the ranking a choice,
        // and by at most one cent a part.
        assert!(
            has_exact_tie(whole, weights, total),
            "{whole} by {weights:?}: differ without an exact tie"
        );
        for (o, n) in old.iter().zip(&new) {
            let moved = ((o - n).abs() * Decimal::from(100)).trunc();
            let moved = i64::try_from(moved.mantissa()).unwrap();
            assert!(moved <= 1, "{whole} by {weights:?}: moved {moved} cents");
            self.max_moved_cents = self.max_moved_cents.max(moved);
        }
        if self.examples.len() < 3 {
            let show = |v: &[Decimal]| {
                v.iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join(" / ")
            };
            self.examples.push(format!(
                "{whole} by {} -> was {}, now {}",
                weights
                    .iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join("/"),
                show(&old),
                show(&new)
            ));
        }
    }

    fn report(&self, name: &str) {
        println!(
            "{name:<44} {:>8} runs {:>8} differ  {:>7.3}%  max {} cent/part",
            self.runs,
            self.differ,
            100.0 * self.differ as f64 / self.runs as f64,
            self.max_moved_cents
        );
        for e in &self.examples {
            println!("    e.g. {e}");
        }
    }
}

const RUNS: u64 = 60_000;

/// A random whole of 0.01 to 1,000,000.00; `dollars` rounds it to whole
/// dollars (round amounts are the realistic ones, and tie more).
fn random_whole(rng: &mut Rng, dollars: bool) -> Decimal {
    let c = 1 + rng.below(100_000_000) as i64;
    if dollars {
        cents(((c + 99) / 100) * 100)
    } else {
        cents(c)
    }
}

fn random_percentages(rng: &mut Rng) -> Vec<Decimal> {
    let n = 2 + rng.below(4) as usize;
    // A random composition of 100 into n positive whole percentages.
    let mut cuts: Vec<u64> = Vec::new();
    while cuts.len() < n - 1 {
        let c = 1 + rng.below(99);
        if !cuts.contains(&c) {
            cuts.push(c);
        }
    }
    cuts.sort_unstable();
    let mut prev = 0;
    let mut out = Vec::new();
    for c in cuts.iter().copied().chain(std::iter::once(100)) {
        out.push(Decimal::from(c - prev));
        prev = c;
    }
    out
}

fn random_small_integers(rng: &mut Rng) -> Vec<Decimal> {
    let n = 2 + rng.below(4) as usize;
    (0..n).map(|_| Decimal::from(1 + rng.below(5))).collect()
}

/// The splits an agreement is likely to name.
const COMMON: &[&[i64]] = &[
    &[50, 50],
    &[60, 40],
    &[70, 30],
    &[75, 25],
    &[80, 20],
    &[90, 10],
    &[34, 33, 33],
    &[40, 30, 30],
    &[50, 25, 25],
    &[60, 20, 20],
    &[25, 25, 25, 25],
    &[40, 20, 20, 20],
    &[20, 20, 20, 20, 20],
];

/// Equal weights: every remainder is `w x (whole - n x part)`, equal for
/// every part, so both rankings give the leftover cents to the earliest
/// parts and never differ. Two 50/50 members, thirds, quarters: no cent
/// moves, ever.
#[test]
fn equal_weights_never_differ() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut t = Tally::default();
    for i in 0..RUNS {
        let n = 2 + rng.below(4) as usize;
        let w = Decimal::from(1 + rng.below(100));
        let weights = vec![w; n];
        t.run(random_whole(&mut rng, i % 2 == 0), &weights);
    }
    t.report("equal weights (2-5 parties)");
    assert_eq!(t.differ, 0);
}

/// Two parties never differ, whatever the weights. With two parts the
/// leftover is at most one cent, and the two remainders sum to `total` times
/// that cent, so they tie only at exactly half a cent over `total` each:
/// both shares then end in exactly half a cent, the quotient terminates,
/// and the rounded quotient ranks them as the exact one does. A two-member
/// company (the live books' shape) is therefore unmoved by K-U10.
#[test]
fn two_parties_never_differ() {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let mut t = Tally::default();
    for i in 0..RUNS {
        let weights = [
            Decimal::from(1 + rng.below(10_000)),
            Decimal::from(1 + rng.below(10_000)),
        ];
        t.run(random_whole(&mut rng, i % 2 == 0), &weights);
    }
    // And the smallest weights, where ties are the most frequent.
    for a in 1..=12u64 {
        for b in 1..=12u64 {
            for c in 1..=300 {
                t.run(cents(c), &[Decimal::from(a), Decimal::from(b)]);
            }
        }
    }
    t.report("two parties, any weights");
    assert_eq!(t.differ, 0);
}

/// The measurement: how often the old and new rankings differ, by class
/// of weights and of amount. Asserts only the structure (both sum to the
/// whole; a difference needs an exact tie and is at most a cent a part);
/// the rates are printed for the owner.
#[test]
fn what_exact_tie_ranking_moves() {
    println!();
    for dollars in [false, true] {
        let kind = if dollars {
            "whole dollars"
        } else {
            "any cents"
        };
        let mut rng = Rng(0xD1B5_4A32_D192_ED03);

        let mut t = Tally::default();
        for _ in 0..RUNS {
            let w = random_percentages(&mut rng);
            t.run(random_whole(&mut rng, dollars), &w);
        }
        t.report(&format!("whole percentages, {kind}"));

        let mut t = Tally::default();
        for _ in 0..RUNS {
            let w: Vec<Decimal> = COMMON[rng.below(COMMON.len() as u64) as usize]
                .iter()
                .map(|&p| Decimal::from(p))
                .collect();
            t.run(random_whole(&mut rng, dollars), &w);
        }
        t.report(&format!("common agreements, {kind}"));

        let mut t = Tally::default();
        for _ in 0..RUNS {
            let w = random_small_integers(&mut rng);
            t.run(random_whole(&mut rng, dollars), &w);
        }
        t.report(&format!("small integers 1-5, {kind}"));
    }
    // The K-1 property split's real shape: weights are the members'
    // credits in one entry (whole dollars to 10,000), the whole at most
    // their sum.
    let mut rng = Rng(0xA076_1D64_78BD_642F);
    let mut t = Tally::default();
    for _ in 0..RUNS {
        let n = 2 + rng.below(4) as usize;
        let w: Vec<Decimal> = (0..n)
            .map(|_| Decimal::from(1 + rng.below(10_000)))
            .collect();
        let total: Decimal = w.iter().copied().sum();
        let pool = cents(1 + rng.below(total.mantissa() as u64 * 100) as i64);
        t.run(pool, &w);
    }
    t.report("K-1 credits, whole dollars to 10,000");
    // The 4/1/1 shape, every whole-dollar amount to 100.
    let mut t = Tally::default();
    for d in 1..=100 {
        t.run(
            Decimal::from(d),
            &[Decimal::from(4), Decimal::ONE, Decimal::ONE],
        );
    }
    t.report("4/1/1, $1 to $100 whole dollars");
}

/// The pinned examples, in dollars and cents, with both answers.
#[test]
fn the_pinned_examples_move_as_the_owner_was_told() {
    let w = [Decimal::from(4), Decimal::ONE, Decimal::ONE];
    let six = Decimal::from(6);
    let show = |v: Vec<Decimal>| v.iter().map(Decimal::to_string).collect::<Vec<_>>();
    assert_eq!(
        show(old_split(cents(2000), &w, six)),
        ["13.33", "3.34", "3.33"]
    );
    assert_eq!(
        show(new_split(cents(2000), &w, six)),
        ["13.34", "3.33", "3.33"]
    );
}

/// Weights of `places` places that sum to `total` exactly: `n - 1` of
/// `total / n` rounded down, and a last one taking what is left. Built on
/// the mantissas: `Decimal` cannot hold the running sum of percents of 27
/// places (88.888...8 has 29 digits and passes its 96 bits), and would
/// round it without a word, which is the fault K-U10 closed in the kernel.
fn even_weights(total: i128, n: u32, places: u32) -> Vec<Decimal> {
    let whole = total * 10i128.pow(places);
    let w = whole / i128::from(n);
    let last = whole - w * i128::from(n - 1);
    let mut weights = vec![Decimal::from_i128_with_scale(w, places); n as usize];
    *weights.last_mut().unwrap() = Decimal::from_i128_with_scale(last, places);
    assert_eq!(weights.iter().map(Decimal::mantissa).sum::<i128>(), whole);
    weights
}

/// K-U10 made the kernel's product exact or refused. A split by shares
/// of 28 places that sum to one (as a convention's shares do) or by
/// percents of 27 places that sum to 100 (thirds, sevenths, ninths) has
/// products past `Decimal`'s 28 digits, so the ranking uses the rounding
/// product there: the split is made, not refused, the parts sum to the
/// whole, and each is within a cent of its share. A negative whole is
/// rounded down, as apportioning requires (the parts rounded toward zero
/// would pass the whole, which `apportion` refuses).
#[test]
fn a_split_by_weights_of_27_places_is_made_not_refused() {
    let cent = cents(1);
    for (total, places) in [(1i128, 28u32), (100, 27)] {
        for n in [3u32, 6, 7, 9, 11] {
            let weights = even_weights(total, n, places);
            let total = Decimal::from_i128_with_scale(total, 0);
            let amounts = (1..=400_000i64)
                .step_by(37)
                .chain([23_769, 99_999_999_999_999_999, 100_000_000_000_000_000])
                .flat_map(|c| [c, -c]);
            for c in amounts {
                let whole = cents(c);
                let mode = if c < 0 {
                    Rounding::TowardNegativeInfinity
                } else {
                    Rounding::TowardZero
                };
                let parts = split_by_weights_with(
                    whole,
                    &weights,
                    total,
                    cent,
                    |x| round_places(x, 2, mode),
                    Leftover::LargestRemainder,
                )
                .unwrap_or_else(|| panic!("{whole} by {n} parts of {places} places was refused"));
                assert_eq!(
                    parts.iter().copied().sum::<Decimal>(),
                    whole,
                    "{whole} by {n}"
                );
                for (p, &wt) in parts.iter().zip(&weights) {
                    let share = share_of(whole, wt, total).unwrap();
                    assert!(
                        (*p - share).abs() < cent,
                        "{whole} by {n}: {p} for a share of {share}"
                    );
                }
            }
        }
    }
}
