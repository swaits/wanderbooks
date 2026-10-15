//! `wanderbooks` CLI: one command from nothing to talking with your
//! bookkeeper.
//!
//! ```text
//! wanderbooks new [DIR] [--codex] [--force] [--no-start]   (alias: init)
//! wanderbooks gui [DIR]
//! ```
//!
//! `new` makes a complete books directory and starts the agent in it:
//!
//! 1. Finds `ledger-mcp` and `ledger-gui` next to this binary or on the
//!    PATH, and installs them with cargo if they're missing.
//! 2. Writes `AGENTS.md` (the bookkeeper's operating contract, with a
//!    `CLAUDE.md` that imports it), `wanderbooks.toml`, a `justfile`,
//!    `.gitignore`, and `books-data/`.
//! 3. Registers the MCP server for this directory only: `.mcp.json`, plus
//!    `.claude/settings.json` approving it, so Claude Code loads the tools
//!    without `claude mcp add` or a prompt. Claude Code
//!    (`.claude/settings.json`) and Codex (`.codex/hooks.json`) both get a
//!    SessionStart hook that runs `just session-start`: the viewer check.
//! 4. `git init`s the directory (its history is the archive) and commits.
//! 5. Starts `claude` in the directory with the onboarding prompt, unless
//!    `--no-start` or there's no terminal. With `--codex`, registers the
//!    server with Codex (as `wanderbooks-<dir name>`) and starts Codex.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where `wanderbooks update` installs from.
const REPO_URL: &str = "https://git.swaits.com/swaits/wanderbooks";

/// The checkout this binary was built from, if it still exists.
fn repo_root() -> Option<PathBuf> {
    let raw = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = raw.canonicalize().ok()?;
    root.join("crates/ledger-mcp/Cargo.toml")
        .exists()
        .then_some(root)
}

const JUSTFILE: &str = r#"# wanderbooks books dir. Run `just` to list recipes.
# Day to day you talk to the agent (`just agent`); the recipes are the
# handful of things you might do yourself.

set shell := ["bash", "-cu"]

# List recipes
default:
    @just --list

# Start a bookkeeping session with the agent (Claude Code)
agent:
    claude

# Start a bookkeeping session in Codex (run `just codex-setup` once first)
codex:
    codex

# Register these books' MCP server with Codex (once per machine)
codex-setup:
    codex mcp add "@@CODEX_NAME@@" -- "@@LEDGER_MCP@@" --data-dir "{{justfile_directory()}}/books-data"

# Open the live, read-only view of these books (in the background; once per books dir)
gui:
    #!/usr/bin/env bash
    set -u
    if just gui-status &>/dev/null; then echo "The viewer is already open."; exit 0; fi
    log="{{justfile_directory()}}/books-data/gui.log"
    nohup "@@LEDGER_GUI@@" --data-dir "{{justfile_directory()}}/books-data" \
        --books-toml "{{justfile_directory()}}/wanderbooks.toml" >"$log" 2>&1 </dev/null &
    sleep 1
    if kill -0 $! 2>/dev/null; then echo "Viewer open (pid $!)."; else echo "The viewer didn't start:"; cat "$log"; exit 1; fi

# Open the viewer in the foreground (logs in the terminal; Ctrl-C closes it)
gui-fg:
    "@@LEDGER_GUI@@" --data-dir "{{justfile_directory()}}/books-data" --books-toml "{{justfile_directory()}}/wanderbooks.toml"

# Is the viewer open for these books? Prints open/closed; exits 0 only if open
gui-status:
    @pgrep -f -- "ledger-gui --data-dir {{justfile_directory()}}/books-data( |$)" >/dev/null && echo open || { echo closed; exit 1; }

# The agents' session-start hook: the viewer's state, handled per [viewer] open_at_start
session-start:
    #!/usr/bin/env bash
    set -u
    just gui-status &>/dev/null && { echo "wanderbooks viewer: open."; exit 0; }
    pref=$(sed -n 's/^open_at_start *= *"\([a-z]*\)".*/\1/p' "{{justfile_directory()}}/wanderbooks.toml")
    case "${pref:-ask}" in
        never) ;;
        always) echo "wanderbooks viewer: closed; open_at_start = \"always\", so it was opened: $(just gui 2>&1 | tail -1)" ;;
        *) echo "wanderbooks viewer: CLOSED. [viewer] open_at_start = \"ask\": at the top of your first reply, before anything else, offer to open it (\`just gui\`)." ;;
    esac

# Commit the journal: the git history of this dir is the archive
backup:
    git add -A && (git commit -qm "books backup $(date -u +%Y-%m-%dT%H:%M:%SZ)" && echo committed || echo "nothing to commit")

# Update wanderbooks (server, GUI, CLI) to the latest version
update:
    cargo install --locked --git @@REPO_URL@@ ledger-cli ledger-mcp ledger-gui
"#;

const AGENTS_MD: &str = r#"# AGENTS.md — wanderbooks books dir

This directory is a **wanderbooks** books dir: AI-native double-entry
bookkeeping. You (the AI agent) are the bookkeeper. The human talks to you;
you drive the books through the `wanderbooks` MCP server (registered for
this directory in `.mcp.json`). The humans read the books in the GUI
(`just gui`): read-only and live, so whatever you post appears there within
a second. Everything else they see, you show them.

## The MCP contract

- **Amounts are decimal strings** (`"123.45"`). Never floats, never
  locale-formatted. Money is exact or it is wrong.
- **Draft → add lines → preview → confirmed post.** `post_entry` without
  confirmation returns a preview and posts nothing: show the preview to the
  human before confirming. Same pattern for `reverse_entry`.
- **Every entry balances.** Debits equal credits exactly. No epsilon.
- **Posted is forever.** Corrections are reversing entries linked to the
  original, never edits, never deletes.
- **Receipts:** attach with `attach_receipt` and link to the entry. Anything
  over the receipt threshold ($25 unless `wanderbooks.toml` says otherwise)
  requires a receipt — the server enforces this, don't argue with it. One
  file per receipt: PDF preferred, an image is fine.
- **Show the numbers.** After every mutation, show the affected balances.
  When the human questions a figure, trace it to journal entries.

## First run: onboarding

If `books-data/journal.jsonl` is missing or empty, onboard before recording
anything. Keep it conversational; ask a few things at a time.

1. Ask: company legal name, entity type (sole proprietorship or
   single-member LLC, partnership or multi-member LLC, S corp, C corp),
   owners and their shares, fiscal year start, and whether they want
   accrual or cash basis.
2. Propose a chart of accounts with `books.chart_template` for that entity
   type and those owners. Walk the human through it, adjust, then import it
   with `books.import_chart_csv`. Put the template's settings (members,
   tax form, close target) in `wanderbooks.toml`.
3. Fill in `wanderbooks.toml` `[company]`: name, entity type, tax
   classification, formation state and date, EIN, address. Don't store
   SSNs or full account numbers anywhere in this directory. The server
   reloads the file on every change: check `books.settings` after each
   edit (no `reload_error`, no `problems`).
   If the imported chart numbers receivables, payables, revenue, payroll,
   or retained earnings differently from the defaults (`books.roles`),
   assign those roles to its accounts with `books.assign_role`.
4. Record opening balances as one balanced opening entry (or the owners'
   first contributions). Verify the trial balance before moving on.
5. File the formation documents they have (articles, EIN letter,
   operating agreement) with `books.file_document`.
6. If they're migrating from another system, import from wherever the books
   live now — QBO/CSV, bank or card CSVs, email receipts, a spreadsheet,
   whatever they point you at. Diff trial balance, P&L, balance sheet, and
   cash flow against the old system **to the cent** before calling the
   migration done.
7. Tell them about `just gui`, then commit with `just backup`.

## Receipts from email

If the human has connected their email (a Gmail or Outlook connector, or a
browser extension with their mail open), find receipts there yourself:
search for the vendor and amount around the transaction date, then file
one file per receipt — the PDF attachment if there is one; otherwise save
the message and render it to PDF. Never file a raw `.eml`. Ask before
downloading anything, and never type a password or account number into a
form.

## Switching from QuickBooks, Xero, or a spreadsheet

1. Ask the human to export, from the old system, for every year they want
   to bring over: the chart of accounts, the general journal (or general
   ledger), and the trial balance, profit and loss, and balance sheet. CSV
   or XLSX. Receipts usually live as attachments; export those too.
2. Set up the chart (`books.import_chart_csv` or account by account), then
   rebuild the history as entries — or, if they only want a fresh start, one
   opening entry from the old trial balance at the cutover date.
3. Diff your trial balance, P&L, balance sheet, and cash flow against the
   old system's reports for the same dates, **to the cent**, and show the
   human the comparison. Not done until every number matches.
4. Keep the old system read-only until a full month has been kept in
   wanderbooks and closed cleanly. Then they can cancel it.

Bank feeds aren't automatic here: import the bank's CSV export
(`books.import_bank_csv`, drafts to review) or read their statements.

## Working with the CPA

- At year end (or whenever the CPA asks), build the package with
  `books.export_cpa_package`: every statement as PDF and XLSX, the tax
  summary, the general ledger and journal, a QuickBooks Desktop IIF
  journal, the 1099 worksheet, and every receipt with an index. Save it
  where the human can send it.
- Before that: every expense and revenue account has a tax category
  (`books.tax_categories`), the period's entries all have receipts, and
  `books.verify` is clean.
- A CPA who wants to look around can get a read-only copy of this
  directory (it's a git repository: push it to a private remote and give
  them read access) and open it with `wanderbooks gui`, or `ledger-gui
  --data-dir books-data`. Nothing they do there can change the books.

## Session start

A SessionStart hook runs `just session-start` and hands you the result;
if it didn't, run `just gui-status` yourself. If the viewer isn't open, follow `[viewer]
open_at_start` in `wanderbooks.toml`: `"ask"` (the default) offers to open
it, `"always"` opens it with `just gui`, `"never"` leaves it. `just gui`
starts it in the background and does nothing if it's already open. If the
human tells you their preference, write it there.

## Ongoing

- Every transaction: draft, preview, confirm with the human, post, attach
  the receipt. Tag payments to contractors with their vendor (1099s).
- The human usually has the viewer open beside you (`just gui`). When they
  ask to see something, and after you post, put it on their screen with
  `books.show` (an entry, an account register, a report, a document, or a
  search).
- `books.digest` is the review surface — "what did you do with my books
  lately." Offer it; don't make the human ask.
- Reports are pure functions of the journal. If a number looks wrong, the
  journal is the evidence — read it, don't guess.
- Integrity: `books.verify` checks the journal's hash chain, the database
  against the journal (rebuilding it if it drifted), and every stored
  file. Run it when a number looks wrong or the human asks.
- Back up: `just backup` after every session that posted entries. The git
  history of this directory is the archive; `books-data/journal.jsonl` is
  the system of record and the SQLite database is disposable.
"#;

const CLAUDE_MD: &str = "@AGENTS.md\n";

const WANDERBOOKS_TOML: &str = r#"# wanderbooks books dir config. [company] is filled in during onboarding —
# the agent asks, the human answers, the agent writes it here.
#
# Every setting is optional. A commented-out line is an example to fill in
# (uncomment it); a live line is the default. The MCP server and the
# viewer reload this file when it changes: no restart needed.

[company]
# Legal name, for report headers.
# name = "Acme Widgets LLC"
# Shown in the profile. The journal's currency is fixed when the books are
# created (books-data/config.json base_currency, default USD).
base_currency = "USD"
# "MM-01": a fiscal year starts on the first of a month and ends the day
# before the next start (it ends on the last day of a month: IRS Pub. 538).
fiscal_year_start = "01-01"
# The return the business files: "schedule_c", "1065", "1120s", or "1120".
# tax_form = "1065"
# Or the federal classification, and the return follows from it:
# "partnership", "S corporation", "C corporation", or "disregarded entity"
# (Treas. Reg. §301.7701-3). tax_form wins when both are set.
# tax_classification = "partnership"
# Identity facts the CPA needs, each from a primary document (EIN letter,
# certificate of organization). Any simple key = value line here shows in
# the viewer's profile, labelled from its key.
# entity_type = "LLC, multi-member, member-managed"
# formation_state = "UT"
# formed = "2026-01-15"
# ein = "00-0000000"
# address = "123 Main St, Salt Lake City, UT 84101"
# Never store owners' SSNs (or any TIN) in this directory: its git history
# keeps everything. The CPA collects them through their own channel.

# Members of a multi-member LLC, one block each: shares come from the
# operating agreement (IRC §704(a)), "50%" or "0.5", summing to 100%. Each
# names a capital account in the chart, and optionally a distributions
# account that a close zeroes into capital.
# [[company.members]]
# name = "Alex Rivera"
# share = "50%"
# capital_account = "3000"
# distribution_account = "3100"
#
# [[company.members]]
# name = "Sam Chen"
# share = "50%"
# capital_account = "3001"
# distribution_account = "3101"

[books]
# The books' data, relative to this file. The tools take it from --data-dir
# (the justfile, .mcp.json), not from here: move them together.
data_dir = "books-data"
# "accrual" or "cash", for report footers. Use the same method from year
# to year (IRS Pub. 538).
accounting_basis = "accrual"
# An entry whose debits are strictly over this needs a receipt; "0.00"
# means every entry (IRS Pub. 583: keep the documents that support your
# books). A receipt_policy in books-data/config.json overrides it.
receipt_threshold = "25.00"
# Account codes that hold cash (bank, savings, cash on hand), for the cash
# flow statement and dashboards. Unset: cash accounts are found by name.
# cash_accounts = ["1000", "1010"]
# Liabilities the cash flow statement treats as financing: loans, and what
# owners advanced by paying company expenses (ASC 230-10-45-14(b)).
# financing_accounts = ["2100"]
# Assets it treats as investing: loans the company makes, investments it
# buys (ASC 230-10-20). Registered fixed assets need not be listed.
# investing_accounts = ["1400"]
# Single owner only (with [[company.members]] a close goes to each
# member's capital): the equity account a period close puts net income in.
# Unset: Retained Earnings (3999).
# close_into = "3000"
# Equity accounts a close zeroes into close_into (or Retained Earnings):
# draws, distributions, dividends.
# close_sweep = ["3100"]

# The prefix of generated asset tags (the labels on equipment: FL-0001,
# FL-0002, ...). Unset: the company name's initials, without LLC/Inc
# ("Fernhill Labs LLC" -> FL); 1 to 8 letters or digits.
# asset_tag_prefix = "FL"

# Which accounts invoices, bills, payroll, and closes post to (receivables,
# payables, revenue, payroll, retained earnings) is not set here: those
# roles are part of the books, journaled with books.assign_role and listed
# by books.roles. Until assigned, each uses its default code (1200, 2000,
# 4000, 6999, 3999, 6100, 6110, 2110-2150).

[vault]
# The TIN vault (full taxpayer identification numbers and W-9s, encrypted,
# in vault/ beside the books, which git ignores) is opened with this key
# file, kept OUTSIDE the books. Keep a copy in your password manager:
# without it the vault can't be read. Unset: ~/.config/wanderbooks/vault.key
# key_file = "~/.config/wanderbooks/vault.key"

[viewer]
# When a session starts and the viewer isn't open (`just gui-status`), the
# agent: "ask" offers to open it, "always" opens it (`just gui`), "never"
# leaves it closed.
open_at_start = "ask"
"#;

const GITIGNORE: &str = r#"# The SQLite projection is disposable — it rebuilds from journal.jsonl.
books-data/wanderbooks.db
books-data/wanderbooks.db-*
# The TIN vault: encrypted, but never in git (it must be purgeable).
vault/
# Copies of a database that disagreed with the journal, kept when it is
# rebuilt; disposable like the database itself.
books-data/wanderbooks.mismatch.*
# Held while an agent's MCP server has the books open: one writer at a time.
books-data/wanderbooks.lock
# Generated exports can always be regenerated from the journal.
exports/
# The background viewer's log (`just gui`).
books-data/gui.log
"#;

const START_PROMPT: &str = "Hi! These are brand-new books. Please set them up with me.";

fn print_usage() {
    eprintln!("usage: wanderbooks new [DIR] [--codex] [--force] [--no-start]");
    eprintln!("       wanderbooks gui [DIR]");
    eprintln!();
    eprintln!("  new        make a books directory and start the agent in it (alias: init)");
    eprintln!("  gui        open the live, read-only view of a books directory");
    eprintln!("  DIR        the books directory (default: current directory)");
    eprintln!("  --force    rewrite the scaffold in an existing books directory");
    eprintln!("  --no-start don't start the agent afterwards");
    eprintln!("  --codex    keep the books with Codex instead of Claude Code");
}

/// A binary next to this one (cargo installs them side by side), else on
/// the PATH.
fn find_binary(name: &str) -> Option<PathBuf> {
    let name = format!("{name}{}", env::consts::EXE_SUFFIX);
    let name = name.as_str();
    if let Some(dir) = env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
    {
        let p = dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    env::var_os("PATH").and_then(|paths| {
        env::split_paths(&paths)
            .map(|d| d.join(name))
            .find(|p| p.is_file())
    })
}

/// Install the server and GUI with cargo: from this checkout when there is
/// one, else from the repo.
fn install_binaries() -> Result<(), String> {
    let mut cmd = Command::new("cargo");
    cmd.args(["install", "--locked"]);
    match repo_root() {
        Some(root) => {
            eprintln!(
                "Installing ledger-mcp and ledger-gui from {} (a few minutes, once)…",
                root.display()
            );
            // `--path` takes one crate at a time.
            for c in ["ledger-mcp", "ledger-gui"] {
                let ok = Command::new("cargo")
                    .args(["install", "--locked", "--path"])
                    .arg(root.join("crates").join(c))
                    .status()
                    .map_err(|e| {
                        format!("can't run cargo: {e} (install Rust from https://rustup.rs)")
                    })?
                    .success();
                if !ok {
                    return Err(format!("cargo install of {c} failed"));
                }
            }
            return Ok(());
        }
        None => {
            eprintln!(
                "Installing ledger-mcp and ledger-gui from {REPO_URL} (a few minutes, once)…"
            );
            cmd.args(["--git", REPO_URL, "ledger-mcp", "ledger-gui"]);
        }
    }
    let ok = cmd
        .status()
        .map_err(|e| format!("can't run cargo: {e} (install Rust from https://rustup.rs)"))?
        .success();
    if ok {
        Ok(())
    } else {
        Err("cargo install failed".into())
    }
}

fn binaries() -> Result<(PathBuf, PathBuf), String> {
    if let (Some(m), Some(g)) = (find_binary("ledger-mcp"), find_binary("ledger-gui")) {
        return Ok((m, g));
    }
    install_binaries()?;
    match (find_binary("ledger-mcp"), find_binary("ledger-gui")) {
        (Some(m), Some(g)) => Ok((m, g)),
        _ => Err(
            "installed, but ledger-mcp/ledger-gui aren't on the PATH; add ~/.cargo/bin to it"
                .into(),
        ),
    }
}

fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `.mcp.json`: the wanderbooks server for this directory only.
fn mcp_json(ledger_mcp: &Path, books: &Path) -> String {
    format!(
        "{{\n  \"mcpServers\": {{\n    \"wanderbooks\": {{\n      \"command\": {},\n      \"args\": [\"--data-dir\", {}]\n    }}\n  }}\n}}\n",
        json_string(&ledger_mcp.to_string_lossy()),
        json_string(&books.join("books-data").to_string_lossy())
    )
}

/// `.claude/settings.json`: approve this directory's server, and its tools,
/// so the first session starts working instead of asking. The server still
/// previews every posting for the human to confirm. A SessionStart hook
/// runs `just session-start`, so the viewer check doesn't hang on the agent
/// remembering AGENTS.md.
const CLAUDE_SETTINGS: &str = r#"{
  "enabledMcpjsonServers": ["wanderbooks"],
  "permissions": {
    "allow": ["mcp__wanderbooks"]
  },
  "hooks": {
    "SessionStart": [
      {
        "matcher": "startup|resume|clear",
        "hooks": [
          { "type": "command", "command": "cd \"$CLAUDE_PROJECT_DIR\" && just session-start" }
        ]
      }
    ]
  }
}
"#;

/// `.codex/hooks.json`: the same session-start check for Codex. Codex runs a
/// project hook only once the project is trusted and the hook approved
/// (`/hooks`); until then AGENTS.md's "Session start" is the fallback.
const CODEX_HOOKS: &str = r#"{
  "hooks": {
    "SessionStart": [
      {
        "matcher": "startup|resume|clear",
        "hooks": [
          { "type": "command", "command": "cd \"$(git rev-parse --show-toplevel)\" && just session-start", "timeout": 10 }
        ]
      }
    ]
  }
}
"#;

/// `git init` the dir unless already inside a repo; commit the scaffold.
/// Failures are warnings, not errors — the scaffold is what matters.
fn git_bootstrap(dir: &Path) {
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .is_ok_and(|o| o.status.success())
    };
    if !git(&["rev-parse", "--is-inside-work-tree"]) && !git(&["init", "-q"]) {
        eprintln!("warning: git init failed; skipping the first commit");
        return;
    }
    if !git(&["add", "-A"]) {
        eprintln!("warning: git add failed; skipping the first commit");
        return;
    }
    // Use the person's identity when they have one; fall back so a fresh
    // machine still gets a first commit.
    let has_identity = git(&["config", "user.email"]);
    let mut args = Vec::new();
    if !has_identity {
        args.extend([
            "-c",
            "user.name=wanderbooks",
            "-c",
            "user.email=wanderbooks@localhost",
        ]);
    }
    args.extend(["commit", "-qm", "init: wanderbooks books dir"]);
    git(&args);
}

/// Write the books directory. Returns it, canonical.
fn scaffold(
    dir: &Path,
    force: bool,
    ledger_mcp: &Path,
    ledger_gui: &Path,
) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let dir = dir
        .canonicalize()
        .map_err(|e| format!("cannot resolve {}: {e}", dir.display()))?;
    if dir.join("wanderbooks.toml").exists() && !force {
        return Err(format!(
            "{} is already a books dir (wanderbooks.toml exists); use --force to rewrite the scaffold",
            dir.display()
        ));
    }
    let justfile = JUSTFILE
        .replace("@@LEDGER_GUI@@", &ledger_gui.to_string_lossy())
        .replace("@@LEDGER_MCP@@", &ledger_mcp.to_string_lossy())
        .replace("@@CODEX_NAME@@", &codex_name(&dir))
        .replace("@@REPO_URL@@", REPO_URL);
    let mut files: Vec<(&str, String)> = vec![
        ("justfile", justfile),
        ("AGENTS.md", AGENTS_MD.to_string()),
        ("CLAUDE.md", CLAUDE_MD.to_string()),
        (".gitignore", GITIGNORE.to_string()),
        (".mcp.json", mcp_json(ledger_mcp, &dir)),
    ];
    // The company profile is the person's data: never overwrite it.
    if !dir.join("wanderbooks.toml").exists() {
        files.push(("wanderbooks.toml", WANDERBOOKS_TOML.to_string()));
    }
    for (name, content) in files {
        fs::write(dir.join(name), content).map_err(|e| format!("cannot write {name}: {e}"))?;
    }
    fs::create_dir_all(dir.join(".claude")).map_err(|e| format!("cannot create .claude: {e}"))?;
    fs::write(dir.join(".claude/settings.json"), CLAUDE_SETTINGS)
        .map_err(|e| format!("cannot write .claude/settings.json: {e}"))?;
    fs::create_dir_all(dir.join(".codex")).map_err(|e| format!("cannot create .codex: {e}"))?;
    fs::write(dir.join(".codex/hooks.json"), CODEX_HOOKS)
        .map_err(|e| format!("cannot write .codex/hooks.json: {e}"))?;
    fs::create_dir_all(dir.join("books-data"))
        .map_err(|e| format!("cannot create books-data: {e}"))?;
    fs::write(dir.join("books-data/.gitkeep"), "")
        .map_err(|e| format!("cannot write .gitkeep: {e}"))?;
    git_bootstrap(&dir);
    Ok(dir)
}

fn is_terminal() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && std::io::stdout().is_terminal()
}

/// Which agent keeps the books.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Agent {
    Claude,
    Codex,
}

impl Agent {
    fn binary(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
        }
    }
}

/// Codex keeps MCP servers in one global config, so each books dir gets
/// its own server name: `wanderbooks-<dir name>`.
fn codex_name(dir: &Path) -> String {
    let base: String = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let base = base.trim_matches('-');
    if base.is_empty() {
        "wanderbooks".to_string()
    } else {
        format!("wanderbooks-{base}")
    }
}

/// Register this books dir's MCP server with Codex.
fn codex_register(dir: &Path, ledger_mcp: &Path) -> Result<(), String> {
    let Some(codex) = find_binary("codex") else {
        return Err("Codex isn't installed: https://developers.openai.com/codex".into());
    };
    let name = codex_name(dir);
    let ok = Command::new(codex)
        .args(["mcp", "add", &name, "--"])
        .arg(ledger_mcp)
        .arg("--data-dir")
        .arg(dir.join("books-data"))
        .status()
        .map_err(|e| format!("can't run codex: {e}"))?
        .success();
    if ok {
        println!("Registered the books with Codex as MCP server {name:?}.");
        Ok(())
    } else {
        Err(format!("codex mcp add {name} failed"))
    }
}

/// Replace this process with the agent in the books dir, onboarding.
fn start_agent(dir: &Path, agent: Agent) -> Result<(), String> {
    let Some(bin) = find_binary(agent.binary()) else {
        println!(
            "{} isn't installed. Install it, then run `{}` in {}.",
            if agent == Agent::Claude {
                "Claude Code"
            } else {
                "Codex"
            },
            agent.binary(),
            dir.display()
        );
        return Ok(());
    };
    println!("Starting your bookkeeper…");
    let mut cmd = Command::new(bin);
    cmd.arg(START_PROMPT).current_dir(dir);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = cmd.exec();
        Err(format!("can't start {}: {err}", agent.binary()))
    }
    #[cfg(not(unix))]
    {
        cmd.status()
            .map_err(|e| format!("can't start {}: {e}", agent.binary()))?;
        Ok(())
    }
}

fn new(dir: &Path, force: bool, start: bool, agent: Agent) -> Result<(), String> {
    let (ledger_mcp, ledger_gui) = binaries()?;
    let dir = scaffold(dir, force, &ledger_mcp, &ledger_gui)?;
    if agent == Agent::Codex {
        codex_register(&dir, &ledger_mcp)?;
    }
    println!("Books ready in {}", dir.display());
    println!("  just gui     the live, read-only view of the books");
    println!(
        "  just {}  a session with your bookkeeper",
        if agent == Agent::Codex {
            "codex"
        } else {
            "agent"
        }
    );
    println!("  just backup  commit the journal");
    if agent == Agent::Codex {
        println!("In Codex, trust this directory and approve its session-start hook (/hooks).");
    }
    if start && is_terminal() {
        start_agent(&dir, agent)?;
    } else {
        println!("Next: cd {} && {}", dir.display(), agent.binary());
    }
    Ok(())
}

fn gui(dir: &Path) -> Result<(), String> {
    let gui = find_binary("ledger-gui")
        .ok_or("ledger-gui isn't installed; run `wanderbooks new` or `just update`")?;
    let status = Command::new(gui)
        .arg("--data-dir")
        .arg(dir.join("books-data"))
        .arg("--books-toml")
        .arg(dir.join("wanderbooks.toml"))
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("ledger-gui exited with an error".into())
    }
}

fn main() {
    let mut dir: Option<PathBuf> = None;
    let mut force = false;
    let mut start = true;
    let mut agent = Agent::Claude;
    let mut cmd: Option<String> = None;
    for arg in env::args().skip(1) {
        match arg.as_str() {
            "--help" | "-h" => {
                print_usage();
                return;
            }
            "--force" => force = true,
            "--no-start" => start = false,
            "--codex" => agent = Agent::Codex,
            s if s.starts_with('-') => {
                eprintln!("unknown flag: {s}");
                print_usage();
                std::process::exit(2);
            }
            s if cmd.is_none() => cmd = Some(s.to_string()),
            s if dir.is_none() => dir = Some(PathBuf::from(s)),
            s => {
                eprintln!("unexpected argument: {s}");
                print_usage();
                std::process::exit(2);
            }
        }
    }
    let dir = dir.unwrap_or_else(|| PathBuf::from("."));
    let result = match cmd.as_deref() {
        Some("new") | Some("init") => new(&dir, force, start, agent),
        Some("gui") => gui(&dir),
        _ => {
            print_usage();
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
