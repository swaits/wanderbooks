//! Projects: every class, with its facts (what it is, where it stands, who
//! it is for), and one project completely: its snapshot (revenue, direct
//! costs, contribution before overhead and its margin, to date or this
//! fiscal year, rules.md B-37 item 7), revenue and costs by month with the
//! running net, costs by category and by account, its entries, documents,
//! decisions and history (rules.md B-37). Every figure is ledger-core's (`project_figures`,
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
    /// A project's snapshot shows this fiscal year to date (B-30's
    /// omitted dates), not to date.
    pub fiscal: bool,
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
                // The cue repeats what the net's color says: it goes
                // first as the list narrows, then the kind.
                let kind_w = if avail >= 640.0 { 140.0 } else { 0.0 };
                let cue_w = if avail >= 760.0 { 120.0 } else { 0.0 };
                let party_w = if avail >= 900.0 {
                    (avail * 0.18).clamp(140.0, 220.0)
                } else {
                    0.0
                };
                let w = avail.max(status_w + kind_w + party_w + cue_w + amt + 180.0);
                let cells = |c: [(String, f32, bool, egui::Color32); 6]| -> Vec<_> {
                    c.into_iter()
                        .enumerate()
                        .filter(|(i, _)| {
                            (*i != 2 || kind_w > 0.0)
                                && (*i != 3 || party_w > 0.0)
                                && (*i != 4 || cue_w > 0.0)
                        })
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
                            head("", cue_w, true),
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
                        let snap = r.to_date.as_ref().map(Snap::to_date);
                        let (net_t, net_c) = match &snap {
                            Some(s) if s.standing == Standing::Nothing => {
                                ("—".to_string(), p.faint)
                            }
                            Some(s) => (fmt::accounting(s.contribution), s.standing.colors(&p).0),
                            None => (fmt::CANT_COMPUTE.to_string(), p.faint),
                        };
                        // A small cue beside the net: where it stands.
                        let (cue_t, cue_c) = match snap.as_ref().map(|s| s.standing) {
                            Some(st @ (Standing::Black | Standing::Red | Standing::Even)) => {
                                let mark = match st {
                                    Standing::Black => icon::TREND_UP,
                                    Standing::Red => icon::TREND_DOWN,
                                    _ => icon::EQUALS,
                                };
                                (
                                    format!("{mark} {}", st.words().to_lowercase()),
                                    st.colors(&p).0,
                                )
                            }
                            _ => (String::new(), p.faint),
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
                                (cue_t, cue_w, true, cue_c),
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
                    "Net to date: every posted revenue and expense line tagged to the project through {}, closing entries left out; revenue less its direct costs, before overhead (untagged company costs aren't counted).",
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

/// A snapshot tile: (label, value, the line under it, how it is
/// computed, the amount its menu copies).
type Tile<'a> = (
    &'a str,
    String,
    Option<(String, egui::Color32)>,
    String,
    Option<Decimal>,
);

/// Where a project stands on its contribution before overhead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    /// Contribution above zero.
    Black,
    /// Contribution below zero.
    Red,
    /// Lines tagged, netting to zero (a cost and its reversal too).
    Even,
    /// No revenue or expense line tagged (in the span shown).
    Nothing,
}

impl Standing {
    /// From a contribution, and whether any posted revenue or expense
    /// line is tagged at all (counted lines, not accounts left with a
    /// balance: a reversed cost is tagged and breaks even).
    pub fn of(contribution: Decimal, any: bool) -> Self {
        if contribution > Decimal::ZERO {
            Standing::Black
        } else if contribution < Decimal::ZERO {
            Standing::Red
        } else if any {
            Standing::Even
        } else {
            Standing::Nothing
        }
    }

    /// Its words on a pill.
    pub fn words(self) -> &'static str {
        match self {
            Standing::Black => "In the black",
            Standing::Red => "In the red",
            Standing::Even => "Break-even",
            Standing::Nothing => "Nothing tagged yet",
        }
    }

    /// Its colors: (text, fill), the palette's positive and negative.
    pub fn colors(self, p: &theme::Palette) -> (egui::Color32, egui::Color32) {
        match self {
            Standing::Black => (p.positive, p.positive_soft),
            Standing::Red => (p.negative, p.negative_soft),
            Standing::Even | Standing::Nothing => (p.muted, p.hover),
        }
    }
}

/// What a snapshot shows, all ledger-core's: to date
/// (`project_to_date`) or for a period (`project_figures`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snap {
    /// Tagged revenue.
    pub revenue: Decimal,
    /// Tagged expenses: its direct costs.
    pub expenses: Decimal,
    /// Revenue less direct costs: its contribution before overhead.
    pub contribution: Decimal,
    /// Contribution as a percent of revenue (none without revenue).
    pub margin_pct: Option<Decimal>,
    /// Where it stands.
    pub standing: Standing,
}

impl Snap {
    /// To date.
    pub fn to_date(t: &ledger_core::ProjectToDate) -> Self {
        let any = t.any_tagged();
        Snap {
            revenue: t.revenue,
            expenses: t.expenses,
            contribution: t.net(),
            margin_pct: t.margin_pct(),
            standing: Standing::of(t.net(), any),
        }
    }

    /// For the figures' period.
    pub fn period(f: &ledger_core::ProjectFigures) -> Self {
        let pl = &f.period;
        let any = f.period_lines > 0;
        Snap {
            revenue: pl.total_revenue,
            expenses: pl.total_expenses,
            contribution: pl.net_income(),
            margin_pct: f.period_margin_pct(),
            standing: Standing::of(pl.net_income(), any),
        }
    }
}

/// The note on contribution: what it counts and what it leaves out.
pub const CONTRIBUTION_NOTE: &str = "Revenue less this project's direct costs: every expense line tagged to it, fixed and variable alike (what management accounting calls a segment margin, not a variable-cost contribution margin). Company overhead that isn't tagged to a project (rent, software, insurance, the owner's time) isn't counted, so a project in the black can still leave the company in the red. A management-accounting view (rules.md B-37 item 7), not a GAAP figure.";

/// The snapshot: a header with where it stands to date and the to date /
/// fiscal year switch, then its tiles (`shown`: to date, or for `year`,
/// the fiscal year to date, when `fiscal`). Returns the switch's new
/// setting (true: this fiscal year) when clicked.
fn snapshot(
    ui: &mut Ui,
    b: &Books,
    kind: Option<ProjectKind>,
    to_date: Option<&ledger_core::ProjectToDate>,
    year: Option<(NaiveDate, NaiveDate)>,
    shown: Option<Snap>,
    fiscal: bool,
) -> Option<bool> {
    let p = pal(ui.ctx());
    let mut switch = None;
    let year = year.filter(|_| fiscal);
    let span = match year {
        Some((s, e)) => format!(
            "Every posted line tagged to this project from {} through {} (the fiscal year to date), closing entries left out.",
            fmt::date(s),
            fmt::date(e)
        ),
        None => format!(
            "Every posted line tagged to this project through {}, closing entries left out.",
            fmt::date(b.today)
        ),
    };
    let when = match year {
        Some((s, _)) => format!("since {}", fmt::date(s)),
        None => format!("through {}", fmt::date(b.today)),
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new("Snapshot").font(theme::semibold(14.0)));
        // Where it stands is always to date, whichever view: the list's
        // cue too, and the page's to-date cards beside it agree.
        if let Some(t) = to_date.map(Snap::to_date) {
            let (fg, bg) = t.standing.colors(&p);
            widgets::pill(ui, t.standing.words(), fg, bg).on_hover_text(format!(
                "Contribution before overhead, to date (through {}): {}.",
                fmt::date(b.today),
                fmt::dollars(t.contribution)
            ));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if chip(ui, "This fiscal year", fiscal) && !fiscal {
                switch = Some(true);
            }
            if chip(ui, "To date", !fiscal) && fiscal {
                switch = Some(false);
            }
        });
    });
    ui.add_space(8.0);
    let money = |v: Option<Decimal>| fmt::dollars_or(v);
    let contribution_sub = shown.as_ref().map(|s| {
        let (fg, _) = s.standing.colors(&p);
        let text = match s.standing {
            Standing::Black | Standing::Red | Standing::Even => "before overhead",
            Standing::Nothing if fiscal => "nothing this fiscal year",
            Standing::Nothing => "nothing tagged yet",
        };
        (
            text.to_string(),
            if s.standing == Standing::Nothing {
                p.muted
            } else {
                fg
            },
        )
    });
    let mut tiles: Vec<Tile<'_>> = vec![
        (
            "Revenue",
            money(shown.as_ref().map(|s| s.revenue)),
            Some((when.clone(), p.muted)),
            format!("{span} Revenue lines."),
            shown.as_ref().map(|s| s.revenue),
        ),
        (
            "Direct costs",
            money(shown.as_ref().map(|s| s.expenses)),
            Some((when.clone(), p.muted)),
            format!("{span} Expense lines: the costs tagged to it."),
            shown.as_ref().map(|s| s.expenses),
        ),
        (
            "Contribution",
            money(shown.as_ref().map(|s| s.contribution)),
            contribution_sub,
            format!("{span} {CONTRIBUTION_NOTE}"),
            shown.as_ref().map(|s| s.contribution),
        ),
    ];
    if let Some(m) = shown.as_ref().and_then(|s| s.margin_pct) {
        tiles.push((
            "Contribution margin",
            format!("{m}%"),
            Some(("of revenue".to_string(), p.muted)),
            format!("{span} Contribution as a share of revenue, to one place. Shown only when there is revenue."),
            None,
        ));
    }
    if kind == Some(ProjectKind::OnSpec)
        && let Some(u) = to_date.and_then(|t| t.unrecovered_cost)
    {
        tiles.push((
            "Unrecovered cost",
            fmt::dollars(u),
            Some(("if repaid: not a receivable".into(), p.muted)),
            format!(
                "Every posted line tagged to this project through {}, whichever view: direct costs less revenue to date, never below zero, what it would take to be repaid in full. A hoped-for payment isn't income or a receivable until it arrives (ASC 450-30-25-1), so this is a figure, not something owed to the company.",
                fmt::date(b.today)
            ),
            Some(u),
        ));
    }
    let n = tiles.len();
    super::grid(ui, n, 170.0, 6, |i, ui| {
        let (label, value, sub, how, copy) = &tiles[i];
        let r = widgets::kpi(
            ui,
            widgets::Kpi {
                label,
                value: value.clone(),
                sub: sub.clone(),
                spark: None,
                how,
            },
        );
        if let Some(v) = copy {
            r.context_menu(|ui| widgets::copy_item(ui, "Copy amount", &v.to_string()));
        }
    });
    switch
}

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
    let fiscal = shell.state.projects.fiscal;
    let figs = figures(ui.ctx(), b, id, f.started_on, fiscal);
    let to_date = figs.to_date.as_ref();
    let entries = &figs.entries;
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

        // The snapshot: what it earned and cost, to date or this fiscal
        // year, before overhead.
        let shown = if fiscal {
            figs.year.as_ref().map(Snap::period)
        } else {
            to_date.map(Snap::to_date)
        };
        let year = figs.year.as_ref().map(|y| (y.period.start, y.period.end));
        if let Some(fy) = snapshot(ui, b, f.kind, to_date, year, shown, fiscal) {
            shell.state.projects.fiscal = fy;
        }
        ui.add_space(2.0);

        // By month and by category.
        super::columns(ui, 860.0, &[1.3, 1.0], |col, ui| {
            if col == 0 {
                months_card(ui, &figs);
            } else {
                costs_card(ui, b, to_date, &mut go);
            }
        });
        super::columns(ui, 860.0, &[1.0, 1.0], |col, ui| {
            if col == 0 {
                facts_card(ui, b, id, &class.name, &f, &mut go);
            } else {
                accounts_card(ui, b, to_date, &mut go);
                ui.add_space(16.0);
                decisions_card(ui, b, id, &mut go);
            }
        });
        entries_card(ui, b, entries, &mut go);
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

/// The most months the by-month chart shows: the latest.
pub const CHART_MONTHS: usize = 24;

/// The months the by-month chart spans: from the month the project
/// started, or of its first posted tagged entry if earlier, through
/// today, the last [`CHART_MONTHS`] at most. None with neither date.
pub fn chart_span(
    today: NaiveDate,
    started_on: Option<NaiveDate>,
    first_entry: Option<NaiveDate>,
) -> Option<(NaiveDate, NaiveDate)> {
    let from = match (started_on, first_entry) {
        (Some(s), Some(e)) => s.min(e),
        (s, e) => s.or(e)?,
    };
    let start = ledger_core::analysis::interval_start(
        from.min(today),
        ledger_core::analysis::Interval::Month,
    );
    Some(Books::last_months((start, today), CHART_MONTHS as u32))
}

/// A project page's figures, all ledger-core's, worked out once per load
/// of the books (and the fiscal year's once it is asked for), not every
/// frame: a hovered chart repaints continuously.
#[derive(Clone)]
pub struct PageFigures {
    /// Through today (`project_to_date`).
    pub to_date: Option<ledger_core::ProjectToDate>,
    /// Every entry tagged to it (`project_entries`).
    pub entries: Vec<(u64, Decimal)>,
    /// The by-month chart's span ([`chart_span`]) and figures, when
    /// anything tagged is posted.
    pub months: Option<((NaiveDate, NaiveDate), ledger_core::ProjectFigures)>,
    /// The fiscal year to date (B-30's omitted dates, as `books.project`),
    /// only when the snapshot shows it.
    pub year: Option<ledger_core::ProjectFigures>,
}

/// [`PageFigures`] for project `id`, kept in egui's memory against the
/// load of the books they came from (a reload, a new day or another
/// project works them out again).
pub fn figures(
    ctx: &egui::Context,
    b: &Books,
    id: ClassId,
    started_on: Option<NaiveDate>,
    fiscal: bool,
) -> std::sync::Arc<PageFigures> {
    type Stamp = (chrono::DateTime<chrono::Utc>, String, NaiveDate, bool);
    let key = egui::Id::new(("project_figures", id));
    let stamp: Stamp = (b.loaded_at, b.head.clone(), b.today, fiscal);
    if let Some((was, figs)) = ctx.data(|d| d.get_temp::<(Stamp, std::sync::Arc<PageFigures>)>(key))
        && was == stamp
    {
        return figs;
    }
    let figs = std::sync::Arc::new(page_figures(b, id, started_on, fiscal));
    ctx.data_mut(|d| d.insert_temp(key, (stamp, figs.clone())));
    figs
}

/// [`PageFigures`], worked out.
pub fn page_figures(
    b: &Books,
    id: ClassId,
    started_on: Option<NaiveDate>,
    fiscal: bool,
) -> PageFigures {
    let to_date = ledger_core::project_to_date(&b.ledger, id, b.today).ok();
    let entries = ledger_core::project_entries(&b.ledger, id);
    let first = entries
        .iter()
        .filter_map(|(e, _)| b.ledger.get_entry(*e))
        .filter(|e| e.status() == ledger_core::EntryStatus::Posted)
        .map(|e| e.date())
        .min();
    // Nothing posted yet: no bars to draw, however long ago it started.
    let months = chart_span(b.today, started_on, first)
        .filter(|_| first.is_some())
        .and_then(|(s, e)| {
            let fig = ledger_core::project_figures(&b.ledger, id, s, e).ok()?;
            Some(((s, e), fig))
        });
    let year = fiscal
        .then(|| b.periods().statement(None, None).ok())
        .flatten()
        .and_then(|(s, e)| ledger_core::project_figures(&b.ledger, id, s, e).ok());
    PageFigures {
        to_date,
        entries,
        months,
        year,
    }
}

/// Revenue and costs by month, with the net to date at each month's end:
/// the trend toward or away from break-even.
fn months_card(ui: &mut Ui, figs: &PageFigures) {
    let p = pal(ui.ctx());
    let span = figs.months.as_ref().map(|(span, _)| *span);
    let fig = figs.months.as_ref().map(|(_, fig)| fig);
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
            let Some(fig) = fig.filter(|f| !f.months.is_empty()) else {
                widgets::empty(ui, icon::CHART_BAR, "No tagged lines yet", "");
                return;
            };
            let months = &fig.months;
            let multi_year =
                months.first().map(|m| m.start.year()) != months.last().map(|m| m.start.year());
            let groups: Vec<BarGroup> = months
                .iter()
                .map(|m| BarGroup {
                    label: m.start.format("%b").to_string(),
                    values: vec![m.revenue, m.expenses],
                    year: multi_year.then(|| m.start.year()),
                })
                .collect();
            // Hover shows a month's figures; a click opens nothing, since
            // the reports are the whole company's, not the project's.
            charts::bar_chart_view_with_line(
                ui,
                &groups,
                &[("Revenue", p.series[1]), ("Direct costs", p.series[2])],
                &charts::Overlay {
                    name: "Net to date",
                    color: p.series[0],
                    values: &fig.months_net_to_date,
                },
                200.0,
            );
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "Net to date: every tagged line through each month's end, before overhead; above $0 it is in the black.",
                    )
                    .font(theme::regular(11.5))
                    .color(p.faint),
                )
                .wrap(),
            );
        },
    );
}

/// Its direct costs by expense account to date, largest first; a click
/// opens the account's register.
fn costs_card(
    ui: &mut Ui,
    b: &Books,
    to_date: Option<&ledger_core::ProjectToDate>,
    go: &mut Option<Route>,
) {
    let p = pal(ui.ctx());
    let mut lines: Vec<&ledger_core::StatementLine> = to_date
        .map(|t| t.expenses_by_account.iter().collect())
        .unwrap_or_default();
    // Largest first, then by code: the bars rank.
    lines.sort_by(|x, y| y.amount.cmp(&x.amount).then_with(|| x.code.cmp(&y.code)));
    widgets::titled_card(
        ui,
        "Costs by category, to date",
        |ui| {
            if let Some(t) = to_date.filter(|_| !lines.is_empty()) {
                ui.label(
                    RichText::new(fmt::dollars(t.expenses))
                        .font(theme::regular(12.0))
                        .color(p.muted),
                );
            }
        },
        |ui| {
            if lines.is_empty() {
                // Costs tagged and cancelled out (a reversal, or repaid
                // exactly) leave no account, but they were tagged.
                if to_date.is_some_and(|t| t.expenses_lines > 0) {
                    widgets::empty(
                        ui,
                        icon::CHART_BAR_HORIZONTAL,
                        "Tagged costs net to zero",
                        "Every cost tagged to it was reversed or repaid in full.",
                    );
                } else {
                    widgets::empty(ui, icon::CHART_BAR_HORIZONTAL, "No costs tagged", "");
                }
                return;
            }
            let rows: Vec<(String, Decimal)> = lines
                .iter()
                .map(|l| (format!("{} {}", l.code, l.name), l.amount))
                .collect();
            if let Some(i) = charts::hbars_signed(ui, &rows, p.series[2], p.positive)
                && let Some(a) = b.ledger.chart().find_by_code(&lines[i].code)
            {
                *go = Some(Route::Account(a.id));
            }
            if lines.iter().any(|l| l.amount < Decimal::ZERO) {
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(
                            "Below zero: an account credited more than it cost (a reimbursement).",
                        )
                        .font(theme::regular(11.5))
                        .color(p.faint),
                    )
                    .wrap(),
                );
            }
        },
    );
}

/// Its revenue then expense accounts to date, with a total.
fn accounts_card(
    ui: &mut Ui,
    b: &Books,
    to_date: Option<&ledger_core::ProjectToDate>,
    go: &mut Option<Route>,
) {
    let p = pal(ui.ctx());
    widgets::titled_card(
        ui,
        "By account, to date",
        |_| {},
        |ui| {
            let Some(t) = to_date
                .filter(|t| !t.revenue_by_account.is_empty() || !t.expenses_by_account.is_empty())
            else {
                if to_date.is_some_and(|t| t.any_tagged()) {
                    widgets::empty(
                        ui,
                        icon::TREE_STRUCTURE,
                        "Tagged lines net to zero",
                        "Each account's tagged lines cancel out (a reversal, or a cost repaid in full): a contribution of 0.00.",
                    );
                } else {
                    widgets::empty(ui, icon::TREE_STRUCTURE, "No income or spending tagged", "");
                }
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
            for l in &t.revenue_by_account {
                line(ui, l, false);
            }
            for l in &t.expenses_by_account {
                line(ui, l, true);
            }
            widgets::rule(ui);
            let net = t.net();
            row(
                ui,
                w,
                &[
                    ("Contribution".to_string(), 0.0, false, p.text),
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
            )
            .on_hover_text(CONTRIBUTION_NOTE);
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
