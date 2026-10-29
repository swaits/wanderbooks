//! bookspec's rule identifiers and their catalogue.
//!
//! Every rule of `docs/spec/rules.md` (B-1 to B-45) has one bookspec
//! identifier, `BS-<area>-<n>`, allocated by [`Area`] and numbered in
//! B-rule order within its area. `bookspec/spec/rules-index.md` is the
//! table, and a test keeps the two in step. The rules from B-39 on are
//! allocated in `plan/served.md` 8.0.3 but not yet stated; they carry an
//! identifier and an area and no status, so a later unit fills them in
//! without moving anyone else's number.

use std::sync::LazyLock;

use crate::engine::RuleId;

/// One of bookspec's areas: the subject a rule belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Area {
    /// The mechanics of an entry: lines, sides, amounts, dates.
    Entry,
    /// The ledger: the chart, accounts, the journal, the format.
    Ledger,
    /// Reports: the trial balance, the accounting equation, statements.
    Statement,
    /// Periods: closes, closed periods, what an omitted date means.
    Period,
    /// Partners and parties: shares, members' capital, the directory.
    Partner,
    /// Fixed assets and capitalization.
    Asset,
    /// Prepaid expenses and deferred revenue.
    Prepaid,
    /// Receivables and payables: documents, aging, settlements.
    Arap,
    /// Revenue: when it is recognized and how it is presented.
    Revenue,
    /// Information returns: 1099s and their thresholds.
    Tax1099,
    /// Interchange formats: the journal format gate and the IIF.
    Format,
}

impl Area {
    /// Every area, in the order the specification lists them.
    pub const ALL: [Area; 11] = [
        Area::Entry,
        Area::Ledger,
        Area::Statement,
        Area::Period,
        Area::Partner,
        Area::Asset,
        Area::Prepaid,
        Area::Arap,
        Area::Revenue,
        Area::Tax1099,
        Area::Format,
    ];

    /// The area's key in a rule identifier.
    pub fn key(self) -> &'static str {
        match self {
            Area::Entry => "entry",
            Area::Ledger => "ledger",
            Area::Statement => "statement",
            Area::Period => "period",
            Area::Partner => "partner",
            Area::Asset => "asset",
            Area::Prepaid => "prepaid",
            Area::Arap => "arap",
            Area::Revenue => "revenue",
            Area::Tax1099 => "1099",
            Area::Format => "format",
        }
    }

    /// The area `key` names.
    pub fn parse(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.key() == key)
    }
}

/// The status of a rule, in bookspec's vocabulary: what its statement
/// rests on. `docs/spec/rules.md`'s "DECISION" is bookspec's [`Status::Choice`]
/// (`plan/served.md` P13 step 3): the specification names the options, and
/// an engine declares its pick through [`crate::Claims`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Status {
    /// A cited authority states the rule.
    Authority,
    /// Common practice, where no authority states the rule.
    Convention,
    /// A human's or the house's choice among options the specification
    /// names.
    Choice,
}

impl Status {
    /// The status as the specification writes it.
    pub fn key(self) -> &'static str {
        match self {
            Status::Authority => "AUTHORITY",
            Status::Convention => "CONVENTION",
            Status::Choice => "CHOICE",
        }
    }

    /// The status `key` names.
    pub fn parse(key: &str) -> Option<Self> {
        [Status::Authority, Status::Convention, Status::Choice]
            .into_iter()
            .find(|s| s.key() == key)
    }
}

/// One rule of the specification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    /// Its number in `docs/spec/rules.md`, 1 to 45.
    pub b: u8,
    /// Its bookspec identifier.
    pub id: RuleId,
    /// Its area.
    pub area: Area,
    /// Its normative title, as `docs/spec/rules.md` states it.
    pub title: &'static str,
    /// What its statement rests on; `None` for a rule allocated but not
    /// yet stated (B-39 to B-45, `plan/served.md` 8.0.3).
    pub status: Option<Status>,
    /// The example ids that demonstrate it, by `Example::id`.
    pub examples: &'static [&'static str],
}

impl Rule {
    /// True when the rule has at least one example to check it.
    pub fn has_examples(&self) -> bool {
        !self.examples.is_empty()
    }
}

/// The catalogue: every rule, in B-rule order.
pub fn rules() -> &'static [Rule] {
    static RULES: LazyLock<Vec<Rule>> = LazyLock::new(build);
    &RULES
}

/// The rule `b` (1 to 45).
pub fn rule(b: u8) -> Option<&'static Rule> {
    rules().iter().find(|r| r.b == b)
}

/// The rule `id` names, if the specification states it.
pub fn rule_for_id(id: &RuleId) -> Option<&'static Rule> {
    rules().iter().find(|r| &r.id == id)
}

/// The rule a refusal belongs to, by its reason in the examples' words
/// (bookspec/README.md).
pub fn rule_for_refusal(reason: &str) -> Option<&'static Rule> {
    let b = match reason {
        "unbalanced" | "too_few_lines" => 1,
        "negative_amount" | "zero_amount" => 2,
        "finer_than_minor_unit" => 3,
        "parent_type_mismatch" => 4,
        "already_reversed" => 10,
        "period_closed" => 12,
        "end_before_start" | "start_after_today" => 30,
        "blank" | "unchanged" | "sibling_has_it" | "holds_a_tin" | "not_one_line"
        | "code_taken" => 31,
        "tax_category_required" => 32,
        "depreciation_past_disposal"
        | "entry_irreversible"
        | "already_disposed"
        | "invalid_depreciation_period" => 15,
        "invalid_schedule" => 16,
        "untagged_control_line" => 38,
        _ => return None,
    };
    rule(b)
}

/// The rules an engine can claim today: those with at least one example.
/// A rule with no example has no evidence, so an engine cannot claim it
/// (`plan/served.md` P13 step 4 fills the rest in).
pub fn covered_rules() -> Vec<&'static Rule> {
    rules().iter().filter(|r| r.has_examples()).collect()
}

fn build() -> Vec<Rule> {
    // The areas' ordinals are assigned here in B-rule order, one line per
    // rule: an area's n is its place in `Area::ALL`'s set of rules.
    let mut next = std::collections::BTreeMap::new();
    let mut out = Vec::with_capacity(45);
    for (b, area, title, status, examples) in SPEC {
        let n: u32 = *next.entry(*area).and_modify(|n| *n += 1).or_insert(1);
        out.push(Rule {
            b: *b,
            id: RuleId::new(*area, n),
            area: *area,
            title,
            status: *status,
            examples,
        });
    }
    out
}

type Spec = &'static [(
    u8,
    Area,
    &'static str,
    Option<Status>,
    &'static [&'static str],
)];

const SPEC: Spec = &[
    (
        1,
        Area::Entry,
        "Every entry is a double entry: debits equal credits",
        Some(Status::Authority),
        &[
            "entry-an-unbalanced-entry-is-refused",
            "statement-the-books-balance-and-the-equation-holds",
        ],
    ),
    (
        2,
        Area::Entry,
        "A line's amount is positive; its side gives the direction",
        Some(Status::Convention),
        &["entry-a-line-amount-is-positive"],
    ),
    (
        3,
        Area::Ledger,
        "One money through the ledger; foreign amounts at the transaction-date rate",
        Some(Status::Authority),
        &["ledger-a-fraction-of-a-cent-is-refused"],
    ),
    (
        4,
        Area::Ledger,
        "Five elements, closed list",
        Some(Status::Authority),
        &["ledger-a-child-account-has-its-parents-type"],
    ),
    (
        5,
        Area::Ledger,
        "Normal balances",
        Some(Status::Convention),
        &["statement-the-books-balance-and-the-equation-holds"],
    ),
    (
        6,
        Area::Statement,
        "The trial balance balances",
        Some(Status::Authority),
        &["statement-the-books-balance-and-the-equation-holds"],
    ),
    (
        7,
        Area::Statement,
        "The accounting equation",
        Some(Status::Authority),
        &["statement-the-books-balance-and-the-equation-holds"],
    ),
    (
        8,
        Area::Ledger,
        "Posted is forever: the book is written only forward",
        Some(Status::Authority),
        &["ledger-a-mistake-is-reversed-never-erased"],
    ),
    (
        9,
        Area::Ledger,
        "A mistake is corrected by an opposite entry, never erased",
        Some(Status::Authority),
        &["ledger-a-mistake-is-reversed-never-erased"],
    ),
    (
        10,
        Area::Ledger,
        "An entry is reversed at most once; a closing entry never",
        Some(Status::Choice),
        &["ledger-a-mistake-is-reversed-never-erased"],
    ),
    (
        11,
        Area::Period,
        "Revenue and expenses close into profit and loss, and that into capital",
        Some(Status::Authority),
        &["period-a-close-sweeps-the-year-into-capital"],
    ),
    (
        12,
        Area::Period,
        "A closed period takes no new postings; periods close in order",
        Some(Status::Convention),
        &["period-a-close-sweeps-the-year-into-capital"],
    ),
    (
        13,
        Area::Partner,
        "Partners share income by the partnership agreement; capital accounts carry it",
        Some(Status::Authority),
        &[
            "members-close.odd-cent-across-closes",
            "members-close.statement-before-and-after-a-close",
            "members-close.thirds-over-twelve-months",
            "members-close.true-up-of-a-per-close-split",
            "p3-party-members-equity-and-k1",
        ],
    ),
    (
        14,
        Area::Statement,
        "Statements: P&L for a period; cash flows by the direct method",
        Some(Status::Authority),
        &[
            "cash-flow.debt-to-equity",
            "cash-flow.government-fees",
            "cash-flow.paid-on-the-companys-behalf",
            "cash-flow.part-cash-purchase",
            "cash-flow.reversal-of-a-reversal",
            "cash-flow.reversed-property-contribution",
            "cash-flow.two-member-llc-first-year",
            "period-a-close-sweeps-the-year-into-capital",
        ],
    ),
    (
        15,
        Area::Asset,
        "Depreciation: straight line, salvage never depreciated, partial months by days",
        Some(Status::Convention),
        &[
            "assets-acquired-before-in-service",
            "assets-a-reversed-run-is-not-posted",
            "assets-book-value-is-what-the-books-carry",
            "depreciation-catch-up-month",
            "depreciation-disposal-after-a-late-change-undone",
            "depreciation-disposal-before-a-later-run",
            "depreciation-disposal-in-final-month",
            "depreciation-disposal-inside-a-posted-month",
            "depreciation-disposal-mid-month",
            "depreciation-final-month-by-days",
            "depreciation-final-month-drift",
            "depreciation-reversal-before-its-run-is-refused",
            "depreciation-run-reversed-after-its-month",
        ],
    ),
    (
        16,
        Area::Prepaid,
        "Prepaid expenses and deferred revenue are recognized over the period benefited or earned",
        Some(Status::Authority),
        &[
            "schedule-daily-deferred-revenue",
            "schedule-daily-small-first-and-last-month",
            "schedule-equal-months-start-on-the-first",
        ],
    ),
    (
        17,
        Area::Revenue,
        "Platform payouts are recorded gross when the business is the principal",
        Some(Status::Authority),
        &[],
    ),
    (
        18,
        Area::Arap,
        "Aging buckets: Current, 1-30, 31-60, 61-90, 90+ days past due",
        Some(Status::Convention),
        &[],
    ),
    (
        19,
        Area::Tax1099,
        "1099 reporting thresholds, payment methods, and decisions",
        Some(Status::Authority),
        &[
            "1099-a-dated-status-applies-by-payment-date",
            "1099-a-members-check-through-financing-counts",
            "1099-and-party-totals-for-a-members-payment",
            "1099-card-payments-are-excluded",
            "1099-money-between-own-accounts-pays-no-one",
            "1099-money-from-a-payee-is-netted-only-as-a-refund",
            "1099-the-threshold-is-the-years",
            "1099-worksheet-through-as-of",
        ],
    ),
    (
        20,
        Area::Ledger,
        "Every transaction has supporting documents; records are kept",
        Some(Status::Authority),
        &[],
    ),
    (
        21,
        Area::Entry,
        "Exact amounts in the books; rounding only on returns",
        Some(Status::Authority),
        &["entry-cents-add-exactly"],
    ),
    (
        22,
        Area::Asset,
        "Expense or capitalize (book policy, mirrors tax)",
        Some(Status::Authority),
        &[],
    ),
    (
        23,
        Area::Ledger,
        "Control roles: one account each, of the right kind, moved only when empty",
        Some(Status::Convention),
        &["arap-hand-control-posting-refused"],
    ),
    (
        24,
        Area::Asset,
        "Asset tags: one label per asset, never reused",
        Some(Status::Choice),
        &[],
    ),
    (
        25,
        Area::Asset,
        "Tax figures beside book value: the section 723 basis, the depreciable basis, the section 704(c) built-in gain",
        Some(Status::Authority),
        &[
            "assets-723-basis-apart-from-depreciable-basis",
            "assets-built-in-gain-fixed-at-contribution",
            "assets-four-contributed-devices",
        ],
    ),
    (
        26,
        Area::Partner,
        "Parties: one directory, no full TIN in the books",
        Some(Status::Authority),
        &["p3-party-tin-last4"],
    ),
    (
        27,
        Area::Format,
        "The journal format gate: newer is not corrupt",
        Some(Status::Choice),
        &[],
    ),
    (
        28,
        Area::Entry,
        "What new commands take: four-digit years, amounts a line can carry",
        Some(Status::Convention),
        &[],
    ),
    (
        29,
        Area::Ledger,
        "The books keep how they are kept",
        Some(Status::Choice),
        &[],
    ),
    (
        30,
        Area::Period,
        "A report asked for without dates covers this fiscal year so far",
        Some(Status::Authority),
        &["default-period-fiscal-year-to-date"],
    ),
    (
        31,
        Area::Ledger,
        "Renaming an account changes its label, nothing else",
        Some(Status::Choice),
        &["rename-changes-no-amount"],
    ),
    (
        32,
        Area::Ledger,
        "Every revenue and expense account maps to a line of the return",
        Some(Status::Authority),
        &["tax-category-required"],
    ),
    (
        33,
        Area::Statement,
        "A package as of a day shows the books through that day",
        Some(Status::Convention),
        &["as-of-package-stops-at-as-of"],
    ),
    (
        34,
        Area::Format,
        "The QuickBooks Desktop file (IIF): plain ASCII, every entry through as_of, closes left out, classes carried",
        Some(Status::Authority),
        &["iif-quickbooks-journal-file"],
    ),
    (
        35,
        Area::Partner,
        "A party's part of an entry, and a party's totals",
        Some(Status::Convention),
        &[
            "p3-party-accrual-counts-once",
            "p3-party-distribution-still-owed",
            "p3-party-gift-card-is-a-promise",
            "p3-party-members-equity-and-k1",
            "p3-party-own-liability-never-pays",
            "p3-party-reversed-card-payment",
            "p3-party-signs-and-reversals",
        ],
    ),
    (
        36,
        Area::Partner,
        "The K-1 worksheet reports tax-basis capital beside book capital",
        Some(Status::Authority),
        &[
            "partnership-k1-tax-basis-capital",
            "partnership-k1-tax-basis-odd-cents",
        ],
    ),
    (
        37,
        Area::Statement,
        "A project is a class with its facts; its figures are its tagged lines",
        Some(Status::Choice),
        &[
            "projects-invoice-and-bill-lines-count-by-project",
            "projects-net-is-its-tagged-lines",
            "projects-on-spec-costs-are-expenses",
        ],
    ),
    (
        38,
        Area::Arap,
        "A document's figures are its ledger lines that stand; a control account is its documents",
        Some(Status::Choice),
        &["arap-hand-control-posting-refused"],
    ),
    (
        39,
        Area::Arap,
        "Terms: when a document is due (allocated, not yet stated)",
        None,
        &[],
    ),
    (
        40,
        Area::Arap,
        "Numbers and the issued copy (allocated, not yet stated)",
        None,
        &[],
    ),
    (
        41,
        Area::Arap,
        "A debt that will not be collected is written off, and a recovery is income",
        Some(Status::Choice),
        &["bad-debt-direct-write-off"],
    ),
    (
        42,
        Area::Arap,
        "A settlement is exact, one party, and names its lines; the gross discount and the gross fee",
        Some(Status::Convention),
        &[],
    ),
    (
        43,
        Area::Entry,
        "Drafts and evidence (allocated, not yet stated)",
        None,
        &[],
    ),
    (
        44,
        Area::Ledger,
        "Evidence is read from the bytes, never trusted over them; identical files are kept once",
        Some(Status::Convention),
        &[],
    ),
    (
        45,
        Area::Ledger,
        "Audits and attestations (allocated, not yet stated)",
        None,
        &[],
    ),
];
