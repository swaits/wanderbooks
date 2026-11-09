#!/usr/bin/env bash
# Money arithmetic outside the kernel (AGENTS.md, "Formal methods: the
# bar", item 1: a sum, a sign flip, a split or a rounding written anywhere
# but `kernel/` is a second copy, and the engine never gets one).
#
#   scripts/raw-decimal-arith.sh [package [file.rs ...]]
#
# The package defaults to ledger-core. With file names after it (as they
# are under the package's src/: `reports.rs`, `kernel/fx.rs`), only the
# places in those files are printed and counted.
#
# Prints one line for every place in the package's own source (tests and
# proof harnesses are not compiled, `src/kernel/` is where arithmetic
# belongs) that does `Decimal` arithmetic without the kernel:
#
#   op      a raw operator on a `Decimal`: + - * / %, their `=` forms, unary -
#   method  one of `rust_decimal`'s own arithmetic or rounding methods
#           (`a.checked_add(b)` on a `Decimal` is rust_decimal's, not the
#           kernel's, even with `Amount` in scope: an inherent method wins)
#   sum     `Iterator::sum` or `Iterator::product`, of any type
#
# then the count. Exit 0 when there are none, 1 when there are some, 2
# when the tools could not run. A place under an `#[allow(clippy::...)]`
# for either lint is not printed: that is the waiver, it carries a
# `reason`, and the last line counts them so one cannot slip in unseen.
#
# Why it matters, beyond the rule: a raw operator panics when the result
# does not fit, and rust_decimal's operators and `checked_*` methods alike
# round a result that needs more than 96 bits instead of refusing it. The
# kernel's `Amount` for `Decimal` is exact or `None` (K-U10), and Kani and
# Lean prove the laws over the kernel only.
#
# How: clippy cannot limit `arithmetic_side_effects` to one type, so it
# runs twice, once flagging every type and once with `Decimal` allowed;
# the places that drop out are the `Decimal` ones. The methods come from
# `disallowed_methods`. Each run has its own target directory, so one
# never reuses the other's result.
set -euo pipefail

pkg=${1:-ledger-core}
[ $# -gt 0 ] && shift
only=("$@")
cd "$(git rev-parse --show-toplevel)"
[ -d "crates/$pkg/src" ] || { echo "raw-decimal-arith: no crates/$pkg/src" >&2; exit 2; }
for tool in cargo jq; do
    command -v "$tool" >/dev/null || { echo "raw-decimal-arith: $tool is not installed (mise install)" >&2; exit 2; }
done

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir "$work/all" "$work/rest"

# rust_decimal's inherent arithmetic and rounding, and the two iterator
# folds that add or multiply without a check.
methods='
  "rust_decimal::Decimal::checked_add", "rust_decimal::Decimal::checked_sub",
  "rust_decimal::Decimal::checked_mul", "rust_decimal::Decimal::checked_div",
  "rust_decimal::Decimal::checked_rem",
  "rust_decimal::Decimal::saturating_add", "rust_decimal::Decimal::saturating_sub",
  "rust_decimal::Decimal::saturating_mul",
  "rust_decimal::Decimal::round", "rust_decimal::Decimal::round_dp",
  "rust_decimal::Decimal::round_dp_with_strategy", "rust_decimal::Decimal::round_sf",
  "rust_decimal::Decimal::round_sf_with_strategy",
  "rust_decimal::Decimal::trunc", "rust_decimal::Decimal::trunc_with_scale",
  "rust_decimal::Decimal::floor", "rust_decimal::Decimal::ceil",
  "rust_decimal::Decimal::rescale",
  "core::iter::Iterator::sum", "core::iter::Iterator::product",
'
printf 'disallowed-methods = [%s]\n' "$methods" > "$work/all/clippy.toml"
{
    printf 'disallowed-methods = [%s]\n' "$methods"
    printf 'arithmetic-side-effects-allowed = ["rust_decimal::Decimal", "rust_decimal::decimal::Decimal"]\n'
} > "$work/rest/clippy.toml"

# One clippy run: the findings as `file<TAB>line<TAB>column<TAB>lint<TAB>what<TAB>source`.
run() {
    local name=$1 status=0
    CLIPPY_CONF_DIR="$work/$name" cargo clippy --quiet -p "$pkg" --lib \
        --target-dir "target/raw-arith/$name" --message-format=json -- \
        -A clippy::all -W clippy::arithmetic_side_effects -W clippy::disallowed_methods \
        > "$work/$name.json" 2> "$work/$name.err" || status=$?
    if [ "$status" -ne 0 ]; then
        cat "$work/$name.err" >&2
        echo "raw-decimal-arith: clippy did not finish on $pkg" >&2
        exit 2
    fi
    jq -r --arg src "crates/$pkg/src/" '
        select(.reason == "compiler-message") | .message
        | select(.code != null)
        | select(.code.code == "clippy::arithmetic_side_effects" or .code.code == "clippy::disallowed_methods")
        | .code.code as $lint | .message as $what
        | .spans[] | select(.is_primary) | select(.file_name | startswith($src))
        | [.file_name, .line_start, .column_start, $lint, $what, (.text[0].text // "" | gsub("^\\s+|\\s+$"; ""))]
        | @tsv' "$work/$name.json" | sort -u > "$work/$name.tsv"
}
run all
run rest

# A `Decimal` operator is flagged by the first run and not by the second.
cut -f1-3 "$work/rest.tsv" | sort -u > "$work/rest.keys"
awk -F'\t' -v keys="$work/rest.keys" '
    BEGIN { while ((getline k < keys) > 0) rest[k] = 1 }
    index($1, "/src/kernel/") || index($1, "/src/proofs/") { next }
    $4 == "clippy::arithmetic_side_effects" {
        if (!(($1 "\t" $2 "\t" $3) in rest)) print $1 ":" $2 ":" $3 ": op: " $6
        next
    }
    {
        kind = ($5 ~ /Iterator::(sum|product)/) ? "sum" : "method"
        name = $5; sub(/^[^`]*`/, "", name); sub(/`.*$/, "", name); sub(/^.*::/, "", name)
        print $1 ":" $2 ":" $3 ": " kind " " name ": " $6
    }' "$work/all.tsv" | sort -t: -k1,1 -k2,2n -k3,3n > "$work/sites.txt"

if [ ${#only[@]} -gt 0 ]; then
    for f in "${only[@]}"; do
        [ -f "crates/$pkg/src/$f" ] || { echo "raw-decimal-arith: no crates/$pkg/src/$f" >&2; exit 2; }
        grep -F "crates/$pkg/src/$f:" "$work/sites.txt" || true
    done > "$work/chosen.txt"
    mv "$work/chosen.txt" "$work/sites.txt"
fi

cat "$work/sites.txt"
count=$(wc -l < "$work/sites.txt")
waived=$( { grep -rEn 'allow\(clippy::(arithmetic_side_effects|disallowed_methods)' "crates/$pkg/src" \
    --include='*.rs' || true; } | { grep -v -e '/src/kernel/' -e '/src/proofs/' || true; } | wc -l)
echo "raw-decimal-arith: $pkg: $count places do Decimal arithmetic outside the kernel ($waived waived by an allow)"
[ "$count" -eq 0 ]
