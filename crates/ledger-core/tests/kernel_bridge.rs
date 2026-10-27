//! The bridge between the proofs and the engine.
//!
//! The Kani harnesses (`src/proofs.rs`) prove the accounting kernel on
//! `i64` money in the smallest unit; the engine runs the same kernel on
//! `Decimal`. These properties check the two instantiations agree, value
//! for value, on cents: whatever Kani proved of the `i64` kernel holds of
//! the `Decimal` one for the same amounts.

use ledger_core::kernel::{self, Amount, LineError};
use ledger_core::{AccountType, Side};
use proptest::prelude::*;
use rust_decimal::Decimal;

/// Cents within both instantiations' line range.
const MAX_CENTS: i64 = 1 << 40;

fn dec(cents: i64) -> Decimal {
    Decimal::new(cents, 2)
}

fn arb_side() -> impl Strategy<Value = Side> {
    prop_oneof![Just(Side::Debit), Just(Side::Credit)]
}

fn arb_type() -> impl Strategy<Value = AccountType> {
    prop_oneof![
        Just(AccountType::Asset),
        Just(AccountType::Liability),
        Just(AccountType::Equity),
        Just(AccountType::Revenue),
        Just(AccountType::Expense),
    ]
}

fn arb_cents() -> impl Strategy<Value = i64> {
    prop_oneof![
        -MAX_CENTS..=MAX_CENTS,
        // Small amounts collide, so balanced entries and zeros show up.
        -3i64..=3,
    ]
}

/// Lines over four accounts. Balanced entries are rare at random, so half
/// the time the last line plugs the difference.
fn arb_lines() -> impl Strategy<Value = Vec<(u8, Side, i64)>> {
    (
        prop::collection::vec((0u8..4, arb_side(), arb_cents()), 0..7),
        any::<bool>(),
    )
        .prop_map(|(mut lines, plug)| {
            if plug && !lines.is_empty() {
                let net: i64 = lines[..lines.len() - 1]
                    .iter()
                    .map(|&(_, s, a)| if s == Side::Debit { a } else { -a })
                    .sum();
                let last = lines.len() - 1;
                if net.abs() > MAX_CENTS {
                    // Out of both ranges, where the two differ by design.
                    return lines;
                }
                lines[last].1 = if net > 0 { Side::Credit } else { Side::Debit };
                lines[last].2 = net.abs();
            }
            lines
        })
}

fn to_dec(lines: &[(u8, Side, i64)]) -> Vec<(u8, Side, Decimal)> {
    lines.iter().map(|&(k, s, a)| (k, s, dec(a))).collect()
}

fn sides<T: Copy>(lines: &[(u8, Side, T)]) -> Vec<(Side, T)> {
    lines.iter().map(|&(_, s, a)| (s, a)).collect()
}

fn map_err(e: LineError<i64>) -> LineError<Decimal> {
    match e {
        LineError::TooFewLines => LineError::TooFewLines,
        LineError::ZeroAmount => LineError::ZeroAmount,
        LineError::NegativeAmount => LineError::NegativeAmount,
        LineError::OutOfRange => LineError::OutOfRange,
        LineError::Unbalanced { debits, credits } => LineError::Unbalanced {
            debits: dec(debits),
            credits: dec(credits),
        },
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn check_lines_agrees(lines in arb_lines()) {
        let i = kernel::check_lines(&sides(&lines)).map_err(map_err);
        let d = kernel::check_lines(&sides(&to_dec(&lines)));
        prop_assert_eq!(i, d);
    }

    #[test]
    fn sums_agree(lines in arb_lines()) {
        let (i, d) = (sides(&lines), sides(&to_dec(&lines)));
        let ti = kernel::totals(i.iter().copied()).unwrap();
        let td = kernel::totals(d.iter().copied()).unwrap();
        prop_assert_eq!((dec(ti.0), dec(ti.1)), td);
        prop_assert_eq!(
            kernel::net(i.iter().copied()).map(dec),
            kernel::net(d.iter().copied())
        );
        let mi: Vec<_> = kernel::movements(lines.iter().copied())
            .unwrap()
            .into_iter()
            .map(|(k, m)| (k, dec(m)))
            .collect();
        prop_assert_eq!(Some(mi), kernel::movements(to_dec(&lines)));
    }

    #[test]
    fn reversal_agrees(lines in arb_lines()) {
        let ri: Vec<_> = kernel::reversal(&lines)
            .into_iter()
            .map(|(k, s, a)| (k, s, dec(a)))
            .collect();
        prop_assert_eq!(ri, kernel::reversal(&to_dec(&lines)));
    }

    /// The 1099 kernel (rules.md B-19): a payment split into its buckets,
    /// and netted, the same at both types.
    #[test]
    fn payment_buckets_agree(lines in arb_lines()) {
        let bucket = |k: u8| match k {
            0 => Some(kernel::Bucket::Counts),
            1 => Some(kernel::Bucket::Card),
            2 => Some(kernel::Bucket::Undetermined),
            _ => None,
        };
        let i: Vec<_> = lines.iter().map(|&(k, s, a)| (bucket(k), s, a)).collect();
        let d: Vec<_> = lines.iter().map(|&(k, s, a)| (bucket(k), s, dec(a))).collect();
        let si = kernel::split_payment(&i).unwrap();
        let sd = kernel::split_payment(&d).unwrap();
        prop_assert_eq!(si.map(dec), sd);
        prop_assert_eq!(
            kernel::net_buckets(si).map(|b| b.map(dec)),
            kernel::net_buckets(sd)
        );
    }

    #[test]
    fn balances_agree(t in arb_type(), m in arb_cents()) {
        prop_assert_eq!(kernel::normal_balance(t, m).map(dec), kernel::normal_balance(t, dec(m)));
        let ci = kernel::columns(m).unwrap();
        prop_assert_eq!(Some((dec(ci.0), dec(ci.1))), kernel::columns(dec(m)));
        prop_assert_eq!(
            kernel::closing_line(m).map(|(s, a)| (s, dec(a))),
            kernel::closing_line(dec(m))
        );
        prop_assert_eq!(
            kernel::credit_line(m).map(|(s, a)| (s, dec(a))),
            kernel::credit_line(dec(m))
        );
    }

    #[test]
    fn plug_last_agrees(whole in arb_cents(), parts in prop::collection::vec(arb_cents(), 0..5)) {
        let mut pi = parts.clone();
        let mut pd: Vec<Decimal> = parts.iter().copied().map(dec).collect();
        kernel::plug_last(whole, &mut pi).unwrap();
        kernel::plug_last(dec(whole), &mut pd).unwrap();
        prop_assert_eq!(pi.into_iter().map(dec).collect::<Vec<_>>(), pd);
    }

    #[test]
    fn apportion_agrees(
        parts in prop::collection::vec(arb_cents(), 0..5),
        rems in prop::collection::vec(0i64..100, 5),
        short in 0i64..500,
    ) {
        let n = parts.len();
        let whole: i64 = parts.iter().sum::<i64>() + short;
        let mut pi = parts.clone();
        let mut pd: Vec<Decimal> = parts.iter().copied().map(dec).collect();
        let ri = &rems[..n];
        let rd: Vec<Decimal> = ri.iter().copied().map(dec).collect();
        let oi = kernel::apportion(whole, &mut pi, ri, 100);
        let od = kernel::apportion(dec(whole), &mut pd, &rd, dec(100));
        prop_assert_eq!(oi.is_some(), od.is_some());
        if oi.is_some() {
            prop_assert_eq!(pi.into_iter().map(dec).collect::<Vec<_>>(), pd);
        }
    }

    #[test]
    fn line_range_agrees(cents in arb_cents()) {
        prop_assert_eq!(cents.in_line_range(), dec(cents).in_line_range());
    }

    #[test]
    fn apportion_cumulative_agrees(
        parts in prop::collection::vec(arb_cents(), 0..5),
        rems in prop::collection::vec(0i64..100, 5),
        given in prop::collection::vec(arb_cents(), 5),
        short in 0i64..500,
        unpaired in any::<bool>(),
    ) {
        let n = parts.len();
        let whole: i64 = parts.iter().sum::<i64>() + short;
        // Past both line ranges the two differ by design (i64's is 2^40
        // units, Decimal's a quadrillion).
        prop_assume!(whole.unsigned_abs() <= MAX_CENTS as u64);
        let mut pi = parts.clone();
        let mut pd: Vec<Decimal> = parts.iter().copied().map(dec).collect();
        let ri = &rems[..n];
        let rd: Vec<Decimal> = ri.iter().copied().map(dec).collect();
        // Sometimes one `given` too many: both refuse.
        let gi = &given[..(n + usize::from(unpaired)).min(5)];
        let gd: Vec<Decimal> = gi.iter().copied().map(dec).collect();
        let oi = kernel::apportion_cumulative(whole, &mut pi, ri, 100, gi);
        let od = kernel::apportion_cumulative(dec(whole), &mut pd, &rd, dec(100), &gd);
        prop_assert_eq!(oi.is_some(), od.is_some());
        prop_assert_eq!(pi.into_iter().map(dec).collect::<Vec<_>>(), pd);
    }

    /// periods::allocate_cumulative, on Decimal shares w_i / W (exact:
    /// W is 100 or 10,000), against its integer model at cents: floors
    /// |T|·w_i div W, remainders |T|·w_i mod W, kernel::apportion_cumulative
    /// at i64, a loss as the negated gain (the model Kani's
    /// cumulative_close_never_drifts_a_unit_at_* and Lean's allocW prove).
    #[test]
    fn allocate_cumulative_agrees_with_the_integer_model(
        four_places in any::<bool>(),
        cuts in prop::collection::vec(0i64..=10_000, 0..3),
        closed in -10_000_000i64..10_000_000,
        net in -10_000_000i64..10_000_000,
        spread in prop::collection::vec(-5_000i64..5_000, 3),
    ) {
        // Weights summing to W: the gaps between sorted cut points.
        let big_w: i64 = if four_places { 10_000 } else { 100 };
        let mut points: Vec<i64> = cuts.iter().map(|c| c % (big_w + 1)).collect();
        points.push(0);
        points.push(big_w);
        points.sort_unstable();
        let weights: Vec<i64> = points.windows(2).map(|w| w[1] - w[0]).filter(|&w| w > 0).collect();
        let n = weights.len();
        let scale = if four_places { 4 } else { 2 };
        let shares: Vec<Decimal> = weights.iter().map(|&w| Decimal::new(w, scale)).collect();
        // What earlier closes gave: anything summing to what they closed.
        let mut given: Vec<i64> = spread[..n - 1].to_vec();
        given.push(closed - given.iter().sum::<i64>());
        let total = closed + net;
        let got = ledger_core::periods::allocate_cumulative(
            dec(total),
            &shares,
            &given.iter().copied().map(dec).collect::<Vec<_>>(),
            2,
        )
        .unwrap();
        let magnitude = total.abs();
        let mut parts: Vec<i64> = weights.iter().map(|w| magnitude * w / big_w).collect();
        let rems: Vec<i64> = weights.iter().map(|w| magnitude * w % big_w).collect();
        let against: Vec<i64> = given.iter().map(|&g| if total < 0 { -g } else { g }).collect();
        kernel::apportion_cumulative(magnitude, &mut parts, &rems, 1, &against).unwrap();
        let want: Vec<Decimal> = parts.iter().map(|&p| dec(if total < 0 { -p } else { p })).collect();
        prop_assert_eq!(&got, &want);
        prop_assert_eq!(got.iter().sum::<Decimal>(), dec(net));
        for ((g, p), w) in given.iter().zip(&got).zip(&weights) {
            let drift = (dec(*g) + p) * Decimal::from(big_w) - dec(total) * Decimal::from(*w);
            prop_assert!(drift.abs() < Decimal::from(big_w) / Decimal::from(100), "{drift}");
        }
    }

    /// rules.md B-14 rule 3: the statement of cash flows' per-entry split
    /// (`kernel::entry_flows`, Kani `entry_flows_*`) is the same at both
    /// instantiations, on balanced and unbalanced lines alike.
    #[test]
    fn entry_flows_agree(lines in arb_lines()) {
        let tagged = |a: u8| match a {
            0 => kernel::Flow::Cash,
            1 => kernel::Flow::Operating,
            2 => kernel::Flow::Investing,
            _ => kernel::Flow::Financing,
        };
        let li: Vec<_> = lines.iter().map(|&(k, s, a)| (tagged(k), s, a)).collect();
        let ld: Vec<_> = lines.iter().map(|&(k, s, a)| (tagged(k), s, dec(a))).collect();
        let fi = kernel::entry_flows(&li).map(|f| kernel::EntryFlows {
            cash: dec(f.cash),
            flows: f.flows.map(dec),
            noncash: f.noncash.map(dec),
        });
        prop_assert_eq!(fi, kernel::entry_flows(&ld));
    }

    /// `kernel::apply_cash` (how a section's cash splits among its kinds)
    /// and `kernel::exchanged` agree at both instantiations.
    #[test]
    fn apply_cash_and_exchanged_agree(
        cash in arb_cents(),
        moves in prop::collection::vec(arb_cents(), 0..5),
        d in 0i64..=MAX_CENTS,
        c in 0i64..=MAX_CENTS,
        inflow in 0i64..=MAX_CENTS,
        outflow in 0i64..=MAX_CENTS,
    ) {
        let mut ai = vec![0i64; moves.len()];
        let mut ad = vec![Decimal::ZERO; moves.len()];
        let md: Vec<Decimal> = moves.iter().copied().map(dec).collect();
        let oi = kernel::apply_cash(cash, &moves, &mut ai);
        let od = kernel::apply_cash(dec(cash), &md, &mut ad);
        prop_assert_eq!(oi, od);
        if oi.is_some() {
            prop_assert_eq!(ai.into_iter().map(dec).collect::<Vec<_>>(), ad);
        }
        prop_assert_eq!(
            kernel::exchanged(d, c, inflow, outflow).map(dec),
            kernel::exchanged(dec(d), dec(c), dec(inflow), dec(outflow))
        );
    }

    /// rules.md B-14 rule 11: `kernel::constructive` (what a member or
    /// lender paid or received for the company; Kani
    /// `constructive_nets_to_zero`, `constructive_mirrors`) agrees at both
    /// instantiations.
    #[test]
    fn constructive_agrees(
        paid in any::<bool>(),
        sources in prop::collection::vec(arb_cents(), 0..4),
        uses in prop::collection::vec(arb_cents(), 0..4),
    ) {
        let way = if paid { kernel::OnBehalf::Paid } else { kernel::OnBehalf::Received };
        let (mut fi, mut ti) = (vec![0i64; sources.len()], vec![0i64; uses.len()]);
        let sd: Vec<Decimal> = sources.iter().copied().map(dec).collect();
        let ud: Vec<Decimal> = uses.iter().copied().map(dec).collect();
        let (mut fd, mut td) = (vec![Decimal::ZERO; sd.len()], vec![Decimal::ZERO; ud.len()]);
        let oi = kernel::constructive(way, &sources, &uses, &mut fi, &mut ti);
        let od = kernel::constructive(way, &sd, &ud, &mut fd, &mut td);
        prop_assert_eq!(oi, od);
        prop_assert!(oi.is_some());
        prop_assert_eq!(fi.into_iter().map(dec).collect::<Vec<_>>(), fd);
        prop_assert_eq!(ti.into_iter().map(dec).collect::<Vec<_>>(), td);
    }

    /// rules.md B-35: a party's target, and a reversal's, are the same on
    /// i64 cents and on Decimal.
    #[test]
    fn party_target_agrees(
        capital in prop::option::of((arb_cents(), 0i64..MAX_CENTS)),
        net in arb_cents(),
        own in arb_cents(),
        debits in 0i64..MAX_CENTS,
    ) {
        let as_dec = |t: kernel::PartyTarget<i64>| kernel::PartyTarget {
            amount: dec(t.amount),
            direction: t.direction,
            cash: dec(t.cash),
            in_kind: dec(t.in_kind),
        };
        let ti = kernel::party_target(capital, net, own, debits);
        let td = kernel::party_target(
            capital.map(|(e, p)| (dec(e), dec(p))),
            dec(net),
            dec(own),
            dec(debits),
        );
        prop_assert_eq!(ti.map(as_dec), td);
        // Never a negative zero on Decimal (party_target INVARIANT 6).
        for t in td.iter().chain(td.and_then(kernel::reverse_target).iter()) {
            for v in [t.amount, t.cash, t.in_kind] {
                prop_assert!(!(v.is_zero() && v.is_sign_negative()), "{t:?}");
            }
        }
        let ri = ti.and_then(kernel::reverse_target);
        prop_assert_eq!(ri.map(as_dec), td.and_then(kernel::reverse_target));
    }

    /// The proration Kani proves at i64 is the engine's at i128, part for
    /// part, on calendar-shaped parts (each month 28 to 31 days, 1 to all
    /// of them) and on parts no calendar has, refusals included.
    #[test]
    fn prorate_agrees_at_i64_and_i128(
        total in 0i64..=(1 << 30),
        months in 1u32..=24,
        parts in prop::collection::vec((28u32..=31, 0u32..=32), 1..=26),
        tenths in any::<bool>(),
    ) {
        let parts: Vec<kernel::MonthPart> = parts
            .into_iter()
            .map(|(month_days, days)| kernel::MonthPart { days, month_days })
            .collect();
        let unit = if tenths { 10 } else { 1 };
        let mut oi = vec![0i64; parts.len()];
        let mut ow = vec![0i128; parts.len()];
        let ri = kernel::prorate_by_days(total, months, &parts, unit, &mut oi);
        let rw = kernel::prorate_by_days(i128::from(total), months, &parts, i128::from(unit), &mut ow);
        prop_assert_eq!(ri.is_some(), rw.is_some());
        if ri.is_some() {
            prop_assert_eq!(oi.iter().map(|&o| i128::from(o)).collect::<Vec<_>>(), ow);
        }
    }

    /// The charge Kani proves at i32 (small totals, 1 or 2 months) is the
    /// engine's at i128, through every day of a part, refusals included.
    #[test]
    fn prorated_through_agrees_at_i32_and_i128(
        total in 0i32..=2_048,
        months in 1u32..=2,
        month_days in 28u32..=31,
        days in 0u32..=32,
        k in 0u32..=32,
        before in 0i32..=800_000,
        last in any::<bool>(),
        tenths in any::<bool>(),
    ) {
        let part = kernel::MonthPart { days, month_days };
        let unit = if tenths { 10 } else { 1 };
        let narrow = kernel::prorated_through(total, months, before, part, k, last, unit);
        let wide = kernel::prorated_through(
            i128::from(total), months, i128::from(before), part, k, last, i128::from(unit),
        );
        // At i32 a product past its range is refused; at i128 it isn't.
        if let Some(n) = narrow {
            prop_assert_eq!(Some(i128::from(n)), wide);
        } else if wide.is_some() {
            // Refused at i32 only when a product the kernel forms there
            // overflows it.
            let w = i128::from(months) * i128::from(kernel::MONTH_L);
            let (t, b, k, d, u) = (
                i128::from(total),
                i128::from(before),
                i128::from(k),
                i128::from(days),
                i128::from(unit),
            );
            let formed: Vec<i128> = if last {
                let elapsed = b * d + (w - b) * k;
                vec![b * d, (w - b) * k, elapsed, t * elapsed, w * d, w * d * u]
            } else {
                let elapsed = b + k * i128::from(kernel::MONTH_L / month_days);
                vec![elapsed, t * elapsed, w * u]
            };
            prop_assert!(
                formed.iter().any(|&p| p > i128::from(i32::MAX)),
                "refused at i32 though nothing overflows it: {formed:?}"
            );
        }
    }

    /// catch_up and true_up agree at i64 and on Decimal (the engine's).
    #[test]
    fn catch_up_and_true_up_agree(
        pairs in prop::collection::vec((arb_cents(), arb_cents()), 0..6),
    ) {
        let (si, pi): (Vec<i64>, Vec<i64>) = pairs.iter().copied().unzip();
        let sd: Vec<Decimal> = si.iter().copied().map(dec).collect();
        let pd: Vec<Decimal> = pi.iter().copied().map(dec).collect();
        prop_assert_eq!(
            kernel::catch_up(&si, &pi).map(dec),
            kernel::catch_up(&sd, &pd)
        );
        if let Some(&(t, p)) = pairs.first() {
            prop_assert_eq!(
                kernel::true_up(t, p).map(|(u, d)| (dec(u), dec(d))),
                kernel::true_up(dec(t), dec(p))
            );
        }
    }

    /// kernel::basis_figures at i64 (what Kani's basis_figures_laws
    /// proved) and at Decimal agree, rule by rule.
    #[test]
    fn basis_figures_agree(
        rule in prop_oneof![
            Just(ledger_core::BasisRule::Purchase),
            Just(ledger_core::BasisRule::Carryover723),
            Just(ledger_core::BasisRule::PersonalUseConversion),
        ],
        cost in arb_cents(),
        adjusted in arb_cents(),
        fmv in arb_cents(),
    ) {
        let i = kernel::basis_figures(rule, cost, adjusted, fmv);
        let d = kernel::basis_figures(rule, dec(cost), dec(adjusted), dec(fmv));
        prop_assert_eq!(i.is_some(), d.is_some());
        if let (Some(i), Some(d)) = (i, d) {
            prop_assert_eq!(i.tax_basis_723.map(dec), d.tax_basis_723);
            prop_assert_eq!(dec(i.depreciable_basis), d.depreciable_basis);
            prop_assert_eq!(i.built_in_gain_704c.map(dec), d.built_in_gain_704c);
        }
    }

    /// kernel::discount_max at i64 (what Kani's discount_max_is_bounded
    /// proved) and at Decimal agree where both are exact: the bound is
    /// the same money, never above the total, and result x 10,000 =
    /// total x basis points on both.
    #[test]
    fn discount_max_agrees(
        total in 0..=MAX_CENTS,
        percent_bp in 1u32..=9_999,
    ) {
        let narrow = kernel::discount_max(total, percent_bp);
        let wide = kernel::discount_max(dec(total), percent_bp)
            .expect("a decimal division by 10,000 is exact");
        if let Some(n) = narrow {
            prop_assert_eq!(wide, dec(n));
        }
        prop_assert!(wide >= Decimal::ZERO);
        prop_assert!(wide <= dec(total));
        prop_assert_eq!(
            wide * Decimal::from(10_000u32),
            dec(total) * Decimal::from(percent_bp)
        );
    }

    /// The discount bound Kani proves at i32 (small totals) is the same
    /// bound at i64, which the property above carries to Decimal.
    #[test]
    fn discount_max_i32_agrees_with_i64(
        total in 0i32..=200_000,
        percent_bp in 1u32..=9_999,
    ) {
        prop_assert_eq!(
            kernel::discount_max(total, percent_bp).map(i64::from),
            kernel::discount_max(i64::from(total), percent_bp)
        );
    }

    /// kernel::settle at i64 (what Kani's settle_is_exact proved) and at
    /// Decimal agree: the same cash, discounts and fee give the same
    /// applications total and deposit, and both refuse the same cases.
    #[test]
    fn settle_agrees(
        cash in prop::collection::vec(0..=MAX_CENTS, 0..4),
        discounts in prop::collection::vec(0..=MAX_CENTS, 0..4),
        fee in 0..=MAX_CENTS,
    ) {
        let narrow = kernel::settle(&cash, &discounts, fee);
        let wide = kernel::settle(
            &cash.iter().map(|&c| dec(c)).collect::<Vec<_>>(),
            &discounts.iter().map(|&c| dec(c)).collect::<Vec<_>>(),
            dec(fee),
        );
        prop_assert_eq!(
            narrow.map(|s| (dec(s.applications_total), dec(s.deposit))),
            wide.map(|s| (s.applications_total, s.deposit))
        );
    }

    /// kernel::line_amount at i64 (what Kani's line_amount_is_exact
    /// proved) and at Decimal agree: cents x cents at i64 is the same
    /// product as dollars x dollars at Decimal, to four places.
    #[test]
    fn line_amount_agrees(
        quantity in 0..=1_000_000i64,
        unit_price in 0..=1_000_000i64,
    ) {
        let narrow = kernel::line_amount(quantity, unit_price);
        let wide = kernel::line_amount(dec(quantity), dec(unit_price));
        prop_assert_eq!(narrow.map(|n| Decimal::new(n, 4)), wide);
    }
}
