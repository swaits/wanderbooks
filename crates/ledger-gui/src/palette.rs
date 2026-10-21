//! Ctrl+K: jump to anything. Pages, reports, accounts, entries (by text,
//! amount, or #id), fixed assets (by tag, serial, or description), parties
//! (vendors and customers among them, by name or display name),
//! documents, bank and card statements, and reconciliations (by account
//! and date), ranked by a simple fuzzy match.

use crate::books::Books;
use crate::fmt;
use crate::nav::{ReportKind, Route, Section};
use crate::theme::{self, pal};
use egui::{CornerRadius, Key, Margin, RichText, Sense, Stroke, vec2};
use egui_phosphor::regular as icon;

#[derive(Default)]
pub struct Palette {
    open: bool,
    query: String,
    selected: usize,
    focus: bool,
}

/// One search result.
pub struct Item {
    pub icon: &'static str,
    pub title: String,
    pub detail: String,
    pub route: Route,
    pub score: i32,
}

impl Palette {
    pub fn open(&mut self) {
        self.open = true;
        self.query.clear();
        self.selected = 0;
        self.focus = true;
    }

    /// Show the palette if open; returns where to go.
    pub fn show(&mut self, ctx: &egui::Context, books: &Books) -> Option<Route> {
        if !self.open {
            return None;
        }
        let p = pal(ctx);
        let items = search(books, &self.query);
        let mut go = None;
        let (up, down, enter, esc) = ctx.input(|i| {
            (
                i.key_pressed(Key::ArrowUp),
                i.key_pressed(Key::ArrowDown),
                i.key_pressed(Key::Enter),
                i.key_pressed(Key::Escape),
            )
        });
        if down {
            self.selected = (self.selected + 1).min(items.len().saturating_sub(1));
        }
        if up {
            self.selected = self.selected.saturating_sub(1);
        }
        if esc {
            self.open = false;
            return None;
        }
        if enter && let Some(it) = items.get(self.selected) {
            self.open = false;
            return Some(it.route.clone());
        }
        let modal = egui::Modal::new(egui::Id::new("palette"))
            .frame(
                egui::Frame::new()
                    .fill(p.surface)
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(CornerRadius::same(14))
                    .inner_margin(Margin::same(10)),
            )
            .show(ctx, |ui| {
                // Never wider or taller than the window allows.
                let screen = ctx.content_rect().size();
                ui.set_width(620.0_f32.min(screen.x - 48.0).max(200.0));
                let edit = egui::TextEdit::singleline(&mut self.query)
                    .hint_text("Search entries, accounts, assets, vendors, documents, reports…")
                    .font(theme::regular(16.0))
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY);
                let r = ui.add(edit);
                if self.focus {
                    r.request_focus();
                    self.focus = false;
                }
                if r.changed() {
                    self.selected = 0;
                }
                ui.add_space(6.0);
                crate::widgets::rule(ui);
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .max_height(420.0_f32.min(screen.y - 160.0).max(120.0))
                    .show(ui, |ui| {
                        if items.is_empty() {
                            ui.label(RichText::new("Nothing matches.").color(p.muted));
                        }
                        for (i, it) in items.iter().enumerate() {
                            let (rect, resp) = ui.allocate_exact_size(
                                vec2(ui.available_width(), 40.0),
                                Sense::click(),
                            );
                            if i == self.selected || resp.hovered() {
                                ui.painter()
                                    .rect_filled(rect, CornerRadius::same(8), p.hover);
                            }
                            if i == self.selected {
                                ui.scroll_to_rect(rect, None);
                            }
                            let painter = ui.painter_at(rect);
                            painter.text(
                                rect.left_center() + vec2(12.0, 0.0),
                                egui::Align2::LEFT_CENTER,
                                it.icon,
                                egui::FontId::proportional(16.0),
                                p.muted,
                            );
                            painter.text(
                                rect.left_center() + vec2(38.0, -7.0),
                                egui::Align2::LEFT_CENTER,
                                &it.title,
                                theme::regular(14.0),
                                p.text,
                            );
                            painter.text(
                                rect.left_center() + vec2(38.0, 9.0),
                                egui::Align2::LEFT_CENTER,
                                &it.detail,
                                theme::regular(11.5),
                                p.faint,
                            );
                            if resp.clicked() {
                                go = Some(it.route.clone());
                            }
                        }
                    });
            });
        if modal.should_close() {
            self.open = false;
        }
        if go.is_some() {
            self.open = false;
        }
        go
    }
}

/// Subsequence match: every query character in order, rewarding
/// contiguous runs and word starts. None if no match.
/// Fuzzy match score, higher is better.
///
/// ```text
/// INVARIANTS:
///   1. Matches (Some) exactly when the query's non-space characters,
///      lower-cased, appear in order in the lower-cased hay; the empty
///      query matches everything with score 0.
///   2. A match's score is at least the number of non-space query
///      characters; consecutive matches and matches at a word start score
///      more. The score is the best over every way to match (not the
///      first match found), so a hay holding the query whole at a word
///      start scores at least as high as any hay's match of it.
/// ```
pub fn fuzzy(hay: &str, query: &str) -> Option<i32> {
    let hay: Vec<char> = hay.to_lowercase().chars().collect();
    // Query characters, each marked whether a space came just before it
    // (a space ends a run: the next match earns no run bonus).
    let mut q: Vec<(char, bool)> = Vec::new();
    let mut after_space = false;
    for c in query.to_lowercase().chars() {
        if c == ' ' {
            after_space = true;
        } else {
            q.push((c, after_space));
            after_space = false;
        }
    }
    if q.is_empty() {
        return Some(0);
    }
    // The best score over every way to match (a greedy left-most match
    // can miss a word start further on): best[j] is the top score for the
    // query so far with its last character at hay[j].
    let word_start = |j: usize| j == 0 || !hay[j - 1].is_alphanumeric();
    let mut best: Vec<Option<i32>> = hay
        .iter()
        .enumerate()
        .map(|(j, &c)| (c == q[0].0).then(|| 1 + if word_start(j) { 4 } else { 0 }))
        .collect();
    for &(qc, gap) in &q[1..] {
        let mut next = vec![None; hay.len()];
        // The best score ending strictly before j.
        let mut before: Option<i32> = None;
        for j in 0..hay.len() {
            if hay[j] == qc {
                let bonus = if word_start(j) { 4 } else { 0 };
                let run = if gap {
                    None
                } else {
                    j.checked_sub(1)
                        .and_then(|p| best[p])
                        .map(|s| s + 6 + bonus)
                };
                let jump = before.map(|s| s + 1 + bonus);
                next[j] = run.max(jump);
            }
            before = before.max(best[j]);
        }
        best = next;
    }
    best.into_iter().flatten().max()
}

/// What the palette lists for `query`, best first (at most 40).
///
/// ```text
/// INVARIANTS (fixed assets; everything else as it was):
///   1. An asset whose tag equals the trimmed query, ignoring ASCII case,
///      scores 2000: above every other result (an entry's exact #id is
///      1000).
///   2. Otherwise an asset is listed when the query fuzzily matches its
///      tag, serial, and description; a query found whole in its tag or
///      serial (ignoring case) adds 500, in its description 50.
///   3. Every asset result opens Route::Asset(its id).
/// INVARIANTS (parties):
///   4. A party whose name or display name equals the trimmed query,
///      ignoring case, scores 1500: above an entry's exact #id, below an
///      exact asset tag.
///   5. Otherwise a party is listed when the query fuzzily matches its
///      name and display name; a query found whole in either (ignoring
///      case) adds 500.
///   6. Every party result opens Route::Party(its id); archived parties
///      are found too, and say so.
/// INVARIANTS (statements and reconciliations):
///   7. A statement is listed when the query fuzzily matches "statement",
///      its account's code and name, its period's dates (as shown and as
///      YYYY-MM-DD), and its file name; it opens Route::Document(its
///      attachment).
///   8. A reconciliation is listed when the query fuzzily matches
///      "reconciliation reconciled", its account's code and name, and its
///      statement date (as shown and as YYYY-MM-DD); it opens
///      Route::Reconciliation(its id), voided ones too, saying so.
/// ```
pub fn search(b: &Books, query: &str) -> Vec<Item> {
    let q = query.trim();
    let mut out: Vec<Item> = Vec::new();
    let mut add = |icon: &'static str, title: String, detail: String, route: Route, hay: &str| {
        if let Some(score) = fuzzy(hay, q) {
            out.push(Item {
                icon,
                title,
                detail,
                route,
                score,
            });
        }
    };
    for s in Section::ALL {
        add(
            s.icon(),
            s.title().into(),
            "Page".into(),
            s.home(),
            s.title(),
        );
    }
    for r in ReportKind::ALL {
        add(
            icon::FILE_TEXT,
            r.title().into(),
            "Report".into(),
            Route::Reports(r),
            &format!("report {}", r.title()),
        );
    }
    if !q.is_empty() {
        for a in b.ledger.chart().all() {
            add(
                icon::TREE_STRUCTURE,
                format!("{} {}", a.code, a.name),
                format!("{:?} account", a.account_type),
                Route::Account(a.id),
                &format!("{} {}", a.code, a.name),
            );
        }
        let id_query = q.strip_prefix('#').and_then(|s| s.parse::<u64>().ok());
        for e in &b.entries {
            let exact = id_query == Some(e.id);
            let hay = &e.haystack;
            let score = if exact {
                Some(1000)
            } else {
                fuzzy(hay, q).filter(|_| q.len() >= 2)
            };
            if let Some(score) = score {
                // Prefer substring hits for entries: fuzzy over long text is noisy.
                let bonus = if hay.contains(&q.to_lowercase()) {
                    50
                } else {
                    0
                };
                if bonus == 0 && !exact && q.len() < 4 {
                    continue;
                }
                out.push(Item {
                    icon: icon::RECEIPT,
                    title: format!("#{} {}", e.id, e.description),
                    detail: format!("{} · {}", fmt::date(e.date), fmt::dollars_or(e.amount)),
                    route: Route::Entry(e.id),
                    score: score + bonus,
                });
            }
        }
        let ql = q.to_lowercase();
        for a in b.ledger.registered_assets() {
            let tag = a.tag.as_deref().unwrap_or("");
            let serial = a.serial.as_deref().unwrap_or("");
            let hay = format!("{tag} {serial} {}", a.description);
            let score = if !tag.is_empty() && tag.eq_ignore_ascii_case(q) {
                Some(2000)
            } else {
                fuzzy(&hay, q).map(|s| {
                    let whole = |t: &str| !t.is_empty() && t.to_lowercase().contains(&ql);
                    s + if whole(tag) || whole(serial) {
                        500
                    } else if whole(&a.description) {
                        50
                    } else {
                        0
                    }
                })
            };
            if let Some(score) = score {
                let mut detail = vec!["Fixed asset".to_string()];
                if !serial.is_empty() {
                    detail.push(format!("serial {serial}"));
                }
                detail.push(format!("in service {}", fmt::date(a.in_service_date)));
                out.push(Item {
                    icon: icon::PACKAGE,
                    title: b.asset_label(a.id),
                    detail: detail.join(" · "),
                    route: Route::Asset(a.id),
                    score,
                });
            }
        }
        for p in b.ledger.parties() {
            let display = p.display_name.as_deref().unwrap_or("");
            let exact = |t: &str| !t.is_empty() && t.trim().to_lowercase() == ql;
            let whole = |t: &str| !t.is_empty() && t.to_lowercase().contains(&ql);
            let score = if exact(&p.name) || exact(display) {
                Some(1500)
            } else {
                fuzzy(&format!("{} {display}", p.name), q).map(|s| {
                    s + if whole(&p.name) || whole(display) {
                        500
                    } else {
                        0
                    }
                })
            };
            if let Some(score) = score {
                let mut detail = vec![crate::pages::parties::roles_text(p)];
                if p.display_name.is_some() {
                    detail.push(p.name.clone());
                }
                if p.archived {
                    detail.push("archived".into());
                }
                out.push(Item {
                    icon: crate::pages::parties::party_icon(p),
                    title: p.label().to_string(),
                    detail: detail.join(" · "),
                    route: Route::Party(p.id),
                    score,
                });
            }
        }
        for st in &b.statements {
            let file = b
                .attachment(st.attachment_id)
                .map_or("", |a| a.file_name.as_str());
            let hay = format!(
                "statement {} {} {} {} {} {file}",
                b.account_label(st.account_id),
                fmt::date(st.period_start),
                fmt::date(st.period_end),
                st.period_start,
                st.period_end
            );
            if let Some(score) = fuzzy(&hay, q) {
                out.push(Item {
                    icon: icon::FILE_TEXT,
                    title: format!(
                        "Statement: {}, {} – {}",
                        b.account_name(st.account_id),
                        fmt::date_short(st.period_start),
                        fmt::date(st.period_end)
                    ),
                    detail: format!("Closing {} · {file}", fmt::dollars(st.closing_balance)),
                    route: Route::Document(st.attachment_id),
                    score,
                });
            }
        }
        for r in &b.reconciliations {
            let hay = format!(
                "reconciliation reconciled {} {} {}",
                b.account_label(r.account_id),
                fmt::date(r.statement_date),
                r.statement_date
            );
            if let Some(score) = fuzzy(&hay, q) {
                let mut detail = vec![
                    fmt::dollars(r.statement_balance),
                    fmt::count(r.cleared.len(), "cleared line", "cleared lines"),
                ];
                if r.voided.is_some() {
                    detail.push("voided".into());
                }
                out.push(Item {
                    icon: icon::CHECK_SQUARE,
                    title: format!(
                        "Reconciliation: {} through {}",
                        b.account_name(r.account_id),
                        fmt::date(r.statement_date)
                    ),
                    detail: detail.join(" · "),
                    route: Route::Reconciliation(r.id),
                    score,
                });
            }
        }
        for a in &b.attachments {
            let hay = format!("{} {}", a.file_name, a.description.as_deref().unwrap_or(""));
            if let Some(score) = fuzzy(&hay, q) {
                out.push(Item {
                    icon: icon::FILE,
                    title: a.file_name.clone(),
                    detail: format!(
                        "{} · {}",
                        fmt::label(&a.category),
                        a.description.as_deref().unwrap_or("")
                    ),
                    route: Route::Document(a.id),
                    score,
                });
            }
        }
    }
    out.sort_by_key(|i| std::cmp::Reverse(i.score));
    out.truncate(40);
    out
}

/// The breadcrumb for a route, outermost first: its section, any page it
/// sits under (a reconciliation under its account), then the route
/// itself. Each crumb is its text and where clicking it goes; `None` for
/// the page you're on, which isn't a link.
///
/// ```text
/// INVARIANTS:
///   1. The first crumb is the section's title, going to Section::home.
///   2. The last crumb is the current page: describe(r) when it says
///      anything, else the section itself (a section's own page).
///   3. Only the last crumb is None; a crumb whose route is the current
///      one is never a link.
/// ```
pub fn crumbs(b: Option<&Books>, r: &Route) -> Vec<(String, Option<Route>)> {
    let section = r.section();
    let here = b.map(|b| describe(b, r)).unwrap_or_default();
    let mut out = vec![(section.title().to_string(), Some(section.home()))];
    if let (Some(b), Route::Reconciliation(id)) = (b, r)
        && let Some(rec) = b.reconciliation(*id)
    {
        let account = Route::Account(rec.account_id);
        out.push((describe(b, &account), Some(account)));
    }
    if here.is_empty() {
        out.truncate(1);
    } else {
        out.push((here, None));
    }
    for c in &mut out {
        if c.1.as_ref() == Some(r) {
            c.1 = None;
        }
    }
    if let Some(last) = out.last_mut() {
        last.1 = None;
    }
    out
}

/// A short description of a route for the breadcrumb.
pub fn describe(b: &Books, r: &Route) -> String {
    match r {
        Route::Entry(id) => b
            .entry(*id)
            .map(|e| format!("#{} {}", e.id, e.description))
            .unwrap_or_default(),
        Route::Reports(k) => k.title().into(),
        Route::Account(id) => b.account_label(*id),
        Route::Document(id) => b
            .attachments
            .iter()
            .find(|a| a.id == *id)
            .map(|a| a.file_name.clone())
            .unwrap_or_default(),
        Route::Asset(id) => b.asset_label(*id),
        Route::Party(id) => b.party_label(*id),
        Route::Reconciliation(id) => b.subject_label(ledger_store::Subject::Reconciliation(*id)),
        Route::JournalEvent(seq) => format!("Event {seq}"),
        _ => String::new(),
    }
}
