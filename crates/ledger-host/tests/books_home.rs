//! Books homes (P11b, D-27): the folder an agent works in for a book. It
//! holds the agent's pointer and settings and nothing of the books.

use ledger_host::commands::{
    AgentStart, books_home_in, find_agent_in, installer, make_books_home_in, start_agent_in,
};
use std::path::{Path, PathBuf};

/// Every file under `dir`, relative, sorted.
fn names(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                out.push(p.strip_prefix(base).unwrap().to_string_lossy().into_owned());
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

#[test]
fn books_home_holds_no_books_data() {
    let docs = tempfile::tempdir().unwrap();
    // A data root with a journal, a key and a token file, elsewhere.
    let root = tempfile::tempdir().unwrap();
    let book = root.path().join("books").join("acme");
    std::fs::create_dir_all(&book).unwrap();
    std::fs::write(book.join("journal.jsonl"), "{}").unwrap();
    let exe = PathBuf::from("/usr/bin/wanderbooks");

    let home = make_books_home_in(docs.path(), "acme", Some("Acme Inc."), &exe).unwrap();
    assert_eq!(home, books_home_in(docs.path(), "acme"));
    assert_eq!(home, docs.path().join("Wanderbooks").join("acme"));
    // Exactly these four files.
    assert_eq!(
        names(&home),
        [
            ".claude/settings.json",
            ".mcp.json",
            "AGENTS.md",
            "CLAUDE.md"
        ]
    );
    // None names the data root, a token, a key or a journal.
    for f in names(&home) {
        let text = std::fs::read_to_string(home.join(&f)).unwrap();
        for bad in [
            root.path().to_str().unwrap(),
            "token",
            "journal",
            "vault",
            "key",
        ] {
            assert!(!text.contains(bad), "{f} mentions {bad:?}: {text}");
        }
    }
    let agents = std::fs::read_to_string(home.join("AGENTS.md")).unwrap();
    assert_eq!(agents.lines().count(), 3);
    assert!(agents.contains("Acme Inc.'s books"), "{agents}");
    assert!(agents.contains("books.start"));
    assert_eq!(
        agents,
        std::fs::read_to_string(home.join("CLAUDE.md")).unwrap()
    );
    // The pointer and the allowance.
    let mcp: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(".mcp.json")).unwrap()).unwrap();
    assert_eq!(
        mcp["mcpServers"]["wanderbooks-acme"],
        serde_json::json!({"command": "/usr/bin/wanderbooks", "args": ["mcp", "--book", "acme"]})
    );
    let settings: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(".claude/settings.json")).unwrap())
            .unwrap();
    assert_eq!(
        settings["permissions"]["allow"],
        serde_json::json!(["mcp__wanderbooks-acme__*"])
    );
}

#[test]
fn a_books_home_made_again_keeps_what_a_person_kept() {
    let docs = tempfile::tempdir().unwrap();
    let exe = PathBuf::from("/usr/bin/wanderbooks");
    let home = make_books_home_in(docs.path(), "acme", None, &exe).unwrap();
    // Their words in AGENTS.md; their own server and rule in the JSON.
    std::fs::write(home.join("AGENTS.md"), "Mine.\n").unwrap();
    std::fs::write(
        home.join(".mcp.json"),
        r#"{"mcpServers":{"other":{"command":"x"}}}"#,
    )
    .unwrap();
    std::fs::write(
        home.join(".claude/settings.json"),
        r#"{"permissions":{"allow":["Bash(ls)"]},"model":"opus"}"#,
    )
    .unwrap();
    let exe2 = PathBuf::from("/opt/wanderbooks");
    make_books_home_in(docs.path(), "acme", None, &exe2).unwrap();
    make_books_home_in(docs.path(), "acme", None, &exe2).unwrap();
    assert_eq!(
        std::fs::read_to_string(home.join("AGENTS.md")).unwrap(),
        "Mine.\n"
    );
    let mcp: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(".mcp.json")).unwrap()).unwrap();
    assert_eq!(mcp["mcpServers"]["other"]["command"], "x");
    assert_eq!(
        mcp["mcpServers"]["wanderbooks-acme"]["command"],
        "/opt/wanderbooks"
    );
    let settings: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join(".claude/settings.json")).unwrap())
            .unwrap();
    assert_eq!(settings["model"], "opus");
    assert_eq!(
        settings["permissions"]["allow"],
        serde_json::json!(["Bash(ls)", "mcp__wanderbooks-acme__*"]),
        "once, however often it is made"
    );
}

#[test]
fn a_books_home_is_never_made_for_a_name_that_is_not_an_id() {
    let docs = tempfile::tempdir().unwrap();
    let exe = PathBuf::from("/usr/bin/wanderbooks");
    for bad in ["../x", "a/b", "", "A B"] {
        assert!(
            make_books_home_in(docs.path(), bad, None, &exe).is_err(),
            "{bad:?}"
        );
    }
    assert!(std::fs::read_dir(docs.path()).unwrap().next().is_none());
    // And not into a relative place.
    assert!(make_books_home_in(Path::new("Documents"), "acme", None, &exe).is_err());
}

#[cfg(unix)]
#[test]
fn an_agent_is_found_on_the_path_or_where_its_installer_puts_it() {
    use std::os::unix::fs::PermissionsExt;
    let bin = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let docs = tempfile::tempdir().unwrap();
    let exe = PathBuf::from("/usr/bin/wanderbooks");
    let path = std::ffi::OsString::from(bin.path());
    // Neither: missing, with the vendor's installer to offer.
    let nowhere = std::ffi::OsString::from("/nonexistent");
    match start_agent_in(
        docs.path(),
        &exe,
        Some(&nowhere),
        Some(home.path()),
        "claude",
        "acme",
    )
    .unwrap()
    {
        AgentStart::Missing(i) => {
            assert_eq!(i, installer("claude").unwrap());
            assert!(i.command.contains("claude.ai/install"), "{}", i.command);
        }
        other => panic!("{other:?}"),
    }
    // On the PATH.
    let claude = bin.path().join("claude");
    std::fs::write(&claude, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        find_agent_in("claude", Some(&path), Some(home.path())),
        Some(claude.clone())
    );
    // A file that cannot be run is not the agent.
    let codex = bin.path().join("codex");
    std::fs::write(&codex, "x").unwrap();
    assert_eq!(find_agent_in("codex", Some(&path), Some(home.path())), None);
    // Where its installer puts it.
    let local = home.path().join(".local/bin");
    std::fs::create_dir_all(&local).unwrap();
    let codex = local.join("codex");
    std::fs::write(&codex, "#!/bin/sh\n").unwrap();
    std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        find_agent_in("codex", Some(&nowhere), Some(home.path())),
        Some(codex)
    );
    // Starting: in the books home, nothing marked trusted.
    match start_agent_in(
        docs.path(),
        &exe,
        Some(&path),
        Some(home.path()),
        "claude",
        "acme",
    )
    .unwrap()
    {
        AgentStart::Ready {
            program,
            dir,
            args,
            env,
            label,
        } => {
            assert_eq!(program, claude);
            assert_eq!(dir, books_home_in(docs.path(), "acme"));
            assert_eq!(label, "Claude Code");
            assert!(args.is_empty());
            assert!(
                env.iter()
                    .any(|(k, v)| k == "TERM" && v == "xterm-256color")
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(start_agent_in(docs.path(), &exe, None, None, "gemini", "acme").is_err());
}

#[test]
fn a_first_run_reads_where_it_stands_and_sets_the_first_passphrase_once() {
    use ledger_host::commands::{Common, FirstRun, first_run_state, set_first_passphrase};
    let home = tempfile::tempdir().unwrap();
    let place = home.path().join("data");
    let common = Common {
        root: Some(place.clone()),
        ..Default::default()
    };
    // Nothing there yet.
    assert_eq!(first_run_state(&common), FirstRun::default());
    // Set up (what `wanderbooks setup --root` does), without a terminal.
    ledger_store::root::Root::setup(home.path(), Some(&place), None).unwrap();
    assert_eq!(
        first_run_state(&common),
        FirstRun {
            root: true,
            books: vec![],
            passphrase_set: false
        }
    );
    // Too short is refused; a good one is set; a second first one is refused.
    assert!(set_first_passphrase(&common, "short").is_err());
    set_first_passphrase(&common, "a long enough passphrase").unwrap();
    assert!(first_run_state(&common).passphrase_set);
    let again = set_first_passphrase(&common, "another long passphrase").unwrap_err();
    assert!(again.contains("wanderbooks passphrase"), "{again}");
}
