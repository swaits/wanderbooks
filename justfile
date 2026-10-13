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
