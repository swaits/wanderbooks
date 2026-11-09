#!/bin/bash
# The coverage floor (docs/coverage-floor.md): `just coverage` measures the
# core's and the store's line, region and branch coverage into
# target/coverage/<crate>/; this reads those reports and fails when a
# crate's total line coverage has fallen more than 0.5 percentage points
# below its recorded floor. The report is not regenerated here on purpose:
# `just coverage-check` is the cheap gate over the last `just coverage`
# run, in the same job in CI.
#
# Usage: scripts/coverage-floor.sh
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
doc=docs/coverage-floor.md

if [ ! -f "$doc" ]; then
    echo "coverage floor: $doc is missing" >&2
    exit 1
fi

status=0
for crate in ledger-core ledger-store; do
    summary="target/coverage/$crate/summary.txt"
    if [ ! -f "$summary" ]; then
        echo "coverage floor: $summary is missing; run \`just coverage\` first" >&2
        exit 1
    fi

    # The `**Floor: ...**` line, e.g. `ledger-core` 87.34% lines.
    floor=$(sed -n "s/.*\`$crate\` \([0-9][0-9]*\.[0-9][0-9]*\)% lines.*/\1/p" "$doc" | head -n 1)
    if [ -z "$floor" ]; then
        echo "coverage floor: $doc has no floor for $crate" >&2
        exit 1
    fi

    # THE report table's TOTAL row: columns are Regions, Missed Regions,
    # Cover, Functions, Missed Functions, Executed, Lines, Missed Lines,
    # Cover, Branches, Missed Branches, Cover. $10 is line cover, $13
    # branch cover, $4 region cover.
    total=$(awk '$1 == "TOTAL" { print $10 }' "$summary" | tr -d '%')
    if [ -z "$total" ]; then
        echo "coverage floor: $summary has no TOTAL row" >&2
        exit 1
    fi

    if awk -v actual="$total" -v floor="$floor" \
        'BEGIN { exit !(actual + 0 >= floor - 0.5) }'; then
        echo "coverage floor: $crate ${total}% lines (floor ${floor}%, tolerance 0.5 pp)"
    else
        echo "coverage floor: $crate is at ${total}% lines, more than 0.5 pp below its ${floor}% floor (docs/coverage-floor.md)" >&2
        status=1
    fi
done

exit "$status"
