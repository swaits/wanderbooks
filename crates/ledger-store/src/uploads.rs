//! Files on their way in (plan 4.7, P2 slice 5): an agent asks for an
//! upload (`books.begin_upload`), sends the bytes on its own (`wanderbooks
//! upload`, which streams them to the server), then names the upload in
//! the tool that files them.
//!
//! ```text
//! GOAL: a file of any size up to the limit reaches the books whole,
//!       exactly as declared, without passing through the agent's context
//!       or naming a path on the server.
//! SOURCE: plan 4.7 ("single-use, valid 15 minutes, bound to the
//!         principal, sha and size, capped at 25 MiB by default, and
//!         re-hashed before acceptance"), D-28 (the agent does it).
//! INVARIANTS:
//!   1. A grant is made for one token, one SHA-256 and one size, and
//!      lasts UPLOAD_TTL; its bytes are received once, by that token
//!      alone, and filed once: a call that takes them holds them (no other
//!      call can) and spends them only when it files them; a call that
//!      fails, or panics, gives them back for the next.
//!   2. Bytes are written aside (`<id>.part`, by one request at a time)
//!      and kept (`<id>.bin`) only if they are exactly the size declared
//!      and hash to the SHA-256 declared; anything else leaves nothing but
//!      a refusal. Taken, they are hashed again.
//!   3. Grants and their bytes live in the book's `uploads/` folder (0700,
//!      files 0600), so they survive a restart; lapsed ones are swept when
//!      the books open, whenever a grant is made, and (by the server) every
//!      10 seconds, so none lingers; at open, bytes a call held when the
//!      server stopped are given back.
//!   4. No error names a path.
//!   5. A token has at most MAX_LIVE grants under way, so what one agent
//!      can keep on the disk is bounded.
//! ```

use crate::journal::sha256_hex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// The uploads' folder, in a book's folder.
pub const UPLOADS_DIR: &str = "uploads";
/// The largest file taken (plan 4.7).
pub const MAX_UPLOAD: u64 = 25 * 1024 * 1024;
/// How long a grant lasts, seconds (plan 4.7: 15 minutes).
pub const UPLOAD_TTL: i64 = 15 * 60;
/// The most grants one token may have under way at once (INVARIANT 5).
pub const MAX_LIVE: usize = 16;
/// What an upload may be for: the tools that file it take only theirs.
pub const PURPOSES: [&str; 4] = ["receipt", "document", "import", "w9"];

/// A grant: what may be sent, by whom, until when (INVARIANT 1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    /// Its id: `up_` and 32 letters `a` to `p`.
    pub id: String,
    /// The token it was made for.
    pub token: String,
    /// The bytes' SHA-256, hex.
    pub sha256: String,
    /// Their size.
    pub size: u64,
    /// Their MIME type.
    pub mime: String,
    /// The file's name.
    pub file_name: String,
    /// What it is for: receipt, document, import (a CSV) or w9.
    pub purpose: String,
    /// When it lapses (Unix seconds).
    pub expires: i64,
}

/// Bytes received and checked, ready to file.
#[derive(Debug)]
pub struct Received {
    /// The grant they came under.
    pub grant: Grant,
    /// The bytes.
    pub bytes: Vec<u8>,
    /// The hold on them: spend it once they're filed (INVARIANT 1).
    pub claim: Claim,
}

/// A call's hold on an upload's bytes (INVARIANT 1): [`Claim::spend`]
/// once they're filed; dropped unspent, they're given back.
#[derive(Debug)]
pub struct Claim {
    uploads: Uploads,
    id: String,
    spent: bool,
}

impl Claim {
    /// The bytes are filed: the upload is used up.
    pub fn spend(mut self) {
        self.spent = true;
        self.uploads.forget(&self.id);
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        if !self.spent {
            let _held = self.uploads.locked();
            let _ = std::fs::rename(
                self.uploads.path(&self.id, TAKEN),
                self.uploads.path(&self.id, "bin"),
            );
        }
    }
}

/// Why an upload is refused. Never names a path (INVARIANT 4).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UploadError {
    /// No such grant, or it lapsed.
    #[error("there is no upload {0:?}, or it lapsed (an upload lasts 15 minutes): begin another")]
    Unknown(String),
    /// Another token's.
    #[error("that upload was begun with another token")]
    NotYours,
    /// Its bytes came already.
    #[error("that upload's bytes were sent already")]
    AlreadySent,
    /// Another request is sending its bytes, or another call is filing
    /// them.
    #[error("that upload is in use by another request: wait for it to finish")]
    InUse,
    /// Its bytes haven't come.
    #[error(
        "that upload's bytes haven't been sent yet: send them with the command books.begin_upload gave"
    )]
    NotSent,
    /// Not the size declared.
    #[error("the bytes sent were {got} long, not the {want} declared: nothing was kept")]
    WrongSize {
        /// Declared.
        want: u64,
        /// Sent.
        got: u64,
    },
    /// More than the size declared.
    #[error("more than the {want} bytes declared were sent: nothing was kept")]
    TooMuch {
        /// Declared.
        want: u64,
    },
    /// Not the SHA-256 declared.
    #[error("the bytes sent don't have the SHA-256 declared: nothing was kept")]
    WrongHash,
    /// Over the limit.
    #[error("a file is at most {MAX_UPLOAD} bytes")]
    TooLarge,
    /// A request that can't make a grant.
    #[error("{0}")]
    Bad(String),
    /// The disk refused.
    #[error("the upload couldn't be kept: {0}")]
    Io(std::io::ErrorKind),
}

impl From<std::io::Error> for UploadError {
    fn from(e: std::io::Error) -> Self {
        UploadError::Io(e.kind())
    }
}

/// A book's uploads.
#[derive(Debug, Clone)]
pub struct Uploads {
    dir: PathBuf,
    /// Held while grants are made or swept and while bytes change name
    /// (INVARIANTS 1, 2, 5): one server serves a book, so this orders
    /// every request to its uploads.
    lock: std::sync::Arc<std::sync::Mutex<()>>,
}

/// The longest file name taken, in bytes (most file systems' limit).
pub const MAX_NAME_BYTES: usize = 255;

/// Characters that change how a name shows without showing themselves
/// (Unicode's format characters: bidirectional overrides, zero widths).
pub fn is_hidden_char(c: char) -> bool {
    matches!(c,
        // Format characters (Unicode's Cf).
        '\u{00AD}' | '\u{0600}'..='\u{0605}' | '\u{061C}' | '\u{06DD}'
        | '\u{070F}' | '\u{0890}' | '\u{0891}' | '\u{08E2}' | '\u{180E}'
        | '\u{200B}'..='\u{200F}'
        | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{206F}' | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}'
        | '\u{110BD}' | '\u{110CD}' | '\u{13430}'..='\u{1343F}'
        | '\u{1BCA0}'..='\u{1BCA3}' | '\u{1D173}'..='\u{1D17A}'
        | '\u{E0001}' | '\u{E0020}'..='\u{E007F}'
        // Line and paragraph separators (Zl, Zp).
        | '\u{2028}' | '\u{2029}'
        // Characters that show as nothing: the grapheme joiner, the Hangul
        // fillers, the Khmer inherent vowels, the variation selectors.
        | '\u{034F}' | '\u{115F}' | '\u{1160}' | '\u{17B4}' | '\u{17B5}'
        | '\u{180B}'..='\u{180F}' | '\u{2800}' | '\u{3164}' | '\u{FFA0}'
        | '\u{FE00}'..='\u{FE0F}' | '\u{E0100}'..='\u{E01EF}')
}

/// Why `name` can't name a file filed in the books, if it can't: a plain
/// name of 1 to MAX_NAME_BYTES bytes, no folder in it, nothing invisible.
pub fn file_name_problem(name: &str) -> Option<String> {
    if name.trim().is_empty() {
        return Some("a file name is needed".into());
    }
    if name.len() > MAX_NAME_BYTES {
        return Some(format!("a file name is at most {MAX_NAME_BYTES} bytes"));
    }
    if name.contains(['/', '\\']) || name == "." || name == ".." {
        return Some("a file name is a name alone, with no folder in it".into());
    }
    if name.chars().any(|c| c.is_control() || is_hidden_char(c)) {
        return Some("a file name has no control or invisible characters".into());
    }
    None
}

/// Why `mime` can't be a file's type, if it can't: `type/subtype` (and
/// any parameters), visible ASCII, at most 127 bytes.
pub fn mime_problem(mime: &str) -> Option<String> {
    let ok = (3..=127).contains(&mime.len())
        && mime.bytes().all(|b| b.is_ascii_graphic() || b == b' ')
        && mime
            .split(';')
            .next()
            .and_then(|t| t.trim().split_once('/'))
            .is_some_and(|(a, b)| !a.is_empty() && !b.is_empty());
    (!ok).then(|| "a file's type is a MIME type such as application/pdf".to_string())
}

/// The bytes of an upload a call holds (INVARIANT 1).
const TAKEN: &str = "taken";

/// An upload's id: `up_` and 32 letters `a` to `p` (16 random bytes, a
/// letter a nibble). No digits, so the access log, which masks what could
/// be a TIN, keeps an id whole.
pub fn is_id(id: &str) -> bool {
    id.len() == 3 + 32
        && id.starts_with("up_")
        && id[3..].bytes().all(|b| (b'a'..=b'p').contains(&b))
}

fn new_id(raw: &[u8; 16]) -> String {
    let mut id = String::from("up_");
    for b in raw {
        id.push(char::from(b'a' + (b >> 4)));
        id.push(char::from(b'a' + (b & 0x0f)));
    }
    id
}

fn private_file(path: &Path) -> std::io::Result<std::fs::File> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)
}

impl Uploads {
    /// The uploads of the book in `book_dir` (its folder made, private, if
    /// missing), as the books open, before any call can run (INVARIANT 3):
    /// lapsed grants swept, bytes a call held given back, and any half
    /// sent thrown away.
    pub fn open(book_dir: &Path, now: i64) -> Result<Self, UploadError> {
        let dir = crate::root::private_book_folder(book_dir, UPLOADS_DIR)
            .map_err(|e| UploadError::Bad(format!("the uploads can't be kept: {e}")))?;
        let uploads = Self {
            dir,
            lock: Default::default(),
        };
        if let Ok(entries) = std::fs::read_dir(&uploads.dir) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if let Some(id) = name.strip_suffix(&format!(".{TAKEN}")) {
                    let _ = std::fs::rename(e.path(), uploads.path(id, "bin"));
                } else if name.ends_with(".part") {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        uploads.sweep(now);
        Ok(uploads)
    }

    fn locked(&self) -> std::sync::MutexGuard<'_, ()> {
        self.lock.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn path(&self, id: &str, ext: &str) -> PathBuf {
        self.dir.join(format!("{id}.{ext}"))
    }

    /// Make a grant for `token` (INVARIANT 1).
    #[allow(clippy::too_many_arguments)]
    pub fn grant(
        &self,
        token: &str,
        sha256: &str,
        size: u64,
        mime: &str,
        file_name: &str,
        purpose: &str,
        now: i64,
    ) -> Result<Grant, UploadError> {
        let sha = sha256.trim().to_ascii_lowercase();
        if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(UploadError::Bad(
                "sha256 is the file's SHA-256: 64 hex digits".into(),
            ));
        }
        if size == 0 {
            return Err(UploadError::Bad("an empty file isn't filed".into()));
        }
        if let Some(why) = file_name_problem(file_name).or_else(|| mime_problem(mime)) {
            return Err(UploadError::Bad(why));
        }
        if size > MAX_UPLOAD {
            return Err(UploadError::TooLarge);
        }
        if !PURPOSES.contains(&purpose) {
            return Err(UploadError::Bad(
                "purpose is receipt, document, import (a bank or chart CSV) or w9".into(),
            ));
        }
        let _held = self.locked();
        self.sweep_held(now);
        // INVARIANT 5.
        if self.live_for(token, now) >= MAX_LIVE {
            return Err(UploadError::Bad(format!(
                "this agent has {MAX_LIVE} uploads under way: file them, or let them lapse (15 minutes)"
            )));
        }
        let mut raw = [0u8; 16];
        getrandom::getrandom(&mut raw)
            .map_err(|_| UploadError::Bad("the system gave no randomness".into()))?;
        let grant = Grant {
            id: new_id(&raw),
            token: token.to_string(),
            sha256: sha,
            size,
            mime: mime.to_string(),
            file_name: file_name.to_string(),
            purpose: purpose.to_string(),
            expires: now + UPLOAD_TTL,
        };
        // Written aside and renamed, so a grant is read whole or not at all.
        let aside = self.path(&grant.id, "json.new");
        let mut f = private_file(&aside)?;
        f.write_all(&serde_json::to_vec(&grant).expect("a grant serializes"))?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&aside, self.path(&grant.id, "json"))?;
        Ok(grant)
    }

    /// How many grants `token` has under way at `now`.
    fn live_for(&self, token: &str, now: i64) -> usize {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return 0;
        };
        entries
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".json"))
            .filter_map(|e| std::fs::read(e.path()).ok())
            .filter_map(|b| serde_json::from_slice::<Grant>(&b).ok())
            .filter(|g| g.token == token && g.expires > now)
            .count()
    }

    /// The live grant `id`, for `token` (INVARIANT 1).
    fn live(&self, id: &str, token: &str, now: i64) -> Result<Grant, UploadError> {
        if !is_id(id) {
            return Err(UploadError::Unknown(id.chars().take(40).collect()));
        }
        let unknown = || UploadError::Unknown(id.to_string());
        let bytes = std::fs::read(self.path(id, "json")).map_err(|_| unknown())?;
        let grant: Grant = serde_json::from_slice(&bytes).map_err(|_| unknown())?;
        if grant.expires <= now {
            self.forget(id);
            return Err(unknown());
        }
        if grant.token != token {
            return Err(UploadError::NotYours);
        }
        Ok(grant)
    }

    /// Receive grant `id`'s bytes from `body`, by `token` (INVARIANTS 1,
    /// 2): kept only if exactly as declared.
    pub fn receive(
        &self,
        id: &str,
        token: &str,
        body: &mut impl Read,
        now: i64,
    ) -> Result<Grant, UploadError> {
        let grant = self.live(id, token, now)?;
        let started = std::time::Instant::now();
        let done = self.path(id, "bin");
        let part = self.path(id, "part");
        // One request at a time (INVARIANT 2): no bytes kept or held, and
        // no `.part` of another request.
        let mut out = {
            let _held = self.locked();
            if done.exists() || self.path(id, TAKEN).exists() {
                return Err(UploadError::AlreadySent);
            }
            match private_file(&part) {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    return Err(UploadError::InUse);
                }
                Err(e) => return Err(e.into()),
            }
        };
        let mut hasher = Sha256::new();
        let mut got: u64 = 0;
        let mut buf = vec![0u8; 64 * 1024];
        // Read one byte past the size declared, to know when there's more.
        let mut limited = body.take(grant.size + 1);
        let read = loop {
            let n = match limited.read(&mut buf) {
                Ok(0) => break Ok(()),
                Ok(n) => n,
                Err(e) => break Err(UploadError::from(e)),
            };
            got += n as u64;
            if got > grant.size {
                break Ok(());
            }
            hasher.update(&buf[..n]);
            if let Err(e) = out.write_all(&buf[..n]) {
                break Err(e.into());
            }
        };
        let checked = read.and_then(|()| {
            if got > grant.size {
                return Err(UploadError::TooMuch { want: grant.size });
            }
            if got != grant.size {
                return Err(UploadError::WrongSize {
                    want: grant.size,
                    got,
                });
            }
            if hex::encode(hasher.finalize()) != grant.sha256 {
                return Err(UploadError::WrongHash);
            }
            out.sync_all()?;
            Ok(())
        });
        drop(out);
        match checked {
            Ok(()) => {
                // Kept only if its grant still stands (it may have lapsed,
                // or been filed by another request's bytes, meanwhile).
                let _held = self.locked();
                let standing = std::fs::read(self.path(id, "json"))
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Grant>(&b).ok())
                    // At the time the bytes finished coming, not began.
                    .is_some_and(|g| {
                        g == grant && g.expires > now + started.elapsed().as_secs() as i64
                    });
                if !standing || self.path(id, TAKEN).exists() || done.exists() {
                    let _ = std::fs::remove_file(&part);
                    return Err(if standing {
                        UploadError::AlreadySent
                    } else {
                        UploadError::Unknown(id.to_string())
                    });
                }
                std::fs::rename(&part, &done)?;
                Ok(grant)
            }
            Err(e) => {
                let _ = std::fs::remove_file(&part);
                Err(e)
            }
        }
    }

    /// Take grant `id`'s bytes, by `token` (INVARIANTS 1, 2): held for
    /// this call alone, checked again against the SHA-256 declared, and
    /// used up only when the claim is spent.
    pub fn take(&self, id: &str, token: &str, now: i64) -> Result<Received, UploadError> {
        let grant = self.live(id, token, now)?;
        let taken = self.path(id, TAKEN);
        // The rename is the hold: of two calls, one gets it.
        {
            let _held = self.locked();
            if let Err(e) = std::fs::rename(self.path(id, "bin"), &taken) {
                return Err(match e.kind() {
                    std::io::ErrorKind::NotFound if taken.exists() => UploadError::InUse,
                    std::io::ErrorKind::NotFound => UploadError::NotSent,
                    k => UploadError::Io(k),
                });
            }
        }
        let claim = Claim {
            uploads: self.clone(),
            id: id.to_string(),
            spent: false,
        };
        let bytes = match std::fs::read(&taken) {
            Ok(b) => b,
            Err(e) => return Err(UploadError::Io(e.kind())),
        };
        if bytes.len() as u64 != grant.size || sha256_hex(&bytes) != grant.sha256 {
            // Changed since it came: never filed, never given back.
            claim.spend();
            return Err(UploadError::WrongHash);
        }
        Ok(Received {
            grant,
            bytes,
            claim,
        })
    }

    /// Throw away the bytes received for grant `id` (their record couldn't
    /// be written): the grant stands, its bytes are to be sent again.
    pub fn discard(&self, id: &str) {
        if is_id(id) {
            let _held = self.locked();
            let _ = std::fs::remove_file(self.path(id, "bin"));
        }
    }

    /// Forget grant `id` and anything kept for it.
    fn forget(&self, id: &str) {
        let _held = self.locked();
        for ext in ["json", "part", "bin", TAKEN] {
            let _ = std::fs::remove_file(self.path(id, ext));
        }
    }

    /// Sweep away every grant that lapsed by `now`, and anything left
    /// without one (INVARIANT 3).
    pub fn sweep(&self, now: i64) {
        let _held = self.locked();
        self.sweep_held(now);
    }

    /// [`Self::sweep`], the lock held.
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
                .and_then(|b| serde_json::from_slice::<Grant>(&b).ok())
                .is_none_or(|g| g.expires <= now);
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

    fn grant_for(u: &Uploads, token: &str, bytes: &[u8]) -> Grant {
        u.grant(
            token,
            &sha256_hex(bytes),
            bytes.len() as u64,
            "application/pdf",
            "r.pdf",
            "receipt",
            NOW,
        )
        .unwrap()
    }

    #[test]
    fn bytes_sent_as_declared_are_taken_once() {
        let dir = tempfile::tempdir().unwrap();
        let u = Uploads::open(dir.path(), NOW).unwrap();
        let bytes = b"%PDF-1.4 a receipt".to_vec();
        let g = grant_for(&u, "tok_a", &bytes);
        assert!(is_id(&g.id));
        assert!(!g.id[3..].bytes().any(|b| b.is_ascii_digit()), "{}", g.id);
        assert_eq!(new_id(&[0x00; 16]), format!("up_{}", "a".repeat(32)));
        assert_eq!(new_id(&[0xff; 16]), format!("up_{}", "p".repeat(32)));
        assert!(!is_id(&format!("up_{}", "0".repeat(32))));
        assert!(!is_id(&format!("up_{}", "q".repeat(32))));
        assert_eq!(g.expires, NOW + UPLOAD_TTL);
        assert_eq!(
            u.take(&g.id, "tok_a", NOW).unwrap_err(),
            UploadError::NotSent
        );
        u.receive(&g.id, "tok_a", &mut &bytes[..], NOW).unwrap();
        assert_eq!(
            u.receive(&g.id, "tok_a", &mut &bytes[..], NOW).unwrap_err(),
            UploadError::AlreadySent
        );
        assert_eq!(
            u.take(&g.id, "tok_b", NOW).unwrap_err(),
            UploadError::NotYours
        );
        let r = u.take(&g.id, "tok_a", NOW + 1).unwrap();
        assert_eq!(r.bytes, bytes);
        assert_eq!(r.grant, g);
        r.claim.spend();
        assert!(matches!(
            u.take(&g.id, "tok_a", NOW + 2).unwrap_err(),
            UploadError::Unknown(_)
        ));
        assert_eq!(
            std::fs::read_dir(dir.path().join(UPLOADS_DIR))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn anything_but_the_bytes_declared_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let u = Uploads::open(dir.path(), NOW).unwrap();
        let bytes = b"exactly these".to_vec();
        for (sent, want) in [
            (b"exactly thes".to_vec(), "short"),
            (b"exactly these!".to_vec(), "long"),
            (b"exactly thosE".to_vec(), "hash"),
        ] {
            let g = grant_for(&u, "tok_a", &bytes);
            let e = u.receive(&g.id, "tok_a", &mut &sent[..], NOW).unwrap_err();
            match want {
                "hash" => assert_eq!(e, UploadError::WrongHash),
                "long" => assert_eq!(e, UploadError::TooMuch { want: 13 }),
                _ => assert!(matches!(e, UploadError::WrongSize { .. }), "{e}"),
            }
            assert!(
                !dir.path()
                    .join(UPLOADS_DIR)
                    .join(format!("{}.bin", g.id))
                    .exists()
            );
            assert!(
                !dir.path()
                    .join(UPLOADS_DIR)
                    .join(format!("{}.part", g.id))
                    .exists()
            );
            assert_eq!(
                u.take(&g.id, "tok_a", NOW).unwrap_err(),
                UploadError::NotSent
            );
        }
        // Another token's grant takes no bytes.
        let g = grant_for(&u, "tok_a", &bytes);
        assert_eq!(
            u.receive(&g.id, "tok_b", &mut &bytes[..], NOW).unwrap_err(),
            UploadError::NotYours
        );
    }

    #[test]
    fn a_grant_lapses_and_is_swept() {
        let dir = tempfile::tempdir().unwrap();
        let u = Uploads::open(dir.path(), NOW).unwrap();
        let bytes = b"late".to_vec();
        let g = grant_for(&u, "tok_a", &bytes);
        let later = NOW + UPLOAD_TTL;
        assert!(matches!(
            u.receive(&g.id, "tok_a", &mut &bytes[..], later)
                .unwrap_err(),
            UploadError::Unknown(_)
        ));
        let g2 = grant_for(&u, "tok_a", &bytes);
        u.receive(&g2.id, "tok_a", &mut &bytes[..], NOW).unwrap();
        std::fs::write(dir.path().join(UPLOADS_DIR).join("stray"), "x").unwrap();
        u.sweep(NOW + UPLOAD_TTL - 1);
        assert!(
            dir.path()
                .join(UPLOADS_DIR)
                .join(format!("{}.bin", g2.id))
                .exists()
        );
        assert!(!dir.path().join(UPLOADS_DIR).join("stray").exists());
        u.sweep(later);
        assert_eq!(
            std::fs::read_dir(dir.path().join(UPLOADS_DIR))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn a_grant_needs_a_hash_a_size_and_a_purpose() {
        let dir = tempfile::tempdir().unwrap();
        let u = Uploads::open(dir.path(), NOW).unwrap();
        let sha = sha256_hex(b"x");
        for (s, size, purpose) in [
            ("nothex", 1, "receipt"),
            (&sha[..63], 1, "receipt"),
            (&sha, 0, "receipt"),
            (&sha, MAX_UPLOAD + 1, "receipt"),
            (&sha, 1, "anything"),
        ] {
            assert!(
                u.grant("t", s, size, "text/plain", "x", purpose, NOW)
                    .is_err(),
                "{s} {size} {purpose}"
            );
        }
        assert!(
            u.grant(
                "t",
                &sha.to_uppercase(),
                MAX_UPLOAD,
                "text/plain",
                "x",
                "import",
                NOW
            )
            .is_ok()
        );
        // An id that isn't one is no path into the folder.
        for bad in ["../../journal", "up_", "up_XYZ", ""] {
            assert!(matches!(
                u.take(bad, "t", NOW).unwrap_err(),
                UploadError::Unknown(_)
            ));
        }
    }

    #[test]
    fn bytes_are_held_by_one_call_and_given_back_unless_filed() {
        let dir = tempfile::tempdir().unwrap();
        let u = Uploads::open(dir.path(), NOW).unwrap();
        let bytes = b"%PDF-1.4 held".to_vec();
        let g = grant_for(&u, "tok_a", &bytes);
        u.receive(&g.id, "tok_a", &mut &bytes[..], NOW).unwrap();
        // Held: no other call gets them, and they can't be sent again.
        let first = u.take(&g.id, "tok_a", NOW).unwrap();
        assert_eq!(u.take(&g.id, "tok_a", NOW).unwrap_err(), UploadError::InUse);
        assert_eq!(
            u.receive(&g.id, "tok_a", &mut &bytes[..], NOW).unwrap_err(),
            UploadError::AlreadySent
        );
        // A call that failed gives them back.
        drop(first);
        let again = u.take(&g.id, "tok_a", NOW).unwrap();
        assert_eq!(again.bytes, bytes);
        // A call that panicked does too.
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _held = again;
            panic!("the call failed");
        }));
        assert!(caught.is_err());
        let filed = u.take(&g.id, "tok_a", NOW).unwrap();
        filed.claim.spend();
        assert!(matches!(
            u.take(&g.id, "tok_a", NOW).unwrap_err(),
            UploadError::Unknown(_)
        ));
        assert_eq!(
            std::fs::read_dir(dir.path().join(UPLOADS_DIR))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn opening_gives_back_what_a_stopped_server_held_and_drops_half_sends() {
        let dir = tempfile::tempdir().unwrap();
        let u = Uploads::open(dir.path(), NOW).unwrap();
        let bytes = b"%PDF-1.4 kept".to_vec();
        let g = grant_for(&u, "tok_a", &bytes);
        u.receive(&g.id, "tok_a", &mut &bytes[..], NOW).unwrap();
        let held = u.take(&g.id, "tok_a", NOW).unwrap();
        // The server stops mid-call: nothing runs the claim's drop.
        std::mem::forget(held);
        let half = grant_for(&u, "tok_a", &bytes);
        let folder = dir.path().join(UPLOADS_DIR);
        std::fs::write(folder.join(format!("{}.part", half.id)), b"%PDF").unwrap();
        // While running, a half send blocks another.
        assert_eq!(
            u.receive(&half.id, "tok_a", &mut &bytes[..], NOW)
                .unwrap_err(),
            UploadError::InUse
        );
        // A grant that lapses by the next open is swept by it.
        let lapsing = u
            .grant(
                "tok_a",
                &sha256_hex(b"z"),
                1,
                "text/plain",
                "z",
                "receipt",
                NOW + 1 - UPLOAD_TTL,
            )
            .unwrap();
        assert!(folder.join(format!("{}.json", lapsing.id)).exists());
        let u = Uploads::open(dir.path(), NOW + 1).unwrap();
        assert!(!folder.join(format!("{}.json", lapsing.id)).exists());
        assert!(!folder.join(format!("{}.part", half.id)).exists());
        u.receive(&half.id, "tok_a", &mut &bytes[..], NOW + 1)
            .unwrap();
        let back = u.take(&g.id, "tok_a", NOW + 1).unwrap();
        assert_eq!(back.bytes, bytes);
        back.claim.spend();
    }

    /// A body that, half read, has the grant forgotten (filed by another
    /// request's bytes, or swept) under it.
    struct Forgetting<'a> {
        bytes: &'a [u8],
        then: Option<Box<dyn FnOnce() + 'a>>,
    }

    impl std::io::Read for Forgetting<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if let Some(f) = self.then.take() {
                f();
            }
            self.bytes.read(buf)
        }
    }

    #[test]
    fn bytes_are_kept_only_while_their_grant_stands() {
        let dir = tempfile::tempdir().unwrap();
        let u = Uploads::open(dir.path(), NOW).unwrap();
        let bytes = b"%PDF-1.4 late".to_vec();
        let g = grant_for(&u, "tok_a", &bytes);
        let folder = dir.path().join(UPLOADS_DIR);
        let json = folder.join(format!("{}.json", g.id));
        let mut body = Forgetting {
            bytes: &bytes,
            then: Some(Box::new(|| std::fs::remove_file(&json).unwrap())),
        };
        let e = u.receive(&g.id, "tok_a", &mut body, NOW).unwrap_err();
        assert!(matches!(e, UploadError::Unknown(_)), "{e}");
        assert_eq!(
            std::fs::read_dir(&folder).unwrap().count(),
            0,
            "nothing kept"
        );
        // Bytes whose record couldn't be written are thrown away; the
        // grant stands, for them to be sent again.
        let g = grant_for(&u, "tok_a", &bytes);
        u.receive(&g.id, "tok_a", &mut &bytes[..], NOW).unwrap();
        u.discard(&g.id);
        assert_eq!(
            u.take(&g.id, "tok_a", NOW).unwrap_err(),
            UploadError::NotSent
        );
        u.receive(&g.id, "tok_a", &mut &bytes[..], NOW).unwrap();
        u.discard("../../journal");
        assert_eq!(u.take(&g.id, "tok_a", NOW).unwrap().bytes, bytes);
    }

    /// A body that takes its time.
    struct Slow<'a>(&'a [u8], std::time::Duration);

    impl std::io::Read for Slow<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            std::thread::sleep(std::mem::take(&mut self.1));
            self.0.read(buf)
        }
    }

    #[test]
    fn bytes_that_finish_coming_after_their_grant_lapsed_are_not_kept() {
        let dir = tempfile::tempdir().unwrap();
        let u = Uploads::open(dir.path(), NOW).unwrap();
        let bytes = b"%PDF-1.4 slow".to_vec();
        // It lapses a second after the send begins; the send takes longer.
        let g = u
            .grant(
                "tok_a",
                &sha256_hex(&bytes),
                bytes.len() as u64,
                "application/pdf",
                "r.pdf",
                "receipt",
                NOW + 1 - UPLOAD_TTL,
            )
            .unwrap();
        let mut body = Slow(&bytes, std::time::Duration::from_millis(1100));
        let e = u.receive(&g.id, "tok_a", &mut body, NOW).unwrap_err();
        assert!(matches!(e, UploadError::Unknown(_)), "{e}");
        let folder = dir.path().join(UPLOADS_DIR);
        assert!(!folder.join(format!("{}.bin", g.id)).exists());
        assert!(!folder.join(format!("{}.part", g.id)).exists());
    }

    #[test]
    fn a_name_is_plain_and_visible() {
        for ok in ["r.pdf", "Résumé 2026 (final).pdf", "收据.png", "a"] {
            assert_eq!(file_name_problem(ok), None, "{ok}");
        }
        for bad in [
            "",
            " ",
            ".",
            "..",
            "a/b",
            "a\\b",
            "a\nb",
            "a\u{202E}b",
            "a\u{200B}b",
            "a\u{E0041}b",
            "a\u{2028}b",
            "a\u{FE0F}b",
            "a\u{3164}b",
            "a\u{2800}b",
            "a\u{0890}b",
        ] {
            assert!(file_name_problem(bad).is_some(), "{bad:?}");
        }
        assert_eq!(file_name_problem(&"a".repeat(MAX_NAME_BYTES)), None);
        assert!(file_name_problem(&"a".repeat(MAX_NAME_BYTES + 1)).is_some());
        for ok in ["application/pdf", "text/plain; charset=utf-8", "a/b"] {
            assert_eq!(mime_problem(ok), None, "{ok}");
        }
        let long = format!("a/{}", "b".repeat(125));
        assert_eq!(mime_problem(&long), None);
        for bad in [
            "pdf",
            "/pdf",
            "a/",
            "a/b\r\nX: y",
            "a/é",
            &format!("{long}b"),
        ] {
            assert!(mime_problem(bad).is_some(), "{bad:?}");
        }
    }

    #[test]
    fn bytes_changed_after_they_came_are_never_filed() {
        let dir = tempfile::tempdir().unwrap();
        let u = Uploads::open(dir.path(), NOW).unwrap();
        let bytes = b"%PDF-1.4 as sent".to_vec();
        let g = grant_for(&u, "tok_a", &bytes);
        u.receive(&g.id, "tok_a", &mut &bytes[..], NOW).unwrap();
        let bin = dir.path().join(UPLOADS_DIR).join(format!("{}.bin", g.id));
        std::fs::write(&bin, b"%PDF-1.4 as kept").unwrap();
        assert_eq!(
            u.take(&g.id, "tok_a", NOW).unwrap_err(),
            UploadError::WrongHash
        );
        assert!(matches!(
            u.take(&g.id, "tok_a", NOW).unwrap_err(),
            UploadError::Unknown(_)
        ));
    }

    #[test]
    fn making_a_grant_sweeps_the_lapsed() {
        let dir = tempfile::tempdir().unwrap();
        let u = Uploads::open(dir.path(), NOW).unwrap();
        let old = grant_for(&u, "tok_a", b"old");
        u.receive(&old.id, "tok_a", &mut &b"old"[..], NOW).unwrap();
        u.grant(
            "tok_a",
            &sha256_hex(b"new"),
            3,
            "text/plain",
            "x",
            "receipt",
            NOW + UPLOAD_TTL,
        )
        .unwrap();
        let names: Vec<String> = std::fs::read_dir(dir.path().join(UPLOADS_DIR))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(names.iter().all(|n| !n.starts_with(&old.id)), "{names:?}");
        assert_eq!(names.len(), 1);
    }

    #[test]
    fn a_token_has_only_so_many_uploads_under_way() {
        let dir = tempfile::tempdir().unwrap();
        let u = Uploads::open(dir.path(), NOW).unwrap();
        let sha = sha256_hex(b"x");
        for _ in 0..MAX_LIVE {
            u.grant("tok_a", &sha, 1, "text/plain", "x", "w9", NOW)
                .unwrap();
        }
        let e = u
            .grant("tok_a", &sha, 1, "text/plain", "x", "receipt", NOW)
            .unwrap_err();
        assert!(matches!(e, UploadError::Bad(_)), "{e}");
        // Another token's are its own; lapsed ones don't count.
        u.grant("tok_b", &sha, 1, "text/plain", "x", "receipt", NOW)
            .unwrap();
        u.grant(
            "tok_a",
            &sha,
            1,
            "text/plain",
            "x",
            "receipt",
            NOW + UPLOAD_TTL,
        )
        .unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn its_folder_and_files_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let u = Uploads::open(dir.path(), NOW).unwrap();
        let g = grant_for(&u, "t", b"abc");
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir.path().join(UPLOADS_DIR)), 0o700);
        assert_eq!(
            mode(&dir.path().join(UPLOADS_DIR).join(format!("{}.json", g.id))),
            0o600
        );
        // The bytes, kept and held, are private too.
        let folder = dir.path().join(UPLOADS_DIR);
        u.receive(&g.id, "t", &mut &b"abc"[..], NOW).unwrap();
        assert_eq!(mode(&folder.join(format!("{}.bin", g.id))), 0o600);
        let held = u.take(&g.id, "t", NOW).unwrap();
        assert_eq!(mode(&folder.join(format!("{}.{TAKEN}", g.id))), 0o600);
        drop(held);
    }
}
