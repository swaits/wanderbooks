//! The app's identity at the relay: its key, its four-word name and the
//! secret label of its address.
//!
//! ```text
//! GOAL: an installation has one durable public address,
//!       `https://<secret>.<four-words>.wanderbooks.link`; its key never
//!       leaves the machine; and a person can have a new address, after
//!       which the old one stops working.
//! SOURCE: WB-link-1 (the identity), plan/p15-app-and-relay.md section 5
//!         ("Names": one name per installation, each company a path under
//!         it; the secret label stops listing and guessing), D-82, D-85,
//!         D-87; relay `names.rs` (a secret label is 25 to 63 characters
//!         of a-z and 0-9; the app makes it from at least 128 random bits).
//! INVARIANTS:
//!   1. The name is four words from the relay's list, picked here at
//!      random and kept: the same installation has the same name until it
//!      is asked for a new address after the relay lost it.
//!   2. The secret label is 32 random characters of a-z and 0-9 (about
//!      165 bits; the relay needs 128 at least), made by rejection
//!      sampling with no modulo bias, never from the name, a time or a
//!      counter.
//!   3. The key (Ed25519, PKCS#8) and the label are in the data root's
//!      private `link/` folder, owner-only files (private.rs); neither is
//!      ever sent to the relay (the relay is sent the public key) or
//!      written to a log, an error or a status.
//!   4. A new label replaces the old one in one atomic write: the old
//!      address stops being served the moment this returns.
//!   5. A file that is damaged is not guessed at: it is an error in
//!      words, and nothing is overwritten.
//! SCENARIOS:
//!   1. The first call makes a name, a label and a key; the second
//!      reads the same ones.
//!   2. A new label differs and is persisted; the name and key stay.
//!   3. A reset makes a new key and a new name too.
//! ```

use crate::private;
use relay::names::{Name, SECRET_LEN};
use relay::proto::AppKey;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

/// The length of the secret label this app makes.
pub const LABEL_LEN: usize = 32;

const FILE: &str = "identity.json";
const KEY_FILE: &str = "line.key";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    v: u8,
    name: String,
    label: String,
    /// True once the relay said the name is ours.
    claimed: bool,
}

/// Why the identity could not be read or kept.
#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    /// The folder could not be read or written.
    #[error("Wanderbooks can't keep its web address on this computer ({0})")]
    Io(std::io::ErrorKind),
    /// A file is not what it should be.
    #[error("the file that keeps the web address is damaged")]
    Damaged,
}

/// One installation's identity.
pub struct Identity {
    dir: PathBuf,
    name: Name,
    label: String,
    claimed: bool,
    key: AppKey,
    pkcs8: Zeroizing<Vec<u8>>,
}

fn base36(n: usize) -> String {
    let rng = SystemRandom::new();
    let mut out = String::with_capacity(n);
    while out.len() < n {
        let mut b = [0u8; 64];
        if rng.fill(&mut b).is_err() {
            // The system generator failing is not recoverable.
            panic!("the system random number generator failed");
        }
        for byte in b {
            // 252 is the largest multiple of 36 that fits in a byte:
            // anything above is dropped, so no character is likelier.
            if byte < 252 && out.len() < n {
                let v = byte % 36;
                out.push(char::from(if v < 10 { b'0' + v } else { b'a' + v - 10 }));
            }
        }
    }
    out
}

/// A new secret label.
pub fn new_label() -> String {
    base36(LABEL_LEN)
}

fn label_ok(label: &str) -> bool {
    SECRET_LEN.contains(&label.len())
        && label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

impl Identity {
    /// Read the identity kept in `dir`, or make one (INVARIANT 1).
    pub fn load_or_create(dir: &Path) -> Result<Self, IdentityError> {
        private::make_dir(dir).map_err(|e| IdentityError::Io(e.kind()))?;
        match std::fs::read(dir.join(FILE)) {
            Ok(bytes) => Self::read(dir, &bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::make(dir),
            Err(e) => Err(IdentityError::Io(e.kind())),
        }
    }

    /// The identity kept in `dir`, if there is one.
    pub fn load(dir: &Path) -> Result<Option<Self>, IdentityError> {
        match std::fs::read(dir.join(FILE)) {
            Ok(bytes) => Self::read(dir, &bytes).map(Some),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(IdentityError::Io(e.kind())),
        }
    }

    fn read(dir: &Path, bytes: &[u8]) -> Result<Self, IdentityError> {
        let stored: Stored = serde_json::from_slice(bytes).map_err(|_| IdentityError::Damaged)?;
        let name = Name::parse(&stored.name).map_err(|_| IdentityError::Damaged)?;
        if stored.v != 1 || !label_ok(&stored.label) {
            return Err(IdentityError::Damaged);
        }
        let pkcs8 = Zeroizing::new(
            std::fs::read(dir.join(KEY_FILE)).map_err(|e| IdentityError::Io(e.kind()))?,
        );
        let key = AppKey::from_pkcs8(&pkcs8).ok_or(IdentityError::Damaged)?;
        Ok(Identity {
            dir: dir.to_path_buf(),
            name,
            label: stored.label,
            claimed: stored.claimed,
            key,
            pkcs8,
        })
    }

    fn make(dir: &Path) -> Result<Self, IdentityError> {
        let (key, pkcs8) = AppKey::generate();
        let me = Identity {
            dir: dir.to_path_buf(),
            name: Name::random(),
            label: new_label(),
            claimed: false,
            key,
            pkcs8: Zeroizing::new(pkcs8),
        };
        // The key first: a name with no key is damaged; a key with no
        // name is just unused.
        private::write(&dir.join(KEY_FILE), &me.pkcs8).map_err(|e| IdentityError::Io(e.kind()))?;
        me.save()?;
        Ok(me)
    }

    fn save(&self) -> Result<(), IdentityError> {
        let stored = Stored {
            v: 1,
            name: self.name.to_string(),
            label: self.label.clone(),
            claimed: self.claimed,
        };
        let text = serde_json::to_vec_pretty(&stored).map_err(|_| IdentityError::Damaged)?;
        private::write(&self.dir.join(FILE), &text).map_err(|e| IdentityError::Io(e.kind()))
    }

    /// The four-word name.
    pub fn name(&self) -> Name {
        self.name
    }

    /// The address: `https://<secret>.<four-words>.<domain>`.
    pub fn address(&self, domain: &str) -> String {
        format!("https://{}", self.host(domain))
    }

    /// The host name of the address.
    pub fn host(&self, domain: &str) -> String {
        format!("{}.{}.{domain}", self.label, self.name)
    }

    /// The key, to prove the name to the relay.
    pub fn key(&self) -> &AppKey {
        &self.key
    }

    /// A second handle on the same key (the line signs with it while the
    /// identity stays behind its lock).
    pub fn key_copy(&self) -> AppKey {
        AppKey::from_pkcs8(&self.pkcs8)
            .unwrap_or_else(|| unreachable!("a key that parsed parses again"))
    }

    /// Whether the relay has said the name is ours.
    pub fn claimed(&self) -> bool {
        self.claimed
    }

    /// Keep that the relay said the name is ours.
    pub fn set_claimed(&mut self) -> Result<(), IdentityError> {
        if !self.claimed {
            self.claimed = true;
            self.save()?;
        }
        Ok(())
    }

    /// Another name, for a name somebody else holds (only while this one
    /// is not claimed).
    pub fn retry_name(&mut self) -> Result<(), IdentityError> {
        self.name = Name::random();
        self.save()
    }

    /// A new secret label: the old address stops being served
    /// (INVARIANT 4).
    pub fn new_label(&mut self) -> Result<(), IdentityError> {
        self.label = new_label();
        self.save()
    }

    /// A whole new identity: key, name and label (the relay no longer
    /// knows this one).
    pub fn reset(dir: &Path) -> Result<Self, IdentityError> {
        Self::make(dir)
    }
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the key, never the label (INVARIANT 3).
        f.debug_struct("Identity")
            .field("name", &self.name.to_string())
            .field("claimed", &self.claimed)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_call_makes_and_the_second_reads_the_same() {
        let dir = tempfile::tempdir().unwrap();
        let a = Identity::load_or_create(dir.path()).unwrap();
        let b = Identity::load_or_create(dir.path()).unwrap();
        assert_eq!(a.name(), b.name());
        assert_eq!(a.host("wanderbooks.link"), b.host("wanderbooks.link"));
        assert_eq!(a.key().public_key(), b.key().public_key());
        assert!(!a.claimed());
        assert!(a.address("wanderbooks.link").starts_with("https://"));
    }

    #[test]
    fn labels_are_long_enough_and_use_only_the_relays_characters() {
        for _ in 0..200 {
            let l = new_label();
            assert_eq!(l.len(), LABEL_LEN);
            assert!(label_ok(&l));
        }
        // 32 characters of base 36 are far over 128 bits.
        assert!(32.0 * 36f64.log2() > 128.0);
        // Every character is reached: no modulo bias hides one.
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..400 {
            seen.extend(new_label().chars());
        }
        assert_eq!(seen.len(), 36);
        assert_ne!(new_label(), new_label());
    }

    #[test]
    fn a_new_label_replaces_the_old_and_a_reset_replaces_everything() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = Identity::load_or_create(dir.path()).unwrap();
        let (name, host, key) = (a.name(), a.host("x.link"), a.key().public_key());
        a.set_claimed().unwrap();
        a.new_label().unwrap();
        assert_ne!(a.host("x.link"), host);
        let b = Identity::load_or_create(dir.path()).unwrap();
        assert_eq!(b.host("x.link"), a.host("x.link"));
        assert_eq!(b.name(), name);
        assert_eq!(b.key().public_key(), key);
        assert!(b.claimed());
        let c = Identity::reset(dir.path()).unwrap();
        assert_ne!(c.key().public_key(), key);
        assert!(!c.claimed());
    }

    #[test]
    fn a_damaged_file_is_an_error_not_a_new_identity() {
        let dir = tempfile::tempdir().unwrap();
        Identity::load_or_create(dir.path()).unwrap();
        std::fs::write(dir.path().join(FILE), b"{ not json").unwrap();
        assert!(matches!(
            Identity::load_or_create(dir.path()),
            Err(IdentityError::Damaged)
        ));
        // Nothing was overwritten.
        assert_eq!(std::fs::read(dir.path().join(FILE)).unwrap(), b"{ not json");
    }

    #[test]
    fn debug_shows_neither_key_nor_label() {
        let dir = tempfile::tempdir().unwrap();
        let a = Identity::load_or_create(dir.path()).unwrap();
        let shown = format!("{a:?}");
        let label = a.host("x.link");
        let label = label.split('.').next().unwrap();
        assert!(!shown.contains(label));
    }
}
