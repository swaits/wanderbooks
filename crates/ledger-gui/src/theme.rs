//! Look and feel: the palette of the color scheme in force (schemes.rs),
//! Inter with tabular figures, and the spacing the whole app shares.

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

/// Where the palette in force is kept, in the context's memory.
fn palette_id() -> egui::Id {
    egui::Id::new("wanderbooks-palette")
}

/// The palette in force: the scheme's that `apply` set, else the
/// viewer's own for the current mode.
pub fn pal(ctx: &Context) -> Palette {
    ctx.data(|d| d.get_temp::<Palette>(palette_id()))
        .unwrap_or_else(|| crate::schemes::own(ctx.global_style().visuals.dark_mode).palette())
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

/// Install fonts ([`font_definitions`]).
pub fn install_fonts(ctx: &Context) {
    ctx.set_fonts(font_definitions());
}

/// The viewer's fonts: Inter with tabular figures frozen in, so every
/// number in the app lines up digit for digit; egui's own fonts and
/// Phosphor's icons as fallbacks in every family; then the PDFs' Noto
/// fallbacks (`ledger_import::fonts::FALLBACKS`: Chinese, Japanese,
/// Korean, Arabic, Hebrew, Thai, Devanagari), the very bytes the PDFs use,
/// so a name in any of those scripts shows as written, never as empty
/// boxes, and no font is in the binary twice. (egui shapes each word,
/// joining Arabic and setting it right to left, but doesn't reorder the
/// words of a right-to-left phrase: they read left to right.)
pub fn font_definitions() -> FontDefinitions {
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
    for (name, bytes) in &ledger_import::fonts::FALLBACKS {
        add(&mut fonts, name, bytes);
        for list in fonts.families.values_mut() {
            list.push((*name).to_owned());
        }
    }
    fonts
}

/// Apply the viewer's own style for light or dark mode (tests).
#[cfg(test)]
pub fn apply(ctx: &Context, dark: bool) {
    apply_scheme(ctx, crate::schemes::own(dark));
}

/// Apply a color scheme: its palette, everywhere.
pub fn apply_scheme(ctx: &Context, scheme: &crate::schemes::Scheme) {
    let dark = scheme.dark;
    let palette = scheme.palette();
    ctx.data_mut(|d| d.insert_temp(palette_id(), palette));
    let p = &palette;
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
    // egui's weak text (placeholders, hints) is the text at 60% alpha
    // unless set: the palette's secondary text, which is kept readable.
    visuals.weak_text_color = Some(p.muted);
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
    visuals.widgets.open.bg_fill = p.hover;
    visuals.widgets.open.weak_bg_fill = p.hover;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, p.border);
    visuals.widgets.open.fg_stroke = Stroke::new(1.0, p.text);
    // egui keeps a light and a dark style and paints with the one its
    // theme picks, which follows the system unless told: both get the
    // scheme's visuals, and the theme is pinned to the scheme's mode, so
    // a system that turns light or dark later changes nothing (review of
    // the color schemes, 2026-10-19: figures went unreadable).
    ctx.set_theme(if dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });
    ctx.all_styles_mut(|style| style.visuals = visuals.clone());
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

#[cfg(test)]
mod tests {
    use super::*;

    /// font_definitions: every family ends with the PDFs' Noto fallbacks,
    /// in their order, after Inter, egui's fonts and the icons, each font
    /// the very bytes ledger-import bundles (the same pointer: one copy in
    /// the binary); and a name in each script shows with real glyphs in
    /// every family the viewer uses.
    #[test]
    fn every_family_falls_back_to_the_noto_fonts() {
        let defs = font_definitions();
        let names: Vec<String> = ledger_import::fonts::FALLBACKS
            .iter()
            .map(|(n, _)| (*n).to_owned())
            .collect();
        for family in [
            FontFamily::Proportional,
            FontFamily::Monospace,
            FontFamily::Name(SEMIBOLD.into()),
            FontFamily::Name(BOLD.into()),
        ] {
            let list = &defs.families[&family];
            assert!(list.len() > names.len(), "{family:?}: {list:?}");
            assert_eq!(list[list.len() - names.len()..], names[..], "{family:?}");
        }
        assert!(defs.families[&FontFamily::Proportional][0] == "InterTab-Regular");
        for (name, bytes) in &ledger_import::fonts::FALLBACKS {
            let data: &[u8] = &defs.font_data[*name].font;
            assert!(
                std::ptr::eq(data, *bytes),
                "{name} is ledger-import's own bytes, not a copy"
            );
        }
        let ctx = Context::default();
        ctx.set_fonts(defs);
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        out.textures_delta.clear();
        for text in [
            "北京 東京 서울",
            "مكتب الرياض",
            "שלום עולם",
            "สวัสดีครับ",
            "नमस्ते",
        ] {
            for id in [
                regular(14.0),
                semibold(14.0),
                bold(14.0),
                FontId::monospace(12.5),
            ] {
                // Letters only: egui says a character the replacement
                // glyph's own font draws is missing, the space in Hack.
                let missing: Vec<char> = text
                    .chars()
                    .filter(|c| !c.is_whitespace() && !ctx.fonts_mut(|f| f.has_glyph(&id, *c)))
                    .collect();
                assert!(missing.is_empty(), "{text} in {id:?}: {missing:?}");
            }
        }
    }
}
