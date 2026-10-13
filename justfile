# wanderbooks repo recipes. `just` lists them.

set shell := ["bash", "-cu"]

# The Netlify CLI, fetched on demand.
netlify := "npx --yes netlify-cli@27"

# List recipes
default:
    @just --list

# Everything CI should check: format, clippy (zero warnings), tests
check:
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace

# Formal proofs: Kani on the accounting kernel, Lean for books of any size (docs/verification.md)
prove:
    #!/usr/bin/env bash
    set -euo pipefail
    # Capped: a harness that blows up should fail, not eat the machine.
    log=$(mktemp)
    (ulimit -v 12000000; cd crates/ledger-core &&
        timeout 1800 cargo kani -j "$(nproc)" --output-format terse | tee "$log")
    # A cover that can't be satisfied means a proof is vacuous.
    if grep -E "\*\* [0-9]+ of [0-9]+ cover properties satisfied" "$log" |
        awk '{ if ($2 != $4) bad = 1 } END { exit !bad }'; then
        echo "Kani: an unsatisfiable cover; some proof is vacuous" >&2
        exit 1
    fi
    rm -f "$log"
    cd verify/lean
    lake build
    audit=$(lake env lean Audit.lean)
    if grep -v -E "depends on axioms: \[(propext|Quot.sound|Classical.choice)(, (propext|Quot.sound|Classical.choice))*\]$|does not depend on any axioms" <<<"$audit"; then
        echo "Lean audit: a theorem rests on something beyond the standard axioms" >&2
        exit 1
    fi
    echo "Lean: $(wc -l <<<"$audit") headline theorems, standard axioms only"

# Build the release binaries (CLI, MCP server, GUI)
build:
    cargo build --release -p ledger-cli -p ledger-mcp -p ledger-gui

# Install the binaries from this checkout into ~/.cargo/bin
install:
    for c in ledger-cli ledger-mcp ledger-gui; do cargo install --locked --path crates/$c; done

# Open the GUI on the demo company
demo:
    cargo run --release -p ledger-gui -- --demo

# Regenerate the website's screenshots (dark) from the demo company
site-shots:
    #!/usr/bin/env bash
    set -euo pipefail
    tmp=$(mktemp -d)
    SHOT_DIR="$tmp" SHOT_DARK=1 SHOT_LIVE=1 SHOT_ONLY=overview,pnl,cash cargo test -p ledger-gui --test screens
    # The hero's viewer pane: taller, to sit beside the session.
    SHOT_DIR="$tmp" SHOT_DARK=1 SHOT_LIVE=1 SHOT_ONLY=entry SHOT_ENTRY=245 SHOT_HEIGHT=1150 cargo test -p ledger-gui --test screens
    for f in overview entry pnl cash; do cp "$tmp/$f.png" site/assets/; done
    rm -f site/assets/overview-dark.png
    rm -rf "$tmp"

# Assemble the website in site/dist (index.html, install.sh, assets)
site:
    rm -rf site/dist && mkdir -p site/dist
    cp site/index.html install.sh site/dist/
    cp -r site/assets site/dist/

# Deploy the website to wanderbooks.ai (Netlify; `netlify login` once first)
deploy: site
    {{netlify}} deploy --prod --dir site/dist --no-build

# Deploy a preview of the website and print its URL
deploy-preview: site
    {{netlify}} deploy --dir site/dist --no-build
