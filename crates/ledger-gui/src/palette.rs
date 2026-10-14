//! Ctrl+K: jump to anything. Pages, reports, accounts, entries (by text,
//! amount, or #id), vendors, and documents, ranked by a simple fuzzy match.

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

struct Item {
    icon: &'static str,
    title: String,
    detail: String,
    route: Route,
    score: i32,
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
                    .hint_text("Search entries, accounts, vendors, documents, reports…")
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
///      more, so a word-start prefix scores at least as high as any
///      other match of the same query.
/// ```
fn fuzzy(hay: &str, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let hay: Vec<char> = hay.to_lowercase().chars().collect();
    let mut score = 0;
    let mut hi = 0;
    let mut prev_match = false;
    for qc in query.to_lowercase().chars() {
        if qc == ' ' {
            prev_match = false;
            continue;
        }
        let mut found = false;
        while hi < hay.len() {
            let c = hay[hi];
            hi += 1;
            if c == qc {
                score += if prev_match { 6 } else { 1 };
                if hi == 1 || !hay[hi - 2].is_alphanumeric() {
                    score += 4;
                }
                prev_match = true;
                found = true;
                break;
            }
            prev_match = false;
        }
        if !found {
            return None;
        }
    }
    Some(score)
}

fn search(b: &Books, query: &str) -> Vec<Item> {
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
                    detail: format!("{} · {}", fmt::date(e.date), fmt::dollars(e.amount)),
                    route: Route::Entry(e.id),
                    score: score + bonus,
                });
            }
        }
        for v in b.ledger.list_vendors() {
            if let Some(score) = fuzzy(&v.name, q) {
                out.push(Item {
                    icon: icon::STOREFRONT,
                    title: v.name.clone(),
                    detail: "Vendor".into(),
                    route: Route::Company,
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
        Route::JournalEvent(seq) => format!("Event {seq}"),
        _ => String::new(),
    }
}
