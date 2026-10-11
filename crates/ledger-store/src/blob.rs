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

    fn path_for(&self, sha_hex: &str) -> PathBuf {
        self.root
            .join(&sha_hex[0..2])
            .join(&sha_hex[2..4])
            .join(sha_hex)
    }

    /// Store bytes. Returns `(sha256_hex, deduplicated)`. Identical bytes
    /// store once: the second put is a no-op that reports `deduplicated`.
    pub fn put(&self, bytes: &[u8]) -> Result<(String, bool), StoreError> {
        let sha = sha256_hex(bytes);
        let path = self.path_for(&sha);
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
        // Fsync the directory so the rename itself is durable.
        let dir = fs::File::open(path.parent().unwrap())?;
        dir.sync_all()?;
        Ok((sha, false))
    }

    /// Read bytes, verifying the SHA-256. A mismatch is corruption, and it
    /// is reported — never served.
    pub fn get(&self, sha_hex: &str) -> Result<Vec<u8>, StoreError> {
        let sha_hex = sha_hex.to_lowercase();
        let bytes = fs::read(self.path_for(&sha_hex))?;
        let actual = sha256_hex(&bytes);
        if actual != sha_hex {
            return Err(StoreError::CorruptBlob(sha_hex));
        }
        Ok(bytes)
    }

    /// True when the blob exists (content not verified; use [`Self::get`]
    /// for a verifying read).
    pub fn exists(&self, sha_hex: &str) -> bool {
        self.path_for(&sha_hex.to_lowercase()).exists()
    }
}
