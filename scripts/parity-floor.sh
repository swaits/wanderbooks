#!/bin/bash
# The parity catalog's floor never decreases (docs/parity.md; AGENTS.md,
# "Parity tracking"). `crates/ledger-cli/tests/parity_floor.rs` checks the
# floor against the rows in one commit; this checks it against the base
# commit, which a test cannot do.
#
# Usage: scripts/parity-floor.sh <base-ref>
#   prints both floors; exits 1 when the working tree's floor is lower.
set -euo pipefail

base_ref="${1:?usage: scripts/parity-floor.sh <base-ref>}"
cd "$(git rev-parse --show-toplevel)"

floor_of() {
    # The first `**Floor: N done**` line, or nothing.
    sed -n 's/^\*\*Floor: \([0-9][0-9]*\) done\*\*.*/\1/p' | head -n 1
}

if ! base_doc=$(git show "${base_ref}:docs/parity.md" 2>/dev/null); then
    echo "parity floor: cannot read docs/parity.md at ${base_ref}" >&2
    exit 1
fi
base_floor=$(printf '%s\n' "$base_doc" | floor_of)

if [ -z "$base_floor" ]; then
    echo "parity floor: ${base_ref} has no floor line; nothing to compare"
    exit 0
fi

work_floor=$(floor_of < docs/parity.md)
if [ -z "$work_floor" ]; then
    echo "parity floor: docs/parity.md in the working tree has no floor line" >&2
    exit 1
fi

echo "parity floor: ${base_ref} says ${base_floor} done; the working tree says ${work_floor} done"
if [ "$work_floor" -lt "$base_floor" ]; then
    echo "parity floor: the floor never decreases (docs/parity.md; AGENTS.md, \"Parity tracking\")" >&2
    exit 1
fi
