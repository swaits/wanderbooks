//! One executable (plan D-22): `wanderbooks` runs the books server
//! (`server`) and the viewer (`gui`) as modes of itself, each with its own
//! flags, and the server-only build has no viewer.

#[cfg_attr(not(unix), allow(unused_imports))]
use std::process::{Command, Stdio};

#[cfg(all(feature = "gui", target_os = "linux"))]
mod common;

fn wanderbooks() -> Command {
    Command::new(env!("CARGO_BIN_EXE_wanderbooks"))
}

#[test]
fn each_mode_names_itself_in_its_usage() {
    let out = wanderbooks().args(["server", "--help"]).output().unwrap();
    assert!(out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("usage: wanderbooks server [--root DIR]"),
        "{err}"
    );

    let out = wanderbooks().arg("--help").output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr);
    for line in ["wanderbooks gui [BOOK", "wanderbooks mcp --book ID"] {
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
        text.contains("usage: wanderbooks gui [--root PATH] [--book ID]"),
        "{text}"
    );
}

#[test]
fn a_mode_refuses_what_it_does_not_know() {
    let modes: &[&str] = if cfg!(feature = "gui") {
        &["gui", "server"]
    } else {
        &["server"]
    };
    for &mode in modes {
        let out = wanderbooks()
            .args([mode, "--no-such-flag"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{mode}: {out:?}");
    }
    // A mode's flags are its own: the CLI's `--force` means nothing there.
    let out = wanderbooks().args(["server", "--force"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2), "{out:?}");
}

#[test]
fn the_books_folder_scaffold_is_gone_and_says_so() {
    for gone in ["new", "init"] {
        let out = wanderbooks().args([gone, "acme"]).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{gone}: {out:?}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains(&format!("`wanderbooks {gone}` is gone")),
            "{err}"
        );
        assert!(err.contains("wanderbooks books create ID"), "{err}");
    }
}

#[test]
fn a_mode_comes_first() {
    for mode in ["gui", "server"] {
        let out = wanderbooks().args(["--codex", mode]).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{mode}: {out:?}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains(&format!("`{mode}` must come first")), "{err}");
    }
}

#[test]
fn a_server_that_cannot_start_says_why_and_exits_1() {
    // A flag missing its value is an error, not a usage slip (exit 2).
    let out = wanderbooks().args(["server", "--root"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("error: --root needs a folder"), "{err}");
}

/// With no display to open a window on, the viewer fails and says so
/// (Linux only: elsewhere there is always a display, and a window would
/// open).
#[cfg(all(feature = "gui", target_os = "linux"))]
#[test]
fn a_viewer_that_cannot_open_a_window_exits_1() {
    // Empty books in a data root of the test's own, and a home of its
    // own: the viewer opens no loose books folder (D-44), and it reads its
    // preferences from the home, never this account's.
    let home = common::Home::new("one-exe");
    let server = common::Server::start(&home);
    home.create(common::BOOK, "USD");
    assert!(server.close().success());
    let mut gui = wanderbooks();
    gui.args(["gui", "--root"])
        .arg(home.root())
        .args(["--book", common::BOOK]);
    common::clean_env(&mut gui, home.dir.path());
    let mut child = gui
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + common::TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the viewer neither opened nor failed in time");
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

/// The books server needs a data root: with none, it says to run setup.
#[test]
fn the_books_server_needs_a_root() {
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

/// `upload` and `download` say how they ended in their exit code, as
/// every mode does: 0 for their usage on request, 2 for a slip in how
/// they were called, 1 for what went wrong doing it. A file's path may be
/// a name that isn't text: it reaches the command as given.
#[cfg(unix)]
#[test]
fn the_file_commands_end_with_the_code_that_says_how() {
    use std::os::unix::ffi::OsStrExt;
    let home = tempfile::Builder::new()
        .prefix("wb-one-files-")
        .tempdir()
        .unwrap();
    let root = home.path().join("wanderbooks");
    let run = |args: &[&std::ffi::OsStr]| {
        wanderbooks()
            .args(args)
            .env_remove("XDG_DATA_HOME")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let os = |s: &'static str| std::ffi::OsStr::new(s);
    for (mode, usage) in [
        ("upload", "usage: wanderbooks upload --book ID"),
        ("download", "usage: wanderbooks download --book ID"),
    ] {
        // Its usage on request.
        let out = run(&[os(mode), os("--help")]);
        assert_eq!(out.status.code(), Some(0), "{mode}: {out:?}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains(usage),
            "{out:?}"
        );
        // Called wrong: no file, or no books named.
        let out = run(&[
            os(mode),
            os("--root"),
            root.as_os_str(),
            os("--book"),
            os("acme"),
        ]);
        assert_eq!(out.status.code(), Some(2), "{mode}: {out:?}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains(usage),
            "{out:?}"
        );
        let out = run(&[os(mode), os("--root"), root.as_os_str(), os("1")]);
        assert_eq!(out.status.code(), Some(2), "{mode}: {out:?}");
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("which books? give --book ID"),
            "{out:?}"
        );
    }
    // A file that isn't there, named by a path that isn't text: what went
    // wrong, said, and nothing sent.
    let missing = home
        .path()
        .join(std::ffi::OsStr::from_bytes(b"receipt-\xff.pdf"));
    let out = run(&[
        os("upload"),
        os("--root"),
        root.as_os_str(),
        os("--book"),
        os("acme"),
        os("up_abcdefghijklmnopabcdefghijklmnop"),
        missing.as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("error: can't read"),
        "{out:?}"
    );
    // A folder to save in that isn't there: the same.
    let out = run(&[
        os("download"),
        os("--root"),
        root.as_os_str(),
        os("--book"),
        os("acme"),
        os("7"),
        os("--to"),
        home.path().join("no-such-folder").as_os_str(),
    ]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("isn't a folder"),
        "{out:?}"
    );
    assert!(!root.exists(), "the books were reached for");
}
