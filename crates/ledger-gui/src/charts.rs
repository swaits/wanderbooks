//! Charts, painted directly: a balance line (step-shaped, because a
//! balance holds until the next transaction), grouped bars, ranked
//! horizontal bars, and sparklines. Every point answers hover with its
//! exact value; bars report clicks so they can drill down.

use crate::fmt;
use crate::theme::{self, pal};
use chrono::NaiveDate;
use egui::{
    Align2, Color32, CornerRadius, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui, pos2, vec2,
};
use rust_decimal::Decimal;

fn f(d: Decimal) -> f64 {
    d.try_into().unwrap_or(0.0)
}

/// ```text
/// INVARIANTS (finite lo <= hi, n >= 1):
///   1. Evenly spaced by a step of 1, 2, 2.5, or 5 times a power of ten
///      (each tick a whole multiple of the step), all finite.
///   2. The first tick is at or below lo and the last at or above hi, so
///      no data falls outside the gridlines.
///   3. At most n + 2 ticks.
///   When no such step exists in f64 (nice ticks past f64's range, within
///   a factor of ten of it, or a span too small for the magnitude), the
///   ticks are just [lo, hi] (or [lo]).
/// ```
/// "Nice" axis ticks covering [lo, hi].
pub fn ticks(lo: f64, hi: f64, n: usize) -> Vec<f64> {
    let n = n.max(1);
    let fallback = || if lo == hi { vec![lo] } else { vec![lo, hi] };
    let span = (hi - lo).max(f64::MIN_POSITIVE);
    if !span.is_finite() {
        return fallback();
    }
    let raw = span / n as f64;
    let mut mag = 10f64.powf(raw.log10().floor());
    // Try the nice steps from the smallest that fits n, coarser as needed
    // (rounding at large magnitudes can cost a tick).
    for _ in 0..4 {
        for m in [1.0, 2.0, 2.5, 5.0] {
            let step = m * mag;
            if !step.is_finite() || step <= 0.0 || span / step > n as f64 {
                continue;
            }
            let (k0, k1) = ((lo / step).floor(), (hi / step).ceil());
            if !(k0.is_finite() && k1.is_finite()) || k1 - k0 + 1.0 > (n + 2) as f64 {
                continue;
            }
            let v: Vec<f64> = (0..=(k1 - k0) as i64)
                .map(|k| (k0 + k as f64) * step)
                .collect();
            let ok = v.iter().all(|x| x.is_finite())
                && v.windows(2).all(|w| w[1] > w[0])
                && v.first().is_some_and(|f| *f <= lo)
                && v.last().is_some_and(|l| *l >= hi);
            if ok {
                return v;
            }
        }
        mag *= 10.0;
    }
    fallback()
}

fn money_label(v: f64) -> String {
    fmt::compact(Decimal::from_f64_retain(v).unwrap_or_default())
}

/// A balance over time as a step line with a soft fill. `points` are
/// (date, balance at end of day), ascending; the line runs to `until`.
pub fn balance_line(ui: &mut Ui, points: &[(NaiveDate, Decimal)], until: NaiveDate, height: f32) {
    let p = pal(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let painter = ui.painter_at(rect);
    if points.is_empty() {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "No cash activity yet",
            theme::regular(12.5),
            p.faint,
        );
        return;
    }
    let first = points[0].0;
    let last = until.max(points[points.len() - 1].0);
    let days = ((last - first).num_days().max(1)) as f64;
    let vals: Vec<f64> = points.iter().map(|(_, v)| f(*v)).collect();
    let lo = vals.iter().cloned().fold(0.0f64, f64::min);
    let hi = vals.iter().cloned().fold(f64::MIN, f64::max).max(lo + 1.0);
    let ts = ticks(lo, hi, 4);
    let (lo, hi) = (ts[0].min(lo), ts[ts.len() - 1].max(hi));
    let plot = Rect::from_min_max(rect.min + vec2(52.0, 6.0), rect.max - vec2(8.0, 22.0));
    let x =
        |d: NaiveDate| plot.left() + ((d - first).num_days() as f64 / days) as f32 * plot.width();
    let y = |v: f64| plot.bottom() - ((v - lo) / (hi - lo)) as f32 * plot.height();
    for t in &ts {
        let yy = y(*t);
        painter.line_segment(
            [pos2(plot.left(), yy), pos2(plot.right(), yy)],
            Stroke::new(1.0, p.border),
        );
        painter.text(
            pos2(plot.left() - 8.0, yy),
            Align2::RIGHT_CENTER,
            money_label(*t),
            theme::regular(11.0),
            p.faint,
        );
    }
    // X labels: first, last, and a few between.
    let n_labels = ((plot.width() / 90.0) as usize).clamp(1, 4);
    for i in 0..=n_labels {
        // At most `last` (a one-day chart is one day wide, and its first day
        // may be chrono's last).
        let d = first
            .checked_add_days(chrono::Days::new(
                ((days * i as f64) / n_labels as f64).round() as u64,
            ))
            .map_or(last, |d| d.min(last));
        let align = match i {
            0 => Align2::LEFT_TOP,
            i if i == n_labels => Align2::RIGHT_TOP,
            _ => Align2::CENTER_TOP,
        };
        painter.text(
            pos2(x(d), plot.bottom() + 6.0),
            align,
            fmt::date_short(d),
            theme::regular(11.0),
            p.faint,
        );
    }
    // Step path.
    let mut line: Vec<Pos2> = Vec::new();
    for (i, (d, v)) in points.iter().enumerate() {
        let px = x(*d);
        if i > 0 {
            line.push(pos2(px, line.last().unwrap().y));
        }
        line.push(pos2(px, y(f(*v))));
    }
    line.push(pos2(x(last), line.last().unwrap().y));
    let base = y(0.0f64.clamp(lo, hi));
    let mut fill = line.clone();
    fill.push(pos2(x(last), base));
    fill.push(pos2(x(first), base));
    let soft = Color32::from_rgba_unmultiplied(p.accent.r(), p.accent.g(), p.accent.b(), 28);
    // Fill: one mesh of strips under each flat step (the outline may be
    // non-convex). A mesh has no anti-aliased edges, so strips meet without
    // seams.
    let mut mesh = egui::Mesh::default();
    for w in line.windows(2) {
        if (w[0].y - w[1].y).abs() < 0.01 && w[1].x > w[0].x {
            mesh.add_colored_rect(
                Rect::from_min_max(
                    pos2(w[0].x, w[0].y.min(base)),
                    pos2(w[1].x, w[0].y.max(base)),
                ),
                soft,
            );
        }
    }
    painter.add(Shape::mesh(mesh));
    let _ = fill;
    painter.add(Shape::line(line.clone(), Stroke::new(2.0, p.accent)));
    // Hover: the balance on the hovered day.
    if let Some(hp) = resp.hover_pos()
        && plot.x_range().contains(hp.x)
    {
        let frac = ((hp.x - plot.left()) / plot.width()) as f64;
        let day = first
            .checked_add_days(chrono::Days::new((frac * days).round() as u64))
            .map_or(last, |d| d.min(last));
        let bal = points
            .iter()
            .take_while(|(d, _)| *d <= day)
            .last()
            .map(|(_, v)| *v);
        if let Some(b) = bal {
            let hx = x(day);
            painter.line_segment(
                [pos2(hx, plot.top()), pos2(hx, plot.bottom())],
                Stroke::new(1.0, p.faint),
            );
            painter.circle_filled(pos2(hx, y(f(b))), 4.0, p.accent);
            tooltip(
                &painter,
                pos2(hx, plot.top()),
                &fmt::date(day),
                &fmt::dollars(b),
                p.surface,
                p.border,
                p.text,
                p.muted,
                rect,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn tooltip(
    painter: &Painter,
    at: Pos2,
    title: &str,
    value: &str,
    bg: Color32,
    border: Color32,
    fg: Color32,
    muted: Color32,
    bounds: Rect,
) {
    let t1 = painter.layout_no_wrap(title.to_string(), theme::regular(11.5), muted);
    let t2 = painter.layout_no_wrap(value.to_string(), theme::semibold(13.0), fg);
    let w = t1.size().x.max(t2.size().x) + 16.0;
    let h = t1.size().y + t2.size().y + 12.0;
    let mut r = Rect::from_min_size(pos2(at.x + 8.0, at.y), vec2(w, h));
    if r.right() > bounds.right() {
        r = r.translate(vec2(-w - 16.0, 0.0));
    }
    painter.rect(
        r,
        CornerRadius::same(6),
        bg,
        Stroke::new(1.0, border),
        egui::StrokeKind::Inside,
    );
    painter.galley(r.min + vec2(8.0, 6.0), t1.clone(), muted);
    painter.galley(r.min + vec2(8.0, 6.0 + t1.size().y), t2, fg);
}

/// One group of bars on a [`bar_chart`].
pub struct BarGroup {
    pub label: String,
    pub values: Vec<Decimal>,
    /// The group's year, when the chart spans more than one: the first
    /// label drawn, and each drawn in a new year, gains it ("Jan 26"),
    /// whichever labels a crowded chart leaves out.
    pub year: Option<i32>,
}

/// Grouped bars (e.g. income and spending per month), with a legend.
/// Returns the clicked group, if any.
pub fn bar_chart(
    ui: &mut Ui,
    groups: &[BarGroup],
    series: &[(&str, Color32)],
    height: f32,
) -> Option<usize> {
    bars(ui, groups, series, None, height, true)
}

/// [`bar_chart`] that opens nothing: hovering shows each group's figures,
/// and the pointer stays an arrow, since no page shows those figures.
pub fn bar_chart_view(ui: &mut Ui, groups: &[BarGroup], series: &[(&str, Color32)], height: f32) {
    bars(ui, groups, series, None, height, false);
}

/// A line under a [`bar_chart_view`]'s bars: one value per group, marked
/// at each group's center (e.g. a running net toward break-even), in a
/// strip of its own with its own scale and a $0 rule, so a running total
/// many times one group's figures doesn't flatten the bars.
pub struct Overlay<'a> {
    pub name: &'a str,
    pub color: Color32,
    pub values: &'a [Decimal],
}

/// [`bar_chart_view`] with an [`Overlay`] line in a strip under the bars
/// ([`LINE_STRIP`] taller); its value joins each group's hover figures.
pub fn bar_chart_view_with_line(
    ui: &mut Ui,
    groups: &[BarGroup],
    series: &[(&str, Color32)],
    line: &Overlay<'_>,
    height: f32,
) {
    bars(ui, groups, series, Some(line), height, false);
}

/// How much taller an [`Overlay`] makes a bar chart: its strip.
pub const LINE_STRIP: f32 = 84.0;

fn bars(
    ui: &mut Ui,
    groups: &[BarGroup],
    series: &[(&str, Color32)],
    line: Option<&Overlay<'_>>,
    height: f32,
    clickable: bool,
) -> Option<usize> {
    let p = pal(ui.ctx());
    ui.horizontal(|ui| {
        for (name, color) in series {
            let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
            ui.painter().rect_filled(r, CornerRadius::same(3), *color);
            ui.label(
                egui::RichText::new(*name)
                    .font(theme::regular(12.0))
                    .color(p.muted),
            );
            ui.add_space(8.0);
        }
        if let Some(l) = line {
            let (r, _) = ui.allocate_exact_size(vec2(16.0, 10.0), Sense::hover());
            ui.painter().line_segment(
                [r.left_center(), r.right_center()],
                Stroke::new(2.0, l.color),
            );
            ui.painter().circle_filled(r.center(), 3.0, l.color);
            ui.label(
                egui::RichText::new(l.name)
                    .font(theme::regular(12.0))
                    .color(p.muted),
            );
        }
    });
    let sense = if clickable {
        Sense::click()
    } else {
        Sense::hover()
    };
    let strip_h = if line.is_some() { LINE_STRIP } else { 0.0 };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), height + strip_h), sense);
    let painter = ui.painter_at(rect);
    if groups.is_empty() {
        return None;
    }
    // The bars' scale is the bars' own; the line has its strip's.
    let all: Vec<f64> = groups
        .iter()
        .flat_map(|g| g.values.iter().map(|v| f(*v)))
        .collect();
    let lo = all.iter().cloned().fold(0.0f64, f64::min);
    let hi = all.iter().cloned().fold(0.0f64, f64::max).max(lo + 1.0);
    let ts = ticks(lo, hi, 4);
    let (lo, hi) = (ts[0].min(lo), ts[ts.len() - 1].max(hi));
    let plot = Rect::from_min_max(
        rect.min + vec2(52.0, 6.0),
        pos2(rect.max.x - 4.0, rect.min.y + height - 22.0),
    );
    let strip = line.map(|_| {
        Rect::from_min_max(
            pos2(plot.left(), plot.bottom() + 24.0),
            pos2(plot.right(), plot.bottom() + strip_h - 4.0),
        )
    });
    let bottom = strip.map_or(plot.bottom(), |s| s.bottom());
    let y = |v: f64| plot.bottom() - ((v - lo) / (hi - lo)) as f32 * plot.height();
    for t in &ts {
        let yy = y(*t);
        painter.line_segment(
            [pos2(plot.left(), yy), pos2(plot.right(), yy)],
            Stroke::new(1.0, p.border),
        );
        painter.text(
            pos2(plot.left() - 8.0, yy),
            Align2::RIGHT_CENTER,
            money_label(*t),
            theme::regular(11.0),
            p.faint,
        );
    }
    let gw = plot.width() / groups.len() as f32;
    let n = series.len().max(1) as f32;
    let bw = ((gw * 0.7) / n).min(28.0);
    let mut clicked = None;
    let hover = resp.hover_pos();
    let mut drawn_year: Option<i32> = None;
    let mut tip: Option<(egui::Pos2, String, String)> = None;
    // The line goes over the hover band and under the tooltip, on its
    // own scale with room above and below (its markers never sit on the
    // strip's edge), and a dashed $0: break-even.
    let mut overlay = Vec::new();
    if let (Some(l), Some(s)) = (line, strip) {
        let vals: Vec<f64> = l.values.iter().take(groups.len()).map(|v| f(*v)).collect();
        let lo = vals.iter().cloned().fold(0.0f64, f64::min);
        let hi = vals.iter().cloned().fold(0.0f64, f64::max).max(lo + 1.0);
        let pad = (hi - lo) * 0.12;
        let (lo, hi) = (lo - pad, hi + pad);
        let sy = |v: f64| s.bottom() - ((v - lo) / (hi - lo)) as f32 * s.height();
        let zero = sy(0.0);
        overlay.extend(egui::Shape::dashed_line(
            &[pos2(s.left(), zero), pos2(s.right(), zero)],
            Stroke::new(1.0, p.faint),
            4.0,
            3.0,
        ));
        let label = |v: f64, at: f32| {
            let g = painter.layout_no_wrap(money_label(v), theme::regular(11.0), p.faint);
            let r = Align2::RIGHT_CENTER.anchor_size(pos2(s.left() - 8.0, at), g.size());
            egui::Shape::galley(r.min, g, p.faint)
        };
        overlay.push(label(0.0, zero));
        let (vmin, vmax) = (
            vals.iter().cloned().fold(0.0f64, f64::min),
            vals.iter().cloned().fold(0.0f64, f64::max),
        );
        for v in [vmax, vmin] {
            if v != 0.0 && (sy(v) - zero).abs() >= 12.0 {
                overlay.push(label(v, sy(v)));
            }
        }
        let pts: Vec<egui::Pos2> = vals
            .iter()
            .enumerate()
            .map(|(gi, v)| pos2(plot.left() + gw * (gi as f32 + 0.5), sy(*v)))
            .collect();
        overlay.push(egui::Shape::line(pts.clone(), Stroke::new(2.0, l.color)));
        for pt in pts {
            overlay.push(egui::Shape::circle_filled(pt, 3.0, l.color));
        }
    }
    for (gi, g) in groups.iter().enumerate() {
        let full = match g.year {
            Some(y) => format!("{} {:02}", g.label, y.rem_euclid(100)),
            None => g.label.clone(),
        };
        let gx = plot.left() + gw * gi as f32;
        let grect = Rect::from_min_max(pos2(gx, plot.top()), pos2(gx + gw, bottom));
        let hovered = hover.is_some_and(|h| grect.contains(h));
        if hovered {
            painter.rect_filled(grect, CornerRadius::same(6), p.hover);
        }
        let total_w = bw * n + 3.0 * (n - 1.0);
        let start = gx + (gw - total_w) / 2.0;
        for (si, v) in g.values.iter().enumerate() {
            let color = series.get(si).map(|s| s.1).unwrap_or(p.accent);
            let x0 = start + si as f32 * (bw + 3.0);
            let (y0, y1) = (y(0.0), y(f(*v)));
            let r = Rect::from_min_max(
                pos2(x0, y0.min(y1)),
                pos2(x0 + bw, y0.max(y1).max(y0.min(y1) + 1.0)),
            );
            painter.rect_filled(
                r,
                CornerRadius {
                    nw: 3,
                    ne: 3,
                    sw: 0,
                    se: 0,
                },
                color,
            );
        }
        // Crowded, label every other group (or fewer), counting back from
        // the latest so it's always labeled.
        let every = (34.0 / gw).ceil().max(1.0) as usize;
        if (groups.len() - 1 - gi).is_multiple_of(every) {
            let new_year = g.year.is_some() && g.year != drawn_year;
            drawn_year = g.year.or(drawn_year);
            painter.text(
                pos2(gx + gw / 2.0, bottom + 6.0),
                Align2::CENTER_TOP,
                if new_year { &full } else { &g.label },
                theme::regular(11.0),
                p.faint,
            );
        }
        if hovered {
            let mut lines: Vec<String> = series
                .iter()
                .zip(&g.values)
                .map(|((name, _), v)| format!("{name}: {}", fmt::dollars(*v)))
                .collect();
            if let Some(l) = line
                && let Some(v) = l.values.get(gi)
            {
                lines.push(format!("{}: {}", l.name, fmt::dollars(*v)));
            }
            tip = Some((pos2(gx + gw / 2.0, plot.top()), full, lines.join("\n")));
            if clickable {
                if resp.clicked() {
                    clicked = Some(gi);
                }
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        }
    }
    painter.extend(overlay);
    if let Some((at, title, body)) = tip {
        tooltip(
            &painter, at, &title, &body, p.surface, p.border, p.text, p.muted, rect,
        );
    }
    clicked
}

/// Ranked horizontal bars with labels and exact amounts. Returns the
/// clicked row.
pub fn hbars(ui: &mut Ui, rows: &[(String, Decimal)], color: Color32) -> Option<usize> {
    hbars_signed(ui, rows, color, color)
}

/// [`hbars`] with a row below zero (a loss) drawn in `negative`, its
/// amount too, so the longest bar can't read as the best.
pub fn hbars_signed(
    ui: &mut Ui,
    rows: &[(String, Decimal)],
    color: Color32,
    negative: Color32,
) -> Option<usize> {
    let p = pal(ui.ctx());
    let max = rows
        .iter()
        .map(|(_, v)| f(*v).abs())
        .fold(0.0, f64::max)
        .max(1e-9);
    let mut clicked = None;
    for (i, (label, v)) in rows.iter().enumerate() {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
        let painter = ui.painter_at(rect);
        if resp.hovered() {
            painter.rect_filled(rect, CornerRadius::same(6), p.hover);
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        let below = *v < Decimal::ZERO;
        let fill = if below { negative } else { color };
        let amount_color = if below && negative != color {
            negative
        } else {
            p.text
        };
        let amount = fmt::dollars(*v);
        let ag = painter.layout_no_wrap(amount, theme::regular(13.0), amount_color);
        let label_w = (rect.width() * 0.38).min(220.0);
        let lg = painter.layout(label.clone(), theme::regular(13.0), p.text, label_w);
        painter.galley(
            pos2(rect.left() + 6.0, rect.center().y - lg.size().y / 2.0),
            lg,
            p.text,
        );
        let bar_left = rect.left() + label_w + 14.0;
        let bar_max = rect.right() - ag.size().x - 18.0 - bar_left;
        let w = (f(*v).abs() / max) as f32 * bar_max.max(0.0);
        let bar = Rect::from_min_size(
            pos2(bar_left, rect.center().y - 6.0),
            vec2(w.max(2.0), 12.0),
        );
        painter.rect_filled(bar, CornerRadius::same(3), fill);
        painter.galley(
            pos2(
                rect.right() - ag.size().x - 6.0,
                rect.center().y - ag.size().y / 2.0,
            ),
            ag,
            amount_color,
        );
        if resp.clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

/// A tiny trend line.
pub fn sparkline(painter: &Painter, rect: Rect, values: &[f64], color: Color32) {
    let lo = values.iter().cloned().fold(f64::MAX, f64::min);
    let hi = values.iter().cloned().fold(f64::MIN, f64::max);
    let span = (hi - lo).max(1e-9);
    let n = values.len().max(2) - 1;
    let pts: Vec<Pos2> = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            pos2(
                rect.left() + rect.width() * i as f32 / n as f32,
                rect.bottom() - ((v - lo) / span) as f32 * rect.height(),
            )
        })
        .collect();
    painter.add(Shape::line(pts, Stroke::new(1.5, color)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    /// Every text a frame painted, with where it was painted.
    fn painted(out: &egui::FullOutput) -> Vec<(String, egui::Pos2, f32)> {
        fn walk(s: &egui::Shape, out: &mut Vec<(String, egui::Pos2, f32)>) {
            match s {
                egui::Shape::Text(t) => {
                    out.push((t.galley.text().to_string(), t.pos, t.galley.size().x))
                }
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut texts = Vec::new();
        for c in &out.shapes {
            walk(&c.shape, &mut texts);
        }
        texts
    }

    /// charts::balance_line: the x axis is labeled with its first day, its
    /// last, and days evenly between (rounded); hovering shows the balance on
    /// the hovered day.
    #[test]
    fn the_balance_chart_labels_its_days_and_shows_the_hovered_one() {
        let points = vec![
            (d(2026, 1, 1), Decimal::new(10_000, 2)),
            (d(2026, 1, 11), Decimal::new(5_000, 2)),
        ];
        let mut h = Harness::builder()
            .with_size(egui::vec2(800.0, 300.0))
            .build_ui(move |ui| balance_line(ui, &points, d(2026, 1, 11), 200.0));
        crate::theme::install_fonts(&h.ctx);
        crate::theme::apply(&h.ctx, false);
        h.run();
        let texts = painted(h.output());
        let labels: Vec<&str> = texts.iter().map(|(t, _, _)| t.as_str()).collect();
        // Ten days, four gaps (the plot is wide): 0, 2.5, 5, 7.5, 10 rounded.
        for want in ["Jan 1", "Jan 4", "Jan 6", "Jan 9", "Jan 11"] {
            assert!(labels.contains(&want), "{want} not in {labels:?}");
        }
        // The plot runs from the first label's left edge to the last's right.
        let (_, first, _) = texts.iter().find(|(t, _, _)| t == "Jan 1").unwrap();
        let (_, last, w) = texts.iter().find(|(t, _, _)| t == "Jan 11").unwrap();
        let (left, right) = (first.x, last.x + w);
        for (frac, day) in [(0.3, d(2026, 1, 4)), (0.5, d(2026, 1, 6))] {
            h.hover_at(egui::pos2(left + (right - left) * frac, first.y - 40.0));
            h.run();
            let texts = painted(h.output());
            let want = crate::fmt::date(day);
            assert!(
                texts.iter().any(|(t, _, _)| *t == want),
                "{want} not shown at {frac}: {:?}",
                texts.iter().map(|(t, _, _)| t).collect::<Vec<_>>()
            );
        }
    }

    /// Months from November 2024 through October 2026, as the project and
    /// party charts give them.
    fn two_years() -> Vec<BarGroup> {
        (0..24)
            .map(|i| {
                let m = d(2024, 11, 1) + chrono::Months::new(i);
                BarGroup {
                    label: m.format("%b").to_string(),
                    values: vec![Decimal::new(i64::from(i) * 100, 2)],
                    year: Some(chrono::Datelike::year(&m)),
                }
            })
            .collect()
    }

    /// A crowded chart over two years drops every other label, January's
    /// too, yet the first label drawn and the first drawn in each new year
    /// carry the year, so no month name stands for two months unread.
    #[test]
    fn a_crowded_chart_keeps_its_years() {
        for width in [380.0, 800.0] {
            let groups = two_years();
            let mut h = Harness::builder()
                .with_size(egui::vec2(width, 300.0))
                .build_ui(move |ui| {
                    bar_chart_view(ui, &groups, &[("Net", Color32::GRAY)], 200.0);
                });
            crate::theme::install_fonts(&h.ctx);
            crate::theme::apply(&h.ctx, false);
            h.run();
            let mut labels: Vec<(String, f32)> = painted(h.output())
                .into_iter()
                .filter(|(t, _, _)| {
                    t.len() <= 6 && t.chars().next().is_some_and(char::is_alphabetic)
                })
                .filter(|(t, _, _)| t != "Net")
                .map(|(t, p, _)| (t, p.x))
                .collect();
            labels.sort_by(|a, b| a.1.total_cmp(&b.1));
            let names: Vec<&str> = labels.iter().map(|l| l.0.as_str()).collect();
            assert!(names.len() < 24, "{width}: crowded, so thinned: {names:?}");
            let tagged = |n: &&str| n.ends_with(char::is_numeric);
            assert!(tagged(&names[0]), "{width}: {names:?}");
            for year in [" 25", " 26"] {
                assert_eq!(
                    names.iter().filter(|n| n.ends_with(year)).count(),
                    1,
                    "{width}: {names:?}"
                );
            }
            // 2026's tag sits on the first label drawn in 2026.
            let at = names.iter().position(|n| n.ends_with(" 26")).unwrap();
            assert!(
                ["Jan", "Feb", "Mar"].contains(&&names[at][..3]) && !names[at - 1].ends_with(" 26"),
                "{width}: {names:?}"
            );
        }
    }

    /// A chart with nothing to open takes no click: hovering shows its
    /// figures, and the pointer stays an arrow.
    #[test]
    fn a_view_only_chart_shows_figures_and_takes_no_click() {
        let groups = two_years();
        let mut h = Harness::builder()
            .with_size(egui::vec2(800.0, 300.0))
            .build_ui(move |ui| {
                bar_chart_view(ui, &groups, &[("Net", Color32::GRAY)], 200.0);
            });
        crate::theme::install_fonts(&h.ctx);
        crate::theme::apply(&h.ctx, false);
        h.run();
        h.hover_at(egui::pos2(780.0, 120.0));
        h.run();
        let texts = painted(h.output());
        assert!(
            texts.iter().any(|(t, _, _)| t == "Oct 26"),
            "the tooltip names its month and year"
        );
        assert_ne!(
            h.output().platform_output.cursor_icon,
            egui::CursorIcon::PointingHand
        );
    }
}
