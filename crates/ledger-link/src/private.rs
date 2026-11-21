//! Files only this account can read: the app's key, its certificate's key,
//! the secret in its address.
//!
//! ```text
//! GOAL: nothing the relay could use to read traffic, and nothing that
//!       makes the address guessable, is ever readable by another account
//!       on this computer, nor half-written after a crash.
//! SOURCE: WB-link-1 (the app's identity), D-80, D-87; the token files'
//!         own rule (ledger-host tokens.rs: 0600, the owner-only DACL on
//!         Windows).
//! INVARIANTS:
//!   1. A file is made with mode 0600 (Unix) before a byte of it is
//!      written, or made owner-only before a byte is written (Windows,
//!      `ledger_store::winsec::set_owner_only`, the helper the token
//!      files use), and its folder 0700 or owner-only.
//!   2. A write is a temporary file, synced, then renamed over the old
//!      one, and the folder is synced after the rename (Unix; Windows has
//!      no such call and the rename is made durable by the file system):
//!      a reader sees the old file or the new one, never a part, and a
//!      crash right after cannot lose the rename.
//! SCENARIOS:
//!   1. A file written here reads back, has mode 0600, and its folder
//!      0700, on Unix; on Windows the file and its folder are owner-only
//!      (`ledger_store::winsec::is_private`).
//! ```

use std::io::{self, Write};
use std::path::Path;

/// Make `dir` (and its parents) private to this account.
pub fn make_dir(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(windows)]
    ledger_store::winsec::set_owner_only(dir)?;
    Ok(())
}

/// Write `bytes` to `path`, privately and atomically (INVARIANTS 1, 2).
pub fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        #[cfg(windows)]
        ledger_store::winsec::set_owner_only(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)?;
        sync_folder(path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Make the rename durable: the folder that holds the file is synced
/// (INVARIANT 2). Windows cannot open a folder to sync it; there the
/// rename is the file system's own.
fn sync_folder(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        let dir = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => Path::new("."),
        };
        std::fs::File::open(dir)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_private_file_reads_back_and_is_mode_0600() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("link");
        make_dir(&sub).unwrap();
        let f = sub.join("k");
        write(&f, b"one").unwrap();
        write(&f, b"two").unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"two");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&f), 0o600);
            assert_eq!(mode(&sub), 0o700);
        }
        // No temporary file is left behind.
        assert_eq!(std::fs::read_dir(&sub).unwrap().count(), 1);
    }

    /// Windows: the file and its folder are owner-only, as the token files
    /// are (the helper is `ledger_store::winsec`). Compiled for Windows
    /// only; it runs on the Windows jobs of the full test run.
    #[cfg(windows)]
    #[test]
    fn a_private_file_and_its_folder_are_owner_only_on_windows() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("link");
        make_dir(&sub).unwrap();
        let f = sub.join("k");
        write(&f, b"one").unwrap();
        write(&f, b"two").unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"two");
        assert!(ledger_store::winsec::is_private(&sub).unwrap());
        assert!(ledger_store::winsec::is_private(&f).unwrap());
    }
}
