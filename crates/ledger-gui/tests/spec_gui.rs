//! Spec-blind tests of the viewer's pure logic. Every expected value here
//! comes from a written spec (the doc comments' GOAL/SOURCE/INVARIANTS
//! blocks, docs/spec/sources/file-names.md) or from `ledger-core` itself,
//! never from reading the code under test.

use chrono::NaiveDate;
use ledger_core::{AccountId, AccountType, Currency, EntryId, EntryStatus, Money, Side};
use ledger_gui::books::{Books, Severity};
use ledger_gui::data::{Source, load_once};
use ledger_gui::nav::{Nav, ReportKind, Route};
use ledger_gui::{attention, charts, export, fmt, palette};
use ledger_store::{Snapshot, Store};
use proptest::prelude::*;
use rust_decimal::Decimal;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn neg_zero(scale: u32) -> Decimal {
    Decimal::from_parts(0, 0, 0, true, scale)
}

/// Read formatted money back: strip commas and `$`, parentheses and U+2212
/// become a minus sign.
fn read_back(s: &str) -> Decimal {
    let mut t: String = s.chars().filter(|&c| c != ',' && c != '$').collect();
    let mut negative = false;
    if t.starts_with('(') && t.ends_with(')') {
        negative = true;
        t = t[1..t.len() - 1].to_string();
    }
    if let Some(rest) = t.strip_prefix('\u{2212}') {
        negative = !negative;
        t = rest.to_string();
    }
    let v = Decimal::from_str(&t).unwrap_or_else(|e| panic!("{s:?} does not read back: {e}"));
    if negative { -v } else { v }
}

/// The digits part of a formatted amount: everything but sign, `$`, and
/// parentheses.
fn digits_part(s: &str) -> String {
    s.chars()
        .filter(|&c| !matches!(c, '$' | '(' | ')' | '\u{2212}'))
        .collect()
}

/// Check INVARIANT 1's shape: thousands grouped in threes, at least two
/// decimals.
fn check_grouping(s: &str) -> Result<(), String> {
    let body = digits_part(s);
    let (int, frac) = body
        .split_once('.')
        .ok_or_else(|| format!("{s:?}: no decimal point"))?;
    if frac.len() < 2 || !frac.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("{s:?}: fewer than two decimals"));
    }
    let groups: Vec<&str> = int.split(',').collect();
    let first = groups[0];
    if first.is_empty() || first.len() > 3 || !first.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("{s:?}: bad leading group"));
    }
    if groups.len() > 1 && first.starts_with('0') {
        return Err(format!("{s:?}: leading zero before a comma"));
    }
    for g in &groups[1..] {
        if g.len() != 3 || !g.chars().all(|c| c.is_ascii_digit()) {
            return Err(format!("{s:?}: group {g:?} isn't three digits"));
        }
    }
    if int.len() > 1 && int.starts_with('0') {
        return Err(format!("{s:?}: leading zero"));
    }
    Ok(())
}

/// A random decimal: magnitude below 1e15, scale 0..=10, either sign.
fn arb_decimal() -> impl Strategy<Value = Decimal> {
    (0u32..=10, any::<bool>(), 0u32..=16).prop_flat_map(|(scale, neg, mag_digits)| {
        let max: i128 = 10i128.pow(mag_digits.min(15)) * 10i128.pow(scale);
        (0..=max).prop_map(move |m| {
            let mut d = Decimal::from_i128_with_scale(m, scale);
            d.set_sign_negative(neg);
            d
        })
    })
}

// ---------------------------------------------------------------------------
// 1. fmt
// ---------------------------------------------------------------------------

#[test]
fn fmt_doc_examples() {
    assert_eq!(fmt::money(dec("-1234.56")), "\u{2212}1,234.56");
    assert_eq!(fmt::accounting(dec("-1234.56")), "(1,234.56)");
    assert_eq!(fmt::dollars(dec("1234.56")), "$1,234.56");
    assert_eq!(fmt::dollars(dec("-12")), "\u{2212}$12.00");
    assert_eq!(fmt::dollars(dec("-12.00")), "\u{2212}$12.00");
    assert_eq!(fmt::compact(dec("950")), "$950");
    assert_eq!(fmt::compact(dec("1200")), "$1.2k");
    assert_eq!(fmt::compact(dec("25000")), "$25k");
    assert_eq!(fmt::compact(dec("3400000")), "$3.4M");
}

#[test]
fn fmt_never_rounds_and_keeps_two_decimals() {
    assert_eq!(fmt::money(dec("0.001")), "0.001");
    assert_eq!(
        fmt::money(dec("1234567.1234567891")),
        "1,234,567.1234567891"
    );
    assert_eq!(fmt::money(dec("5")), "5.00");
    assert_eq!(fmt::money(dec("5.1")), "5.10");
    assert_eq!(fmt::money(dec("999.995")), "999.995");
    assert_eq!(fmt::accounting(dec("-0.005")), "(0.005)");
    assert_eq!(fmt::dollars(dec("1000000")), "$1,000,000.00");
}

#[test]
fn fmt_zero_and_negative_zero_are_unsigned() {
    for z in [
        Decimal::ZERO,
        neg_zero(0),
        neg_zero(2),
        neg_zero(7),
        dec("-0.00"),
    ] {
        for s in [fmt::money(z), fmt::accounting(z), fmt::dollars(z)] {
            assert!(
                !s.contains('\u{2212}') && !s.contains('-') && !s.contains('('),
                "zero {z:?} shown signed: {s:?}"
            );
            assert_eq!(read_back(&s), Decimal::ZERO);
        }
        assert_eq!(fmt::money(z), "0.00");
        assert_eq!(fmt::dollars(z), "$0.00");
        let c = fmt::compact(z);
        assert!(!c.contains('\u{2212}') && !c.contains('-'), "{c:?}");
    }
}

#[test]
fn fmt_compact_never_signs_what_rounds_to_zero() {
    for s in ["-0.001", "-0.004", "-0.01", "-0.4", "-0.49", "0.4", "-0"] {
        let c = fmt::compact(dec(s));
        let shows_zero = c.chars().filter(|c| c.is_ascii_digit()).all(|c| c == '0');
        if shows_zero {
            assert!(
                !c.contains('\u{2212}') && !c.contains('-') && !c.contains('('),
                "compact({s}) = {c:?}"
            );
        }
    }
}

#[test]
fn fmt_other_helpers_doc_examples() {
    let d = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
    assert_eq!(fmt::date(d), "Oct 12, 2026");
    assert_eq!(fmt::date_short(d), "Oct 12");
    assert_eq!(fmt::month(d), "Oct 2026");
    assert_eq!(fmt::count(1, "receipt", "receipts"), "1 receipt");
    assert_eq!(fmt::count(3, "receipt", "receipts"), "3 receipts");
    assert_eq!(
        fmt::timestamp("2026-10-11T18:04:00Z"),
        "Oct 11, 2026 18:04 UTC"
    );
    assert!(
        fmt::bytes(12_300).ends_with(" KB"),
        "{}",
        fmt::bytes(12_300)
    );
    assert_eq!(fmt::label("snake_case_key"), "Snake case key");
    assert_eq!(fmt::label("ein"), "EIN");
    assert_eq!(fmt::words("PrepaidExpense"), "Prepaid expense");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// fmt INVARIANTS 1 and 2 for money, accounting, and dollars.
    #[test]
    fn fmt_exact_round_trip(d in arb_decimal()) {
        let m = fmt::money(d);
        let a = fmt::accounting(d);
        let s = fmt::dollars(d);
        for t in [&m, &a, &s] {
            prop_assert_eq!(read_back(t), d, "{:?}", t);
            if let Err(e) = check_grouping(t) {
                prop_assert!(false, "{}", e);
            }
            prop_assert!(!t.contains('-'), "ASCII minus in {:?}", t);
            // Every decimal place the amount has is shown.
            let frac = digits_part(t).split_once('.').unwrap().1.len();
            prop_assert!(frac as u32 >= d.normalize().scale(), "{:?} for {}", t, d);
        }
        prop_assert!(s.contains('$'));
        if d.is_zero() {
            for t in [&m, &a, &s] {
                prop_assert!(!t.contains('\u{2212}') && !t.contains('('), "{:?}", t);
            }
        } else if d.is_sign_negative() {
            prop_assert!(m.starts_with('\u{2212}') && !m.contains('('), "{:?}", m);
            prop_assert!(s.starts_with('\u{2212}') && !s.contains('('), "{:?}", s);
            prop_assert!(a.starts_with('(') && a.ends_with(')') && !a.contains('\u{2212}'), "{:?}", a);
        } else {
            for t in [&m, &a, &s] {
                prop_assert!(!t.contains('\u{2212}') && !t.contains('('), "{:?}", t);
            }
        }
    }

    /// fmt INVARIANT 3: compact never signs what it shows as zero.
    #[test]
    fn fmt_compact_unsigned_zero(d in arb_decimal()) {
        let c = fmt::compact(d);
        let shows_zero = c.chars().filter(|c| c.is_ascii_digit()).all(|c| c == '0');
        if shows_zero {
            prop_assert!(!c.contains('\u{2212}') && !c.contains('-') && !c.contains('('), "compact({}) = {:?}", d, c);
        }
        if d.is_zero() {
            prop_assert!(!c.contains('\u{2212}') && !c.contains('-'), "{:?}", c);
        }
    }
}

// ---------------------------------------------------------------------------
// 2. export::file_name
// ---------------------------------------------------------------------------

const RESERVED_CHARS: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
const DEVICE_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "COM¹", "COM²", "COM³", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8",
    "LPT9", "LPT¹", "LPT²", "LPT³",
];

fn is_device_name(stem: &str) -> bool {
    let up = stem.to_uppercase();
    DEVICE_NAMES.iter().any(|d| d.to_uppercase() == up)
}

fn check_file_name(name: &str, ext: &str) -> Result<String, String> {
    let out = export::file_name(name, ext);
    let suffix = format!(".{ext}");
    let base = out
        .strip_suffix(&suffix)
        .ok_or_else(|| format!("{name:?} -> {out:?}: doesn't end with {suffix:?}"))?;
    if let Some(c) = base
        .chars()
        .find(|c| RESERVED_CHARS.contains(c) || (*c as u32) <= 31)
    {
        return Err(format!("{name:?} -> {out:?}: holds {c:?}"));
    }
    if base.is_empty() || base == "." || base == ".." {
        return Err(format!("{name:?} -> {out:?}: bad base {base:?}"));
    }
    if base.ends_with(' ') || base.ends_with('.') {
        return Err(format!("{name:?} -> {out:?}: base ends with space/period"));
    }
    let stem = base.split('.').next().unwrap_or("");
    if is_device_name(stem) {
        return Err(format!("{name:?} -> {out:?}: device name {stem:?}"));
    }
    if out.len() > 255 {
        return Err(format!("{name:?} -> {} bytes", out.len()));
    }
    let dir = Path::new("/tmp/downloads-dir");
    let joined = dir.join(&out);
    if joined.parent() != Some(dir) {
        return Err(format!("{name:?} -> {out:?}: escapes the directory"));
    }
    if joined.file_name().and_then(|f| f.to_str()) != Some(out.as_str()) {
        return Err(format!("{name:?} -> {out:?}: not one path component"));
    }
    Ok(out)
}

#[test]
fn file_name_adversarial_examples() {
    let long_multibyte = "é".repeat(300);
    let long_cjk = "帳".repeat(300);
    let long_emoji = "📒".repeat(100);
    let long_ascii = "a".repeat(400);
    let names: Vec<&str> = vec![
        "",
        ".",
        "..",
        "...",
        "../../etc/passwd",
        "/etc/passwd",
        "..\\..\\windows",
        "CON",
        "con",
        "con.report",
        "Con.Tar.Gz",
        "NUL",
        "nul.txt",
        "PRN",
        "AUX",
        "COM1",
        "com9",
        "COM¹",
        "LPT1",
        "lpt²",
        "LPT¹",
        "LPT³.x",
        "report. ",
        "report...",
        "report   ",
        " . . ",
        "a<b>c:d\"e/f\\g|h?i*j",
        "tab\there\nnewline\r\u{0}nul\u{1f}",
        "\u{1}\u{2}\u{3}",
        "P&L — Fernhill Labs LLC, Jan–Sep 2026",
        &long_multibyte,
        &long_cjk,
        &long_emoji,
        &long_ascii,
    ];
    for n in names {
        for ext in ["pdf", "csv"] {
            if let Err(e) = check_file_name(n, ext) {
                panic!("{e}");
            }
        }
    }
}

#[test]
fn file_name_device_names_get_a_leading_underscore() {
    for n in ["CON", "con", "LPT¹", "nul", "COM1"] {
        assert_eq!(export::file_name(n, "pdf"), format!("_{n}.pdf"));
    }
    assert_eq!(export::file_name("con.report", "pdf"), "_con.report.pdf");
}

#[test]
fn file_name_long_names_cut_at_char_boundary() {
    let n = "é".repeat(300);
    let out = export::file_name(&n, "pdf");
    assert!(out.len() <= 255, "{} bytes", out.len());
    assert!(out.ends_with(".pdf"));
    let base = out.strip_suffix(".pdf").unwrap();
    assert!(base.chars().all(|c| c == 'é'), "{base:?}");
    assert!(!base.is_empty());
}

/// The spec says "no control character"; the cited source names only 0-31.
/// DEL and C1 controls are checked here separately (spec gap).
#[test]
fn file_name_strips_other_control_characters() {
    let out = export::file_name("a\u{7f}b\u{85}c\u{9b}d", "pdf");
    assert!(!out.chars().any(char::is_control), "{out:?}");
}

fn arb_name() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        any::<String>(),
        "[ .a-zA-Z0-9]{0,8}",
        Just("CON".to_string()),
        Just("lpt¹".to_string()),
        Just("nul".to_string()),
        Just("Com3".to_string()),
        Just("..".to_string()),
        Just("/".to_string()),
        Just("\\".to_string()),
        Just(" ".to_string()),
        Just(".".to_string()),
        "[\\x00-\\x1f<>:\"/\\\\|?*]{1,4}",
        "[é帳📒]{50,120}",
    ];
    prop::collection::vec(piece, 0..5).prop_map(|v| v.concat())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]

    /// export::file_name INVARIANTS 1-3, and the GOAL: never outside the
    /// directory.
    #[test]
    fn file_name_invariants(name in arb_name(), ext in prop_oneof![Just("pdf"), Just("csv"), Just("txt")]) {
        if let Err(e) = check_file_name(&name, ext) {
            prop_assert!(false, "{}", e);
        }
    }
}

// ---------------------------------------------------------------------------
// 3. Nav
// ---------------------------------------------------------------------------

/// Reference browser history, from Nav's INVARIANTS.
#[derive(Debug)]
struct ModelNav {
    back: Vec<Route>,
    current: Route,
    forward: Vec<Route>,
}

impl ModelNav {
    fn go(&mut self, r: Route) {
        if r != self.current {
            let old = std::mem::replace(&mut self.current, r);
            self.back.push(old);
            self.forward.clear();
        }
    }
    fn back(&mut self) {
        if let Some(p) = self.back.pop() {
            let old = std::mem::replace(&mut self.current, p);
            self.forward.push(old);
        }
    }
    fn forward(&mut self) {
        if let Some(n) = self.forward.pop() {
            let old = std::mem::replace(&mut self.current, n);
            self.back.push(old);
        }
    }
}

fn routes() -> Vec<Route> {
    vec![
        Route::Overview,
        Route::Cash,
        Route::Transactions,
        Route::Entry(1),
        Route::Entry(2),
        Route::Reports(ReportKind::ProfitAndLoss),
        Route::Account(7),
    ]
}

#[derive(Clone, Debug)]
enum NavOp {
    Go(usize),
    Back,
    Forward,
}

fn arb_nav_op() -> impl Strategy<Value = NavOp> {
    prop_oneof![
        3 => (0usize..7).prop_map(NavOp::Go),
        2 => Just(NavOp::Back),
        2 => Just(NavOp::Forward),
    ]
}

#[test]
fn nav_examples() {
    let mut n = Nav::new(Route::Overview);
    assert!(!n.can_back() && !n.can_forward());
    n.go(Route::Overview);
    assert!(!n.can_back(), "go to the current route does nothing");
    n.go(Route::Cash);
    n.go(Route::Transactions);
    n.back();
    assert_eq!(n.current, Route::Cash);
    n.go(Route::Entry(3));
    assert!(!n.can_forward(), "going somewhere new drops forward");
    n.back();
    n.forward();
    assert_eq!(n.current, Route::Entry(3));
    n.forward();
    assert_eq!(n.current, Route::Entry(3));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1024))]

    /// Nav INVARIANTS 1-3 against a two-stack model.
    #[test]
    fn nav_matches_browser_model(start in 0usize..7, ops in prop::collection::vec(arb_nav_op(), 0..60)) {
        let rs = routes();
        let mut nav = Nav::new(rs[start].clone());
        let mut model = ModelNav { back: vec![], current: rs[start].clone(), forward: vec![] };
        for op in ops {
            match op {
                NavOp::Go(i) => { nav.go(rs[i].clone()); model.go(rs[i].clone()); }
                NavOp::Back => { nav.back(); model.back(); }
                NavOp::Forward => { nav.forward(); model.forward(); }
            }
            prop_assert_eq!(&nav.current, &model.current, "after {:?}", op);
            prop_assert_eq!(nav.can_back(), !model.back.is_empty());
            prop_assert_eq!(nav.can_forward(), !model.forward.is_empty());
        }
    }
}

// ---------------------------------------------------------------------------
// 4. palette::fuzzy
// ---------------------------------------------------------------------------

/// INVARIANT 1's reference: the query's non-space characters, lower-cased,
/// appear in order in the lower-cased hay.
fn is_subsequence(hay: &str, query: &str) -> bool {
    let hay = hay.to_lowercase();
    let q: Vec<char> = query.to_lowercase().chars().filter(|&c| c != ' ').collect();
    let mut it = hay.chars();
    q.iter().all(|qc| it.any(|h| h == *qc))
}

fn non_space_count(query: &str) -> i32 {
    query.to_lowercase().chars().filter(|&c| c != ' ').count() as i32
}

#[test]
fn fuzzy_examples() {
    assert_eq!(palette::fuzzy("anything at all", ""), Some(0));
    assert_eq!(palette::fuzzy("", ""), Some(0));
    assert!(palette::fuzzy("Office Rent", "rent").is_some());
    assert!(palette::fuzzy("Office Rent", "OR").is_some());
    assert!(palette::fuzzy("Office Rent", "o r").is_some());
    assert!(palette::fuzzy("Office Rent", "tner").is_none());
    assert!(palette::fuzzy("", "a").is_none());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]

    /// INVARIANT 1 (against the reference) and 2 (the lower bound).
    #[test]
    fn fuzzy_matches_subsequence(
        hay in "[abcdeABCDE éÉ._-]{0,16}",
        query in "[abcdeABCDE éÉ._-]{0,5}",
    ) {
        let got = palette::fuzzy(&hay, &query);
        prop_assert_eq!(got.is_some(), is_subsequence(&hay, &query), "hay {:?} query {:?}", hay, query);
        if let Some(score) = got {
            prop_assert!(score >= non_space_count(&query), "score {} for {:?} in {:?}", score, query, hay);
        }
        if query.is_empty() {
            prop_assert_eq!(got, Some(0));
        }
    }

    /// INVARIANT 2: a word-start prefix (the hay starting with the query)
    /// scores at least as high as any other match of the same query.
    #[test]
    fn fuzzy_leading_prefix_scores_highest(
        query in "[a-e]{1,4}",
        tail in "[a-e ]{0,8}",
        other in "[a-e ]{0,16}",
    ) {
        let prefixed = format!("{query}{tail}");
        let best = palette::fuzzy(&prefixed, &query).expect("a prefix matches");
        if let Some(s) = palette::fuzzy(&other, &query) {
            prop_assert!(best >= s, "{:?} scored {} but {:?} scored {}", prefixed, best, other, s);
        }
    }

    /// INVARIANT 2 with the prefix at a later word start.
    #[test]
    fn fuzzy_word_start_prefix_scores_highest(
        query in "[a-e]{1,4}",
        head in "[a-e]{1,6}",
        tail in "[a-e ]{0,6}",
        other in "[a-e ]{0,16}",
    ) {
        let prefixed = format!("{head} {query}{tail}");
        let best = palette::fuzzy(&prefixed, &query).expect("a prefix matches");
        if let Some(s) = palette::fuzzy(&other, &query) {
            prop_assert!(best >= s, "{:?} scored {} but {:?} scored {}", prefixed, best, other, s);
        }
    }
}

// ---------------------------------------------------------------------------
// 5. charts::ticks
// ---------------------------------------------------------------------------

/// `strict`: INVARIANT 2 to the bit. Otherwise coverage may miss by 1e-9 of
/// the axis's magnitude (see the ignored `ticks_cover_exactly`).
fn check_ticks_with(lo: f64, hi: f64, n: usize, strict: bool) -> Result<(), String> {
    let t = charts::ticks(lo, hi, n);
    let ctx = format!("ticks({lo:e}, {hi:e}, {n}) = {t:?}");
    if t.is_empty() {
        return Err(format!("{ctx}: empty"));
    }
    if t.iter().any(|x| !x.is_finite()) {
        return Err(format!("{ctx}: non-finite tick"));
    }
    if t.len() > n + 2 {
        return Err(format!("{ctx}: more than n + 2"));
    }
    // The spec's fallback, allowed only where f64 has no usable step: a
    // span past f64's range, or one too small for the magnitude.
    let fallback = if lo == hi { vec![lo] } else { vec![lo, hi] };
    let no_step = !(hi - lo).is_finite()
        || lo.abs().max(hi.abs()) > f64::MAX / 10.0
        || (hi - lo) < 1e-12 * lo.abs().max(hi.abs());
    if t == fallback && no_step {
        return Ok(());
    }
    let scale = t
        .iter()
        .chain([lo, hi].iter())
        .fold(0f64, |m, x| m.max(x.abs()));
    let slack = if strict { 0.0 } else { 1e-9 * scale };
    if t[0] > lo + slack || *t.last().unwrap() < hi - slack {
        return Err(format!("{ctx}: doesn't cover"));
    }
    if t.len() >= 2 {
        let step = t[1] - t[0];
        if step <= 0.0 {
            return Err(format!("{ctx}: not ascending"));
        }
        for w in t.windows(2) {
            if ((w[1] - w[0]) - step).abs() > 1e-9 * scale.max(step) {
                return Err(format!("{ctx}: uneven"));
            }
        }
        // The step, up to the rounding of differences at this magnitude.
        let tol = 1e-12 * scale + 1e-9 * step;
        let k = step.log10().floor() as i32;
        let nice = (k.saturating_sub(1)..=k.saturating_add(1)).any(|e| {
            [1.0, 2.0, 2.5, 5.0]
                .iter()
                .any(|m| (step - m * 10f64.powi(e)).abs() <= tol)
        });
        if !nice {
            return Err(format!("{ctx}: step {step:e} isn't 1/2/2.5/5 x 10^k"));
        }
    }
    Ok(())
}

fn check_ticks(lo: f64, hi: f64, n: usize) -> Result<(), String> {
    check_ticks_with(lo, hi, n, false)
}

#[test]
fn ticks_examples() {
    for (lo, hi, n) in [
        (0.0, 100.0, 5),
        (0.0, 0.0, 1),
        (5.0, 5.0, 4),
        (-5.0, -5.0, 4),
        (-1234.5, 987.0, 6),
        (0.0, 1e-9, 3),
        (1.0, 1.0 + 1e-6, 12),
        (-1e15, 1e15, 12),
        (1e15, 1e15 + 1.0, 4),
    ] {
        if let Err(e) = check_ticks(lo, hi, n) {
            panic!("{e}");
        }
    }
}

#[test]
fn ticks_extreme_finite_ranges() {
    for (lo, hi, n) in [
        (-1e300, 1e300, 5),
        (f64::MIN, f64::MAX, 5),
        (0.0, f64::MAX, 3),
        (f64::MIN_POSITIVE, f64::MIN_POSITIVE * 2.0, 3),
    ] {
        if let Err(e) = check_ticks(lo, hi, n) {
            panic!("{e}");
        }
    }
}

fn arb_f64() -> impl Strategy<Value = f64> {
    prop_oneof![
        -1e6f64..1e6,
        -1.0f64..1.0,
        (-1e15f64..1e15),
        (-12i32..15, -1.0f64..1.0).prop_map(|(e, m)| m * 10f64.powi(e)),
        Just(0.0),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4096))]

    /// charts::ticks INVARIANTS 1-3.
    #[test]
    fn ticks_invariants(a in arb_f64(), b in arb_f64(), n in 1usize..=12, same in prop::bool::weighted(0.1)) {
        let (lo, hi) = if same { (a, a) } else if a <= b { (a, b) } else { (b, a) };
        if let Err(e) = check_ticks(lo, hi, n) {
            prop_assert!(false, "{}", e);
        }
    }

    /// INVARIANT 2 exactly: the last tick at or above hi, bit for bit.
    #[test]
    fn ticks_cover_exactly(a in arb_f64(), b in arb_f64(), n in 1usize..=12) {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        if let Err(e) = check_ticks_with(lo, hi, n, true) {
            prop_assert!(false, "{}", e);
        }
    }

    /// Tiny spans (down to 1e-10 of the center).
    #[test]
    fn ticks_tiny_spans(center in -1e4f64..1e4, span_exp in -6i32..0, n in 1usize..=12) {
        let lo = center;
        let hi = center + 10f64.powi(span_exp);
        if let Err(e) = check_ticks(lo, hi, n) {
            prop_assert!(false, "{}", e);
        }
    }

    /// Tiny spans around a large center, near f64's resolution.
    #[test]
    fn ticks_tiny_spans_large_center(center in -1e9f64..1e9, span_exp in -6i32..0, n in 1usize..=12) {
        let lo = center;
        let hi = center + 10f64.powi(span_exp);
        if let Err(e) = check_ticks(lo, hi, n) {
            prop_assert!(false, "{}", e);
        }
    }
}

// ---------------------------------------------------------------------------
// Books on disk
// ---------------------------------------------------------------------------

static DIRS: AtomicU64 = AtomicU64::new(0);

/// A fresh, empty temporary directory; removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let n = DIRS.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("ledger-gui-spec-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 12).unwrap()
}

fn usd(d: Decimal) -> Money {
    Money::new(d, Currency::USD)
}

fn write_toml(dir: &Path, threshold: &str, cash: &[&str]) -> PathBuf {
    let cash: Vec<String> = cash.iter().map(|c| format!("\"{c}\"")).collect();
    let p = dir.join("wanderbooks.toml");
    std::fs::write(
        &p,
        format!(
            "[company]\nname = \"Spec Test LLC\"\n\n[books]\nreceipt_threshold = \"{threshold}\"\ncash_accounts = [{}]\n",
            cash.join(", ")
        ),
    )
    .unwrap();
    p
}

fn load(dir: &Path, toml: Option<PathBuf>) -> Books {
    load_once(&Source {
        data_dir: dir.join("books-data"),
        books_toml: toml,
        today: Some(today()),
    })
    .unwrap()
}

/// The chart used by the random books: (code, name, type).
const CHART: &[(&str, &str, AccountType)] = &[
    ("1000", "Checking", AccountType::Asset),
    ("1010", "Savings", AccountType::Asset),
    ("1500", "Equipment", AccountType::Asset),
    ("2100", "Due to Shareholders", AccountType::Liability),
    ("2200", "Loan", AccountType::Liability),
    ("3000", "Capital", AccountType::Equity),
    ("4000", "App Revenue", AccountType::Revenue),
    ("6000", "Software", AccountType::Expense),
    ("6100", "Rent", AccountType::Expense),
];
const CASH_CODES: &[&str] = &["1000", "1010"];

#[derive(Clone, Debug)]
struct EntryPlan {
    day: u32,
    debits: Vec<(usize, u64)>,
    credit_accounts: Vec<usize>,
    split_pct: u64,
    scale: u32,
    post: bool,
    receipt: bool,
}

#[derive(Clone, Debug)]
struct BooksPlan {
    threshold: &'static str,
    entries: Vec<EntryPlan>,
    reverse: Option<usize>,
}

fn arb_entry() -> impl Strategy<Value = EntryPlan> {
    (
        0u32..285,
        prop::collection::vec((0usize..CHART.len(), 1u64..1_000_000), 1..=3),
        prop::collection::vec(0usize..CHART.len(), 1..=2),
        1u64..100,
        prop_oneof![Just(2u32), Just(0u32)],
        prop::bool::weighted(0.75),
        prop::bool::weighted(0.4),
    )
        .prop_map(
            |(day, debits, credit_accounts, split_pct, scale, post, receipt)| EntryPlan {
                day,
                debits,
                credit_accounts,
                split_pct,
                scale,
                post,
                receipt,
            },
        )
}

fn arb_books() -> impl Strategy<Value = BooksPlan> {
    (
        prop_oneof![Just("0.00"), Just("25.00"), Just("1000.00")],
        prop::collection::vec(arb_entry(), 0..10),
        prop::option::of(0usize..10),
    )
        .prop_map(|(threshold, entries, reverse)| BooksPlan {
            threshold,
            entries,
            reverse,
        })
}

struct Built {
    _dir: TempDir,
    books: Books,
    data: PathBuf,
    entry_ids: Vec<EntryId>,
    receipts: HashMap<EntryId, usize>,
    reversal: Option<(EntryId, EntryId)>,
}

fn build_books(plan: &BooksPlan) -> Built {
    let dir = TempDir::new("books");
    let data = dir.0.join("books-data");
    let toml = write_toml(&dir.0, plan.threshold, CASH_CODES);
    let mut store = Store::create(&data, Currency::USD).unwrap();
    let ids: Vec<AccountId> = CHART
        .iter()
        .map(|(c, n, t)| store.add_account(c, n, *t, None, false).unwrap())
        .collect();
    let start = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let mut entry_ids = vec![];
    let mut posted = vec![];
    let mut receipts = HashMap::new();
    for (i, e) in plan.entries.iter().enumerate() {
        let date = start + chrono::Days::new(u64::from(e.day));
        let id = store
            .create_entry(date, &format!("Entry {i} for spec"))
            .unwrap();
        let mut total = Decimal::ZERO;
        for (acct, cents) in &e.debits {
            let amt = Decimal::new(*cents as i64, e.scale);
            total += amt;
            store
                .add_line(id, ids[*acct], Side::Debit, usd(amt))
                .unwrap();
        }
        // Split the total across the credit accounts, every part nonzero.
        let unit = Decimal::new(1, e.scale);
        let mut parts = vec![];
        if e.credit_accounts.len() == 2 && total > unit {
            let mut first =
                (total * Decimal::from(e.split_pct) / Decimal::from(100)).round_dp(e.scale);
            if first <= Decimal::ZERO {
                first = unit;
            }
            if first >= total {
                first = total - unit;
            }
            parts.push((e.credit_accounts[0], first));
            parts.push((e.credit_accounts[1], total - first));
        } else {
            parts.push((e.credit_accounts[0], total));
        }
        for (acct, amt) in parts {
            store
                .add_line(id, ids[acct], Side::Credit, usd(amt))
                .unwrap();
        }
        if e.receipt {
            store
                .attach_receipt(
                    Some(id),
                    "receipt.pdf",
                    "application/pdf",
                    format!("%PDF receipt for entry {id}").as_bytes(),
                    Some("spec receipt"),
                )
                .unwrap();
            receipts.insert(id, 1);
        }
        if e.post {
            store.post_entry(id).unwrap();
            posted.push(id);
        }
        entry_ids.push(id);
    }
    let mut reversal = None;
    if let Some(k) = plan.reverse
        && !posted.is_empty()
    {
        let orig = posted[k % posted.len()];
        let rev = store
            .reverse_entry(orig, today(), &format!("Reverse {orig}"))
            .unwrap();
        entry_ids.push(rev);
        reversal = Some((orig, rev));
    }
    drop(store);
    let books = load(&dir.0, Some(toml));
    Built {
        _dir: dir,
        books,
        data,
        entry_ids,
        receipts,
        reversal,
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// Books::build arranges ledger-core's numbers and changes none of them.
    #[test]
    fn books_build_agrees_with_ledger_core(plan in arb_books()) {
        let built = build_books(&plan);
        let b = &built.books;
        let snap = Snapshot::load(&built.data).unwrap();
        let ledger = snap.ledger();
        let chart = ledger.chart();
        let cash: Vec<AccountId> = CASH_CODES
            .iter()
            .map(|c| chart.find_by_code(c).unwrap().id)
            .collect();
        let cash_set: HashSet<AccountId> = cash.iter().copied().collect();

        // Cash accounts come from `[books] cash_accounts`.
        let got_cash: HashSet<AccountId> = b.cash_accounts.iter().copied().collect();
        prop_assert_eq!(&got_cash, &cash_set);

        // Newest first: date, then id.
        for w in b.entries.windows(2) {
            prop_assert!(
                (w[0].date, w[0].id) > (w[1].date, w[1].id),
                "out of order: #{} {} before #{} {}", w[0].id, w[0].date, w[1].id, w[1].date
            );
        }

        let policy = ledger_config::effective_receipt_policy(
            &built.data.join("config.json"),
            &b.settings,
        )
        .unwrap();

        for &id in &built.entry_ids {
            let info = b.entry(id);
            prop_assert!(info.is_some(), "entry #{} not indexed", id);
            let info = info.unwrap();
            prop_assert_eq!(info.id, id);
            let e = ledger.get_entry(id).unwrap();
            prop_assert_eq!(info.date, e.date());
            prop_assert_eq!(info.status, e.status());
            prop_assert_eq!(info.reversal_of, e.reversal_of());
            let debits: Decimal = e
                .lines()
                .iter()
                .filter(|l| l.side == Side::Debit)
                .map(|l| l.amount.amount)
                .sum();
            prop_assert_eq!(info.amount, Some(debits), "amount of #{}", id);
            prop_assert_eq!(
                info.receipts.len(),
                built.receipts.get(&id).copied().unwrap_or(0),
                "receipts of #{}", id
            );
            if e.status() == EntryStatus::Posted {
                let net: Decimal = e
                    .lines()
                    .iter()
                    .filter(|l| cash_set.contains(&l.account))
                    .map(|l| match l.side {
                        Side::Debit => l.amount.amount,
                        Side::Credit => -l.amount.amount,
                    })
                    .sum();
                prop_assert_eq!(info.cash_effect, Some(net), "cash effect of #{}", id);

                let has_receipt = built.receipts.contains_key(&id);
                if e.reversal_of().is_some() {
                    prop_assert!(!info.missing_receipt, "reversal #{} flagged", id);
                } else {
                    let expect = !has_receipt && policy.receipt_required(ledger, e);
                    prop_assert_eq!(info.missing_receipt, expect, "missing_receipt of #{}", id);
                }
            }
        }
        if let Some((orig, rev)) = built.reversal {
            prop_assert_eq!(b.entry(rev).unwrap().reversal_of, Some(orig));
        }

        // Kpis.cash: the cash accounts' balance at the end of today.
        let want = ledger_core::analysis::balance_series(ledger, &cash, &[b.today])[0];
        prop_assert_eq!(b.kpis.cash, want);
        let by_account: Decimal = cash.iter().map(|&a| b.balance_on(a, b.today)).sum();
        prop_assert_eq!(by_account, want);
        prop_assert_eq!(
            cash.iter().map(|&a| ledger.account_balance(a).unwrap()).sum::<Decimal>(),
            want
        );
    }
}

// ---------------------------------------------------------------------------
// 7. attention::find
// ---------------------------------------------------------------------------

struct Small {
    dir: TempDir,
    store: Store,
    cash: AccountId,
    rent: AccountId,
}

fn small_books(tag: &str) -> Small {
    let dir = TempDir::new(tag);
    let mut store = Store::create(&dir.0.join("books-data"), Currency::USD).unwrap();
    let cash = store
        .add_account("1000", "Checking", AccountType::Asset, None, false)
        .unwrap();
    let rent = store
        .add_account("6100", "Rent", AccountType::Expense, None, false)
        .unwrap();
    Small {
        dir,
        store,
        cash,
        rent,
    }
}

fn add_rent(s: &mut Small, i: u32, post: bool) -> EntryId {
    let d = NaiveDate::from_ymd_opt(2026, 9, 1 + i).unwrap();
    let id = s.store.create_entry(d, &format!("Rent {i}")).unwrap();
    let m = usd(Decimal::new(10_000 + i64::from(i), 2));
    s.store.add_line(id, s.rent, Side::Debit, m).unwrap();
    s.store.add_line(id, s.cash, Side::Credit, m).unwrap();
    if post {
        s.store.post_entry(id).unwrap();
    }
    id
}

fn finish(s: Small) -> (TempDir, Books) {
    let Small { dir, store, .. } = s;
    drop(store);
    let toml = write_toml(&dir.0, "0.00", &["1000"]);
    let b = load(&dir.0, Some(toml));
    (dir, b)
}

fn check_prompts(b: &Books) {
    for item in attention::find(b) {
        assert!(
            item.prompt.contains("wanderbooks"),
            "prompt doesn't name wanderbooks: {:?}",
            item.prompt
        );
    }
}

fn titles(b: &Books) -> Vec<String> {
    attention::find(b).into_iter().map(|a| a.title).collect()
}

#[test]
fn attention_empty_books_have_nothing() {
    let s = small_books("att-empty");
    let (_d, b) = finish(s);
    let items = attention::find(&b);
    assert!(
        items.is_empty(),
        "{:?}",
        items.iter().map(|a| &a.title).collect::<Vec<_>>()
    );

    let dir = TempDir::new("att-bare");
    drop(Store::create(&dir.0.join("books-data"), Currency::USD).unwrap());
    let b = load(&dir.0, None);
    assert!(attention::find(&b).is_empty());
}

#[test]
fn attention_one_draft_is_one_todo() {
    let mut s = small_books("att-draft");
    let id = add_rent(&mut s, 0, false);
    let (_d, b) = finish(s);
    let items = attention::find(&b);
    let todos: Vec<_> = items
        .iter()
        .filter(|a| a.severity == Severity::Todo)
        .collect();
    assert_eq!(todos.len(), 1, "{:?}", titles(&b));
    let target = todos[0].target.clone();
    assert!(
        target.is_none() || target == Some(Route::Entry(id)),
        "{target:?}"
    );
    // A draft is not posted, so nothing is missing a receipt yet.
    assert!(!items.iter().any(|a| a.severity == Severity::Problem));
    check_prompts(&b);
    assert_eq!(titles(&b), titles(&b));
}

#[test]
fn attention_missing_receipt_is_one_problem() {
    let mut s = small_books("att-missing");
    let id = add_rent(&mut s, 0, true);
    let (_d, b) = finish(s);
    let problems: Vec<_> = attention::find(&b)
        .into_iter()
        .filter(|a| a.severity == Severity::Problem)
        .collect();
    assert_eq!(problems.len(), 1, "{:?}", titles(&b));
    assert!(b.entry(id).unwrap().missing_receipt);
    check_prompts(&b);
    assert_eq!(titles(&b), titles(&b));
}

#[test]
fn attention_receipt_attached_is_no_problem() {
    let mut s = small_books("att-ok");
    let id = add_rent(&mut s, 0, false);
    s.store
        .attach_receipt(Some(id), "r.pdf", "application/pdf", b"%PDF rent", None)
        .unwrap();
    s.store.post_entry(id).unwrap();
    let (_d, b) = finish(s);
    assert!(
        !attention::find(&b)
            .iter()
            .any(|a| a.severity == Severity::Problem),
        "{:?}",
        titles(&b)
    );
    assert!(!b.entry(id).unwrap().missing_receipt);
}

#[test]
fn attention_missing_receipts_combine_at_four() {
    for (n, want) in [(2u32, 2usize), (3, 3), (4, 1), (6, 1)] {
        let mut s = small_books("att-many");
        for i in 0..n {
            add_rent(&mut s, i, true);
        }
        let (_d, b) = finish(s);
        let problems = attention::find(&b)
            .into_iter()
            .filter(|a| a.severity == Severity::Problem)
            .count();
        assert_eq!(problems, want, "{n} missing: {:?}", titles(&b));
        check_prompts(&b);
        assert_eq!(titles(&b), titles(&b));
    }
}

#[test]
fn attention_is_pure_on_mixed_books() {
    let mut s = small_books("att-mixed");
    for i in 0..5 {
        add_rent(&mut s, i, i % 2 == 0);
    }
    let (_d, b) = finish(s);
    let first = attention::find(&b);
    let second = attention::find(&b);
    assert_eq!(first.len(), second.len());
    for (x, y) in first.iter().zip(&second) {
        assert_eq!(x.title, y.title);
        assert_eq!(x.detail, y.detail);
        assert_eq!(x.prompt, y.prompt);
        assert_eq!(x.severity, y.severity);
    }
    // Two drafts, three posted without receipts (and the checking
    // account, with activity before this month, due its reconciliation).
    let todos = first
        .iter()
        .filter(|a| a.severity == Severity::Todo && !a.title.starts_with("Reconcile "))
        .count();
    let problems = first
        .iter()
        .filter(|a| a.severity == Severity::Problem)
        .count();
    assert_eq!((todos, problems), (2, 3), "{:?}", titles(&b));
    check_prompts(&b);
}
