# wanderbooks repo recipes. `just` lists them.

set shell := ["bash", "-cu"]

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

# Regenerate the website's screenshots from the demo company
site-shots:
    #!/usr/bin/env bash
    set -euo pipefail
    tmp=$(mktemp -d)
    SHOT_DIR="$tmp/light" SHOT_ONLY=overview,entry,pnl SHOT_ENTRY=249 cargo test -p ledger-gui --test screens
    SHOT_DIR="$tmp/dark" SHOT_DARK=1 SHOT_ONLY=overview cargo test -p ledger-gui --test screens
    for f in overview entry pnl; do cp "$tmp/light/$f.png" site/assets/; done
    cp "$tmp/dark/overview.png" site/assets/overview-dark.png
    rm -rf "$tmp"

# Assemble the website in site/dist (index.html, install.sh, assets)
site:
    rm -rf site/dist && mkdir -p site/dist
    cp site/index.html install.sh site/dist/
    cp -r site/assets site/dist/

# Publish the website: just publish user@host:/var/www/wanderbooks
publish dest: site
    rsync -av --delete site/dist/ "{{dest}}"
