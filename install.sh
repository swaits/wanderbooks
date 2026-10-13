#!/bin/sh
# wanderbooks installer. One line on a new machine:
#
#   curl -fsSL https://wanderbooks.ai/install.sh | sh -s -- ~/Books/Acme
#
# Linux and macOS. On Windows, run it inside WSL2 (Ubuntu): the GUI opens as
# a normal window on Windows 11.
#
# Installs Rust if needed, then the wanderbooks CLI, MCP server, and GUI
# (into ~/.cargo/bin), then — given a directory — makes your books there and
# starts the agent. Without a directory it just installs.
#
# WANDERBOOKS_REPO overrides where it installs from (a git URL).
set -eu

REPO="${WANDERBOOKS_REPO:-https://git.swaits.com/swaits/wanderbooks}"

say() { printf '\033[1m%s\033[0m\n' "$*"; }
die() { printf '\033[1;31m%s\033[0m\n' "$*" >&2; exit 1; }

case "$(uname -s)" in
    Linux) os=linux ;;
    Darwin) os=macos ;;
    MINGW* | MSYS* | CYGWIN*)
        die "On Windows, install WSL2 (wsl --install), open Ubuntu, and run this there." ;;
    *) die "Unsupported system: $(uname -s). wanderbooks runs on Linux and macOS (Windows via WSL2)." ;;
esac

# A C compiler: SQLite and a few other pieces are compiled from source.
if ! command -v cc >/dev/null 2>&1 && ! command -v gcc >/dev/null 2>&1 && ! command -v clang >/dev/null 2>&1; then
    if [ "$os" = macos ]; then
        die "Needs Apple's command line tools. Run: xcode-select --install   (then run this again)"
    fi
    hint="your distribution's C compiler package"
    if command -v apt-get >/dev/null 2>&1; then hint="sudo apt-get install -y build-essential"
    elif command -v dnf >/dev/null 2>&1; then hint="sudo dnf install -y gcc"
    elif command -v pacman >/dev/null 2>&1; then hint="sudo pacman -S --needed base-devel"
    elif command -v zypper >/dev/null 2>&1; then hint="sudo zypper install -y gcc"
    elif command -v apk >/dev/null 2>&1; then hint="sudo apk add build-base"
    fi
    die "Needs a C compiler. Run: $hint   (then run this again)"
fi
command -v git >/dev/null 2>&1 || die "Needs git. Install it, then run this again."

if ! command -v cargo >/dev/null 2>&1; then
    if [ -x "$HOME/.cargo/bin/cargo" ]; then
        PATH="$HOME/.cargo/bin:$PATH"
    else
        say "Installing Rust (rustup)…"
        curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal
        # shellcheck disable=SC1091
        . "$HOME/.cargo/env"
    fi
fi

say "Installing wanderbooks from $REPO (a few minutes, once)…"
cargo install --locked --git "$REPO" ledger-cli ledger-mcp ledger-gui

if ! command -v just >/dev/null 2>&1; then
    say "Installing just (for the books dir's shortcuts)…"
    cargo install --locked just
fi

if ! command -v claude >/dev/null 2>&1; then
    say "Claude Code isn't installed yet: https://claude.com/claude-code"
fi

case ":$PATH:" in
    *":$HOME/.cargo/bin:"*) ;;
    *) say "Add ~/.cargo/bin to your PATH (rustup's installer usually does; open a new terminal)." ;;
esac

if [ $# -gt 0 ]; then
    # Hand the terminal to the agent when there is one.
    if [ -t 1 ] && [ -e /dev/tty ]; then
        exec "$HOME/.cargo/bin/wanderbooks" new "$@" </dev/tty
    fi
    exec "$HOME/.cargo/bin/wanderbooks" new "$@" --no-start
fi
say "Done. Make your books with: wanderbooks new ~/Books/YourCompany"
