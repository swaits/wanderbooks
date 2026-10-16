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
                        "{} Search description, account, party, amount",
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
    resp.context_menu(|ui| {
        widgets::copy_item(
            ui,
            "Copy amount",
            &fmt::plain(&fmt::money_or(e.amount)).unwrap_or_default(),
        );
        if !e.cash_effect.is_some_and(|c| c.is_zero()) {
            widgets::copy_item(
                ui,
                "Copy cash effect",
                &fmt::plain(&fmt::money_or(e.cash_effect)).unwrap_or_default(),
            );
        }
        widgets::copy_item(ui, "Copy entry number", &format!("#{}", e.id));
        widgets::copy_item(ui, "Copy description", &e.description);
    });
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
    // Description and accounts, cut with an ellipsis before the receipt
    // icon and the amount. Cash in reads green with a plus, cash out with
    // a minus; entries that don't move cash show their size, muted.
    let text_left = left + 44.0;
    let (amount, color) = match e.cash_effect {
        Some(c) if c > rust_decimal::Decimal::ZERO => (format!("+{}", fmt::money(c)), p.positive),
        Some(c) if c < rust_decimal::Decimal::ZERO => (fmt::money(c), p.text),
        Some(_) => (fmt::money_or(e.amount), p.muted),
        None => (fmt::CANT_COMPUTE.to_string(), p.muted),
    };
    let amount_w = painter
        .layout_no_wrap(amount.clone(), theme::semibold(14.0), color)
        .size()
        .x
        .max(64.0);
    let max_w = (rect.right() - 10.0 - amount_w - 44.0 - text_left).max(40.0);
    widgets::clipped_text(
        &painter,
        egui::pos2(text_left, rect.center().y - 8.0),
        &e.description,
        theme::semibold(13.5),
        p.text,
        max_w,
    );
    let accounts: Vec<String> = e.accounts.iter().map(|a| b.account_name(*a)).collect();
    // The party first: it's what's cut last when the row is narrow.
    let sub = match &e.party {
        Some(v) => format!("#{} · {} · {v} · {}", e.id, e.origin, accounts.join(" → ")),
        None => format!("#{} · {} · {}", e.id, e.origin, accounts.join(" → ")),
    };
    widgets::clipped_text(
        &painter,
        egui::pos2(text_left, rect.center().y + 10.0),
        &sub,
        theme::regular(12.0),
        p.muted,
        max_w,
    );
    // Amount and badges.
    let right = rect.right() - 10.0;
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
            egui::pos2(rect.right() - 10.0 - amount_w - 20.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            g,
            egui::FontId::proportional(16.0),
            c,
        );
    }
    resp
}

fn filters(ui: &mut Ui, b: &Books, st: &mut State) {
    let options = [
        (Filter::All, "All"),
        (Filter::Posted, "Posted"),
        (Filter::Drafts, "Drafts"),
        (Filter::WithReceipt, "With receipt"),
        (Filter::NoReceipt, "No receipt"),
    ];
    // The segmented control needs about 330 points; narrower, a dropdown.
    if ui.available_width() >= 340.0 {
        widgets::segmented(ui, &mut st.filter, &options);
    } else {
        let current = options.iter().find(|o| o.0 == st.filter).map(|o| o.1);
        egui::ComboBox::from_id_salt("txn_filter")
            .selected_text(format!("{} {}", icon::FUNNEL, current.unwrap_or("All")))
            .show_ui(ui, |ui| {
                for (f, label) in options {
                    ui.selectable_value(&mut st.filter, f, label);
                }
            });
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Build, d, find, harness, has, painted};

    /// A transaction row: cash in green with a plus, cash out with its
    /// minus, an entry moving no cash its size, muted; the description
    /// 44 points right of the date; and its menu offers the cash effect
    /// only when there is one.
    #[test]
    fn a_transaction_row_says_how_it_moved_cash() {
        let mut b = Build::new("transactions");
        let (cash, capital, rent) = (b.cash, b.capital, b.rent);
        b.post_as("Deposit in", d(2026, 10, 2), cash, capital, 100_000);
        b.post_as("Rent out", d(2026, 10, 3), rent, cash, 25_000);
        b.post_as("Owner paid rent", d(2026, 10, 4), rent, capital, 4_000);
        let (_dir, books) = b.books();
        let mut h = harness(books, Route::Transactions);
        let texts = painted(h.output());
        let p = crate::theme::pal(&h.ctx);
        let money = |c: i64| fmt::money(rust_decimal::Decimal::new(c, 2));
        assert_eq!(
            find(&texts, &format!("+{}", money(100_000))).color,
            p.positive
        );
        assert_eq!(find(&texts, &money(-25_000)).color, p.text);
        assert_eq!(find(&texts, &money(4_000)).color, p.muted);
        assert!(!has(&texts, &format!("+{}", money(-25_000))));
        // The description beside its day: 44 points right of the date.
        let desc = find(&texts, "Deposit in");
        let day = texts
            .iter()
            .filter(|t| t.text == "2" && (t.pos.y - desc.pos.y).abs() < 30.0)
            .min_by(|a, b| a.pos.x.total_cmp(&b.pos.x))
            .expect("the day");
        assert!(
            (desc.pos.x - day.pos.x - 44.0).abs() < 1.0,
            "{} vs {}",
            desc.pos.x,
            day.pos.x
        );
        // The menus: "Copy cash effect" for cash moved, none for none.
        for (row, offered) in [("Deposit in", true), ("Owner paid rent", false)] {
            let at = find(&painted(h.output()), row).pos + egui::vec2(4.0, 4.0);
            h.hover_at(at);
            h.run_steps(2);
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
                h.run_steps(2);
            }
            let menu = painted(h.output());
            let item = |label: &str| menu.iter().any(|t| t.text.ends_with(label));
            assert!(item("Copy entry number"), "{row}: no menu");
            assert_eq!(item("Copy cash effect"), offered, "{row}");
            h.key_press(egui::Key::Escape);
            h.run_steps(3);
        }
    }
}
