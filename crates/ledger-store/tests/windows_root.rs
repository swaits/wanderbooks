//! The data root on Windows (P11c, D-62): the same promises as Unix's
//! mode 0700, kept with an owner-only DACL. Runs on the Windows job only.
#![cfg(windows)]

use ledger_store::root::{Root, RootError, default_root};
use ledger_store::winsec;
use std::path::Path;

fn tmp(tag: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(&format!("wb-winroot-{tag}-"))
        .tempdir()
        .unwrap()
}

#[test]
fn the_default_root_sits_under_local_app_data() {
    let home = tmp("default");
    assert_eq!(
        default_root(home.path()),
        home.path()
            .join("AppData")
            .join("Local")
            .join("wanderbooks")
    );
    // This account's own home: the shell's LocalAppData, wherever it is.
    let own = ledger_store::root::account_home().unwrap();
    assert_eq!(
        default_root(&own),
        winsec::local_app_data().unwrap().join("wanderbooks")
    );
}

#[test]
fn setup_makes_a_private_root_and_its_folders() {
    let home = tmp("setup");
    let (root, report) = Root::setup(home.path(), None, None).unwrap();
    assert_eq!(root.path(), default_root(home.path()));
    assert!(report.created && !report.tightened && !report.recorded);
    assert!(winsec::is_private(root.path()).unwrap());
    for sub in ["books", "run", "legacy"] {
        assert!(winsec::is_private(&root.path().join(sub)).unwrap(), "{sub}");
    }
    // Again: nothing to do.
    let (_, again) = Root::setup(home.path(), None, None).unwrap();
    assert!(!again.created && !again.tightened && !again.recorded);
    assert!(Root::find_in(home.path(), None).is_ok());
}

/// Let everyone into `path` and what it holds, as `icacls` does.
fn open_to_everyone(path: &Path) {
    let out = std::process::Command::new("icacls")
        .arg(path)
        .args(["/grant", "*S-1-1-0:(OI)(CI)F"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
}

#[test]
fn a_root_others_can_open_is_refused_and_setup_makes_it_private_again() {
    let home = tmp("loose");
    let (root, _) = Root::setup(home.path(), None, None).unwrap();
    open_to_everyone(root.path());
    assert!(!winsec::is_private(root.path()).unwrap());
    assert!(matches!(Root::open(root.path()), Err(RootError::Unsafe(_))));
    let (_, report) = Root::setup(home.path(), None, None).unwrap();
    assert!(report.tightened);
    assert!(winsec::is_private(root.path()).unwrap());
    assert!(Root::open(root.path()).is_ok());
}

#[test]
fn what_is_made_in_a_private_folder_is_private() {
    let home = tmp("inherit");
    let (root, _) = Root::setup(home.path(), None, None).unwrap();
    let file = root.path().join("books").join("note.txt");
    std::fs::write(&file, b"x").unwrap();
    assert!(winsec::is_private(&file).unwrap());
    let sub = root.path().join("books").join("inner");
    std::fs::create_dir(&sub).unwrap();
    assert!(winsec::is_private(&sub).unwrap());
}

#[test]
fn a_file_made_owner_only_is_private_and_a_loosened_one_is_not() {
    let dir = tmp("file");
    let file = dir.path().join("f");
    std::fs::write(&file, b"x").unwrap();
    winsec::set_owner_only(&file).unwrap();
    assert!(winsec::is_private(&file).unwrap());
    let out = std::process::Command::new("icacls")
        .arg(&file)
        .args(["/grant", "*S-1-1-0:R"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(!winsec::is_private(&file).unwrap());
}

#[test]
fn a_rename_never_goes_over_what_is_there() {
    let dir = tmp("rename");
    let (a, b, c) = (
        dir.path().join("a"),
        dir.path().join("b"),
        dir.path().join("c"),
    );
    std::fs::write(&a, b"a").unwrap();
    std::fs::write(&b, b"b").unwrap();
    let err = winsec::rename_new(&a, &b).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists, "{err:?}");
    assert_eq!(std::fs::read(&b).unwrap(), b"b");
    winsec::rename_new(&a, &c).unwrap();
    assert_eq!(std::fs::read(&c).unwrap(), b"a");
    assert!(!a.exists());
}

#[test]
fn this_process_runs_as_the_account_that_the_token_says() {
    let me = winsec::current_user().unwrap();
    assert_eq!(winsec::process_user(std::process::id()).unwrap(), me);
    assert!(me.to_text().unwrap().starts_with("S-1-5-"));
}
