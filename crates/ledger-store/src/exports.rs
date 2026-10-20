//! Files on their way out (plan 4.7, D-29): a file the server makes for an
//! agent (the CPA package, the 1099 worksheet, a W-9) is kept here a
//! while, for the token that asked for it, until `wanderbooks download`
//! fetches it.
//!
//! ```text
//! GOAL: an agent hands the human a file of any size the books make,
//!       without its bytes passing through the agent's context or naming a
//!       path on the server.
//! SOURCE: plan 4.7 (`download_link`: "valid 10 minutes, access-logged");
//!         D-29 ("their files are made by the server and handed over by a
//!         command, `wanderbooks download`").
//! INVARIANTS:
//!   1. An export is kept for one token, under an id of no digits (the
//!      access log masks what could be a TIN), and lasts EXPORT_TTL; only
//!      that token fetches it, as often as it likes until it lapses.
//!   2. It is written aside and renamed whole: a fetch gets the whole file
//!      or none, and its bytes hash to the SHA-256 kept with it.
//!   3. Exports live in the book's `exports/` folder (0700, files 0600);
//!      lapsed ones are swept when the books open, whenever one is made,
//!      and by the server's sweeper. A token has at most MAX_LIVE.
//!   4. No error names a path.
//! ```

use crate::journal::sha256_hex;
use crate::uploads::{file_name_problem, mime_problem};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

/// The exports' folder, in a book's folder.
pub const EXPORTS_DIR: &str = "exports";
/// How long an export is kept, seconds.
pub const EXPORT_TTL: i64 = 15 * 60;
/// The most exports one token may have kept at once (INVARIANT 3).
pub const MAX_LIVE: usize = 16;

/// What an export is (INVARIANT 1): its bytes are beside it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Export {
    /// Its id: `ex_` and 32 letters `a` to `p`.
    pub id: String,
    /// The token it was made for.
    pub token: String,
    /// The file's name.
    pub file_name: String,
    /// Its MIME type.
    pub mime: String,
    /// Its size.
    pub size: u64,
    /// Its SHA-256, hex.
    pub sha256: String,
    /// When it lapses (Unix seconds).
    pub expires: i64,
}

/// Why an export can't be kept or fetched. Never names a path.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExportError {
    /// No such export, or it lapsed.
    #[error("there is no export {0:?}, or it lapsed (an export is kept 15 minutes): make it again")]
    Unknown(String),
    /// Another token's.
    #[error("that export was made for another token")]
    NotYours,
    /// Kept, but not whole (changed on the disk).
    #[error("that export was damaged on the disk: make it again")]
    Damaged,
    /// A request that can't make one.
    #[error("{0}")]
    Bad(String),
    /// The disk refused.
    #[error("the export couldn't be kept: {0}")]
    Io(std::io::ErrorKind),
}

impl From<std::io::Error> for ExportError {
    fn from(e: std::io::Error) -> Self {
        ExportError::Io(e.kind())
    }
}

/// A book's exports.
#[derive(Debug, Clone)]
pub struct Exports {
    dir: PathBuf,
    /// Held while exports are made or swept (INVARIANT 3).
    lock: std::sync::Arc<std::sync::Mutex<()>>,
}

/// `ex_` and 32 letters `a` to `p`, a letter a nibble (INVARIANT 1).
pub fn is_id(id: &str) -> bool {
    id.len() == 3 + 32
        && id.starts_with("ex_")
        && id[3..].bytes().all(|b| (b'a'..=b'p').contains(&b))
}

fn new_id(raw: &[u8; 16]) -> String {
    let mut id = String::from("ex_");
    for b in raw {
        id.push(char::from(b'a' + (b >> 4)));
        id.push(char::from(b'a' + (b & 0x0f)));
    }
    id
}

fn private_new(path: &Path) -> std::io::Result<std::fs::File> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)
}

impl Exports {
    /// The exports of the book in `book_dir` (its folder made, private, if
    /// missing), lapsed ones swept.
    pub fn open(book_dir: &Path, now: i64) -> Result<Self, ExportError> {
        let dir = crate::root::private_book_folder(book_dir, EXPORTS_DIR)
            .map_err(|e| ExportError::Bad(format!("the exports can't be kept: {e}")))?;
        let exports = Self {
            dir,
            lock: Default::default(),
        };
        exports.sweep(now);
        Ok(exports)
    }

    fn locked(&self) -> std::sync::MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn path(&self, id: &str, ext: &str) -> PathBuf {
        self.dir.join(format!("{id}.{ext}"))
    }

    /// Keep `bytes` as `file_name` for `token` (INVARIANTS 1-3).
    pub fn put(
        &self,
        token: &str,
        file_name: &str,
        mime: &str,
        bytes: &[u8],
        now: i64,
    ) -> Result<Export, ExportError> {
        if let Some(why) = file_name_problem(file_name).or_else(|| mime_problem(mime)) {
            return Err(ExportError::Bad(why));
        }
        let _held = self.locked();
        self.sweep_held(now);
        if self.live_for(token, now) >= MAX_LIVE {
            return Err(ExportError::Bad(format!(
                "this agent has {MAX_LIVE} exports kept: each lapses 15 minutes after it was made; make this one again then"
            )));
        }
        let mut raw = [0u8; 16];
        getrandom::getrandom(&mut raw)
            .map_err(|_| ExportError::Bad("the system gave no randomness".into()))?;
        let export = Export {
            id: new_id(&raw),
            token: token.to_string(),
            file_name: file_name.to_string(),
            mime: mime.to_string(),
            size: bytes.len() as u64,
            sha256: sha256_hex(bytes),
            expires: now + EXPORT_TTL,
        };
        // The bytes first, then the record that names them: each written
        // aside and renamed (INVARIANT 2).
        for (ext, content) in [
            ("bin", bytes.to_vec()),
            (
                "json",
                serde_json::to_vec(&export).expect("an export serializes"),
            ),
        ] {
            let aside = self.path(&export.id, &format!("{ext}.new"));
            let mut f = private_new(&aside)?;
            f.write_all(&content)?;
            f.sync_all()?;
            drop(f);
            std::fs::rename(&aside, self.path(&export.id, ext))?;
        }
        Ok(export)
    }

    /// Export `id` for `token` and its bytes, checked (INVARIANTS 1, 2).
    pub fn get(&self, id: &str, token: &str, now: i64) -> Result<(Export, Vec<u8>), ExportError> {
        if !is_id(id) {
            return Err(ExportError::Unknown(id.chars().take(40).collect()));
        }
        let unknown = || ExportError::Unknown(id.to_string());
        let record = std::fs::read(self.path(id, "json")).map_err(|_| unknown())?;
        let export: Export = serde_json::from_slice(&record).map_err(|_| unknown())?;
        if export.expires <= now {
            return Err(unknown());
        }
        if export.token != token {
            return Err(ExportError::NotYours);
        }
        let bytes = std::fs::read(self.path(id, "bin")).map_err(|_| unknown())?;
        if bytes.len() as u64 != export.size || sha256_hex(&bytes) != export.sha256 {
            return Err(ExportError::Damaged);
        }
        Ok((export, bytes))
    }

    /// How many exports `token` has kept at `now`.
    fn live_for(&self, token: &str, now: i64) -> usize {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return 0;
        };
        entries
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".json"))
            .filter_map(|e| std::fs::read(e.path()).ok())
            .filter_map(|b| serde_json::from_slice::<Export>(&b).ok())
            .filter(|x| x.token == token && x.expires > now)
            .count()
    }

    /// Sweep away every export that lapsed by `now`, and anything left
    /// without one (INVARIANT 3).
    pub fn sweep(&self, now: i64) {
        let _held = self.locked();
        self.sweep_held(now);
    }

    fn sweep_held(&self, now: i64) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let Some((id, _)) = name.split_once('.') else {
                let _ = std::fs::remove_file(e.path());
                continue;
            };
            let lapsed = std::fs::read(self.path(id, "json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<Export>(&b).ok())
                .is_none_or(|x| x.expires <= now);
            if lapsed {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    #[test]
    fn an_export_is_kept_fifteen_minutes() {
        // What its refusal tells the human: kept 15 minutes, then gone.
        let dir = tempfile::tempdir().unwrap();
        let x = Exports::open(dir.path(), NOW).unwrap();
        let e = x.put("t", "a.csv", "text/csv", b"x", NOW).unwrap();
        assert_eq!(e.expires, NOW + 15 * 60);
        assert!(x.get(&e.id, "t", NOW + 15 * 60 - 1).is_ok());
        let lapsed = x.get(&e.id, "t", NOW + 15 * 60).unwrap_err();
        assert!(lapsed.to_string().contains("kept 15 minutes"), "{lapsed}");
    }

    #[test]
    fn an_id_is_ex_and_thirty_two_letters_a_to_p_and_nothing_else() {
        assert!(is_id(&format!("ex_{}", "a".repeat(32))));
        assert!(is_id(&format!("ex_{}", "p".repeat(32))));
        for bad in [
            String::new(),
            "ex_".into(),
            format!("ex_{}", "a".repeat(31)),
            format!("ex_{}", "a".repeat(33)),
            format!("up_{}", "a".repeat(32)),
            format!("exx{}", "a".repeat(32)),
            format!("ex_{}", "q".repeat(32)),
            format!("ex_{}0", "a".repeat(31)),
            format!("ex_{}.", "a".repeat(31)),
        ] {
            assert!(!is_id(&bad), "{bad}");
        }
    }

    #[test]
    fn a_fetch_never_reaches_past_the_exports_folder() {
        // A whole export kept beside the books, not among their exports:
        // no id names it.
        let dir = tempfile::tempdir().unwrap();
        let x = Exports::open(dir.path(), NOW).unwrap();
        let bytes = b"not an export".to_vec();
        for id in ["../outside", &format!("up_{}", "a".repeat(32))] {
            let at = dir.path().join(EXPORTS_DIR).join(id);
            let record = Export {
                id: id.into(),
                token: "t".into(),
                file_name: "a.csv".into(),
                mime: "text/csv".into(),
                size: bytes.len() as u64,
                sha256: sha256_hex(&bytes),
                expires: NOW + EXPORT_TTL,
            };
            std::fs::write(
                at.with_extension("json"),
                serde_json::to_vec(&record).unwrap(),
            )
            .unwrap();
            std::fs::write(at.with_extension("bin"), &bytes).unwrap();
            assert!(
                matches!(x.get(id, "t", NOW).unwrap_err(), ExportError::Unknown(_)),
                "{id}"
            );
        }
        // And a refusal quotes no more than 40 characters of what it got.
        let long = format!("../{}", "x".repeat(100));
        let ExportError::Unknown(quoted) = x.get(&long, "t", NOW).unwrap_err() else {
            panic!("an unknown export")
        };
        assert_eq!(quoted.chars().count(), 40);
    }

    #[test]
    fn an_export_counts_against_its_token_until_the_moment_it_lapses() {
        let dir = tempfile::tempdir().unwrap();
        let x = Exports::open(dir.path(), NOW).unwrap();
        let e = x.put("t", "a.csv", "text/csv", b"x", NOW).unwrap();
        assert_eq!(x.live_for("t", e.expires - 1), 1);
        // Lapsed at its expiry, as a fetch then finds (not yet swept).
        assert_eq!(x.live_for("t", e.expires), 0);
        assert_eq!(x.live_for("u", NOW), 0);
    }

    #[test]
    fn an_export_is_fetched_by_its_token_until_it_lapses() {
        let dir = tempfile::tempdir().unwrap();
        let x = Exports::open(dir.path(), NOW).unwrap();
        let bytes = b"PK cpa package".to_vec();
        let e = x
            .put("tok_a", "cpa-2026.zip", "application/zip", &bytes, NOW)
            .unwrap();
        assert!(is_id(&e.id));
        assert!(!e.id[3..].bytes().any(|b| b.is_ascii_digit()));
        assert_eq!(e.expires, NOW + EXPORT_TTL);
        assert_eq!(e.sha256, sha256_hex(&bytes));
        // As often as it likes, by its own token only.
        for _ in 0..2 {
            let (got, b) = x.get(&e.id, "tok_a", NOW + 1).unwrap();
            assert_eq!((got, b), (e.clone(), bytes.clone()));
        }
        assert_eq!(
            x.get(&e.id, "tok_b", NOW).unwrap_err(),
            ExportError::NotYours
        );
        assert!(matches!(
            x.get(&e.id, "tok_a", NOW + EXPORT_TTL).unwrap_err(),
            ExportError::Unknown(_)
        ));
        for bad in [
            "../../journal",
            "ex_",
            "up_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "",
        ] {
            assert!(matches!(
                x.get(bad, "tok_a", NOW).unwrap_err(),
                ExportError::Unknown(_)
            ));
        }
        // Swept once lapsed, nothing left.
        x.sweep(NOW + EXPORT_TTL);
        assert_eq!(
            std::fs::read_dir(dir.path().join(EXPORTS_DIR))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn a_damaged_export_is_never_handed_over() {
        let dir = tempfile::tempdir().unwrap();
        let x = Exports::open(dir.path(), NOW).unwrap();
        let e = x
            .put("t", "w9.pdf", "application/pdf", b"%PDF-1.4 W-9", NOW)
            .unwrap();
        let bin = dir.path().join(EXPORTS_DIR).join(format!("{}.bin", e.id));
        std::fs::write(&bin, b"%PDF-1.4 W-8").unwrap();
        assert_eq!(x.get(&e.id, "t", NOW).unwrap_err(), ExportError::Damaged);
        std::fs::write(&bin, b"%PDF").unwrap();
        assert_eq!(x.get(&e.id, "t", NOW).unwrap_err(), ExportError::Damaged);
    }

    #[test]
    fn a_token_keeps_only_so_many_and_names_are_plain() {
        let dir = tempfile::tempdir().unwrap();
        let x = Exports::open(dir.path(), NOW).unwrap();
        for _ in 0..MAX_LIVE {
            x.put("t", "a.csv", "text/csv", b"x", NOW).unwrap();
        }
        assert!(matches!(
            x.put("t", "a.csv", "text/csv", b"x", NOW).unwrap_err(),
            ExportError::Bad(_)
        ));
        x.put("u", "a.csv", "text/csv", b"x", NOW).unwrap();
        // Lapsed ones don't count: making one sweeps them.
        x.put("t", "a.csv", "text/csv", b"x", NOW + EXPORT_TTL)
            .unwrap();
        assert_eq!(
            std::fs::read_dir(dir.path().join(EXPORTS_DIR))
                .unwrap()
                .count(),
            2,
            "every lapsed one swept (both tokens'): the new one kept, a record and its bytes"
        );
        for (name, mime) in [("../x.zip", "application/zip"), ("x.zip", "zip")] {
            assert!(matches!(
                x.put("u", name, mime, b"x", NOW).unwrap_err(),
                ExportError::Bad(_)
            ));
        }
    }

    #[cfg(unix)]
    #[test]
    fn its_folder_and_files_are_private_and_strays_go() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let x = Exports::open(dir.path(), NOW).unwrap();
        let e = x.put("t", "a.csv", "text/csv", b"x", NOW).unwrap();
        let folder = dir.path().join(EXPORTS_DIR);
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&folder), 0o700);
        for ext in ["json", "bin"] {
            assert_eq!(mode(&folder.join(format!("{}.{ext}", e.id))), 0o600);
        }
        // A file left half made, or with no record, is swept.
        std::fs::write(folder.join("ex_x.bin.new"), b"half").unwrap();
        std::fs::write(folder.join("stray"), b"x").unwrap();
        let _ = Exports::open(dir.path(), NOW).unwrap();
        let mut left: Vec<String> = std::fs::read_dir(&folder)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, [format!("{}.bin", e.id), format!("{}.json", e.id)]);
    }
}
