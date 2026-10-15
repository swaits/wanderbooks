//! The vault against its spec (crates/ledger-vault/src/lib.rs).

use ledger_vault::{Error, Tin, TinType, Vault, W9File};
use std::path::PathBuf;

fn dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("wb-vault-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn vault(d: &std::path::Path) -> Vault {
    Vault::open(&d.join("vault"), &d.join("keys/vault.key"), true).unwrap()
}

/// INVARIANT 3: nine digits, hyphens allowed; ITINs start with 9 and SSNs
/// and EINs don't.
#[test]
fn tins_are_nine_digits_of_their_type() {
    assert_eq!(
        Tin::new("123-45-6789", TinType::Ssn).unwrap().digits(),
        "123456789"
    );
    assert_eq!(
        Tin::new("12-3456789", TinType::Ein).unwrap().last4(),
        "6789"
    );
    // Pub. 1915: fourth and fifth digits 50-65, 70-88, 90-92, 94-99.
    for ok in [
        "900-50-0000",
        "900-65-0000",
        "900-70-0000",
        "900-88-0000",
        "900-90-0000",
        "900-92-0000",
        "900-94-0000",
        "900-99-0000",
    ] {
        assert!(Tin::new(ok, TinType::Itin).is_ok(), "{ok}");
    }
    for bad in [
        "900-49-0000",
        "900-66-0000",
        "900-69-0000",
        "900-89-0000",
        "900-93-0000",
        "900-00-0000",
    ] {
        assert!(Tin::new(bad, TinType::Itin).is_err(), "{bad}");
    }
    for (bad, kind) in [
        ("12345678", TinType::Ssn),
        ("1234567890", TinType::Ssn),
        ("12345678a", TinType::Ein),
        ("123 45 6789", TinType::Ssn),
        ("912-70-1234", TinType::Ssn),
        ("923456789", TinType::Ein),
        ("123456789", TinType::Itin),
    ] {
        assert!(Tin::new(bad, kind).is_err(), "{bad} as {kind:?}");
    }
    assert_eq!(
        Tin::new("123456789", TinType::Ssn).unwrap().formatted(),
        "123-45-6789"
    );
    assert_eq!(
        Tin::new("123456789", TinType::Ein).unwrap().formatted(),
        "12-3456789"
    );
}

/// INVARIANT 4: Debug and errors never show the digits.
#[test]
fn a_tin_never_prints() {
    let tin = Tin::new("123-45-6789", TinType::Ssn).unwrap();
    let shown = format!("{tin:?}");
    assert!(
        !shown.contains("12345") && shown.contains("6789"),
        "{shown}"
    );
    let err = Tin::new("123-45-678", TinType::Ssn)
        .unwrap_err()
        .to_string();
    assert!(!err.contains("12345678"), "{err}");
}

/// INVARIANT 1, 5: a TIN and a W-9 round-trip; the record on disk holds
/// neither in the clear; record_hash matches the bytes.
#[test]
fn records_round_trip_encrypted() {
    let d = dir("roundtrip");
    let v = vault(&d);
    let tin = Tin::new("12-3456789", TinType::Ein).unwrap();
    let hash = v.put_tin(7, &tin).unwrap();
    let back = v.tin(7).unwrap().unwrap();
    assert_eq!(back.digits(), "123456789");
    assert_eq!(back.kind, TinType::Ein);
    let raw = std::fs::read(d.join("vault/tin-7.bin")).unwrap();
    assert!(
        !raw.windows(9).any(|w| w == b"123456789"),
        "TIN in the clear"
    );
    assert_eq!(v.record_hash(7).unwrap().as_deref(), Some(hash.as_str()));
    let w9 = W9File {
        file_name: "w9.pdf".into(),
        mime_type: "application/pdf".into(),
        bytes: b"%PDF secret 123456789".to_vec(),
    };
    v.put_w9(7, &w9).unwrap();
    assert_eq!(v.w9(7).unwrap().unwrap(), w9);
    assert!(v.tin(8).unwrap().is_none(), "SCENARIO 2: missing is None");
    // A fresh nonce each write: the same TIN twice is two different records.
    let again = v.put_tin(7, &tin).unwrap();
    assert_ne!(again, hash);
}

/// INVARIANT 2 and SCENARIO 3: a record moved to another party, altered,
/// or opened with another key is unreadable, never another party's TIN.
#[test]
fn moved_altered_or_foreign_records_refuse() {
    let d = dir("tamper");
    let v = vault(&d);
    v.put_tin(1, &Tin::new("111-22-3333", TinType::Ssn).unwrap())
        .unwrap();
    std::fs::copy(d.join("vault/tin-1.bin"), d.join("vault/tin-2.bin")).unwrap();
    assert!(matches!(v.tin(2), Err(Error::Unreadable { party: 2, .. })));
    let mut raw = std::fs::read(d.join("vault/tin-1.bin")).unwrap();
    let last = raw.len() - 1;
    raw[last] ^= 1;
    std::fs::write(d.join("vault/tin-1.bin"), &raw).unwrap();
    assert!(matches!(v.tin(1), Err(Error::Unreadable { party: 1, .. })));
    let other = Vault::open(&d.join("vault"), &d.join("other.key"), true).unwrap();
    v.put_tin(3, &Tin::new("111-22-3333", TinType::Ssn).unwrap())
        .unwrap();
    assert!(matches!(other.tin(3), Err(Error::Unreadable { .. })));
}

/// SCENARIO 1: no key file and no permission to create one is a locked
/// vault; a created key file is 0600 and survives reopening.
#[test]
fn the_key_file() {
    let d = dir("key");
    assert!(matches!(
        Vault::open(&d.join("vault"), &d.join("missing.key"), false),
        Err(Error::NoKey(_))
    ));
    let v = vault(&d);
    v.put_tin(1, &Tin::new("123456789", TinType::Ein).unwrap())
        .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(d.join("keys/vault.key"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let reopened = Vault::open(&d.join("vault"), &d.join("keys/vault.key"), false).unwrap();
    assert_eq!(reopened.tin(1).unwrap().unwrap().digits(), "123456789");
    std::fs::write(d.join("bad.key"), "not hex").unwrap();
    assert!(matches!(
        Vault::open(&d.join("vault"), &d.join("bad.key"), false),
        Err(Error::BadKey(_))
    ));
}

/// INVARIANT 6: purge deletes the party's records and only theirs.
#[test]
fn purge_removes_a_party() {
    let d = dir("purge");
    let v = vault(&d);
    let tin = Tin::new("123456789", TinType::Ein).unwrap();
    v.put_tin(1, &tin).unwrap();
    v.put_tin(2, &tin).unwrap();
    assert!(v.purge(1).unwrap());
    assert!(v.tin(1).unwrap().is_none());
    assert!(!d.join("vault/tin-1.bin").exists());
    assert!(v.tin(2).unwrap().is_some());
    assert!(!v.purge(1).unwrap(), "nothing left to purge");
}

/// INVARIANT 6, without the key: `records_held` says what a party has on
/// disk and `purge_records` deletes it, so a vault whose key is lost can
/// still be purged.
#[test]
fn purge_and_holdings_need_no_key() {
    let d = dir("keyless");
    let v = vault(&d);
    let tin = Tin::new("123456789", TinType::Ssn).unwrap();
    v.put_tin(1, &tin).unwrap();
    v.put_w9(
        1,
        &W9File {
            file_name: "w9.pdf".into(),
            mime_type: "application/pdf".into(),
            bytes: b"%PDF".to_vec(),
        },
    )
    .unwrap();
    v.put_tin(2, &tin).unwrap();
    std::fs::remove_file(d.join("keys/vault.key")).unwrap();
    let vdir = d.join("vault");
    assert_eq!(ledger_vault::records_held(&vdir, 1).unwrap(), (true, true));
    assert_eq!(ledger_vault::records_held(&vdir, 2).unwrap(), (true, false));
    assert_eq!(
        ledger_vault::records_held(&vdir, 3).unwrap(),
        (false, false)
    );
    assert!(ledger_vault::purge_records(&vdir, 1).unwrap());
    assert_eq!(
        ledger_vault::records_held(&vdir, 1).unwrap(),
        (false, false)
    );
    assert_eq!(ledger_vault::records_held(&vdir, 2).unwrap(), (true, false));
    assert!(!ledger_vault::purge_records(&vdir, 1).unwrap());
    // A vault directory that doesn't exist holds nothing.
    assert_eq!(
        ledger_vault::records_held(&d.join("nowhere"), 1).unwrap(),
        (false, false)
    );
}
