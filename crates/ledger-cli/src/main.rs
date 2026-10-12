//! `wanderbooks` CLI — the bootstrap for AI-native bookkeeping.
//!
//! Usage: `wanderbooks init [DIR] [--force]`
//!
//! Scaffolds a new books directory: `mise.toml` (pinned toolchain + tasks),
//! `AGENT.md` (the agent operating contract), `wanderbooks.toml` (company
//! profile, filled in during onboarding), and an empty `books-data/` dir.
//! From there: `mise trust && mise install`, `mise run build`,
//! `mise run mcp-add`, then start `claude`/`codex` in the dir and let the
//! agent take the onboarding from AGENT.md.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Absolute path of the wanderbooks repo checkout this binary was built from.
fn repo_root() -> PathBuf {
    let raw = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    raw.canonicalize().unwrap_or(raw)
}

const MISE_TOML: &str = r#"# wanderbooks books dir — provisioned by `mise`.
# Bootstrap: `mise trust && mise install` then `mise run build && mise run mcp-add`.

[tools]
rust = "stable"

[tasks.build]
description = "Build ledger-mcp from the wanderbooks repo checkout"
dir = "{{REPO}}"
run = "cargo build --release -p ledger-mcp"

[tasks.mcp-add]
description = "Register wanderbooks with Claude Code (stdio transport; OS user boundary is the auth)"
run = 'claude mcp add wanderbooks -- "{{REPO}}/target/release/ledger-mcp" --data-dir "{{BOOKS}}/books-data"'

[tasks.backup]
description = "Commit the journal — the git history of this dir is the archive"
run = 'git add -A && (git commit -m "books backup $(date -u +%Y-%m-%dT%H:%M:%SZ)" || true)'
"#;

const AGENT_MD: &str = r#"# AGENT.md — wanderbooks books dir

This directory is a **wanderbooks** books dir: AI-native double-entry
bookkeeping. You (the AI agent) are the bookkeeper. The human talks to you;
you drive the books through the `wanderbooks` MCP server. There is no
bookkeeping UI; the numbers you show the human *are* the interface.

## Bootstrap (once, in order)

1. `mise trust && mise install` — installs the pinned Rust toolchain.
2. `mise run build` — builds `ledger-mcp` from the wanderbooks repo.
3. `mise run mcp-add` — registers the server with Claude Code over stdio.
   (Codex: add the same command + `--data-dir` to your MCP config instead.)
4. Restart the session so the MCP tools load, then continue below.

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
  over $25 requires a receipt — the server enforces this, don't argue with it.
- **Show the numbers.** After every mutation, show the affected balances.
  When the human questions a figure, trace it to journal entries.

## First run: onboarding

If `books-data/journal.jsonl` is missing or empty, onboard before recording
anything:

1. Ask: company legal name, entity type, owners and equity split, base
   currency, fiscal year start.
2. Set up the chart of accounts: propose a sensible default for the entity
   type and let the human adjust, or import one they hand you.
3. Record opening balances as one balanced opening entry. Verify the trial
   balance balances before moving on.
4. Write the answers into `wanderbooks.toml` under `[company]`.
5. If they're migrating from another system, import from wherever the books
   live now — QBO/CSV via `ledger-import`, bank or card CSVs, email receipts,
   a spreadsheet, whatever they point you at. Diff trial balance, P&L,
   balance sheet, and cash flow against the old system **to the cent**
   before calling the migration done.

## Ongoing

- Every transaction: draft, preview, confirm with the human, post, attach
  the receipt.
- `books.digest` is the review surface — "what did you do with my books
  lately." Offer it; don't make the human ask.
- Reports are pure functions of the journal. If a number looks wrong, the
  journal is the evidence — read it, don't guess.
- Back up: run `mise run backup` after every session that posted entries.
  The git history of this directory is the archive; the journal file is the
  system of record and the SQLite database is disposable.
"#;

const WANDERBOOKS_TOML: &str = r#"# wanderbooks books dir config. [company] is filled in during onboarding —
# the agent asks, the human answers, the agent writes it here.

[company]
name = ""
entity_type = ""
owners = ""
base_currency = "USD"
fiscal_year_start = "01-01"

[books]
data_dir = "books-data"
receipt_threshold = "25.00"
"#;

const GITIGNORE: &str = r#"# The SQLite projection is disposable — it rebuilds from journal.jsonl.
books-data/wanderbooks.db
books-data/wanderbooks.db-*
"#;

fn print_usage() {
    eprintln!("usage: wanderbooks init [DIR] [--force]");
    eprintln!("  DIR      books directory to create (default: current directory)");
    eprintln!("  --force  overwrite an existing wanderbooks.toml");
}

fn render(template: &str, repo: &str, books: &str) -> String {
    template
        .replace("{{REPO}}", repo)
        .replace("{{BOOKS}}", books)
}

/// `git init` the dir unless already inside a repo; commit the scaffold.
/// Failures are warnings, not errors — the scaffold is what matters.
fn git_bootstrap(dir: &Path) {
    let inside = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(dir)
        .output()
        .is_ok_and(|o| o.status.success());
    if !inside {
        let init_ok = Command::new("git")
            .arg("init")
            .current_dir(dir)
            .output()
            .is_ok_and(|o| o.status.success());
        if !init_ok {
            eprintln!("warning: git init failed; skipping scaffold commit");
            return;
        }
    }
    let add = Command::new("git")
        .args(["add", "-A"])
        .current_dir(dir)
        .output();
    if !add.is_ok_and(|o| o.status.success()) {
        eprintln!("warning: git add failed; skipping scaffold commit");
        return;
    }
    let _ = Command::new("git")
        .args([
            "-c",
            "user.name=wanderbooks",
            "-c",
            "user.email=wanderbooks@localhost",
            "commit",
            "-m",
            "init: wanderbooks books dir scaffold",
        ])
        .current_dir(dir)
        .output();
}

/// Scaffold a books directory. Returns the dir on success.
fn init(dir: &Path, force: bool) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let dir = dir
        .canonicalize()
        .map_err(|e| format!("cannot resolve {}: {e}", dir.display()))?;

    let marker = dir.join("wanderbooks.toml");
    if marker.exists() && !force {
        return Err(format!(
            "{} is already a books dir (wanderbooks.toml exists); use --force to overwrite",
            dir.display()
        ));
    }

    let repo = repo_root();
    let repo_s = repo.to_string_lossy();
    let books_s = dir.to_string_lossy();

    let files: &[(&str, String)] = &[
        ("mise.toml", render(MISE_TOML, &repo_s, &books_s)),
        ("AGENT.md", AGENT_MD.to_string()),
        ("wanderbooks.toml", WANDERBOOKS_TOML.to_string()),
        (".gitignore", GITIGNORE.to_string()),
    ];
    for (name, content) in files {
        fs::write(dir.join(name), content).map_err(|e| format!("cannot write {name}: {e}"))?;
    }
    fs::create_dir_all(dir.join("books-data"))
        .map_err(|e| format!("cannot create books-data: {e}"))?;
    fs::write(dir.join("books-data").join(".gitkeep"), "")
        .map_err(|e| format!("cannot write .gitkeep: {e}"))?;

    git_bootstrap(&dir);
    Ok(dir)
}

fn main() {
    let args = env::args().skip(1);
    let mut dir: Option<PathBuf> = None;
    let mut force = false;
    let mut cmd: Option<String> = None;

    for arg in args {
        match arg.as_str() {
            "--help" | "-h" => {
                print_usage();
                return;
            }
            "--force" => force = true,
            s if s.starts_with('-') => {
                eprintln!("unknown flag: {s}");
                print_usage();
                std::process::exit(2);
            }
            s => {
                if cmd.is_none() {
                    cmd = Some(s.to_string());
                } else if dir.is_none() {
                    dir = Some(PathBuf::from(s));
                } else {
                    eprintln!("unexpected argument: {s}");
                    print_usage();
                    std::process::exit(2);
                }
            }
        }
    }

    match cmd.as_deref() {
        Some("init") => {
            let dir = dir.unwrap_or_else(|| PathBuf::from("."));
            match init(&dir, force) {
                Ok(d) => {
                    println!("initialized books dir: {}", d.display());
                    println!("next:");
                    println!("  cd {}", d.display());
                    println!("  mise trust && mise install");
                    println!("  mise run build && mise run mcp-add");
                    println!("then start claude (or codex) here and follow AGENT.md");
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        }
        _ => {
            print_usage();
            std::process::exit(2);
        }
    }
}
