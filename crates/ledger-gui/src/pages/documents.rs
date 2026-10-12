//! Every receipt and document the books hold, rendered: what it is, what
//! entry it proves, and whether it was withdrawn (and why).

use super::page;
use crate::app::Shell;
use crate::books::Books;
use crate::fmt;
use crate::nav::Route;
use crate::receipts::State as Render;
use crate::theme::{self, pal};
use crate::widgets;
use egui::{CornerRadius, RichText, Sense, Stroke, Ui, vec2};
use egui_phosphor::regular as icon;
use ledger_store::Attachment;

#[derive(Default)]
pub struct State {
    pub category: Option<String>,
    pub show_withdrawn: bool,
    pub query: String,
}

pub fn show(ui: &mut Ui, b: &Books, selected: Option<u64>, shell: &mut Shell<'_>) {
    match selected.and_then(|id| b.attachments.iter().find(|a| a.id == id)) {
        Some(a) => super::split(
            ui,
            "documents",
            (ui.available_width() * 0.5).max(460.0),
            shell,
            |ui, shell| grid(ui, b, Some(a.id), shell),
            |ui, shell| detail(ui, b, a, shell),
        ),
        None => grid(ui, b, None, shell),
    }
}

fn grid(ui: &mut Ui, b: &Books, selected: Option<u64>, shell: &mut Shell<'_>) {
    page(ui, "documents", |ui| {
        let current = b
            .attachments
            .iter()
            .filter(|a| a.withdrawn_at.is_none())
            .count();
        widgets::page_header(
            ui,
            "Documents",
            &format!(
                "{} on file: receipts that prove entries, and the company's records. Bytes are content-addressed and verified on every read.",
                fmt::count(current, "document", "documents")
            ),
        );
        let mut cats: Vec<String> = b.attachments.iter().map(|a| a.category.clone()).collect();
        cats.sort();
        cats.dedup();
        let st = &mut shell.state.documents;
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut st.query)
                    .hint_text(format!("{} Search file names", icon::MAGNIFYING_GLASS))
                    .desired_width(260.0),
            );
            if ui.selectable_label(st.category.is_none(), "All").clicked() {
                st.category = None;
            }
            for c in &cats {
                if ui
                    .selectable_label(st.category.as_deref() == Some(c.as_str()), fmt::label(c))
                    .clicked()
                {
                    st.category = Some(c.clone());
                }
            }
            ui.checkbox(&mut st.show_withdrawn, "Show withdrawn");
        });
        ui.add_space(12.0);
        let q = st.query.to_lowercase();
        let docs: Vec<&Attachment> = b
            .attachments
            .iter()
            .rev()
            .filter(|a| st.show_withdrawn || a.withdrawn_at.is_none())
            .filter(|a| st.category.as_ref().is_none_or(|c| &a.category == c))
            .filter(|a| {
                q.is_empty()
                    || a.file_name.to_lowercase().contains(&q)
                    || a.description
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&q)
            })
            .collect();
        if docs.is_empty() {
            widgets::card(ui, |ui| widgets::empty(ui, icon::FILES, "No documents", ""));
            return;
        }
        let tile_w = 200.0;
        let per_row = ((ui.available_width() + 16.0) / (tile_w + 16.0))
            .floor()
            .max(1.0) as usize;
        let mut go = None;
        for row in docs.chunks(per_row) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                for a in row {
                    if tile(ui, b, a, tile_w, selected == Some(a.id), shell).clicked() {
                        go = Some(a.id);
                    }
                }
            });
            ui.add_space(16.0);
        }
        if let Some(id) = go {
            shell.go(Route::Document(id));
        }
    });
}

fn tile(
    ui: &mut Ui,
    b: &Books,
    a: &Attachment,
    w: f32,
    on: bool,
    shell: &mut Shell<'_>,
) -> egui::Response {
    let p = pal(ui.ctx());
    let h = 250.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    let fill = if on { p.selected } else { p.surface };
    ui.painter().rect(
        rect,
        CornerRadius::same(10),
        fill,
        Stroke::new(1.0, if resp.hovered() { p.accent } else { p.border }),
        egui::StrokeKind::Inside,
    );
    let thumb = egui::Rect::from_min_size(rect.min + vec2(10.0, 10.0), vec2(w - 20.0, 170.0));
    ui.painter()
        .rect_filled(thumb, CornerRadius::same(6), p.hover);
    match shell.receipts.get(&a.blob_sha, &a.mime_type, b.blob_root()) {
        Render::Ready(pages) => {
            if let Some(tex) = pages.first() {
                // Top of the first page, filling the width.
                let size = tex.size_vec2();
                let scale = thumb.width() / size.x;
                let visible_h = (thumb.height() / scale).min(size.y);
                let uv = egui::Rect::from_min_max(
                    egui::pos2(0.0, 0.0),
                    egui::pos2(1.0, visible_h / size.y),
                );
                let img_rect =
                    egui::Rect::from_min_size(thumb.min, vec2(thumb.width(), visible_h * scale));
                ui.painter()
                    .image(tex.id(), img_rect, uv, egui::Color32::WHITE);
            }
        }
        Render::Loading => {
            ui.painter().text(
                thumb.center(),
                egui::Align2::CENTER_CENTER,
                "…",
                theme::regular(16.0),
                p.faint,
            );
        }
        Render::Failed(_) => {
            ui.painter().text(
                thumb.center(),
                egui::Align2::CENTER_CENTER,
                icon::FILE,
                egui::FontId::proportional(28.0),
                p.faint,
            );
        }
    }
    // Title: what it proves, else what it is.
    let title = a
        .entry_id
        .and_then(|e| b.entry(e))
        .map(|e| e.description.clone())
        .or_else(|| a.description.clone())
        .unwrap_or_else(|| a.file_name.clone());
    let max_chars = ((w - 20.0) / 7.2) as usize;
    let title = if title.chars().count() > max_chars {
        title.chars().take(max_chars - 1).collect::<String>() + "…"
    } else {
        title
    };
    ui.painter().text(
        egui::pos2(rect.left() + 10.0, thumb.bottom() + 18.0),
        egui::Align2::LEFT_CENTER,
        title,
        theme::semibold(12.5),
        p.text,
    );
    let sub = match (a.entry_id, &a.withdrawn_at) {
        (_, Some(_)) => format!("Withdrawn · {}", a.file_name),
        (Some(e), None) => match b.entry(e) {
            Some(x) => format!(
                "#{e} · {} · {}",
                fmt::date_short(x.date),
                fmt::dollars(x.amount)
            ),
            None => format!("#{e}"),
        },
        (None, None) => format!("{} · {}", fmt::label(&a.category), a.file_name),
    };
    let sub = if sub.chars().count() > max_chars + 4 {
        sub.chars().take(max_chars + 3).collect::<String>() + "…"
    } else {
        sub
    };
    ui.painter().text(
        egui::pos2(rect.left() + 10.0, thumb.bottom() + 40.0),
        egui::Align2::LEFT_CENTER,
        sub,
        theme::regular(11.5),
        if a.withdrawn_at.is_some() {
            p.warning
        } else {
            p.muted
        },
    );
    resp
}

fn detail(ui: &mut Ui, b: &Books, a: &Attachment, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    egui::ScrollArea::vertical()
        .id_salt(("doc", a.id))
        .auto_shrink(false)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("Document {}", a.id))
                        .font(theme::regular(12.5))
                        .color(p.muted),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(egui::Button::new(RichText::new(icon::X).size(15.0)).frame(false))
                        .clicked()
                    {
                        shell.go(Route::Documents);
                    }
                });
            });
            ui.label(RichText::new(&a.file_name).font(theme::bold(20.0)));
            ui.add_space(8.0);
            egui::Grid::new(("docfacts", a.id))
                .num_columns(2)
                .spacing(vec2(20.0, 6.0))
                .show(ui, |ui| {
                    let k = |ui: &mut Ui, s: &str| {
                        ui.label(RichText::new(s).font(theme::regular(12.5)).color(p.muted))
                    };
                    k(ui, "Category");
                    ui.label(fmt::label(&a.category));
                    ui.end_row();
                    k(ui, "Type");
                    ui.label(format!("{} · {}", a.mime_type, fmt::bytes(a.size_bytes)));
                    ui.end_row();
                    k(ui, "Filed");
                    ui.label(fmt::timestamp(&a.created_at));
                    ui.end_row();
                    k(ui, "SHA-256");
                    ui.add(
                        egui::Label::new(RichText::new(&a.blob_sha).monospace().size(11.5)).wrap(),
                    );
                    ui.end_row();
                    if let Some(e) = a.entry_id {
                        k(ui, "Proves");
                        let label = b
                            .entry(e)
                            .map(|x| {
                                format!("#{} {} ({})", x.id, x.description, fmt::dollars(x.amount))
                            })
                            .unwrap_or_else(|| format!("#{e}"));
                        if widgets::link(ui, RichText::new(label).color(p.accent)).clicked() {
                            shell.go(Route::Entry(e));
                        }
                        ui.end_row();
                    }
                });
            ui.add_space(10.0);
            super::entry::render(ui, b, a, shell, ui.available_width());
            if let Some(d) = &a.description {
                ui.add_space(6.0);
                ui.label(RichText::new(d).color(p.muted));
            }
            if a.withdrawn_at.is_some() {
                ui.add_space(6.0);
                widgets::pill(
                    ui,
                    &format!("Withdrawn: {}", a.withdrawn_reason.as_deref().unwrap_or("")),
                    p.warning,
                    p.warning_soft,
                );
            }
        });
}
