# The viewer's fonts

Every font the viewer draws with is in this folder or in
`crates/ledger-import/fonts/` (the PDFs' fonts, which the viewer shares),
each beside its licence. `theme.rs` (`font_definitions`) names them all;
egui's own default font set is switched off, since it includes Ubuntu
Light under the Ubuntu Font Licence 1.0, which Debian and Fedora class as
non-free. `deny.toml` does not allow that licence, so the crate that
carries it (`epaint_default_fonts`) cannot come back unnoticed.

| File | What it draws | Licence | From |
|---|---|---|---|
| `InterTab-Regular.ttf`, `InterTab-SemiBold.ttf`, `InterTab-Bold.ttf` | All text. Inter with tabular figures made the default and the private-use glyphs removed; named "InterTab" as a modified version | SIL OFL 1.1 (`Inter-LICENSE.txt`); Copyright 2016 The Inter Project Authors, no reserved font name | https://github.com/rsms/inter |
| `Hack-Regular.ttf` | Fixed-width text: identifiers, hashes, JSON | MIT, and the Bitstream Vera licence for the Vera outlines it descends from (`Hack-LICENSE.txt`) | `epaint_default_fonts` 0.36.2, unchanged |
| `NotoEmoji-Regular.ttf` | Symbols and emoji, one colour | SIL OFL 1.1 (`NotoEmoji-LICENSE.txt`); Copyright 2013 Google Inc. | `epaint_default_fonts` 0.36.2, unchanged |
| `emoji-icon-font.ttf` | Symbols Noto Emoji lacks (arrows on a menu, the date picker) | MIT (`emoji-icon-font-LICENSE.txt`); Copyright 2014 John Slegers | `epaint_default_fonts` 0.36.2, unchanged |

Phosphor's icons come from the `egui-phosphor` crate (MIT).

Why these licences fit a program under MIT OR Apache-2.0: the SIL Open
Font License lets a font be bundled, embedded and shipped with any
software, sold or free. It asks that the font not be sold by itself, that
its copyright and licence travel with it, and that a changed font stay
under the OFL without a name the authors reserved. It is the licence of
Inter itself and of the Noto fonts, and Debian, Fedora, the FSF and the
OSI all accept it. The Bitstream Vera licence asks the same things. None
of them reaches the program's own code.

SHA-256 of each file, so a changed font is a seen change:

```
a7ef8d15be539471b4a21095930d39f3af50f676de32a21ee515225692c8d0b4  InterTab-Bold.ttf
0e41e795bbb28ee70c6417349e430149c19e98d87485e6b6308e1ab142ee43ca  InterTab-Regular.ttf
366d6fc641ac090310828b5e9c3c33331bf0699f84cb421b283502e18ae4fcd7  InterTab-SemiBold.ttf
15f55cc0c85a2988d2b4b3a8cdb5d77fdfbaf319e1bb5309d725db9818fb7125  Hack-Regular.ttf
415dc6290378574135b64c808dc640c1df7531973290c4970c51fdeb849cb0c5  NotoEmoji-Regular.ttf
f426bec371f484646d002717e2c555a95cd5145e635d83a48bbef83f3abf18ca  emoji-icon-font.ttf
```
