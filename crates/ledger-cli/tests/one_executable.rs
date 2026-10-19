//! One executable (plan D-22): `wanderbooks` runs the books server
//! (`dev-stdio`) and the viewer (`gui`) as modes of itself, each with its
//! own flags, and the server-only build has no viewer.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

fn wanderbooks() -> Command {
    Command::new(env!("CARGO_BIN_EXE_wanderbooks"))
}

/// One JSON-RPC request over the child's stdio, its response parsed.
fn ask(
    stdin: &mut std::process::ChildStdin,
    stdout: &mut BufReader<std::process::ChildStdout>,
    id: u64,
    method: &str,
    params: Value,
) -> Value {
    let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
    writeln!(stdin, "{msg}").unwrap();
    stdin.flush().unwrap();
    let mut line = String::new();
    stdout.read_line(&mut line).expect("read a response");
    let resp: Value = serde_json::from_str(&line).expect("parse the response");
    assert_eq!(resp["id"], id, "{resp}");
    resp
}

#[test]
fn dev_stdio_is_the_books_server() {
    let dir = tempfile::Builder::new()
        .prefix("wb-one-exe-")
        .tempdir()
        .unwrap();
    let data = dir.path().join("books").join("books-data");
    let mut child = wanderbooks()
        .arg("dev-stdio")
        .arg("--data-dir")
        .arg(&data)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn wanderbooks dev-stdio");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let init = ask(
        &mut stdin,
        &mut stdout,
        1,
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {},
               "clientInfo": {"name": "one-executable", "version": "0"}}),
    );
    assert!(init["result"]["serverInfo"].is_object(), "{init}");
    writeln!(
        stdin,
        "{}",
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
    )
    .unwrap();
    let tools = ask(&mut stdin, &mut stdout, 2, "tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .expect("a tool list")
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    assert!(names.contains(&"books.start"), "{names:?}");
    // The books it serves are real books: a fresh store was created.
    let tb = ask(
        &mut stdin,
        &mut stdout,
        3,
        "tools/call",
        json!({"name": "books.trial_balance", "arguments": {}}),
    );
    assert!(tb.get("error").is_none(), "{tb}");
    assert!(data.join("journal.jsonl").is_file());
    drop(stdin);
    let status = child.wait().unwrap();
    assert!(status.success(), "{status}");
}

#[test]
fn each_mode_names_itself_in_its_usage() {
    let out = wanderbooks()
        .args(["dev-stdio", "--help"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("usage: wanderbooks dev-stdio [--data-dir PATH]"),
        "{err}"
    );

    let out = wanderbooks().args(["server", "--help"]).output().unwrap();
    assert!(out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("usage: wanderbooks server [--root DIR]"),
        "{err}"
    );

    let out = wanderbooks().arg("--help").output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    for line in [
        "wanderbooks gui [DIR",
        "wanderbooks dev-stdio",
        "dev-stdio  run the books server",
    ] {
        assert!(err.contains(line), "{line:?} missing from {err}");
    }
}

#[cfg(feature = "gui")]
#[test]
fn the_viewer_names_itself_in_its_usage() {
    let out = wanderbooks().args(["gui", "--help"]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("usage: wanderbooks gui [--data-dir PATH]"),
        "{text}"
    );
}

#[test]
fn a_mode_refuses_what_it_does_not_know() {
    let modes: &[&str] = if cfg!(feature = "gui") {
        &["dev-stdio", "gui", "server"]
    } else {
        &["dev-stdio", "server"]
    };
    for &mode in modes {
        let out = wanderbooks()
            .args([mode, "--no-such-flag"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{mode}: {out:?}");
    }
    // A mode's flags are its own: the CLI's `--force` means nothing there.
    let out = wanderbooks()
        .args(["dev-stdio", "--force"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{out:?}");
}

#[test]
fn a_mode_comes_first() {
    for mode in ["gui", "dev-stdio", "server"] {
        let out = wanderbooks().args(["--codex", mode]).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{mode}: {out:?}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains(&format!("`{mode}` must come first")), "{err}");
    }
}

#[test]
fn a_server_that_cannot_start_says_why_and_exits_1() {
    // A flag missing its value is an error, not a usage slip (exit 2).
    let out = wanderbooks()
        .args(["dev-stdio", "--data-dir"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("error: --data-dir needs a path"), "{err}");
}

/// With no display to open a window on, the viewer fails and says so
/// (Linux only: elsewhere there is always a display, and a window would
/// open).
#[cfg(all(feature = "gui", target_os = "linux"))]
#[test]
fn a_viewer_that_cannot_open_a_window_exits_1() {
    let dir = tempfile::Builder::new()
        .prefix("wb-one-exe-")
        .tempdir()
        .unwrap();
    let mut child = wanderbooks()
        .args(["gui", "--data-dir"])
        .arg(dir.path().join("books-data"))
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            panic!("the viewer neither opened nor failed within a minute");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    assert_eq!(status.code(), Some(1), "{status}");
    let mut err = String::new();
    std::io::Read::read_to_string(&mut child.stderr.take().unwrap(), &mut err).unwrap();
    assert!(err.contains("error: "), "{err}");
}

#[cfg(not(feature = "gui"))]
#[test]
fn the_server_only_build_has_no_viewer() {
    let out = wanderbooks().args(["gui", "--help"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("the server-only build: it has no viewer"),
        "{err}"
    );
}

/// Books in the data root are the books server's alone (plan 3.1): the
/// test server refuses them, and the books server needs a root.
#[cfg(unix)]
#[test]
fn dev_stdio_refuses_books_in_the_private_folder() {
    let home = tempfile::Builder::new()
        .prefix("wb-one-root-")
        .tempdir()
        .unwrap();
    let out = wanderbooks()
        .args(["setup", "--home"])
        .arg(home.path())
        .env_remove("XDG_DATA_HOME")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let root = home.path().join(".local/share/wanderbooks");
    for inside in [
        root.join("books/acme"),
        root.clone(),
        root.join("books/a/b"),
    ] {
        let out = wanderbooks()
            .args(["dev-stdio", "--data-dir"])
            .arg(&inside)
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{inside:?}: {out:?}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("only the books server opens"), "{err}");
        assert!(!inside.join("journal.jsonl").exists(), "{inside:?}");
    }
    assert!(!root.join("books/acme").exists());
    // A relative path, from inside the root.
    let out = wanderbooks()
        .args(["dev-stdio", "--data-dir", "acme"])
        .current_dir(root.join("books"))
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(!root.join("books/acme").exists());

    // No root yet: the books server says to run setup.
    let empty = tempfile::Builder::new()
        .prefix("wb-one-noroot-")
        .tempdir()
        .unwrap();
    let out = wanderbooks()
        .args(["server", "--root"])
        .arg(empty.path().join("none"))
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("run `wanderbooks setup` first"),
        "{out:?}"
    );
}
