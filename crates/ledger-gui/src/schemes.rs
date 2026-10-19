//! Color schemes: the viewer's own (Wanderbooks Light and Dark) and twenty
//! families of popular editor schemes, each with all its variants, from
//! each scheme's official palette (table.rs names the sources).
//!
//! ```text
//! GOAL: the viewer can wear any of the well-known editor color schemes,
//!       and whichever it wears, every figure stays plainly legible.
//! SOURCE: owner, 2026-10-19 ("full blown colorscheme support … all the top
//!         neovim style colorschemes"; "on the order of 20 families, each
//!         with a flyout of all its variants"; "a light/dark/all filter").
//! INVARIANTS:
//!   1. A scheme only colors the viewer: it changes no figure, no text and
//!      nothing in the books, and the choice is kept with the viewer's own
//!      preferences.
//!   2. Every palette a scheme gives keeps every color that carries text
//!      or a figure (text, secondary and faintest text, gains, losses,
//!      warnings, links) at WCAG AA contrast (4.5:1) on every background
//!      it is drawn on (grounds()); a scheme's own color that falls short
//!      keeps its hue and saturation and is made lighter (dark schemes) or
//!      darker (light ones) just far enough. Gains, losses and text stay
//!      plainly different to the eye, and so do the chart series shown
//!      together. The scheme holds whatever the system's light or dark
//!      mode does later.
//!   3. Each scheme is light or dark as its background is; the filter
//!      shows exactly the schemes of the mode chosen.
//!   4. A remembered scheme that no longer exists falls back to the
//!      viewer's own scheme of the same mode.
//!   5. The menu lists the families alphabetically (case and accents
//!      aside), and each family's variants alphabetically (owner,
//!      2026-10-19).
//! ```

use crate::theme::Palette;
use egui::Color32;

mod table;

/// A scheme's own colors, as its authors named them (see the table's
/// source, docs/color-schemes.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Core {
    /// The main background.
    pub bg: Color32,
    /// A panel or sidebar background beside it.
    pub dim: Color32,
    /// A raised surface (floats, the status line).
    pub raised: Color32,
    /// The selection.
    pub sel: Color32,
    /// Borders and splits.
    pub border: Color32,
    /// The main text.
    pub fg: Color32,
    /// Secondary text.
    pub muted: Color32,
    /// Comments: the faintest text.
    pub faint: Color32,
    /// The scheme's signature accent.
    pub accent: Color32,
    /// Its named colors.
    pub red: Color32,
    pub green: Color32,
    pub yellow: Color32,
    pub orange: Color32,
    pub blue: Color32,
    pub purple: Color32,
    pub cyan: Color32,
}

/// One scheme: a family's variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scheme {
    /// Its family ("Catppuccin").
    pub family: &'static str,
    /// Its variant ("Mocha").
    pub name: &'static str,
    /// Whether it is dark.
    pub dark: bool,
    /// Its colors; `None` for the viewer's own, whose palettes are
    /// hand-made (theme::LIGHT, theme::DARK).
    pub core: Option<Core>,
}

/// Which schemes the menu shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Filter {
    /// Light and dark.
    #[default]
    All,
    /// Light ones.
    Light,
    /// Dark ones.
    Dark,
}

impl Filter {
    /// What it is called in the menu and in the viewer's preferences.
    pub fn name(self) -> &'static str {
        match self {
            Filter::All => "All",
            Filter::Light => "Light",
            Filter::Dark => "Dark",
        }
    }

    /// The filter of that name, if any.
    pub fn from_name(name: &str) -> Option<Self> {
        [Filter::All, Filter::Light, Filter::Dark]
            .into_iter()
            .find(|f| f.name() == name)
    }

    /// Whether it shows `scheme` (INVARIANT 3).
    pub fn shows(self, scheme: &Scheme) -> bool {
        match self {
            Filter::All => true,
            Filter::Light => !scheme.dark,
            Filter::Dark => scheme.dark,
        }
    }
}

/// The viewer's own family.
pub const OWN: &str = "Wanderbooks";

/// The viewer's own schemes.
static OWN_SCHEMES: [Scheme; 2] = [
    Scheme {
        family: OWN,
        name: "Light",
        dark: false,
        core: None,
    },
    Scheme {
        family: OWN,
        name: "Dark",
        dark: true,
        core: None,
    },
];

/// Every scheme: the viewer's own first, then the families in menu order.
pub fn all() -> impl Iterator<Item = &'static Scheme> {
    OWN_SCHEMES.iter().chain(table::SCHEMES.iter())
}

/// How a name sorts: without case or accents ("Rosé" as "rose").
fn sort_key(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ä' | 'ã' | 'å' => 'a',
            'ç' => 'c',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'ñ' => 'n',
            'ò' | 'ó' | 'ô' | 'ö' | 'õ' => 'o',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            c => c.to_ascii_lowercase(),
        })
        .collect()
}

/// The families in alphabetical order (INVARIANT 5), each with its
/// variants that `filter` shows, also alphabetical; a family none of whose
/// variants it shows is left out.
pub fn families(filter: Filter) -> Vec<(&'static str, Vec<&'static Scheme>)> {
    let mut out: Vec<(&'static str, Vec<&'static Scheme>)> = Vec::new();
    for s in all().filter(|s| filter.shows(s)) {
        match out.iter_mut().find(|(family, _)| *family == s.family) {
            Some((_, variants)) => variants.push(s),
            None => out.push((s.family, vec![s])),
        }
    }
    out.sort_by_key(|(family, _)| sort_key(family));
    for (_, variants) in &mut out {
        variants.sort_by_key(|s| sort_key(s.name));
    }
    out
}

/// The viewer's own scheme for light or dark.
pub fn own(dark: bool) -> &'static Scheme {
    &OWN_SCHEMES[usize::from(dark)]
}

impl Scheme {
    /// How it is kept in the viewer's preferences: "Family/Variant".
    pub fn key(&self) -> String {
        format!("{}/{}", self.family, self.name)
    }

    /// The scheme kept as `key`, else the viewer's own of mode `dark`
    /// (INVARIANT 4).
    pub fn from_key(key: &str, dark: bool) -> &'static Scheme {
        all().find(|s| s.key() == key).unwrap_or_else(|| own(dark))
    }

    /// How the menu names it.
    pub fn label(&self) -> String {
        if self.family == self.name {
            self.name.to_string()
        } else {
            format!("{} {}", self.family, self.name)
        }
    }

    /// The palette the viewer paints with (INVARIANT 2), made once per
    /// scheme.
    pub fn palette(&self) -> Palette {
        static MADE: std::sync::OnceLock<Vec<Palette>> = std::sync::OnceLock::new();
        let made = MADE.get_or_init(|| all().map(Scheme::make_palette).collect());
        all()
            .position(|s| s == self)
            .map_or_else(|| self.make_palette(), |i| made[i])
    }

    fn make_palette(&self) -> Palette {
        readable(match &self.core {
            None if self.dark => crate::theme::DARK,
            None => crate::theme::LIGHT,
            Some(core) => derive(core),
        })
    }

    /// A few of its colors, for the menu's swatch.
    pub fn swatch(&self) -> [Color32; 5] {
        let p = self.palette();
        [p.bg, p.accent, p.positive, p.warning, p.negative]
    }
}

/// `a` moved toward `b` by `t` (0: `a`, 1: `b`).
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

/// WCAG relative luminance.
pub fn luminance(c: Color32) -> f32 {
    let lin = |v: u8| {
        let s = f32::from(v) / 255.0;
        if s <= 0.039_28 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
}

/// WCAG contrast ratio of two colors (1 to 21).
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// The least the readable colors reach on every background (INVARIANT 2).
pub const READABLE: f32 = 4.5;

/// `c`, or `c` with its hue and saturation kept and its lightness moved
/// toward `toward` (black or white) just far enough to reach `min`
/// contrast on each of `on` (INVARIANT 2).
pub fn legible(c: Color32, toward: Color32, on: &[Color32], min: f32) -> Color32 {
    let ok = |c: Color32| on.iter().all(|b| contrast(c, *b) >= min);
    if ok(c) {
        return c;
    }
    let (h, s, l) = hsl(c);
    let end = if luminance(toward) > 0.5 { 1.0 } else { 0.0 };
    (1..=100)
        .map(|i| from_hsl(h, s, l + (end - l) * i as f32 / 100.0))
        .find(|c| ok(*c))
        .unwrap_or(toward)
}

/// Hue (0 to 1), saturation and lightness of `c`.
fn hsl(c: Color32) -> (f32, f32, f32) {
    let [r, g, b] = [c.r(), c.g(), c.b()].map(|v| f32::from(v) / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    if max == min {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

/// The color of hue `h` (0 to 1), saturation `s` and lightness `l`.
fn from_hsl(h: f32, s: f32, l: f32) -> Color32 {
    let l = l.clamp(0.0, 1.0);
    if s == 0.0 {
        let v = (l * 255.0).round() as u8;
        return Color32::from_rgb(v, v, v);
    }
    let q = if l < 0.5 {
        l * (1.0 + s)
    } else {
        l + s - l * s
    };
    let p = 2.0 * l - q;
    let channel = |t: f32| {
        let t = t.rem_euclid(1.0);
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (v.clamp(0.0, 1.0) * 255.0).round() as u8
    };
    Color32::from_rgb(channel(h + 1.0 / 3.0), channel(h), channel(h - 1.0 / 3.0))
}

/// A color in CIELAB (D65): lightness, then the two color axes.
fn lab(c: Color32) -> [f32; 3] {
    let lin = |v: u8| {
        let s = f32::from(v) / 255.0;
        if s <= 0.040_45 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = (lin(c.r()), lin(c.g()), lin(c.b()));
    let x = (0.412_456_4 * r + 0.357_576_1 * g + 0.180_437_5 * b) / 0.950_47;
    let y = 0.212_672_9 * r + 0.715_152_2 * g + 0.072_175 * b;
    let z = (0.019_333_9 * r + 0.119_192 * g + 0.950_304_1 * b) / 1.088_83;
    let f = |t: f32| {
        if t > 216.0 / 24_389.0 {
            t.cbrt()
        } else {
            (24_389.0 / 27.0 * t + 16.0) / 116.0
        }
    };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// How different two colors look: CIE76 ΔE (about 2: just noticeable;
/// 20 and more: plainly different colors).
pub fn distance(a: Color32, b: Color32) -> f32 {
    let (a, b) = (lab(a), lab(b));
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// How far apart a chart's series colors are kept, at least.
pub const SERIES_APART: f32 = 20.0;

/// The chart's series colors (the charts use the first three together:
/// Net, Income, Spending): the scheme's own named colors in this order,
/// each one that looks like one already taken passed over (some schemes
/// name one color twice: Monokai Pro's cyan is its blue); when none is
/// left that is plainly different, the one most unlike those taken.
/// `muted` is last. Each is first made readable on the cards, as
/// `readable` would, so two that only part once lifted aren't taken for
/// different.
fn series(k: &Core, card: Color32, muted: Color32) -> [Color32; 6] {
    let end = readable_end(card);
    let mut left: Vec<Color32> = [
        k.blue, k.cyan, k.yellow, k.purple, k.orange, k.green, k.red, k.accent,
    ]
    .into_iter()
    .map(|c| legible(c, end, &[card], 3.0))
    .collect();
    let mut out: Vec<Color32> = vec![left.remove(0)];
    while out.len() < 5 {
        let nearest = |c: &Color32| {
            out.iter()
                .map(|t| distance(*t, *c))
                .fold(f32::INFINITY, f32::min)
        };
        let at = left
            .iter()
            .position(|c| nearest(c) >= SERIES_APART)
            .unwrap_or_else(|| {
                (0..left.len())
                    .max_by(|&i, &j| nearest(&left[i]).total_cmp(&nearest(&left[j])))
                    .expect("eight colors for five places")
            });
        out.push(left.remove(at));
    }
    [out[0], out[1], out[2], out[3], out[4], muted]
}

/// The viewer's palette from a scheme's colors.
fn derive(k: &Core) -> Palette {
    // The page is the darker of the scheme's two backgrounds, the cards the
    // lighter: cards stand out in light and dark schemes alike. A scheme
    // with one background gets a second a shade toward its text: the cards
    // in a dark scheme (lighter), the page in a light one (darker).
    let (page, card) = if luminance(k.dim) <= luminance(k.bg) {
        (k.dim, k.bg)
    } else {
        (k.bg, k.dim)
    };
    let (page, card) = if page != card {
        (page, card)
    } else if luminance(k.bg) < luminance(k.fg) {
        (page, mix(page, k.fg, 0.04))
    } else {
        (mix(card, k.fg, 0.04), card)
    };
    // Secondary text sits between the text and the comments; a scheme
    // whose "subtle" tone is no dimmer than its text (Nord's) gets one
    // in between.
    let muted = if contrast(k.muted, page) >= contrast(k.fg, page) {
        mix(k.fg, k.faint, 0.45)
    } else {
        k.muted
    };
    let sidebar = mix(page, k.fg, 0.02);
    let hover = mix(card, k.fg, 0.06);
    let selected = mix(card, k.accent, 0.14);
    // A tint light enough that amounts on it keep their own colors (at
    // 0.22 it bleached Nord's red to a pink; review, 2026-10-19).
    let new_highlight = mix(card, k.yellow, 0.10);
    let soft = |c: Color32| mix(card, c, 0.14);
    Palette {
        bg: page,
        surface: card,
        sidebar,
        hover,
        selected,
        border: mix(card, k.fg, 0.14),
        text: k.fg,
        muted,
        faint: k.faint,
        accent: k.accent,
        accent_soft: soft(k.accent),
        positive: k.green,
        negative: k.red,
        warning: k.orange,
        warning_soft: soft(k.orange),
        negative_soft: soft(k.red),
        positive_soft: soft(k.green),
        new_highlight,
        series: series(k, card, muted),
    }
}

/// Every background text or a figure is drawn on: the page, cards, the
/// sidebar, hovered, selected and new rows, and the soft fills of pills
/// and badges.
pub fn grounds(p: &Palette) -> [Color32; 10] {
    [
        p.bg,
        p.surface,
        p.sidebar,
        p.hover,
        p.selected,
        p.new_highlight,
        p.accent_soft,
        p.warning_soft,
        p.negative_soft,
        p.positive_soft,
    ]
}

/// `p` with every color that carries text or a figure (the faintest
/// text included: it carries chart amounts and entry numbers) readable on
/// every background it is drawn on (INVARIANT 2): its lightness moved
/// toward black or white, whichever the cards are farther from, just far
/// enough.
fn readable(mut p: Palette) -> Palette {
    let grounds = grounds(&p);
    let end = readable_end(p.surface);
    let read = |c: Color32| legible(c, end, &grounds, READABLE);
    // The faintest text first, then each level above it reading at least
    // STEP times better: text over secondary over faint, always.
    p.faint = read(p.faint);
    let above = |c: Color32, below: Color32| {
        legible(
            c,
            end,
            &grounds,
            (worst(below, &grounds) * STEP).max(READABLE),
        )
    };
    p.muted = above(p.muted, p.faint);
    p.text = above(p.text, p.muted);
    p.positive = read(p.positive);
    p.negative = read(p.negative);
    p.warning = read(p.warning);
    p.accent = read(p.accent);
    // Chart bars sit on the cards, and on the hover tint when pointed at.
    let (surface, hover) = (p.surface, p.hover);
    p.series = p.series.map(|c| legible(c, end, &[surface, hover], 3.0));
    p
}

/// How much better each level of text reads than the one below it, at
/// least, on the ground it reads worst on: text over secondary text over
/// the faintest (review, 2026-10-19: lifting each alone made them alike).
pub const STEP: f32 = 1.2;

/// The contrast `c` has on the ground it reads worst on.
pub fn worst(c: Color32, grounds: &[Color32]) -> f32 {
    grounds
        .iter()
        .map(|g| contrast(c, *g))
        .fold(f32::INFINITY, f32::min)
}

/// Black or white, whichever reads better on `c`: for text on a filled
/// color (the logo's initial on the accent).
pub fn on(c: Color32) -> Color32 {
    readable_end(c)
}

/// Black or white, whichever reads better on `c`.
fn readable_end(c: Color32) -> Color32 {
    if contrast(Color32::BLACK, c) >= contrast(Color32::WHITE, c) {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

/// `Color32` from three bytes (the table's shorthand).
pub(crate) const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twenty_families_each_with_all_its_variants() {
        let families = families(Filter::All);
        // The viewer's own, and twenty more.
        assert!(families.len() > 20, "{} families", families.len());
        // Alphabetical, families and variants (INVARIANT 5).
        let keys: Vec<String> = families.iter().map(|f| sort_key(f.0)).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
        assert_eq!(families[0].0, "Ayu");
        assert_eq!(families.last().unwrap().0, OWN);
        for (family, variants) in &families {
            let names: Vec<String> = variants.iter().map(|s| sort_key(s.name)).collect();
            let mut sorted = names.clone();
            sorted.sort();
            assert_eq!(names, sorted, "{family}");
        }
        let rose = families.iter().position(|f| f.0 == "Rosé Pine").unwrap();
        assert_eq!(families[rose - 1].0, "Oxocarbon", "Rosé sorts as Rose");
        let names: Vec<&str> = families.iter().map(|f| f.0).collect();
        for want in [
            "Catppuccin",
            "Tokyo Night",
            "Gruvbox",
            "Kanagawa",
            "Rosé Pine",
            "Ayu",
            "Nord",
            "Dracula",
            "One Dark",
            "Solarized",
        ] {
            assert!(names.contains(&want), "{want} missing: {names:?}");
        }
        let ayu = &families.iter().find(|f| f.0 == "Ayu").unwrap().1;
        let ayu: Vec<&str> = ayu.iter().map(|s| s.name).collect();
        assert_eq!(ayu, ["Dark", "Light", "Mirage"]);
        // A family appears once, its variants together; keys are unique.
        let mut seen = std::collections::BTreeSet::new();
        for (family, _) in &families {
            assert!(seen.insert(*family), "{family} twice");
        }
        let mut keys = std::collections::BTreeSet::new();
        for s in all() {
            assert!(keys.insert(s.key()), "{} twice", s.key());
        }
    }

    #[test]
    fn every_scheme_keeps_figures_and_text_readable() {
        for s in all() {
            let p = s.palette();
            for (what, c) in [
                ("text", p.text),
                ("secondary text", p.muted),
                ("faint text", p.faint),
                ("positive amounts", p.positive),
                ("negative amounts", p.negative),
                ("warnings", p.warning),
                ("links", p.accent),
            ] {
                for g in grounds(&p) {
                    let ratio = contrast(c, g);
                    assert!(
                        ratio >= READABLE,
                        "{}: {what} {c:?} on {g:?} is {ratio:.2}:1",
                        s.label()
                    );
                }
            }
            for c in p.series {
                assert!(
                    contrast(c, p.surface) >= 3.0,
                    "{}: a chart series",
                    s.label()
                );
            }
            // The series the charts show together (Net, Income and
            // Spending: 0, 1 and 2) look plainly different.
            for i in 0..3 {
                for j in 0..i {
                    let d = distance(p.series[i], p.series[j]);
                    assert!(
                        d >= SERIES_APART,
                        "{}: series {j} and {i} alike (ΔE {d:.1})",
                        s.label()
                    );
                }
            }
            // Gains, losses and plain text look different to the eye.
            for (what, a, b) in [
                ("gains and losses", p.positive, p.negative),
                ("gains and text", p.positive, p.text),
                ("losses and text", p.negative, p.text),
            ] {
                let d = distance(a, b);
                assert!(d >= 20.0, "{}: {what} alike (ΔE {d:.1})", s.label());
            }
            // Text over secondary over faint, each plainly above the next.
            let g = grounds(&p);
            let (t, m, f) = (worst(p.text, &g), worst(p.muted, &g), worst(p.faint, &g));
            assert!(
                t >= m * STEP * 0.999 && m >= f * STEP * 0.999,
                "{}: text {t:.2}, secondary {m:.2}, faint {f:.2}",
                s.label()
            );
            for c in p.series {
                assert!(
                    contrast(c, p.hover) >= 3.0,
                    "{}: a series on hover",
                    s.label()
                );
            }
            // The logo's initial reads on the accent.
            assert!(
                contrast(on(p.accent), p.accent) >= READABLE,
                "{}",
                s.label()
            );
            // Gains and losses never look alike: their hues lie at least
            // 30 degrees apart (or, for a scheme with greyish ones, the
            // colors plainly differ).
            let ((h0, s0, _), (h1, s1, _)) = (hsl(p.positive), hsl(p.negative));
            let apart = (h0 - h1).abs().min(1.0 - (h0 - h1).abs());
            let d = {
                let f = |x: u8, y: u8| (f32::from(x) - f32::from(y)).powi(2);
                let (a, b) = (p.positive, p.negative);
                (f(a.r(), b.r()) + f(a.g(), b.g()) + f(a.b(), b.b())).sqrt()
            };
            assert!(
                (s0 > 0.15 && s1 > 0.15 && apart >= 30.0 / 360.0) || d > 60.0,
                "{}: + {:?} and - {:?} alike",
                s.label(),
                p.positive,
                p.negative
            );
            // Cards stand out from the page.
            assert_ne!(p.bg, p.surface, "{}", s.label());
        }
    }

    #[test]
    fn a_scheme_is_dark_exactly_when_its_background_is() {
        for s in all() {
            let p = s.palette();
            let dark_bg = luminance(p.bg) < luminance(p.text);
            assert_eq!(s.dark, dark_bg, "{}", s.label());
        }
    }

    #[test]
    fn the_filter_shows_exactly_its_mode() {
        let count = |f| families(f).iter().map(|(_, v)| v.len()).sum::<usize>();
        assert_eq!(count(Filter::All), all().count());
        assert_eq!(count(Filter::Light) + count(Filter::Dark), all().count());
        for (_, variants) in families(Filter::Light) {
            assert!(variants.iter().all(|s| !s.dark));
        }
        for (_, variants) in families(Filter::Dark) {
            assert!(variants.iter().all(|s| s.dark));
        }
        // A family with no variant of the mode isn't listed (Nord has no
        // light one).
        assert!(!families(Filter::Light).iter().any(|(f, _)| *f == "Nord"));
        for f in [Filter::All, Filter::Light, Filter::Dark] {
            assert_eq!(Filter::from_name(f.name()), Some(f));
        }
        assert_eq!(Filter::from_name("Sepia"), None);
    }

    #[test]
    fn a_kept_scheme_comes_back_and_a_lost_one_falls_back() {
        for s in all() {
            assert!(std::ptr::eq(Scheme::from_key(&s.key(), !s.dark), s));
        }
        assert!(std::ptr::eq(Scheme::from_key("Nope/Gone", true), own(true)));
        assert!(std::ptr::eq(Scheme::from_key("", false), own(false)));
        assert_eq!(own(false).palette().bg, crate::theme::LIGHT.bg);
        assert_eq!(own(true).palette().bg, crate::theme::DARK.bg);
        assert_eq!(Scheme::from_key("Ayu/Mirage", false).label(), "Ayu Mirage");
        assert_eq!(Scheme::from_key("Nord/Nord", false).label(), "Nord");
    }

    #[test]
    fn hsl_round_trips_and_keeps_hue() {
        for c in [
            Color32::from_rgb(0xe6, 0x7e, 0x80),
            Color32::from_rgb(0xa7, 0xc0, 0x80),
            Color32::from_rgb(0x12, 0x34, 0x56),
            Color32::from_rgb(0x80, 0x80, 0x80),
            Color32::WHITE,
            Color32::BLACK,
        ] {
            let (h, s, l) = hsl(c);
            assert_eq!(from_hsl(h, s, l), c);
        }
        // A red made lighter is still a red, not a pink-grey.
        let red = Color32::from_rgb(0xe6, 0x7e, 0x80);
        let dark = Color32::from_rgb(0x30, 0x30, 0x30);
        let fixed = legible(red, Color32::WHITE, &[dark], 7.0);
        assert!(contrast(fixed, dark) >= 7.0);
        let ((h0, s0, _), (h1, s1, _)) = (hsl(red), hsl(fixed));
        assert!(
            (h0 - h1).abs() < 0.02 && (s0 - s1).abs() < 0.05,
            "{fixed:?}"
        );
    }

    #[test]
    fn a_color_already_readable_is_left_as_it_is() {
        let white = Color32::WHITE;
        let navy = Color32::from_rgb(0x00, 0x00, 0x80);
        assert_eq!(legible(navy, Color32::BLACK, &[white], READABLE), navy);
        // A pale yellow on white is darkened just enough.
        let pale = Color32::from_rgb(0xFF, 0xEE, 0x88);
        let fixed = legible(pale, Color32::BLACK, &[white], READABLE);
        assert!(contrast(fixed, white) >= READABLE);
        assert!(contrast(pale, white) < READABLE);
        assert!((contrast(fixed, white) - READABLE) < 1.0, "moved too far");
    }
}

/// Every scheme on the viewer's pages, as painted: the same words and
/// figures in the same places, each readable on what is under it.
#[cfg(test)]
mod painted {
    use super::*;
    use crate::nav::{ReportKind, Route};
    use crate::test_support::{Painted, painted, rects};
    use egui_kittest::Harness;

    /// The text that can be seen, and where it is, to the pixel.
    fn words(texts: &[Seen], label: &str) -> Vec<(String, i32, i32)> {
        // The scheme button names the scheme in force: that one differs.
        // A document's thumbnail still rendering shows "…" (documents.rs,
        // Render::Loading) until its picture comes, whenever that is: not
        // a word of the page (its contrast is still checked). CI's slower
        // machines caught a page drawn with a few still loading.
        let button = format!("{} {label}", egui_phosphor::regular::PALETTE);
        texts
            .iter()
            .filter(|t| t.text != button && t.text != "…")
            .map(|t| {
                (
                    t.text.clone(),
                    t.pos.x.round() as i32,
                    t.pos.y.round() as i32,
                )
            })
            .collect()
    }

    /// `c` laid over `ground` (a translucent color as seen).
    fn over(c: Color32, ground: Color32) -> Color32 {
        let [r, g, b, a] = c.to_srgba_unmultiplied();
        let t = f32::from(a) / 255.0;
        mix(ground, Color32::from_rgb(r, g, b), t)
    }

    /// One run of painted text, its color as seen, and the ground under
    /// it as painted by then.
    struct Seen {
        text: String,
        pos: egui::Pos2,
        color: Color32,
        ground: Color32,
    }

    /// Every run of text a frame painted where it can be seen, with the
    /// ground under its middle: the fills painted before it, in paint
    /// order, each cut to its clip, translucent ones laid over what was
    /// below, all on `base` (the page).
    fn seen(out: &egui::FullOutput, base: Color32) -> Vec<Seen> {
        fn walk(
            s: &egui::Shape,
            clip: egui::Rect,
            base: Color32,
            fills: &mut Vec<(egui::Rect, Color32)>,
            out: &mut Vec<Seen>,
        ) {
            match s {
                egui::Shape::Rect(r) if r.fill.a() > 0 => {
                    fills.push((r.rect.intersect(clip), r.fill));
                }
                egui::Shape::Text(t) => {
                    let rect = egui::Rect::from_min_size(t.pos, t.galley.size());
                    let mid = rect.center();
                    if !clip.contains(mid) {
                        return;
                    }
                    let ground = fills
                        .iter()
                        .filter(|(r, _)| r.contains(mid))
                        .fold(base, |g, (_, c)| over(*c, g));
                    let job = &t.galley.job;
                    for section in &job.sections {
                        let range = &section.byte_range;
                        let text = &job.text[range.start.0..range.end.0];
                        if text.trim().is_empty() {
                            continue;
                        }
                        let color = t.override_text_color.unwrap_or(
                            if section.format.color == Color32::PLACEHOLDER {
                                t.fallback_color
                            } else {
                                section.format.color
                            },
                        );
                        out.push(Seen {
                            text: text.to_string(),
                            pos: t.pos,
                            color: over(color, ground),
                            ground,
                        });
                    }
                }
                egui::Shape::Vec(v) => {
                    for s in v {
                        walk(s, clip, base, fills, out);
                    }
                }
                _ => {}
            }
        }
        let mut fills = Vec::new();
        let mut texts = Vec::new();
        for c in &out.shapes {
            walk(&c.shape, c.clip_rect, base, &mut fills, &mut texts);
        }
        texts
    }

    #[test]
    fn the_system_turning_light_or_dark_changes_nothing() {
        let (_dir, books) = crate::test_support::Build::new("schemes-flip").books();
        let mut h = Harness::builder()
            .with_size(egui::vec2(1400.0, 1200.0))
            .build_eframe(|cc| crate::App::with_books(&cc.egui_ctx, books, false, Route::Overview));
        // As eframe starts: following the system (kittest pins a theme).
        h.ctx
            .options_mut(|o| o.theme_preference = egui::ThemePreference::System);
        h.run_steps(3);
        for (key, system) in [
            ("Catppuccin/Mocha", egui::Theme::Light),
            ("Catppuccin/Latte", egui::Theme::Dark),
            ("Wanderbooks/Dark", egui::Theme::Light),
        ] {
            let ctx = h.ctx.clone();
            h.state_mut().set_scheme(&ctx, key);
            h.run_steps(2);
            let before = painted(h.output());
            let fills_before: Vec<Color32> =
                rects(h.output()).into_iter().map(|(_, c)| c).collect();
            h.input_mut().system_theme = Some(system);
            h.run_steps(3);
            let p = Scheme::from_key(key, false).palette();
            let v = h.ctx.global_style().visuals.clone();
            assert_eq!(v.override_text_color, Some(p.text), "{key}");
            assert_eq!(v.panel_fill, p.bg, "{key}");
            let after = painted(h.output());
            let colors = |t: &[Painted]| t.iter().map(|t| t.color).collect::<Vec<_>>();
            assert_eq!(colors(&after), colors(&before), "{key}: a color changed");
            let fills =
                |r: Vec<(egui::Rect, Color32)>| r.into_iter().map(|(_, c)| c).collect::<Vec<_>>();
            assert_eq!(
                fills(rects(h.output())),
                fills_before,
                "{key}: a fill changed"
            );
        }
    }

    /// One page in every scheme; how many texts were checked.
    fn page_in_every_scheme(source: &crate::data::Source, route: Route) -> usize {
        let mut checked = 0usize;
        let books = crate::data::load_once(source).unwrap();
        let mut h = Harness::builder()
            .with_size(egui::vec2(1400.0, 2400.0))
            .build_eframe(|cc| crate::App::with_books(&cc.egui_ctx, books, false, route.clone()));
        h.run_steps(4);
        let mut first: Option<Vec<(String, i32, i32)>> = None;
        for s in all() {
            let ctx = h.ctx.clone();
            h.state_mut().set_scheme(&ctx, &s.key());
            h.run_steps(3);
            // Thumbnails render in the background: all of them first.
            h.state_mut().settle(&ctx);
            h.run_steps(2);
            let texts = seen(h.output(), s.palette().bg);
            let p = s.palette();
            let shown = crate::theme::pal(&h.ctx);
            assert_eq!((shown.bg, shown.text), (p.bg, p.text), "{}", s.label());
            let now = words(&texts, &s.label());
            assert!(now.len() > 10, "{route:?}: little was painted");
            match &first {
                None => first = Some(now),
                Some(want) => assert_eq!(&now, want, "{route:?} in {}", s.label()),
            }
            // Each run of text against what was painted under it by
            // then.
            for t in texts {
                let ratio = contrast(t.color, t.ground);
                assert!(
                    ratio >= READABLE,
                    "{route:?} in {}: {:?} at {:?} is {ratio:.2}:1 ({:?} on {:?})",
                    s.label(),
                    t.text,
                    t.pos,
                    t.color,
                    t.ground
                );
                checked += 1;
            }
        }
        checked
    }

    #[test]
    fn every_scheme_paints_the_same_words_and_figures_readably() {
        let dir = crate::test_support::TempDir(
            std::env::temp_dir().join(format!("ledger-gui-schemes-painted-{}", std::process::id())),
        );
        let _ = std::fs::remove_dir_all(&dir.0);
        let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
        crate::demo::build(&dir.0, today).unwrap();
        let source = crate::data::Source {
            data_dir: dir.0.join("books-data"),
            books_toml: Some(dir.0.join("wanderbooks.toml")),
            today: Some(today),
        };
        let entry = crate::data::load_once(&source)
            .unwrap()
            .ledger
            .posted_entries()
            .next()
            .map(|e| e.id())
            .expect("the demo books have entries");
        let routes = [
            Route::Entry(entry),
            Route::Company,
            Route::Assets,
            Route::Documents,
            Route::Overview,
            Route::Cash,
            Route::Profit,
            Route::Transactions,
            Route::Reports(ReportKind::ProfitAndLoss),
            Route::Reports(ReportKind::BalanceSheet),
            Route::Accounts,
            Route::Parties,
            Route::Journal,
        ];
        // A page a thread: each checks every scheme.
        let checked: usize = std::thread::scope(|scope| {
            let threads: Vec<_> = routes
                .into_iter()
                .map(|route| {
                    let source = &source;
                    scope.spawn(move || page_in_every_scheme(source, route))
                })
                .collect();
            threads.into_iter().map(|t| t.join().unwrap()).sum()
        });
        assert!(checked > 10_000, "{checked} texts checked");
    }
}
