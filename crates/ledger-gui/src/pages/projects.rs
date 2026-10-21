//! Projects: every class, with its facts (what it is, where it stands, who
//! it is for), and one project completely: what it earned and cost, by
//! month and by account, its entries, documents, decisions and history
//! (rules.md B-37). Every figure is ledger-core's (`project_figures`,
//! `project_to_date`, `project_entries`), the same the server's
//! `books.projects` and `books.project` return; this module only arranges
//! them, and never writes.

use super::assets::{facts, row};
use super::page;
use super::parties::chip;
use crate::app::Shell;
use crate::books::Books;
use crate::charts::{self, BarGroup};
use crate::fmt;
use crate::nav::Route;
use crate::theme::{self, pal};
use crate::widgets;
use chrono::{Datelike, NaiveDate};
use egui::{RichText, Ui, vec2};
use egui_phosphor::regular as icon;
use ledger_core::{ClassId, ProjectFacts, ProjectKind, ProjectStatus};
use rust_decimal::Decimal;

/// The list's filter, kept across navigation.
#[derive(Default)]
pub struct State {
    /// Only projects with this status (None: every one).
    pub status: Option<ProjectStatus>,
}

/// One project on the list: its id, name, facts and net to date.
pub struct Listed {
    pub id: ClassId,
    pub name: String,
    pub facts: ProjectFacts,
    pub to_date: Option<ledger_core::ProjectToDate>,
}

/// Every project (every class), by name ignoring capitals, with its
/// figures to date through today.
pub fn listed(b: &Books) -> Vec<Listed> {
    let mut out: Vec<Listed> = b
        .ledger
        .list_classes()
        .into_iter()
        .map(|c| Listed {
            id: c.id,
            name: c.name.clone(),
            facts: b.ledger.project_facts(c.id).unwrap_or_default(),
            to_date: ledger_core::project_to_date(&b.ledger, c.id, b.today).ok(),
        })
        .collect();
    out.sort_by_key(|a| (a.name.to_lowercase(), a.id));
    out
}

/// A status's colors: (text, fill).
pub(crate) fn status_colors(
    s: ProjectStatus,
    p: &theme::Palette,
) -> (egui::Color32, egui::Color32) {
    match s {
        ProjectStatus::Active => (p.positive, p.positive_soft),
        ProjectStatus::Paused => (p.warning, p.warning_soft),
        ProjectStatus::Finished => (p.muted, p.hover),
        ProjectStatus::Abandoned => (p.faint, p.hover),
    }
}

/// A kind in words, or that it isn't recorded.
pub fn kind_words(k: Option<ProjectKind>) -> &'static str {
    k.map_or("Kind not recorded", ProjectKind::words)
}

/// The list's sentence: how many in each status, and the net across them.
fn summary(rows: &[Listed]) -> String {
    if rows.is_empty() {
        return "No projects yet.".into();
    }
    let parts: Vec<String> = ProjectStatus::ALL
        .into_iter()
        .filter_map(|s| {
            let n = rows.iter().filter(|r| r.facts.status == s).count();
            (n > 0).then(|| format!("{n} {}", s.key()))
        })
        .collect();
    let net: Option<Decimal> = rows
        .iter()
        .map(|r| r.to_date.as_ref().map(|t| t.net()))
        .try_fold(Decimal::ZERO, |t, n| t.checked_add(n?));
    let counts = if parts.len() == 1 && rows.len() > 1 {
        format!(
            "{}, all {}",
            fmt::count(rows.len(), "project", "projects"),
            rows[0].facts.status.key()
        )
    } else if rows.len() == 1 {
        format!("1 project, {}", rows[0].facts.status.key())
    } else {
        format!(
            "{}: {}",
            fmt::count(rows.len(), "project", "projects"),
            parts.join(", ")
        )
    };
    format!(
        "{counts}. Net to date across them: {}.",
        fmt::dollars_or(net)
    )
}

/// The projects.
pub fn show_list(ui: &mut Ui, b: &Books, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    page(ui, "projects", |ui| {
        let all = listed(b);
        widgets::page_header(ui, "Projects", &summary(&all));
        if all.is_empty() {
            widgets::card(ui, |ui| {
                widgets::empty(
                    ui,
                    icon::BRIEFCASE,
                    "No projects yet",
                    "The agent adds them (books.add_project). Tag lines with a project to see what it earns and costs.",
                )
            });
            return;
        }
        let st = &mut shell.state.projects;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            if chip(ui, &format!("All {}", all.len()), st.status.is_none()) {
                st.status = None;
            }
            for s in ProjectStatus::ALL {
                let n = all.iter().filter(|r| r.facts.status == s).count();
                if n == 0 {
                    continue;
                }
                let on = st.status == Some(s);
                if chip(ui, &format!("{} {n}", s.words()), on) {
                    st.status = if on { None } else { Some(s) };
                }
            }
        });
        ui.add_space(12.0);
        let rows: Vec<&Listed> = all
            .iter()
            .filter(|r| st.status.is_none_or(|s| r.facts.status == s))
            .collect();
        let mut go = None;
        widgets::titled_card(
            ui,
            "Projects",
            |ui| {
                widgets::pill(ui, &rows.len().to_string(), p.muted, p.hover);
            },
            |ui| {
                let amt = widgets::AMOUNT_MIN + 24.0;
                let avail = ui.available_width();
                let status_w = 100.0;
                let kind_w = if avail >= 760.0 { 140.0 } else { 0.0 };
                let party_w = if avail >= 900.0 {
                    (avail * 0.18).clamp(140.0, 220.0)
                } else {
                    0.0
                };
                let w = avail.max(status_w + kind_w + party_w + amt + 180.0);
                let cells = |c: [(String, f32, bool, egui::Color32); 5]| -> Vec<_> {
                    c.into_iter()
                        .enumerate()
                        .filter(|(i, _)| (*i != 2 || kind_w > 0.0) && (*i != 3 || party_w > 0.0))
                        .map(|(_, c)| c)
                        .collect()
                };
                widgets::min_width(ui, "project_list", w, |ui| {
                    let head = |t: &str, cw: f32, right: bool| (t.to_string(), cw, right, p.muted);
                    row(
                        ui,
                        w,
                        &cells([
                            head("Name", 0.0, false),
                            head("Status", status_w, false),
                            head("Kind", kind_w, false),
                            head("Party", party_w, false),
                            head("Net to date", amt, true),
                        ]),
                        None,
                        true,
                        false,
                    );
                    widgets::rule(ui);
                    for r in &rows {
                        let (sc, _) = status_colors(r.facts.status, &p);
                        let party = r
                            .facts
                            .party
                            .map(|x| b.party_label(x))
                            .unwrap_or_else(|| "—".into());
                        let (net_t, net_c) = match r.to_date.as_ref().map(|t| t.net()) {
                            Some(n) if n.is_zero() => ("—".to_string(), p.faint),
                            Some(n) if n < Decimal::ZERO => (fmt::accounting(n), p.negative),
                            Some(n) => (fmt::accounting(n), p.text),
                            None => (fmt::CANT_COMPUTE.to_string(), p.faint),
                        };
                        let sub = r
                            .facts
                            .description
                            .clone()
                            .unwrap_or_else(|| format!("Project {}", r.id));
                        let sub = sub.lines().next().unwrap_or("").to_string();
                        let resp = row(
                            ui,
                            w,
                            &cells([
                                (r.name.clone(), 0.0, false, p.text),
                                (r.facts.status.words().to_string(), status_w, false, sc),
                                (kind_words(r.facts.kind).to_string(), kind_w, false, p.muted),
                                (party, party_w, false, p.muted),
                                (net_t, amt, true, net_c),
                            ]),
                            Some((sub.as_str(), p.faint)),
                            false,
                            true,
                        );
                        let resp = resp.on_hover_text("Open this project");
                        resp.context_menu(|ui| {
                            widgets::copy_item(ui, "Copy name", &r.name);
                            widgets::copy_item(ui, "Copy project id", &r.id.to_string());
                        });
                        if resp.clicked() {
                            go = Some(r.id);
                        }
                    }
                });
            },
        );
        ui.add_space(4.0);
        ui.add(
            egui::Label::new(
                RichText::new(format!(
                    "Net to date: every posted revenue and expense line tagged to the project through {}, closing entries left out; revenue less expenses.",
                    fmt::date(b.today)
                ))
                .font(theme::regular(11.5))
                .color(p.faint),
            )
            .wrap(),
        );
        if let Some(id) = go {
            shell.go(Route::Project(id));
        }
    });
}

/// The journal events naming a project: its class and project records,
/// links of files to it, and decisions naming it (indexes into
/// [`Books::journal`], oldest first).
pub fn project_events(b: &Books, id: ClassId) -> Vec<usize> {
    let id64 = u64::from(id);
    let u = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_u64());
    b.journal
        .iter()
        .enumerate()
        .filter(|(_, line)| {
            let p = &line.payload;
            match line.line_type.as_str() {
                "class.added" | "project.set" => u(p, "id") == Some(id64),
                "attachment.linked" | "attachment.unlinked" => {
                    p.get("subject")
                        .and_then(|s| s.get("kind"))
                        .and_then(|k| k.as_str())
                        == Some("project")
                        && p.get("subject").and_then(|s| u(s, "id")) == Some(id64)
                }
                "decision.recorded" => p
                    .get("projects")
                    .and_then(|v| v.as_array())
                    .is_some_and(|a| a.iter().any(|x| x.as_u64() == Some(id64))),
                _ => false,
            }
        })
        .map(|(i, _)| i)
        .collect()
}

/// A number tile: (label, value, how it is computed, the line under it).
type Tile = (
    String,
    Option<Decimal>,
    String,
    Option<(String, egui::Color32)>,
);

/// One project, completely.
pub fn show_project(ui: &mut Ui, b: &Books, id: ClassId, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    let (Some(class), Some(f)) = (b.ledger.get_class(id), b.ledger.project_facts(id)) else {
        page(ui, "project", |ui| {
            widgets::empty(
                ui,
                icon::BRIEFCASE,
                "No such project",
                "It isn't in these books.",
            );
        });
        return;
    };
    let mut go: Option<Route> = None;
    let today = b.today;
    let to_date = ledger_core::project_to_date(&b.ledger, id, today).ok();
    let entries = ledger_core::project_entries(&b.ledger, id);
    page(ui, "project", |ui| {
        ui.horizontal(|ui| {
            if widgets::link(
                ui,
                RichText::new(format!("{} Projects", icon::ARROW_LEFT))
                    .font(theme::regular(12.5))
                    .color(p.muted),
            )
            .clicked()
            {
                go = Some(Route::Projects);
            }
            widgets::copyable(
                ui,
                RichText::new(format!("· Project {id}"))
                    .font(theme::regular(12.5))
                    .color(p.muted),
                &id.to_string(),
            );
        });
        ui.add_space(4.0);
        widgets::copyable_truncated(
            ui,
            RichText::new(&class.name).font(theme::bold(26.0)),
            &class.name,
        );
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            let (fg, bg) = status_colors(f.status, &p);
            widgets::pill(ui, f.status.words(), fg, bg);
            match f.kind {
                Some(ProjectKind::OnSpec) => {
                    widgets::pill(ui, ProjectKind::OnSpec.words(), p.accent, p.accent_soft)
                        .on_hover_text(
                            "Work done hoping to be paid: its costs are expenses as they are spent, and a repayment is income only when it arrives.",
                        );
                }
                k => {
                    widgets::pill(ui, kind_words(k), p.muted, p.hover);
                }
            }
        });
        if let Some(d) = &f.description {
            ui.add_space(6.0);
            let w = ui.available_width().min(860.0);
            ui.allocate_ui(vec2(w, 0.0), |ui| {
                ui.set_max_width(w);
                ui.label(RichText::new(d).font(theme::regular(15.0)).color(p.muted));
            });
        }
        ui.add_space(14.0);

        // What it earned and cost, to date.
        let span = format!(
            "Every posted line tagged to this project through {}, closing entries left out.",
            fmt::date(today)
        );
        let mut tiles: Vec<Tile> = vec![
            (
                "Income to date".into(),
                to_date.as_ref().map(|t| t.revenue),
                format!("{span} Revenue lines."),
                None,
            ),
            (
                "Spending to date".into(),
                to_date.as_ref().map(|t| t.expenses),
                format!("{span} Expense lines."),
                None,
            ),
            (
                "Net to date".into(),
                to_date.as_ref().map(|t| t.net()),
                format!("{span} Income less spending."),
                to_date.as_ref().map(|t| {
                    let n = t.net();
                    if n < Decimal::ZERO {
                        ("a loss so far".to_string(), p.negative)
                    } else if n.is_zero() {
                        ("even".to_string(), p.muted)
                    } else {
                        ("a profit so far".to_string(), p.positive)
                    }
                }),
            ),
        ];
        if let Some(u) = to_date.as_ref().and_then(|t| t.unrecovered_cost) {
            tiles.push((
                "Unrecovered cost".into(),
                Some(u),
                format!(
                    "{span} Spending less income, never below zero: what it would take to be repaid in full. A hoped-for payment isn't income or a receivable until it arrives (ASC 450-30-25-1), so this is a figure, not something owed to the company."
                ),
                Some(("if repaid: not a receivable".into(), p.muted)),
            ));
        }
        if f.kind == Some(ProjectKind::Client)
            && let Some(t) = &to_date
            && !t.revenue.is_zero()
        {
            tiles.push((
                "Margin to date".into(),
                None,
                format!("{span} Net as a share of income."),
                None,
            ));
        }
        let margin = to_date
            .as_ref()
            .and_then(|t| super::profit::pct(t.net(), t.revenue));
        let n = tiles.len();
        super::grid(ui, n, 170.0, 6, |i, ui| {
            let (label, value, how, sub) = &tiles[i];
            let text = if label == "Margin to date" {
                margin.map_or_else(|| "—".into(), |m| format!("{m}%"))
            } else {
                fmt::dollars_or(*value)
            };
            let r = widgets::kpi(
                ui,
                widgets::Kpi {
                    label,
                    value: text,
                    sub: sub.clone(),
                    spark: None,
                    how,
                },
            );
            if let Some(v) = value {
                r.context_menu(|ui| widgets::copy_item(ui, "Copy amount", &v.to_string()));
            }
        });
        ui.add_space(2.0);

        // By month and by account.
        let first = entries
            .iter()
            .filter_map(|(e, _)| b.ledger.get_entry(*e))
            .filter(|e| e.status() == ledger_core::EntryStatus::Posted)
            .map(|e| e.date())
            .min();
        super::columns(ui, 860.0, &[1.0, 1.0], |col, ui| {
            if col == 0 {
                months_card(ui, b, id, first);
            } else {
                accounts_card(ui, b, id, &mut go);
            }
        });
        super::columns(ui, 860.0, &[1.0, 1.0], |col, ui| {
            if col == 0 {
                facts_card(ui, b, id, &class.name, &f, &mut go);
            } else {
                decisions_card(ui, b, id, &mut go);
            }
        });
        entries_card(ui, b, &entries, &mut go);
        ui.add_space(14.0);
        let here = [ledger_store::Subject::Project(u64::from(id))];
        super::documents::documents_card_with(
            ui,
            b,
            "Documents",
            &here,
            &here,
            "No documents linked to it (its contract, spec or pitch).",
            &mut go,
        );
        ui.add_space(14.0);
        let events = project_events(b, id);
        widgets::titled_card(
            ui,
            "History",
            |_| {},
            |ui| {
                if events.is_empty() {
                    ui.label(RichText::new("No journal events name it.").color(p.muted));
                }
                for &i in &events {
                    let line = &b.journal[i];
                    let ev = super::journal::describe_any(b, line);
                    if widgets::history_row(ui, ev.icon, &ev.text, line.seq, &line.ts).clicked() {
                        go = Some(Route::JournalEvent(line.seq));
                    }
                }
            },
        );
    });
    if let Some(r) = go {
        shell.go(r);
    }
}

/// Income, spending and net by month: from the month of its first posted
/// tagged entry through this month, at most the last 24.
fn months_card(ui: &mut Ui, b: &Books, id: ClassId, first: Option<NaiveDate>) {
    let p = pal(ui.ctx());
    let span = first.map(|f| {
        let start = ledger_core::analysis::interval_start(
            f.min(b.today),
            ledger_core::analysis::Interval::Month,
        );
        crate::books::Books::last_months((start, b.today), 24)
    });
    let months = span
        .and_then(|(s, e)| ledger_core::project_figures(&b.ledger, id, s, e).ok())
        .map(|f| f.months)
        .unwrap_or_default();
    widgets::titled_card(
        ui,
        "By month",
        |ui| {
            if let Some((s, _)) = span {
                ui.label(
                    RichText::new(format!("since {}", fmt::month(s)))
                        .font(theme::regular(12.0))
                        .color(p.muted),
                );
            }
        },
        |ui| {
            if months.is_empty() {
                widgets::empty(ui, icon::CHART_BAR, "No tagged lines yet", "");
                return;
            }
            let multi_year =
                months.first().map(|m| m.start.year()) != months.last().map(|m| m.start.year());
            let groups: Vec<BarGroup> = months
                .iter()
                .map(|m| BarGroup {
                    label: m.start.format("%b").to_string(),
                    values: vec![m.revenue, m.expenses, m.net()],
                    year: multi_year.then(|| m.start.year()),
                })
                .collect();
            // Hover shows a month's figures; a click opens nothing, since
            // the reports are the whole company's, not the project's.
            charts::bar_chart_view(
                ui,
                &groups,
                &[
                    ("Income", p.series[1]),
                    ("Spending", p.series[2]),
                    ("Net", p.series[0]),
                ],
                200.0,
            );
        },
    );
}

/// Its revenue then expense accounts to date, with a total.
fn accounts_card(ui: &mut Ui, b: &Books, id: ClassId, go: &mut Option<Route>) {
    let p = pal(ui.ctx());
    // The to-date statement alone: project_figures would build every
    // month's too, each frame.
    let pl = ledger_core::reports::profit_and_loss_for_period(
        &b.ledger,
        ledger_core::calendar::FIRST_DAY.min(b.today),
        b.today,
        Some(id),
    )
    .ok();
    widgets::titled_card(
        ui,
        "By account, to date",
        |_| {},
        |ui| {
            let Some(pl) = pl.filter(|pl| !pl.revenue.is_empty() || !pl.expenses.is_empty()) else {
                widgets::empty(ui, icon::TREE_STRUCTURE, "No income or spending tagged", "");
                return;
            };
            let amt = widgets::amount_col_w(ui.available_width(), 1, 160.0, 140.0);
            let w = ui.available_width();
            let mut line = |ui: &mut Ui, l: &ledger_core::StatementLine, expense: bool| {
                let shown = if expense { -l.amount } else { l.amount };
                let resp = row(
                    ui,
                    w,
                    &[
                        (format!("{} {}", l.code, l.name), 0.0, false, p.text),
                        (
                            fmt::accounting(shown),
                            amt,
                            true,
                            if shown < Decimal::ZERO {
                                p.negative
                            } else {
                                p.text
                            },
                        ),
                    ],
                    None,
                    false,
                    true,
                );
                if resp.on_hover_text("Open this account's register").clicked()
                    && let Some(a) = b.ledger.chart().find_by_code(&l.code)
                {
                    *go = Some(Route::Account(a.id));
                }
            };
            for l in &pl.revenue {
                line(ui, l, false);
            }
            for l in &pl.expenses {
                line(ui, l, true);
            }
            widgets::rule(ui);
            let net = pl.net_income();
            row(
                ui,
                w,
                &[
                    ("Net".to_string(), 0.0, false, p.text),
                    (
                        fmt::accounting(net),
                        amt,
                        true,
                        if net < Decimal::ZERO {
                            p.negative
                        } else {
                            p.text
                        },
                    ),
                ],
                None,
                true,
                false,
            );
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "Spending shows in parentheses; only revenue and expense lines count.",
                )
                .font(theme::regular(11.5))
                .color(p.faint),
            );
        },
    );
}

/// What it is: kind, status, party, dates, links, ids, description, notes.
fn facts_card(
    ui: &mut Ui,
    b: &Books,
    id: ClassId,
    name: &str,
    f: &ProjectFacts,
    go: &mut Option<Route>,
) {
    let p = pal(ui.ctx());
    widgets::titled_card(
        ui,
        "Facts",
        |_| {},
        |ui| {
            facts(ui, |ui, k| {
                let none = |ui: &mut Ui| {
                    ui.label(RichText::new("not recorded").color(p.faint));
                };
                k(ui, "Name");
                widgets::copyable_truncated(ui, RichText::new(name), name);
                ui.end_row();
                k(ui, "Kind");
                match f.kind {
                    Some(kind) => {
                        ui.label(kind.words());
                    }
                    None => none(ui),
                }
                ui.end_row();
                k(ui, "Status");
                ui.label(f.status.words());
                ui.end_row();
                k(ui, "Party");
                match f.party {
                    Some(pid) => {
                        if widgets::link(ui, RichText::new(b.party_label(pid)).color(p.accent))
                            .on_hover_text("Open this party")
                            .clicked()
                        {
                            *go = Some(Route::Party(pid));
                        }
                    }
                    None => none(ui),
                }
                ui.end_row();
                k(ui, "Started");
                match f.started_on {
                    Some(d) => {
                        ui.label(fmt::date(d));
                    }
                    None => none(ui),
                }
                ui.end_row();
                if f.ended_on.is_some() || f.status.is_over() {
                    k(ui, "Ended");
                    match f.ended_on {
                        Some(d) => {
                            ui.label(fmt::date(d));
                        }
                        None => none(ui),
                    }
                    ui.end_row();
                }
                k(ui, "Links");
                if f.links.is_empty() {
                    none(ui);
                } else {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for l in &f.links {
                            widgets::copyable_truncated(ui, RichText::new(l), l);
                        }
                    });
                }
                ui.end_row();
                k(ui, "Project id");
                widgets::copyable(ui, RichText::new(id.to_string()), &id.to_string());
                ui.end_row();
            });
            if let Some(n) = &f.notes {
                ui.add_space(8.0);
                ui.label(
                    RichText::new("Notes")
                        .font(theme::regular(12.5))
                        .color(p.muted),
                );
                ui.add(egui::Label::new(RichText::new(n).font(theme::regular(13.0))).wrap());
            }
        },
    );
}

/// The decisions naming it, newest first; replaced ones muted.
fn decisions_card(ui: &mut Ui, b: &Books, id: ClassId, go: &mut Option<Route>) {
    let p = pal(ui.ctx());
    let decisions: Vec<_> = b
        .ledger
        .conventions()
        .decisions()
        .filter(|d| d.new.projects.contains(&id))
        .collect();
    widgets::titled_card(
        ui,
        "Decisions",
        |ui| {
            if !decisions.is_empty() {
                widgets::pill(ui, &decisions.len().to_string(), p.muted, p.hover);
            }
        },
        |ui| {
            if decisions.is_empty() {
                ui.label(RichText::new("No decision names this project.").color(p.muted));
                return;
            }
            for (i, d) in decisions.iter().rev().enumerate() {
                if i > 0 {
                    ui.add_space(4.0);
                    widgets::rule(ui);
                    ui.add_space(4.0);
                }
                let replaced = d.superseded_by.is_some();
                let text = if replaced { p.muted } else { p.text };
                if widgets::link(
                    ui,
                    RichText::new(&d.new.subject)
                        .font(theme::semibold(13.0))
                        .color(text),
                )
                .on_hover_text("Open it in the journal")
                .clicked()
                {
                    *go = Some(Route::JournalEvent(d.seq));
                }
                ui.add(
                    egui::Label::new(
                        RichText::new(&d.new.decision_md)
                            .font(theme::regular(12.5))
                            .color(if replaced { p.muted } else { p.text }),
                    )
                    .wrap(),
                );
                let mut by = format!(
                    "{} · {}",
                    d.new.decided_by.join(" and "),
                    fmt::date(d.new.decided_on)
                );
                if let Some(n) = d.superseded_by {
                    by.push_str(&format!(" · replaced by #{n}"));
                }
                ui.label(RichText::new(by).font(theme::regular(11.0)).color(p.faint));
            }
        },
    );
}

/// Every entry with a line tagged to it, newest first, with its project
/// amount.
fn entries_card(ui: &mut Ui, b: &Books, entries: &[(u64, Decimal)], go: &mut Option<Route>) {
    let p = pal(ui.ctx());
    widgets::titled_card(
        ui,
        "Entries",
        |ui| {
            ui.label(
                RichText::new(fmt::count(entries.len(), "entry", "entries"))
                    .font(theme::regular(12.0))
                    .color(p.muted),
            );
        },
        |ui| {
            if entries.is_empty() {
                ui.label(
                    RichText::new("No line in the books is tagged with it yet.").color(p.muted),
                );
                return;
            }
            let amt = widgets::AMOUNT_MIN + 24.0;
            let w = ui.available_width().max(104.0 + 64.0 + 150.0 + amt + 200.0);
            widgets::min_width(ui, "project_entries", w, |ui| {
                let head = |t: &str, cw: f32, right: bool| (t.to_string(), cw, right, p.muted);
                row(
                    ui,
                    w,
                    &[
                        head("Date", 104.0, false),
                        head("Entry", 64.0, false),
                        head("Description", 0.0, false),
                        head("Kind", 150.0, false),
                        head("Project amount", amt, true),
                    ],
                    None,
                    true,
                    false,
                );
                widgets::rule(ui);
                for (eid, amount) in entries {
                    let Some(e) = b.entry(*eid) else {
                        continue;
                    };
                    let draft = e.status != ledger_core::EntryStatus::Posted;
                    let text = if draft { p.muted } else { p.text };
                    let kind = if draft {
                        format!("{} · draft", e.origin)
                    } else {
                        e.origin.to_string()
                    };
                    let mut sub: Vec<String> = Vec::new();
                    if e.amount != Some(amount.abs()) {
                        sub.push(format!("of {}", fmt::dollars_or(e.amount)));
                    }
                    if !e.receipts.is_empty() {
                        sub.push(fmt::count(e.receipts.len(), "receipt", "receipts"));
                    }
                    let accounts: Vec<String> =
                        e.accounts.iter().map(|a| b.account_name(*a)).collect();
                    sub.push(accounts.join(" → "));
                    let sub = sub.join(" · ");
                    let resp = row(
                        ui,
                        w,
                        &[
                            (fmt::date(e.date), 104.0, false, text),
                            (format!("#{}", e.id), 64.0, false, p.muted),
                            (e.description.clone(), 0.0, false, text),
                            (kind, 150.0, false, if draft { p.warning } else { p.muted }),
                            (
                                fmt::accounting(*amount),
                                amt,
                                true,
                                if *amount < Decimal::ZERO {
                                    p.negative
                                } else {
                                    text
                                },
                            ),
                        ],
                        Some((sub.as_str(), p.faint)),
                        false,
                        true,
                    );
                    resp.context_menu(|ui| {
                        widgets::copy_item(ui, "Copy entry number", &format!("#{}", e.id));
                        widgets::copy_item(ui, "Copy project amount", &amount.to_string());
                    });
                    if resp.on_hover_text("Open this entry").clicked() {
                        *go = Some(Route::Entry(e.id));
                    }
                }
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "Project amount: the entry's tagged income less its tagged spending; a tagged balance-sheet line counts nothing.",
                )
                .font(theme::regular(11.5))
                .color(p.faint),
            );
        },
    );
}
