//! A company's conventions, decisions and open items, journaled in the
//! books (plan/served.md 5.2: `convention.*`, `decision.recorded`,
//! `open_item.*`, and `entry.approved` for approval rules).
//!
//! Conventions are how this company keeps its books: posting templates
//! ("an expense a member paid personally: Dr expense / Cr Due to
//! Members"), policies, close steps, calendar items, notes. Decisions are
//! the choices the owners made, with who made them and when. Open items
//! are what is waiting on someone. All are company data that any agent
//! reads through the server, not prose in a file beside the books.
//!
//! ```text
//! GOAL: keep the company's way of working, its decisions and its loose
//!       ends in the books, sourced, so every agent everywhere sees them.
//! SOURCE: plan/served.md I-3, 4.2 (books.start lists conventions), 5.2;
//!         DECISION (owner, 2026-10-16): light governance, approval
//!         rules exist for companies that want them and are off by
//!         default.
//! INVARIANTS:
//!   1. A convention's id is a slug ([a-z0-9-], 1 to 64, starting with a
//!      letter or digit), never reused: setting a live id revises it (its
//!      kind can't change), a retired id is refused.
//!   2. A posting template has a debit and a credit line, each side's
//!      shares summing to exactly 1, each on an account in the chart that
//!      takes postings or on a slot ("{expense}") that names one when a
//!      draft is made from it; drafting from it makes lines on accounts in
//!      the chart that take postings, summing on each side to the amount
//!      exactly, each a whole number of minor units, none zero.
//!   3. An approval rule names its approver and limits itself by accounts
//!      that take postings, an amount, or both; under
//!      `policy.approval_rules = enforce` an entry a rule covers posts only
//!      once that approver approved it as it stands (its fingerprint: a
//!      line added, a class or a party changed after an approval voids
//!      it), and an
//!      entry the server would post at once (an invoice, a bill, a
//!      payment, payroll, a remittance, a disposal) that a rule covers is
//!      refused: draft it instead. Reversals, closes, depreciation and
//!      amortization follow from entries already posted and are exempt.
//!      Under `off` (the default) rules never block anything.
//!   4. Decision and open-item ids are allocated in order and journaled;
//!      a decision supersedes at most one live decision; an item resolves
//!      once.
//!   5. Replay takes what was journaled (rules.md B-28): text limits and
//!      chart checks are for new commands only.
//!   6. No text holds a full TIN (settings INVARIANT 4).
//! ```

use crate::settings::{MAX_LONG_TEXT, MAX_TEXT, Source, line_text, long_text};
use crate::{EntryId, Side};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Longest convention or decision body.
pub const MAX_BODY: usize = 20_000;

/// What a convention is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConventionKind {
    /// How a kind of transaction is posted; drafts come from it
    /// (`books.draft_from_template`).
    PostingTemplate,
    /// Who must approve which entries (enforced only under
    /// `policy.approval_rules = enforce`).
    ApprovalRule,
    /// A rule of practice (numbering, file formats, capitalization).
    Policy,
    /// A step of the month-end close.
    CloseStep,
    /// Something due on a date every year (annual report, returns).
    Calendar,
    /// A mapping between these books and another system's names.
    Mapping,
    /// Anything else worth keeping.
    Note,
}

impl ConventionKind {
    /// Every kind, in order.
    pub const ALL: [ConventionKind; 7] = [
        ConventionKind::PostingTemplate,
        ConventionKind::ApprovalRule,
        ConventionKind::Policy,
        ConventionKind::CloseStep,
        ConventionKind::Calendar,
        ConventionKind::Mapping,
        ConventionKind::Note,
    ];

    /// Its wire name.
    pub fn key(self) -> &'static str {
        match self {
            ConventionKind::PostingTemplate => "posting_template",
            ConventionKind::ApprovalRule => "approval_rule",
            ConventionKind::Policy => "policy",
            ConventionKind::CloseStep => "close_step",
            ConventionKind::Calendar => "calendar",
            ConventionKind::Mapping => "mapping",
            ConventionKind::Note => "note",
        }
    }

    /// From its wire name.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.key() == s)
    }
}

/// One line of a posting template.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateLine {
    /// The account's code, or a slot, `{name}` (lowercase letters,
    /// digits and `_`), that a draft names: `{expense}`.
    pub account: String,
    /// Debit or credit (`"debit"`, `"credit"`).
    #[serde(with = "side_word")]
    pub side: Side,
    /// Its share of the amount, as a decimal string ("1", "0.5").
    pub share: String,
}

/// A side as a word, `"debit"` or `"credit"`.
mod side_word {
    use crate::Side;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(side: &Side, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(match side {
            Side::Debit => "debit",
            Side::Credit => "credit",
        })
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Side, D::Error> {
        match String::deserialize(d)?.to_lowercase().as_str() {
            "debit" => Ok(Side::Debit),
            "credit" => Ok(Side::Credit),
            other => Err(serde::de::Error::custom(format!(
                "side {other:?}: debit or credit"
            ))),
        }
    }
}

/// A posting template's structure.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Template {
    /// The entry description drafts take unless given one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The lines.
    pub lines: Vec<TemplateLine>,
}

/// An approval rule's structure.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalRule {
    /// Who approves: a name.
    pub approver: String,
    /// The rule covers entries with a line in one of these accounts
    /// (codes); empty: any account.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accounts: Vec<String>,
    /// The rule covers entries whose debits are strictly greater than
    /// this; unset: any amount.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount_over: Option<String>,
}

/// A convention being set: what `books.set_convention` takes.
#[derive(Clone, Debug, PartialEq)]
pub struct NewConvention {
    /// Its id (a slug).
    pub id: String,
    /// What it is.
    pub kind: ConventionKind,
    /// A short title.
    pub title: String,
    /// The convention in words (Markdown).
    pub body_md: String,
    /// Its structure: a [`Template`] for a posting template, an
    /// [`ApprovalRule`] for an approval rule; none for other kinds.
    pub structured: Option<Value>,
    /// Who decided it.
    pub source: Source,
}

/// Why and when something was retired or resolved.
#[derive(Clone, Debug, PartialEq)]
pub struct Closing {
    /// Why (a convention's retirement) or how (an item's resolution).
    pub text: String,
    /// Who decided.
    pub source: Source,
    /// The journal line.
    pub seq: u64,
}

/// A convention as the journal holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct Convention {
    /// Its id.
    pub id: String,
    /// What it is.
    pub kind: ConventionKind,
    /// A short title.
    pub title: String,
    /// The convention in words.
    pub body_md: String,
    /// Its structure, if any.
    pub structured: Option<Value>,
    /// Who decided this revision.
    pub source: Source,
    /// The journal line of this revision.
    pub seq: u64,
    /// 1 for the first version, then 2, 3, ...
    pub revision: u32,
    /// Set once retired.
    pub retired: Option<Closing>,
}

impl Convention {
    /// Its template, when it is a posting template.
    pub fn template(&self) -> Option<Template> {
        (self.kind == ConventionKind::PostingTemplate)
            .then(|| serde_json::from_value(self.structured.clone()?).ok())
            .flatten()
    }

    /// Its rule, when it is an approval rule.
    pub fn approval_rule(&self) -> Option<ApprovalRule> {
        (self.kind == ConventionKind::ApprovalRule)
            .then(|| serde_json::from_value(self.structured.clone()?).ok())
            .flatten()
    }

    /// The first line of its body, at most 160 characters: a summary.
    pub fn summary(&self) -> String {
        let first = self
            .body_md
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("");
        let s: String = first.trim().chars().take(160).collect();
        if first.trim().chars().count() > 160 {
            format!("{s}…")
        } else {
            s
        }
    }
}

/// A decision being recorded.
#[derive(Clone, Debug, PartialEq)]
pub struct NewDecision {
    /// What it is about ("Materiality policy").
    pub subject: String,
    /// The decision in words.
    pub decision_md: String,
    /// Who decided (names), at least one.
    pub decided_by: Vec<String>,
    /// When.
    pub decided_on: NaiveDate,
    /// The document behind it (an attachment id), if any.
    pub evidence_attachment: Option<u64>,
    /// The decision it replaces, if any.
    pub supersedes: Option<u64>,
    /// The rule or document it rests on, if any.
    pub citation: Option<String>,
}

/// A decision as the journal holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    /// Its id.
    pub id: u64,
    /// What was decided, and by whom.
    pub new: NewDecision,
    /// The journal line.
    pub seq: u64,
    /// The decision that replaced it, if any.
    pub superseded_by: Option<u64>,
}

/// An open item being raised.
#[derive(Clone, Debug, PartialEq)]
pub struct NewOpenItem {
    /// What is open, in a line.
    pub title: String,
    /// More, if any (Markdown).
    pub detail_md: Option<String>,
    /// When it is due, if it is.
    pub due: Option<NaiveDate>,
    /// Who raised it.
    pub source: Source,
}

/// An open item as the journal holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenItem {
    /// Its id.
    pub id: u64,
    /// What it is.
    pub new: NewOpenItem,
    /// The journal line that raised it.
    pub seq: u64,
    /// How it was resolved, once it is.
    pub resolved: Option<Closing>,
}

/// One approval of a draft entry.
#[derive(Clone, Debug, PartialEq)]
pub struct Approval {
    /// Who approved.
    pub approver: String,
    /// The draft as approved ([`fingerprint`]): a line added or a class
    /// changed since voids it (INVARIANT 3).
    pub fingerprint: String,
    /// Anything they said.
    pub note: Option<String>,
    /// Who recorded it, and when.
    pub source: Source,
    /// The journal line.
    pub seq: u64,
}

/// Everything this module keeps.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Conventions {
    conventions: BTreeMap<String, Convention>,
    decisions: BTreeMap<u64, Decision>,
    open_items: BTreeMap<u64, OpenItem>,
    approvals: BTreeMap<EntryId, Vec<Approval>>,
}

/// A template line's slot name: `expense` for `{expense}`; `None` for an
/// account code.
pub fn slot(account: &str) -> Option<&str> {
    let name = account.strip_prefix('{')?.strip_suffix('}')?;
    (!name.is_empty()
        && name.len() <= 32
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'))
    .then_some(name)
}

/// An entry as an approval sees it: a fixed text of its date,
/// description, every line (account, side, amount, currency, foreign
/// amount and currency, class) and who it is with (`parties`: each party
/// and its share; or why it has none), hashed (FNV-1a, 64 bits, hex).
/// Journaled with each approval and recomputed at post, so it must never
/// change meaning. A fingerprint tells a draft changed by mistake; it is
/// no seal against someone forging a collision (whoever records approvals
/// could record a false one anyway).
pub fn fingerprint(
    e: &crate::JournalEntry,
    parties: &[crate::PartyShare],
    no_party: Option<&str>,
) -> String {
    let mut text = format!("{}\u{1f}{}", e.date(), e.description());
    for p in parties {
        let share = match p.share {
            crate::Share::Amount(a) => format!("a{}", a.normalize()),
            crate::Share::Percent(x) => format!("p{}", x.normalize()),
            crate::Share::Derived => "d".to_string(),
        };
        text.push_str(&format!("\u{1d}{}\u{1f}{share}", p.party));
    }
    if let Some(why) = no_party {
        text.push_str(&format!("\u{1d}none\u{1f}{why}"));
    }
    for l in e.lines() {
        text.push_str(&format!(
            "\u{1e}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
            l.account,
            match l.side {
                Side::Debit => "D",
                Side::Credit => "C",
            },
            l.amount.amount.normalize(),
            l.amount.currency.code(),
            l.foreign_amount
                .map(|a| a.normalize().to_string())
                .unwrap_or_default(),
            l.foreign_currency
                .map(|c| c.code().to_string())
                .unwrap_or_default(),
            l.class_id.map(|c| c.to_string()).unwrap_or_default(),
        ));
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Check a convention id (INVARIANT 1).
pub fn check_id(id: &str) -> Result<(), String> {
    if crate::settings::holds_printed_tin(id) {
        return Err("a convention id never holds a number that reads as a full TIN".into());
    }
    let ok = !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-');
    if ok {
        Ok(())
    } else {
        Err(format!(
            "convention id {id:?}: 1 to 64 lowercase letters, digits and hyphens, not starting with a hyphen"
        ))
    }
}

/// Split `amount` by `shares` (which sum to 1), each part a whole number
/// of `places` decimal places (none: exact), by largest remainder
/// ([`crate::kernel::apportion`]): the parts sum to `amount` exactly
/// (INVARIANT 2).
pub fn split(amount: Decimal, shares: &[Decimal], places: Option<u32>) -> Option<Vec<Decimal>> {
    use rust_decimal::RoundingStrategy::ToZero;
    let exact: Vec<Decimal> = shares
        .iter()
        .map(|s| amount.checked_mul(*s))
        .collect::<Option<_>>()?;
    let Some(places) = places else {
        let mut parts = exact;
        crate::kernel::plug_last(amount, &mut parts)?;
        return Some(parts);
    };
    let mut parts: Vec<Decimal> = exact
        .iter()
        .map(|e| e.round_dp_with_strategy(places, ToZero))
        .collect();
    let remainders: Vec<Decimal> = exact
        .iter()
        .zip(&parts)
        .map(|(e, p)| e.checked_sub(*p))
        .collect::<Option<_>>()?;
    crate::kernel::apportion(amount, &mut parts, &remainders, Decimal::new(1, places))?;
    Some(parts)
}

impl Conventions {
    /// Every convention, retired ones too, by id.
    pub fn conventions(&self) -> impl Iterator<Item = &Convention> {
        self.conventions.values()
    }

    /// The convention `id`, retired or not.
    pub fn convention(&self, id: &str) -> Option<&Convention> {
        self.conventions.get(id)
    }

    /// Every decision, by id.
    pub fn decisions(&self) -> impl Iterator<Item = &Decision> {
        self.decisions.values()
    }

    /// The decision `id`.
    pub fn decision(&self, id: u64) -> Option<&Decision> {
        self.decisions.get(&id)
    }

    /// The id the next decision gets.
    pub fn next_decision_id(&self) -> u64 {
        self.decisions.keys().next_back().map_or(1, |k| k + 1)
    }

    /// Every open item, resolved ones too, by id.
    pub fn open_items(&self) -> impl Iterator<Item = &OpenItem> {
        self.open_items.values()
    }

    /// The open item `id`.
    pub fn open_item(&self, id: u64) -> Option<&OpenItem> {
        self.open_items.get(&id)
    }

    /// The id the next open item gets.
    pub fn next_open_item_id(&self) -> u64 {
        self.open_items.keys().next_back().map_or(1, |k| k + 1)
    }

    /// The approvals recorded on `entry`.
    pub fn approvals(&self, entry: EntryId) -> &[Approval] {
        self.approvals.get(&entry).map_or(&[], Vec::as_slice)
    }

    /// The live approval rules (not retired), by id, each with its rule,
    /// or `None` when this version can't read it (a newer version's): an
    /// unreadable rule covers every entry (fail closed).
    pub fn approval_rules(&self) -> impl Iterator<Item = (&Convention, Option<ApprovalRule>)> {
        self.conventions
            .values()
            .filter(|c| c.retired.is_none() && c.kind == ConventionKind::ApprovalRule)
            .map(|c| (c, c.approval_rule()))
    }

    /// Set (or revise) a convention as journaled (INVARIANTS 1, 5).
    pub fn apply_convention(&mut self, c: NewConvention, seq: u64) -> Result<(), String> {
        let revision = match self.conventions.get(&c.id) {
            Some(old) if old.retired.is_some() => {
                return Err(format!(
                    "convention {} is retired; ids are never reused: choose a new one",
                    c.id
                ));
            }
            Some(old) if old.kind != c.kind => {
                return Err(format!(
                    "convention {} is a {}; its kind can't change (retire it and set a new id)",
                    c.id,
                    old.kind.key()
                ));
            }
            Some(old) => old.revision + 1,
            None => 1,
        };
        self.conventions.insert(
            c.id.clone(),
            Convention {
                id: c.id,
                kind: c.kind,
                title: c.title,
                body_md: c.body_md,
                structured: c.structured,
                source: c.source,
                seq,
                revision,
                retired: None,
            },
        );
        Ok(())
    }

    /// Retire a live convention as journaled.
    pub fn apply_retire(
        &mut self,
        id: &str,
        reason: String,
        source: Source,
        seq: u64,
    ) -> Result<(), String> {
        let c = self
            .conventions
            .get_mut(id)
            .ok_or_else(|| format!("unknown convention {id:?}"))?;
        if c.retired.is_some() {
            return Err(format!("convention {id} is already retired"));
        }
        c.retired = Some(Closing {
            text: reason,
            source,
            seq,
        });
        Ok(())
    }

    /// Record a decision as journaled, under `id` (INVARIANT 4).
    pub fn apply_decision(&mut self, id: u64, d: NewDecision, seq: u64) -> Result<(), String> {
        if id != self.next_decision_id() {
            return Err(format!(
                "decision id {id} is not the next ({})",
                self.next_decision_id()
            ));
        }
        if let Some(old) = d.supersedes {
            let o = self
                .decisions
                .get(&old)
                .ok_or_else(|| format!("decision {old} doesn't exist"))?;
            if let Some(by) = o.superseded_by {
                return Err(format!("decision {old} was already superseded by {by}"));
            }
        }
        if let Some(old) = d.supersedes
            && let Some(o) = self.decisions.get_mut(&old)
        {
            o.superseded_by = Some(id);
        }
        self.decisions.insert(
            id,
            Decision {
                id,
                new: d,
                seq,
                superseded_by: None,
            },
        );
        Ok(())
    }

    /// Raise an open item as journaled, under `id`.
    pub fn apply_raise(&mut self, id: u64, item: NewOpenItem, seq: u64) -> Result<(), String> {
        if id != self.next_open_item_id() {
            return Err(format!(
                "open item id {id} is not the next ({})",
                self.next_open_item_id()
            ));
        }
        self.open_items.insert(
            id,
            OpenItem {
                id,
                new: item,
                seq,
                resolved: None,
            },
        );
        Ok(())
    }

    /// Resolve an open item as journaled (INVARIANT 4).
    pub fn apply_resolve(
        &mut self,
        id: u64,
        resolution: String,
        source: Source,
        seq: u64,
    ) -> Result<(), String> {
        let item = self
            .open_items
            .get_mut(&id)
            .ok_or_else(|| format!("unknown open item {id}"))?;
        if item.resolved.is_some() {
            return Err(format!("open item {id} is already resolved"));
        }
        item.resolved = Some(Closing {
            text: resolution,
            source,
            seq,
        });
        Ok(())
    }

    /// Record an approval as journaled.
    pub fn apply_approval(&mut self, entry: EntryId, a: Approval) {
        self.approvals.entry(entry).or_default().push(a);
    }
}

/// Check a new convention's text and structure (INVARIANTS 1, 2, 3, 6);
/// the chart checks are the ledger's ([`crate::Ledger::check_convention`]).
pub fn check_new(c: &NewConvention) -> Result<NewConvention, String> {
    check_id(&c.id)?;
    c.source.check()?;
    let title = line_text("title", &c.title, MAX_TEXT)?;
    let body_md = long_text("body", &c.body_md, MAX_BODY)?;
    let structured = match (c.kind, &c.structured) {
        (ConventionKind::PostingTemplate, Some(v)) => {
            let t: Template =
                serde_json::from_value(v.clone()).map_err(|e| format!("posting template: {e}"))?;
            Some(serde_json::to_value(check_template(&t)?).map_err(|e| e.to_string())?)
        }
        (ConventionKind::PostingTemplate, None) => {
            return Err(
                "a posting template needs structured: {description?, lines: [{account, side, share}]}"
                    .into(),
            );
        }
        (ConventionKind::ApprovalRule, Some(v)) => {
            let r: ApprovalRule =
                serde_json::from_value(v.clone()).map_err(|e| format!("approval rule: {e}"))?;
            Some(serde_json::to_value(check_rule(&r)?).map_err(|e| e.to_string())?)
        }
        (ConventionKind::ApprovalRule, None) => {
            return Err(
                "an approval rule needs structured: {approver, accounts?, amount_over?}".into(),
            );
        }
        (k, Some(_)) => {
            return Err(format!(
                "a {} convention takes no structured part: put it in the body",
                k.key()
            ));
        }
        (_, None) => None,
    };
    Ok(NewConvention {
        id: c.id.clone(),
        kind: c.kind,
        title,
        body_md,
        structured,
        source: c.source.clone(),
    })
}

fn check_template(t: &Template) -> Result<Template, String> {
    let description = t
        .description
        .as_deref()
        .map(|d| line_text("template description", d, MAX_TEXT))
        .transpose()?;
    if t.lines.len() < 2 || t.lines.len() > 50 {
        return Err("a posting template has 2 to 50 lines".into());
    }
    let mut lines = Vec::new();
    let (mut debit, mut credit) = (Decimal::ZERO, Decimal::ZERO);
    for (i, l) in t.lines.iter().enumerate() {
        let n = i + 1;
        let share = crate::settings::decimal(l.share.trim())
            .filter(|s| *s > Decimal::ZERO && *s <= Decimal::ONE)
            .ok_or_else(|| {
                format!(
                    "template line {n}: share {:?} must be a decimal in (0, 1]",
                    l.share
                )
            })?;
        match l.side {
            Side::Debit => debit += share,
            Side::Credit => credit += share,
        }
        let account = line_text(&format!("template line {n} account"), &l.account, 64)?;
        if account.starts_with('{') && slot(&account).is_none() {
            return Err(format!(
                "template line {n}: slot {account:?} is a name in braces, lowercase letters, digits and _: {{expense}}"
            ));
        }
        lines.push(TemplateLine {
            account,
            side: l.side,
            share: share.normalize().to_string(),
        });
    }
    if debit != Decimal::ONE || credit != Decimal::ONE {
        return Err(format!(
            "a posting template's debit shares and credit shares each sum to exactly 1 (debits {debit}, credits {credit})"
        ));
    }
    Ok(Template { description, lines })
}

fn check_rule(r: &ApprovalRule) -> Result<ApprovalRule, String> {
    let approver = line_text("approver", &r.approver, MAX_TEXT)?;
    let mut accounts = Vec::new();
    for a in &r.accounts {
        let a = line_text("approval rule account", a, 64)?;
        if !accounts.contains(&a) {
            accounts.push(a);
        }
    }
    let amount_over = match &r.amount_over {
        Some(s) => {
            let a = crate::settings::decimal(s.trim())
                .filter(|a| !a.is_sign_negative() && *a <= crate::kernel::MAX_LINE_AMOUNT)
                .ok_or_else(|| format!("amount_over {s:?} must be a decimal at least zero"))?;
            Some(a.to_string())
        }
        None => None,
    };
    if accounts.is_empty() && amount_over.is_none() {
        return Err(
            "an approval rule covers some accounts, entries over an amount, or both: give accounts, amount_over, or both"
                .into(),
        );
    }
    Ok(ApprovalRule {
        approver,
        accounts,
        amount_over,
    })
}

/// Check a new decision's text (INVARIANT 6); the ledger checks ids.
pub fn check_decision(d: &NewDecision) -> Result<NewDecision, String> {
    if d.decided_by.is_empty() {
        return Err("decided_by names at least one person".into());
    }
    crate::calendar::check(d.decided_on).map_err(|e| e.to_string())?;
    Ok(NewDecision {
        subject: line_text("subject", &d.subject, MAX_TEXT)?,
        decision_md: long_text("decision", &d.decision_md, MAX_BODY)?,
        decided_by: d
            .decided_by
            .iter()
            .map(|n| line_text("decided_by", n, MAX_TEXT))
            .collect::<Result<_, _>>()?,
        decided_on: d.decided_on,
        evidence_attachment: d.evidence_attachment,
        supersedes: d.supersedes,
        citation: d
            .citation
            .as_deref()
            .map(|c| long_text("citation", c, MAX_LONG_TEXT))
            .transpose()?,
    })
}

/// Check a new open item's text (INVARIANT 6).
pub fn check_open_item(i: &NewOpenItem) -> Result<NewOpenItem, String> {
    i.source.check()?;
    if let Some(d) = i.due {
        crate::calendar::check(d).map_err(|e| e.to_string())?;
    }
    Ok(NewOpenItem {
        title: line_text("title", &i.title, MAX_TEXT)?,
        detail_md: i
            .detail_md
            .as_deref()
            .map(|d| long_text("detail", d, MAX_BODY))
            .transpose()?,
        due: i.due,
        source: i.source.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn src() -> Source {
        Source {
            decided_by: "Owner".into(),
            decided_on: NaiveDate::from_ymd_opt(2026, 10, 17).unwrap(),
            citation: None,
            note: None,
        }
    }

    fn conv(id: &str, kind: ConventionKind, structured: Option<Value>) -> NewConvention {
        NewConvention {
            id: id.into(),
            kind,
            title: "T".into(),
            body_md: "Body".into(),
            structured,
            source: src(),
        }
    }

    /// INVARIANT 1: slugs; revisions keep the kind; retired ids never come
    /// back.
    #[test]
    fn ids_and_revisions() {
        for bad in [
            "",
            "-x",
            "Upper",
            "a b",
            "a_b",
            &"x".repeat(65),
            "123-45-6789",
            "ssn-123456789",
        ] {
            assert!(check_id(bad).is_err(), "{bad}");
        }
        let mut c = Conventions::default();
        c.apply_convention(conv("n", ConventionKind::Note, None), 1)
            .unwrap();
        c.apply_convention(conv("n", ConventionKind::Note, None), 2)
            .unwrap();
        assert_eq!(c.convention("n").unwrap().revision, 2);
        assert!(
            c.apply_convention(conv("n", ConventionKind::Policy, None), 3)
                .is_err()
        );
        c.apply_retire("n", "gone".into(), src(), 4).unwrap();
        assert!(c.apply_retire("n", "again".into(), src(), 5).is_err());
        assert!(
            c.apply_convention(conv("n", ConventionKind::Note, None), 6)
                .is_err()
        );
    }

    /// INVARIANT 2: shares sum to one on each side.
    #[test]
    fn templates_balance() {
        let t = |lines: Value| {
            check_new(&conv(
                "t",
                ConventionKind::PostingTemplate,
                Some(json!({ "lines": lines })),
            ))
        };
        assert!(
            t(json!([
                {"account": "1000", "side": "debit", "share": "1"},
                {"account": "3000", "side": "credit", "share": "0.5"},
                {"account": "3001", "side": "credit", "share": "0.50"}
            ]))
            .is_ok()
        );
        assert!(
            t(json!([
                {"account": "1000", "side": "debit", "share": "1"},
                {"account": "3000", "side": "credit", "share": "0.5"}
            ]))
            .is_err()
        );
        assert!(
            t(json!([
                {"account": "1000", "side": "debit", "share": "1"},
                {"account": "3000", "side": "credit", "share": "1e0"}
            ]))
            .is_err()
        );
        assert!(check_new(&conv("t", ConventionKind::PostingTemplate, None)).is_err());
        assert!(check_new(&conv("n", ConventionKind::Note, Some(json!({})))).is_err());
    }

    #[test]
    fn splits_are_exact() {
        let h = Decimal::new(5, 1);
        assert_eq!(
            split(Decimal::new(1001, 2), &[h, h], Some(2)).unwrap(),
            vec![Decimal::new(501, 2), Decimal::new(500, 2)]
        );
        let third = Decimal::ONE / Decimal::from(3);
        let parts = split(
            Decimal::new(10000, 2),
            &[third, third, Decimal::ONE - third - third],
            Some(2),
        )
        .unwrap();
        assert_eq!(parts.iter().sum::<Decimal>(), Decimal::new(10000, 2));
        assert_eq!(
            split(Decimal::from(7), &[h, h], Some(0))
                .unwrap()
                .iter()
                .sum::<Decimal>(),
            Decimal::from(7)
        );
    }

    #[test]
    fn rules_name_their_scope() {
        assert!(
            check_new(&conv(
                "r",
                ConventionKind::ApprovalRule,
                Some(json!({"approver": "A"}))
            ))
            .is_err()
        );
        assert!(
            check_new(&conv(
                "r",
                ConventionKind::ApprovalRule,
                Some(json!({"approver": "A", "amount_over": "500"}))
            ))
            .is_ok()
        );
        assert!(
            check_new(&conv(
                "r",
                ConventionKind::ApprovalRule,
                Some(json!({"approver": "A", "amount_over": "-1"}))
            ))
            .is_err()
        );
        assert!(
            check_new(&conv(
                "r",
                ConventionKind::ApprovalRule,
                Some(json!({"approver": "A", "accounts": ["6000"], "extra": 1}))
            ))
            .is_err()
        );
    }

    /// INVARIANT 4.
    #[test]
    fn decisions_and_items_in_order() {
        let mut c = Conventions::default();
        let d = NewDecision {
            subject: "S".into(),
            decision_md: "D".into(),
            decided_by: vec!["A".into()],
            decided_on: src().decided_on,
            evidence_attachment: None,
            supersedes: None,
            citation: None,
        };
        assert!(c.apply_decision(2, d.clone(), 1).is_err());
        c.apply_decision(1, d.clone(), 1).unwrap();
        c.apply_decision(
            2,
            NewDecision {
                supersedes: Some(1),
                ..d.clone()
            },
            2,
        )
        .unwrap();
        assert!(
            c.apply_decision(
                3,
                NewDecision {
                    supersedes: Some(1),
                    ..d.clone()
                },
                3
            )
            .is_err()
        );
        assert_eq!(c.decision(1).unwrap().superseded_by, Some(2));
        let i = NewOpenItem {
            title: "T".into(),
            detail_md: None,
            due: None,
            source: src(),
        };
        c.apply_raise(1, i, 4).unwrap();
        c.apply_resolve(1, "done".into(), src(), 5).unwrap();
        assert!(c.apply_resolve(1, "again".into(), src(), 6).is_err());
    }
}
