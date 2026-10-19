//! `wanderbooks setup`: the books' private folder, made from the command
//! line, with its location never printed.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn wanderbooks() -> Command {
    Command::new(env!("CARGO_BIN_EXE_wanderbooks"))
}

fn mode(path: &std::path::Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn setup_makes_the_private_folder_and_never_names_it() {
    let home = tempfile::Builder::new()
        .prefix("wb-setup-")
        .tempdir()
        .unwrap();
    let root = home.path().join(".local/share/wanderbooks");
    let out = wanderbooks()
        .args(["setup", "--home"])
        .arg(home.path())
        .env_remove("XDG_DATA_HOME")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("The books' private folder: made"), "{text}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--home isn't this account's home"),
        "{out:?}"
    );
    assert!(
        !text.contains(home.path().to_str().unwrap()),
        "names the folder: {text}"
    );
    assert!(!String::from_utf8_lossy(&out.stderr).contains(home.path().to_str().unwrap()));
    assert_eq!(mode(&root), 0o700);
    for sub in ["books", "run", "legacy"] {
        assert_eq!(mode(&root.join(sub)), 0o700, "{sub}");
    }
    // Again: already there.
    let out = wanderbooks()
        .args(["setup", "--home"])
        .arg(home.path())
        .env_remove("XDG_DATA_HOME")
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("already there."),
        "{out:?}"
    );
    // Others could open it: made private again.
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = wanderbooks()
        .args(["setup", "--home"])
        .arg(home.path())
        .env_remove("XDG_DATA_HOME")
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("made private again"),
        "{out:?}"
    );
    assert_eq!(mode(&root), 0o700);
}

#[test]
fn a_root_elsewhere_is_noted_for_next_time() {
    // (XDG_DATA_HOME is honoured only for this account's own home, which
    // a test never touches: Root::setup's tests cover it.)
    let home = tempfile::Builder::new()
        .prefix("wb-setup-else-")
        .tempdir()
        .unwrap();
    let elsewhere = home.path().join("elsewhere").join("wanderbooks");
    let out = wanderbooks()
        .args(["setup", "--home"])
        .arg(home.path())
        .arg("--root")
        .arg(&elsewhere)
        .env_remove("XDG_DATA_HOME")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("noted for next time"),
        "{out:?}"
    );
    assert_eq!(mode(&elsewhere), 0o700);
    let noted =
        std::fs::read_to_string(home.path().join(".local/share/wanderbooks.location")).unwrap();
    assert_eq!(noted.trim_end(), elsewhere.display().to_string());
}

#[test]
fn setup_refuses_what_it_does_not_know_and_unsafe_folders() {
    let out = wanderbooks().args(["setup", "--bogus"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let out = wanderbooks().args(["setup", "--root"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--root needs a path"));
    let home = tempfile::Builder::new()
        .prefix("wb-setup-link-")
        .tempdir()
        .unwrap();
    let real = home.path().join("real");
    std::fs::create_dir(&real).unwrap();
    std::os::unix::fs::symlink(&real, home.path().join("link")).unwrap();
    let out = wanderbooks()
        .args(["setup", "--home"])
        .arg(home.path())
        .arg("--root")
        .arg(home.path().join("link"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("is a symlink"));
    // A mode comes first.
    let out = wanderbooks().args(["--force", "setup"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}

/// With another home given (tests), this account's XDG_DATA_HOME is never
/// read: setup can't be aimed at this account's real books by it.
#[test]
fn another_home_ignores_this_accounts_data_home() {
    let home = tempfile::Builder::new()
        .prefix("wb-setup-xdg-")
        .tempdir()
        .unwrap();
    let elsewhere = home.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    let out = wanderbooks()
        .args(["setup", "--home"])
        .arg(home.path())
        .env("XDG_DATA_HOME", &elsewhere)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(
        home.path()
            .join(".local/share/wanderbooks/.wanderbooks-root")
            .is_file()
    );
    assert_eq!(
        std::fs::read_dir(&elsewhere).unwrap().count(),
        0,
        "XDG_DATA_HOME was used"
    );
}
