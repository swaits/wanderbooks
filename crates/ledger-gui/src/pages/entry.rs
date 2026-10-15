//! One entry, completely: what it is, every line, the receipt itself, and
//! every journal event that touched it.

use crate::app::Shell;
use crate::books::{Books, EntryInfo};
use crate::fmt;
use crate::nav::Route;
use crate::receipts::State as Render;
use crate::theme::{self, pal};
use crate::widgets;
use egui::{CornerRadius, Margin, RichText, Stroke, Ui, vec2};
use egui_phosphor::regular as icon;
use ledger_core::{EntryStatus, Side};
use ledger_store::Attachment;
use rust_decimal::Decimal;

pub fn detail(ui: &mut Ui, b: &Books, e: &EntryInfo, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    let Some(entry) = b.ledger.get_entry(e.id) else {
        return;
    };
    egui::ScrollArea::vertical()
        .id_salt(("entry", e.id))
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                widgets::copyable(
                    ui,
                    RichText::new(format!("Entry #{}", e.id))
                        .font(theme::regular(12.5))
                        .color(p.muted),
                    &format!("#{}", e.id),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(egui::Button::new(RichText::new(icon::X).size(15.0)).frame(false))
                        .on_hover_text("Close")
                        .clicked()
                    {
                        shell.go(Route::Transactions);
                    }
                });
            });
            ui.label(RichText::new(&e.description).font(theme::bold(21.0)));
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                match e.status {
                    EntryStatus::Posted => widgets::pill(ui, "Posted", p.positive, p.positive_soft),
                    EntryStatus::Draft => {
                        widgets::pill(ui, "Draft — not in any report", p.warning, p.warning_soft)
                    }
                };
                widgets::pill(ui, e.origin, p.muted, p.hover);
                if let Some(v) = &e.vendor {
                    widgets::pill(ui, &format!("{} {v}", icon::STOREFRONT), p.muted, p.hover);
                }
                if e.missing_receipt {
                    widgets::pill(ui, "Missing receipt", p.negative, p.negative_soft);
                }
            });
            if let Some(o) = e.reversal_of {
                ui.add_space(4.0);
                if widgets::link(
                    ui,
                    RichText::new(format!("{} Reverses #{o}", icon::ARROW_COUNTER_CLOCKWISE))
                        .color(p.accent),
                )
                .clicked()
                {
                    shell.go(Route::Entry(o));
                }
            }
            if let Some(r) = e.reversed_by {
                ui.add_space(4.0);
                if widgets::link(
                    ui,
                    RichText::new(format!(
                        "{} Reversed by #{r}",
                        icon::ARROW_COUNTER_CLOCKWISE
                    ))
                    .color(p.accent),
                )
                .clicked()
                {
                    shell.go(Route::Entry(r));
                }
            }
            if let Some(&asset) = b.entry_assets.get(&e.id) {
                ui.add_space(4.0);
                if widgets::link(
                    ui,
                    RichText::new(format!("{} {}", icon::PACKAGE, b.asset_label(asset)))
                        .color(p.accent),
                )
                .on_hover_text("Open this fixed asset")
                .clicked()
                {
                    shell.go(Route::Asset(asset));
                }
            }
            ui.add_space(16.0);

            // Facts.
            let created = e.events.first().map(|&i| fmt::timestamp(&b.journal[i].ts));
            // Posted by entry.posted, or in one step by whatever created it
            // (a schedule, a close, a bill).
            let posted = e
                .events
                .iter()
                .find(|&&i| b.journal[i].line_type == "entry.posted")
                .or_else(|| e.events.first())
                .map(|&i| fmt::timestamp(&b.journal[i].ts));
            egui::Grid::new(("facts", e.id))
                .num_columns(2)
                .spacing(vec2(24.0, 6.0))
                .show(ui, |ui| {
                    let k = |ui: &mut Ui, s: &str| {
                        ui.label(RichText::new(s).font(theme::regular(12.5)).color(p.muted))
                    };
                    k(ui, "Date");
                    ui.label(fmt::date(e.date));
                    ui.end_row();
                    k(ui, "Amount");
                    widgets::copyable(
                        ui,
                        RichText::new(fmt::dollars(e.amount)).font(theme::semibold(14.0)),
                        &fmt::plain(&fmt::dollars(e.amount)).unwrap_or_default(),
                    );
                    ui.end_row();
                    if let Some(c) = &created {
                        k(ui, "Recorded");
                        ui.add(egui::Label::new(c).wrap());
                        ui.end_row();
                    }
                    if e.status == EntryStatus::Posted
                        && let Some(t) = &posted
                    {
                        k(ui, "Posted");
                        ui.add(egui::Label::new(t).wrap());
                        ui.end_row();
                    }
                });
            ui.add_space(16.0);

            // Lines.
            section(ui, "Lines");
            let total_w = ui.available_width();
            let amt_w = ((total_w - 26.0) * 0.25).clamp(64.0, 104.0);
            let class_names: std::collections::HashMap<u32, String> = b
                .ledger
                .list_classes()
                .into_iter()
                .map(|c| (c.id, c.name.clone()))
                .collect();
            egui::Frame::new()
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_width(total_w - 26.0);
                    ui.horizontal(|ui| {
                        let w = ui.available_width() - amt_w * 2.0 - 16.0;
                        let (r, _) = ui.allocate_exact_size(vec2(w, 18.0), egui::Sense::hover());
                        ui.painter().text(
                            r.left_center(),
                            egui::Align2::LEFT_CENTER,
                            "Account",
                            theme::regular(12.0),
                            p.muted,
                        );
                        widgets::amount_cell(ui, "Debit", amt_w, p.muted, false, false);
                        widgets::amount_cell(ui, "Credit", amt_w, p.muted, false, false);
                    });
                    let (mut dr, mut cr) = (Decimal::ZERO, Decimal::ZERO);
                    for line in entry.lines() {
                        ui.horizontal(|ui| {
                            let w = ui.available_width() - amt_w * 2.0 - 16.0;
                            ui.allocate_ui(vec2(w, 22.0), |ui| {
                                ui.set_width(w);
                                ui.horizontal(|ui| {
                                    let r = widgets::link_truncated(
                                        ui,
                                        RichText::new(b.account_label(line.account))
                                            .font(theme::regular(13.5)),
                                    );
                                    if r.clicked() {
                                        shell.go(Route::Account(line.account));
                                    }
                                    if let Some(c) = line.class_id.and_then(|c| class_names.get(&c))
                                    {
                                        ui.label(
                                            RichText::new(format!("{} {c}", icon::TAG))
                                                .font(theme::regular(11.5))
                                                .color(p.muted),
                                        );
                                    }
                                    if let (Some(cur), Some(fa)) =
                                        (&line.foreign_currency, line.foreign_amount)
                                    {
                                        ui.label(
                                            RichText::new(format!(
                                                "{} {}",
                                                fmt::money(fa),
                                                cur.code()
                                            ))
                                            .font(theme::regular(11.5))
                                            .color(p.muted),
                                        );
                                    }
                                });
                            });
                            let a = fmt::money(line.amount.amount);
                            let (d, c) = match line.side {
                                Side::Debit => {
                                    dr += line.amount.amount;
                                    (a, String::new())
                                }
                                Side::Credit => {
                                    cr += line.amount.amount;
                                    (String::new(), a)
                                }
                            };
                            widgets::amount_cell(ui, &d, amt_w, p.text, false, false);
                            widgets::amount_cell(ui, &c, amt_w, p.text, false, false);
                        });
                    }
                    widgets::rule(ui);
                    ui.horizontal(|ui| {
                        let w = ui.available_width() - amt_w * 2.0 - 16.0;
                        let balanced = dr == cr;
                        let (r, _) = ui.allocate_exact_size(vec2(w, 22.0), egui::Sense::hover());
                        ui.painter().text(
                            r.left_center(),
                            egui::Align2::LEFT_CENTER,
                            if balanced {
                                format!("{} Balanced", icon::CHECK)
                            } else {
                                "Out of balance".into()
                            },
                            theme::regular(12.0),
                            if balanced { p.positive } else { p.negative },
                        );
                        widgets::amount_cell(ui, &fmt::money(dr), amt_w, p.text, true, false);
                        widgets::amount_cell(ui, &fmt::money(cr), amt_w, p.text, true, false);
                    });
                });
            ui.add_space(18.0);

            // Receipts.
            let all_for_entry: Vec<&Attachment> = b
                .attachments
                .iter()
                .filter(|a| a.entry_id == Some(e.id))
                .collect();
            section(
                ui,
                if all_for_entry.len() == 1 {
                    "Receipt"
                } else {
                    "Receipts"
                },
            );
            if all_for_entry.is_empty() {
                let msg = if e.missing_receipt {
                    format!("None attached, and {}.", b.receipt_rule())
                } else {
                    "None attached.".to_string()
                };
                ui.label(RichText::new(msg).color(if e.missing_receipt {
                    p.negative
                } else {
                    p.muted
                }));
            }
            for a in all_for_entry {
                attachment_view(ui, b, a, shell, true);
                ui.add_space(12.0);
            }
            ui.add_space(12.0);

            // History.
            section(ui, "History");
            for &i in &e.events {
                let line = &b.journal[i];
                let ev = super::journal::describe_any(b, line);
                ui.horizontal_top(|ui| {
                    ui.label(RichText::new(ev.icon).color(p.muted));
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let r = ui.add(
                            egui::Label::new(RichText::new(&ev.text).font(theme::regular(12.5)))
                                .wrap()
                                .sense(egui::Sense::click()),
                        );
                        if r.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if r.clicked() {
                            shell.go(Route::JournalEvent(line.seq));
                        }
                        ui.label(
                            RichText::new(format!(
                                "event {} · {}",
                                line.seq,
                                fmt::timestamp(&line.ts)
                            ))
                            .font(theme::regular(11.0))
                            .color(p.faint),
                        );
                    });
                });
                ui.add_space(4.0);
            }
        });
}

pub fn section(ui: &mut Ui, title: &str) {
    let p = pal(ui.ctx());
    ui.label(
        RichText::new(title.to_uppercase())
            .font(theme::semibold(11.5))
            .color(p.faint),
    );
    ui.add_space(6.0);
}

/// A receipt or document: its name and facts, then the file itself.
pub fn attachment_view(
    ui: &mut Ui,
    b: &Books,
    a: &Attachment,
    shell: &mut Shell<'_>,
    link_to_doc: bool,
) {
    let p = pal(ui.ctx());
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(if a.mime_type == "application/pdf" {
                icon::FILE_PDF
            } else {
                icon::FILE_IMAGE
            })
            .size(18.0)
            .color(p.muted),
        );
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            let name = RichText::new(&a.file_name).font(theme::semibold(13.0));
            if link_to_doc {
                if widgets::link(ui, name).clicked() {
                    shell.go(Route::Document(a.id));
                }
            } else {
                ui.label(name);
            }
            ui.label(
                RichText::new(format!(
                    "{} · {} · filed {}",
                    fmt::label(&a.category),
                    fmt::bytes(a.size_bytes),
                    fmt::timestamp(&a.created_at)
                ))
                .font(theme::regular(11.5))
                .color(p.muted),
            );
        });
    });
    if let Some(when) = &a.withdrawn_at {
        widgets::pill(
            ui,
            &format!(
                "Withdrawn {}: {}",
                fmt::timestamp(when),
                a.withdrawn_reason.as_deref().unwrap_or("")
            ),
            p.warning,
            p.warning_soft,
        );
    }
    if let Some(d) = &a.description {
        ui.label(RichText::new(d).font(theme::regular(12.5)).color(p.muted));
    }
    ui.add_space(6.0);
    render(ui, b, a, shell, ui.available_width());
}

/// The file itself, pages stacked, scaled to `width`.
pub fn render(ui: &mut Ui, b: &Books, a: &Attachment, shell: &mut Shell<'_>, width: f32) {
    let p = pal(ui.ctx());
    match shell.receipts.get(&a.blob_sha, &a.mime_type, b.blob_root()) {
        Render::Loading => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Rendering…").color(p.muted));
            });
        }
        Render::Failed(err) => {
            ui.label(
                RichText::new(format!("{} Can't show this file: {err}", icon::WARNING))
                    .color(p.negative),
            );
        }
        Render::Ready(pages) => {
            for (i, tex) in pages.iter().enumerate() {
                let size = tex.size_vec2();
                let scale = (width / size.x).min(1.0);
                let img = egui::Image::new(tex)
                    .fit_to_exact_size(size * scale)
                    .corner_radius(CornerRadius::same(4));
                egui::Frame::new()
                    .stroke(Stroke::new(1.0, p.border))
                    .corner_radius(CornerRadius::same(6))
                    .show(ui, |ui| {
                        ui.add(img);
                    });
                if pages.len() > 1 {
                    ui.label(
                        RichText::new(format!("Page {} of {}", i + 1, pages.len()))
                            .font(theme::regular(11.0))
                            .color(p.faint),
                    );
                }
                ui.add_space(6.0);
            }
        }
    }
}
