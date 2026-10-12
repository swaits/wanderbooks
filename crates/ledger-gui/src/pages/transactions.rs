//! Every entry, searchable and filterable, with the selected one open
//! beside the list: its lines, its receipt rendered, its history.

use super::page;
use crate::app::Shell;
use crate::books::{Books, EntryInfo};
use crate::fmt;
use crate::nav::Route;
use crate::theme::{self, pal};
use crate::widgets;
use egui::{CornerRadius, RichText, Sense, Ui, vec2};
use egui_phosphor::regular as icon;
use ledger_core::{AccountId, EntryStatus};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Filter {
    #[default]
    All,
    Posted,
    Drafts,
    NoReceipt,
    WithReceipt,
}

#[derive(Default)]
pub struct State {
    pub query: String,
    pub filter: Filter,
    pub account: Option<AccountId>,
}

pub fn matches(e: &EntryInfo, st: &State) -> bool {
    let f = match st.filter {
        Filter::All => true,
        Filter::Posted => e.status == EntryStatus::Posted,
        Filter::Drafts => e.status == EntryStatus::Draft,
        Filter::NoReceipt => e.receipts.is_empty(),
        Filter::WithReceipt => !e.receipts.is_empty(),
    };
    let a = st.account.is_none_or(|a| e.accounts.contains(&a));
    let q = st.query.trim().to_lowercase();
    let t = q.is_empty() || q.split_whitespace().all(|w| e.haystack.contains(w));
    f && a && t
}

pub fn show(ui: &mut Ui, b: &Books, selected: Option<u64>, shell: &mut Shell<'_>) {
    match selected.and_then(|id| b.entry(id)) {
        Some(e) => {
            let w = (ui.available_width() * 0.5).max(420.0);
            super::split(
                ui,
                "transactions",
                w,
                shell,
                |ui, shell| list(ui, b, selected, shell),
                |ui, shell| super::entry::detail(ui, b, e, shell),
            );
        }
        None => list(ui, b, None, shell),
    }
}

fn list(ui: &mut Ui, b: &Books, selected: Option<u64>, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    page(ui, "transactions", |ui| {
        let total = b.entries.len();
        widgets::page_header(
            ui,
            "Transactions",
            &format!(
                "{} recorded, newest first. Click one to see its lines, its receipt, and its history.",
                fmt::count(total, "entry", "entries")
            ),
        );
        let st = &mut shell.state.transactions;
        let narrow = ui.available_width() < 720.0;
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut st.query)
                    .hint_text(format!(
                        "{} Search description, account, vendor, amount",
                        icon::MAGNIFYING_GLASS
                    ))
                    .desired_width(if narrow { ui.available_width() } else { 340.0 }),
            );
            if !narrow {
                filters(ui, b, st);
            }
        });
        if narrow {
            ui.add_space(6.0);
            ui.horizontal(|ui| filters(ui, b, st));
        }
        ui.add_space(12.0);
        let rows: Vec<&EntryInfo> = b.entries.iter().filter(|e| matches(e, st)).collect();
        if rows.is_empty() {
            widgets::card(ui, |ui| {
                widgets::empty(
                    ui,
                    icon::MAGNIFYING_GLASS,
                    "No matching entries",
                    "Try a different search or filter.",
                )
            });
            return;
        }
        widgets::card(ui, |ui| {
            let mut last_month: Option<(i32, u32)> = None;
            for e in rows {
                let m = (
                    chrono::Datelike::year(&e.date),
                    chrono::Datelike::month(&e.date),
                );
                if last_month != Some(m) {
                    if last_month.is_some() {
                        ui.add_space(10.0);
                    }
                    last_month = Some(m);
                    ui.label(
                        RichText::new(fmt::month(e.date))
                            .font(theme::semibold(12.5))
                            .color(p.muted),
                    );
                    ui.add_space(2.0);
                }
                if row(ui, b, e, selected == Some(e.id), shell.since_update).clicked() {
                    shell.go(Route::Entry(e.id));
                }
            }
        });
    });
}

fn row(ui: &mut Ui, b: &Books, e: &EntryInfo, on: bool, since_update: f64) -> egui::Response {
    let p = pal(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 50.0), Sense::click());
    let fresh = b.fresh.contains(&e.id) && since_update < 8.0;
    if on {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(8), p.selected);
    } else if fresh {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(8), p.new_highlight);
    } else if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(8), p.hover);
    }
    let painter = ui.painter_at(rect);
    let left = rect.left() + 10.0;
    // Date block.
    painter.text(
        egui::pos2(left, rect.center().y - 8.0),
        egui::Align2::LEFT_CENTER,
        e.date.format("%b").to_string().to_uppercase(),
        theme::semibold(10.5),
        p.faint,
    );
    painter.text(
        egui::pos2(left, rect.center().y + 8.0),
        egui::Align2::LEFT_CENTER,
        e.date.format("%-d").to_string(),
        theme::semibold(16.0),
        p.text,
    );
    // Description and accounts.
    let text_left = left + 44.0;
    let amount_w = 120.0;
    let max_w = (rect.right() - amount_w - 70.0 - text_left).max(60.0);
    let desc = painter.layout(e.description.clone(), theme::semibold(13.5), p.text, max_w);
    let desc = if desc.rows.len() > 1 {
        painter.layout_no_wrap(
            truncate(&e.description, (max_w / 7.5) as usize),
            theme::semibold(13.5),
            p.text,
        )
    } else {
        desc
    };
    painter.galley(egui::pos2(text_left, rect.center().y - 17.0), desc, p.text);
    let accounts: Vec<String> = e.accounts.iter().map(|a| b.account_name(*a)).collect();
    let mut sub = format!("#{} · {} · {}", e.id, e.origin, accounts.join(" → "));
    if let Some(v) = &e.party {
        sub.push_str(&format!(" · {v}"));
    }
    let sub = truncate(&sub, (max_w / 6.3) as usize);
    painter.text(
        egui::pos2(text_left, rect.center().y + 10.0),
        egui::Align2::LEFT_CENTER,
        sub,
        theme::regular(12.0),
        p.muted,
    );
    // Amount and badges.
    let right = rect.right() - 10.0;
    // Cash in reads green with a plus, cash out with a minus; entries that
    // don't move cash show their size, muted.
    let (amount, color) = if e.cash_effect > rust_decimal::Decimal::ZERO {
        (format!("+{}", fmt::money(e.cash_effect)), p.positive)
    } else if e.cash_effect < rust_decimal::Decimal::ZERO {
        (fmt::money(e.cash_effect), p.text)
    } else {
        (fmt::money(e.amount), p.muted)
    };
    painter.text(
        egui::pos2(right, rect.center().y - 6.0),
        egui::Align2::RIGHT_CENTER,
        amount,
        theme::semibold(14.0),
        color,
    );
    let mut badges: Vec<(&str, egui::Color32)> = Vec::new();
    if e.status == EntryStatus::Draft {
        badges.push(("DRAFT", p.warning));
    }
    if e.reversed_by.is_some() {
        badges.push(("REVERSED", p.muted));
    }
    if e.reversal_of.is_some() {
        badges.push(("REVERSAL", p.muted));
    }
    let mut x = right;
    for (text, color) in badges {
        let g = painter.layout_no_wrap(text.to_string(), theme::semibold(10.0), color);
        x -= g.size().x;
        painter.galley(egui::pos2(x, rect.center().y + 6.0), g, color);
        x -= 8.0;
    }
    let clip = if !e.receipts.is_empty() {
        Some((icon::PAPERCLIP, p.muted))
    } else if e.missing_receipt {
        Some((icon::WARNING_CIRCLE, p.negative))
    } else {
        None
    };
    if let Some((g, c)) = clip {
        painter.text(
            egui::pos2(rect.right() - amount_w - 24.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            g,
            egui::FontId::proportional(16.0),
            c,
        );
    }
    resp
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n.saturating_sub(1)).collect();
        format!("{t}…")
    }
}

fn filters(ui: &mut Ui, b: &Books, st: &mut State) {
    widgets::segmented(
        ui,
        &mut st.filter,
        &[
            (Filter::All, "All"),
            (Filter::Posted, "Posted"),
            (Filter::Drafts, "Drafts"),
            (Filter::WithReceipt, "With receipt"),
            (Filter::NoReceipt, "No receipt"),
        ],
    );
    if let Some(a) = st.account {
        ui.add_space(6.0);
        let label = format!("{} {}  {}", icon::FUNNEL, b.account_label(a), icon::X);
        if ui
            .button(label)
            .on_hover_text("Clear the account filter")
            .clicked()
        {
            st.account = None;
        }
    }
}
