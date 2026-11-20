//! The tab bar along the top of the window: one tab per open company, a
//! close control on each, and a "+" that lists the companies of the data
//! root that are not open. It runs from the sidebar's right edge to the
//! window's right edge: the sidebar keeps the window's whole height, which
//! the owner's 1479x519 window needs (`tests/sidenav.rs`: with the bar
//! above the sidebar the links scroll there, even with a bar of 24
//! points); with no company open there is no sidebar and it runs the whole
//! width.
//!
//! ```text
//! GOAL: a person has several companies open at once, each in its own
//!       tab, and moves between them, opens and closes them, without
//!       any one disturbing another.
//! SOURCE: plan/p15-app-and-relay.md 3.2 and 3.3 (a tab per open
//!         company); D-78 (one top-level tab per open company: a
//!         company is reachable exactly while it has a tab).
//! INVARIANTS:
//!   1. One tab per open company, in the order opened, showing the
//!      company's name (the id of its books until the name is read). The
//!      bar is there with one company open, and with none.
//!   2. The tab shown is marked by more than colour: a heavier name, a
//!      bar in the accent colour along its top edge, and "selected" to
//!      assistive technology.
//!   3. Each tab has its own close control, labelled for assistive
//!      technology with the company's name; a middle click on a tab
//!      closes it as well.
//!   4. "+" lists only companies of the data root that are not open
//!      (a company can't be opened twice), and says so when there are
//!      none.
//!   5. Tabs never overlap or leave the bar: they shrink to a least
//!      width, then the strip scrolls, with a step control at each end
//!      that is clipped; the tab shown is scrolled into view when it
//!      changes.
//!   6. This only says what was asked (show, close, open); the app does
//!      it.
//! ```

use crate::theme::{self, pal};
use egui::{
    Align, CornerRadius, Frame, Layout, Margin, RichText, Sense, Stroke, Ui, UiBuilder, vec2,
};
use egui_phosphor::regular as icon;
use std::time::{Duration, Instant};

/// The bar's height.
pub const BAR_H: f32 = 36.0;
/// A tab's widest and least widths (a name is cut with an ellipsis to fit).
const TAB_MAX: f32 = 220.0;
const TAB_MIN: f32 = 120.0;
/// The close control's size.
const CLOSE: f32 = 20.0;
/// How long a reading of the data root's companies is kept while "+" is
/// open.
const LIST_KEPT: Duration = Duration::from_millis(750);

/// What a tab shows of its company.
pub struct Tab {
    /// The company's name, else the id of its books.
    pub title: String,
    /// More about it, on hover.
    pub detail: String,
}

/// What a person asked of the bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TabAction {
    /// Show tab `n`.
    Show(usize),
    /// Close tab `n`.
    Close(usize),
    /// Open the company of the data root with this id.
    Open(String),
}

/// The bar's memory: whether to scroll the shown tab into view, and the
/// data root's companies as last read for "+".
#[derive(Default)]
pub struct TabBar {
    reveal: bool,
    listed: Option<(Instant, Vec<String>)>,
}

impl TabBar {
    /// The shown tab changed (or a tab was added): scroll it into view.
    pub fn reveal(&mut self) {
        self.reveal = true;
    }

    /// The bar. `shown` is the tab in front, if there is one; `others`
    /// reads the ids of the root's companies that are not open (called
    /// now and then while "+" is open, never otherwise).
    pub fn show(
        &mut self,
        ui: &mut Ui,
        tabs: &[Tab],
        shown: Option<usize>,
        others: &dyn Fn() -> Vec<String>,
    ) -> Option<TabAction> {
        let p = pal(ui.ctx());
        let mut action = None;
        let mut reveal = std::mem::take(&mut self.reveal);
        egui::Panel::top("company_tabs")
            .exact_size(BAR_H)
            .resizable(false)
            .frame(
                Frame::new()
                    .fill(p.sidebar)
                    .inner_margin(Margin::symmetric(8, 0))
                    .stroke(Stroke::new(1.0, p.border)),
            )
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(2.0, 0.0);
                let bar = ui.max_rect();
                // The "+" at the right end, always whole; the tabs get the
                // rest.
                let plus_w = 30.0;
                let strip_w = (ui.available_width() - plus_w - 6.0).max(0.0);
                let strip = egui::Rect::from_min_size(bar.min, vec2(strip_w, bar.height()));
                let mut strip_ui = ui.new_child(
                    UiBuilder::new()
                        .max_rect(strip)
                        .layout(Layout::left_to_right(Align::Max)),
                );
                strip_ui.set_clip_rect(strip);
                self.strip(&mut strip_ui, tabs, shown, &mut reveal, &mut action);
                let plus = egui::Rect::from_min_size(
                    egui::pos2(strip.right() + 6.0, bar.top()),
                    vec2(plus_w, bar.height()),
                );
                let mut plus_ui = ui.new_child(
                    UiBuilder::new()
                        .max_rect(plus)
                        .layout(Layout::left_to_right(Align::Center)),
                );
                self.plus(&mut plus_ui, others, &mut action);
                ui.allocate_rect(bar, Sense::hover());
            });
        if reveal {
            // The tab wasn't there to scroll to yet: try again next frame.
            self.reveal = true;
        }
        action
    }

    /// The tabs, scrolling sideways when they don't fit.
    fn strip(
        &mut self,
        ui: &mut Ui,
        tabs: &[Tab],
        shown: Option<usize>,
        reveal: &mut bool,
        action: &mut Option<TabAction>,
    ) {
        let p = pal(ui.ctx());
        let room = ui.available_width();
        let n = tabs.len().max(1) as f32;
        let each = ((room - (n - 1.0) * 2.0) / n).clamp(TAB_MIN, TAB_MAX);
        let overflows = each * n + (n - 1.0) * 2.0 > room + 0.5;
        // A step control at each end while the tabs overflow.
        let step_w = if overflows { 22.0 } else { 0.0 };
        let area_id = ui.id().with("tab_scroll");
        let mut step = 0.0_f32;
        if overflows {
            let (r, resp) = ui.allocate_exact_size(vec2(step_w, 26.0), Sense::click());
            step_button(ui, r, &resp, icon::CARET_LEFT, "Earlier tabs");
            if resp.clicked() {
                step = -each;
            }
        }
        let view_w = room - 2.0 * step_w;
        let mut state = egui::scroll_area::State::load(ui.ctx(), area_id).unwrap_or_default();
        if step != 0.0 {
            state.offset.x = (state.offset.x + step).max(0.0);
            state.store(ui.ctx(), area_id);
        }
        let out = egui::ScrollArea::horizontal()
            .id_salt("tab_scroll")
            .max_width(view_w.max(0.0))
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.horizontal_top(|ui| {
                    for (i, tab) in tabs.iter().enumerate() {
                        let here = shown == Some(i);
                        let r = one_tab(ui, i, tab, here, each, &p, action);
                        if here && *reveal {
                            r.scroll_to_me(Some(Align::Center));
                            *reveal = false;
                        }
                    }
                });
            });
        if overflows {
            let offset = out.state.offset.x;
            let max = (out.content_size.x - out.inner_rect.width()).max(0.0);
            let (r, resp) = ui.allocate_exact_size(vec2(step_w, 26.0), Sense::click());
            step_button(ui, r, &resp, icon::CARET_RIGHT, "Later tabs");
            if resp.clicked() {
                let mut s = out.state;
                s.offset.x = (offset + each).min(max);
                s.store(ui.ctx(), out.id);
            }
        }
        if tabs.is_empty() {
            *reveal = false;
        }
    }

    /// The "+" and its list.
    fn plus(
        &mut self,
        ui: &mut Ui,
        others: &dyn Fn() -> Vec<String>,
        action: &mut Option<TabAction>,
    ) {
        let p = pal(ui.ctx());
        let button = egui::Button::new(RichText::new(icon::PLUS).size(16.0).color(p.muted))
            .frame(false)
            .min_size(vec2(28.0, 28.0));
        let (response, open) =
            egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
                ui.set_min_width(200.0);
                let stale = self
                    .listed
                    .as_ref()
                    .is_none_or(|(at, _)| at.elapsed() > LIST_KEPT);
                if stale {
                    self.listed = Some((Instant::now(), others()));
                }
                ui.ctx().request_repaint_after(LIST_KEPT);
                let list = self
                    .listed
                    .as_ref()
                    .map(|(_, l)| l.as_slice())
                    .unwrap_or(&[]);
                if list.is_empty() {
                    ui.label(
                        RichText::new("Every company here is open.")
                            .font(theme::regular(13.0))
                            .color(p.muted),
                    );
                } else {
                    ui.label(
                        RichText::new("Open a company")
                            .font(theme::regular(12.0))
                            .color(p.muted),
                    );
                    for id in list {
                        if ui.button(id).clicked() {
                            *action = Some(TabAction::Open(id.clone()));
                            ui.close();
                        }
                    }
                }
            });
        // Forget the list once it is closed, so it is read afresh.
        if open.is_none() {
            self.listed = None;
        }
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Open another company")
        });
        response.on_hover_text("Open another company");
    }
}

/// A step control at an end of the strip.
fn step_button(ui: &Ui, r: egui::Rect, resp: &egui::Response, glyph: &str, label: &'static str) {
    let p = pal(ui.ctx());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(6), p.hover);
    }
    ui.painter().text(
        r.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        egui::FontId::proportional(13.0),
        p.muted,
    );
}

/// One tab, `width` wide.
fn one_tab(
    ui: &mut Ui,
    i: usize,
    tab: &Tab,
    here: bool,
    width: f32,
    p: &theme::Palette,
    action: &mut Option<TabAction>,
) -> egui::Response {
    let height = BAR_H - 6.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    // The tab's own id for its close control: tabs are told apart by
    // their place, and two tabs never share one.
    let close_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - 6.0 - CLOSE / 2.0, rect.center().y),
        vec2(CLOSE, CLOSE),
    );
    let close = ui.interact(close_rect, resp.id.with("close"), Sense::click());
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, here, &tab.title)
    });
    close.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("Close {}", tab.title),
        )
    });
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let top = CornerRadius {
            nw: 8,
            ne: 8,
            sw: 0,
            se: 0,
        };
        let fill = if here {
            Some(p.bg)
        } else if resp.hovered() || close.hovered() {
            Some(p.hover)
        } else {
            None
        };
        if let Some(fill) = fill {
            painter.rect_filled(rect, top, fill);
        }
        if here {
            // A bar along the top edge and the page's own fill: the tab
            // in front is part of the page below it.
            painter.rect_filled(
                egui::Rect::from_min_size(rect.min, vec2(rect.width(), 3.0)),
                CornerRadius {
                    nw: 8,
                    ne: 8,
                    sw: 0,
                    se: 0,
                },
                p.accent,
            );
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(rect.left(), rect.bottom() - 1.0),
                    egui::pos2(rect.right(), rect.bottom() + 1.0),
                ),
                0.0,
                p.bg,
            );
        }
        let font = if here {
            theme::semibold(13.0)
        } else {
            theme::regular(13.0)
        };
        let text_w = rect.width() - 14.0 - CLOSE - 10.0;
        let mut job = egui::text::LayoutJob::single_section(
            tab.title.clone(),
            egui::TextFormat::simple(font, if here { p.text } else { p.muted }),
        );
        job.wrap.max_width = text_w.max(10.0);
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        job.wrap.overflow_character = Some('…');
        let galley = painter.layout_job(job);
        painter.galley(
            egui::pos2(
                rect.left() + 14.0,
                rect.center().y - galley.size().y / 2.0 + 1.0,
            ),
            galley,
            p.text,
        );
        if close.hovered() {
            painter.rect_filled(close_rect, CornerRadius::same(5), p.selected);
        }
        painter.text(
            close_rect.center(),
            egui::Align2::CENTER_CENTER,
            icon::X,
            egui::FontId::proportional(12.0),
            if close.hovered() { p.text } else { p.muted },
        );
    }
    let resp = resp.on_hover_text(&tab.detail);
    // The close control sits over the tab and is the one under the pointer
    // there: a click on it is its own, never the tab's.
    if close
        .on_hover_text(format!("Close {}", tab.title))
        .clicked()
        || resp.middle_clicked()
    {
        *action = Some(TabAction::Close(i));
    } else if resp.clicked() {
        *action = Some(TabAction::Show(i));
    }
    resp
}
