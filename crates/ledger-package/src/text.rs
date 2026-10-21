//! How the books' amounts, shares, dates, and CSV cells are written and
//! read: one spelling for the server's tools, the package, and the viewer.
//!
//! ```text
//! GOAL: an amount, a share, or a date reads the same in every file and
//!       tool the books make, and a date in is checked one way.
//! SOURCE: ledger-mcp server.rs INVARIANT 3 (amounts out are decimal
//!         strings with at least two places, never rounded); RFC 4180
//!         (a CSV field holding a comma, quote, or line break is quoted,
//!         its quotes doubled); P0 review round 3, S1 (dates).
//! INVARIANTS:
//!   1. money pads to two places and never rounds; it never prints "-0".
//!   2. share_text depends on a share's value, never its spelling.
//!   3. csv_escape quotes exactly the fields that need it.
//!   4. parse_date takes exactly YYYY-MM-DD, a day the calendar keeps.
//! ```

use chrono::NaiveDate;
use rust_decimal::Decimal;

/// A money amount as a decimal string with at least two places: "0.00",
/// "318.42", "1000.00". Padding only, never rounding: an exact amount with
/// more places (an FX-converted line) keeps them. Never "-0".
pub fn money(n: Decimal) -> String {
    let mut n = if n.is_zero() { Decimal::ZERO } else { n };
    if n.scale() < 2 {
        n.rescale(2);
    }
    n.to_string()
}

/// A member's share as the K-1 worksheet prints it: at least two places,
/// however it was written ("50%", "0.5" and "0.500" all print "0.50"; a
/// share of a third keeps its places). The printed text depends on the
/// share, never on its spelling.
pub fn share_text(n: Decimal) -> String {
    let mut n = n.normalize();
    if n.scale() < 2 {
        n.rescale(2);
    }
    n.to_string()
}

/// A CSV field: quoted, with its quotes doubled, when it holds a comma, a
/// quote, or a line feed; else as it is.
pub fn csv_escape(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// A date: exactly YYYY-MM-DD, a real day in the calendar the books keep
/// (0001-01-01 through 9999-12-31, `ledger_core::calendar`). No sign, no
/// space, no five-digit year: every date past here is one date arithmetic
/// reaches (P0 review round 3, S1).
pub fn parse_date(s: &str) -> Result<NaiveDate, String> {
    let shaped = s.len() == 10
        && s.bytes().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        });
    shaped
        .then(|| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        .flatten()
        .filter(|d| ledger_core::calendar::in_calendar(*d))
        .ok_or_else(|| {
            format!("bad date {s:?}: expected YYYY-MM-DD, a real day from 0001-01-01 to 9999-12-31")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn money_always_has_cents_and_never_rounds() {
        let m = |s: &str| money(Decimal::from_str(s).unwrap());
        assert_eq!(money(Decimal::ZERO), "0.00");
        assert_eq!(m("-0"), "0.00");
        assert_eq!(m("1000"), "1000.00");
        assert_eq!(m("5.8"), "5.80");
        assert_eq!(m("-318.42"), "-318.42");
        assert_eq!(m("12.3456"), "12.3456");
    }

    /// A share prints by its value, never its spelling: at least two
    /// places, a third keeping its own.
    #[test]
    fn a_share_prints_the_same_however_it_was_written() {
        let t = |s: &str| share_text(Decimal::from_str(s).unwrap());
        for half in ["0.5", "0.50", "0.500000"] {
            assert_eq!(t(half), "0.50", "{half}");
        }
        assert_eq!(t("1"), "1.00");
        assert_eq!(t("1.0"), "1.00");
        assert_eq!(t("0.25"), "0.25");
        assert_eq!(t("0.3333"), "0.3333");
        assert_eq!(t("0.125"), "0.125");
    }

    #[test]
    fn csv_fields_are_quoted_only_when_they_must_be() {
        assert_eq!(csv_escape("plain"), "plain");
        assert_eq!(csv_escape("a,b"), "\"a,b\"");
        assert_eq!(csv_escape("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_escape("two\nlines"), "\"two\nlines\"");
        assert_eq!(csv_escape(""), "");
    }

    /// A date is exactly YYYY-MM-DD, a real day from 0001-01-01 to
    /// 9999-12-31 (ledger_core::calendar): no sign, no space, no fifth
    /// year digit, no year 0 (P0 review round 3, S1).
    #[test]
    fn dates_are_four_digit_years_and_real_days() {
        assert_eq!(parse_date("2026-02-28"), Ok(d(2026, 2, 28)));
        assert_eq!(parse_date("2028-02-29"), Ok(d(2028, 2, 29)));
        assert_eq!(parse_date("0001-01-01"), Ok(d(1, 1, 1)));
        assert_eq!(parse_date("9999-12-31"), Ok(d(9999, 12, 31)));
        for bad in [
            "0000-01-01",
            "0000-00-00",
            "2026-02-29",
            "2026-02-30",
            "2026-13-01",
            "2026-1-05",
            "2026-01-5",
            " 2026-01-05",
            "2026-01-05 ",
            "+2026-01-05",
            "-0001-01-01",
            "+262142-12-31",
            "-262143-01-01",
            "99999-12-31",
            "2026/01/05",
            "2026-01-05T00:00",
            "２０２６-01-05",
            "",
        ] {
            let err = parse_date(bad).expect_err(bad);
            assert!(err.contains("0001-01-01 to 9999-12-31"), "{bad:?}: {err}");
        }
    }
}
