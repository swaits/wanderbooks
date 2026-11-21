//! The color scheme button and its menu (app-wide: one scheme for the
//! whole window, whatever company's tab is shown).

use crate::schemes::{self, Filter, Scheme};
use crate::theme::{self, pal};
use egui::{Align, CornerRadius, Layout, RichText, Sense, Stroke, Ui, vec2};
use egui_phosphor::regular as icon;

/// The height of a family's row in the color scheme menu, for laying the
/// families out in columns.
const FAMILY_ROW: f32 = 34.0;

/// How long the pointer rests on a scheme's row (or off every row) before
/// that scheme (or the one in force) is worn, seconds.
const PREVIEW_AFTER: f64 = 0.05;

/// The width of a variant's row in a family's flyout: room for its colors
/// and the longest name.
const SCHEME_ROW_WIDTH: f32 = 200.0;

/// Where the viewer keeps its color scheme ("Family/Variant").
const SCHEME_KEY: &str = "scheme";
/// Where it keeps its scheme menu's filter.
const FILTER_KEY: &str = "scheme_filter";

/// One variant in the color scheme menu: a strip of its colors and its
/// name, the whole row one button, selected when it is the one in force.
fn scheme_row(ui: &mut Ui, scheme: &Scheme, current: bool) -> egui::Response {
    let p = pal(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(vec2(SCHEME_ROW_WIDTH, 28.0), Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            true,
            current,
            scheme.name,
        )
    });
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let fill = if current {
            Some(p.selected)
        } else if response.hovered() {
            Some(p.hover)
        } else {
            None
        };
        if let Some(fill) = fill {
            painter.rect_filled(rect, CornerRadius::same(6), fill);
        }
        let strip = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 8.0, rect.center().y - 7.0),
            vec2(50.0, 14.0),
        );
        let w = strip.width() / 5.0;
        for (i, c) in scheme.swatch().into_iter().enumerate() {
            let r = egui::Rect::from_min_size(
                strip.min + vec2(w * i as f32, 0.0),
                vec2(w, strip.height()),
            );
            painter.rect_filled(r, 0.0, c);
        }
        painter.rect_stroke(
            strip,
            3.0,
            Stroke::new(1.0, p.border),
            egui::StrokeKind::Outside,
        );
        painter.text(
            egui::pos2(strip.right() + 10.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            scheme.name,
            theme::regular(14.0),
            p.text,
        );
    }
    response
}

/// The scheme and menu filter kept from last time (`get` reads the
/// viewer's preferences): the scheme kept; before schemes, the light or
/// dark choice kept as "dark"; else the viewer's own for the system's
/// mode (schemes INVARIANT 4).
fn restored(get: impl Fn(&str) -> Option<String>, system_dark: bool) -> (&'static Scheme, Filter) {
    let dark = get("dark").map_or(system_dark, |v| v == "true");
    let scheme = get(SCHEME_KEY).map_or_else(|| schemes::own(dark), |k| Scheme::from_key(&k, dark));
    let filter = get(FILTER_KEY)
        .and_then(|f| Filter::from_name(&f))
        .unwrap_or_default();
    (scheme, filter)
}

/// The color scheme in force, the one worn (a hovered one previewed), and
/// the menu's filter: the app's, shared by every tab.
pub struct SchemeMenu {
    /// The color scheme in force (schemes.rs).
    scheme: &'static Scheme,
    /// The scheme being worn: the one in force, or one previewed while
    /// its row in the menu is hovered.
    shown: &'static Scheme,
    /// The scheme the pointer asks for, and since when: worn once it has
    /// asked for PREVIEW_AFTER.
    wanted: Option<(&'static Scheme, f64)>,
    /// Which schemes its menu shows.
    filter: Filter,
}

impl SchemeMenu {
    pub fn new(scheme: &'static Scheme) -> Self {
        Self {
            scheme,
            shown: scheme,
            wanted: None,
            filter: Filter::default(),
        }
    }

    /// The scheme and filter kept from last time (`get` reads the
    /// viewer's preferences; the system's mode decides without them).
    pub fn restored(get: impl Fn(&str) -> Option<String>, system_dark: bool) -> Self {
        let (scheme, filter) = restored(get, system_dark);
        let mut me = Self::new(scheme);
        me.filter = filter;
        me
    }

    /// Keep the preferences (the keys they have always had).
    pub fn keep(&self, set: &mut dyn FnMut(&str, String)) {
        set(SCHEME_KEY, self.scheme.key());
        set(FILTER_KEY, self.filter.name().to_string());
        set("dark", self.scheme.dark.to_string());
    }

    /// The scheme in force.
    pub fn in_force(&self) -> &'static Scheme {
        self.scheme
    }

    /// Whether the scheme in force is a dark one.
    pub fn dark(&self) -> bool {
        self.scheme.dark
    }

    /// Wear the color scheme kept as `key` ("Family/Variant"; tests).
    pub fn set_scheme(&mut self, ctx: &egui::Context, key: &str) {
        self.choose(ctx, Scheme::from_key(key, self.scheme.dark));
    }

    /// The color scheme in force, as kept ("Family/Variant").
    pub fn scheme_key(&self) -> String {
        self.scheme.key()
    }

    /// The scheme being worn ("Family/Variant"): a previewed one while
    /// its row is hovered (tests).
    pub fn shown_scheme_key(&self) -> String {
        self.shown.key()
    }

    fn choose(&mut self, ctx: &egui::Context, scheme: &'static Scheme) {
        self.scheme = scheme;
        self.wear(ctx, scheme);
    }

    /// Wear `scheme`, in force or previewed.
    fn wear(&mut self, ctx: &egui::Context, scheme: &'static Scheme) {
        self.shown = scheme;
        self.wanted = None;
        theme::apply_scheme(ctx, scheme);
    }

    /// The preview (owner, 2026-10-19: "when we hover over a scheme for
    /// some time switch to it live; when we switch out of it go to either
    /// the new thing we're hovering or back to the current one"). While
    /// the menu is open, the scheme whose row is hovered, else the one in
    /// force, is worn once the pointer has asked for it PREVIEW_AFTER
    /// without change; the menu closed, the one in force is worn at once.
    /// A preview is never kept: only a click chooses.
    fn preview(&mut self, ctx: &egui::Context, open: bool, hovered: Option<&'static Scheme>) {
        let want = match (open, hovered) {
            (true, Some(s)) => s,
            _ => self.scheme,
        };
        if std::ptr::eq(want, self.shown) {
            self.wanted = None;
            return;
        }
        if !open {
            self.wear(ctx, want);
            return;
        }
        let now = ctx.input(|i| i.time);
        match self.wanted {
            Some((s, since)) if std::ptr::eq(s, want) => {
                let waited = now - since;
                if waited >= PREVIEW_AFTER {
                    self.wear(ctx, want);
                } else {
                    ctx.request_repaint_after(std::time::Duration::from_secs_f64(
                        PREVIEW_AFTER - waited,
                    ));
                }
            }
            _ => {
                self.wanted = Some((want, now));
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(PREVIEW_AFTER));
            }
        }
    }

    /// The color scheme button and its menu: a filter (all, light, dark),
    /// then each family with a flyout of its variants. `compact`: the
    /// narrow rail's icon alone.
    pub fn menu(&mut self, ui: &mut Ui, compact: bool) {
        let p = pal(ui.ctx());
        let text = if compact {
            RichText::new(icon::PALETTE).size(16.0).color(p.muted)
        } else {
            RichText::new(format!("{} {}", icon::PALETTE, self.scheme.label()))
                .font(theme::regular(12.5))
                .color(p.muted)
        };
        let mut button = egui::Button::new(text).frame(false);
        if compact {
            button = button.min_size(vec2(32.0, ui.spacing().interact_size.y + 4.0));
        }
        // Open while the filter changes; closed when a scheme is chosen.
        let config = egui::containers::menu::MenuConfig::new()
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
        let mut chosen = None;
        let mut hovered = None;
        let (response, open) = egui::containers::menu::MenuButton::from_button(button)
            .config(config)
            .ui(ui, |ui| {
                ui.set_min_width(200.0);
                ui.horizontal(|ui| {
                    for f in [Filter::All, Filter::Light, Filter::Dark] {
                        ui.selectable_value(&mut self.filter, f, f.name());
                    }
                });
                ui.separator();
                // Every family on screen at once, in as many columns as
                // the window's height needs: nothing hides below a scroll.
                let families = schemes::families(self.filter);
                let rows = (((ui.ctx().content_rect().height() - 160.0) / FAMILY_ROW).floor()
                    as usize)
                    .clamp(6, families.len().max(6));
                let font = theme::regular(14.0);
                let check = ui
                    .painter()
                    .layout_no_wrap(icon::CHECK.into(), font.clone(), p.accent)
                    .size()
                    .x;
                ui.horizontal_top(|ui| {
                    for column in families.chunks(rows) {
                        // Each family's button as wide as its column, its
                        // flyout opening beside the column.
                        ui.allocate_ui_with_layout(
                            vec2(170.0, 0.0),
                            Layout::top_down_justified(Align::LEFT),
                            |ui| {
                                ui.set_width(170.0);
                                for (family, variants) in column {
                                    let here = *family == self.scheme.family
                                        && self.filter.shows(self.scheme);
                                    // Only the family in force is checked; the
                                    // others are indented by the check's width,
                                    // so names line up and a screen reader hears
                                    // the check only where it is.
                                    let mut label = egui::text::LayoutJob::default();
                                    if here {
                                        label.append(
                                            icon::CHECK,
                                            0.0,
                                            egui::TextFormat::simple(font.clone(), p.accent),
                                        );
                                    }
                                    let indent = if here { 8.0 } else { check + 8.0 };
                                    label.append(
                                        family,
                                        indent,
                                        egui::TextFormat::simple(font.clone(), p.text),
                                    );
                                    ui.menu_button(label, |ui| {
                                        for v in variants {
                                            let row =
                                                scheme_row(ui, v, std::ptr::eq(*v, self.scheme));
                                            if row.hovered() {
                                                hovered = Some(*v);
                                            }
                                            if row.clicked() {
                                                chosen = Some(*v);
                                                ui.close();
                                            }
                                        }
                                    });
                                }
                            },
                        );
                    }
                });
            });
        response.on_hover_text(format!("Color scheme: {}", self.scheme.label()));
        match chosen {
            Some(v) => self.choose(ui.ctx(), v),
            None => self.preview(ui.ctx(), open.is_some(), hovered),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn kept(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn the_kept_scheme_and_filter_come_back() {
        let (s, f) = restored(
            kept(&[
                ("scheme", "Ayu/Mirage"),
                ("scheme_filter", "Dark"),
                ("dark", "false"),
            ]),
            false,
        );
        assert_eq!(s.key(), "Ayu/Mirage", "the scheme wins over the old choice");
        assert_eq!(f, Filter::Dark);
    }

    #[test]
    fn before_schemes_the_light_or_dark_choice_is_kept() {
        let (s, f) = restored(kept(&[("dark", "true")]), false);
        assert!(std::ptr::eq(s, schemes::own(true)));
        assert_eq!(f, Filter::All);
        let (s, _) = restored(kept(&[("dark", "false")]), true);
        assert!(std::ptr::eq(s, schemes::own(false)));
    }

    #[test]
    fn with_nothing_kept_the_systems_mode_decides() {
        for dark in [false, true] {
            let (s, f) = restored(kept(&[]), dark);
            assert!(std::ptr::eq(s, schemes::own(dark)));
            assert_eq!(f, Filter::All);
        }
    }

    #[test]
    fn a_lost_scheme_or_filter_falls_back() {
        let (s, f) = restored(
            kept(&[
                ("scheme", "Gone/Away"),
                ("scheme_filter", "Sepia"),
                ("dark", "true"),
            ]),
            false,
        );
        assert!(std::ptr::eq(s, schemes::own(true)));
        assert_eq!(f, Filter::All);
    }
}
