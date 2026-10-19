//! The owner passphrase (plan 3.3): the human factor that the server's
//! human pages and the viewer's consents ask for, kept as an Argon2id
//! hash in the root's `auth/owner.json`.
//!
//! ```text
//! GOAL: only someone who knows the passphrase gets past a check of it,
//!       and its text is never kept.
//! SOURCE: plan 3.3 ("the owner passphrase (Argon2id, RFC 9106, m=64 MiB,
//!         t=3, p=1)"), 3.9 (consents carry it), NIST SP 800-63B 3.1.1.2
//!         (a length floor, no composition rules, any characters).
//! INVARIANTS:
//!   1. The file holds an Argon2id PHC string (its own random salt) and
//!      when it was set; never the passphrase.
//!   2. A passphrase is 12 to 1024 characters, any characters; nothing
//!      else is asked of it.
//!   3. Changing one that is set needs the one in force; setting the
//!      first needs none. The file is replaced whole (written aside,
//!      synced, renamed), so a reader sees the old or the new, never part.
//!   4. A check uses the stored hash's own parameters, and refuses a hash
//!      weaker than the plan's, of another Argon2 version, with a short
//!      salt or output, or asking more than a check can afford (a record
//!      changed by hand).
//!   5. Nothing here names a path or repeats a passphrase.
//! ```

use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use argon2::{Algorithm, Argon2, Params, Version};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// The owner passphrase's file, in the root's `auth/`.
pub const OWNER_FILE: &str = "owner.json";
/// The shortest passphrase taken (INVARIANT 2).
pub const MIN_CHARS: usize = 12;
/// The longest.
pub const MAX_CHARS: usize = 1024;
/// Argon2id's memory, KiB (plan 3.3: 64 MiB).
pub const M_COST: u32 = 64 * 1024;
/// Its passes (plan 3.3).
pub const T_COST: u32 = 3;
/// Its lanes (plan 3.3).
pub const P_COST: u32 = 1;
/// The most memory a stored hash may ask a check for, KiB (1 GiB).
const MAX_M_COST: u32 = 1024 * 1024;
/// The most passes.
const MAX_T_COST: u32 = 16;
/// The most lanes.
const MAX_P_COST: u32 = 8;
/// The shortest salt taken, in bytes.
const MIN_SALT: usize = 16;

/// Why a passphrase can't be set or checked. Never names a path, never
/// repeats a passphrase.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PassError {
    /// Too short or too long (INVARIANT 2).
    #[error("a passphrase is {MIN_CHARS} to {MAX_CHARS} characters long")]
    Length,
    /// No passphrase is set yet.
    #[error("no owner passphrase is set yet: set one with `wanderbooks passphrase`")]
    NotSet,
    /// Changing it needs the one in force, which wasn't given or didn't
    /// match.
    #[error("that isn't the owner passphrase in force")]
    Wrong,
    /// The record can't be read, or was changed by hand.
    #[error("the owner passphrase's record can't be used: {0}")]
    Record(String),
    /// The disk refused.
    #[error("the owner passphrase couldn't be kept: {0}")]
    Io(std::io::ErrorKind),
}

impl From<std::io::Error> for PassError {
    fn from(e: std::io::Error) -> Self {
        PassError::Io(e.kind())
    }
}

#[derive(Serialize, Deserialize)]
struct Owner {
    argon2id: String,
    set: String,
}

fn argon2() -> Argon2<'static> {
    Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(M_COST, T_COST, P_COST, None).expect("the plan's parameters are valid"),
    )
}

/// Check a passphrase's length (INVARIANT 2).
pub fn check_length(passphrase: &str) -> Result<(), PassError> {
    if (MIN_CHARS..=MAX_CHARS).contains(&passphrase.chars().count()) {
        Ok(())
    } else {
        Err(PassError::Length)
    }
}

fn read(auth_dir: &Path) -> Result<Option<Owner>, PassError> {
    match std::fs::read(auth_dir.join(OWNER_FILE)) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| PassError::Record("it isn't what wanderbooks writes".into())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Whether a passphrase is set.
pub fn is_set(auth_dir: &Path) -> Result<bool, PassError> {
    Ok(read(auth_dir)?.is_some())
}

/// Whether `given` is the passphrase in force (INVARIANT 4).
pub fn verify(auth_dir: &Path, given: &str) -> Result<bool, PassError> {
    let owner = read(auth_dir)?.ok_or(PassError::NotSet)?;
    let hash = PasswordHash::new(&owner.argon2id)
        .map_err(|_| PassError::Record("its hash can't be read".into()))?;
    let params =
        Params::try_from(&hash).map_err(|_| PassError::Record("its hash can't be read".into()))?;
    // Nothing weaker than the plan's, and nothing a check couldn't afford
    // (a record changed by hand to ask for terabytes): INVARIANT 4.
    let version = hash.version.unwrap_or(0);
    let salt = hash.salt.as_ref().map_or(0, |s| s.len());
    let out = hash.hash.as_ref().map_or(0, |h| h.len());
    if hash.algorithm.as_str() != "argon2id"
        || version != 0x13
        || !(M_COST..=MAX_M_COST).contains(&params.m_cost())
        || !(T_COST..=MAX_T_COST).contains(&params.t_cost())
        || !(P_COST..=MAX_P_COST).contains(&params.p_cost())
        || salt < MIN_SALT
        || out != 32
    {
        return Err(PassError::Record("it was changed by hand".into()));
    }
    Ok(argon2().verify_password(given.as_bytes(), &hash).is_ok())
}

/// Set the passphrase (INVARIANTS 1, 3): the first one with `current`
/// None; a new one only with the one in force.
pub fn set(auth_dir: &Path, new: &str, current: Option<&str>) -> Result<(), PassError> {
    check_length(new)?;
    if is_set(auth_dir)? {
        match current {
            Some(c) if verify(auth_dir, c)? => {}
            _ => return Err(PassError::Wrong),
        }
    }
    let hash = argon2()
        .hash_password(new.as_bytes())
        .map_err(|_| PassError::Record("it couldn't be hashed".into()))?
        .to_string();
    let owner = Owner {
        argon2id: hash,
        set: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    };
    let mut text = serde_json::to_string_pretty(&owner).expect("the record serializes");
    text.push('\n');
    replace(auth_dir, OWNER_FILE, text.as_bytes())
}

/// Replace a file in `dir` whole (INVARIANT 3): written aside, 0600,
/// synced, renamed over, the folder synced.
fn replace(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), PassError> {
    use std::io::Write;
    let aside = dir.join(format!(".{name}.new"));
    match std::fs::remove_file(&aside) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&aside)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&aside, dir.join(name))?;
    #[cfg(unix)]
    std::fs::File::open(dir)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIRST: &str = "correct horse battery";
    const SECOND: &str = "staple ünïcode ✓ phrase";

    #[test]
    fn lengths_are_the_only_rule() {
        assert_eq!(check_length("short"), Err(PassError::Length));
        assert_eq!(check_length(&"a".repeat(11)), Err(PassError::Length));
        assert!(check_length(&"a".repeat(12)).is_ok());
        assert!(
            check_length("ééééééééééé✓").is_ok(),
            "characters, not bytes"
        );
        assert!(check_length(&"a".repeat(1024)).is_ok());
        assert_eq!(check_length(&"a".repeat(1025)), Err(PassError::Length));
    }

    #[test]
    fn set_checked_and_changed_only_with_the_one_in_force() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_set(dir.path()).unwrap());
        assert_eq!(verify(dir.path(), FIRST), Err(PassError::NotSet));
        set(dir.path(), FIRST, None).unwrap();
        assert!(is_set(dir.path()).unwrap());
        assert!(verify(dir.path(), FIRST).unwrap());
        assert!(!verify(dir.path(), "correct horse batterY").unwrap());
        assert!(!verify(dir.path(), "").unwrap());
        // Changing needs the one in force.
        assert_eq!(set(dir.path(), SECOND, None), Err(PassError::Wrong));
        assert_eq!(
            set(dir.path(), SECOND, Some("not the passphrase")),
            Err(PassError::Wrong)
        );
        assert!(verify(dir.path(), FIRST).unwrap(), "unchanged");
        set(dir.path(), SECOND, Some(FIRST)).unwrap();
        assert!(verify(dir.path(), SECOND).unwrap());
        assert!(!verify(dir.path(), FIRST).unwrap());
        assert_eq!(
            set(dir.path(), "short", Some(SECOND)),
            Err(PassError::Length)
        );
        // The file holds a hash, never the passphrase, private.
        let text = std::fs::read_to_string(dir.path().join(OWNER_FILE)).unwrap();
        assert!(!text.contains("staple"));
        assert!(text.contains("$argon2id$v=19$m=65536,t=3,p=1$"), "{text}");
        assert!(!dir.path().join(".owner.json.new").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join(OWNER_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn a_weakened_or_mangled_record_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        set(dir.path(), FIRST, None).unwrap();
        let path = dir.path().join(OWNER_FILE);
        let good = std::fs::read_to_string(&path).unwrap();
        for (from, to) in [
            ("m=65536", "m=8"),
            ("m=65536", "m=2097152"),
            ("t=3", "t=1"),
            ("t=3", "t=99"),
            ("p=1", "p=64"),
            ("v=19", "v=16"),
            ("$argon2id$", "$argon2i$"),
            ("$argon2id$", "nonsense"),
        ] {
            std::fs::write(&path, good.replace(from, to)).unwrap();
            assert!(
                matches!(verify(dir.path(), FIRST), Err(PassError::Record(_))),
                "{to}"
            );
        }
        std::fs::write(&path, "not json").unwrap();
        assert!(matches!(
            verify(dir.path(), FIRST),
            Err(PassError::Record(_))
        ));
    }

    #[test]
    fn a_leftover_from_an_interrupted_change_doesnt_stop_the_next() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".owner.json.new"), "half").unwrap();
        set(dir.path(), FIRST, None).unwrap();
        assert!(verify(dir.path(), FIRST).unwrap());
    }

    #[test]
    fn errors_name_no_path_and_repeat_no_passphrase() {
        for e in [
            PassError::Length,
            PassError::NotSet,
            PassError::Wrong,
            PassError::Record("x".into()),
            PassError::Io(std::io::ErrorKind::PermissionDenied),
        ] {
            let text = e.to_string();
            assert!(!text.contains('/') && !text.contains(FIRST), "{text}");
        }
    }
}
