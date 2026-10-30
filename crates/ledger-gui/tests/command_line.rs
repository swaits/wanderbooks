//! The viewer's command line: an unknown argument stops the viewer with
//! status 2, and it opens only books of the data root (D-44): no flag
//! names a loose books folder, its settings file or a vault key.

use std::process::Command;

/// The viewer, run in a home of the test's own with an empty environment
/// but PATH: if an argument it should refuse were ever taken, it would
/// find no books, no display and no preferences of this account's.
fn viewer(home: &std::path::Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_ledger-gui"));
    c.env_clear().env("PATH", "/usr/bin:/bin").env("HOME", home);
    // A Windows program started with no environment at all can't load.
    #[cfg(windows)]
    for name in ["SystemRoot", "windir"] {
        if let Some(value) = std::env::var_os(name) {
            c.env(name, value);
        }
    }
    c
}

#[test]
fn an_unknown_argument_stops_the_viewer() {
    let home = tempfile::Builder::new()
        .prefix("wb-gui-args-")
        .tempdir()
        .unwrap();
    let out = viewer(home.path()).arg("--no-such-flag").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("unknown argument --no-such-flag"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let help = viewer(home.path()).arg("--help").output().unwrap();
    assert!(help.status.success());
    let help = String::from_utf8_lossy(&help.stdout);
    assert!(help.contains("--book ID"), "{help}");
    for gone in ["--data-dir", "--books-toml", "--vault-key-file"] {
        assert!(!help.contains(gone), "{help}");
    }
    // A gone flag is an unknown argument like any other.
    for gone in ["--data-dir", "--books-toml", "--vault-key-file"] {
        let out = viewer(home.path()).args([gone, "x"]).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{gone}");
    }
}
