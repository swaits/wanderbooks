//! The tab bar along the top of the window: one tab per open company, a
//! close control on each, and a "+" that lists the companies of the data
//! root that are not open. It runs from the sidebar's right edge to the
//! window's right edge: the sidebar keeps the window's whole height, which
//! the owner's 1479x519 window needs (`tests/sidenav.rs`: with the bar
//! above the sidebar the links scroll there, even with a bar of 24
//! points); with no company open there is no sidebar and it runs the whole
//! width.
//!
//! The bar spans the whole window, above the sidebar: a tab is the top
//! level, and the sidebar (its pages, its status card) is the company in
//! the tab's, which the layout shows. The tab is the company's identity
//! in the frame (badge and name); nothing else at the top of the window
//! repeats it, and the status card at the foot of the sidebar names it
//! again, in the same hue. With no company open
//! it is the window's top all the same. The "+" follows the last tab, and
//! stays at the right edge, whole, while the tabs scroll.
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
//!      company's badge (its hue, from the id of its books: theme.rs
//!      `hue_of`; the status card draws the same) and name (the name last known, else the id of its books, until it is
//!      read). The bar is there with one company open, and with none.
//!   2. The tab shown is marked by more than colour: a heavier name, a
//!      bar in the accent colour along its top edge, and "selected" to
//!      assistive technology.
//!   3. Each tab has its own close control, labelled for assistive
//!      technology with the company's name; a middle click on a tab
//!      closes it as well.
//!   4. "+" lists only companies of the data root that are not open
//!      (a company can't be opened twice), by name with the id of the
//!      books beside it (the id alone for a company never opened), and
//!      says so when there are none.
//!   5. Tabs never overlap or leave the bar: each is as wide as its
//!      content between a least and a greatest width (a longer name is
//!      cut, whole in its hover text), then the strip scrolls, with a
//!      step control at each end; the tab shown is scrolled into view
//!      when it changes. Each tab leaves room left of its close control
//!      for a state mark (drawn by a later unit, nothing now).
//!   6. This only says what was asked (show, close, open); the app does
//!      it.
//!   7. A tab whose company has a job under way that changes its books
//!      (`Tab::held`) has its close control off: disabled to assistive
//!      technology, drawn faint, no click and no middle click closes it,
//!      and its hover text says what is running.
//! ```

use crate::theme::{self, pal};
use egui::{
    Align, CornerRadius, Frame, Layout, Margin, RichText, Sense, Stroke, Ui, UiBuilder, vec2,
};
use egui_phosphor::regular as icon;
use std::time::{Duration, Instant};

/// The bar's height: what the bar takes from the window, which the tests
/// of the sidebar's fit add to their window heights (D-78).
pub const BAR_H: f32 = 36.0;
/// A tab's widest and least widths (a name is cut with an ellipsis to fit).
const TAB_MAX: f32 = 240.0;
const TAB_MIN: f32 = 110.0;
/// The close control's size.
const CLOSE: f32 = 20.0;
/// The badge's size, and its gap to the name.
const BADGE: f32 = 18.0;
const GAP: f32 = 8.0;
/// Room left of the close control for the connection state, which a later
/// unit draws there.
const STATE_W: f32 = 14.0;
/// The room round a tab's content: left and right.
const PAD_L: f32 = 10.0;
const PAD_R: f32 = 6.0;
/// The "+"'s width, and the gap before it.
const PLUS_W: f32 = 30.0;
const PLUS_GAP: f32 = 6.0;
/// The "+" list's width.
const LIST_W: f32 = 260.0;
/// The gap between tabs.
const TAB_GAP: f32 = 2.0;
/// How long a reading of the data root's companies is kept while "+" is
/// open.
const LIST_KEPT: Duration = Duration::from_millis(750);

/// What a tab shows of its company.
pub struct Tab {
    /// The company's name, else the id of its books.
    pub title: String,
    /// More about it, on hover.
    pub detail: String,
    /// What its badge's hue is chosen from: the id of its books
    /// ([`hue_of`]).
    pub hue_key: String,
    /// Why the tab can't be closed now: a job that changes its books is
    /// running ("Restoring these books: wait for it to finish"). Its close
    /// control is off, and says so on hover.
    pub held: Option<String>,
}

/// The badge hue of the company whose books have id `book_id`, light or
/// dark: the one its tab and its status card both draw (theme.rs).
pub fn hue_of(book_id: &str, dark: bool) -> egui::Color32 {
    theme::hue_of(book_id, dark)
}

/// A company of the data root that is not open, as "+" lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    /// The id of its books.
    pub id: String,
    /// Its name as last known, if it was ever opened here.
    pub name: Option<String>,
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
    listed: Option<(Instant, Vec<Listed>)>,
}

impl TabBar {
    /// The shown tab changed (or a tab was added): scroll it into view.
    pub fn reveal(&mut self) {
        self.reveal = true;
    }

    /// The bar, across the top of the window. `shown` is the tab in
    /// front, if there is one; `others` reads the companies of the root
    /// that are not open (called now and then while "+" is open, never
    /// otherwise).
    pub fn show(
        &mut self,
        ui: &mut Ui,
        tabs: &[Tab],
        shown: Option<usize>,
        others: &dyn Fn() -> Vec<Listed>,
    ) -> Option<TabAction> {
        let p = pal(ui.ctx());
        let mut action = None;
        let mut reveal = std::mem::take(&mut self.reveal);
        egui::Panel::top("company_tabs")
            .exact_size(BAR_H)
            .resizable(false)
            .frame(Frame::new().inner_margin(Margin::symmetric(8, 0)))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(TAB_GAP, 0.0);
                let bar = ui.max_rect();
                // The line along the bar's foot; the tab in front covers
                // it, and so is one with what is below.
                let outer = ui.clip_rect();
                ui.painter().rect_filled(outer, 0.0, p.sidebar);
                ui.painter().hline(
                    outer.x_range(),
                    outer.bottom() - 0.5,
                    Stroke::new(1.0, p.border),
                );
                // The tabs as wide as they are, the "+" right after them;
                // when they don't fit, the "+" stays at the right edge
                // and the tabs scroll.
                let widths: Vec<f32> = tabs.iter().map(|t| tab_width(ui, t)).collect();
                let total: f32 =
                    widths.iter().sum::<f32>() + widths.len().saturating_sub(1) as f32 * TAB_GAP;
                let room = (ui.available_width() - PLUS_W - PLUS_GAP).max(0.0);
                let strip_w = total.min(room);
                let strip = egui::Rect::from_min_size(bar.min, vec2(strip_w, bar.height()));
                let mut strip_ui = ui.new_child(
                    UiBuilder::new()
                        .max_rect(strip)
                        .layout(Layout::left_to_right(Align::Max)),
                );
                strip_ui.set_clip_rect(strip);
                self.strip(
                    &mut strip_ui,
                    tabs,
                    &widths,
                    total > room + 0.5,
                    shown,
                    &mut reveal,
                    &mut action,
                );
                let plus = egui::Rect::from_min_size(
                    egui::pos2(strip.right() + PLUS_GAP, bar.top()),
                    vec2(PLUS_W, bar.height()),
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
    #[allow(clippy::too_many_arguments)]
    fn strip(
        &mut self,
        ui: &mut Ui,
        tabs: &[Tab],
        widths: &[f32],
        overflows: bool,
        shown: Option<usize>,
        reveal: &mut bool,
        action: &mut Option<TabAction>,
    ) {
        let p = pal(ui.ctx());
        let room = ui.available_width();
        // A step control at each end while the tabs overflow.
        let step_w = if overflows { 22.0 } else { 0.0 };
        let step_by = widths.first().copied().unwrap_or(TAB_MIN) + TAB_GAP;
        let area_id = ui.id().with("tab_scroll");
        let mut step = 0.0_f32;
        if overflows {
            let (r, resp) = ui.allocate_exact_size(vec2(step_w, 26.0), Sense::click());
            step_button(ui, r, &resp, icon::CARET_LEFT, "Earlier tabs");
            if resp.clicked() {
                step = -step_by;
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
                ui.spacing_mut().item_spacing.x = TAB_GAP;
                ui.horizontal_top(|ui| {
                    for (i, tab) in tabs.iter().enumerate() {
                        let here = shown == Some(i);
                        let r = one_tab(ui, i, tab, here, widths[i], &p, action);
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
                s.offset.x = (offset + step_by).min(max);
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
        others: &dyn Fn() -> Vec<Listed>,
        action: &mut Option<TabAction>,
    ) {
        let p = pal(ui.ctx());
        let button = egui::Button::new(RichText::new(icon::PLUS).size(16.0).color(p.text))
            .frame(false)
            .min_size(vec2(28.0, 28.0));
        let (response, open) =
            egui::containers::menu::MenuButton::from_button(button).ui(ui, |ui| {
                ui.set_min_width(LIST_W);
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
                    .map(|(_, l)| l.clone())
                    .unwrap_or_default();
                if list.is_empty() {
                    ui.label(
                        RichText::new("Every company is already open.")
                            .font(theme::regular(13.0))
                            .color(p.muted),
                    );
                } else {
                    ui.label(
                        RichText::new("Open a company")
                            .font(theme::regular(12.0))
                            .color(p.muted),
                    );
                    for c in &list {
                        if listed_row(ui, c, &p).clicked() {
                            *action = Some(TabAction::Open(c.id.clone()));
                            ui.close();
                        }
                    }
                }
            });
        // Forget the list once it is closed, so it is read afresh.
        if open.is_none() {
            self.listed = None;
        }
        // Named as it always was for assistive technology; the hover says
        // what it does.
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Open another company")
        });
        response.on_hover_text("Open a company");
    }
}

/// One company in the "+" list: its name, the id of its books beside it
/// in small text (the id alone when the name was never known).
fn listed_row(ui: &mut Ui, c: &Listed, p: &theme::Palette) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(LIST_W, 28.0), Sense::click());
    let primary = c.name.as_deref().unwrap_or(&c.id);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, primary));
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(6), p.hover);
    }
    let mut right = rect.right() - 8.0;
    if c.name.is_some() {
        let g = ui
            .painter()
            .layout_no_wrap(c.id.clone(), theme::regular(11.0), p.muted);
        right -= g.size().x;
        ui.painter().galley(
            egui::pos2(right, rect.center().y - g.size().y / 2.0),
            g,
            p.muted,
        );
    }
    let mut job = egui::text::LayoutJob::single_section(
        primary.to_string(),
        egui::TextFormat::simple(theme::regular(13.0), p.text),
    );
    job.wrap.max_width = (right - 8.0 - (rect.left() + 8.0)).max(10.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let g = ui.painter().layout_job(job);
    ui.painter().galley(
        egui::pos2(rect.left() + 8.0, rect.center().y - g.size().y / 2.0),
        g,
        p.text,
    );
    resp
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

/// What a tab holds besides its name: the badge, the room for the state
/// mark, the close control and the room round them.
const TAB_FIXED: f32 = PAD_L + BADGE + GAP + GAP + STATE_W + CLOSE + PAD_R;

/// A tab's width: its content's, between the least and the greatest. The
/// name is measured in the heavier face for every tab, so the tab in front
/// doesn't move the others.
fn tab_width(ui: &Ui, tab: &Tab) -> f32 {
    let g = ui.painter().layout_no_wrap(
        tab.title.clone(),
        theme::semibold(13.0),
        egui::Color32::PLACEHOLDER,
    );
    (TAB_FIXED + g.size().x.ceil()).clamp(TAB_MIN, TAB_MAX)
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
    let height = BAR_H - 4.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    // The tab's own id for its close control: tabs are told apart by
    // their place, and two tabs never share one.
    let close_rect = egui::Rect::from_center_size(
        egui::pos2(rect.right() - PAD_R - CLOSE / 2.0, rect.center().y),
        vec2(CLOSE, CLOSE),
    );
    let close = ui.interact(close_rect, resp.id.with("close"), Sense::click());
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, here, &tab.title)
    });
    close.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            tab.held.is_none(),
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
        if here {
            // The lit tab: the page's surface, outlined on its top and
            // sides, open at the foot where it meets what is below, with a
            // bar in the accent colour along its top edge.
            painter.rect_filled(rect, top, p.surface);
            painter.rect_stroke(
                rect,
                top,
                Stroke::new(1.0, p.border),
                egui::StrokeKind::Inside,
            );
            painter.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(rect.left() + 1.0, rect.bottom() - 1.0),
                    egui::pos2(rect.right() - 1.0, rect.bottom() + 1.0),
                ),
                0.0,
                p.surface,
            );
            painter.rect_filled(
                egui::Rect::from_min_size(rect.min, vec2(rect.width(), 3.0)),
                top,
                p.accent,
            );
        } else if resp.hovered() || close.hovered() {
            painter.rect_filled(rect.shrink2(vec2(0.0, 2.0)), CornerRadius::same(8), p.hover);
        }
        // The badge: the company's hue, as its status card draws it.
        let badge = egui::Rect::from_center_size(
            egui::pos2(rect.left() + PAD_L + BADGE / 2.0, rect.center().y + 1.0),
            vec2(BADGE, BADGE),
        );
        let initial = tab
            .title
            .chars()
            .next()
            .map(|c| c.to_uppercase().next().unwrap_or(c))
            .unwrap_or('W');
        theme::paint_badge(
            painter,
            badge,
            5,
            initial,
            theme::badge_hue(ui.ctx(), &tab.hue_key),
            theme::bold(11.0),
        );
        let font = if here {
            theme::semibold(13.0)
        } else {
            theme::regular(13.0)
        };
        let text_x = badge.right() + GAP;
        let text_w = close_rect.left() - STATE_W - GAP - text_x;
        let mut job = egui::text::LayoutJob::single_section(
            tab.title.clone(),
            egui::TextFormat::simple(font, p.text),
        );
        job.wrap.max_width = text_w.max(10.0);
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        job.wrap.overflow_character = Some('…');
        let galley = painter.layout_job(job);
        painter.galley(
            egui::pos2(text_x, rect.center().y - galley.size().y / 2.0 + 1.0),
            galley,
            p.text,
        );
        if close.hovered() && tab.held.is_none() {
            painter.rect_filled(close_rect, CornerRadius::same(5), p.selected);
        }
        painter.text(
            close_rect.center(),
            egui::Align2::CENTER_CENTER,
            icon::X,
            egui::FontId::proportional(12.0),
            if tab.held.is_some() {
                p.faint
            } else if close.hovered() || here {
                p.text
            } else {
                p.muted
            },
        );
    }
    let resp = resp.on_hover_text(&tab.detail);
    // The close control sits over the tab and is the one under the pointer
    // there: a click on it is its own, never the tab's.
    let close_tip = match &tab.held {
        Some(why) => why.clone(),
        None => format!("Close {}", tab.title),
    };
    let closes = close.on_hover_text(close_tip).clicked() || resp.middle_clicked();
    if closes && tab.held.is_none() {
        *action = Some(TabAction::Close(i));
    } else if closes {
        // Held open: the click is the close control's, and does nothing.
    } else if resp.clicked() {
        *action = Some(TabAction::Show(i));
    }
    resp
}
