//! The TIN vault: full taxpayer identification numbers and W-9 files,
//! encrypted, outside the journal.
//!
//! ```text
//! GOAL: hand the CPA complete 1099 data (full TINs) without the books ever
//!       holding one: the journal is permanent and replicated in git, so a
//!       TIN written there could never be purged.
//! SOURCE: docs/parties-and-attachments.md "The TIN vault"; DECISIONS
//!         (Stephen, 2026-10-15): wanderbooks holds full TINs in an
//!         encrypted vault; no external dependencies (no OS keyring).
//! INVARIANTS:
//!   1. Records live in the vault directory (the books' `vault/`, which git
//!      ignores), each encrypted with XChaCha20-Poly1305 under a 256-bit key
//!      from a key file outside the books dir, a fresh random nonce per
//!      write.
//!   2. A record is bound to its party and kind (associated data): moved,
//!      renamed, or altered, it fails to open rather than yield another
//!      party's TIN.
//!   3. A TIN is exactly nine digits (hyphens allowed when given); an ITIN
//!      starts with 9 with fourth and fifth digits in 50-65, 70-88, 90-92,
//!      or 94-99, and an SSN or EIN doesn't start with 9 (IRS Pub. 1915;
//!      docs/spec/sources/irs-tin-formats.md).
//!   4. The vault never writes a TIN or a key anywhere but its own files;
//!      errors never contain one.
//!   5. `record_hash` is the SHA-256 of a record's bytes on disk, so the
//!      journal can prove a record existed without holding it.
//!   6. Purging deletes the record's file; nothing else keeps a copy.
//! SCENARIOS:
//!   1. IF the key file is missing: `open` creates one (mode 0600) only when
//!      asked to (`create_key`), else the vault is locked (Error::NoKey).
//!   2. IF a record is missing: `tin` returns None.
//!   3. IF a record fails to decrypt (wrong key, tampered, moved): SHOULD
//!      refuse with Error::Unreadable, naming the party, never guessing.
//! ```

use base64::Engine as _;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use thiserror::Error;
use zeroize::Zeroizing;

/// What can go wrong.
#[derive(Debug, Error)]
pub enum Error {
    /// The key file isn't there (the vault is locked).
    #[error("the vault key file {0} is missing; the vault is locked")]
    NoKey(PathBuf),
    /// The key file isn't a 256-bit key.
    #[error("the vault key file {0} is not a vault key (64 hex digits)")]
    BadKey(PathBuf),
    /// A TIN of the wrong form.
    #[error("bad TIN: {0}")]
    BadTin(String),
    /// A record that fails to decrypt or parse.
    #[error(
        "the vault record for party {party} ({kind}) can't be read: wrong key, altered, or moved"
    )]
    Unreadable {
        /// The party.
        party: u64,
        /// Its kind.
        kind: &'static str,
    },
    /// Filesystem trouble.
    #[error("vault io: {0}")]
    Io(#[from] std::io::Error),
}

/// A TIN's type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TinType {
    /// Social Security number (an individual).
    Ssn,
    /// Employer identification number (a business).
    Ein,
    /// IRS individual taxpayer identification number.
    Itin,
}

/// A full TIN: nine digits and its type. Its digits are wiped from memory
/// when dropped, and its Debug never prints them.
#[derive(Clone, Serialize, Deserialize)]
pub struct Tin {
    digits: Zeroizing<String>,
    /// Its type.
    pub kind: TinType,
}

impl std::fmt::Debug for Tin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Tin({:?}, last4 {})", self.kind, self.last4())
    }
}

impl Tin {
    /// A TIN from its digits (hyphens allowed), checked against its type
    /// (INVARIANT 3).
    pub fn new(text: &str, kind: TinType) -> Result<Tin, Error> {
        let digits: String = text.chars().filter(|c| *c != '-').collect();
        if digits.len() != 9 || !digits.chars().all(|c| c.is_ascii_digit()) {
            return Err(Error::BadTin("a TIN is nine digits".into()));
        }
        let starts_9 = digits.starts_with('9');
        match kind {
            TinType::Itin if !starts_9 => {
                return Err(Error::BadTin("an ITIN begins with 9".into()));
            }
            TinType::Itin => {
                let group: u8 = digits[3..5].parse().expect("two digits");
                if !matches!(group, 50..=65 | 70..=88 | 90..=92 | 94..=99) {
                    return Err(Error::BadTin(
                        "an ITIN's fourth and fifth digits are 50-65, 70-88, 90-92, or 94-99"
                            .into(),
                    ));
                }
            }
            TinType::Ssn | TinType::Ein if starts_9 => {
                return Err(Error::BadTin(
                    "a nine-digit number beginning with 9 is an ITIN, not an SSN or EIN".into(),
                ));
            }
            _ => {}
        }
        Ok(Tin {
            digits: Zeroizing::new(digits),
            kind,
        })
    }

    /// The nine digits, for the 1099 worksheet only.
    pub fn digits(&self) -> &str {
        &self.digits
    }

    /// The last four digits (what the books keep).
    pub fn last4(&self) -> &str {
        &self.digits[5..]
    }

    /// Formatted as the IRS writes it: 123-45-6789 for an SSN or ITIN,
    /// 12-3456789 for an EIN.
    pub fn formatted(&self) -> String {
        let d = self.digits();
        match self.kind {
            TinType::Ein => format!("{}-{}", &d[..2], &d[2..]),
            _ => format!("{}-{}-{}", &d[..3], &d[3..5], &d[5..]),
        }
    }
}

/// A W-9 file kept in the vault.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct W9File {
    /// Original file name.
    pub file_name: String,
    /// MIME type.
    pub mime_type: String,
    /// The bytes.
    #[serde(with = "b64")]
    pub bytes: Vec<u8>,
}

mod b64 {
    use base64::Engine as _;
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(b: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(b))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        base64::engine::general_purpose::STANDARD
            .decode(s)
            .map_err(serde::de::Error::custom)
    }
}

const MAGIC: &[u8; 4] = b"WBV1";
const NONCE_LEN: usize = 24;

/// The two kinds of record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Tin,
    W9,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Tin => "tin",
            Kind::W9 => "w9",
        }
    }
}

/// An open vault.
pub struct Vault {
    dir: PathBuf,
    cipher: XChaCha20Poly1305,
}

impl std::fmt::Debug for Vault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vault")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

/// The default key file: `$XDG_CONFIG_HOME/wanderbooks/vault.key`, else
/// `~/.config/wanderbooks/vault.key`.
pub fn default_key_file() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("wanderbooks").join("vault.key")
}

impl Vault {
    /// Open the vault in `dir` with the key in `key_file`. With
    /// `create_key`, a missing key file is created (a fresh random key,
    /// mode 0600); without, a missing one is Error::NoKey. The directory is
    /// created on first write.
    pub fn open(dir: &Path, key_file: &Path, create_key: bool) -> Result<Vault, Error> {
        let key = match std::fs::read_to_string(key_file) {
            Ok(text) => {
                let text = Zeroizing::new(text);
                let bytes = Zeroizing::new(
                    hex::decode(text.trim()).map_err(|_| Error::BadKey(key_file.to_path_buf()))?,
                );
                if bytes.len() != 32 {
                    return Err(Error::BadKey(key_file.to_path_buf()));
                }
                let mut k = Zeroizing::new([0u8; 32]);
                k.copy_from_slice(&bytes);
                k
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && create_key => {
                let mut k = Zeroizing::new([0u8; 32]);
                getrandom::getrandom(&mut k[..])
                    .map_err(|e| Error::Io(std::io::Error::other(e)))?;
                write_private(key_file, hex::encode(&k[..]).as_bytes())?;
                k
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(Error::NoKey(key_file.to_path_buf()));
            }
            Err(e) => return Err(Error::Io(e)),
        };
        let cipher = XChaCha20Poly1305::new_from_slice(&key[..]).expect("32-byte key");
        Ok(Vault {
            dir: dir.to_path_buf(),
            cipher,
        })
    }

    fn path(&self, party: u64, kind: Kind) -> PathBuf {
        self.dir.join(format!("{}-{party}.bin", kind.name()))
    }

    fn aad(party: u64, kind: Kind) -> Vec<u8> {
        format!("wanderbooks-vault-v1|{}|party:{party}", kind.name()).into_bytes()
    }

    fn seal(&self, party: u64, kind: Kind, plain: &[u8]) -> Result<String, Error> {
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::getrandom(&mut nonce).map_err(|e| Error::Io(std::io::Error::other(e)))?;
        let aad = Self::aad(party, kind);
        let ct = self
            .cipher
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: plain,
                    aad: &aad,
                },
            )
            .expect("encryption of an in-memory buffer");
        let mut out = Vec::with_capacity(4 + NONCE_LEN + ct.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        std::fs::create_dir_all(&self.dir)?;
        write_private(&self.path(party, kind), &out)?;
        Ok(hex::encode(Sha256::digest(&out)))
    }

    fn open_record(&self, party: u64, kind: Kind) -> Result<Option<Zeroizing<Vec<u8>>>, Error> {
        let bytes = match std::fs::read(self.path(party, kind)) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(Error::Io(e)),
        };
        let unreadable = Error::Unreadable {
            party,
            kind: kind.name(),
        };
        if bytes.len() < 4 + NONCE_LEN || &bytes[..4] != MAGIC {
            return Err(unreadable);
        }
        let aad = Self::aad(party, kind);
        let plain = self
            .cipher
            .decrypt(
                &XNonce::try_from(&bytes[4..4 + NONCE_LEN]).expect("length checked"),
                Payload {
                    msg: &bytes[4 + NONCE_LEN..],
                    aad: &aad,
                },
            )
            .map_err(|_| unreadable)?;
        Ok(Some(Zeroizing::new(plain)))
    }

    /// Store a party's TIN; returns the record's hash (INVARIANT 5).
    pub fn put_tin(&self, party: u64, tin: &Tin) -> Result<String, Error> {
        let plain = Zeroizing::new(serde_json::to_vec(tin).expect("serializes"));
        self.seal(party, Kind::Tin, &plain)
    }

    /// A party's TIN, if the vault holds one.
    pub fn tin(&self, party: u64) -> Result<Option<Tin>, Error> {
        match self.open_record(party, Kind::Tin)? {
            None => Ok(None),
            Some(plain) => serde_json::from_slice(&plain)
                .map(Some)
                .map_err(|_| Error::Unreadable { party, kind: "tin" }),
        }
    }

    /// Store a party's W-9 file; returns the record's hash.
    pub fn put_w9(&self, party: u64, file: &W9File) -> Result<String, Error> {
        let plain = Zeroizing::new(serde_json::to_vec(file).expect("serializes"));
        self.seal(party, Kind::W9, &plain)
    }

    /// A party's W-9 file, if the vault holds one.
    pub fn w9(&self, party: u64) -> Result<Option<W9File>, Error> {
        match self.open_record(party, Kind::W9)? {
            None => Ok(None),
            Some(plain) => serde_json::from_slice(&plain)
                .map(Some)
                .map_err(|_| Error::Unreadable { party, kind: "w9" }),
        }
    }

    /// The SHA-256 of a party's TIN record as stored, if any.
    pub fn record_hash(&self, party: u64) -> Result<Option<String>, Error> {
        match std::fs::read(self.path(party, Kind::Tin)) {
            Ok(b) => Ok(Some(hex::encode(Sha256::digest(&b)))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::Io(e)),
        }
    }

    /// Delete everything the vault holds for a party (INVARIANT 6). Returns
    /// whether anything was there.
    pub fn purge(&self, party: u64) -> Result<bool, Error> {
        let mut any = false;
        for kind in [Kind::Tin, Kind::W9] {
            match std::fs::remove_file(self.path(party, kind)) {
                Ok(()) => any = true,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(Error::Io(e)),
            }
        }
        Ok(any)
    }
}

/// Write a file readable by its owner only (0600 on Unix), creating its
/// directory.
fn write_private(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        use std::io::Write;
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Base64 of bytes (for callers handing a W-9 to a tool).
pub fn to_base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
