//! The books' named pipes on Windows (P11c, D-62, plan 3.9): what keeps a
//! second account out of them, in the two ways the design has (the pipe's
//! DACL, and the SID check of the process at the other end). Windows job
//! only.
#![cfg(windows)]

use ledger_mcp::ipc::{Endpoint, Listener};
use ledger_store::winsec;
use std::path::Path;

/// The process id of `image` (a system program), if `tasklist` finds it.
fn pid_of(image: &str) -> Option<u32> {
    let out = std::process::Command::new("tasklist")
        .args(["/FI", &format!("IMAGENAME eq {image}"), "/FO", "CSV", "/NH"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    // "services.exe","700","Services","0","12,345 K"
    text.lines()
        .find(|l| l.to_lowercase().contains(&image.to_lowercase()))?
        .split(',')
        .nth(1)?
        .trim_matches('"')
        .parse()
        .ok()
}

#[tokio::test]
async fn a_second_user_cannot_open_the_pipe() {
    let dir = tempfile::tempdir().unwrap();
    let endpoint = Endpoint::new(dir.path(), "lease.sock").unwrap();
    let _listener = Listener::bind(&endpoint).unwrap();

    // The pipe's own DACL names this account and no one else: Everyone,
    // Users and Authenticated Users are not on it, so the system itself
    // refuses another account's open.
    let pipe = Path::new(endpoint.pipe_name());
    assert!(
        winsec::is_private(pipe).unwrap(),
        "the pipe is open to more than this account"
    );

    // And a process of another account is never taken for this one: this
    // account's own is, and a system service (SYSTEM) is not.
    assert!(winsec::pid_is_us(std::process::id()));
    if let Some(pid) = pid_of("services.exe") {
        assert!(
            !winsec::pid_is_us(pid),
            "a SYSTEM process ({pid}) was taken for this account"
        );
    }
    assert!(!winsec::pid_is_us(0));
}
