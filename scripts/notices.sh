#!/usr/bin/env bash
# The third-party notices the program must travel with (MIT, Apache-2.0,
# BSD, ISC, Zlib, SIL OFL 1.1 and the rest all ask that their copyright
# notice and licence text go with every copy).
#
#   scripts/notices.sh           write THIRD-PARTY-NOTICES.txt
#   scripts/notices.sh --check   fail if it is out of date (`just notices-check`)
#
# The file has two parts. The first is cargo-about's: every crate the
# `wanderbooks` binary links, with the licence it carries and that
# licence's text. cargo-about reads the manifest of `ledger-cli` (the one
# executable), every feature and every platform the program ships on, so
# the list is what the program contains, not what this machine happens to
# build. The second part is the fonts the program embeds: each file under
# `crates/ledger-gui/assets/fonts/` and `crates/ledger-import/fonts/`,
# with the licence file beside it. A font file that no entry below names
# fails the script, so a new font cannot ship without its notice.
#
# The output is plain text, says nothing about this machine (no dates, no
# absolute paths), and is the same on two runs; `--check` compares it with
# the committed file byte for byte, so a changed dependency shows up as a
# red `just check` until the file is regenerated and committed.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
out=THIRD-PARTY-NOTICES.txt

case "${1:-}" in
    "" | --check) ;;
    *)
        echo "usage: scripts/notices.sh [--check]" >&2
        exit 2
        ;;
esac
check=${1:-}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Which licence each bundled font carries. Every `*.ttf` and `*.otf`
# under the two font folders must match one pattern here; a new font is
# added here beside its licence file, never left out of the notices.
font_map=(
    "crates/ledger-gui/assets/fonts/InterTab-*.ttf|crates/ledger-gui/assets/fonts/Inter-LICENSE.txt"
    "crates/ledger-gui/assets/fonts/Hack-Regular.ttf|crates/ledger-gui/assets/fonts/Hack-LICENSE.txt"
    "crates/ledger-gui/assets/fonts/NotoEmoji-Regular.ttf|crates/ledger-gui/assets/fonts/NotoEmoji-LICENSE.txt"
    "crates/ledger-gui/assets/fonts/emoji-icon-font.ttf|crates/ledger-gui/assets/fonts/emoji-icon-font-LICENSE.txt"
    # The PDFs' Inter is the same font and licence as the viewer's.
    "crates/ledger-import/fonts/Inter-*.ttf|crates/ledger-gui/assets/fonts/Inter-LICENSE.txt"
    "crates/ledger-import/fonts/NotoSans*-Regular.ttf|crates/ledger-import/fonts/NotoSans-LICENSE.txt"
    "crates/ledger-import/fonts/NotoSansCJKsc-*.otf|crates/ledger-import/fonts/NotoSansCJK-LICENSE.txt"
)

# font -> licence, filled from font_map below.
declare -A font_licence=()

for entry in "${font_map[@]}"; do
    pattern=${entry%%|*}
    licence=${entry#*|}
    matched=0
    for file in $pattern; do
        [ -f "$file" ] || continue
        font_licence["$file"]=$licence
        matched=1
    done
    if [ "$matched" -eq 0 ]; then
        echo "notices: no font matches $pattern (the map in scripts/notices.sh is stale)" >&2
        exit 2
    fi
done

# Every font file must be accounted for: a font that no entry names has
# no notice to travel with.
fonts=$(find crates/ledger-gui/assets/fonts crates/ledger-import/fonts \
    -type f \( -name '*.ttf' -o -name '*.otf' \) | sort)
for font in $fonts; do
    if [ -z "${font_licence[$font]:-}" ]; then
        echo "notices: no licence for $font: add it to the map in scripts/notices.sh" >&2
        exit 1
    fi
done

# The fonts section, grouped by licence file: every font that carries a
# licence is listed, then that licence's text once.
font_section() {
    echo "================================================================================"
    echo "FONTS"
    echo "================================================================================"
    echo
    echo "The program embeds the fonts below. Each font file is listed under"
    echo "the licence it carries, and that licence's text follows."
    echo
    local seen=() entry licence font found seen_licence
    for entry in "${font_map[@]}"; do
        licence=${entry#*|}
        found=0
        for seen_licence in ${seen[@]+"${seen[@]}"}; do
            [ "$seen_licence" = "$licence" ] && found=1
        done
        [ "$found" -eq 0 ] || continue
        seen+=("$licence")
        if [ ! -f "$licence" ]; then
            echo "notices: no licence file $licence" >&2
            exit 1
        fi
        echo "--------------------------------------------------------------------------------"
        echo "$licence"
        echo "--------------------------------------------------------------------------------"
        echo
        echo "Fonts:"
        while IFS= read -r font; do
            [ -n "$font" ] || continue
            [ "${font_licence[$font]:-}" = "$licence" ] || continue
            echo "  $font"
        done <<<"$fonts"
        echo
        cat "$licence"
        echo
    done
}

# One full copy of the notices, in $1.
write_notices() {
    local target=$1
    command -v cargo >/dev/null || {
        echo "notices: cargo is not installed (mise install)" >&2
        exit 2
    }
    # --offline: no network lookup, so the same crates give the same file
    # on every machine. The manifest is ledger-cli's, the one executable:
    # that dependency graph is what the binary links.
    cargo about generate --locked --all-features --offline \
        -m crates/ledger-cli/Cargo.toml -c about.toml \
        deploy/notices/about.hbs > "$work/about.txt"
    {
        cat "$work/about.txt"
        echo
        font_section
    } > "$work/all.txt"
    # Plain text: trailing whitespace off every line, and exactly one
    # newline at the end (the licence files and cargo-about differ).
    sed 's/[[:space:]]*$//' "$work/all.txt" > "$work/clean.txt"
    printf '%s\n' "$(cat "$work/clean.txt")" > "$target"
}

if [ "$check" = --check ]; then
    write_notices "$work/check.txt"
    if ! cmp -s "$work/check.txt" "$out"; then
        diff -u "$out" "$work/check.txt" | head -40 >&2 || true
        echo 'THIRD-PARTY-NOTICES.txt is out of date: run `just notices` and commit the result' >&2
        exit 1
    fi
    echo "notices: $out is up to date"
else
    write_notices "$out"
    echo "notices: wrote $out"
fi
