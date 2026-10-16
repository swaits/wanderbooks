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
    link_label(ui, egui::Label::new(text))
}

/// [`link`] on one line, cut with an ellipsis where the space ends.
pub fn link_truncated(ui: &mut Ui, text: RichText) -> Response {
    link_label(ui, egui::Label::new(text).truncate())
}

fn link_label(ui: &mut Ui, label: egui::Label) -> Response {
    let p = pal(ui.ctx());
    let r = ui.add(label.sense(Sense::click()));
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
    // An amount copies as its plain decimal: a click when the cell doesn't
    // drill down, the right-click menu when it does.
    let copy = crate::fmt::plain(text);
    let (rect, resp) = ui.allocate_exact_size(
        vec2(width, 22.0),
        if drill || copy.is_some() {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let p = pal(ui.ctx());
    let copy_id = resp.id.with("copy");
    if let Some(v) = &copy {
        if drill {
            resp.context_menu(|ui| copy_item(ui, "Copy amount", v));
        } else if resp.clicked() {
            copy_now(ui, copy_id, v);
        }
    }
    let copied = copy.is_some() && !drill && recently_copied(ui, copy_id);
    if copy.is_some() && !drill && resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let resp = if copy.is_some() && !drill {
        resp.on_hover_text(if copied { "Copied" } else { "Click to copy" })
    } else {
        resp
    };
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
    if copied {
        ui.painter().text(
            egui::pos2(pos.x - 4.0, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            egui_phosphor::regular::CHECK,
            egui::FontId::proportional(12.0),
            p.positive,
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
            let font = fit_font(
                ui,
                &k.value,
                theme::semibold,
                24.0,
                15.0,
                ui.available_width(),
            );
            ui.add(egui::Label::new(RichText::new(&k.value).font(font)).truncate());
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

/// Put `value` on the clipboard, remembering when under `id` (for the
/// brief "Copied" check).
pub fn copy_now(ui: &Ui, id: egui::Id, value: &str) {
    ui.ctx().copy_text(value.to_string());
    let now = ui.input(|i| i.time);
    ui.ctx().data_mut(|d| d.insert_temp(id, now));
}

/// Whether `id` was copied in the last moment; keeps repainting until the
/// check fades.
pub fn recently_copied(ui: &Ui, id: egui::Id) -> bool {
    let at: Option<f64> = ui.ctx().data(|d| d.get_temp(id));
    let now = ui.input(|i| i.time);
    let recent = at.is_some_and(|t| now - t < 1.2);
    if recent {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(200));
    }
    recent
}

/// A value that copies itself when clicked: a pointer and a copy icon on
/// hover, a check for a moment after.
pub fn copyable(ui: &mut Ui, text: RichText, value: &str) -> Response {
    let id = ui.id().with(("copy", value));
    let p = pal(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let r = ui.add(egui::Label::new(text).sense(Sense::click()));
        let recent = recently_copied(ui, id);
        let (slot, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
        if r.hovered() || recent {
            ui.painter().text(
                slot.center(),
                egui::Align2::CENTER_CENTER,
                if recent {
                    egui_phosphor::regular::CHECK
                } else {
                    egui_phosphor::regular::COPY
                },
                egui::FontId::proportional(12.0),
                if recent { p.positive } else { p.muted },
            );
        }
        if r.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let r = r.on_hover_text(if recent { "Copied" } else { "Click to copy" });
        if r.clicked() {
            copy_now(ui, id, value);
        }
        r
    })
    .inner
}

/// A secret that copies itself when clicked but is never drawn: `text` is
/// its masked form; `fetch` reads the real value, only on a click (or the
/// right-click menu's "Copy digits only"). A fetch that fails shows why,
/// beside it, for a few seconds.
pub fn secret_copyable(
    ui: &mut Ui,
    text: RichText,
    hover: &str,
    fetch: impl Fn() -> Result<String, String>,
) -> Response {
    let id = ui.id().with(("secret-copy", text.text()));
    let err_id = id.with("error");
    let p = pal(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let r = ui.add(egui::Label::new(text).sense(Sense::click()));
        let recent = recently_copied(ui, id);
        let (slot, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
        if r.hovered() || recent {
            ui.painter().text(
                slot.center(),
                egui::Align2::CENTER_CENTER,
                if recent {
                    egui_phosphor::regular::CHECK
                } else {
                    egui_phosphor::regular::COPY
                },
                egui::FontId::proportional(12.0),
                if recent { p.positive } else { p.muted },
            );
        }
        if r.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let copy = |ui: &mut Ui, digits_only: bool| match fetch() {
            Ok(v) => {
                let v: String = if digits_only {
                    v.chars().filter(char::is_ascii_digit).collect()
                } else {
                    v
                };
                ui.ctx().data_mut(|d| d.remove::<(f64, String)>(err_id));
                copy_now(ui, id, &v);
            }
            Err(e) => {
                let now = ui.input(|i| i.time);
                ui.ctx().data_mut(|d| d.insert_temp(err_id, (now, e)));
            }
        };
        let r = r.on_hover_text(if recent { "Copied" } else { hover });
        if r.clicked() {
            copy(ui, false);
        }
        r.context_menu(|ui| {
            if ui
                .button(format!("{} Copy", egui_phosphor::regular::COPY))
                .clicked()
            {
                copy(ui, false);
                ui.close();
            }
            if ui
                .button(format!("{} Copy digits only", egui_phosphor::regular::COPY))
                .clicked()
            {
                copy(ui, true);
                ui.close();
            }
        });
        let failed: Option<(f64, String)> = ui.ctx().data(|d| d.get_temp(err_id));
        let now = ui.input(|i| i.time);
        if let Some((at, e)) = failed.filter(|(at, _)| now - at < 6.0) {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(6.0 - (now - at)));
            ui.add(
                egui::Label::new(RichText::new(e).font(theme::regular(12.0)).color(p.warning))
                    .wrap(),
            );
        }
        r
    })
    .inner
}

/// A "Copy …" item for a right-click menu.
pub fn copy_item(ui: &mut Ui, label: &str, value: &str) {
    if ui
        .button(format!("{} {label}", egui_phosphor::regular::COPY))
        .clicked()
    {
        ui.ctx().copy_text(value.to_string());
        ui.close();
    }
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

/// Left-aligned single-line text, vertically centered at `pos`, cut with
/// an ellipsis at `max_width` so it never runs into whatever is to its
/// right (an amount, a date).
pub fn clipped_text(
    painter: &egui::Painter,
    pos: egui::Pos2,
    text: &str,
    font: egui::FontId,
    color: Color32,
    max_width: f32,
) {
    use egui::text::{LayoutJob, TextFormat, TextWrapping};
    let mut job = LayoutJob::single_section(text.to_string(), TextFormat::simple(font, color));
    job.wrap = TextWrapping {
        max_width: max_width.max(10.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    let galley = painter.layout_job(job);
    let y = pos.y - galley.size().y / 2.0;
    painter.galley(egui::pos2(pos.x, y), galley, color);
}

/// How many columns at least `min_width` wide (with `gap` between) fit
/// in `avail`, never fewer than one nor more than `max`.
pub fn fit_columns(avail: f32, min_width: f32, gap: f32, max: usize) -> usize {
    (((avail + gap) / (min_width + gap)).floor() as usize).clamp(1, max.max(1))
}

/// The width of each of `n` right-aligned amount columns in a painted
/// table `width` wide, leaving at least `label_min` for the label on the
/// left: `max` when there's room, shrinking to a floor that still fits an
/// amount. Pair with [`min_width`] so a table that can't fit scrolls.
pub fn amount_col_w(width: f32, n: usize, label_min: f32, max: f32) -> f32 {
    ((width - label_min) / n.max(1) as f32).clamp(AMOUNT_MIN, max)
}

/// The narrowest an amount column gets ("(1,234,567.89)" at 13.5px).
pub const AMOUNT_MIN: f32 = 92.0;

/// Lay out `add` at least `width` wide: as is when there's room, in a
/// sideways scroll when the window is narrower, so a table never paints
/// over itself or past the edge.
pub fn min_width<R>(ui: &mut Ui, id: &str, width: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    if ui.available_width() >= width {
        return add(ui);
    }
    egui::ScrollArea::horizontal()
        .id_salt(id)
        .show(ui, |ui| {
            ui.set_width(width);
            add(ui)
        })
        .inner
}

/// The largest size up to `size` at which `text` fits on one line in
/// `width` (down to `floor`): big numbers shrink rather than wrap.
pub fn fit_font(
    ui: &Ui,
    text: &str,
    font: impl Fn(f32) -> egui::FontId,
    size: f32,
    floor: f32,
    width: f32,
) -> egui::FontId {
    let measured = ui
        .painter()
        .layout_no_wrap(text.to_string(), font(size), Color32::WHITE)
        .size()
        .x;
    if measured <= width {
        return font(size);
    }
    font((size * width / measured).floor().max(floor))
}

/// A date picker for `d` (`id` its egui id salt), returning the date
/// picked. The picker's calendar holds years -9999 through 9999; a date
/// outside them (books an earlier version wrote may hold one, rules.md
/// B-28) is shown as text and kept, never truncated into another year.
pub fn date_picker(ui: &mut Ui, id: &str, d: chrono::NaiveDate) -> chrono::NaiveDate {
    use chrono::Datelike;
    let jiff_date = i16::try_from(d.year())
        .ok()
        .and_then(|y| jiff::civil::Date::new(y, d.month() as i8, d.day() as i8).ok());
    let Some(mut j) = jiff_date else {
        ui.label(crate::fmt::date(d));
        return d;
    };
    ui.add(
        egui_extras::DatePickerButton::new(&mut j)
            .id_salt(id)
            .calendar_week(false),
    );
    chrono::NaiveDate::from_ymd_opt(i32::from(j.year()), j.month() as u32, j.day() as u32)
        .unwrap_or(d)
}
