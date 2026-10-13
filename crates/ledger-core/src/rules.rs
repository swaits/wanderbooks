//! Rules set by law that change over time, and how far wanderbooks knows
//! them.
//!
//! The engine has no clock, so it never decides on its own that a rule is
//! out of date. Instead each dated rule says which years it is known for
//! and when it is due for review; callers that know today's date (the MCP
//! server, the viewer) ask [`stale_rules`] and surface what is due.
//!
//! ```text
//! GOAL: never apply a legal amount for a year it isn't known for, and say
//!       so before anyone needs it.
//! SOURCE: DECISION (Stephen, 2026-10-13): when what wanderbooks knows is
//!         stale, research it or ask the owner or CPA, and update the rule
//!         for everyone (a pull request, or at least an issue, on
//!         github.com/swaits/wanderbooks).
//! INVARIANTS:
//!   1. NEVER returns a rule's value for a year past `known_through`.
//!   2. MUST report a rule as stale from its review date on.
//! ```
//!
//! To update a rule: vendor the new official text beside the old in its
//! `source` file (docs/spec/README.md, "Vendoring rules"), extend the
//! rule's table and `known_through`, move `review_after` to when the next
//! change is expected, and say so in the commit.

use chrono::NaiveDate;

/// A rule set by law whose value changes over time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DatedRule {
    /// Plain name, e.g. "1099 reporting threshold".
    pub name: &'static str,
    /// Its entry in docs/spec/rules.md, e.g. "B-19".
    pub catalog: &'static str,
    /// The vendored official text it comes from.
    pub source: &'static str,
    /// The last year wanderbooks knows the rule for.
    pub known_through: i32,
    /// When to look again, as (year, month, day): the day the next value
    /// is expected to be published or to take effect.
    pub review: (i32, u32, u32),
    /// Why that date, and where the next value will come from.
    pub review_why: &'static str,
}

impl DatedRule {
    /// The review date.
    pub fn review_after(&self) -> NaiveDate {
        let (y, m, d) = self.review;
        NaiveDate::from_ymd_opt(y, m, d).expect("rule review dates are valid")
    }

    /// Whether the rule is due for review on `today`.
    pub fn is_stale(&self, today: NaiveDate) -> bool {
        today >= self.review_after()
    }
}

/// The 1099-NEC / 1099-MISC reporting threshold (rules.md B-19): $600 for
/// payments through 2025, $2,000 in 2026, then indexed for inflation.
pub const THRESHOLD_1099: DatedRule = DatedRule {
    name: "1099 reporting threshold",
    catalog: "B-19",
    source: "docs/spec/sources/irs-1099-thresholds.md",
    known_through: 2026,
    review: (2026, 11, 1),
    review_why: "26 U.S.C. §6041(h) indexes the threshold for calendar years after 2026, \
                 rounded to the nearest $100; the IRS publishes each year's inflation \
                 adjustments late in the year before (CONVENTION: the usual timing, not a \
                 cited date), so look for the 2027 figure from November 2026.",
};

/// Every dated rule the engine applies.
pub const DATED_RULES: &[DatedRule] = &[THRESHOLD_1099];

/// The dated rules due for review on `today`.
pub fn stale_rules(today: NaiveDate) -> Vec<&'static DatedRule> {
    DATED_RULES.iter().filter(|r| r.is_stale(today)).collect()
}
