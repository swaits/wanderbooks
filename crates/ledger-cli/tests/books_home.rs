//! `wanderbooks connect` makes the book's books home (P11b, D-27): a folder in
//! the account's Documents holding the agent's pointer and nothing of the
//! books. Run with a home of the test's own: nothing here touches the account's
//! real Documents or data root.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const KNOBS: &str =
    "close_after=2500,backoff=50,exe_poll=100,init_wait=5000,queue_wait=30000,backup_quiet=300";

struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("wb-books-home-")
            .tempdir()
            .unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_wanderbooks"))
            .args(["setup", "--home"])
            .arg(dir.path())
            .env_remove("XDG_DATA_HOME")
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        Self { dir }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join(".local/share/wanderbooks")
    }

    fn cmd(&self, mode: &str, path: &Path) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_wanderbooks"));
        c.arg(mode)
            .arg("--root")
            .arg(self.root())
            .args(["--test-knobs", KNOBS])
            .env_clear()
            .env("PATH", format!("{}:/usr/bin:/bin", path.display()))
            .env("HOME", self.dir.path())
            .stdin(Stdio::null());
        c
    }
}

/// A stand-in `claude` that accepts whatever it is told.
fn fake_agent(home: &Home) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = home.dir.path().join("agent-bin");
    std::fs::create_dir_all(&bin).unwrap();
    let script = bin.join("claude");
    std::fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

#[test]
fn connect_makes_the_books_home_and_it_holds_no_books_data() {
    let home = Home::new();
    let out = home
        .cmd("books", Path::new("/nonexistent"))
        .args(["create", "beta"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let docs = home.dir.path().join("Documents/Wanderbooks/beta");

    // An agent that isn't installed: said, and no folder made for nothing.
    let out = home
        .cmd("connect", Path::new("/nonexistent"))
        .args(["claude", "--book", "beta"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(!docs.exists(), "a home for an agent that isn't there");

    // Connected: the user-scope entry as ever, and the books home beside it.
    let bin = fake_agent(&home);
    let out = home
        .cmd("connect", &bin)
        .args(["claude", "--book", "beta"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        said.contains("Claude Code reaches the books beta"),
        "{said}"
    );
    assert!(
        said.contains(&format!("The books home is {}", docs.display())),
        "{said}"
    );
    let mut names: Vec<String> = Vec::new();
    for dir in [docs.clone(), docs.join(".claude")] {
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_file() {
                names.push(
                    p.strip_prefix(&docs)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    names.sort();
    assert_eq!(
        names,
        [
            ".claude/settings.json",
            ".mcp.json",
            "AGENTS.md",
            "CLAUDE.md"
        ]
    );
    // Nothing in it names the data root, a token or a key.
    let root = home.root();
    for n in &names {
        let text = std::fs::read_to_string(docs.join(n)).unwrap();
        assert!(!text.contains(root.to_str().unwrap()), "{n}: {text}");
        assert!(!text.contains("wb_"), "{n} holds a token: {text}");
        assert!(!text.contains("--root"), "{n}: {text}");
    }
    let mcp: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(docs.join(".mcp.json")).unwrap()).unwrap();
    assert_eq!(
        mcp["mcpServers"]["wanderbooks-beta"]["args"],
        serde_json::json!(["mcp", "--book", "beta"])
    );
    // Run again: still four files, the rule once.
    let out = home
        .cmd("connect", &bin)
        .args(["claude", "--book", "beta"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let settings: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(docs.join(".claude/settings.json")).unwrap())
            .unwrap();
    assert_eq!(
        settings["permissions"]["allow"],
        serde_json::json!(["mcp__wanderbooks-beta__*"])
    );
}
