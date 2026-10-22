//! Projects through the server: `books.add_project`,
//! `books.update_project`, `books.projects` and `books.project` (rules.md
//! B-37). A project is a class with its facts: `books.add_line`'s class
//! tags a line with it, and every class tool keeps working.
//!
//! ```text
//! GOAL: the agent records what each project is and links what belongs to
//!       it; anyone reads what it earned and cost from the same figures
//!       the viewer and bookspec use.
//! SOURCE: docs/spec/rules.md B-37 (DECISION, owner, 2026-10-21; job
//!         costing; ASC 450-30-25-1 for on-spec work), B-30 (omitted
//!         dates), B-27 (journal feature `projects`).
//! INVARIANTS:
//!   1. Every field is checked before anything is written; a refusal
//!      journals nothing.
//!   2. Figures come only from ledger_core::project_figures,
//!      project_to_date and project_entries (never re-summed here).
//!   3. books.update_project with nothing new journals nothing.
//!   4. A project is named by id or name (ignoring capitals); a number
//!      that is one project's id and another's name is refused.
//! ```

use super::kept::DecisionOut;
use super::*;
use ledger_core::{ClassId, ProjectFacts, ProjectKind, ProjectStatus};
use schemars::JsonSchema;
use serde::Serialize;

/// Result of `books.project`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ProjectDetailOut {
    /// Its facts.
    pub project: ProjectOut,
    /// The period asked.
    pub period: ProjectPeriodOut,
    /// Every tagged line through the period's end.
    pub to_date: ToDateOut,
    /// Every entry with a line tagged to it (drafts included), newest first.
    pub entries: Vec<ProjectEntryOut>,
    /// The files linked to it (its contract, spec, pitch), not withdrawn.
    pub documents: Vec<ReceiptOut>,
    /// The decisions naming it, oldest first, superseded ones included.
    pub decisions: Vec<DecisionOut>,
}

/// A status key, or why not.
fn parse_status(s: &str) -> Result<ProjectStatus, String> {
    ProjectStatus::parse(s.trim())
        .ok_or_else(|| format!("status {s:?}: expected active, paused, finished, or abandoned"))
}

/// A kind key, or why not.
fn parse_kind(s: &str) -> Result<ProjectKind, String> {
    ProjectKind::parse(s.trim())
        .ok_or_else(|| format!("kind {s:?}: expected internal, client, on_spec, or other"))
}

/// A project's facts and party for a tool's output.
pub(super) fn project_out(ledger: &Ledger, id: ClassId) -> ProjectOut {
    let name = ledger
        .get_class(id)
        .map(|c| c.name.clone())
        .unwrap_or_default();
    let f = ledger.project_facts(id).unwrap_or_default();
    ProjectOut {
        id,
        name,
        status: f.status.key().to_string(),
        kind: f.kind.map(|k| k.key().to_string()),
        description: f.description,
        party: f.party.map(|p| PartyRefOut {
            party_id: p,
            label: ledger
                .get_party(p)
                .map(|x| x.label().to_string())
                .unwrap_or_default(),
        }),
        started_on: f.started_on.map(|d| d.to_string()),
        ended_on: f.ended_on.map(|d| d.to_string()),
        links: f.links,
        notes: f.notes,
    }
}

/// A project named by id and name.
pub(super) fn project_ref(ledger: &Ledger, id: ClassId) -> ProjectRefOut {
    ProjectRefOut {
        id,
        name: ledger
            .get_class(id)
            .map(|c| c.name.clone())
            .unwrap_or_default(),
    }
}

/// Figures to date for a tool's output.
fn to_date_out(t: &ledger_core::ProjectToDate) -> ToDateOut {
    ToDateOut {
        through: t.through.to_string(),
        revenue: money(t.revenue),
        expenses: money(t.expenses),
        net: money(t.net()),
        margin_pct: t.margin_pct().map(|m| m.to_string()),
        unrecovered_cost: t.unrecovered_cost.map(money),
        by_account: None,
    }
}

/// Statement lines for a tool's output: revenue, then expenses.
fn accounts_out(
    revenue: &[ledger_core::StatementLine],
    expenses: &[ledger_core::StatementLine],
) -> Vec<ProjectAccountOut> {
    let account = |l: &ledger_core::StatementLine, t: &str| ProjectAccountOut {
        code: l.code.clone(),
        name: l.name.clone(),
        account_type: t.to_string(),
        amount: money(l.amount),
    };
    revenue
        .iter()
        .map(|l| account(l, "revenue"))
        .chain(expenses.iter().map(|l| account(l, "expense")))
        .collect()
}

/// An optional date from a tool: "" clears it.
fn optional_date(what: &str, s: &str) -> Result<Option<NaiveDate>, String> {
    match optional_text(s) {
        None => Ok(None),
        Some(t) => parse_date(&t).map(Some).map_err(|e| format!("{what}: {e}")),
    }
}

/// The projects ordered by name, ignoring capitals (then id).
fn by_name(ledger: &Ledger) -> Vec<ClassId> {
    let mut ids: Vec<(String, ClassId)> = ledger
        .list_classes()
        .into_iter()
        .map(|c| (c.name.to_lowercase(), c.id))
        .collect();
    ids.sort();
    ids.into_iter().map(|(_, id)| id).collect()
}

/// The note `books.add_line` and `books.record_payout` give for a line tagged to a finished or
/// abandoned project (rules.md B-37 item 5).
pub(super) fn late_tag_note(ledger: &Ledger, id: ClassId) -> Option<String> {
    let f = ledger.project_facts(id)?;
    if !f.status.is_over() {
        return None;
    }
    let name = &ledger.get_class(id)?.name;
    let ended = f
        .ended_on
        .map(|d| format!(" (ended {d})"))
        .unwrap_or_default();
    Some(format!(
        "project {name} is {}{ended}: tag a late cost or invoice to it only if it belongs to it",
        f.status.key()
    ))
}

#[tool_router(router = projects_router, vis = "pub(super)")]
impl BooksServer {
    #[tool(
        name = "books.add_project",
        title = "Add a project",
        description = "Add a project: a class (books.add_line's class tags a line with it) with its facts. status: active (default), paused, finished, abandoned. kind: internal (the company's own product), client (work for a customer), on_spec (work done hoping to be paid: its costs are expenses, a repayment is income when received), other. party: who it is for or with (id or name). started_on, ended_on (finished or abandoned only), links (web addresses), description, notes. Names are unique. A name alone journals class.added; facts add project.set. Returns the project.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn add_project(
        &self,
        params: Parameters<AddProjectParams>,
    ) -> Result<Json<ProjectOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let facts = ProjectFacts {
            description: p.description.as_deref().and_then(optional_text),
            status: p
                .status
                .as_deref()
                .map(parse_status)
                .transpose()?
                .unwrap_or_default(),
            kind: match p.kind.as_deref().and_then(optional_text) {
                Some(k) => Some(parse_kind(&k)?),
                None => None,
            },
            party: match p.party.as_deref().and_then(optional_text) {
                Some(r) => Some(resolve_party(&store, &r)?),
                None => None,
            },
            started_on: optional_date("started_on", p.started_on.as_deref().unwrap_or(""))?,
            ended_on: optional_date("ended_on", p.ended_on.as_deref().unwrap_or(""))?,
            links: p.links,
            notes: p.notes.as_deref().and_then(optional_text),
        };
        let id = store
            .add_project(&p.name, &facts)
            .map_err(|e| e.to_string())?;
        Ok(Json(project_out(store.ledger(), id)))
    }

    #[tool(
        name = "books.update_project",
        title = "Update a project",
        description = "Change a project's facts (see books.add_project): give only the fields to change; \"\" clears description, kind, party, started_on, ended_on or notes, and links replaces the list ([] clears it). Every field is checked before anything is written, then the whole record is journaled as project.set; nothing new journals nothing. A rename changes the label every report prints, nothing else; a name another project has is refused. Returns the project.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn update_project(
        &self,
        params: Parameters<UpdateProjectParams>,
    ) -> Result<Json<ProjectOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let id = resolve_class(&store, &p.project)?;
        if p.name.is_none()
            && p.description.is_none()
            && p.status.is_none()
            && p.kind.is_none()
            && p.party.is_none()
            && p.started_on.is_none()
            && p.ended_on.is_none()
            && p.links.is_none()
            && p.notes.is_none()
        {
            return Err(
                "give at least one field to change: name, description, status, kind, party, started_on, ended_on, links, notes"
                    .into(),
            );
        }
        let ledger = store.ledger();
        let mut f = ledger.project_facts(id).expect("resolved");
        let name = p
            .name
            .clone()
            .unwrap_or_else(|| ledger.get_class(id).expect("resolved").name.clone());
        if let Some(v) = &p.description {
            f.description = optional_text(v);
        }
        if let Some(v) = &p.status {
            f.status = parse_status(v)?;
        }
        if let Some(v) = &p.kind {
            f.kind = optional_text(v).map(|k| parse_kind(&k)).transpose()?;
        }
        if let Some(v) = &p.party {
            f.party = optional_text(v)
                .map(|r| resolve_party(&store, &r))
                .transpose()?;
        }
        if let Some(v) = &p.started_on {
            f.started_on = optional_date("started_on", v)?;
        }
        if let Some(v) = &p.ended_on {
            f.ended_on = optional_date("ended_on", v)?;
        }
        if let Some(v) = p.links {
            f.links = v;
        }
        if let Some(v) = &p.notes {
            f.notes = optional_text(v);
        }
        store
            .set_project(id, &name, &f)
            .map_err(|e| e.to_string())?;
        Ok(Json(project_out(store.ledger(), id)))
    }

    #[tool(
        name = "books.projects",
        title = "Projects",
        description = "Every project (every class, with its facts), by name: status, kind, party, dates, and its figures to date (every posted line tagged to it through today in the company's time zone, closes left out): revenue, expenses, net (contribution before overhead), margin_pct, and for an on_spec project its unrecovered cost. status filters to one status.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn projects(
        &self,
        params: Parameters<ProjectsParams>,
    ) -> Result<Json<ProjectsOut>, String> {
        let only = params.0.status.as_deref().map(parse_status).transpose()?;
        let through = today();
        let store = self.lock()?;
        let ledger = store.ledger();
        let mut projects = Vec::new();
        for id in by_name(ledger) {
            let f = ledger.project_facts(id).expect("listed");
            if only.is_some_and(|s| s != f.status) {
                continue;
            }
            let t = ledger_core::project_to_date(ledger, id, through).map_err(|e| e.to_string())?;
            projects.push(ProjectRowOut {
                project: project_out(ledger, id),
                to_date: to_date_out(&t),
            });
        }
        Ok(Json(ProjectsOut {
            through: through.to_string(),
            projects,
        }))
    }

    #[tool(
        name = "books.project",
        title = "Get a project",
        description = described(
            "One project, by id or name: facts, party, decisions, linked documents, and every entry with a line tagged to it. For the period: revenue, expenses, net (contribution before overhead: untagged costs aren't counted), margin_pct, by account, by month (with net_to_date). To date (every tagged line through end): the same totals, by account, and for on_spec the unrecovered cost, the 'if repaid' figure, never a receivable. Only revenue and expense lines count.",
            ledger_core::calendar::PeriodKind::Statement
        ),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn project(
        &self,
        params: Parameters<ProjectParams>,
    ) -> Result<Json<ProjectDetailOut>, String> {
        let p = params.0;
        let (start, end) = self.statement_range(p.start.as_deref(), p.end.as_deref())?;
        let store = self.lock()?;
        let ledger = store.ledger();
        let id = resolve_class(&store, &p.project)?;
        let fig =
            ledger_core::project_figures(ledger, id, start, end).map_err(|e| e.to_string())?;
        let by_account = accounts_out(&fig.period.revenue, &fig.period.expenses);
        let by_month = fig
            .months
            .iter()
            .zip(&fig.months_net_to_date)
            .map(|(m, to_date)| ProjectMonthOut {
                month: m.start.format("%Y-%m").to_string(),
                start: m.start.to_string(),
                end: m.end.to_string(),
                revenue: money(m.revenue),
                expenses: money(m.expenses),
                net: money(m.net()),
                net_to_date: money(*to_date),
            })
            .collect();
        let mut entries = Vec::new();
        for (eid, amount) in ledger_core::project_entries(ledger, id) {
            let e = ledger.get_entry(eid).expect("listed");
            let receipts = store
                .attachments_for(ledger_store::Subject::Entry(eid))
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|a| a.id)
                .collect();
            entries.push(ProjectEntryOut {
                entry_id: eid,
                date: e.date().to_string(),
                description: e.description().to_string(),
                status: match e.status() {
                    ledger_core::EntryStatus::Draft => "draft",
                    ledger_core::EntryStatus::Posted => "posted",
                }
                .to_string(),
                project_amount: money(amount),
                total: ReceiptPolicy::entry_debits(e).ok().map(money),
                reversal_of: e.reversal_of(),
                receipts,
            });
        }
        let documents = store
            .attachments_for(ledger_store::Subject::Project(u64::from(id)))
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|a| receipt_out(&store, a))
            .collect::<Result<_, _>>()?;
        let decisions = ledger
            .conventions()
            .decisions()
            .filter(|d| d.new.projects.contains(&id))
            .map(|d| super::kept::decision_out(ledger, d))
            .collect();
        Ok(Json(ProjectDetailOut {
            project: project_out(ledger, id),
            period: ProjectPeriodOut {
                start: start.to_string(),
                end: end.to_string(),
                revenue: money(fig.period.total_revenue),
                expenses: money(fig.period.total_expenses),
                net: money(fig.period.net_income()),
                margin_pct: fig.period_margin_pct().map(|m| m.to_string()),
                by_account,
                by_month,
            },
            to_date: ToDateOut {
                by_account: Some(accounts_out(
                    &fig.to_date.revenue_by_account,
                    &fig.to_date.expenses_by_account,
                )),
                ..to_date_out(&fig.to_date)
            },
            entries,
            documents,
            decisions,
        }))
    }
}
