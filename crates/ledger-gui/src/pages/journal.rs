//! The journal: every event, in order, as recorded. The audit trail an
//! accountant wants and the raw truth everything else is built from.

use super::page;
use crate::app::Shell;
use crate::books::{Books, entry_refs};
use crate::fmt;
use crate::nav::Route;
use crate::theme::{self, pal};
use crate::widgets;
use egui::{CornerRadius, Margin, RichText, Sense, Ui, vec2};
use egui_phosphor::regular as icon;
use ledger_store::JournalLine;
use serde_json::Value;

#[derive(Default)]
pub struct State {
    pub query: String,
    pub hide_steps: bool,
}

pub struct Event {
    pub icon: &'static str,
    pub text: String,
    pub route: Option<Route>,
}

fn s<'a>(p: &'a Value, k: &str) -> &'a str {
    p.get(k).and_then(|v| v.as_str()).unwrap_or("")
}

fn amount(p: &Value) -> String {
    p.get("amount")
        .and_then(|v| v.as_str())
        .and_then(|a| a.parse().ok())
        .map(fmt::dollars)
        .unwrap_or_default()
}

fn entry_text(b: &Books, id: u64) -> String {
    b.entry(id)
        .map(|e| format!("#{} {} ({})", e.id, e.description, fmt::dollars(e.amount)))
        .unwrap_or_else(|| format!("#{id}"))
}

fn attachment_name(b: &Books, id: u64) -> String {
    b.attachments
        .iter()
        .find(|a| a.id == id)
        .map(|a| a.file_name.clone())
        .unwrap_or_else(|| format!("attachment {id}"))
}

/// A sentence for a meaningful event; `None` for the intermediate steps of
/// building an entry (created, line added), which the posted event covers.
pub fn describe(b: &Books, line: &JournalLine) -> Option<Event> {
    let p = &line.payload;
    let id = p.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
    let entry_id = p.get("entry_id").and_then(|v| v.as_u64());
    let att = p.get("attachment_id").and_then(|v| v.as_u64()).unwrap_or(0);
    let ev = |icon, text: String, route: Option<Route>| Some(Event { icon, text, route });
    match line.line_type.as_str() {
        "entry.created" | "entry.line_added" | "entry.line_class_set" => None,
        "store.created" => ev(icon::BOOK_OPEN, "Books created".into(), None),
        "entry.posted" => ev(
            icon::CHECK_CIRCLE,
            format!("Posted {}", entry_text(b, id)),
            Some(Route::Entry(id)),
        ),
        "entry.reversed" => ev(
            icon::ARROW_COUNTER_CLOCKWISE,
            format!(
                "Reversed #{} with #{id}",
                p.get("reverses").and_then(|v| v.as_u64()).unwrap_or(0)
            ),
            Some(Route::Entry(id)),
        ),
        "receipt.attached" => ev(
            icon::PAPERCLIP,
            match entry_id {
                Some(e) => format!("Attached {} to #{e}", s(p, "file_name")),
                None => format!("Filed receipt {}", s(p, "file_name")),
            },
            Some(Route::Document(att)),
        ),
        "document.filed" => ev(
            icon::FILE_ARROW_UP,
            format!(
                "Filed {} ({})",
                s(p, "file_name"),
                fmt::label(s(p, "category"))
            ),
            Some(Route::Document(att)),
        ),
        "receipt.linked" => ev(
            icon::LINK,
            format!(
                "Linked {} to #{}",
                attachment_name(b, att),
                entry_id.unwrap_or(0)
            ),
            entry_id.map(Route::Entry),
        ),
        "receipt.unlinked" => ev(
            icon::LINK_BREAK,
            format!(
                "Unlinked {} from #{}: {}",
                attachment_name(b, att),
                entry_id.unwrap_or(0),
                s(p, "reason")
            ),
            entry_id.map(Route::Entry),
        ),
        "attachment.withdrawn" => ev(
            icon::ARCHIVE,
            format!("Withdrew {}: {}", attachment_name(b, att), s(p, "reason")),
            Some(Route::Document(att)),
        ),
        "account.added" => ev(
            icon::PLUS_CIRCLE,
            format!("Added account {} {}", s(p, "code"), s(p, "name")),
            Some(Route::Account(id)),
        ),
        "account.tax_set" => ev(
            icon::TAG,
            format!(
                "Tax category of {}: {}",
                b.account_label(p.get("account_id").and_then(|v| v.as_u64()).unwrap_or(0)),
                p.get("tax_category")
                    .and_then(|v| v.as_str())
                    .unwrap_or("none")
            ),
            p.get("account_id")
                .and_then(|v| v.as_u64())
                .map(Route::Account),
        ),
        "vendor.added" => ev(
            icon::STOREFRONT,
            format!("Added vendor {}", s(p, "name")),
            Some(Route::Company),
        ),
        "customer.added" => ev(
            icon::USER_PLUS,
            format!("Added customer {}", s(p, "name")),
            Some(Route::Company),
        ),
        "class.added" => ev(icon::TAG, format!("Added class {}", s(p, "name")), None),
        "vendor.1099_set" => ev(
            icon::IDENTIFICATION_CARD,
            "Updated a vendor's 1099 details".into(),
            Some(Route::Company),
        ),
        "entry.vendor_set" => ev(
            icon::STOREFRONT,
            format!("Tagged #{} with a vendor", entry_id.unwrap_or(0)),
            entry_id.map(Route::Entry),
        ),
        "invoice.created" => ev(
            icon::INVOICE,
            format!("Invoice {id} created"),
            entry_id.map(Route::Entry),
        ),
        "bill.created" => ev(
            icon::RECEIPT,
            format!("Bill {id} recorded"),
            entry_id.map(Route::Entry),
        ),
        "invoice.payment_received" => ev(
            icon::HAND_COINS,
            format!("Payment received, {}", amount(p)),
            entry_id.map(Route::Entry),
        ),
        "bill.paid" => ev(
            icon::MONEY,
            format!("Bill paid, {}", amount(p)),
            entry_id.map(Route::Entry),
        ),
        "period.created" => ev(
            icon::CALENDAR_PLUS,
            format!("Period {} created", s(p, "name")),
            Some(Route::Company),
        ),
        "period.closed" => ev(
            icon::LOCK,
            "Period closed".into(),
            p.get("closing_entry")
                .and_then(|v| v.as_u64())
                .map(Route::Entry),
        ),
        "asset.registered" => ev(
            icon::PACKAGE,
            format!("Registered asset {}", s(p, "description")),
            Some(Route::Company),
        ),
        "asset.depreciation_posted" => ev(
            icon::TREND_DOWN,
            "Depreciation posted".into(),
            entry_id.map(Route::Entry),
        ),
        "asset.disposed" => ev(
            icon::TRASH,
            "Asset disposed".into(),
            entry_id.map(Route::Entry),
        ),
        "schedule.registered" => ev(
            icon::CALENDAR,
            format!("Schedule: {}", s(p, "description")),
            Some(Route::Company),
        ),
        "schedule.posted" => ev(
            icon::CALENDAR_CHECK,
            "Amortization posted".into(),
            p.get("entry")
                .and_then(|e| e.get("id"))
                .and_then(|v| v.as_u64())
                .map(Route::Entry),
        ),
        "recurring.registered" => ev(
            icon::REPEAT,
            format!("Recurring: {}", s(p, "description")),
            Some(Route::Company),
        ),
        "journal.tail_repaired" => ev(
            icon::WARNING,
            format!("Cut-off journal line set aside ({} bytes)", p["bytes"]),
            None,
        ),
        "recurring.generated" => ev(
            icon::REPEAT,
            "Recurring draft created".into(),
            entry_id.map(Route::Entry),
        ),
        "fx.rate_set" => ev(
            icon::CURRENCY_CIRCLE_DOLLAR,
            format!("{} rate set", s(p, "currency")),
            None,
        ),
        "payroll.run" => ev(
            icon::USERS,
            "Payroll run".into(),
            entry_id.map(Route::Entry),
        ),
        "line.cleared" => ev(
            icon::CHECK_SQUARE,
            format!("Cleared a line of #{}", entry_id.unwrap_or(0)),
            entry_id.map(Route::Entry),
        ),
        other => ev(
            icon::DOT,
            other.to_string(),
            entry_refs(line).first().copied().map(Route::Entry),
        ),
    }
}

/// Like [`describe`], but every event gets a sentence.
pub fn describe_any(b: &Books, line: &JournalLine) -> Event {
    if let Some(e) = describe(b, line) {
        return e;
    }
    let p = &line.payload;
    let id = p
        .get("id")
        .and_then(|v| v.as_u64())
        .or_else(|| p.get("entry_id").and_then(|v| v.as_u64()))
        .unwrap_or(0);
    match line.line_type.as_str() {
        "entry.created" => Event {
            icon: icon::PENCIL_SIMPLE,
            text: format!("Drafted #{id} {}", s(p, "description")),
            route: Some(Route::Entry(id)),
        },
        "entry.line_added" => {
            let amount = p
                .get("amount")
                .and_then(|a| a.get("amount").or(Some(a)))
                .and_then(|v| v.as_str())
                .and_then(|v| v.parse().ok())
                .map(fmt::money)
                .unwrap_or_default();
            Event {
                icon: icon::PLUS,
                text: format!(
                    "Line on #{id}: {} {} {amount}",
                    s(p, "side"),
                    b.account_label(
                        p.get("account_id")
                            .or_else(|| p.get("account"))
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0)
                    ),
                ),
                route: Some(Route::Entry(id)),
            }
        }
        "entry.line_class_set" => {
            let class = p
                .get("class_id")
                .and_then(|v| v.as_u64())
                .and_then(|c| b.ledger.get_class(c as u32))
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "no class".into());
            Event {
                icon: icon::TAG,
                text: format!(
                    "Line {} of #{id} tagged {class}",
                    p.get("line_index")
                        .and_then(|v| v.as_u64())
                        .map_or(0, |i| i + 1)
                ),
                route: Some(Route::Entry(id)),
            }
        }
        t => Event {
            icon: icon::DOT,
            text: t.to_string(),
            route: None,
        },
    }
}

pub fn show(ui: &mut Ui, b: &Books, selected: Option<u64>, shell: &mut Shell<'_>) {
    if let Some(seq) = selected
        && let Some(line) = b.journal.get(seq.saturating_sub(1) as usize)
    {
        super::split(
            ui,
            "journal",
            480.0,
            shell,
            |ui, shell| list(ui, b, selected, shell),
            |ui, shell| detail(ui, b, line, shell),
        );
    } else {
        list(ui, b, None, shell);
    }
}

fn list(ui: &mut Ui, b: &Books, selected: Option<u64>, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    page(ui, "journal", |ui| {
        widgets::page_header(
            ui,
            "Journal",
            &format!(
                "Every change to the books, in order: {} events, each hash-chained to the one before. Nothing is ever edited or deleted; corrections are new events.",
                b.journal.len()
            ),
        );
        let st = &mut shell.state.journal;
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut st.query)
                    .hint_text(format!("{} Filter events", icon::MAGNIFYING_GLASS))
                    .desired_width(ui.available_width().min(320.0)),
            );
            ui.checkbox(&mut st.hide_steps, "Hide entry-building steps");
        });
        ui.add_space(10.0);
        let q = st.query.to_lowercase();
        let hide = st.hide_steps;
        let rows: Vec<(&JournalLine, Event)> = b
            .journal
            .iter()
            .rev()
            .filter(|l| !(hide && describe(b, l).is_none()))
            .map(|l| (l, describe_any(b, l)))
            .filter(|(l, e)| {
                q.is_empty() || e.text.to_lowercase().contains(&q) || l.line_type.contains(&q)
            })
            .collect();
        widgets::card(ui, |ui| {
            let row_h = 34.0;
            egui::ScrollArea::vertical()
                .id_salt("journal_rows")
                .max_height(ui.available_height().max(400.0))
                .show_rows(ui, row_h, rows.len(), |ui, range| {
                    for (line, ev) in &rows[range] {
                        let (rect, resp) = ui
                            .allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
                        let on = selected == Some(line.seq);
                        let fresh = line.seq > b.fresh_from_seq && shell.since_update < 8.0;
                        if on {
                            ui.painter()
                                .rect_filled(rect, CornerRadius::same(6), p.selected);
                        } else if fresh {
                            ui.painter()
                                .rect_filled(rect, CornerRadius::same(6), p.new_highlight);
                        } else if resp.hovered() {
                            ui.painter()
                                .rect_filled(rect, CornerRadius::same(6), p.hover);
                        }
                        let painter = ui.painter_at(rect);
                        let y = rect.center().y;
                        painter.text(
                            egui::pos2(rect.left() + 8.0, y),
                            egui::Align2::LEFT_CENTER,
                            format!("{}", line.seq),
                            theme::regular(12.0),
                            p.faint,
                        );
                        painter.text(
                            egui::pos2(rect.left() + 56.0, y),
                            egui::Align2::LEFT_CENTER,
                            ev.icon,
                            egui::FontId::proportional(15.0),
                            p.muted,
                        );
                        // The timestamp yields to the event when it's tight.
                        let ts = painter.layout_no_wrap(
                            fmt::timestamp(&line.ts),
                            theme::regular(12.0),
                            p.faint,
                        );
                        let ts_w = if rect.width() - 80.0 - ts.size().x - 24.0 >= 200.0 {
                            let w = ts.size().x;
                            painter.galley(
                                egui::pos2(rect.right() - 8.0 - w, y - ts.size().y / 2.0),
                                ts,
                                p.faint,
                            );
                            w + 24.0
                        } else {
                            8.0
                        };
                        widgets::clipped_text(
                            &painter,
                            egui::pos2(rect.left() + 80.0, y),
                            &ev.text,
                            theme::regular(13.0),
                            p.text,
                            rect.width() - 80.0 - ts_w,
                        );
                        if resp.clicked() {
                            shell.go(Route::JournalEvent(line.seq));
                        }
                    }
                });
        });
    });
}

fn detail(ui: &mut Ui, b: &Books, line: &JournalLine, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    let ev = describe_any(b, line);
    egui::ScrollArea::vertical()
        .id_salt("event_detail")
        .show(ui, |ui| {
            ui.label(
                RichText::new(format!("Event {}", line.seq))
                    .font(theme::regular(12.5))
                    .color(p.muted),
            );
            ui.label(RichText::new(&ev.text).font(theme::semibold(17.0)));
            ui.add_space(10.0);
            let fact = |ui: &mut Ui, k: &str, v: &str| {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        vec2(90.0, 18.0),
                        egui::Label::new(
                            RichText::new(k).font(theme::regular(12.5)).color(p.muted),
                        ),
                    );
                    ui.add(egui::Label::new(RichText::new(v).font(theme::regular(12.5))).wrap());
                });
            };
            fact(ui, "Type", &line.line_type);
            fact(ui, "Recorded", &fmt::timestamp(&line.ts));
            fact(ui, "Previous", &line.prev);
            let own = ledger_store::journal::sha256_hex(line.to_canonical().as_bytes());
            fact(ui, "This hash", &own);
            if let Some(r) = &ev.route {
                ui.add_space(6.0);
                if ui
                    .button(format!(
                        "{} Open {}",
                        icon::ARROW_SQUARE_OUT,
                        crate::palette::describe(b, r)
                    ))
                    .clicked()
                {
                    shell.go(r.clone());
                }
            }
            ui.add_space(12.0);
            ui.label(RichText::new("Payload").font(theme::semibold(13.0)));
            ui.add_space(4.0);
            let json = serde_json::to_string_pretty(&line.payload).unwrap_or_default();
            egui::Frame::new()
                .fill(p.hover)
                .corner_radius(CornerRadius::same(8))
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(egui::Label::new(RichText::new(json).monospace().size(12.0)).wrap());
                });
        });
}
