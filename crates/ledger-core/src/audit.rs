//! The audit's statements checks (plan P8; rules.md B-45): the books,
//! read as an auditor would, through a date, as a list of findings.
//!
//! ```text
//! GOAL: one deterministic list of what is wrong or unfinished in the
//!       books through a date, from the ledger alone, so an audit run is
//!       reproducible and its workpaper hashes the same twice.
//! SOURCE: IRS Pub. 583 (records that support entries); Rev. Proc. 98-25
//!         §5.01 (the relationship between the records and the books must
//!         be demonstrable); docs/spec/rules.md B-6 to B-14 (the
//!         statements), B-38 (control accounts), B-43 (evidence);
//!         DECISION D-9 (the audit and its bar), D-53 (a run, not a job).
//! INVARIANTS:
//!   1. Pure: reads the ledger and the facts [`AuditInput`] hands in,
//!      changes nothing, reads no clock: the same books and input give
//!      the same findings in the same order.
//!   2. One function per check, one finding code per check ([`CODES`]);
//!      a check that finds nothing says nothing.
//!   3. Only entries dated on or before `through` count, drafts and
//!      posted alike.
//!   4. A check that cannot be made is itself a finding, never a silent
//!      pass (`cash_flow_unfooted` for a statement the engine refuses).
//! SCENARIOS:
//!   1. IF the books are whole: SHOULD give no Failure or Warning.
//!   2. IF a draft is dated on or before `through`: SHOULD give exactly
//!      `draft_open`.
//! ```

use std::collections::BTreeSet;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::Serialize;

use crate::evidence::{Policy, Requirement};
use crate::settings::Settings;
use crate::{AccountType, DocStatus, EntryId, EntryStatus, Ledger, PeriodStatus, Side};

/// How much a finding matters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// The books are wrong or cannot be shown right: an audit with one is
    /// not clean.
    Failure,
    /// Unfinished work or a policy not met.
    Warning,
    /// Listed so the workpaper is whole; not a defect.
    Info,
}

impl Severity {
    /// Its wire name.
    pub fn key(self) -> &'static str {
        match self {
            Severity::Failure => "failure",
            Severity::Warning => "warning",
            Severity::Info => "info",
        }
    }
}

/// What a finding is about.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Subject {
    /// The books as a whole.
    Books,
    /// An account.
    Account(u64),
    /// An entry.
    Entry(u64),
    /// An invoice.
    Invoice(u64),
    /// A bill.
    Bill(u64),
    /// An accounting period.
    Period(u64),
    /// An amortization schedule.
    Schedule(u64),
    /// A filed attachment.
    Attachment(u64),
    /// A vendor, by name.
    Vendor(String),
    /// A stored file or the access log, by name.
    Named(String),
}

impl Subject {
    /// In words: `entry 7`, `books`.
    pub fn describe(&self) -> String {
        match self {
            Subject::Books => "books".into(),
            Subject::Account(i) => format!("account {i}"),
            Subject::Entry(i) => format!("entry {i}"),
            Subject::Invoice(i) => format!("invoice {i}"),
            Subject::Bill(i) => format!("bill {i}"),
            Subject::Period(i) => format!("period {i}"),
            Subject::Schedule(i) => format!("schedule {i}"),
            Subject::Attachment(i) => format!("attachment {i}"),
            Subject::Vendor(n) => format!("vendor {n}"),
            Subject::Named(n) => n.clone(),
        }
    }
}

/// One thing an audit found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Finding {
    /// Its stable code ([`CODES`]).
    pub code: &'static str,
    /// How much it matters.
    pub severity: Severity,
    /// What it is about.
    pub subject: Subject,
    /// In words, with the exact figures.
    pub message: String,
}

impl Finding {
    /// A finding of the severity its code's row says.
    pub fn new(code: &'static str, subject: Subject, message: impl Into<String>) -> Finding {
        let severity = CODES
            .iter()
            .find(|r| r.code == code)
            .map_or(Severity::Failure, |r| r.severity);
        Finding {
            code,
            severity,
            subject,
            message: message.into(),
        }
    }
}

/// One row of the code table: a check's finding code, how much it matters
/// and the predicate it states.
#[derive(Clone, Copy, Debug)]
pub struct CodeRow {
    /// The finding code.
    pub code: &'static str,
    /// Its severity.
    pub severity: Severity,
    /// The predicate: what is true when the code is reported.
    pub predicate: &'static str,
}

/// Every statements check, by code (the integrity checks of the store have
/// their own table, `ledger_store::audit::CODES`). `audit_detects_each_
/// seeded_defect` seeds each row.
pub const CODES: &[CodeRow] = &[
    CodeRow {
        code: "trial_balance_unbalanced",
        severity: Severity::Failure,
        predicate: "total debits differ from total credits in the trial balance through the date",
    },
    CodeRow {
        code: "equation_broken",
        severity: Severity::Failure,
        predicate: "assets differ from liabilities plus equity (current earnings included)",
    },
    CodeRow {
        code: "pnl_not_articulated",
        severity: Severity::Failure,
        predicate: "net income since the first entry differs from current earnings plus what closes moved to equity",
    },
    CodeRow {
        code: "cash_flow_unfooted",
        severity: Severity::Failure,
        predicate: "the statement of cash flows cannot be made, or its sections do not sum to the cash accounts' change",
    },
    CodeRow {
        code: "control_difference",
        severity: Severity::Failure,
        predicate: "a control account's balance differs from its documents plus its unattached lines",
    },
    CodeRow {
        code: "unattached_control_line",
        severity: Severity::Warning,
        predicate: "a line on a control account carries no document",
    },
    CodeRow {
        code: "document_overpaid",
        severity: Severity::Warning,
        predicate: "a document has more paid against it than it is for",
    },
    CodeRow {
        code: "close_split_drifts",
        severity: Severity::Failure,
        predicate: "a member's running share of the closes is more than a minor unit from its exact share",
    },
    CodeRow {
        code: "schedule_behind",
        severity: Severity::Warning,
        predicate: "an amortization schedule has a month due through the date that is not posted",
    },
    CodeRow {
        code: "depreciation_behind",
        severity: Severity::Warning,
        predicate: "the asset schedules say more depreciation is due than the books carry",
    },
    CodeRow {
        code: "register_differs",
        severity: Severity::Failure,
        predicate: "the fixed asset register's cost differs from the cost accounts' balances",
    },
    CodeRow {
        code: "evidence_missing",
        severity: Severity::Failure,
        predicate: "a posted entry the receipt policy covers has no receipt",
    },
    CodeRow {
        code: "legacy_reversal_exempt",
        severity: Severity::Info,
        predicate: "a reversal the policy would cover has no receipt of its own; its original's evidence stands for it (D-47)",
    },
    CodeRow {
        code: "draft_open",
        severity: Severity::Warning,
        predicate: "a draft is dated on or before the date",
    },
    CodeRow {
        code: "tax_category_missing",
        severity: Severity::Warning,
        predicate: "a revenue or expense account has no tax category",
    },
    CodeRow {
        code: "vendor_1099_undecided",
        severity: Severity::Warning,
        predicate: "a vendor paid in a way that could need a 1099 has no decision",
    },
    CodeRow {
        code: "settings_problem",
        severity: Severity::Warning,
        predicate: "the settings name an account or role the chart cannot honor",
    },
];

/// What the ledger cannot know by itself and the caller reads from the
/// books around it.
#[derive(Clone, Copy, Debug)]
pub struct AuditInput<'a> {
    /// The books' settings (the receipt policy).
    pub settings: &'a Settings,
    /// The entries that have at least one receipt filed against them, not
    /// withdrawn.
    pub receipts: &'a BTreeSet<EntryId>,
    /// The statement of cash flows tried from the first entry through the
    /// date with the books' role settings: its footing, or why it could
    /// not be made; `None` when the settings name no cash accounts or
    /// there is nothing to state.
    pub cash_flow: Option<&'a Result<crate::cashflow::CashFlowFooting, String>>,
    /// What the settings check already says is wrong, in words.
    pub settings_problems: &'a [String],
    /// Vendors paid in a way that could need a 1099 with no decision, and
    /// what they were paid.
    pub undecided_1099: &'a [(String, Decimal)],
}

/// All the statements checks through `through`, in a fixed order:
/// failures first, then by code and subject.
pub fn statements(ledger: &Ledger, input: &AuditInput<'_>, through: NaiveDate) -> Vec<Finding> {
    let mut out = Vec::new();
    out.extend(trial_balance(ledger, through));
    out.extend(equation(ledger, through));
    out.extend(articulation(ledger, through));
    out.extend(check_cash_flow(input.cash_flow));
    out.extend(control(ledger, through));
    out.extend(closes(ledger, through));
    out.extend(schedules(ledger, through));
    out.extend(assets(ledger, through));
    out.extend(evidence(ledger, input, through));
    out.extend(drafts(ledger, through));
    out.extend(tax_categories(ledger));
    out.extend(vendors_1099(input.undecided_1099));
    out.extend(settings_problems(input.settings_problems));
    sort(&mut out);
    out
}

/// Findings in their reporting order.
pub fn sort(findings: &mut [Finding]) {
    findings.sort_by(|a, b| {
        (a.severity, a.code, &a.subject, &a.message)
            .cmp(&(b.severity, b.code, &b.subject, &b.message))
    });
}

fn trial_balance(ledger: &Ledger, through: NaiveDate) -> Option<Finding> {
    let tb = crate::reports::trial_balance_as_of(ledger, through);
    check_trial_balance(&tb)
}

/// The trial balance check on a built trial balance.
pub fn check_trial_balance(tb: &crate::reports::TrialBalance) -> Option<Finding> {
    (!tb.balanced()).then(|| {
        Finding::new(
            "trial_balance_unbalanced",
            Subject::Books,
            format!(
                "debits {} differ from credits {}",
                tb.total_debits, tb.total_credits
            ),
        )
    })
}

fn equation(ledger: &Ledger, through: NaiveDate) -> Option<Finding> {
    match crate::reports::balance_sheet_as_of(ledger, through) {
        Ok(bs) => check_equation(&bs),
        Err(e) => Some(Finding::new(
            "equation_broken",
            Subject::Books,
            format!("the balance sheet cannot be made: {e}"),
        )),
    }
}

/// The accounting equation on a built balance sheet.
pub fn check_equation(bs: &crate::reports::BalanceSheetStatement) -> Option<Finding> {
    (!bs.balanced()).then(|| {
        Finding::new(
            "equation_broken",
            Subject::Books,
            format!(
                "assets {} differ from liabilities {} plus equity {}",
                bs.total_assets, bs.total_liabilities, bs.total_equity
            ),
        )
    })
}

/// Net income moved to equity by the closes dated through `through`:
/// revenue closed less expenses closed, read off the closing entries'
/// lines.
fn closed_net(ledger: &Ledger, through: NaiveDate) -> Decimal {
    let mut net = Decimal::ZERO;
    for id in ledger.closing_entry_ids() {
        let Some(e) = ledger.get_entry(id) else {
            continue;
        };
        if e.status() != EntryStatus::Posted || e.date() > through {
            continue;
        }
        for l in e.lines() {
            let Some(a) = ledger.chart().get(l.account) else {
                continue;
            };
            let signed = match l.side {
                Side::Debit => l.amount.amount,
                Side::Credit => -l.amount.amount,
            };
            if matches!(a.account_type, AccountType::Revenue | AccountType::Expense) {
                net += signed;
            }
        }
    }
    net
}

fn articulation(ledger: &Ledger, through: NaiveDate) -> Option<Finding> {
    let first = ledger
        .posted_entries()
        .filter(|e| e.date() <= through)
        .map(|e| e.date())
        .min()?;
    let pnl = crate::reports::profit_and_loss_for_period(ledger, first, through, None).ok()?;
    let bs = crate::reports::balance_sheet_as_of(ledger, through).ok()?;
    check_articulation(
        pnl.net_income(),
        bs.current_earnings,
        closed_net(ledger, through),
    )
}

/// The P&L ties to equity: net income since the first entry is current
/// earnings plus what the closes moved into equity.
pub fn check_articulation(
    net_income: Decimal,
    current_earnings: Decimal,
    closed: Decimal,
) -> Option<Finding> {
    (net_income != current_earnings + closed).then(|| {
        Finding::new(
            "pnl_not_articulated",
            Subject::Books,
            format!(
                "net income {net_income} differs from current earnings {current_earnings} plus closed {closed}"
            ),
        )
    })
}

/// The statement of cash flows, tried by the caller from the books' role
/// settings: its footing, or why it could not be made.
pub fn check_cash_flow(
    made: Option<&Result<crate::cashflow::CashFlowFooting, String>>,
) -> Option<Finding> {
    match made? {
        Ok(f) if f.difference.is_zero() => None,
        Ok(f) => Some(Finding::new(
            "cash_flow_unfooted",
            Subject::Books,
            format!(
                "the sections sum to {} but the cash accounts changed by {}",
                f.sections_sum, f.net_change
            ),
        )),
        Err(e) => Some(Finding::new(
            "cash_flow_unfooted",
            Subject::Books,
            format!("the statement of cash flows cannot be made: {e}"),
        )),
    }
}

fn control(ledger: &Ledger, through: NaiveDate) -> Vec<Finding> {
    match ledger.control_check(through) {
        Ok(checks) => check_control(&checks),
        Err(e) => vec![Finding::new(
            "control_difference",
            Subject::Books,
            format!("the control check cannot be made: {e}"),
        )],
    }
}

/// The control accounts' tie-out, as the engine read it.
pub fn check_control(checks: &[crate::arap::ControlCheck]) -> Vec<Finding> {
    let mut out = Vec::new();
    for c in checks {
        if !c.difference.is_zero() {
            out.push(Finding::new(
                "control_difference",
                Subject::Account(c.account),
                format!(
                    "the {} account's balance {} differs from its documents {} by {}",
                    c.role, c.balance, c.documents_total, c.difference
                ),
            ));
        }
        for u in &c.unattached {
            out.push(Finding::new(
                "unattached_control_line",
                Subject::Entry(u.entry),
                format!(
                    "line {} of entry {} ({}), {} on the {} account, carries no document",
                    u.line, u.entry, u.date, u.amount, c.role
                ),
            ));
        }
        for a in c
            .attention
            .iter()
            .filter(|a| a.status == DocStatus::Overpaid)
        {
            let subject = match a.doc {
                crate::DocRef::Invoice(i) => Subject::Invoice(i),
                crate::DocRef::Bill(b) => Subject::Bill(b),
            };
            out.push(Finding::new(
                "document_overpaid",
                subject,
                format!("{} is overpaid: its total is {}", a.doc, a.total),
            ));
        }
    }
    out
}

fn closes(ledger: &Ledger, through: NaiveDate) -> Vec<Finding> {
    let unit = Decimal::new(1, ledger.base_currency().minor_units().unwrap_or(2));
    let mut out = Vec::new();
    for p in ledger.list_periods() {
        if p.status != PeriodStatus::Closed || p.end > through {
            continue;
        }
        if let Some((_, split)) = ledger.member_close_split(p.id) {
            out.extend(check_split(p.id, &p.name, &split, unit));
        }
    }
    out
}

/// One close's members: each one's running total of the closes must be
/// within a minor unit of its exact share (the cumulative rule,
/// `kernel::apportion_cumulative`).
pub fn check_split(
    period: u64,
    name: &str,
    split: &[crate::periods::MemberSplit],
    unit: Decimal,
) -> Vec<Finding> {
    split
        .iter()
        .filter(|m| (m.running - m.exact).abs() > unit)
        .map(|m| {
            Finding::new(
                "close_split_drifts",
                Subject::Period(period),
                format!(
                    "after the close of {name}, account {} holds {} of the closes where its exact share is {}",
                    m.capital, m.running, m.exact
                ),
            )
        })
        .collect()
}

fn schedules(ledger: &Ledger, through: NaiveDate) -> Vec<Finding> {
    let mut out = Vec::new();
    for s in ledger.schedules() {
        let months = match s.due(through) {
            Ok(m) => m,
            Err(e) => {
                out.push(Finding::new(
                    "schedule_behind",
                    Subject::Schedule(s.id),
                    format!("\"{}\" cannot be planned: {e}", s.description),
                ));
                continue;
            }
        };
        if let Some((first, _)) = months.first() {
            let total: Decimal = months.iter().map(|(_, a)| *a).sum();
            out.push(Finding::new(
                "schedule_behind",
                Subject::Schedule(s.id),
                format!(
                    "\"{}\" has {} month(s) due and not posted, from {first}, {total} in all",
                    s.description,
                    months.len()
                ),
            ));
        }
    }
    out
}

fn assets(ledger: &Ledger, through: NaiveDate) -> Vec<Finding> {
    match crate::reports::fixed_asset_register(ledger, through) {
        Ok(reg) => check_register(&reg),
        Err(e) => vec![Finding::new(
            "register_differs",
            Subject::Books,
            format!("the fixed asset register cannot be made: {e}"),
        )],
    }
}

/// The register against the books' cost and accumulated depreciation.
pub fn check_register(reg: &crate::reports::AssetRegister) -> Vec<Finding> {
    let mut out = Vec::new();
    if reg.unposted_depreciation() > Decimal::ZERO {
        out.push(Finding::new(
            "depreciation_behind",
            Subject::Books,
            format!(
                "the schedules say {} of depreciation is due and the books carry {}",
                reg.total_accumulated_scheduled, reg.ledger_accumulated
            ),
        ));
    }
    if !reg.unregistered_cost().is_zero() {
        out.push(Finding::new(
            "register_differs",
            Subject::Books,
            format!(
                "the register holds {} of cost and the cost accounts {}",
                reg.total_cost, reg.ledger_cost
            ),
        ));
    }
    out
}

fn evidence(ledger: &Ledger, input: &AuditInput<'_>, through: NaiveDate) -> Vec<Finding> {
    let policy = Policy::from_settings(input.settings);
    let mut out = Vec::new();
    for e in ledger.posted_entries().filter(|e| e.date() <= through) {
        if input.receipts.contains(&e.id()) {
            continue;
        }
        match crate::evidence::requirement(ledger, e, input.settings) {
            Requirement::Required { reason } => out.push(Finding::new(
                "evidence_missing",
                Subject::Entry(e.id()),
                format!("entry {} ({}) has no receipt: {reason}", e.id(), e.date()),
            )),
            Requirement::Exempt { .. } if e.reversal_of().is_some() => {
                if policy.receipt_required(ledger, e) {
                    out.push(Finding::new(
                        "legacy_reversal_exempt",
                        Subject::Entry(e.id()),
                        format!(
                            "entry {} reverses entry {}: its evidence is the original's",
                            e.id(),
                            e.reversal_of().unwrap_or_default()
                        ),
                    ));
                }
            }
            Requirement::Exempt { .. } => {}
        }
    }
    out
}

fn drafts(ledger: &Ledger, through: NaiveDate) -> Vec<Finding> {
    ledger
        .drafts()
        .into_iter()
        .filter(|e| e.date() <= through)
        .map(|e| {
            Finding::new(
                "draft_open",
                Subject::Entry(e.id()),
                format!(
                    "draft {} ({}) \"{}\" is not posted",
                    e.id(),
                    e.date(),
                    e.description()
                ),
            )
        })
        .collect()
}

fn tax_categories(ledger: &Ledger) -> Vec<Finding> {
    crate::tax::uncategorized_accounts(ledger)
        .into_iter()
        .map(|a| {
            Finding::new(
                "tax_category_missing",
                Subject::Account(a.id),
                format!("account {} {} has no tax category", a.code, a.name),
            )
        })
        .collect()
}

fn vendors_1099(undecided: &[(String, Decimal)]) -> Vec<Finding> {
    undecided
        .iter()
        .map(|(name, paid)| {
            Finding::new(
                "vendor_1099_undecided",
                Subject::Vendor(name.clone()),
                format!(
                    "{name} was paid {paid} in a way that could need a 1099 and has no decision"
                ),
            )
        })
        .collect()
}

fn settings_problems(problems: &[String]) -> Vec<Finding> {
    problems
        .iter()
        .map(|p| Finding::new("settings_problem", Subject::Books, p.clone()))
        .collect()
}

// ---------------------------------------------------------------------------
// Scope, method and the attestation bar (rules.md B-45; D-9)
// ---------------------------------------------------------------------------

/// What an audit covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// A month's books, for its month-end attestation.
    Month,
    /// A fiscal year.
    Annual,
    /// The year, for the CPA handoff.
    CpaHandoff,
    /// Everything since the first entry.
    Full,
}

impl Scope {
    /// Its wire name.
    pub fn key(self) -> &'static str {
        match self {
            Scope::Month => "month",
            Scope::Annual => "annual",
            Scope::CpaHandoff => "cpa_handoff",
            Scope::Full => "full",
        }
    }

    /// The scope a word names.
    pub fn parse(s: &str) -> Option<Scope> {
        match s.trim().to_ascii_lowercase().as_str() {
            "month" => Some(Scope::Month),
            "annual" => Some(Scope::Annual),
            "cpa_handoff" => Some(Scope::CpaHandoff),
            "full" => Some(Scope::Full),
            _ => None,
        }
    }

    /// Whether a period attested on this audit is held to the whole bar
    /// (adversarial, several verifiers) and not the effort alone (D-9).
    pub fn full_bar(self) -> bool {
        !matches!(self, Scope::Month)
    }

    /// Whether an audit of this scope is one the CPA package may rest on.
    pub fn serves_handoff(self) -> bool {
        matches!(self, Scope::CpaHandoff | Scope::Full)
    }
}

/// How hard the checker thought, as the client reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    /// low
    Low,
    /// medium
    Medium,
    /// high
    High,
    /// xhigh
    Xhigh,
    /// max
    Max,
}

impl Effort {
    /// Its wire name.
    pub fn key(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
            Effort::Xhigh => "xhigh",
            Effort::Max => "max",
        }
    }

    /// The effort a word names.
    pub fn parse(s: &str) -> Option<Effort> {
        match s.trim().to_ascii_lowercase().as_str() {
            "low" => Some(Effort::Low),
            "medium" => Some(Effort::Medium),
            "high" => Some(Effort::High),
            "xhigh" => Some(Effort::Xhigh),
            "max" => Some(Effort::Max),
            _ => None,
        }
    }
}

/// How an attestation was made, as the client says (self-reported: the
/// books record the claim, never prove it; rules.md B-45).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Method {
    /// The model that checked, as it names itself.
    pub model: Option<String>,
    /// How hard it thought.
    pub reasoning_effort: Option<Effort>,
    /// Whether the check was adversarial: a verifier set to find what is
    /// wrong, not to confirm.
    pub adversarial: Option<bool>,
    /// How many independent verifier agents checked.
    pub verifier_agents: Option<u32>,
}

/// What counts as an attestation (`policy.attestation_bar`; D-9). The
/// default: reasoning effort high, two independent verifiers, adversarial.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct AttestationBar {
    /// The least effort that counts, for every attestation.
    pub reasoning_effort: Effort,
    /// The fewest independent verifier agents a year-level attestation
    /// needs.
    pub verifier_agents: u32,
    /// Whether a year-level attestation must be adversarial.
    pub adversarial: bool,
}

impl Default for AttestationBar {
    fn default() -> Self {
        AttestationBar {
            reasoning_effort: Effort::High,
            verifier_agents: 2,
            adversarial: true,
        }
    }
}

impl AttestationBar {
    /// The bar a setting's text names: `key=value` words in any order,
    /// separated by spaces or commas (`effort=high verifiers=2
    /// adversarial=true`); a word left out keeps its default.
    pub fn parse(s: &str) -> Result<AttestationBar, String> {
        let mut bar = AttestationBar::default();
        for word in s.split(|c: char| c.is_whitespace() || c == ',') {
            if word.is_empty() {
                continue;
            }
            let (k, v) = word.split_once('=').ok_or_else(|| {
                format!("{word:?}: expected key=value (effort, verifiers, adversarial)")
            })?;
            match k {
                "effort" | "reasoning_effort" => {
                    bar.reasoning_effort = Effort::parse(v).ok_or_else(|| {
                        format!("effort {v:?} is not low, medium, high, xhigh or max")
                    })?;
                }
                "verifiers" | "verifier_agents" => {
                    bar.verifier_agents = v
                        .parse::<u32>()
                        .ok()
                        .filter(|n| (1..=64).contains(n))
                        .ok_or_else(|| format!("verifiers {v:?} is not a count from 1 to 64"))?;
                }
                "adversarial" => {
                    bar.adversarial = match v {
                        "true" => true,
                        "false" => false,
                        _ => return Err(format!("adversarial {v:?} is not true or false")),
                    };
                }
                other => {
                    return Err(format!(
                        "{other:?} is not a part of the bar: effort, verifiers, adversarial"
                    ));
                }
            }
        }
        Ok(bar)
    }

    /// The bar in its one canonical spelling.
    pub fn describe(&self) -> String {
        format!(
            "effort={} verifiers={} adversarial={}",
            self.reasoning_effort.key(),
            self.verifier_agents,
            self.adversarial
        )
    }

    /// Whether an attestation made `method` counts. A claim that
    /// disagrees with the books never does. Every attestation needs the
    /// effort; one given on a year-level audit (`full`) needs the whole
    /// bar. Evaluated when asked, against the bar as it stands (D-9: no
    /// bar versioning).
    pub fn counts(&self, method: &Method, full: bool, disagrees: bool) -> bool {
        if disagrees {
            return false;
        }
        if !method
            .reasoning_effort
            .is_some_and(|e| e >= self.reasoning_effort)
        {
            return false;
        }
        if !full {
            return true;
        }
        (!self.adversarial || method.adversarial == Some(true))
            && method
                .verifier_agents
                .is_some_and(|n| n >= self.verifier_agents)
    }
}
