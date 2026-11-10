//! A macOS app run translocated refuses to connect an agent (P11a step 6):
//! Gatekeeper runs a quarantined download from a read-only copy under
//! `/private/var/folders/…/AppTranslocation/…`, and that path goes away
//! with the app; an agent pointed at it would lose the books. The owner
//! moves the app out of Downloads first.
//!
//! The check is on the path the program would give the agent
//! (`ledger_host::start::this_executable`), so a test can stand in for
//! macOS on any Unix: `APPIMAGE` names a file there (with `APPDIR` naming
//! the folder the program runs from, as an image's launcher sets the two:
//! `APPIMAGE` alone is not believed, start.rs INVARIANT 6), and a path
//! with an `AppTranslocation` part is refused just as the real one is.

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
            .prefix("wb-translocation-")
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
        ledger_store::root::default_root(self.dir.path())
    }

    /// The program with a root of its own and `PATH` holding the stand-in
    /// agent. Nothing here touches the account's real data root.
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

/// A stand-in `claude` that leaves a mark when it is run, so the test can
/// say the agent never was.
fn fake_agent(home: &Home) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = home.dir.path().join("agent-bin");
    std::fs::create_dir_all(&bin).unwrap();
    let script = bin.join("claude");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\ntouch '{}'\nexit 0\n",
            home.dir.path().join("agent-ran").display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

#[test]
fn translocated_app_refuses_to_connect() {
    let home = Home::new();
    let out = home
        .cmd("books", Path::new("/nonexistent"))
        .args(["create", "beta"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");

    let bin = fake_agent(&home);
    // What macOS hands the program from Gatekeeper's copy: a path under
    // AppTranslocation. A bare file is enough (this_executable only asks
    // that it is one).
    let translocated = home.dir.path().join(
        "private/var/folders/x/AppTranslocation/uuid/d/Wanderbooks.app/Contents/MacOS/wanderbooks",
    );
    std::fs::create_dir_all(translocated.parent().unwrap()).unwrap();
    std::fs::write(&translocated, b"").unwrap();
    // The folder the program under test runs from stands in for the mount.
    let mount = Path::new(env!("CARGO_BIN_EXE_wanderbooks"))
        .parent()
        .unwrap();
    let out = home
        .cmd("connect", &bin)
        .env("APPIMAGE", &translocated)
        .env("APPDIR", mount)
        .args(["claude", "--book", "beta"])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "a translocated app connected an agent"
    );
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("move wanderbooks out of Downloads"), "{said}");
    // Refused before anything was done: no agent ran, no books home was
    // made for a path that will vanish.
    assert!(!home.dir.path().join("agent-ran").exists(), "the agent ran");
    assert!(
        !home.dir.path().join("Documents/Wanderbooks/beta").exists(),
        "a books home was made"
    );

    // The same program at an ordinary path connects: it is the
    // translocation, not the agent or the books, that was refused.
    let ordinary = home.dir.path().join("Wanderbooks.AppImage");
    std::fs::write(&ordinary, b"").unwrap();
    let out = home
        .cmd("connect", &bin)
        .env("APPIMAGE", &ordinary)
        .env("APPDIR", mount)
        .args(["claude", "--book", "beta"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(
        home.dir.path().join("agent-ran").exists(),
        "the agent wasn't run"
    );
}
