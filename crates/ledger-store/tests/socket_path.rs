//! Socket paths for roots too deep for a Unix socket's address (about 104
//! bytes): Linux reaches them through `/proc/self/fd`, the other Unixes
//! through a short private link under `/tmp`, so a books root in a deep
//! home is served like any other (the portable macOS job found the
//! refusal).
#![cfg(unix)]

use ledger_store::root::{SocketPath, short_link_for};
use std::path::PathBuf;

fn deep_dir(tag: &str) -> PathBuf {
    let deep = std::env::temp_dir()
        .join(format!("wb-socket-{tag}-{}", std::process::id()))
        .join("a".repeat(60))
        .join("b".repeat(60))
        .join("run");
    std::fs::create_dir_all(&deep).unwrap();
    deep
}

/// A root deeper than the address allows still gets a socket path that
/// fits, and a second asker (a client) reaches the same place.
#[test]
fn a_deep_root_gets_a_socket_path_that_fits() {
    let deep = deep_dir("fits");
    assert!(deep.as_os_str().len() > 100);
    let a = SocketPath::new(&deep, "mcp.sock").unwrap();
    assert!(a.path().as_os_str().len() <= 100, "{:?}", a.path());
    // The place it reaches is the dir itself.
    let reached = std::fs::canonicalize(a.path().parent().unwrap()).unwrap();
    assert_eq!(reached, std::fs::canonicalize(&deep).unwrap());
}

/// The short link is this account's, names the dir exactly, is the same
/// on a second ask, and replaces a stale link of ours in its place.
#[test]
fn the_short_link_is_private_stable_and_replaces_a_stale_one() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let deep = deep_dir("link");
    let link = short_link_for(&deep).unwrap();
    assert!(
        link.as_os_str().len() + "/lease.sock".len() <= 100,
        "{link:?}"
    );
    // SAFETY: getuid has no preconditions and never fails.
    let uid = unsafe { libc::getuid() };
    assert_eq!(
        link.parent().unwrap(),
        PathBuf::from(format!("/tmp/wanderbooks-{uid}"))
    );
    let folder = std::fs::symlink_metadata(link.parent().unwrap()).unwrap();
    assert_eq!(folder.uid(), uid);
    assert_eq!(folder.permissions().mode() & 0o777, 0o700);
    assert_eq!(
        std::fs::read_link(&link).unwrap(),
        std::fs::canonicalize(&deep).unwrap()
    );
    assert_eq!(short_link_for(&deep).unwrap(), link);
    // A stale link of ours (to somewhere else) in that place is replaced.
    let elsewhere = deep_dir("elsewhere");
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &link).unwrap();
    assert_eq!(short_link_for(&deep).unwrap(), link);
    assert_eq!(
        std::fs::read_link(&link).unwrap(),
        std::fs::canonicalize(&deep).unwrap()
    );
}
