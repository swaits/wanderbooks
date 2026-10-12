#!/bin/sh
# wanderbooks installer. One line on a new machine:
#
#   curl -fsSL https://git.swaits.com/swaits/wanderbooks/raw/branch/main/install.sh | sh -s -- ~/Books/Acme
#
# Installs Rust if needed, then the wanderbooks CLI, MCP server, and GUI
# (into ~/.cargo/bin), then — given a directory — makes your books there and
# starts the agent. Without a directory it just installs.
set -eu

REPO="https://git.swaits.com/swaits/wanderbooks"

say() { printf '\033[1m%s\033[0m\n' "$*"; }

if ! command -v cargo >/dev/null 2>&1; then
    say "Installing Rust (rustup)…"
    curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal
    # shellcheck disable=SC1091
    . "$HOME/.cargo/env"
fi

say "Installing wanderbooks (a few minutes, once)…"
cargo install --locked --git "$REPO" ledger-cli ledger-mcp ledger-gui

if ! command -v just >/dev/null 2>&1; then
    say "Installing just (for the books dir's shortcuts)…"
    cargo install --locked just
fi

if ! command -v claude >/dev/null 2>&1; then
    say "Claude Code isn't installed: see https://claude.com/claude-code"
fi

if [ $# -gt 0 ]; then
    # Hand the terminal to the agent when there is one.
    if [ -t 1 ] && [ -e /dev/tty ]; then
        exec "$HOME/.cargo/bin/wanderbooks" new "$@" </dev/tty
    fi
    exec "$HOME/.cargo/bin/wanderbooks" new "$@" --no-start
fi
say "Done. Make your books with: wanderbooks new ~/Books/YourCompany"
