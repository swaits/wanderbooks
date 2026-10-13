//! The window: sidebar, top bar, and the current page.

use crate::books::Books;
use crate::data::{Loader, Source, Update};
use crate::nav::{Nav, Route, Section};
use crate::pages;
use crate::receipts::Receipts;
use crate::theme::{self, pal};
use egui::{
    Align, Color32, CornerRadius, Frame, Key, Layout, Margin, RichText, Sense, Stroke, Ui, vec2,
};
use egui_phosphor::regular as icon;
use std::sync::Arc;

/// Per-page state that survives navigation.
#[derive(Default)]
pub struct PageState {
    pub transactions: pages::transactions::State,
    pub reports: pages::reports::State,
    pub accounts: pages::accounts::State,
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

pub struct App {
    loader: Option<Loader>,
    books: Option<Arc<Books>>,
    error: Option<String>,
    nav: Nav,
    receipts: Receipts,
    dark: bool,
    state: PageState,
    palette: crate::palette::Palette,
    updated_at: f64,
}

impl App {
    /// The live app over a books directory.
    pub fn new(cc: &eframe::CreationContext<'_>, source: Source) -> Self {
        let dark = cc
            .storage
            .and_then(|s| s.get_string("dark"))
            .map(|v| v == "true")
            .unwrap_or(cc.egui_ctx.global_style().visuals.dark_mode);
        let mut app = Self::bare(&cc.egui_ctx, dark);
        app.loader = Some(Loader::spawn(source, cc.egui_ctx.clone()));
        app
    }

    /// An app showing fixed books (tests and screenshots).
    pub fn with_books(ctx: &egui::Context, books: Books, dark: bool, start: Route) -> Self {
        let mut app = Self::bare(ctx, dark);
        app.books = Some(Arc::new(books));
        app.nav = Nav::new(start);
        app
    }

    fn bare(ctx: &egui::Context, dark: bool) -> Self {
        theme::install_fonts(ctx);
        theme::apply(ctx, dark);
        egui_extras::install_image_loaders(ctx);
        Self {
            loader: None,
            books: None,
            error: None,
            nav: Nav::new(Route::Overview),
            receipts: Receipts::new(ctx),
            dark,
            state: PageState::default(),
            palette: Default::default(),
            updated_at: -1e9,
        }
    }

    /// Navigate (tests).
    pub fn goto(&mut self, r: Route) {
        self.nav.go(r);
    }

    /// Block until every receipt on screen has rendered (screenshots).
    pub fn settle(&mut self, ctx: &egui::Context) {
        self.receipts.settle(ctx);
    }

    /// Page state (tests).
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

    fn sidebar(&mut self, ui: &mut Ui) {
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
                        Color32::WHITE,
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
                for s in Section::ALL {
                    let on = s == current;
                    let (rect, resp) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
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
                        ui.painter()
                            .rect_filled(badge, CornerRadius::same(9), p.warning_soft);
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
                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    let label = if self.dark {
                        format!("{} Light mode", icon::SUN)
                    } else {
                        format!("{} Dark mode", icon::MOON)
                    };
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(label)
                                    .font(theme::regular(12.5))
                                    .color(p.muted),
                            )
                            .frame(false),
                        )
                        .clicked()
                    {
                        self.dark = !self.dark;
                        theme::apply(ui.ctx(), self.dark);
                    }
                    ui.add_space(6.0);
                    if let Some(b) = &books {
                        live_status(
                            ui,
                            b,
                            ui.input(|i| i.time) - self.updated_at,
                            self.loader.is_some(),
                        );
                    }
                });
            });
    }

    fn topbar(&mut self, ui: &mut Ui) {
        let p = pal(ui.ctx());
        egui::Panel::top("topbar")
            .exact_size(52.0)
            .frame(
                Frame::new()
                    .fill(p.bg)
                    .inner_margin(Margin::symmetric(20, 10))
                    .stroke(Stroke::new(1.0, p.border)),
            )
            .show(ui, |ui| {
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
                    let crumb = self
                        .books
                        .as_ref()
                        .map(|b| crate::palette::describe(b, &self.nav.current))
                        .unwrap_or_default();
                    ui.label(
                        RichText::new(self.nav.current.section().title())
                            .font(theme::regular(13.5))
                            .color(p.muted),
                    );
                    if !crumb.is_empty() {
                        ui.label(RichText::new(icon::CARET_RIGHT).color(p.faint));
                        ui.add(
                            egui::Label::new(RichText::new(crumb).font(theme::semibold(13.5)))
                                .truncate(),
                        );
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let search = egui::Button::new(
                            RichText::new(format!(
                                "{}  Search everything…      Ctrl K",
                                icon::MAGNIFYING_GLASS
                            ))
                            .font(theme::regular(13.0))
                            .color(p.faint),
                        )
                        .fill(p.surface)
                        .stroke(Stroke::new(1.0, p.border))
                        .corner_radius(CornerRadius::same(8))
                        .min_size(vec2(260.0, 30.0));
                        if ui.add(search).clicked() {
                            self.palette.open();
                        }
                        if let Some(b) = &self.books {
                            ui.label(
                                RichText::new(format!("Today {}", crate::fmt::date(b.today)))
                                    .font(theme::regular(12.5))
                                    .color(p.muted),
                            );
                        }
                    });
                });
            });
    }
}

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
            let check = |ui: &mut Ui, ok: bool, text: String, tip: String| {
                let (glyph, color) = if ok { (icon::SEAL_CHECK, p.muted) } else { (icon::WARNING_CIRCLE, p.negative) };
                ui.label(RichText::new(format!("{glyph} {text}")).font(theme::regular(11.5)).color(color))
                    .on_hover_text(tip);
            };
            check(
                ui,
                true,
                "Journal chain verified".into(),
                format!("Every journal event's hash chains to the one before it.\nHead: {}\nData: {}", b.head, b.data_dir.display()),
            );
            use ledger_store::DiskCheck;
            let (ok, text, tip) = match &b.integrity.disk {
                DiskCheck::Matches => (true, "Database matches journal".to_string(), "The server's SQLite database matches a fresh projection of the journal, every table, row for row.".to_string()),
                DiskCheck::Behind { applied, journal } => (true, "Database catching up".to_string(), format!("The database has applied {applied} of {journal} journal events; the server applies the rest right after writing them.")),
                DiskCheck::Missing => (true, "Database will rebuild".to_string(), "No database file yet; the server builds it from the journal when it starts.".to_string()),
                DiskCheck::Differs(why) => (false, "Database differs".to_string(), format!("{why}. The server rebuilds it from the journal (books.verify, or on its next start).")),
                DiskCheck::Unreadable(why) => (false, "Database unreadable".to_string(), why.clone()),
            };
            check(ui, ok, text, tip);
            let bad = b.integrity.file_failures.len();
            check(
                ui,
                bad == 0,
                if bad == 0 { format!("{} files verified", b.integrity.files_ok) } else { format!("{bad} files damaged") },
                "Every stored receipt and document re-hashed against its recorded SHA-256.".into(),
            );
        });
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        if let Some(loader) = &self.loader {
            match loader.poll() {
                Some(Update::Books(b)) => {
                    if self.books.is_some() {
                        self.updated_at = ui.input(|i| i.time);
                    }
                    self.books = Some(b);
                    self.error = None;
                }
                Some(Update::Error(e)) => self.error = Some(e),
                None => {}
            }
        }
        self.receipts.pump(ui.ctx());
        self.handle_keys(ui);
        self.sidebar(ui);
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
        storage.set_string("dark", self.dark.to_string());
    }
}
