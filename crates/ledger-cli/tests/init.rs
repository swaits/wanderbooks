//! Integration tests for `wanderbooks new`: one command lands a complete
//! books dir with the MCP server registered for it, the TOML and JSON
//! parse, and re-running is refused without `--force` (which never
//! touches the company profile).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_wanderbooks"))
}

/// Stand-ins for the installed binaries, so tests never run cargo install.
fn fake_bins(tmp: &Path) -> PathBuf {
    let dir = tmp.join("bin");
    fs::create_dir_all(&dir).unwrap();
    for b in ["ledger-mcp", "ledger-gui"] {
        fs::write(dir.join(b), "#!/bin/sh\n").unwrap();
    }
    dir
}

fn run(tmp: &Path, args: &[&str]) -> (bool, String) {
    let path = format!(
        "{}:{}",
        fake_bins(tmp).display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = Command::new(bin())
        .args(args)
        .arg("--no-start")
        .env("PATH", path)
        .output()
        .expect("run wanderbooks");
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.success(), s)
}

fn new(tmp: &Path, books: &Path, extra: &[&str]) -> (bool, String) {
    let mut args = vec!["new", books.to_str().unwrap()];
    args.extend(extra);
    run(tmp, &args)
}

#[test]
fn scaffold_lands_complete() {
    let tmp = tempfile::tempdir().unwrap();
    let books = tmp.path().join("acme-books");
    let (ok, out) = new(tmp.path(), &books, &[]);
    assert!(ok, "new should succeed: {out}");
    for f in [
        "justfile",
        "AGENTS.md",
        "CLAUDE.md",
        "wanderbooks.toml",
        ".gitignore",
        ".mcp.json",
        ".claude/settings.json",
    ] {
        let p = books.join(f);
        assert!(p.exists(), "{f} exists");
        assert!(!fs::read_to_string(&p).unwrap().is_empty(), "{f} non-empty");
    }
    assert!(books.join("books-data").is_dir(), "books-data dir exists");
    assert!(books.join(".git").is_dir(), "fresh dir gets git init");
    let log = Command::new("git")
        .args(["log", "--oneline"])
        .current_dir(&books)
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&log.stdout).contains("init"),
        "scaffold committed"
    );
}

#[test]
fn the_server_is_registered_for_this_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let books = tmp.path().join("b");
    assert!(new(tmp.path(), &books, &[]).0);
    let books = books.canonicalize().unwrap();
    let mcp: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(books.join(".mcp.json")).unwrap()).unwrap();
    let server = &mcp["mcpServers"]["wanderbooks"];
    assert!(server["command"].as_str().unwrap().ends_with("ledger-mcp"));
    assert_eq!(
        server["args"],
        serde_json::json!(["--data-dir", books.join("books-data").to_string_lossy()])
    );
    let settings: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(books.join(".claude/settings.json")).unwrap())
            .unwrap();
    assert_eq!(
        settings["enabledMcpjsonServers"],
        serde_json::json!(["wanderbooks"])
    );
    // The justfile opens this dir's books in the GUI.
    let just = fs::read_to_string(books.join("justfile")).unwrap();
    assert!(just.contains("ledger-gui\" --data-dir books-data"));
    for recipe in ["agent:", "gui:", "backup:", "update:"] {
        assert!(just.contains(recipe), "justfile has {recipe}");
    }
    assert_eq!(
        fs::read_to_string(books.join("CLAUDE.md")).unwrap().trim(),
        "@AGENTS.md"
    );
    let toml: toml::Value =
        toml::from_str(&fs::read_to_string(books.join("wanderbooks.toml")).unwrap()).unwrap();
    assert_eq!(toml["books"]["data_dir"].as_str(), Some("books-data"));
}

#[test]
fn agents_md_carries_the_contract() {
    let tmp = tempfile::tempdir().unwrap();
    let books = tmp.path().join("b");
    assert!(new(tmp.path(), &books, &[]).0);
    let md = fs::read_to_string(books.join("AGENTS.md")).unwrap();
    for phrase in [
        "preview",
        "Posted is forever",
        "reversing entries",
        "receipt",
        "journal.jsonl",
        "onboard",
        "chart_template",
        "to the cent",
        "just gui",
    ] {
        assert!(md.contains(phrase), "AGENTS.md mentions {phrase:?}");
    }
}

#[test]
fn rerun_refused_without_force_and_never_clobbers_the_profile() {
    let tmp = tempfile::tempdir().unwrap();
    let books = tmp.path().join("b");
    assert!(new(tmp.path(), &books, &[]).0);
    let (ok, out) = new(tmp.path(), &books, &[]);
    assert!(!ok, "second new without --force fails");
    assert!(out.contains("--force"), "error points at --force: {out}");
    fs::write(
        books.join("wanderbooks.toml"),
        "[company]\nname = \"Acme LLC\"\n",
    )
    .unwrap();
    let (ok, _) = new(tmp.path(), &books, &["--force"]);
    assert!(ok, "second new with --force succeeds");
    assert!(
        fs::read_to_string(books.join("wanderbooks.toml"))
            .unwrap()
            .contains("Acme LLC"),
        "--force rewrites the scaffold, never the company profile"
    );
}

#[test]
fn init_is_an_alias_and_respects_an_existing_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let books = tmp.path().join("b");
    fs::create_dir_all(&books).unwrap();
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(&books)
            .status()
            .unwrap()
            .success()
    );
    let (ok, out) = run(tmp.path(), &["init", books.to_str().unwrap()]);
    assert!(ok, "{out}");
    assert!(books.join("wanderbooks.toml").exists());
}

#[test]
fn codex_gets_its_own_named_server() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = fake_bins(tmp.path());
    let log = tmp.path().join("codex-args");
    // A stand-in codex that records how it was called.
    fs::write(
        bin.join("codex"),
        format!("#!/bin/sh\necho \"$@\" > {}\n", log.display()),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(bin.join("codex"), fs::Permissions::from_mode(0o755)).unwrap();
    let books = tmp.path().join("Acme Books");
    let (ok, out) = new(tmp.path(), &books, &["--codex"]);
    assert!(ok, "{out}");
    let args = fs::read_to_string(&log).unwrap();
    let books = books.canonicalize().unwrap();
    assert!(
        args.starts_with("mcp add wanderbooks-acme-books -- "),
        "{args}"
    );
    assert!(
        args.trim_end().ends_with(&format!(
            "--data-dir {}",
            books.join("books-data").display()
        )),
        "{args}"
    );
    let just = fs::read_to_string(books.join("justfile")).unwrap();
    assert!(just.contains("codex mcp add \"wanderbooks-acme-books\""));
    assert!(out.contains("just codex"), "{out}");
}
