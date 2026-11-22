//! A backup protected by a passphrase (D-92): the key, the header that
//! lets another computer derive it, the sealed form of a piece, and the
//! key kept privately on this computer.
//!
//! The passphrase is run through Argon2id (the same function and the same
//! cost as the owner passphrase, `passphrase.rs`: 64 MiB, 3 passes, 1 lane,
//! RFC 9106) with a random salt of its own. The 32 bytes it gives (the
//! *master*) are what this computer keeps, never the passphrase. Two keys
//! come from the master, each by SHA-256 of a label and the master: the
//! one pieces are sealed with, and the one their names are made with.
//!
//! A piece is sealed in chunks of 1 MiB with XChaCha20-Poly1305 (as the TIN
//! vault is): a random 16-byte prefix per piece and an 8-byte chunk
//! counter make each chunk's 24-byte nonce; each chunk's associated data
//! is the piece's identity (what is in it) and whether it is the last, so
//! a piece cut short, reordered, or put under another's name is not
//! opened.

use super::BackupError;
use super::form::sha_hex;
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::Path;
use zeroize::Zeroizing;

/// The sealed piece's first bytes.
const MAGIC: &[u8; 4] = b"WBS1";
/// Plaintext bytes in a chunk.
const CHUNK: usize = 1 << 20;
/// What a chunk's seal adds.
const TAG: usize = 16;
/// The salt's length.
const SALT_LEN: usize = 16;
/// The words the header's check seals: opening them tells a right
/// passphrase from a wrong one.
const CHECK_TEXT: &[u8] = b"wanderbooks backup key check";
/// The local key file's folder in a book's folder, and its name.
pub(crate) const LOCAL_DIR: &str = "local";
/// The key file's name.
pub(crate) const KEY_FILE: &str = "backup-key.json";

/// How the passphrase is stretched. Recorded in the header so another
/// computer derives the same key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Kdf {
    pub alg: String,
    pub m: u32,
    pub t: u32,
    pub p: u32,
    /// Hex.
    pub salt: String,
}

/// The key a passphrase makes, with how it was made. Kept on this
/// computer in the book's `local/` folder; never in a backup, never in the
/// journal, never in a log (its `Debug` says nothing of the key).
#[derive(Clone)]
pub struct BackupKey {
    kdf: Kdf,
    master: Zeroizing<[u8; 32]>,
}

impl std::fmt::Debug for BackupKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BackupKey(..)")
    }
}

/// Whether `kdf` asks nothing weaker than this program's own cost, and
/// nothing a computer could not afford (a header changed by hand).
fn kdf_in_bounds(kdf: &Kdf) -> bool {
    kdf.alg == "argon2id"
        && (crate::passphrase::M_COST..=1024 * 1024).contains(&kdf.m)
        && (crate::passphrase::T_COST..=16).contains(&kdf.t)
        && (crate::passphrase::P_COST..=8).contains(&kdf.p)
        && hex::decode(&kdf.salt).is_ok_and(|s| (SALT_LEN..=64).contains(&s.len()))
}

fn stretch(passphrase: &str, kdf: &Kdf) -> Result<Zeroizing<[u8; 32]>, BackupError> {
    if !kdf_in_bounds(kdf) {
        return Err(BackupError::Damaged(
            "its header asks for a key this program won't derive".into(),
        ));
    }
    let salt = hex::decode(&kdf.salt).expect("checked");
    let params = Params::new(kdf.m, kdf.t, kdf.p, Some(32))
        .map_err(|_| BackupError::Damaged("its header's key settings are not valid".into()))?;
    let mut out = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(passphrase.as_bytes(), &salt, &mut *out)
        .map_err(|_| BackupError::Damaged("the key could not be derived".into()))?;
    Ok(out)
}

impl BackupKey {
    /// A new key from `passphrase` with a salt of its own.
    pub fn generate(passphrase: &str) -> Result<Self, BackupError> {
        crate::passphrase::check_length(passphrase).map_err(|_| BackupError::PassphraseLength)?;
        let mut salt = [0u8; SALT_LEN];
        getrandom::getrandom(&mut salt)
            .map_err(|e| BackupError::Io(std::io::Error::from(e).kind()))?;
        let kdf = Kdf {
            alg: "argon2id".into(),
            m: crate::passphrase::M_COST,
            t: crate::passphrase::T_COST,
            p: crate::passphrase::P_COST,
            salt: hex::encode(salt),
        };
        let master = stretch(passphrase, &kdf)?;
        Ok(Self { kdf, master })
    }

    fn label(&self, label: &str) -> Zeroizing<[u8; 32]> {
        let mut h = Sha256::new();
        h.update(b"wanderbooks backup v1 ");
        h.update(label.as_bytes());
        h.update(*self.master);
        Zeroizing::new(h.finalize().into())
    }

    /// The sealing and naming keys.
    pub(crate) fn keys(&self) -> Keys {
        Keys {
            enc: self.label("encryption"),
            names: self.label("names"),
            kdf: self.kdf.clone(),
        }
    }

    /// The same key as another: the same passphrase and salt.
    pub fn same_as(&self, other: &BackupKey) -> bool {
        use subtle::ConstantTimeEq;
        self.kdf.salt == other.kdf.salt && bool::from(self.master.ct_eq(&*other.master))
    }

    /// A short, public name for this key: what the salt it was derived
    /// with hashes to. Said in a record to tell one passphrase's backups
    /// from another's; it opens nothing.
    pub fn fingerprint(&self) -> String {
        sha_hex(self.kdf.salt.as_bytes())[..12].to_string()
    }

    /// Keep it, privately, in the book's folder (INVARIANT: owner-only).
    pub fn store(&self, book_dir: &Path) -> Result<(), BackupError> {
        let dir = crate::root::private_book_folder(book_dir, LOCAL_DIR)
            .map_err(|e| BackupError::Damaged(e.to_string()))?;
        #[derive(Serialize)]
        struct OnDisk<'a> {
            version: u32,
            kdf: &'a Kdf,
            master: String,
        }
        let text = Zeroizing::new(
            serde_json::to_string_pretty(&OnDisk {
                version: 1,
                kdf: &self.kdf,
                master: hex::encode(*self.master),
            })
            .expect("a key serializes"),
        );
        let tmp = dir.join(format!(".{KEY_FILE}.new"));
        let _ = std::fs::remove_file(&tmp);
        crate::root::write_new_private_file(&tmp, text.as_bytes())?;
        std::fs::rename(&tmp, dir.join(KEY_FILE))?;
        super::io::sync_dir(&dir);
        Ok(())
    }

    /// The key kept for the book in `book_dir`, if one is.
    pub fn load(book_dir: &Path) -> Result<Option<Self>, BackupError> {
        let path = book_dir.join(LOCAL_DIR).join(KEY_FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => Zeroizing::new(t),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        #[derive(Deserialize)]
        struct OnDisk {
            version: u32,
            kdf: Kdf,
            master: String,
        }
        let bad = || BackupError::Damaged("the backup passphrase's key can't be read".into());
        let on: OnDisk = serde_json::from_str(&text).map_err(|_| bad())?;
        if on.version != 1 || !kdf_in_bounds(&on.kdf) {
            return Err(bad());
        }
        let master_hex = Zeroizing::new(on.master);
        let raw = Zeroizing::new(hex::decode(master_hex.as_str()).map_err(|_| bad())?);
        let master: [u8; 32] = raw.as_slice().try_into().map_err(|_| bad())?;
        Ok(Some(Self {
            kdf: on.kdf,
            master: Zeroizing::new(master),
        }))
    }

    /// Forget it (the passphrase is taken off): the next backup is plain.
    /// The file is removed from this computer only.
    pub fn forget(book_dir: &Path) -> Result<(), BackupError> {
        match std::fs::remove_file(book_dir.join(LOCAL_DIR).join(KEY_FILE)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Derive the key `passphrase` makes from what a sealed set's header
    /// says: [`BackupError::WrongPassphrase`] when it opens nothing,
    /// [`BackupError::Damaged`] when the header itself is not whole.
    pub(crate) fn from_header(passphrase: &str, header: &Header) -> Result<Self, BackupError> {
        let master = stretch(passphrase, &header.kdf)?;
        let key = Self {
            kdf: header.kdf.clone(),
            master,
        };
        key.keys().check(header)?;
        Ok(key)
    }
}

/// The two keys a set uses.
pub(crate) struct Keys {
    enc: Zeroizing<[u8; 32]>,
    names: Zeroizing<[u8; 32]>,
    kdf: Kdf,
}

/// A sealed set's header: what another computer needs to derive the key,
/// and a sealed check that tells a wrong passphrase from damage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Header {
    pub format: String,
    pub version: u32,
    pub cipher: String,
    pub kdf: Kdf,
    /// Hex of the check text, sealed.
    pub check: String,
    /// SHA-256 of the sealed check, hex: it differs from the check only if
    /// the header was damaged.
    pub check_sha256: String,
}

const HEADER_FORMAT: &str = "wanderbooks-backup-header";

impl Header {
    /// The header's file: JSON, a newline, and its own check line.
    pub(crate) fn encode(&self) -> Vec<u8> {
        let body = serde_json::to_string(self).expect("a header serializes");
        format!(
            "{body}\nwanderbooks-header-sha256 {}\n",
            sha_hex(body.as_bytes())
        )
        .into_bytes()
    }

    /// A header read back, or why it is not one.
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, BackupError> {
        let bad =
            || BackupError::Damaged("the sealed backup's header is cut short or changed".into());
        let text = std::str::from_utf8(bytes).map_err(|_| bad())?;
        let text = text.strip_suffix('\n').ok_or_else(bad)?;
        let (body, check) = text.rsplit_once('\n').ok_or_else(bad)?;
        if check.strip_prefix("wanderbooks-header-sha256 ") != Some(&sha_hex(body.as_bytes())) {
            return Err(bad());
        }
        let h: Header = serde_json::from_str(body).map_err(|_| bad())?;
        if h.format != HEADER_FORMAT || h.version != 1 || h.cipher != "xchacha20poly1305-stream" {
            return Err(BackupError::Damaged(
                "the sealed backup's header is of a kind this program doesn't know".into(),
            ));
        }
        Ok(h)
    }
}

impl Keys {
    /// The header for a set sealed with these keys.
    pub(crate) fn header(&self) -> Header {
        let mut sealed = Vec::new();
        self.seal_stream(&mut &CHECK_TEXT[..], &mut sealed, b"wb1|header")
            .expect("sealing into memory");
        Header {
            format: HEADER_FORMAT.into(),
            version: 1,
            cipher: "xchacha20poly1305-stream".into(),
            kdf: self.kdf.clone(),
            check_sha256: sha_hex(&sealed),
            check: hex::encode(sealed),
        }
    }

    /// Whether these keys open `header`'s check.
    pub(crate) fn check(&self, header: &Header) -> Result<(), BackupError> {
        let sealed = hex::decode(&header.check)
            .ok()
            .filter(|s| sha_hex(s) == header.check_sha256)
            .ok_or_else(|| BackupError::Damaged("the sealed backup's header was changed".into()))?;
        let mut plain = Vec::new();
        match self.open_stream(&mut &sealed[..], b"wb1|header", &mut |b| {
            plain.extend_from_slice(b);
            Ok(())
        }) {
            Ok(()) if plain == CHECK_TEXT => Ok(()),
            _ => Err(BackupError::WrongPassphrase),
        }
    }

    /// The name a piece is kept under: made from the key, the piece's
    /// kind and its hash.
    pub(crate) fn piece_name(&self, kind: &str, sha: &str) -> String {
        let mut h = Sha256::new();
        h.update(*self.names);
        h.update(kind.as_bytes());
        h.update([0]);
        h.update(sha.as_bytes());
        hex::encode(h.finalize())
    }

    fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new_from_slice(&self.enc[..]).expect("a 32-byte key")
    }

    /// The length a piece of `plain` bytes is kept at.
    pub(crate) fn sealed_len(plain: u64) -> u64 {
        let chunks = plain.div_ceil(CHUNK as u64).max(1);
        (MAGIC.len() + 16) as u64 + plain + chunks * TAG as u64
    }

    /// Seal all of `plain` into `out`, bound to `aad` (the piece's
    /// identity).
    pub(crate) fn seal_stream(
        &self,
        plain: &mut dyn Read,
        out: &mut dyn Write,
        aad: &[u8],
    ) -> std::io::Result<()> {
        let cipher = self.cipher();
        let mut prefix = [0u8; 16];
        getrandom::getrandom(&mut prefix).map_err(std::io::Error::from)?;
        out.write_all(MAGIC)?;
        out.write_all(&prefix)?;
        let mut cur = read_full(plain, CHUNK)?;
        let mut counter = 0u64;
        loop {
            let next = read_full(plain, CHUNK)?;
            let last = next.is_empty();
            let mut nonce = [0u8; 24];
            nonce[..16].copy_from_slice(&prefix);
            nonce[16..].copy_from_slice(&counter.to_be_bytes());
            let mut a = aad.to_vec();
            a.push(u8::from(last));
            let ct = cipher
                .encrypt(&XNonce::from(nonce), Payload { msg: &cur, aad: &a })
                .map_err(|_| std::io::Error::other("sealing failed"))?;
            out.write_all(&ct)?;
            if last {
                return Ok(());
            }
            cur = next;
            counter += 1;
        }
    }

    /// Open a sealed piece, handing its contents to `sink` chunk by chunk.
    /// A wrong key, a changed byte, a piece cut short, or one bound to
    /// another identity is an error before the contents are complete; the
    /// caller discards what the sink had.
    pub(crate) fn open_stream(
        &self,
        stored: &mut dyn Read,
        aad: &[u8],
        sink: &mut dyn FnMut(&[u8]) -> std::io::Result<()>,
    ) -> Result<(), BackupError> {
        let bad =
            || BackupError::Damaged("a piece of the sealed backup is damaged or cut short".into());
        let cipher = self.cipher();
        let mut head = [0u8; 4 + 16];
        stored.read_exact(&mut head).map_err(|_| bad())?;
        if &head[..4] != MAGIC {
            return Err(bad());
        }
        let prefix: [u8; 16] = head[4..].try_into().expect("16 bytes");
        let mut cur = read_full(stored, CHUNK + TAG).map_err(super::io::uncarry)?;
        if cur.is_empty() {
            return Err(bad());
        }
        let mut counter = 0u64;
        loop {
            let next = read_full(stored, CHUNK + TAG).map_err(super::io::uncarry)?;
            let last = next.is_empty();
            let mut nonce = [0u8; 24];
            nonce[..16].copy_from_slice(&prefix);
            nonce[16..].copy_from_slice(&counter.to_be_bytes());
            let mut a = aad.to_vec();
            a.push(u8::from(last));
            let plain = cipher
                .decrypt(&XNonce::from(nonce), Payload { msg: &cur, aad: &a })
                .map_err(|_| bad())?;
            sink(&plain).map_err(super::io::uncarry)?;
            if last {
                return Ok(());
            }
            cur = next;
            counter += 1;
        }
    }

    /// A small file sealed whole.
    pub(crate) fn seal_bytes(&self, plain: &[u8], aad: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        self.seal_stream(&mut &plain[..], &mut out, aad)
            .expect("sealing into memory");
        out
    }

    /// A small file opened whole.
    pub(crate) fn open_bytes(&self, stored: &[u8], aad: &[u8]) -> Result<Vec<u8>, BackupError> {
        let mut out = Vec::new();
        self.open_stream(&mut &stored[..], aad, &mut |b| {
            out.extend_from_slice(b);
            Ok(())
        })?;
        Ok(out)
    }
}

/// Up to `n` bytes from `r`: fewer only at its end.
fn read_full(r: &mut dyn Read, n: usize) -> std::io::Result<Vec<u8>> {
    let mut buf = vec![0u8; n];
    let mut got = 0;
    while got < n {
        match r.read(&mut buf[got..]) {
            Ok(0) => break,
            Ok(k) => got += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    buf.truncate(got);
    Ok(buf)
}

/// A piece's identity: what it is bound to when sealed.
pub(crate) fn piece_aad(kind: &str, sha: &str) -> Vec<u8> {
    format!("wb1|{kind}|{sha}").into_bytes()
}

/// A manifest's identity.
pub(crate) fn manifest_aad(number: u64) -> Vec<u8> {
    format!("wb1|manifest|{number}").into_bytes()
}
