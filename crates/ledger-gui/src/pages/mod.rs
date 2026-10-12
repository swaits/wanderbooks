//! One module per screen.

pub mod accounts;
pub mod cash;
pub mod company;
pub mod documents;
pub mod entry;
pub mod journal;
pub mod overview;
pub mod profit;
pub mod reports;
pub mod transactions;

use crate::app::Shell;
use crate::books::Books;
use crate::nav::Route;
use egui::{Margin, Ui};

/// Standard page body: scrollable, padded.
pub fn page(ui: &mut Ui, id: &str, add: impl FnOnce(&mut Ui)) {
    egui::ScrollArea::vertical()
        .id_salt(id)
        .auto_shrink(false)
        .show(ui, |ui| {
            egui::Frame::new()
                .inner_margin(Margin {
                    left: 32,
                    right: 32,
                    top: 24,
                    bottom: 40,
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    add(ui)
                });
        });
}

pub fn show(ui: &mut Ui, b: &Books, route: &Route, shell: &mut Shell<'_>) {
    match route {
        Route::Overview => overview::show(ui, b, shell),
        Route::Cash => cash::show(ui, b, shell),
        Route::Profit => profit::show(ui, b, shell),
        Route::Transactions => transactions::show(ui, b, None, shell),
        Route::Entry(id) => transactions::show(ui, b, Some(*id), shell),
        Route::Reports(kind) => reports::show(ui, b, *kind, shell),
        Route::Accounts => accounts::show_chart(ui, b, shell),
        Route::Account(id) => accounts::show_account(ui, b, *id, shell),
        Route::Documents => documents::show(ui, b, None, shell),
        Route::Document(id) => documents::show(ui, b, Some(*id), shell),
        Route::Company => company::show(ui, b, shell),
        Route::Journal => journal::show(ui, b, None, shell),
        Route::JournalEvent(seq) => journal::show(ui, b, Some(*seq), shell),
    }
}

/// Columns sized by `weights` when the page is at least `min_width`
/// wide; stacked otherwise. `add(i, ui)` fills column `i`.
pub fn columns(ui: &mut Ui, min_width: f32, weights: &[f32], mut add: impl FnMut(usize, &mut Ui)) {
    let gap = 16.0;
    if ui.available_width() < min_width {
        for i in 0..weights.len() {
            add(i, ui);
            ui.add_space(gap);
        }
        return;
    }
    let total: f32 = weights.iter().sum();
    let avail = ui.available_width() - gap * (weights.len() as f32 - 1.0);
    let top = ui.cursor().min;
    let mut x = top.x;
    let mut bottom = top.y;
    for (i, w) in weights.iter().enumerate() {
        let width = avail * w / total;
        let rect =
            egui::Rect::from_min_size(egui::pos2(x, top.y), egui::vec2(width, f32::INFINITY));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        child.set_width(width);
        add(i, &mut child);
        bottom = bottom.max(child.min_rect().bottom());
        x += width + gap;
    }
    let used = egui::Rect::from_min_max(top, egui::pos2(top.x + ui.available_width(), bottom));
    ui.advance_cursor_after_rect(used);
    ui.add_space(gap);
}

/// A list on the left and a detail pane on the right, split by a
/// draggable divider. The split is remembered per `id`.
pub fn split<T: ?Sized>(
    ui: &mut Ui,
    id: &str,
    default_detail: f32,
    state: &mut T,
    left: impl FnOnce(&mut Ui, &mut T),
    right: impl FnOnce(&mut Ui, &mut T),
) {
    let p = crate::theme::pal(ui.ctx());
    let rect = ui.available_rect_before_wrap();
    let key = egui::Id::new(("split", id));
    let mut detail_w: f32 = ui
        .ctx()
        .data_mut(|d| d.get_persisted(key))
        .unwrap_or(default_detail);
    detail_w = detail_w.clamp(360.0, (rect.width() - 380.0).max(360.0));
    let divider_x = rect.right() - detail_w;
    let left_rect = egui::Rect::from_min_max(rect.min, egui::pos2(divider_x, rect.bottom()));
    let right_rect = egui::Rect::from_min_max(egui::pos2(divider_x + 1.0, rect.top()), rect.max);
    ui.painter().rect_filled(right_rect, 0.0, p.surface);
    ui.painter().line_segment(
        [
            egui::pos2(divider_x, rect.top()),
            egui::pos2(divider_x, rect.bottom()),
        ],
        egui::Stroke::new(1.0, p.border),
    );
    let mut l = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(left_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    l.set_clip_rect(left_rect);
    left(&mut l, state);
    let inner = right_rect.shrink2(egui::vec2(24.0, 20.0));
    let mut r = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    r.set_clip_rect(right_rect);
    right(&mut r, state);
    let handle = egui::Rect::from_center_size(
        egui::pos2(divider_x, rect.center().y),
        egui::vec2(8.0, rect.height()),
    );
    let resp = ui.interact(handle, key.with("handle"), egui::Sense::drag());
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        ui.painter().line_segment(
            [
                egui::pos2(divider_x, rect.top()),
                egui::pos2(divider_x, rect.bottom()),
            ],
            egui::Stroke::new(2.0, p.accent),
        );
    }
    if resp.dragged() {
        detail_w -= resp.drag_delta().x;
        ui.ctx().data_mut(|d| d.insert_persisted(key, detail_w));
    }
    ui.allocate_rect(rect, egui::Sense::hover());
}
