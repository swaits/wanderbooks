//! The blob store: content-addressed, immutable bytes.
//!
//! Files are keyed by the SHA-256 of their exact bytes and laid out sharded
//! by hash prefix: `<root>/<aa>/<bb>/<full-hex>`. Reads re-hash and compare,
//! so corruption is detected, never served. Writes are atomic (temp file +
//! rename) and fsync'd. Blobs are never modified or deleted by the
//! application.

use crate::error::StoreError;
use crate::journal::sha256_hex;
use std::fs;
use std::path::{Path, PathBuf};

/// Content-addressed file store rooted at `root`.
pub struct BlobStore {
    root: PathBuf,
}

impl BlobStore {
    /// Open (creating) the blob store at `root`.
    pub fn open(root: &Path) -> Result<Self, StoreError> {
        fs::create_dir_all(root)?;
        Ok(Self {
            root: root.to_path_buf(),
        })
    }

    /// Open an existing blob store for reading, creating nothing. A
    /// missing directory just means no blobs yet.
    pub fn open_read_only(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    /// Where a blob lives, or `None` for anything that isn't a blob id.
    ///
    /// ```text
    /// GOAL: map a blob id to its file, and nothing else to anything.
    /// SOURCE: docs/receipt-store.md (content-addressed by SHA-256); the
    ///         max-testing pass (2026-10-13): ids come from the journal,
    ///         untrusted on disk, and a short one panicked.
    /// INVARIANTS:
    ///   1. NEVER a path outside the blob directory, whatever the input.
    ///   2. ONLY a 64-hex-digit id (a SHA-256) names a blob.
    /// ```
    fn path_for(&self, sha_hex: &str) -> Option<PathBuf> {
        let valid = sha_hex.len() == 64 && sha_hex.bytes().all(|b| b.is_ascii_hexdigit());
        valid.then(|| {
            self.root
                .join(&sha_hex[0..2])
                .join(&sha_hex[2..4])
                .join(sha_hex)
        })
    }

    /// Store bytes. Returns `(sha256_hex, deduplicated)`. Identical bytes
    /// store once: the second put is a no-op that reports `deduplicated`.
    pub fn put(&self, bytes: &[u8]) -> Result<(String, bool), StoreError> {
        let sha = sha256_hex(bytes);
        let path = self.path_for(&sha).expect("a SHA-256 is a blob id");
        if path.exists() {
            return Ok((sha, true));
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        // Atomic write: temp file in the same directory, fsync, rename.
        let tmp = path.with_extension("tmp");
        {
            let mut f = fs::File::create(&tmp)?;
            use std::io::Write;
            f.write_all(bytes)?;
            f.sync_all()?;
        }
        fs::rename(&tmp, &path)?;
        // Fsync the directory so the rename itself is durable. Unix only:
        // Windows can't open a directory as a file, and NTFS journals the
        // rename's metadata itself.
        #[cfg(unix)]
        {
            let dir = fs::File::open(path.parent().unwrap())?;
            dir.sync_all()?;
        }
        Ok((sha, false))
    }

    /// Read bytes, verifying the SHA-256. A mismatch is corruption, and it
    /// is reported — never served.
    pub fn get(&self, sha_hex: &str) -> Result<Vec<u8>, StoreError> {
        let sha_hex = sha_hex.to_lowercase();
        let path = self
            .path_for(&sha_hex)
            .ok_or_else(|| StoreError::CorruptBlob(sha_hex.clone()))?;
        let bytes = fs::read(path)?;
        let actual = sha256_hex(&bytes);
        if actual != sha_hex {
            return Err(StoreError::CorruptBlob(sha_hex));
        }
        Ok(bytes)
    }

    /// True when the blob exists (content not verified; use [`Self::get`]
    /// for a verifying read).
    pub fn exists(&self, sha_hex: &str) -> bool {
        self.path_for(&sha_hex.to_lowercase())
            .is_some_and(|p| p.exists())
    }
}
