//! The window: sidebar, top bar, and the current page.

use crate::books::Books;
use crate::data::{Loader, Source, Update};
use crate::nav::{Nav, Route, Section};
use crate::pages;
use crate::receipts::Receipts;
use crate::schemes::{self, Filter, Scheme};
use crate::theme::{self, pal};
use egui::{Align, CornerRadius, Frame, Key, Layout, Margin, RichText, Sense, Stroke, Ui, vec2};
use egui_phosphor::regular as icon;
use std::sync::Arc;

/// Window widths below this get the icon rail instead of the sidebar.
pub const COMPACT_BELOW: f32 = 1000.0;

/// Per-page state that survives navigation.
#[derive(Default)]
pub struct PageState {
    pub transactions: pages::transactions::State,
    pub reports: pages::reports::State,
    pub accounts: pages::accounts::State,
    pub assets: pages::assets::State,
    pub parties: pages::parties::State,
    pub documents: pages::documents::State,
    pub journal: pages::journal::State,
}

/// What a page can ask of the shell.
pub struct Shell<'a> {
    pub receipts: &'a mut Receipts,
    pub state: &'a mut PageState,
    pub go: Option<Route>,
    /// Seconds since the last live update arrived (for "new" highlights).
    pub since_update: f64,
}

impl Shell<'_> {
    pub fn go(&mut self, r: Route) {
        self.go = Some(r);
    }
}

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

pub struct App {
    loader: Option<Loader>,
    books: Option<Arc<Books>>,
    error: Option<String>,
    nav: Nav,
    receipts: Receipts,
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
    state: PageState,
    palette: crate::palette::Palette,
    updated_at: f64,
    live_label: bool,
    /// The export menu (menu.rs).
    menu: crate::menu::Menu,
    /// The owner's Books menu (owner.rs).
    owner_menu: crate::owner::OwnerMenu,
}

impl App {
    /// The live app over a books directory.
    pub fn new(cc: &eframe::CreationContext<'_>, source: Source) -> Self {
        // The scheme kept from last time; before schemes, a light/dark
        // choice; else the system's mode (schemes INVARIANT 4).
        let (scheme, filter) = restored(
            |key| cc.storage.and_then(|s| s.get_string(key)),
            cc.egui_ctx.global_style().visuals.dark_mode,
        );
        let mut app = Self::bare(&cc.egui_ctx, scheme);
        app.filter = filter;
        app.loader = Some(Loader::spawn(source, cc.egui_ctx.clone()));
        app
    }

    /// An app showing fixed books (tests and screenshots).
    pub fn with_books(ctx: &egui::Context, books: Books, dark: bool, start: Route) -> Self {
        let mut app = Self::bare(ctx, schemes::own(dark));
        app.books = Some(Arc::new(books));
        app.nav = Nav::new(start);
        app
    }

    fn bare(ctx: &egui::Context, scheme: &'static Scheme) -> Self {
        theme::install_fonts(ctx);
        theme::apply_scheme(ctx, scheme);
        egui_extras::install_image_loaders(ctx);
        Self {
            loader: None,
            books: None,
            error: None,
            nav: Nav::new(Route::Overview),
            receipts: Receipts::new(ctx),
            scheme,
            shown: scheme,
            wanted: None,
            filter: Filter::default(),
            state: PageState::default(),
            palette: Default::default(),
            updated_at: -1e9,
            live_label: false,
            menu: Default::default(),
            owner_menu: Default::default(),
        }
    }

    /// Navigate (tests).
    pub fn goto(&mut self, r: Route) {
        self.nav.go(r);
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
    fn scheme_menu(&mut self, ui: &mut Ui, compact: bool) {
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
            button = button.min_size(vec2(32.0, 32.0));
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

    /// Block until every receipt on screen has rendered (screenshots).
    pub fn settle(&mut self, ctx: &egui::Context) {
        self.receipts.settle(ctx);
    }

    /// Go where the agent asked (`books.show`).
    pub fn show(&mut self, target: ledger_store::ops::ViewerTarget) {
        use crate::nav::ReportKind;
        use ledger_store::ops::ViewerTarget as T;
        let Some(b) = self.books.clone() else {
            return;
        };
        let date = |s: &Option<String>| {
            s.as_deref()
                .and_then(|d| d.parse::<chrono::NaiveDate>().ok())
        };
        let route = match target {
            T::Page { page } => match page.as_str() {
                "overview" => Route::Overview,
                "cash" => Route::Cash,
                "profit" => Route::Profit,
                "transactions" => Route::Transactions,
                "reports" => Route::Reports(ReportKind::ProfitAndLoss),
                "accounts" => Route::Accounts,
                "assets" => Route::Assets,
                "parties" => Route::Parties,
                "documents" => Route::Documents,
                "company" => Route::Company,
                "journal" => Route::Journal,
                _ => return,
            },
            T::Entry { entry_id } => Route::Entry(entry_id),
            T::Account { code, start, end } => {
                let Some(a) = b.ledger.chart().find_by_code(&code) else {
                    return;
                };
                self.state.accounts.range = match (date(&start), date(&end)) {
                    (None, None) => None,
                    (s, e) => Some((
                        s.unwrap_or_else(|| b.kpis.first_entry.unwrap_or(b.today)),
                        e.unwrap_or(b.today),
                    )),
                };
                Route::Account(a.id)
            }
            T::Report {
                report,
                start,
                end,
                columns,
                compare,
            } => {
                use crate::pages::reports::{Columns, Compare};
                self.state.reports.columns = match columns.as_deref() {
                    Some("month") => Columns::Month,
                    Some("quarter") => Columns::Quarter,
                    Some("year") => Columns::Year,
                    _ => Columns::Total,
                };
                self.state.reports.compare = match compare.as_deref() {
                    Some("prior_period") => Compare::PriorPeriod,
                    Some("prior_year") => Compare::PriorYear,
                    _ => Compare::None,
                };
                let kind = match report.as_str() {
                    "profit_and_loss" => ReportKind::ProfitAndLoss,
                    "balance_sheet" => ReportKind::BalanceSheet,
                    "cash_flow" => ReportKind::CashFlow,
                    "members_equity" => ReportKind::MembersEquity,
                    "trial_balance" => ReportKind::TrialBalance,
                    "general_ledger" => ReportKind::GeneralLedger,
                    "tax_summary" => ReportKind::TaxSummary,
                    "vendor_1099" => ReportKind::Vendor1099,
                    "ar_aging" => ReportKind::ArAging,
                    "ap_aging" => ReportKind::ApAging,
                    "fixed_asset_register" => ReportKind::FixedAssetRegister,
                    _ => return,
                };
                if let (s, e) = (date(&start), date(&end))
                    && (s.is_some() || e.is_some())
                {
                    let (ds, de) = crate::pages::reports::Range::YearToDate.dates(&b);
                    self.state.reports.range =
                        crate::pages::reports::Range::Custom(s.unwrap_or(ds), e.unwrap_or(de));
                }
                Route::Reports(kind)
            }
            T::Document { attachment_id } => Route::Document(attachment_id),
            T::Asset { asset_id } => {
                if b.ledger.get_asset(asset_id).is_none() {
                    return;
                }
                Route::Asset(asset_id)
            }
            T::Party { party_id } => {
                if b.ledger.get_party(party_id).is_none() {
                    return;
                }
                Route::Party(party_id)
            }
            T::Reconciliation { id } => {
                // Its own page: statement, balance, the lines it cleared.
                if b.reconciliation(id).is_none() {
                    return;
                }
                Route::Reconciliation(id)
            }
            T::Search { query } => {
                let st = &mut self.state.transactions;
                st.query = query;
                st.filter = Default::default();
                st.account = None;
                Route::Transactions
            }
        };
        self.nav.go(route);
    }

    /// Where the app is (tests).
    pub fn route(&self) -> &Route {
        &self.nav.current
    }

    /// Page state (tests).
    /// Label a fixed-books app "Live", as it reads when following a
    /// journal (screenshots of what a running app looks like).
    pub fn show_live_label(&mut self) {
        self.live_label = true;
    }

    pub fn state_mut(&mut self) -> &mut PageState {
        &mut self.state
    }

    fn handle_keys(&mut self, ui: &Ui) {
        let (back, fwd, palette) = ui.input(|i| {
            (
                (i.modifiers.alt && i.key_pressed(Key::ArrowLeft))
                    || i.pointer.button_pressed(egui::PointerButton::Extra1),
                (i.modifiers.alt && i.key_pressed(Key::ArrowRight))
                    || i.pointer.button_pressed(egui::PointerButton::Extra2),
                i.modifiers.command && i.key_pressed(Key::K),
            )
        });
        if back {
            self.nav.back();
        }
        if fwd {
            self.nav.forward();
        }
        if palette {
            self.palette.open();
        }
    }

    /// The sidebar: full below [`COMPACT_BELOW`] window widths, an icon
    /// rail (names in tooltips) when the window is narrower.
    fn sidebar(&mut self, ui: &mut Ui, compact: bool) {
        if compact {
            return self.rail(ui);
        }
        let p = pal(ui.ctx());
        let books = self.books.clone();
        egui::Panel::left("sidebar")
            .exact_size(224.0)
            .resizable(false)
            .frame(
                Frame::new()
                    .fill(p.sidebar)
                    .inner_margin(Margin::symmetric(12, 16)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
                    ui.painter().rect_filled(r, CornerRadius::same(8), p.accent);
                    let initial = books
                        .as_ref()
                        .and_then(|b| b.company_name().chars().next())
                        .unwrap_or('W');
                    ui.painter().text(
                        r.center(),
                        egui::Align2::CENTER_CENTER,
                        initial,
                        theme::bold(15.0),
                        schemes::on(p.accent),
                    );
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let name = books
                            .as_ref()
                            .map(|b| b.company_name().to_string())
                            .unwrap_or_else(|| "wanderbooks".into());
                        ui.add(
                            egui::Label::new(RichText::new(name).font(theme::semibold(13.5)))
                                .truncate(),
                        );
                        ui.label(
                            RichText::new("Books")
                                .font(theme::regular(11.5))
                                .color(p.muted),
                        );
                    });
                });
                ui.add_space(18.0);
                let current = self.nav.current.section();
                let attention = books.as_ref().map(|b| b.attention.len()).unwrap_or(0);
                // Short windows: the links scroll above the status box
                // instead of the box drawing over them.
                let links_h = (ui.available_height() - SIDEBAR_FOOTER).max(64.0);
                egui::ScrollArea::vertical()
                    .id_salt("sidebar_links")
                    .max_height(links_h)
                    .show(ui, |ui| {
                        for s in Section::ALL {
                            let on = s == current;
                            let (rect, resp) = ui.allocate_exact_size(
                                vec2(ui.available_width(), 32.0),
                                Sense::click(),
                            );
                            if on {
                                ui.painter()
                                    .rect_filled(rect, CornerRadius::same(8), p.surface);
                                ui.painter().rect_stroke(
                                    rect,
                                    CornerRadius::same(8),
                                    Stroke::new(1.0, p.border),
                                    egui::StrokeKind::Inside,
                                );
                            } else if resp.hovered() {
                                ui.painter()
                                    .rect_filled(rect, CornerRadius::same(8), p.hover);
                            }
                            let color = if on { p.text } else { p.muted };
                            ui.painter().text(
                                rect.left_center() + vec2(10.0, 0.0),
                                egui::Align2::LEFT_CENTER,
                                s.icon(),
                                egui::FontId::proportional(16.0),
                                if on { p.accent } else { p.muted },
                            );
                            ui.painter().text(
                                rect.left_center() + vec2(36.0, 0.0),
                                egui::Align2::LEFT_CENTER,
                                s.title(),
                                if on {
                                    theme::semibold(13.5)
                                } else {
                                    theme::regular(13.5)
                                },
                                color,
                            );
                            if s == Section::Overview && attention > 0 {
                                let badge = egui::Rect::from_center_size(
                                    rect.right_center() - vec2(18.0, 0.0),
                                    vec2(22.0, 18.0),
                                );
                                ui.painter().rect_filled(
                                    badge,
                                    CornerRadius::same(9),
                                    p.warning_soft,
                                );
                                ui.painter().text(
                                    badge.center(),
                                    egui::Align2::CENTER_CENTER,
                                    attention.to_string(),
                                    theme::semibold(11.0),
                                    p.warning,
                                );
                            }
                            if resp.clicked() {
                                self.nav.go(s.home());
                            }
                        }
                    });
                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    self.scheme_menu(ui, false);
                    if let Some(b) = &books {
                        ui.add_space(2.0);
                        self.menu.ui(ui, b, false);
                        self.owner_menu.ui(ui, b, false);
                    }
                    ui.add_space(6.0);
                    if let Some(b) = &books {
                        live_status(
                            ui,
                            b,
                            ui.input(|i| i.time) - self.updated_at,
                            self.loader.is_some() || self.live_label,
                        );
                    }
                });
            });
    }

    /// The sidebar as a narrow rail of icons, for narrow windows.
    fn rail(&mut self, ui: &mut Ui) {
        let p = pal(ui.ctx());
        let books = self.books.clone();
        egui::Panel::left("rail")
            .exact_size(RAIL_W)
            .resizable(false)
            .frame(
                Frame::new()
                    .fill(p.sidebar)
                    .inner_margin(Margin::symmetric(10, 16)),
            )
            .show(ui, |ui| {
                let name = books
                    .as_ref()
                    .map(|b| b.company_name().to_string())
                    .unwrap_or_else(|| "wanderbooks".into());
                let (r, _) =
                    ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
                let logo = egui::Rect::from_center_size(r.center(), vec2(28.0, 28.0));
                ui.painter()
                    .rect_filled(logo, CornerRadius::same(8), p.accent);
                ui.painter().text(
                    logo.center(),
                    egui::Align2::CENTER_CENTER,
                    name.chars().next().unwrap_or('W'),
                    theme::bold(15.0),
                    schemes::on(p.accent),
                );
                ui.interact(r, ui.id().with("logo"), Sense::hover())
                    .on_hover_text(format!("{name} · Books"));
                ui.add_space(18.0);
                let current = self.nav.current.section();
                let attention = books.as_ref().map(|b| b.attention.len()).unwrap_or(0);
                let links_h = (ui.available_height() - RAIL_FOOTER).max(72.0);
                egui::ScrollArea::vertical()
                    .id_salt("rail_links")
                    .max_height(links_h)
                    .show(ui, |ui| {
                        for s in Section::ALL {
                            let on = s == current;
                            let (rect, resp) = ui.allocate_exact_size(
                                vec2(ui.available_width(), 36.0),
                                Sense::click(),
                            );
                            if on {
                                ui.painter()
                                    .rect_filled(rect, CornerRadius::same(8), p.surface);
                                ui.painter().rect_stroke(
                                    rect,
                                    CornerRadius::same(8),
                                    Stroke::new(1.0, p.border),
                                    egui::StrokeKind::Inside,
                                );
                            } else if resp.hovered() {
                                ui.painter()
                                    .rect_filled(rect, CornerRadius::same(8), p.hover);
                            }
                            ui.painter().text(
                                rect.center(),
                                egui::Align2::CENTER_CENTER,
                                s.icon(),
                                egui::FontId::proportional(18.0),
                                if on { p.accent } else { p.muted },
                            );
                            let tip = if s == Section::Overview && attention > 0 {
                                ui.painter().circle_filled(
                                    rect.center() + vec2(9.0, -8.0),
                                    4.0,
                                    p.warning,
                                );
                                format!("{} · {attention} need attention", s.title())
                            } else {
                                s.title().to_string()
                            };
                            if resp.on_hover_text(tip).clicked() {
                                self.nav.go(s.home());
                            }
                        }
                    });
                ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
                    self.scheme_menu(ui, true);
                    if let Some(b) = &books {
                        self.menu.ui(ui, b, true);
                        self.owner_menu.ui(ui, b, true);
                    }
                    ui.add_space(6.0);
                    if let Some(b) = &books {
                        let live = self.loader.is_some() || self.live_label;
                        let checks = checks(b);
                        let ok = checks.iter().all(|c| c.0);
                        let (r, resp) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::hover());
                        if ok {
                            let pulse = (ui.input(|i| i.time) - self.updated_at < 2.0) as u8 as f32;
                            ui.painter().circle_filled(
                                r.center(),
                                4.0 + pulse,
                                if live { p.positive } else { p.faint },
                            );
                        } else {
                            ui.painter().text(
                                r.center(),
                                egui::Align2::CENTER_CENTER,
                                icon::WARNING_CIRCLE,
                                egui::FontId::proportional(18.0),
                                p.negative,
                            );
                        }
                        let mut tip = format!(
                            "{} · event {}",
                            if live { "Live" } else { "Snapshot" },
                            b.seq()
                        );
                        for (ok, text, _) in &checks {
                            tip.push_str(&format!(
                                "\n{} {text}",
                                if *ok {
                                    icon::SEAL_CHECK
                                } else {
                                    icon::WARNING_CIRCLE
                                }
                            ));
                        }
                        resp.on_hover_text(tip);
                    }
                });
            });
    }

    /// Back, forward, where you are, today, and search. Squeezed, the
    /// search shrinks to its icon, the date goes, and the crumb truncates:
    /// nothing ever overlaps.
    fn topbar(&mut self, ui: &mut Ui) {
        let p = pal(ui.ctx());
        egui::Panel::top("topbar")
            .exact_size(52.0)
            .frame(
                Frame::new()
                    .fill(p.bg)
                    .inner_margin(Margin::symmetric(16, 10))
                    .stroke(Stroke::new(1.0, p.border)),
            )
            .show(ui, |ui| {
                let width = ui.available_width();
                ui.horizontal_centered(|ui| {
                    let nav_btn = |ui: &mut Ui, glyph: &str, enabled: bool, tip: &str| {
                        ui.add_enabled(
                            enabled,
                            egui::Button::new(RichText::new(glyph).size(16.0))
                                .frame(false)
                                .min_size(vec2(28.0, 28.0)),
                        )
                        .on_hover_text(tip)
                        .clicked()
                    };
                    if nav_btn(ui, icon::ARROW_LEFT, self.nav.can_back(), "Back (Alt+←)") {
                        self.nav.back();
                    }
                    if nav_btn(
                        ui,
                        icon::ARROW_RIGHT,
                        self.nav.can_forward(),
                        "Forward (Alt+→)",
                    ) {
                        self.nav.forward();
                    }
                    ui.add_space(6.0);
                    // Right side first, so the crumb gets only what's left.
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let (text, min_w) = if width >= 640.0 {
                            (
                                format!(
                                    "{}  Search everything…      Ctrl K",
                                    icon::MAGNIFYING_GLASS
                                ),
                                260.0,
                            )
                        } else if width >= 420.0 {
                            (format!("{}  Search", icon::MAGNIFYING_GLASS), 0.0)
                        } else {
                            (icon::MAGNIFYING_GLASS.to_string(), 30.0)
                        };
                        let search = egui::Button::new(
                            RichText::new(text)
                                .font(theme::regular(13.0))
                                .color(p.faint),
                        )
                        .fill(p.surface)
                        .stroke(Stroke::new(1.0, p.border))
                        .corner_radius(CornerRadius::same(8))
                        .min_size(vec2(min_w, 30.0));
                        if ui
                            .add(search)
                            .on_hover_text("Search everything (Ctrl K)")
                            .clicked()
                        {
                            self.palette.open();
                        }
                        if let Some(b) = &self.books
                            && width >= 860.0
                        {
                            ui.label(
                                RichText::new(format!("Today {}", crate::fmt::date(b.today)))
                                    .font(theme::regular(12.5))
                                    .color(p.muted),
                            );
                        }
                        ui.add_space(8.0);
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            let crumbs =
                                crate::palette::crumbs(self.books.as_deref(), &self.nav.current);
                            // Narrow, the last crumb alone says where you are.
                            let skip = if ui.available_width() >= 260.0 {
                                0
                            } else {
                                crumbs.len() - 1
                            };
                            let last = crumbs.len() - 1;
                            let mut go = None;
                            for (i, (text, to)) in crumbs.iter().enumerate().skip(skip) {
                                if i > skip && ui.available_width() >= 260.0 - 80.0 {
                                    ui.label(RichText::new(icon::CARET_RIGHT).color(p.faint));
                                }
                                let rich = if i == last && i > 0 {
                                    RichText::new(text).font(theme::semibold(13.5))
                                } else {
                                    RichText::new(text)
                                        .font(theme::regular(13.5))
                                        .color(p.muted)
                                };
                                let label = egui::Label::new(rich).truncate();
                                match to {
                                    Some(to) => {
                                        let r = ui.add(label.sense(egui::Sense::click()));
                                        if r.hovered() {
                                            ui.ctx()
                                                .set_cursor_icon(egui::CursorIcon::PointingHand);
                                        }
                                        if r.on_hover_text(format!("Go to {text}")).clicked() {
                                            go = Some(to.clone());
                                        }
                                    }
                                    None => {
                                        ui.add(label);
                                    }
                                }
                            }
                            if let Some(to) = go {
                                self.nav.go(to);
                            }
                        });
                    });
                });
            });
    }
}

/// Width of the icon rail.
const RAIL_W: f32 = 60.0;
/// Height kept clear under the sidebar's links for the status box and the
/// theme toggle; the rail's equivalent.
const SIDEBAR_FOOTER: f32 = 172.0;
const RAIL_FOOTER: f32 = 84.0;

fn live_status(ui: &mut Ui, b: &Books, since_update: f64, live: bool) {
    let p = pal(ui.ctx());
    Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                let pulse = (since_update < 2.0) as u8 as f32;
                let color = if live { p.positive } else { p.faint };
                ui.painter().circle_filled(r.center(), 4.0 + pulse, color);
                ui.label(
                    RichText::new(if live { "Live" } else { "Snapshot" })
                        .font(theme::semibold(12.0)),
                );
                ui.label(
                    RichText::new(format!("· event {}", b.seq()))
                        .font(theme::regular(12.0))
                        .color(p.muted),
                );
            });
            for (ok, text, tip) in checks(b) {
                let (glyph, color) = if ok {
                    (icon::SEAL_CHECK, p.muted)
                } else {
                    (icon::WARNING_CIRCLE, p.negative)
                };
                let r = ui
                    .label(
                        RichText::new(format!("{glyph} {text}"))
                            .font(theme::regular(11.5))
                            .color(color),
                    )
                    .on_hover_text(tip);
                if text == "Journal chain verified" {
                    r.context_menu(|ui| {
                        crate::widgets::copy_item(ui, "Copy journal head", &b.head);
                        crate::widgets::copy_item(
                            ui,
                            "Copy data folder",
                            &b.data_dir.display().to_string(),
                        );
                    });
                }
            }
        });
}

/// The integrity checks the sidebar shows: (ok, label, explanation).
fn checks(b: &Books) -> Vec<(bool, String, String)> {
    use ledger_store::DiskCheck;
    let mut out = vec![(
        true,
        "Journal chain verified".to_string(),
        format!(
            "Every journal event's hash chains to the one before it.\nHead: {}\nData: {}",
            b.head,
            b.data_dir.display()
        ),
    )];
    out.push(match &b.integrity.disk {
        DiskCheck::Matches => (true, "Database matches journal".to_string(), "The server's SQLite database matches a fresh projection of the journal, every table, row for row.".to_string()),
        DiskCheck::Behind { applied, journal } => (true, "Database catching up".to_string(), format!("The database has applied {applied} of {journal} journal events; the server applies the rest right after writing them.")),
        DiskCheck::Ahead { .. } => (true, "Viewer catching up".to_string(), "The server has written journal events this view hasn't read yet; it reads them on its next refresh.".to_string()),
        DiskCheck::Missing => (true, "Read-only copy".to_string(), "No server database here: a copy of the books (a CPA's, say), or the server hasn't run yet. The journal and every file are verified either way.".to_string()),
        DiskCheck::Differs(why) => (false, "Database differs".to_string(), format!("{why}. The server rebuilds it from the journal (books.verify, or on its next start).")),
        DiskCheck::Unreadable(why) => (false, "Database unreadable".to_string(), why.clone()),
    });
    let bad = b.integrity.file_failures.len();
    out.push((
        bad == 0,
        if bad == 0 {
            format!("{} files verified", b.integrity.files_ok)
        } else {
            format!("{bad} files damaged")
        },
        "Every stored receipt and document re-hashed against its recorded SHA-256.".into(),
    ));
    out
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let updates = self.loader.as_ref().map(|l| l.poll()).unwrap_or_default();
        for u in updates {
            match u {
                Update::Books(b) => {
                    if self.books.is_some() {
                        self.updated_at = ui.input(|i| i.time);
                    }
                    self.books = Some(b);
                    self.error = None;
                }
                Update::Error(e) => self.error = Some(e),
                Update::Show(target) => self.show(target),
            }
        }
        self.receipts.pump(ui.ctx());
        self.handle_keys(ui);
        let compact = ui.available_width() < COMPACT_BELOW;
        self.sidebar(ui, compact);
        self.topbar(ui);
        let p = pal(ui.ctx());
        let since_update = ui.input(|i| i.time) - self.updated_at;
        if since_update < 6.0 {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
        }
        egui::CentralPanel::no_frame()
            .frame(Frame::new().fill(p.bg))
            .show(ui, |ui| {
                if let Some(err) = &self.error {
                    Frame::new()
                        .fill(p.negative_soft)
                        .inner_margin(Margin::symmetric(20, 8))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                RichText::new(format!("{} {err}", icon::WARNING)).color(p.negative),
                            );
                        });
                }
                let Some(books) = self.books.clone() else {
                    ui.centered_and_justified(|ui| {
                        ui.label(RichText::new("Opening the books…").color(p.muted));
                    });
                    return;
                };
                let mut shell = Shell {
                    receipts: &mut self.receipts,
                    state: &mut self.state,
                    go: None,
                    since_update,
                };
                let route = self.nav.current.clone();
                pages::show(ui, &books, &route, &mut shell);
                if let Some(to) = shell.go.take() {
                    self.nav.go(to);
                }
                if let Some(to) = self.palette.show(ui.ctx(), &books) {
                    self.nav.go(to);
                }
            });
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(SCHEME_KEY, self.scheme.key());
        storage.set_string(FILTER_KEY, self.filter.name().to_string());
        storage.set_string("dark", self.scheme.dark.to_string());
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
