//! Small building blocks shared by every screen.

use crate::theme::{self, pal};
use egui::{
    Align, Color32, CornerRadius, Frame, Layout, Margin, Response, RichText, Sense, Stroke, Ui,
    vec2,
};

/// A white (or dark) rounded surface with a hairline border.
pub fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = pal(ui.ctx());
    Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// A card with a title row and a right-aligned extra (a link, a pill).
/// Returns what `extra` and `add` return.
pub fn titled_card<E, R>(
    ui: &mut Ui,
    title: &str,
    extra: impl FnOnce(&mut Ui) -> E,
    add: impl FnOnce(&mut Ui) -> R,
) -> (E, R) {
    card(ui, |ui| {
        let e = ui
            .horizontal(|ui| {
                ui.label(RichText::new(title).font(theme::semibold(14.0)));
                ui.with_layout(Layout::right_to_left(Align::Center), extra)
                    .inner
            })
            .inner;
        ui.add_space(8.0);
        (e, add(ui))
    })
}

/// Page title and the sentence under it.
pub fn page_header(ui: &mut Ui, title: &str, subtitle: &str) {
    let p = pal(ui.ctx());
    ui.label(RichText::new(title).font(theme::bold(26.0)));
    if !subtitle.is_empty() {
        ui.add_space(2.0);
        let w = ui.available_width().min(860.0);
        ui.allocate_ui(vec2(w, 0.0), |ui| {
            ui.set_max_width(w);
            ui.label(
                RichText::new(subtitle)
                    .font(theme::regular(15.0))
                    .color(p.muted),
            );
        });
    }
    ui.add_space(16.0);
}

/// A rounded tag.
pub fn pill(ui: &mut Ui, text: &str, fg: Color32, bg: Color32) -> Response {
    Frame::new()
        .fill(bg)
        .corner_radius(CornerRadius::same(255))
        .inner_margin(Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).font(theme::semibold(11.5)).color(fg))
        })
        .response
}

/// Text that looks like plain text until you hover it, then like a link.
pub fn link(ui: &mut Ui, text: RichText) -> Response {
    let p = pal(ui.ctx());
    let r = ui.add(egui::Label::new(text.clone()).sense(Sense::click()));
    if r.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        let rect = r.rect;
        ui.painter().line_segment(
            [
                rect.left_bottom() + vec2(0.0, 1.0),
                rect.right_bottom() + vec2(0.0, 1.0),
            ],
            Stroke::new(1.0, p.accent),
        );
    }
    r
}

/// An amount, right-aligned in a fixed-width cell, clickable when `drill`.
pub fn amount_cell(
    ui: &mut Ui,
    text: &str,
    width: f32,
    color: Color32,
    strong: bool,
    drill: bool,
) -> Response {
    let font = if strong {
        theme::semibold(14.0)
    } else {
        theme::regular(14.0)
    };
    let (rect, resp) = ui.allocate_exact_size(
        vec2(width, 22.0),
        if drill {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let p = pal(ui.ctx());
    let hovered = drill && resp.hovered();
    let galley = ui.painter().layout_no_wrap(
        text.to_string(),
        font,
        if hovered { p.accent } else { color },
    );
    let pos = egui::pos2(
        rect.right() - galley.size().x,
        rect.center().y - galley.size().y / 2.0,
    );
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        ui.painter().line_segment(
            [
                egui::pos2(pos.x, rect.center().y + galley.size().y / 2.0),
                egui::pos2(rect.right(), rect.center().y + galley.size().y / 2.0),
            ],
            Stroke::new(1.0, p.accent),
        );
    }
    ui.painter().galley(pos, galley, color);
    resp
}

/// A big number with a label, a change line, and an optional sparkline.
pub struct Kpi<'a> {
    pub label: &'a str,
    pub value: String,
    pub sub: Option<(String, Color32)>,
    pub spark: Option<&'a [f64]>,
    pub how: &'a str,
}

pub fn kpi(ui: &mut Ui, k: Kpi<'_>) -> Response {
    let p = pal(ui.ctx());
    let resp = Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(104.0);
            ui.label(
                RichText::new(k.label)
                    .font(theme::regular(12.5))
                    .color(p.muted),
            );
            ui.add_space(4.0);
            ui.label(RichText::new(&k.value).font(theme::semibold(24.0)));
            if let Some((text, color)) = &k.sub {
                ui.add_space(2.0);
                ui.label(RichText::new(text).font(theme::regular(12.0)).color(*color));
            }
            if let Some(values) = k.spark
                && values.len() > 1
            {
                ui.add_space(6.0);
                let (rect, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
                crate::charts::sparkline(ui.painter(), rect, values, p.accent);
            }
        })
        .response;
    if !k.how.is_empty() {
        resp.on_hover_text(k.how)
    } else {
        resp
    }
}

/// A friendly placeholder when there's nothing to show yet.
pub fn empty(ui: &mut Ui, icon: &str, title: &str, text: &str) {
    let p = pal(ui.ctx());
    ui.vertical_centered(|ui| {
        ui.add_space(18.0);
        ui.label(RichText::new(icon).size(28.0).color(p.faint));
        ui.add_space(4.0);
        ui.label(
            RichText::new(title)
                .font(theme::semibold(14.0))
                .color(p.muted),
        );
        ui.label(
            RichText::new(text)
                .font(theme::regular(12.5))
                .color(p.faint),
        );
        ui.add_space(18.0);
    });
}

/// "Copy prompt": puts an agent prompt on the clipboard.
pub fn copy_prompt(ui: &mut Ui, prompt: &str) {
    let p = pal(ui.ctx());
    let id = ui.id().with(("copied", prompt));
    let copied_at: Option<f64> = ui.ctx().data(|d| d.get_temp(id));
    let now = ui.input(|i| i.time);
    let recently = copied_at.is_some_and(|t| now - t < 1.5);
    let label = if recently {
        format!("{} Copied", egui_phosphor::regular::CHECK)
    } else {
        format!("{} Ask the agent", egui_phosphor::regular::SPARKLE)
    };
    let r = ui
        .add(
            egui::Button::new(
                RichText::new(label)
                    .font(theme::regular(12.0))
                    .color(p.accent),
            )
            .fill(p.accent_soft)
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(6)),
        )
        .on_hover_text(format!(
            "Copies this prompt for your agent session:\n\n{prompt}"
        ));
    if r.clicked() {
        ui.ctx().copy_text(prompt.to_string());
        ui.ctx().data_mut(|d| d.insert_temp(id, now));
    }
    if recently {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(300));
    }
}

/// A horizontal rule in the border color.
pub fn rule(ui: &mut Ui) {
    let p = pal(ui.ctx());
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, 1.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, p.border);
}

/// Selectable segmented control.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let p = pal(ui.ctx());
    let mut changed = false;
    Frame::new()
        .fill(p.hover)
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(2))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(|ui| {
                for (v, label) in options {
                    let on = *value == *v;
                    let b = egui::Button::new(
                        RichText::new(*label)
                            .font(if on {
                                theme::semibold(12.5)
                            } else {
                                theme::regular(12.5)
                            })
                            .color(if on { p.text } else { p.muted }),
                    )
                    .fill(if on { p.surface } else { Color32::TRANSPARENT })
                    .stroke(if on {
                        Stroke::new(1.0, p.border)
                    } else {
                        Stroke::NONE
                    })
                    .corner_radius(CornerRadius::same(6))
                    .min_size(vec2(0.0, 24.0));
                    if ui.add(b).clicked() && !on {
                        *value = *v;
                        changed = true;
                    }
                }
            });
        });
    changed
}
