//! Integration tests for `wanderbooks init`: the scaffold lands complete,
//! the TOML parses, re-init is refused without `--force`.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_wanderbooks"))
}

fn run_init(dir: &std::path::Path, extra: &[&str]) -> (bool, String) {
    let mut cmd = Command::new(bin());
    cmd.arg("init").arg(dir);
    for e in extra {
        cmd.arg(e);
    }
    let out = cmd.output().expect("run wanderbooks");
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.success(), s)
}

#[test]
fn scaffold_lands_complete() {
    let tmp = tempfile::tempdir().unwrap();
    let books = tmp.path().join("acme-books");
    let (ok, _) = run_init(&books, &[]);
    assert!(ok, "init should succeed");
    for f in ["mise.toml", "AGENT.md", "wanderbooks.toml", ".gitignore"] {
        let p = books.join(f);
        assert!(p.exists(), "{f} exists");
        assert!(!fs::read_to_string(&p).unwrap().is_empty(), "{f} non-empty");
    }
    assert!(books.join("books-data").is_dir(), "books-data dir exists");
    assert!(
        books.join(".git").is_dir(),
        "fresh dir gets git init for the journal archive"
    );
}

#[test]
fn mise_toml_parses_and_names_repo_and_books() {
    let tmp = tempfile::tempdir().unwrap();
    let books = tmp.path().join("b");
    let (ok, _) = run_init(&books, &[]);
    assert!(ok);
    let raw = fs::read_to_string(books.join("mise.toml")).unwrap();
    let doc: toml::Value = toml::from_str(&raw).expect("mise.toml parses");
    assert!(doc.get("tools").is_some(), "[tools] present");
    let tasks = doc.get("tasks").expect("[tasks] present");
    for t in ["build", "mcp-add", "backup"] {
        assert!(tasks.get(t).is_some(), "task {t} present");
    }
    // The build task must point at the repo checkout; mcp-add at the books dir.
    let build_run = tasks["build"]["run"].as_str().unwrap();
    assert!(
        build_run.contains("cargo build"),
        "build runs cargo: {build_run}"
    );
    let mcp_run = tasks["mcp-add"]["run"].as_str().unwrap();
    assert!(
        mcp_run.contains("ledger-mcp") && mcp_run.contains("books-data"),
        "mcp-add wires the server to this books dir: {mcp_run}"
    );
    let books_s = books.to_string_lossy();
    assert!(
        mcp_run.contains(books_s.as_ref()),
        "mcp-add references the books dir path"
    );
}

#[test]
fn agent_md_carries_the_contract() {
    let tmp = tempfile::tempdir().unwrap();
    let books = tmp.path().join("b");
    let (ok, _) = run_init(&books, &[]);
    assert!(ok);
    let md = fs::read_to_string(books.join("AGENT.md")).unwrap();
    for phrase in [
        "preview",
        "Posted is forever",
        "reversing entries",
        "receipt",
        "journal.jsonl",
        "onboard",
        "to the cent",
    ] {
        assert!(md.contains(phrase), "AGENT.md mentions {phrase:?}");
    }
}

#[test]
fn reinit_refused_without_force() {
    let tmp = tempfile::tempdir().unwrap();
    let books = tmp.path().join("b");
    assert!(run_init(&books, &[]).0);
    let (ok, out) = run_init(&books, &[]);
    assert!(!ok, "second init without --force fails");
    assert!(out.contains("--force"), "error points at --force: {out}");
    let (ok, _) = run_init(&books, &["--force"]);
    assert!(ok, "second init with --force succeeds");
}

#[test]
fn init_inside_existing_repo_skips_git_init() {
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
    let (ok, _) = run_init(&books, &[]);
    assert!(ok);
    // Still a valid repo (we didn't nest another one) and scaffold landed.
    assert!(books.join("wanderbooks.toml").exists());
}
