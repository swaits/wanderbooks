//! Every receipt and document the books hold, rendered: what it is,
//! everything it is linked to, and whether it was withdrawn (and why).
//! Also the compact "Documents" card every subject's page shows
//! ([`documents_card`]).

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
use ledger_store::{Attachment, Subject};

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
                    .desired_width(ui.available_width().min(260.0)),
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
        // As many 200-point tiles as fit, stretched to fill the row.
        let per_row = widgets::fit_columns(ui.available_width(), 200.0, 16.0, usize::MAX);
        let tile_w =
            ((ui.available_width() - 16.0 * (per_row as f32 - 1.0)) / per_row as f32).min(280.0);
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
    match shell
        .receipts
        .thumb(&a.blob_sha, &a.mime_type, b.blob_root())
    {
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
        Render::Text { text, .. } => {
            // The first lines of the text, as written, clipped to the thumb.
            let head: String = text.lines().take(14).collect::<Vec<_>>().join("\n");
            let galley =
                ui.painter()
                    .layout(head, theme::regular(10.5), p.muted, thumb.width() - 16.0);
            ui.painter().with_clip_rect(thumb.shrink(4.0)).galley(
                thumb.min + vec2(8.0, 8.0),
                galley,
                p.muted,
            );
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
                fmt::dollars_or(x.amount)
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
            widgets::copyable(
                ui,
                RichText::new(&a.file_name).font(theme::bold(20.0)),
                &a.file_name,
            );
            ui.add_space(6.0);
            file_actions(ui, b, a);
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
                    widgets::copyable_truncated(
                        ui,
                        RichText::new(&a.blob_sha).monospace().size(11.5),
                        &a.blob_sha,
                    );
                    ui.end_row();
                    if let Some(st) = b.statement(a.id) {
                        k(ui, "Statement");
                        ui.add(
                            egui::Label::new(format!(
                                "{}, {} – {}",
                                b.account_label(st.account_id),
                                fmt::date(st.period_start),
                                fmt::date(st.period_end)
                            ))
                            .wrap(),
                        );
                        ui.end_row();
                        k(ui, "Opening");
                        ui.label(fmt::dollars(st.opening_balance));
                        ui.end_row();
                        k(ui, "Closing");
                        ui.label(
                            RichText::new(fmt::dollars(st.closing_balance))
                                .font(theme::semibold(13.5)),
                        );
                        ui.end_row();
                    }
                });
            ui.add_space(10.0);
            let mut go = None;
            links_section(ui, b, a, &mut go);
            if let Some(r) = go {
                shell.go(r);
            }
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

/// "Save a copy" and "Open" (plan 3.6): a copy in the Downloads folder,
/// never over a file there, opened with the system's app on "Open". The
/// books are only read. What came of it shows under the buttons.
fn file_actions(ui: &mut Ui, b: &Books, a: &Attachment) {
    let p = pal(ui.ctx());
    let said = ui.id().with(("file-said", a.id));
    ui.horizontal(|ui| {
        let save = ui
            .button(format!("{} Save a copy", icon::FILE_ARROW_UP))
            .on_hover_text("A copy in your Downloads folder");
        let open = ui
            .button(format!("{} Open", icon::ARROW_SQUARE_OUT))
            .on_hover_text("A copy in your Downloads folder, opened with its app");
        if save.clicked() || open.clicked() {
            let result = save_copy(b, a, open.clicked());
            ui.data_mut(|d| d.insert_temp(said, result));
        }
    });
    if let Some(result) = ui.data(|d| d.get_temp::<Result<String, String>>(said)) {
        let (text, color) = match result {
            Ok(path) => (format!("Saved {path}"), p.muted),
            Err(why) => (format!("Couldn't save a copy: {why}"), p.warning),
        };
        ui.add(egui::Label::new(RichText::new(text).size(12.0).color(color)).wrap());
    }
}

/// A copy of `a` in the Downloads folder (opened if `open`): its path.
fn save_copy(b: &Books, a: &Attachment, open: bool) -> Result<String, String> {
    let bytes = b.attachment_bytes(a)?;
    let (stem, ext) = crate::export::split_name(&a.file_name);
    let path = crate::export::save(stem, ext, &bytes)?;
    if open {
        crate::export::open(&path);
    }
    Ok(path.display().to_string())
}

/// The icon for a subject's kind.
pub fn subject_icon(s: Subject) -> &'static str {
    match s {
        Subject::Entry(_) => icon::RECEIPT,
        Subject::Account(_) => icon::TREE_STRUCTURE,
        Subject::Asset(_) => icon::PACKAGE,
        Subject::Party(_) => icon::ADDRESS_BOOK,
        Subject::Project(_) => icon::BRIEFCASE,
        Subject::Invoice(_) | Subject::Bill(_) => icon::FILE_TEXT,
        Subject::Period(_) => icon::CALENDAR_BLANK,
        Subject::PayrollRun(_) => icon::USERS,
        Subject::Schedule(_) => icon::CALENDAR_CHECK,
        Subject::Reconciliation(_) => icon::CHECK_SQUARE,
        Subject::Company => icon::BUILDINGS,
    }
}

/// A small chip naming a subject, which opens it (see
/// [`Books::subject_route`]) when it has somewhere to go.
pub fn subject_chip(ui: &mut Ui, b: &Books, s: Subject, go: &mut Option<Route>) {
    let p = pal(ui.ctx());
    let route = b.subject_route(s);
    // A reconciliation's chip names its date before anything is cut.
    let mut label = match s {
        Subject::Reconciliation(r) => b.reconciliation(r).map_or_else(
            || b.subject_label(s),
            |r| {
                format!(
                    "Reconciliation, {} through {}",
                    b.account_name(r.account_id),
                    fmt::date(r.statement_date)
                )
            },
        ),
        _ => b.subject_label(s),
    };
    if label.chars().count() > 60 {
        label = label.chars().take(59).collect::<String>() + "…";
    }
    let r = ui.add(
        egui::Button::new(
            RichText::new(format!("{} {label}", subject_icon(s)))
                .font(theme::regular(11.5))
                .color(if route.is_some() { p.accent } else { p.muted }),
        )
        .fill(if route.is_some() {
            p.accent_soft
        } else {
            p.hover
        })
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(255))
        .min_size(vec2(0.0, 22.0)),
    );
    if let Some(to) = route
        && r.on_hover_text(format!("Open {}", b.subject_label(s)))
            .clicked()
    {
        *go = Some(to);
    }
}

/// The compact list of the documents linked to `subjects`: each file's
/// name (opening its document page), category and description, and the
/// other places it is linked as chips that open them.
///
/// ```text
/// GOAL: every subject's page shows the files that are evidence of it,
///       and where else each one is evidence.
/// SOURCE: docs/parties-and-attachments.md (Attachments, Viewer).
/// INVARIANTS:
///   1. Lists exactly Books::documents_for(subjects), in that order.
///   2. A file's chips are links_of(file) less `here` (the page's own
///      subject; [`documents_card`] passes `subjects`).
///   3. Empty: `empty` in muted text (nothing else).
/// ```
pub fn documents_card_with(
    ui: &mut Ui,
    b: &Books,
    title: &str,
    subjects: &[Subject],
    here: &[Subject],
    empty: &str,
    go: &mut Option<Route>,
) {
    let p = pal(ui.ctx());
    let docs = b.documents_for(subjects);
    widgets::titled_card(
        ui,
        title,
        |ui| {
            if !docs.is_empty() {
                widgets::pill(ui, &docs.len().to_string(), p.muted, p.hover);
            }
        },
        |ui| {
            if docs.is_empty() {
                ui.add(egui::Label::new(RichText::new(empty).color(p.muted)).wrap());
                return;
            }
            for (i, d) in docs.iter().enumerate() {
                if i > 0 {
                    ui.add_space(4.0);
                    widgets::rule(ui);
                    ui.add_space(4.0);
                }
                document_row(ui, b, d, here, go);
            }
        },
    );
}

/// [`documents_card_with`], the chips leaving out `subjects` themselves.
pub fn documents_card(
    ui: &mut Ui,
    b: &Books,
    title: &str,
    subjects: &[Subject],
    empty: &str,
    go: &mut Option<Route>,
) {
    documents_card_with(ui, b, title, subjects, subjects, empty, go);
}

/// One file in a [`documents_card_with`].
fn document_row(ui: &mut Ui, b: &Books, d: &Attachment, here: &[Subject], go: &mut Option<Route>) {
    let p = pal(ui.ctx());
    ui.horizontal_top(|ui| {
        ui.label(RichText::new(file_icon(d)).size(18.0).color(p.muted));
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            if widgets::link_truncated(ui, RichText::new(&d.file_name).font(theme::semibold(13.0)))
                .on_hover_text("Open this document")
                .clicked()
            {
                *go = Some(Route::Document(d.id));
            }
            let mut sub = vec![];
            match b.statement(d.id) {
                None => sub.push(fmt::label(&d.category)),
                Some(st) => sub.push(format!(
                    "Statement {} – {}, closing {}",
                    fmt::date_short(st.period_start),
                    fmt::date(st.period_end),
                    fmt::dollars(st.closing_balance)
                )),
            }
            if let Some(desc) = &d.description {
                sub.push(desc.clone());
            }
            ui.add(
                egui::Label::new(
                    RichText::new(sub.join(" · "))
                        .font(theme::regular(11.5))
                        .color(p.muted),
                )
                .wrap(),
            );
            let elsewhere: Vec<Subject> = b
                .links_of(d.id)
                .iter()
                .copied()
                .filter(|s| !here.contains(s))
                .collect();
            if !elsewhere.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                    ui.label(
                        RichText::new("Also on")
                            .font(theme::regular(11.5))
                            .color(p.faint),
                    );
                    for s in elsewhere {
                        subject_chip(ui, b, s, go);
                    }
                });
            }
        });
    });
}

/// A PDF or an image.
pub fn file_icon(a: &Attachment) -> &'static str {
    if a.mime_type == "application/pdf" {
        icon::FILE_PDF
    } else if a.mime_type.starts_with("image/") {
        icon::FILE_IMAGE
    } else {
        icon::FILE
    }
}

/// Everything a document is linked to: a chip per subject (an entry's
/// with its date and amount in the label), opening it; and, for a
/// statement, the reconciliations that used it.
fn links_section(ui: &mut Ui, b: &Books, a: &Attachment, go: &mut Option<Route>) {
    let p = pal(ui.ctx());
    let links = b.links_of(a.id);
    ui.label(
        RichText::new("LINKED TO")
            .font(theme::semibold(11.5))
            .color(p.faint),
    );
    ui.add_space(4.0);
    if links.is_empty() {
        ui.label(
            RichText::new("Nothing yet: filed on its own.")
                .font(theme::regular(12.5))
                .color(p.muted),
        );
    }
    for s in links {
        ui.horizontal(|ui| {
            subject_chip(ui, b, *s, go);
            if let Subject::Entry(e) = s
                && let Some(x) = b.entry(*e)
            {
                ui.label(
                    RichText::new(format!(
                        "{} · {}",
                        fmt::date(x.date),
                        fmt::dollars_or(x.amount)
                    ))
                    .font(theme::regular(11.5))
                    .color(p.muted),
                );
            }
            if let Subject::Reconciliation(r) = s
                && let Some(r) = b.reconciliation(*r)
                && let Some(why) = &r.voided
            {
                ui.label(
                    RichText::new(format!("voided: {why}"))
                        .font(theme::regular(11.5))
                        .color(p.warning),
                );
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{Build, d, find, render};

    /// A document's page lists what it is linked to: an entry with its date
    /// and amount.
    #[test]
    fn a_document_lists_the_entry_it_is_linked_to() {
        let mut b = Build::new("document-links");
        let (cash, capital) = (b.cash, b.capital);
        let e = b.post(d(2026, 1, 2), cash, capital, 100_000);
        let (att, _, _) =
            b.s.file_document("other", "note.pdf", "application/pdf", b"%PDF note", None)
                .unwrap();
        b.s.link_attachment(att, ledger_store::Subject::Entry(e))
            .unwrap();
        let (_dir, books) = b.books();
        let (texts, _) = render(books, Route::Document(att), |_| {});
        find(&texts, "LINKED TO");
        find(&texts, "Jan 2, 2026 \u{b7} $1,000.00");
    }
}
