# Color schemes in the viewer

The viewer's color-scheme menu (the button in the sidebar's lower corner)
offers its own scheme, Wanderbooks Light and Dark, and twenty families of
popular editor schemes, each with all its variants: 80 schemes in all (78
and the viewer's own two). The menu's filter narrows the list to light or
dark schemes. Families are listed alphabetically, each one's variants too,
in as many columns as the window's height needs, so every family is on
screen at once; a variant's whole row (its colors and its name) chooses
it.
The choice is a viewer preference, kept with eframe's other preferences;
nothing is written to the books, and no figure or word changes with the
scheme (`schemes.rs`, `painted`).

Owner, 2026-10-19: "full blown colorscheme support … all the top neovim
style colorschemes"; "on the order of 20 families, each with a flyout of all
its variants"; a light/dark/all filter; Ayu included.

## Where the colors come from

Each scheme's sixteen colors (`crates/ledger-gui/src/schemes/table.rs`) were
taken from its official source and checked value by value (2026-10-19); the
table names the source above each family:

| Column | What it is |
|---|---|
| bg | the editor background (Normal) |
| dim | a panel or sidebar background (mantle, bg_dark, bg0 …) |
| raised | a raised surface (surface0, bg_highlight, float …); recorded, not used |
| sel | the visual-selection background; recorded, not used |
| border | the split or border color; recorded, not used |
| fg | the text (Normal) |
| muted | secondary text (subtext, fg_dark, subtle …) |
| faint | comments |
| accent | the scheme's signature accent: in most its blue; Ayu's yellow-orange, Dracula's purple, Rosé Pine's iris, Monokai Pro's yellow, Material's teal, Nord's frost cyan (nord8), GitHub's own accent blue |
| red, green, yellow, orange, blue, purple, cyan | its named colors |

Where a scheme has no color for a column, the closest official one stands
in (Dracula has no blue: its ANSI blue, its purple; Sonokai has no cyan: its
blue; Oxocarbon has no yellow: IBM Carbon's, and no orange: its pink). A few
columns repeat another for the same reason: seven schemes have one
background (Gruvbox Dark Hard, Nord, Solarized Dark and Light, Cyberdream
Dark and Light, Melange Dark), dim being their bg. Variant names are the plugins' own (GitHub
Dark Default, not Dark, which is another theme).

## How the viewer paints with them

- The page is the darker of a scheme's bg and dim, the cards the lighter,
  so cards stand out in light and dark schemes alike. A scheme with one
  background gets a second a shade toward its text: lighter cards in a
  dark scheme, a darker page in a light one.
- Secondary text is the scheme's muted color, unless that is no dimmer than
  its text (Nord's documented "subtle" tone is brighter): then a tone
  between its text and its comments.
- Selection is the cards tinted with the accent; the new-entry highlight is
  the cards lightly tinted with the yellow (at 10%: a stronger tint left
  too little room for colored amounts on it, and bleached Nord's red to a
  pink).
- The charts' series are the scheme's blue, cyan, yellow, purple, orange,
  green, red and accent, in that order, each that looks like one already
  taken (CIE76 ΔE under 20) passed over: Monokai Pro and Sonokai name their
  cyan with their blue, and Kanagawa Dragon's blues, cyans and purples are
  near one another. The three the charts show together (net, income,
  spending) are always plainly different.
- **Readable, always.** Every color that carries text or a figure (text,
  secondary text, the faintest text, which carries chart amounts and entry
  numbers, gains, losses, warnings, links) reaches WCAG AA contrast (4.5:1)
  on every background it is drawn on: page, cards, sidebar, hover,
  selection, the new-entry highlight, and the soft fills of pills and
  badges. A scheme's color that falls short keeps its hue and saturation
  and only gets lighter (dark schemes) or darker (light ones), just far
  enough. Gains, losses and plain text stay plainly different to the eye
  (ΔE 20 or more apart). egui's own faint text (placeholders) is the
  secondary text; the logo's initial is black or white, whichever reads on
  the accent. The viewer's own palettes go through the same rule (it lifted
  Wanderbooks Light's green on the sidebar from 4.35:1, its faint text and
  warnings too, and Wanderbooks Dark's faint text).
- **Three levels of text, always apart.** Lifted one by one, secondary and
  faint text came out alike (identical in some schemes, faint above
  secondary in others). So the faintest is lifted first, then secondary
  text until it reads at least 1.2 times better than the faint on the
  ground it reads worst on, then the text the same over secondary.
- The scheme sets both of egui's styles and pins its light or dark theme,
  so the system turning light or dark later changes nothing.
