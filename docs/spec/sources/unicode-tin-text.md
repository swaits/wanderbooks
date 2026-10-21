# Unicode character properties: how a TIN can be written

Governs `ledger_core::parties::has_full_tin` (rules.md B-26): which
characters separate a TIN's digits, and which are invisible between them.
The Unicode Character Database is © Unicode, Inc., under the Unicode
License v3, which permits copying the data files; the code point lists
below are transcribed from them (facts, not prose).

## PropList.txt, Unicode 17.0.0 (2025-06-30)
Source: <https://www.unicode.org/Public/17.0.0/ucd/PropList.txt>  
Retrieved: 2026-10-20  
SHA-256 of the retrieved file: `130dcddcaadaf071008bdfce1e7743e04fdfbc910886f017d9f9ac931d8c64dd`

**Dash** (every dash and hyphen): U+002D, U+058A ARMENIAN HYPHEN, U+05BE
HEBREW PUNCTUATION MAQAF, U+1400, U+1806, U+2010..U+2015 (HYPHEN,
NON-BREAKING HYPHEN, FIGURE DASH, EN DASH, EM DASH, HORIZONTAL BAR),
U+2053 SWUNG DASH, U+207B SUPERSCRIPT MINUS, U+208B SUBSCRIPT MINUS,
U+2212 MINUS SIGN, U+2E17, U+2E1A, U+2E3A..U+2E3B, U+2E40, U+2E5D,
U+301C WAVE DASH, U+3030 WAVY DASH, U+30A0, U+FE31..U+FE32, U+FE58 SMALL
EM DASH, U+FE63 SMALL HYPHEN-MINUS, U+FF0D FULLWIDTH HYPHEN-MINUS,
U+10D6E, U+10EAD.

**White_Space**: U+0009..U+000D, U+0020, U+0085, U+00A0, U+1680,
U+2000..U+200A, U+2028 LINE SEPARATOR, U+2029 PARAGRAPH SEPARATOR,
U+202F, U+205F, U+3000 IDEOGRAPHIC SPACE. Less the controls and the two
line and paragraph separators, these are the space separators (general
category Zs). Rust's `char::is_whitespace` is this property.

## DerivedCoreProperties.txt, Unicode 17.0.0
Source: <https://www.unicode.org/Public/17.0.0/ucd/DerivedCoreProperties.txt>  
Retrieved: 2026-10-20  
SHA-256 of the retrieved file: `24c7fed1195c482faaefd5c1e7eb821c5ee1fb6de07ecdbaa64b56a99da22c08`

**Default_Ignorable_Code_Point** (drawn as nothing when not supported):
U+00AD SOFT HYPHEN, U+034F, U+061C ARABIC LETTER MARK, U+115F..U+1160,
U+17B4..U+17B5, U+180B..U+180F, U+200B..U+200F (ZERO WIDTH SPACE, the
joiners, LEFT-TO-RIGHT and RIGHT-TO-LEFT MARK), U+202A..U+202E,
U+2060..U+206F (WORD JOINER, the invisible operators, the directional
isolates), U+3164, U+FE00..U+FE0F (variation selectors), U+FEFF ZERO
WIDTH NO-BREAK SPACE, U+FFA0, U+FFF0..U+FFF8, U+1BCA0..U+1BCA3,
U+1D173..U+1D17A, U+E0000..U+E0FFF (tags, variation selectors
supplement).

## Compatibility decompositions (UnicodeData.txt)
Derived 2026-10-20 with Python's `unicodedata` (Unicode 16.0.0; these
mappings are unchanged in 17.0): the characters past ASCII whose NFKC
form is one of the ASCII separators.

- `' '`: U+00A0, U+2000..U+200A, U+202F, U+205F, U+3000 (all Zs).
- `'-'`: U+FE63, U+FF0D (both Dash).
- `'.'`: U+2024 ONE DOT LEADER, U+FE52 SMALL FULL STOP, U+FF0E FULLWIDTH
  FULL STOP.
- `'/'`: U+FF0F FULLWIDTH SOLIDUS.
- `'_'`: U+FE33, U+FE34, U+FE4D..U+FE4F, U+FF3F FULLWIDTH LOW LINE.

## How the books read them (rules.md B-26)

A TIN is nine numerals (any script's: `char::is_numeric`) with at most
three separators between neighbors. In ASCII the separators are exactly
the space, hyphen, dot, slash and underscore (a tab or line break is
not one). Past ASCII they are every Zs space, every Dash character, and
the compatibility forms of the dot, slash and underscore above; the line
and paragraph separators and NEL are line breaks, as in ASCII, and end a
run. Default-ignorable characters are invisible, so any number of them
may sit between two digits without counting toward the three.
