//! Look and feel: one palette per mode, Inter with tabular figures, and
//! the spacing the whole app shares.

use egui::{
    Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke,
    TextStyle, Visuals,
};
use std::sync::Arc;

/// Colors for one mode. Every color in the app comes from here.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub bg: Color32,
    pub surface: Color32,
    pub sidebar: Color32,
    pub hover: Color32,
    pub selected: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub positive: Color32,
    pub negative: Color32,
    pub warning: Color32,
    pub warning_soft: Color32,
    pub negative_soft: Color32,
    pub positive_soft: Color32,
    pub new_highlight: Color32,
    /// Chart series, in order.
    pub series: [Color32; 6],
}

pub const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(0xF6, 0xF6, 0xF4),
    surface: Color32::from_rgb(0xFF, 0xFF, 0xFF),
    sidebar: Color32::from_rgb(0xEF, 0xEF, 0xEC),
    hover: Color32::from_rgb(0xF2, 0xF2, 0xF0),
    selected: Color32::from_rgb(0xE8, 0xE9, 0xFB),
    border: Color32::from_rgb(0xE4, 0xE4, 0xE0),
    text: Color32::from_rgb(0x1B, 0x1B, 0x1F),
    muted: Color32::from_rgb(0x67, 0x67, 0x70),
    faint: Color32::from_rgb(0x9C, 0x9C, 0xA4),
    accent: Color32::from_rgb(0x4F, 0x46, 0xE5),
    accent_soft: Color32::from_rgb(0xEE, 0xEE, 0xFD),
    positive: Color32::from_rgb(0x15, 0x80, 0x3D),
    negative: Color32::from_rgb(0xB9, 0x1C, 0x1C),
    warning: Color32::from_rgb(0xB4, 0x53, 0x09),
    warning_soft: Color32::from_rgb(0xFE, 0xF3, 0xE2),
    negative_soft: Color32::from_rgb(0xFD, 0xEC, 0xEC),
    positive_soft: Color32::from_rgb(0xE8, 0xF5, 0xEC),
    new_highlight: Color32::from_rgb(0xFF, 0xF7, 0xD6),
    series: [
        Color32::from_rgb(0x4F, 0x46, 0xE5),
        Color32::from_rgb(0x0D, 0x94, 0x88),
        Color32::from_rgb(0xD9, 0x77, 0x06),
        Color32::from_rgb(0xDB, 0x27, 0x77),
        Color32::from_rgb(0x25, 0x63, 0xEB),
        Color32::from_rgb(0x64, 0x74, 0x8B),
    ],
};

pub const DARK: Palette = Palette {
    bg: Color32::from_rgb(0x11, 0x12, 0x14),
    surface: Color32::from_rgb(0x19, 0x1A, 0x1D),
    sidebar: Color32::from_rgb(0x15, 0x16, 0x18),
    hover: Color32::from_rgb(0x22, 0x23, 0x27),
    selected: Color32::from_rgb(0x26, 0x27, 0x45),
    border: Color32::from_rgb(0x2A, 0x2B, 0x30),
    text: Color32::from_rgb(0xEC, 0xEC, 0xEE),
    muted: Color32::from_rgb(0xA3, 0xA3, 0xAC),
    faint: Color32::from_rgb(0x6E, 0x6E, 0x78),
    accent: Color32::from_rgb(0x8B, 0x93, 0xFA),
    accent_soft: Color32::from_rgb(0x24, 0x25, 0x45),
    positive: Color32::from_rgb(0x4A, 0xDE, 0x80),
    negative: Color32::from_rgb(0xF8, 0x71, 0x71),
    warning: Color32::from_rgb(0xFB, 0xBF, 0x24),
    warning_soft: Color32::from_rgb(0x33, 0x29, 0x16),
    negative_soft: Color32::from_rgb(0x3A, 0x1C, 0x1E),
    positive_soft: Color32::from_rgb(0x16, 0x30, 0x22),
    new_highlight: Color32::from_rgb(0x36, 0x31, 0x18),
    series: [
        Color32::from_rgb(0x8B, 0x93, 0xFA),
        Color32::from_rgb(0x2D, 0xD4, 0xBF),
        Color32::from_rgb(0xFB, 0xBF, 0x24),
        Color32::from_rgb(0xF4, 0x72, 0xB6),
        Color32::from_rgb(0x60, 0xA5, 0xFA),
        Color32::from_rgb(0x94, 0xA3, 0xB8),
    ],
};

/// The palette for the current mode.
pub fn pal(ctx: &Context) -> &'static Palette {
    if ctx.global_style().visuals.dark_mode {
        &DARK
    } else {
        &LIGHT
    }
}

pub const SEMIBOLD: &str = "semibold";
pub const BOLD: &str = "bold";

pub fn regular(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(BOLD.into()))
}

/// Install fonts: Inter with tabular figures frozen in, so every number in
/// the app lines up digit for digit. Phosphor icons ride along as a
/// fallback in every family.
pub fn install_fonts(ctx: &Context) {
    let mut fonts = FontDefinitions::default();
    let add = |fonts: &mut FontDefinitions, name: &str, bytes: &'static [u8]| {
        fonts
            .font_data
            .insert(name.to_owned(), Arc::new(FontData::from_static(bytes)));
    };
    add(
        &mut fonts,
        "InterTab-Regular",
        include_bytes!("../assets/fonts/InterTab-Regular.ttf"),
    );
    add(
        &mut fonts,
        "InterTab-SemiBold",
        include_bytes!("../assets/fonts/InterTab-SemiBold.ttf"),
    );
    add(
        &mut fonts,
        "InterTab-Bold",
        include_bytes!("../assets/fonts/InterTab-Bold.ttf"),
    );
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    let fallbacks: Vec<String> = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    // Text first, icons as a fallback. The bundled Inter copies have their
    // private-use glyphs removed, so Phosphor's icons (which live there)
    // are never shadowed.
    let family = |primary: &str| {
        let mut v = vec![primary.to_owned()];
        v.extend(fallbacks.iter().cloned());
        v
    };
    fonts
        .families
        .insert(FontFamily::Proportional, family("InterTab-Regular"));
    fonts.families.insert(
        FontFamily::Name(SEMIBOLD.into()),
        family("InterTab-SemiBold"),
    );
    fonts
        .families
        .insert(FontFamily::Name(BOLD.into()), family("InterTab-Bold"));
    ctx.set_fonts(fonts);
}

/// Apply the app's style for light or dark mode.
pub fn apply(ctx: &Context, dark: bool) {
    let p = if dark { &DARK } else { &LIGHT };
    let mut visuals = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    visuals.panel_fill = p.bg;
    visuals.window_fill = p.surface;
    visuals.extreme_bg_color = p.surface;
    visuals.faint_bg_color = p.hover;
    visuals.override_text_color = Some(p.text);
    visuals.hyperlink_color = p.accent;
    visuals.selection.bg_fill = p.selected;
    visuals.selection.stroke = Stroke::new(1.0, p.accent);
    visuals.window_stroke = Stroke::new(1.0, p.border);
    visuals.window_corner_radius = CornerRadius::same(12);
    visuals.menu_corner_radius = CornerRadius::same(10);
    for w in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        w.corner_radius = CornerRadius::same(8);
    }
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    visuals.widgets.inactive.bg_fill = p.surface;
    visuals.widgets.inactive.weak_bg_fill = p.surface;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, p.border);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
    visuals.widgets.hovered.bg_fill = p.hover;
    visuals.widgets.hovered.weak_bg_fill = p.hover;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, p.border);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, p.text);
    visuals.widgets.active.bg_fill = p.selected;
    visuals.widgets.active.weak_bg_fill = p.selected;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, p.text);
    ctx.set_visuals(visuals);
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Heading, semibold(20.0)),
            (TextStyle::Body, regular(14.0)),
            (TextStyle::Button, regular(14.0)),
            (TextStyle::Small, regular(12.0)),
            (TextStyle::Monospace, FontId::monospace(12.5)),
        ]
        .into();
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.interact_size.y = 28.0;
        style.spacing.window_margin = Margin::same(16);
        style.spacing.scroll.bar_width = 8.0;
        style.spacing.scroll.floating = true;
        style.interaction.selectable_labels = false;
    });
}
