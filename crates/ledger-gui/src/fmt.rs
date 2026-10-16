//! Formatting money, dates, and counts. Amounts are exact decimals in and
//! exact strings out: nothing here rounds except `compact`, which says so.
//!
//! ```text
//! GOAL: show every amount exactly as the books hold it.
//! SOURCE: DECISION (the MCP contract: "money is exact or it is wrong");
//!         CONVENTION (US accounting display: thousands commas,
//!         negatives in parentheses or with a minus sign).
//! INVARIANTS:
//!   1. money, accounting, and dollars NEVER round: every decimal place
//!      the amount has (at least two) is shown, thousands grouped by
//!      commas in threes, so the text reads back to the same value.
//!   2. Zero (and negative zero) shows without a sign; a negative shows
//!      U+2212 (money, dollars) or parentheses (accounting), never both
//!      and never "-0".
//!   3. compact rounds (chart axes, tight tiles): $950, $1.2k, $25k,
//!      $3.4M; it never shows a sign on what rounds to zero.
//! ```

use chrono::NaiveDate;
use rust_decimal::Decimal;

/// Thousands separators, every decimal (at least two), a true minus sign:
/// `−1,234.56`.
pub fn money(d: Decimal) -> String {
    let s = group(d.abs());
    if is_negative(d) {
        format!("\u{2212}{s}")
    } else {
        s
    }
}

/// What the viewer shows for a figure the books can't compute (rules.md
/// B-28: an earlier version's draft lines, shares, or bases past exact
/// arithmetic): said in words, never a number.
pub const CANT_COMPUTE: &str = "too large to compute";

/// [`money`], or [`CANT_COMPUTE`] for a figure the books can't compute.
pub fn money_or(d: Option<Decimal>) -> String {
    d.map_or_else(|| CANT_COMPUTE.to_string(), money)
}

/// [`dollars`], or [`CANT_COMPUTE`] for a figure the books can't compute.
pub fn dollars_or(d: Option<Decimal>) -> String {
    d.map_or_else(|| CANT_COMPUTE.to_string(), dollars)
}

/// Accounting style: negatives in parentheses, `(1,234.56)`.
pub fn accounting(d: Decimal) -> String {
    let s = group(d.abs());
    if is_negative(d) { format!("({s})") } else { s }
}

/// With the currency sign: `$1,234.56`, `−$12.00`.
pub fn dollars(d: Decimal) -> String {
    let s = format!("${}", group(d.abs()));
    if is_negative(d) {
        format!("\u{2212}{s}")
    } else {
        s
    }
}

/// Rounded for chart axes and tight tiles: `$1.2k`, `$3.4M`, `$950`.
pub fn compact(d: Decimal) -> String {
    let f: f64 = d.try_into().unwrap_or(0.0);
    let a = f.abs();
    let s = if a >= 1_000_000.0 {
        format!("${:.1}M", a / 1_000_000.0)
    } else if a >= 10_000.0 {
        format!("${:.0}k", a / 1_000.0)
    } else if a >= 1_000.0 {
        format!("${:.1}k", a / 1_000.0)
    } else {
        format!("${a:.0}")
    };
    // No sign on what rounds to zero ("$0", not "−$0").
    if f < 0.0 && s != "$0" {
        format!("\u{2212}{s}")
    } else {
        s
    }
}

fn is_negative(d: Decimal) -> bool {
    d.is_sign_negative() && !d.is_zero()
}

/// A non-negative amount, every decimal it has (at least two), grouped.
fn group(d: Decimal) -> String {
    let places = d.normalize().scale().max(2) as usize;
    let s = format!("{:.*}", places, d);
    let (int, frac) = s.split_once('.').unwrap_or((&s, "00"));
    let mut out = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    format!("{out}.{frac}")
}

/// The plain decimal behind a displayed amount, for the clipboard:
/// `$1,234.56` → `1234.56`, `(207.68)` and `−12.00` → `-207.68`, `-12.00`.
/// Exact (the display never rounds). None when the text isn't an amount.
pub fn plain(display: &str) -> Option<String> {
    let t = display.trim();
    let (neg, t) = match t.strip_prefix('(').and_then(|x| x.strip_suffix(')')) {
        Some(inner) => (true, inner),
        None => (false, t),
    };
    let (neg, t) = match t.strip_prefix('\u{2212}').or_else(|| t.strip_prefix('-')) {
        Some(rest) => (!neg, rest),
        None => (neg, t.strip_prefix('+').unwrap_or(t)),
    };
    let digits: String = t.chars().filter(|c| !matches!(c, '$' | ',')).collect();
    let d: Decimal = digits.parse().ok()?;
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }
    Some(if neg && !d.is_zero() {
        format!("-{digits}")
    } else {
        digits
    })
}

/// A percent change from `prev` to `now`, if `prev` isn't zero.
pub fn pct_change(prev: Decimal, now: Decimal) -> Option<f64> {
    if prev.is_zero() {
        return None;
    }
    let p: f64 = prev.try_into().ok()?;
    let n: f64 = now.try_into().ok()?;
    Some((n - p) / p.abs() * 100.0)
}

/// `Oct 12, 2026`.
pub fn date(d: NaiveDate) -> String {
    d.format("%b %-d, %Y").to_string()
}

/// `Oct 12`.
pub fn date_short(d: NaiveDate) -> String {
    d.format("%b %-d").to_string()
}

/// `Oct 2026`.
pub fn month(d: NaiveDate) -> String {
    d.format("%b %Y").to_string()
}

/// `1 receipt`, `3 receipts`.
pub fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// An RFC 3339 timestamp from the journal, shown as local-ish UTC text:
/// `Oct 11, 2026 18:04 UTC`.
pub fn timestamp(ts: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(ts)
        .map(|t| t.format("%b %-d, %Y %H:%M UTC").to_string())
        .unwrap_or_else(|_| ts.to_string())
}

/// Bytes as `12.3 KB`.
pub fn bytes(n: u64) -> String {
    let f = n as f64;
    if f >= 1_048_576.0 {
        format!("{:.1} MB", f / 1_048_576.0)
    } else if f >= 1024.0 {
        format!("{:.1} KB", f / 1024.0)
    } else {
        format!("{n} B")
    }
}

/// `snake_case_key` as `Snake case key`, for settings shown as labels,
/// keeping acronyms upper case (`ein` → `EIN`).
pub fn label(key: &str) -> String {
    const ACRONYMS: [&str; 9] = [
        "ein", "duns", "irs", "cpa", "llc", "tin", "ssn", "id", "url",
    ];
    let words: Vec<String> = key
        .split(['_', ' '])
        .filter(|w| !w.is_empty())
        .enumerate()
        .map(|(i, w)| {
            if ACRONYMS.contains(&w.to_lowercase().as_str()) {
                w.to_uppercase()
            } else if i == 0 {
                let mut c = w.chars();
                c.next()
                    .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                    .unwrap_or_default()
            } else {
                w.to_string()
            }
        })
        .collect();
    words.join(" ")
}

/// A `CamelCase` enum name as words: `PrepaidExpense` → `Prepaid expense`.
pub fn words(debug: &str) -> String {
    let mut out = String::new();
    for (i, c) in debug.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push(' ');
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// The tax return a form enum stands for.
pub fn tax_form(f: ledger_core::TaxForm) -> &'static str {
    match f {
        ledger_core::TaxForm::ScheduleC => "Schedule C (Form 1040)",
        ledger_core::TaxForm::Form1065 => "Form 1065 (partnership)",
        ledger_core::TaxForm::Form1120S => "Form 1120-S (S corporation)",
        ledger_core::TaxForm::Form1120 => "Form 1120 (C corporation)",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    /// money_or / dollars_or: a figure as money_ or dollars does, and one
    /// the books can't compute in words, never a number (rules.md B-28).
    #[test]
    fn a_figure_or_why_not() {
        let v = Decimal::new(-123_456, 2);
        assert_eq!(money_or(Some(v)), money(v));
        assert_eq!(dollars_or(Some(v)), dollars(v));
        assert_eq!(money_or(None), CANT_COMPUTE);
        assert_eq!(dollars_or(None), CANT_COMPUTE);
        assert_eq!(CANT_COMPUTE, "too large to compute");
    }

    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    #[test]
    fn money_formats() {
        assert_eq!(money(dec("1234567.5")), "1,234,567.50");
        assert_eq!(money(dec("-12")), "\u{2212}12.00");
        assert_eq!(money(dec("0")), "0.00");
        assert_eq!(money(dec("-0.00")), "0.00");
        assert_eq!(accounting(dec("-207.68")), "(207.68)");
        assert_eq!(dollars(dec("999.999")), "$999.999");
        assert_eq!(money(dec("0.125")), "0.125");
        assert_eq!(compact(dec("-0.4")), "$0");
        assert_eq!(compact(dec("1234")), "$1.2k");
        assert_eq!(compact(dec("-25000")), "\u{2212}$25k");
        assert_eq!(compact(dec("950.4")), "$950");
        assert_eq!(label("state_entity_number"), "State entity number");
        assert_eq!(label("ein"), "EIN");
        assert_eq!(label("irs_name_control"), "IRS name control");
        assert_eq!(words("PrepaidExpense"), "Prepaid expense");
        assert_eq!(plain("$1,234.56").as_deref(), Some("1234.56"));
        assert_eq!(plain("(207.68)").as_deref(), Some("-207.68"));
        assert_eq!(plain("\u{2212}$12.00").as_deref(), Some("-12.00"));
        assert_eq!(plain("+1,000.005").as_deref(), Some("1000.005"));
        assert_eq!(plain("0.00").as_deref(), Some("0.00"));
        assert_eq!(plain("Debit"), None);
        // Only digits and a point: the decimal parser alone takes these.
        assert_eq!(plain("1_000.00"), None);
        assert_eq!(plain("1e3"), None);
        assert_eq!(plain(""), None);
    }
}
