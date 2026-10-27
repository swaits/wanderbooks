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
use ledger_core::arap::DocumentLine;
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
                if let Some(v) = &e.vendor
                    && e.parties.is_empty()
                {
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
            let several = e.parties.len() > 1;
            for share in &e.parties {
                let Some(party) = b.ledger.get_party(share.id) else {
                    continue;
                };
                ui.add_space(4.0);
                let mut text = format!("{} {}", super::parties::party_icon(party), party.label());
                let what = share_text(share, several, e.reversal_of);
                if !what.is_empty() {
                    text.push_str(&format!(" · {what}"));
                }
                if widgets::link(ui, RichText::new(text).color(p.accent))
                    .on_hover_text(format!(
                        "{} this entry is with ({}): open them",
                        if several {
                            "One of the parties"
                        } else {
                            "The party"
                        },
                        super::parties::roles_text(party).to_lowercase()
                    ))
                    .clicked()
                {
                    shell.go(Route::Party(party.id));
                }
            }
            if several && e.parties.iter().any(|x| x.resolved.is_none()) {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "{} The parties' shares don't add up to what this entry moved",
                        icon::WARNING
                    ))
                    .color(p.negative),
                );
            }
            if e.parties.is_empty()
                && let Some(why) = b.ledger.entry_no_party(e.id)
            {
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("{} No counterparty: {why}", icon::USER_MINUS))
                            .color(p.muted),
                    )
                    .wrap(),
                )
                .on_hover_text("Marked as having no counterparty (entry.no_party)");
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
                        RichText::new(fmt::dollars_or(e.amount)).font(theme::semibold(14.0)),
                        &fmt::plain(&fmt::dollars_or(e.amount)).unwrap_or_default(),
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
                    // None: past exact arithmetic (a draft line an earlier
                    // version journaled, of any size: rules.md B-28).
                    let (mut dr, mut cr) = (Some(Decimal::ZERO), Some(Decimal::ZERO));
                    for line in entry.lines() {
                        ui.horizontal(|ui| {
                            let w = ui.available_width() - amt_w * 2.0 - 16.0;
                            ui.allocate_ui(vec2(w, 22.0), |ui| {
                                ui.set_width(w);
                                ui.horizontal(|ui| {
                                    let tag = project_tag(line.class_id, &class_names);
                                    let (r, project) = name_and_tag(ui, tag.as_ref(), |ui| {
                                        widgets::link_truncated(
                                            ui,
                                            RichText::new(b.account_label(line.account))
                                                .font(theme::regular(13.5)),
                                        )
                                    });
                                    if r.clicked() {
                                        shell.go(Route::Account(line.account));
                                    }
                                    if let Some(cid) = project {
                                        shell.go(Route::Project(cid));
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
                                    dr = dr.and_then(|t| t.checked_add(line.amount.amount));
                                    (a, String::new())
                                }
                                Side::Credit => {
                                    cr = cr.and_then(|t| t.checked_add(line.amount.amount));
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
                        let totalled = dr.is_some() && cr.is_some();
                        let balanced = totalled && dr == cr;
                        let (r, _) = ui.allocate_exact_size(vec2(w, 22.0), egui::Sense::hover());
                        ui.painter().text(
                            r.left_center(),
                            egui::Align2::LEFT_CENTER,
                            if balanced {
                                format!("{} Balanced", icon::CHECK)
                            } else if totalled {
                                "Out of balance".into()
                            } else {
                                "Too large to total".into()
                            },
                            theme::regular(12.0),
                            if balanced { p.positive } else { p.negative },
                        );
                        widgets::amount_cell(ui, &fmt::money_or(dr), amt_w, p.text, true, false);
                        widgets::amount_cell(ui, &fmt::money_or(cr), amt_w, p.text, true, false);
                    });
                });
            ui.add_space(18.0);

            // The invoice or bill it records: its lines as the document
            // gives them, each project a link (rules.md B-37 item 8).
            if let Some(doc) = document_of(b, e.id) {
                section(ui, &doc.title);
                document_lines(ui, &doc, &class_names, total_w, amt_w, shell);
                ui.add_space(18.0);
            }

            // Receipts and documents: filed on it, linked to it, or linked
            // to the invoice, bill, or payroll run it records; a withdrawn
            // receipt filed on it stays, marked.
            let subjects = b.entry_subjects(e.id);
            let all_for_entry: Vec<&Attachment> = b
                .attachments
                .iter()
                .filter(|a| {
                    a.entry_id == Some(e.id)
                        || (a.withdrawn_at.is_none()
                            && b.links_of(a.id).iter().any(|s| subjects.contains(s)))
                })
                .collect();
            section(
                ui,
                if all_for_entry.len() == 1 {
                    "Receipt"
                } else {
                    "Receipts and documents"
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
                let elsewhere: Vec<ledger_store::Subject> = b
                    .links_of(a.id)
                    .iter()
                    .copied()
                    .filter(|s| *s != ledger_store::Subject::Entry(e.id))
                    .collect();
                if !elsewhere.is_empty() {
                    let mut go = None;
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                        ui.label(
                            RichText::new("Linked to")
                                .font(theme::regular(11.5))
                                .color(p.faint),
                        );
                        for s in elsewhere {
                            super::documents::subject_chip(ui, b, s, &mut go);
                        }
                    });
                    if let Some(r) = go {
                        shell.go(r);
                    }
                }
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

/// The invoice or bill an entry records: its name and due date, and its
/// lines.
pub struct DocumentOf<'a> {
    /// "Invoice 3, due Oct 1, 2026".
    pub title: String,
    /// Its lines, as journaled.
    pub lines: &'a [DocumentLine],
}

/// The invoice or bill whose entry is `entry`, if it records one.
pub fn document_of(b: &Books, entry: ledger_core::EntryId) -> Option<DocumentOf<'_>> {
    let due = |what: &str, id: u64, due: chrono::NaiveDate| {
        format!("{what} {id}, due {}", fmt::date(due))
    };
    b.ledger
        .invoices()
        .into_iter()
        .find(|i| i.entry_id == entry)
        .map(|i| DocumentOf {
            title: due("Invoice", i.id, i.due_date),
            lines: &i.lines,
        })
        .or_else(|| {
            b.ledger
                .bills()
                .into_iter()
                .find(|x| x.entry_id == entry)
                .map(|x| DocumentOf {
                    title: due("Bill", x.id, x.due_date),
                    lines: &x.lines,
                })
        })
}

/// A document's lines: each description, its project (a link that opens
/// it), and its amount.
fn document_lines(
    ui: &mut Ui,
    doc: &DocumentOf<'_>,
    class_names: &std::collections::HashMap<u32, String>,
    total_w: f32,
    amt_w: f32,
    shell: &mut Shell<'_>,
) {
    let p = pal(ui.ctx());
    egui::Frame::new()
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(total_w - 26.0);
            ui.horizontal(|ui| {
                let w = ui.available_width() - amt_w - 8.0;
                let (r, _) = ui.allocate_exact_size(vec2(w, 18.0), egui::Sense::hover());
                ui.painter().text(
                    r.left_center(),
                    egui::Align2::LEFT_CENTER,
                    "Line",
                    theme::regular(12.0),
                    p.muted,
                );
                widgets::amount_cell(ui, "Amount", amt_w, p.muted, false, false);
            });
            for line in doc.lines {
                ui.horizontal(|ui| {
                    let w = ui.available_width() - amt_w - 8.0;
                    ui.allocate_ui(vec2(w, 22.0), |ui| {
                        ui.set_width(w);
                        ui.horizontal(|ui| {
                            let tag = project_tag(line.class_id, class_names);
                            let (_, project) = name_and_tag(ui, tag.as_ref(), |ui| {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&line.description).font(theme::regular(13.5)),
                                    )
                                    .truncate(),
                                )
                            });
                            if let Some(cid) = project {
                                shell.go(Route::Project(cid));
                            }
                        });
                    });
                    widgets::amount_cell(ui, &fmt::money(line.amount), amt_w, p.text, false, false);
                });
            }
        });
}

/// A line's project tag: its id and the text it shows.
fn project_tag(
    class: Option<u32>,
    class_names: &std::collections::HashMap<u32, String>,
) -> Option<(u32, String)> {
    let c = class?;
    class_names
        .get(&c)
        .map(|n| (c, format!("{} {n}", icon::TAG)))
}

/// `name` (a truncating label), then `tag` as a link that opens its
/// project: the project, when clicked. The tag's width (at most half the
/// cell, cut with an ellipsis past it) is kept back from the name, so a
/// tagged line never grows past its cell.
fn name_and_tag(
    ui: &mut Ui,
    tag: Option<&(u32, String)>,
    name: impl FnOnce(&mut Ui) -> egui::Response,
) -> (egui::Response, Option<u32>) {
    let p = pal(ui.ctx());
    let avail = ui.available_width();
    let gap = ui.spacing().item_spacing.x;
    let tag_w = tag.map_or(0.0, |(_, t)| {
        ui.painter()
            .layout_no_wrap(t.clone(), theme::regular(11.5), p.muted)
            .size()
            .x
            .min(avail / 2.0)
    });
    let room = if tag.is_some() { tag_w + gap } else { 0.0 };
    let r = ui
        .allocate_ui(vec2((avail - room).max(0.0), 22.0), name)
        .inner;
    let go = tag.and_then(|(cid, t)| {
        ui.allocate_ui(vec2(tag_w, 22.0), |ui| {
            widgets::link_truncated(
                ui,
                RichText::new(t).font(theme::regular(11.5)).color(p.muted),
            )
        })
        .inner
        .on_hover_text("Open this project")
        .clicked()
        .then_some(*cid)
    });
    (r, go)
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
        Render::Text { text, markdown } => {
            let (text, markdown) = (text.clone(), *markdown);
            egui::Frame::new()
                .fill(p.surface)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(CornerRadius::same(6))
                .inner_margin(egui::Margin::same(14))
                .show(ui, |ui| {
                    ui.set_width(width - 30.0);
                    if markdown {
                        egui_commonmark::CommonMarkViewer::new()
                            .max_image_width(Some((width - 30.0) as usize))
                            .show(ui, &mut shell.receipts.markdown, &text);
                    } else {
                        ui.add(
                            egui::Label::new(
                                RichText::new(text).font(egui::FontId::monospace(12.0)),
                            )
                            .selectable(true)
                            .wrap(),
                        );
                    }
                });
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

/// A party's part of an entry in words (rules.md B-35): which way it
/// went and how much ("paid $64.00", "received $740.00, in kind"), what of
/// it is still owed, the share it came from when the entry names several
/// ("(50%)", "(derived)"), and a reversal said as one ("paid $1,480.00
/// (reversal of #12)"), so a property contribution and its reversal never
/// read alike. A part that moved nothing with the party (a bill, a close)
/// says so. Empty for a sole party whose part doesn't resolve.
pub fn share_text(
    s: &crate::books::EntryParty,
    several: bool,
    reversal_of: Option<ledger_core::EntryId>,
) -> String {
    use ledger_core::shares::Direction;
    let how = if several {
        match s.share {
            ledger_core::Share::Amount(_) => None,
            ledger_core::Share::Percent(p) => Some(format!("{}%", p.normalize())),
            ledger_core::Share::Derived => Some("derived".to_string()),
        }
    } else {
        None
    };
    let mut t = match (s.resolved, s.direction) {
        (Some(v), Some(d)) => {
            let amount = fmt::dollars(v.abs());
            let mut t = match d {
                Direction::Paid => format!("paid {amount}"),
                Direction::Received => format!("received {amount}"),
                Direction::Neither => format!("{amount}, no money moved"),
            };
            let cash = s.cash.unwrap_or_default();
            let in_kind = s.in_kind.unwrap_or_default();
            if !in_kind.is_zero() {
                if in_kind == v {
                    t.push_str(", in kind");
                } else {
                    t.push_str(&format!(", {} in kind", fmt::dollars(in_kind.abs())));
                }
            }
            let owed = v - cash - in_kind;
            if d != Direction::Neither && !owed.is_zero() {
                t.push_str(&format!(", {} still owed", fmt::dollars(owed.abs())));
            }
            t
        }
        _ if several => match s.share {
            ledger_core::Share::Amount(a) => fmt::dollars(a),
            _ => String::new(),
        },
        _ => String::new(),
    };
    let mut notes: Vec<String> = how.into_iter().collect();
    if s.resolved.is_some()
        && let Some(o) = reversal_of
    {
        notes.push(format!("reversal of #{o}"));
    }
    if !notes.is_empty() {
        if t.is_empty() {
            t = notes.join(", ");
        } else {
            t.push_str(&format!(" ({})", notes.join(", ")));
        }
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Build, d, find, old_books, render, render_at};

    /// A tagged line keeps its project tag inside its cell, in a narrow
    /// pane too: the name is cut to leave the tag room, so the amounts
    /// stay in their columns (an invoice's revenue line and its document
    /// line, each tagged; rules.md B-37 item 8).
    #[test]
    fn a_tagged_line_keeps_its_amounts_in_their_column() {
        use ledger_core::arap::DocumentLine;
        let mut b = Build::new("entry-tag-room");
        let project =
            b.s.add_project(
                "A project with a rather long name indeed",
                &ledger_core::ProjectFacts::default(),
            )
            .unwrap();
        b.s.add_account(
            "4000",
            "Revenue from design services and consulting",
            ledger_core::AccountType::Revenue,
            None,
            false,
        )
        .unwrap();
        let c = b.s.add_customer("Cedar").unwrap();
        let inv =
            b.s.create_invoice(
                c,
                d(2026, 3, 2),
                d(2026, 4, 1),
                vec![DocumentLine {
                    description: "Brand identity, website and a long tail of revisions".into(),
                    amount: Decimal::new(123_456, 2),
                    class_id: Some(project),
                    account: None,
                    quantity: None,
                    unit_price: None,
                }],
            )
            .unwrap();
        let entry = b.s.ledger().get_invoice(inv).unwrap().entry_id;
        let (_dir, books) = b.books();
        let texts = render_at(books, Route::Entry(entry), 1000.0);
        // The amount: the receivable's debit, the revenue's credit, both
        // totals, the document line's; every one ends at a column's edge,
        // the credits at one, the debits at another.
        let pane = find(&texts, "LINES").pos.x;
        let mut edges: Vec<i32> = texts
            .iter()
            .filter(|t| t.text == fmt::money(Decimal::new(123_456, 2)) && t.pos.x > pane)
            .map(|t| (t.pos.x + t.width).round() as i32)
            .collect();
        assert_eq!(edges.len(), 5, "{edges:?}");
        edges.sort_unstable();
        edges.dedup();
        assert_eq!(
            edges.len(),
            2,
            "the debit column and the credit column: {edges:?}"
        );
        let tag = format!("{} A project with a rather long name indeed", icon::TAG);
        assert_eq!(texts.iter().filter(|t| t.text == tag).count(), 2);
    }

    /// An entry's totals line: "Balanced" when its debits equal its
    /// credits, "Out of balance" when they don't, "Too large to total"
    /// when a side is past exact arithmetic (a draft an earlier version
    /// journaled); its totals right-aligned under the lines' columns.
    #[test]
    fn an_entrys_totals_say_whether_it_balances() {
        let mut b = Build::new("entry-totals");
        let (cash, capital, rent) = (b.cash, b.capital, b.rent);
        let _ = rent;
        let posted = b.post(d(2026, 1, 2), cash, capital, 100_000);
        let usd = |c: i64| ledger_core::Money::new(Decimal::new(c, 2), ledger_core::Currency::USD);
        let (_dir, books) = b.books();
        let (texts, p) = render(books, Route::Entry(posted), |_| {});
        assert_eq!(
            find(&texts, &format!("{} Balanced", icon::CHECK)).color,
            p.positive
        );
        // Each column's amounts end at one edge: the lines' and the total's.
        let mut edges: Vec<i32> = texts
            .iter()
            .filter(|t| t.text == fmt::money(Decimal::new(100_000, 2)))
            .map(|t| (t.pos.x + t.width).round() as i32)
            .collect();
        assert_eq!(edges.len(), 4, "two lines and two totals");
        edges.sort_unstable();
        edges.dedup();
        assert_eq!(edges.len(), 2, "two columns: {edges:?}");
        let mut b = Build::new("entry-unbalanced");
        let (cash, rent) = (b.cash, b.rent);
        let draft = b.s.create_entry(d(2026, 3, 1), "half").unwrap();
        b.s.add_line(draft, rent, Side::Debit, usd(1_000)).unwrap();
        b.s.add_line(draft, cash, Side::Credit, usd(500)).unwrap();
        let (_dir, books) = b.books();
        let (texts, p) = render(books, Route::Entry(draft), |_| {});
        assert_eq!(find(&texts, "Out of balance").color, p.negative);
        // Its debits past exact arithmetic, its credits $1.00.
        let (_dir, books) = old_books("x_draft_max_lines");
        let (texts, p) = render(books, Route::Entry(1), |_| {});
        assert_eq!(find(&texts, "Too large to total").color, p.negative);
    }
}
