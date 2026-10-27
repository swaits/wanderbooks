//! How the books are kept, through the server: `books.start`, settings
//! (`books.settings`, `books.set_setting`, `books.export_settings`),
//! conventions, decisions, open items, drafts from posting templates,
//! and approvals (plan/served.md P1, 4.2, 4.9).
//!
//! ```text
//! GOAL: an agent anywhere learns how these books are kept from the books
//!       themselves, and records how they are kept there, sourced.
//! SOURCE: plan/served.md I-3, 4.2 (books.start v1), 5.2-5.4; DECISION
//!         (owner, 2026-10-16): light governance, decisions recorded
//!         with who made them and when.
//! INVARIANTS:
//!   1. Every change names who decided it and when (decided_by;
//!      decided_on defaults to today in the company's time zone).
//!   2. The books keep their own settings, and only books.set_setting
//!      changes them (old books' settings file is read once, by
//!      `wanderbooks books adopt`).
//!   3. Identity values (the company's EIN) leave the server masked.
//!   4. books.start is read-only and at most START_MAX bytes.
//! ```

use super::*;
use ledger_core::conventions::{ConventionKind, NewConvention, NewDecision, NewOpenItem};
use ledger_core::settings::{self as core_settings, Change, KEYS, Sensitivity, Source};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The most bytes `books.start` returns (plan/served.md 4.2).
pub(crate) const START_MAX: usize = 8 * 1024;

/// Who decided a change, and when: on every tool that records one.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SourceIn {
    /// Who decided: a person's name ("Pat Smith"), as the human says.
    pub decided_by: String,
    /// When they decided, YYYY-MM-DD. Default: today.
    #[serde(default)]
    pub decided_on: Option<String>,
    /// The rule or document it rests on, if any ("IRS Pub. 538").
    #[serde(default)]
    pub citation: Option<String>,
    /// Anything else worth keeping with it.
    #[serde(default)]
    pub note: Option<String>,
}

impl SourceIn {
    fn source(&self) -> Result<Source, String> {
        Ok(Source {
            decided_by: self.decided_by.clone(),
            decided_on: match &self.decided_on {
                Some(d) => parse_date(d)?,
                None => today(),
            },
            citation: self.citation.clone(),
            note: self.note.clone(),
        })
    }
}

/// A source as the server shows it.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SourceOut {
    /// Who decided.
    pub decided_by: String,
    /// When, YYYY-MM-DD.
    pub decided_on: String,
    /// What it rests on, if anything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citation: Option<String>,
    /// Anything kept with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

fn source_out(s: &Source) -> SourceOut {
    SourceOut {
        decided_by: s.decided_by.clone(),
        decided_on: s.decided_on.to_string(),
        citation: s.citation.clone(),
        note: s.note.clone(),
    }
}

// ----------------------------------------------------------------------
// Parameters and results
// ----------------------------------------------------------------------

/// Parameters for `books.start`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct StartParams {
    /// What the session is for: daily, onboarding, close, or audit
    /// (shapes next_steps). Default: daily.
    #[serde(default)]
    pub focus: Option<String>,
}

/// The books, in brief.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StartBookOut {
    /// The company's name.
    pub name: String,
    /// Its time zone (IANA), or null: the computer's.
    pub timezone: Option<String>,
    /// Today, in that time zone.
    pub today: String,
    /// The fiscal year containing today: first and last day.
    pub fiscal_year: FiscalYearOut,
    /// The books' currency.
    pub base_currency: String,
    /// Accrual or Cash.
    pub basis: String,
}

/// A fiscal year.
#[derive(Debug, Serialize, JsonSchema)]
pub struct FiscalYearOut {
    /// First day.
    pub start: String,
    /// Last day.
    pub end: String,
}

/// The journal's head.
#[derive(Debug, Serialize, JsonSchema)]
pub struct HeadOut {
    /// The last line's seq.
    pub seq: u64,
    /// Its hash: a verified copy of the books ends here.
    pub hash: String,
}

/// The rules in force.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RulesOut {
    /// Which entries need a receipt.
    pub evidence: String,
    /// Whether entries name their counterparty: off, suggest, or require.
    pub parties: String,
    /// Whether approval rules block posting: off or enforce.
    pub approval_rules: String,
}

/// A setting, in brief.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SettingBriefOut {
    /// Its key.
    pub key: String,
    /// Its value, as text (identity values masked).
    pub value: String,
    /// Who set it.
    pub set_by: String,
    /// The journal line that set it.
    pub seq: u64,
}

/// A convention, in brief.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ConventionBriefOut {
    /// Its id.
    pub id: String,
    /// What it is.
    pub kind: String,
    /// Its title.
    pub title: String,
    /// Its first line (empty when books.start had to save room: read
    /// books.conventions).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
}

/// An open item, in brief.
#[derive(Debug, Serialize, JsonSchema)]
pub struct OpenItemBriefOut {
    /// Its id.
    pub id: u64,
    /// What is open.
    pub title: String,
    /// When it is due, if it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
}

/// A project, in brief.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ProjectBriefOut {
    /// Its id (a class id).
    pub id: u32,
    /// Its name.
    pub name: String,
    /// active or paused.
    pub status: String,
    /// Its kind, if recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// A handbook section.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SectionOut {
    /// Its id, for books.handbook.
    pub id: String,
    /// Its title.
    pub title: String,
}

/// A format upgrade the books wait for.
#[derive(Debug, Serialize, JsonSchema)]
pub struct FormatUpgradeOut {
    /// The guard: format_upgrade_pending (what a refused change says).
    pub guard: String,
    /// The journal format the books are in.
    pub format: u32,
    /// What the upgrade adds, in words.
    pub adds: Vec<String>,
    /// Who can give the go-ahead, and how.
    pub how: String,
}

/// Something to do next.
#[derive(Debug, Serialize, JsonSchema)]
pub struct NextStepOut {
    /// The tool to call.
    pub tool: String,
    /// Its arguments, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
    /// Why.
    pub why: String,
}

/// Result of `books.start`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StartOut {
    /// The books, in brief.
    pub book: StartBookOut,
    /// The journal's head.
    pub head: HeadOut,
    /// needs_onboarding (no chart of accounts yet) or active.
    pub state: String,
    /// The rules in force.
    pub rules: RulesOut,
    /// Every setting set, with who set it.
    pub settings_summary: Vec<SettingBriefOut>,
    /// How this company keeps its books: its conventions (company policy
    /// data; the whole text is in books.conventions).
    pub conventions: Vec<ConventionBriefOut>,
    /// What is open, oldest first.
    pub open_items: Vec<OpenItemBriefOut>,
    /// The projects under way (active and paused), by name, at most 20
    /// (books.projects lists every one with its figures).
    pub projects: Vec<ProjectBriefOut>,
    /// Lists cut short to fit: what was left out, e.g. "conventions: 4
    /// more" (the full lists are in books.conventions,
    /// books.open_items and books.projects).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub truncated: Vec<String>,
    /// The handbook's sections (books.handbook {section}).
    pub handbook_sections: Vec<SectionOut>,
    /// The format upgrade these books wait for, if they do (plan 3.9,
    /// D-39): an older version wrote them, so they work on their old
    /// journal format, and whatever needs the new one is refused, until
    /// the owner gives the go-ahead. No tool can: tell the human.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_upgrade: Option<FormatUpgradeOut>,
    /// What to do next, in order.
    pub next_steps: Vec<NextStepOut>,
    /// How the books are kept from agents: "same_user (advisory)", hidden
    /// from them in the human's own account, not locked against them
    /// (plan D-32): reach them only through these tools.
    pub isolation: String,
}

/// One setting change.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ChangeIn {
    /// The key (books.settings lists every key with what it takes).
    pub key: String,
    /// The new value as the key takes it (text, or a list for account
    /// codes and members); null clears it back to its default.
    #[serde(default)]
    pub value: Option<Value>,
}

/// Parameters for `books.set_setting`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetSettingParams {
    /// The changes, made together or not at all.
    pub changes: Vec<ChangeIn>,
    /// Who decided, and when.
    #[serde(flatten)]
    pub source: SourceIn,
}

/// One change made (or to make).
#[derive(Debug, Serialize, JsonSchema)]
pub struct ChangeOut {
    /// The key.
    pub key: String,
    /// The value it had, as text (identity values masked), or null.
    pub was: Option<String>,
    /// The value it has now, as text (identity values masked), or null:
    /// back to its default.
    pub value: Option<String>,
}

/// Result of `books.set_setting`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SetSettingOut {
    /// The changes made; empty when every value was already set.
    pub changed: Vec<ChangeOut>,
    /// Settings that disagree with the chart now, each saying what to
    /// change. Empty: nothing to fix.
    pub problems: Vec<String>,
}

/// Result of `books.export_settings`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ExportSettingsOut {
    /// The settings as a settings file (TOML), each with who decided it
    /// as a comment; identity values masked. A copy: editing it changes
    /// nothing.
    pub toml_text: String,
}

/// Parameters for `books.conventions`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ConventionsParams {
    /// Only this kind: posting_template, approval_rule, policy,
    /// close_step, calendar, mapping, or note.
    #[serde(default)]
    pub kind: Option<String>,
    /// Include retired conventions.
    #[serde(default)]
    pub include_retired: bool,
}

/// A convention.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ConventionOut {
    /// Its id.
    pub id: String,
    /// What it is.
    pub kind: String,
    /// Its title.
    pub title: String,
    /// The convention in words.
    pub body_md: String,
    /// Its structure (a posting template's lines, an approval rule's
    /// scope), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured: Option<Value>,
    /// 1 for the first version, then 2, 3, ...
    pub revision: u32,
    /// Who decided this revision, and when.
    pub source: SourceOut,
    /// The journal line of this revision.
    pub seq: u64,
    /// Why it was retired, if it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retired: Option<String>,
}

/// Result of `books.conventions`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ConventionsOut {
    /// The conventions, by id.
    pub conventions: Vec<ConventionOut>,
}

/// Parameters for `books.set_convention`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetConventionParams {
    /// Its id: lowercase letters, digits and hyphens ("joint-expense").
    /// An id already set revises that convention; a retired id can't
    /// return.
    pub id: String,
    /// posting_template, approval_rule, policy, close_step, calendar,
    /// mapping, or note.
    pub kind: String,
    /// A short title.
    pub title: String,
    /// The convention in words (Markdown).
    pub body_md: String,
    /// For a posting_template: {description?, lines: [{account (a code, or
    /// a slot such as "{expense}" that each draft names),
    /// side (debit|credit), share ("1", "0.5")}]}, each side's shares
    /// summing to 1. For an approval_rule: {approver, accounts? (codes),
    /// amount_over?}. Other kinds take none.
    #[serde(default)]
    pub structured: Option<Value>,
    /// Who decided, and when.
    #[serde(flatten)]
    pub source: SourceIn,
}

/// Parameters for `books.retire_convention`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RetireConventionParams {
    /// The convention's id.
    pub id: String,
    /// Why it no longer applies.
    pub reason: String,
    /// Who decided, and when.
    #[serde(flatten)]
    pub source: SourceIn,
}

/// Parameters for `books.record_decision`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RecordDecisionParams {
    /// What it is about ("Materiality policy").
    pub subject: String,
    /// The decision in words (Markdown).
    pub decision_md: String,
    /// Who decided (names), at least one.
    pub decided_by: Vec<String>,
    /// When, YYYY-MM-DD.
    pub decided_on: String,
    /// The filed document behind it, if any (an attachment id).
    #[serde(default)]
    pub evidence_attachment: Option<u64>,
    /// The decision it replaces, if any.
    #[serde(default)]
    pub supersedes: Option<u64>,
    /// The rule or document it rests on, if any.
    #[serde(default)]
    pub citation: Option<String>,
    /// The projects it is about (ids or names), if any.
    #[serde(default)]
    pub projects: Vec<String>,
}

/// A decision.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DecisionOut {
    /// Its id.
    pub id: u64,
    /// What it is about.
    pub subject: String,
    /// The decision.
    pub decision_md: String,
    /// Who decided.
    pub decided_by: Vec<String>,
    /// When.
    pub decided_on: String,
    /// The document behind it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_attachment: Option<u64>,
    /// The decision it replaced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<u64>,
    /// The decision that replaced it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<u64>,
    /// What it rests on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citation: Option<String>,
    /// The projects it is about.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<ProjectRefOut>,
    /// The journal line.
    pub seq: u64,
}

/// Parameters for `books.decisions`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct DecisionsParams {
    /// Include decisions a later one replaced.
    #[serde(default)]
    pub include_superseded: bool,
    /// Only the decisions naming this project (id or name).
    #[serde(default)]
    pub project: Option<String>,
}

/// Result of `books.decisions`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DecisionsOut {
    /// The decisions, oldest first.
    pub decisions: Vec<DecisionOut>,
}

/// Parameters for `books.raise_item`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RaiseItemParams {
    /// What is open, in a line.
    pub title: String,
    /// More, if any (Markdown).
    #[serde(default)]
    pub detail_md: Option<String>,
    /// When it is due, YYYY-MM-DD, if it is.
    #[serde(default)]
    pub due: Option<String>,
    /// Who raised it, and when.
    #[serde(flatten)]
    pub source: SourceIn,
}

/// Parameters for `books.resolve_item`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ResolveItemParams {
    /// The item's id.
    pub id: u64,
    /// How it was resolved.
    pub resolution: String,
    /// Who resolved it, and when.
    #[serde(flatten)]
    pub source: SourceIn,
}

/// An open item.
#[derive(Debug, Serialize, JsonSchema)]
pub struct OpenItemOut {
    /// Its id.
    pub id: u64,
    /// What is open.
    pub title: String,
    /// More.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail_md: Option<String>,
    /// When it is due.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    /// Who raised it, and when.
    pub raised: SourceOut,
    /// How it was resolved, if it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    /// Who resolved it, and when.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<SourceOut>,
}

/// Parameters for `books.open_items`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct OpenItemsParams {
    /// Include resolved items.
    #[serde(default)]
    pub include_resolved: bool,
}

/// Result of `books.open_items`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct OpenItemsOut {
    /// The items, oldest first.
    pub items: Vec<OpenItemOut>,
}

/// An id a change made.
#[derive(Debug, Serialize, JsonSchema)]
pub struct IdOut {
    /// The id.
    pub id: u64,
}

/// A convention's id, as set.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ConventionSetOut {
    /// The convention as recorded.
    pub convention: ConventionOut,
}

/// Plain success.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DoneOut {
    /// Always true.
    pub ok: bool,
}

/// Parameters for `books.draft_from_template`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DraftFromTemplateParams {
    /// The posting template's id (books.conventions kind posting_template).
    pub template_id: String,
    /// The entry's date, YYYY-MM-DD.
    pub date: String,
    /// The amount, a decimal string ("123.45"): each side's lines split
    /// it by the template's shares, to the cent.
    pub amount: String,
    /// The account for each of the template's slots (a line whose account
    /// is a name in braces, like {expense}), by name: {"expense": "6300"}.
    #[serde(default)]
    pub accounts: std::collections::BTreeMap<String, String>,
    /// The entry's description. Default: the template's.
    #[serde(default)]
    pub description: Option<String>,
    /// The counterparty (id or name).
    #[serde(default)]
    pub party: Option<String>,
    /// Instead of a party: why the entry has none.
    #[serde(default)]
    pub no_party_reason: Option<String>,
}

/// A drafted line.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DraftLineOut {
    /// The account's code.
    pub account: String,
    /// debit or credit.
    pub side: String,
    /// The amount.
    pub amount: String,
}

/// Result of `books.draft_from_template`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DraftFromTemplateOut {
    /// The draft's id: preview it with books.post_entry.
    pub entry_id: u64,
    /// Its description.
    pub description: String,
    /// Its lines.
    pub lines: Vec<DraftLineOut>,
}

/// Parameters for `books.approve_entry`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ApproveEntryParams {
    /// The draft entry.
    pub entry_id: u64,
    /// Who approved it (the approver an approval rule names).
    pub approver: String,
    /// What they said, if anything.
    #[serde(default)]
    pub note: Option<String>,
    /// When they approved, YYYY-MM-DD. Default: today.
    #[serde(default)]
    pub approved_on: Option<String>,
}

// ----------------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------------

fn bad(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn shown(key: &str, v: Option<&Value>) -> Option<String> {
    let k = core_settings::key(key)?;
    v.map(|v| core_settings::display(k, v, false))
}

fn convention_out(c: &ledger_core::conventions::Convention) -> ConventionOut {
    ConventionOut {
        id: c.id.clone(),
        kind: c.kind.key().to_string(),
        title: c.title.clone(),
        body_md: c.body_md.clone(),
        structured: c.structured.clone(),
        revision: c.revision,
        source: source_out(&c.source),
        seq: c.seq,
        retired: c.retired.as_ref().map(|r| r.text.clone()),
    }
}

pub(super) fn decision_out(ledger: &Ledger, d: &ledger_core::conventions::Decision) -> DecisionOut {
    DecisionOut {
        id: d.id,
        subject: d.new.subject.clone(),
        decision_md: d.new.decision_md.clone(),
        decided_by: d.new.decided_by.clone(),
        decided_on: d.new.decided_on.to_string(),
        evidence_attachment: d.new.evidence_attachment,
        supersedes: d.new.supersedes,
        superseded_by: d.superseded_by,
        citation: d.new.citation.clone(),
        projects: d
            .new
            .projects
            .iter()
            .map(|&p| super::projects::project_ref(ledger, p))
            .collect(),
        seq: d.seq,
    }
}

fn open_item_out(i: &ledger_core::conventions::OpenItem) -> OpenItemOut {
    OpenItemOut {
        id: i.id,
        title: i.new.title.clone(),
        detail_md: i.new.detail_md.clone(),
        due: i.new.due.map(|d| d.to_string()),
        raised: source_out(&i.new.source),
        resolution: i.resolved.as_ref().map(|r| r.text.clone()),
        resolved: i.resolved.as_ref().map(|r| source_out(&r.source)),
    }
}

impl BooksServer {
    /// The books' `books.start` (INVARIANT 4).
    fn start_out(&self, store: &Store, focus: Option<&str>) -> StartOut {
        let cfg = self.settings();
        let ledger = store.ledger();
        let kept = store.ledger().settings().clone();
        let books = &kept;
        let today = today();
        let (fy_start, fy_end) = cfg.company.fiscal_year_containing(today);
        let (seq, hash) = store.head();
        let onboarding = ledger.chart().all().next().is_none();
        let mut settings_summary: Vec<SettingBriefOut> = books
            .iter()
            .map(|(k, r)| SettingBriefOut {
                key: k.name.to_string(),
                value: core_settings::display(k, &r.value, false),
                set_by: r.source.decided_by.clone(),
                seq: r.seq,
            })
            .collect();
        let mut conventions: Vec<ConventionBriefOut> = ledger
            .conventions()
            .conventions()
            .filter(|c| c.retired.is_none())
            .map(|c| ConventionBriefOut {
                id: c.id.clone(),
                kind: c.kind.key().to_string(),
                title: c.title.clone(),
                summary: c.summary(),
            })
            .collect();
        conventions.sort_by(|a, b| (&a.kind, &a.id).cmp(&(&b.kind, &b.id)));
        let mut open_items: Vec<OpenItemBriefOut> = ledger
            .conventions()
            .open_items()
            .filter(|i| i.resolved.is_none())
            .map(|i| OpenItemBriefOut {
                id: i.id,
                title: i.new.title.clone(),
                due: i.new.due.map(|d| d.to_string()),
            })
            .collect();
        // The projects under way, by name (rules.md B-37): at most 20.
        const START_PROJECTS: usize = 20;
        let mut projects: Vec<ProjectBriefOut> = ledger
            .list_classes()
            .into_iter()
            .filter_map(|c| {
                let f = ledger.project_facts(c.id)?;
                matches!(
                    f.status,
                    ledger_core::ProjectStatus::Active | ledger_core::ProjectStatus::Paused
                )
                .then(|| ProjectBriefOut {
                    id: c.id,
                    name: c.name.clone(),
                    status: f.status.key().to_string(),
                    kind: f.kind.map(|k| k.key().to_string()),
                })
            })
            .collect();
        projects.sort_by_key(|a| (a.name.to_lowercase(), a.id));
        let mut p_cut = projects.len().saturating_sub(START_PROJECTS);
        projects.truncate(START_PROJECTS);
        let focus = focus.unwrap_or("daily");
        let step = |tool: &str, args: Option<serde_json::Value>, why: &str| NextStepOut {
            tool: tool.into(),
            args,
            why: why.into(),
        };
        // An attention title can carry a free-text description; keep the
        // step short so books.start stays within START_MAX (INVARIANT 4).
        let step_title = |t: &str| -> String {
            const MAX: usize = 160;
            if t.chars().count() <= MAX {
                t.to_string()
            } else {
                let mut s: String = t.chars().take(MAX - 1).collect();
                s.push('…');
                s
            }
        };
        let mut next_steps = Vec::new();
        // Books an older version wrote wait for the owner's go-ahead to
        // upgrade their format: first, at every start until it is given
        // (D-39). Only the owner can give it.
        let upgrade = store.journal_format().upgrade();
        if upgrade.is_some() {
            next_steps.push(step(
                "books.handbook",
                Some(serde_json::json!({"section": "format-upgrade"})),
                "Tell the human first: these books wait for a format upgrade, which only the owner can agree to (in the app, which asks when it opens the books, or `wanderbooks books upgrade` at a terminal, with the owner passphrase; a verified backup is made first). Until then they work on their old format and refuse what needs the new one (format_upgrade_pending). No tool can upgrade them.",
            ));
        }
        if onboarding || focus == "onboarding" {
            next_steps.push(step(
                "books.handbook",
                Some(serde_json::json!({"section": "contract"})),
                "New books: onboard first. Ask the human for the company's facts (books.set_setting), set up the chart (books.chart_template), and record opening balances.",
            ));
        }
        let due_soon = today + chrono::Days::new(30);
        if open_items.iter().any(|i| {
            i.due
                .as_deref()
                .and_then(|d| parse_date(d).ok())
                .is_some_and(|d| d <= due_soon)
        }) {
            next_steps.push(step(
                "books.open_items",
                None,
                "An open item is due within 30 days, or overdue: raise it with the human.",
            ));
        }
        // The books' backups (backup.rs): none set up, or one behind or
        // failing.
        {
            let st = self.backups.standing(store);
            if st.note.is_some() {
                next_steps.push(step(
                    "books.backup",
                    None,
                    "No backups are set up: ask the human for a private repository and a folder on another disk, then set backup.git_remote and backup.folder (books.set_setting).",
                ));
            } else if st.needs_attention() {
                next_steps.push(step(
                    "books.backup",
                    None,
                    "A backup is behind or failing: books.backup says which and why; tell the human.",
                ));
            }
        }
        // The attention items, as next steps: the same function the viewer
        // draws (ledger_package::attention), so the agent and the human see
        // one list. The most urgent first (the list is sorted by severity),
        // at most 12: `books.start` stays small (INVARIANT 4).
        {
            let attention_settings = ledger_package::Settings {
                company: &cfg.company,
                cash_accounts: &cfg.roles.cash,
                financing_accounts: &cfg.roles.financing,
                investing_accounts: &cfg.roles.investing,
            };
            let attention = ledger_package::find(store, today, &attention_settings);
            for a in attention.iter().filter(|a| a.tool.is_some()).take(12) {
                let Some((tool, args)) = &a.tool else {
                    continue;
                };
                let args = (!args.is_null()).then(|| args.clone());
                next_steps.push(step(tool, args, &step_title(&a.title)));
            }
        }
        match focus {
            "close" => next_steps.push(step(
                "books.conventions",
                Some(serde_json::json!({"kind": "close_step"})),
                "Closing a month: follow this company's close steps, in order.",
            )),
            "audit" => next_steps.push(step(
                "books.verify",
                None,
                "An audit starts from books that verify: the journal's chain, the replay, the database and every stored file.",
            )),
            _ => {}
        }
        next_steps.push(step(
            "books.handbook",
            Some(serde_json::json!({"section": "contract"})),
            "The rules: read the contract, then each section a task needs.",
        ));
        next_steps.push(step(
            "books.digest",
            None,
            "What changed lately: offer it to the human.",
        ));
        let mut out = StartOut {
            book: StartBookOut {
                // A settings file's name is unbounded; the books' at most
                // 200 characters (INVARIANT 4).
                name: cfg
                    .company
                    .name
                    .chars()
                    .take(core_settings::MAX_TEXT)
                    .collect(),
                timezone: cfg.timezone.clone(),
                today: today.to_string(),
                fiscal_year: FiscalYearOut {
                    start: fy_start.to_string(),
                    end: fy_end.to_string(),
                },
                base_currency: self.currency().to_string(),
                basis: cfg.company.basis.clone(),
            },
            head: HeadOut { seq, hash },
            state: if onboarding {
                "needs_onboarding"
            } else {
                "active"
            }
            .into(),
            rules: RulesOut {
                evidence: cfg.policy.rule(self.currency()),
                parties: cfg.parties.key().into(),
                // What the server enforces: the books' own setting.
                approval_rules: match ledger.settings().approval_rules() {
                    core_settings::ApprovalMode::Off => "off",
                    core_settings::ApprovalMode::Enforce => "enforce",
                }
                .into(),
            },
            settings_summary: Vec::new(),
            conventions: Vec::new(),
            open_items: Vec::new(),
            projects: Vec::new(),
            truncated: Vec::new(),
            handbook_sections: handbook::SECTIONS
                .iter()
                .map(|(id, md)| SectionOut {
                    id: id.to_string(),
                    title: handbook::title(md).to_string(),
                })
                .collect(),
            format_upgrade: upgrade.map(|u| FormatUpgradeOut {
                guard: ledger_store::error::FORMAT_UPGRADE_PENDING.to_string(),
                format: u.from.format,
                adds: u.adds(),
                how: "only the owner, with the owner passphrase: in the app (it asks when it opens the books) or `wanderbooks books upgrade ID` at a terminal; a verified backup with the keys is made first. No tool can.".to_string(),
            }),
            next_steps,
            isolation: "same_user (advisory)".to_string(),
        };
        // Fit (INVARIANT 4): whole lists if they fit; else summaries go,
        // then the lists shorten from the end, each saying how many more.
        let size = |o: &StartOut| serde_json::to_string(o).map_or(usize::MAX, |s| s.len());
        out.settings_summary = std::mem::take(&mut settings_summary);
        out.conventions = std::mem::take(&mut conventions);
        out.open_items = std::mem::take(&mut open_items);
        out.projects = std::mem::take(&mut projects);
        let cut_note = |c: usize, i: usize, s: usize, p: usize| -> Vec<String> {
            [
                ("conventions", c, "books.conventions"),
                ("open_items", i, "books.open_items"),
                ("settings_summary", s, "books.settings"),
                ("projects", p, "books.projects"),
            ]
            .into_iter()
            .filter(|(_, n, _)| *n > 0)
            .map(|(what, n, tool)| format!("{what}: {n} more ({tool})"))
            .collect()
        };
        out.truncated = cut_note(0, 0, 0, p_cut);
        if size(&out) > START_MAX {
            for c in &mut out.conventions {
                c.summary.clear();
            }
        }
        let (mut c_cut, mut i_cut, mut s_cut) = (0usize, 0usize, 0usize);
        while size(&out) > START_MAX {
            if out.conventions.len() > 10 {
                out.conventions.pop();
                c_cut += 1;
            } else if out.open_items.len() > 10 {
                out.open_items.pop();
                i_cut += 1;
            } else if out.projects.len() > 5 {
                out.projects.pop();
                p_cut += 1;
            } else if !out.settings_summary.is_empty() {
                out.settings_summary.pop();
                s_cut += 1;
            } else if !out.conventions.is_empty() {
                out.conventions.pop();
                c_cut += 1;
            } else if !out.open_items.is_empty() {
                out.open_items.pop();
                i_cut += 1;
            } else if !out.projects.is_empty() {
                out.projects.pop();
                p_cut += 1;
            } else {
                break;
            }
            out.truncated = cut_note(c_cut, i_cut, s_cut, p_cut);
        }
        out
    }

    /// The settings in force after a settings change: take the books'.
    fn sync_settings(&self, store: &Store) {
        self.live().sync(store.ledger().settings(), self.currency());
    }
}

#[tool_router(router = kept_router, vis = "pub(super)")]
impl BooksServer {
    #[tool(
        name = "books.start",
        title = "Start here",
        description = "Call first in every session. The books in brief: company, today in its time zone, fiscal year, the journal's head, whether they need onboarding, where settings come from, the rules in force (receipts, parties, approvals), every setting with who set it, this company's conventions (how it keeps its books: posting templates, policies, close steps), open items, the projects under way, the handbook's sections, a format upgrade the books wait for (only the owner can agree to it: tell them), and next steps. Read-only; at most 8 KB.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub(super) async fn start(
        &self,
        params: Parameters<StartParams>,
    ) -> Result<Json<StartOut>, String> {
        if let Some(f) = &params.0.focus
            && !["daily", "onboarding", "close", "audit"].contains(&f.as_str())
        {
            return Err(format!("focus {f:?}: daily, onboarding, close, or audit"));
        }
        let store = self.lock()?;
        Ok(Json(self.start_out(&store, params.0.focus.as_deref())))
    }

    #[tool(
        name = "books.set_setting",
        title = "Change settings",
        description = "Change one or more settings, together or not at all, recording who decided and when. Each change is {key, value}: value as the key takes it (books.settings lists every key and what it takes), or null to clear it back to its default. Values are checked and kept in one spelling; a change to the value already set changes nothing.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn set_setting(
        &self,
        params: Parameters<SetSettingParams>,
    ) -> Result<Json<SetSettingOut>, String> {
        let p = params.0;
        let source = p.source.source()?;
        if p.changes.is_empty() {
            return Err("changes is empty: pass at least one {key, value}".into());
        }
        let mut store = self.lock()?;
        let before = store.ledger().settings().clone();
        let changes: Vec<Change> = p
            .changes
            .into_iter()
            .map(|c| Change {
                key: c.key,
                value: c.value.filter(|v| !v.is_null()),
            })
            .collect();
        let made = store.change_settings(&changes, &source).map_err(bad)?;
        self.sync_settings(&store);
        Ok(Json(SetSettingOut {
            changed: made
                .iter()
                .map(|c| ChangeOut {
                    key: c.key.clone(),
                    was: shown(&c.key, before.get(&c.key).map(|r| &r.value)),
                    value: shown(&c.key, c.value.as_ref()),
                })
                .collect(),
            problems: settings_problems(store.ledger(), &self.settings()),
        }))
    }

    #[tool(
        name = "books.export_settings",
        title = "Settings as a file",
        description = "The books' settings as settings-file text (TOML), each with who decided it and when as a comment, identity values masked. A copy for reading: editing it changes nothing here.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn export_settings(&self) -> Result<Json<ExportSettingsOut>, String> {
        let store = self.lock()?;
        // The books' own: the defaults until the first is set.
        let s = store.ledger().settings();
        Ok(Json(ExportSettingsOut {
            toml_text: ledger_config::codec::export(s, self.currency()),
        }))
    }

    #[tool(
        name = "books.conventions",
        title = "Conventions",
        description = "How this company keeps its books, as the owners decided: posting templates (books.draft_from_template drafts from one), approval rules, policies, close steps, calendar items, mappings and notes, each with its full text, structure, revision, and who decided it. Filter by kind; retired ones on request.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub(super) async fn conventions(
        &self,
        params: Parameters<ConventionsParams>,
    ) -> Result<Json<ConventionsOut>, String> {
        let p = params.0;
        let kind = p
            .kind
            .as_deref()
            .map(|k| {
                ConventionKind::parse(k).ok_or_else(|| {
                    format!("kind {k:?}: posting_template, approval_rule, policy, close_step, calendar, mapping, or note")
                })
            })
            .transpose()?;
        let store = self.lock()?;
        Ok(Json(ConventionsOut {
            conventions: store
                .ledger()
                .conventions()
                .conventions()
                .filter(|c| p.include_retired || c.retired.is_none())
                .filter(|c| kind.is_none_or(|k| c.kind == k))
                .map(convention_out)
                .collect(),
        }))
    }

    #[tool(
        name = "books.set_convention",
        title = "Set a convention",
        description = "Record how this company keeps its books, as the human decided: a posting template (structured {description?, lines: [{account, side, share}]}, each side's shares summing to 1), an approval rule (structured {approver, accounts?, amount_over?}), or a policy, close step, calendar item, mapping or note (words only). An id already set is revised (same kind); retire one with books.retire_convention.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn set_convention(
        &self,
        params: Parameters<SetConventionParams>,
    ) -> Result<Json<ConventionSetOut>, String> {
        let p = params.0;
        let kind = ConventionKind::parse(&p.kind).ok_or_else(|| {
            format!(
                "kind {:?}: posting_template, approval_rule, policy, close_step, calendar, mapping, or note",
                p.kind
            )
        })?;
        let c = NewConvention {
            id: p.id,
            kind,
            title: p.title,
            body_md: p.body_md,
            structured: p.structured.filter(|v| !v.is_null()),
            source: p.source.source()?,
        };
        let mut store = self.lock()?;
        let c = store.set_convention(&c).map_err(bad)?;
        let set = store
            .ledger()
            .conventions()
            .convention(&c.id)
            .map(convention_out)
            .ok_or("the convention was recorded but can't be read back")?;
        Ok(Json(ConventionSetOut { convention: set }))
    }

    #[tool(
        name = "books.retire_convention",
        title = "Retire a convention",
        description = "Retire a convention that no longer applies, with why and who decided. It stays in the record (books.conventions include_retired); its id is never reused.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn retire_convention(
        &self,
        params: Parameters<RetireConventionParams>,
    ) -> Result<Json<DoneOut>, String> {
        let p = params.0;
        let source = p.source.source()?;
        let mut store = self.lock()?;
        store
            .retire_convention(&p.id, &p.reason, &source)
            .map_err(bad)?;
        Ok(Json(DoneOut { ok: true }))
    }

    #[tool(
        name = "books.record_decision",
        title = "Record a decision",
        description = "Record a decision the owners made about the books (a policy, a treatment, a cutover), with who decided, when, and the document behind it if one is filed. A decision that replaces an earlier one names it in supersedes. No signatures or memos needed: the record is the decision with its source. projects names the projects it is about.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn record_decision(
        &self,
        params: Parameters<RecordDecisionParams>,
    ) -> Result<Json<IdOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        // The projects resolved before anything is written.
        let projects = p
            .projects
            .iter()
            .map(|r| resolve_class(&store, r.trim()))
            .collect::<Result<Vec<_>, _>>()?;
        let d = NewDecision {
            subject: p.subject,
            decision_md: p.decision_md,
            decided_by: p.decided_by,
            decided_on: parse_date(&p.decided_on)?,
            evidence_attachment: p.evidence_attachment,
            supersedes: p.supersedes,
            citation: p.citation,
            projects,
        };
        let id = store.record_decision(&d).map_err(bad)?;
        Ok(Json(IdOut { id }))
    }

    #[tool(
        name = "books.decisions",
        title = "Decisions",
        description = "The owners' decisions about the books, oldest first, each with who decided, when, what it rests on, the projects it is about, and what replaced it. project filters to the decisions naming one.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn decisions(
        &self,
        params: Parameters<DecisionsParams>,
    ) -> Result<Json<DecisionsOut>, String> {
        let store = self.lock()?;
        let project = params
            .0
            .project
            .as_deref()
            .map(|r| resolve_class(&store, r.trim()))
            .transpose()?;
        let ledger = store.ledger();
        Ok(Json(DecisionsOut {
            decisions: ledger
                .conventions()
                .decisions()
                .filter(|d| params.0.include_superseded || d.superseded_by.is_none())
                .filter(|d| project.is_none_or(|p| d.new.projects.contains(&p)))
                .map(|d| decision_out(ledger, d))
                .collect(),
        }))
    }

    #[tool(
        name = "books.raise_item",
        title = "Raise an open item",
        description = "Record something open: waiting on someone, due later, or to look into (a letter to mail, a reply expected, a question for the CPA), with a due date if it has one. books.start lists open items; resolve each with books.resolve_item.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn raise_item(&self, params: Parameters<RaiseItemParams>) -> Result<Json<IdOut>, String> {
        let p = params.0;
        let item = NewOpenItem {
            title: p.title,
            detail_md: p.detail_md,
            due: p.due.as_deref().map(parse_date).transpose()?,
            source: p.source.source()?,
        };
        let mut store = self.lock()?;
        let id = store.raise_open_item(&item).map_err(bad)?;
        Ok(Json(IdOut { id }))
    }

    #[tool(
        name = "books.resolve_item",
        title = "Resolve an open item",
        description = "Resolve an open item, saying how and who decided. It stays in the record (books.open_items include_resolved).",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn resolve_item(
        &self,
        params: Parameters<ResolveItemParams>,
    ) -> Result<Json<DoneOut>, String> {
        let p = params.0;
        let source = p.source.source()?;
        let mut store = self.lock()?;
        store
            .resolve_open_item(p.id, &p.resolution, &source)
            .map_err(bad)?;
        Ok(Json(DoneOut { ok: true }))
    }

    #[tool(
        name = "books.open_items",
        title = "Open items",
        description = "What is open on the books, oldest first, with due dates and who raised each; resolved ones on request.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn open_items(
        &self,
        params: Parameters<OpenItemsParams>,
    ) -> Result<Json<OpenItemsOut>, String> {
        let store = self.lock()?;
        Ok(Json(OpenItemsOut {
            items: store
                .ledger()
                .conventions()
                .open_items()
                .filter(|i| params.0.include_resolved || i.resolved.is_none())
                .map(open_item_out)
                .collect(),
        }))
    }

    #[tool(
        name = "books.draft_from_template",
        title = "Draft from a template",
        description = "Draft an entry from one of this company's posting templates (books.conventions kind posting_template): the amount split across the template's lines by their shares, to the cent, each side summing to it exactly; a line whose account is a slot ({expense}) takes the account named in accounts. Then preview and post as usual (books.post_entry), with the receipt attached.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn draft_from_template(
        &self,
        params: Parameters<DraftFromTemplateParams>,
    ) -> Result<Json<DraftFromTemplateOut>, String> {
        let p = params.0;
        let date = parse_date(&p.date)?;
        let amount = parse_amount(&p.amount)?;
        let no_party = p
            .no_party_reason
            .as_deref()
            .map(str::trim)
            .filter(|r| !r.is_empty());
        if no_party.is_some() && p.party.is_some() {
            return Err("pass party or no_party_reason, not both".into());
        }
        let mut store = self.lock()?;
        let party = p
            .party
            .as_deref()
            .map(|r| resolve_party(&store, r))
            .transpose()?;
        // Every check before anything is journaled.
        store
            .ledger()
            .template_lines(&p.template_id, amount, &p.accounts)
            .map_err(bad)?;
        let id = store
            .draft_from_template(
                &p.template_id,
                date,
                amount,
                p.description.as_deref(),
                &p.accounts,
            )
            .map_err(bad)?;
        if party.is_some() {
            store.set_entry_party(id, party).map_err(bad)?;
        } else if let Some(r) = no_party {
            store.set_entry_no_party(id, r).map_err(bad)?;
        }
        let e = store
            .ledger()
            .get_entry(id)
            .ok_or("the draft was made but can't be read back")?;
        let chart = store.ledger().chart();
        Ok(Json(DraftFromTemplateOut {
            entry_id: id,
            description: e.description().to_string(),
            lines: e
                .lines()
                .iter()
                .map(|l| DraftLineOut {
                    account: chart
                        .get(l.account)
                        .map(|a| a.code.clone())
                        .unwrap_or_default(),
                    side: match l.side {
                        Side::Debit => "debit",
                        Side::Credit => "credit",
                    }
                    .into(),
                    amount: money(l.amount.amount),
                })
                .collect(),
        }))
    }

    #[tool(
        name = "books.approve_entry",
        title = "Record an approval",
        description = "Record that the approver an approval rule names approved a draft entry, as it stands now (a line added afterwards needs a new approval). Only when the approver said so. Matters only when the books enforce approval rules (policy.approval_rules); a post's preview lists approvals still needed.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn approve_entry(
        &self,
        params: Parameters<ApproveEntryParams>,
    ) -> Result<Json<DoneOut>, String> {
        let p = params.0;
        let source = Source {
            decided_by: p.approver.clone(),
            decided_on: match &p.approved_on {
                Some(d) => parse_date(d)?,
                None => today(),
            },
            citation: None,
            note: None,
        };
        let mut store = self.lock()?;
        store
            .approve_entry(p.entry_id, &p.approver, p.note.as_deref(), &source)
            .map_err(bad)?;
        Ok(Json(DoneOut { ok: true }))
    }
}

/// Every setting the books know, for `books.settings`: what it takes,
/// its value and who set it.
pub(super) fn keys_out(books: &core_settings::Settings) -> Vec<SettingKeyOut> {
    KEYS.iter()
        .map(|k| {
            let r = books.get(k.name);
            SettingKeyOut {
                key: k.name.to_string(),
                doc: k.doc.to_string(),
                identity: k.sensitivity == Sensitivity::Identity,
                value: r.map(|r| core_settings::display(k, &r.value, false)),
                set_by: r.map(|r| source_out(&r.source)),
                seq: r.map(|r| r.seq),
            }
        })
        .collect()
}

/// A setting the books know.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SettingKeyOut {
    /// Its key, for books.set_setting.
    pub key: String,
    /// What it means and takes.
    pub doc: String,
    /// Whether it is an identity number (shown masked).
    pub identity: bool,
    /// Its value as text, or null: unset (its default applies).
    pub value: Option<String>,
    /// Who set it, and when.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set_by: Option<SourceOut>,
    /// The journal line that set it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
}
