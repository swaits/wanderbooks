//! The interface an accounting engine implements to be checked against
//! bookspec (`plan/served.md` P13, D-34).
//!
//! bookspec is a specification first: its examples are the concrete cases
//! (`bookspec/examples/*.toml`), its rule identifiers the normative rules
//! (`BS-<area>-<n>`), and this trait the only thing an engine has to
//! provide for the runner to check it. The engine's answers are compared
//! with the examples' expected values by the runner; an engine is never
//! asked whether it agrees with itself.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

use crate::rules::{Area, Rule, rule_for_id, rule_for_refusal};
use crate::schema::{ChangeSpec, Event, Query, Setup};

/// A bookspec rule identifier: `BS-<area>-<n>` (e.g. `BS-entry-1`).
///
/// The areas are the specification's subjects (`Area`), and `n` counts
/// from 1 within an area. The catalogue in [`mod@crate::rules`] is the list of
/// every identifier the specification states; an identifier that is not in
/// it is a typo, not a rule.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuleId(String);

/// Why a string is not a bookspec rule identifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleIdError {
    what: String,
}

impl fmt::Display for RuleIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.what)
    }
}

impl std::error::Error for RuleIdError {}

impl RuleId {
    /// The identifier of area `area`, rule number `n` (counting from 1).
    pub fn new(area: Area, n: u32) -> Self {
        assert!(n >= 1, "bookspec rule numbers count from 1");
        Self(format!("BS-{}-{n}", area.key()))
    }

    /// Parse `BS-<area>-<n>`, refusing an unknown area, a missing or
    /// non-numeric number, and a number that is zero or written with a
    /// leading zero (one spelling per rule).
    pub fn parse(s: &str) -> Result<Self, RuleIdError> {
        let rest = s.strip_prefix("BS-").ok_or_else(|| RuleIdError {
            what: format!("{s:?} is no bookspec rule id (BS-<area>-<n>)"),
        })?;
        let (area, n) = rest.rsplit_once('-').ok_or_else(|| RuleIdError {
            what: format!("{s:?} is no bookspec rule id (BS-<area>-<n>)"),
        })?;
        let area = Area::parse(area).ok_or_else(|| RuleIdError {
            what: format!("{s:?}: {:?} is no bookspec area", area),
        })?;
        if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
            return Err(RuleIdError {
                what: format!("{s:?}: {n:?} is no rule number"),
            });
        }
        if n.starts_with('0') {
            return Err(RuleIdError {
                what: format!("{s:?}: rule numbers count from 1, without a leading zero"),
            });
        }
        let n: u32 = n.parse().map_err(|_| RuleIdError {
            what: format!("{s:?}: {n:?} is no rule number"),
        })?;
        if n == 0 {
            return Err(RuleIdError {
                what: format!("{s:?}: rule numbers count from 1"),
            });
        }
        Ok(Self::new(area, n))
    }

    /// The identifier as the specification writes it.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The area it belongs to.
    pub fn area(&self) -> Area {
        // Constructed only by `new`/`parse`, both of which check the area.
        Area::parse(self.0.split('-').nth(1).expect("a checked rule id"))
            .expect("a checked rule id's area")
    }

    /// The rule's entry in the catalogue, if the specification states it.
    pub fn rule(&self) -> Option<&'static Rule> {
        rule_for_id(self)
    }
}

impl fmt::Display for RuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for RuleId {
    type Err = RuleIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// An engine's refusal: the rule that refuses, and why in the example's
/// words (bookspec/README.md's refusal vocabulary, never an engine's own
/// message).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// The bookspec rule the refusal belongs to.
    pub rule: RuleId,
    /// What was refused, in the specification's words.
    pub reason: String,
}

impl Refusal {
    /// A refusal of `reason` under `rule`.
    pub fn new(rule: RuleId, reason: impl Into<String>) -> Self {
        Self {
            rule,
            reason: reason.into(),
        }
    }

    /// A refusal of `reason`, under the rule the specification assigns
    /// that refusal (see [`rule_for_refusal`]); an unknown reason is a
    /// specification defect, and the first rule of the ledger area is
    /// named so the report still has an identifier to show.
    pub fn of_reason(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        let rule = rule_for_refusal(&reason)
            .map(|r| r.id.clone())
            .unwrap_or_else(|| RuleId::new(Area::Ledger, 1));
        Self { rule, reason }
    }
}

/// What an engine declares about itself: the rules it takes the
/// conformance test on, and the option it picks for each of the
/// specification's choices (`CHOICE`: the specification names the options,
/// an engine declares its pick; `plan/served.md` P13 step 3).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Claims {
    /// The rule identifiers the engine claims.
    pub rules: BTreeSet<RuleId>,
    /// The choice rules whose option the engine picks, and the option.
    pub choices: BTreeMap<RuleId, String>,
}

impl Claims {
    /// The claims of an engine that claims `rules`, with no choices.
    pub fn new(rules: impl IntoIterator<Item = RuleId>) -> Self {
        Self {
            rules: rules.into_iter().collect(),
            choices: BTreeMap::new(),
        }
    }

    /// True when `rule` is claimed.
    pub fn claims(&self, rule: &RuleId) -> bool {
        self.rules.contains(rule)
    }

    /// Declare the option this engine picks for `rule`.
    pub fn with_choice(mut self, rule: RuleId, option: impl Into<String>) -> Self {
        self.choices.insert(rule, option.into());
        self
    }

    /// The option this engine picks for `rule`, if it declares one.
    pub fn choice(&self, rule: &RuleId) -> Option<&str> {
        self.choices.get(rule).map(String::as_str)
    }
}

/// An accounting engine, as bookspec checks it.
///
/// The four methods are the plan's (`plan/served.md` 8.2 P13 step 2); the
/// fifth, [`Engine::change`], is the one departure: an example's
/// `[[changes]]` (a rename, the tax-category policy, an asset correction)
/// posts no entry, so it is neither a setup nor an event, and the plan's
/// four methods have no path for it. It is required, never defaulted, so
/// an engine cannot silently skip the examples that use one.
pub trait Engine {
    /// Set the books up: the chart, the fixed assets, the schedules, the
    /// parties, the members, the payees and their dated decisions, the
    /// payment methods and the projects.
    fn load(&mut self, setup: &Setup) -> Result<(), Refusal>;

    /// Apply one event: an entry, a reversal, a close, a bill, an invoice,
    /// a depreciation run, a disposal, a schedules run or a schedule's
    /// registration. An event the engine refuses leaves the books as they
    /// were.
    fn apply(&mut self, event: &Event) -> Result<(), Refusal>;

    /// Answer one query with the engine's own figures. A figure is an
    /// exact decimal string (never a JSON number: no engine may round a
    /// cent through a float), a date is `YYYY-MM-DD`, and a format
    /// query's value is its lines or rows. A query the engine refuses
    /// (a period the rule refuses, a command it refuses) is an
    /// [`Err`].
    fn query(&self, q: &Query) -> Result<serde_json::Value, Refusal>;

    /// The rules and choices this engine declares.
    fn claims(&self) -> Claims;

    /// Apply one change that posts no entry (a `[[changes]]` entry).
    fn change(&mut self, change: &ChangeSpec) -> Result<(), Refusal>;
}

/// The refusal an engine reports for an error it has no example-word
/// reason for: the reason as given, under `fallback`.
pub fn fallback_refusal(fallback: RuleId, reason: impl Into<String>) -> Refusal {
    Refusal::new(fallback, reason)
}
