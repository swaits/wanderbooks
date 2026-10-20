//! vault_tin: a worksheet's full TIN comes from the vault only when the
//! books' TIN facts and W-9 facts agree with it (the module's INVARIANTS
//! 1 and 2), checked against a real vault in a temp directory.

use chrono::NaiveDate;
use ledger_core::{Currency, PartyId, Role, TinFacts, TinType, W9Facts};
use ledger_package::vault_tin;
use ledger_store::Store;
use ledger_vault::{Tin, Vault};
use std::path::Path;

const EIN: &str = "12-3456789";
/// A SHA-256 no vault record has.
const STALE: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Books with one contractor, and a vault holding its EIN; the record's
/// hash comes back.
fn setup(dir: &Path) -> (Store, Result<Vault, String>, PartyId, String) {
    let mut s = Store::create(&dir.join("books-data"), Currency::USD).unwrap();
    let acme = s.add_party("Acme Design", &[Role::Contractor]).unwrap();
    let vault = Vault::open(&dir.join("vault"), &dir.join("vault.key"), true).unwrap();
    let hash = vault
        .put_tin(acme, &Tin::new(EIN, ledger_vault::TinType::Ein).unwrap())
        .unwrap();
    (s, Ok(vault), acme, hash)
}

fn tin_facts(tin_type: TinType, vault_record: Option<&str>) -> TinFacts {
    TinFacts {
        tin_type,
        last4: "6789".into(),
        vault_record: vault_record.map(String::from),
        purged: None,
    }
}

fn w9(vault_record: Option<&str>) -> W9Facts {
    W9Facts {
        received: NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
        entity_type: "C corporation".into(),
        exempt_payee_code: None,
        fatca_code: None,
        tin_last4: "6789".into(),
        kept_at: "wanderbooks vault".into(),
        vault_record: vault_record.map(String::from),
    }
}

#[test]
fn a_tin_of_another_type_with_the_same_last_four_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, vault, acme, _) = setup(dir.path());
    // The books say an SSN ending 6789; the vault holds an EIN ending 6789.
    s.set_party_tin(acme, Some(tin_facts(TinType::Ssn, None)))
        .unwrap();
    let err = vault_tin(s.ledger(), &vault, acme).unwrap_err();
    assert_eq!(
        err,
        "the vault's TIN (ein, ending 6789) isn't the one the books record (ssn, ending 6789): record the right one with books.set_tin"
    );
    // The right type: the TIN.
    s.set_party_tin(acme, Some(tin_facts(TinType::Ein, None)))
        .unwrap();
    assert_eq!(
        vault_tin(s.ledger(), &vault, acme).unwrap().digits(),
        "123456789"
    );
}

#[test]
fn with_only_w9_facts_their_vault_record_must_be_the_records_hash() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, vault, acme, hash) = setup(dir.path());
    // The W-9 facts name the record: the TIN.
    s.set_party_w9(acme, Some(w9(Some(&hash)))).unwrap();
    assert_eq!(
        vault_tin(s.ledger(), &vault, acme).unwrap().digits(),
        "123456789"
    );
    // They name no record: nothing to check it against, the TIN.
    s.set_party_w9(acme, Some(w9(None))).unwrap();
    assert_eq!(
        vault_tin(s.ledger(), &vault, acme).unwrap().digits(),
        "123456789"
    );
    // They name another record: refused.
    s.set_party_w9(acme, Some(w9(Some(STALE)))).unwrap();
    assert_eq!(
        vault_tin(s.ledger(), &vault, acme).unwrap_err(),
        "the vault record isn't the one the books recorded: check the W-9 and record it again with books.set_w9 tin"
    );
}

#[test]
fn with_tin_facts_the_w9_facts_record_is_not_the_one_checked() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, vault, acme, hash) = setup(dir.path());
    // The TIN facts name the record; the W-9 facts name an older one, or
    // none: the TIN facts' record is the one that counts (INVARIANT 2).
    s.set_party_tin(acme, Some(tin_facts(TinType::Ein, Some(&hash))))
        .unwrap();
    for record in [Some(STALE), None] {
        s.set_party_w9(acme, Some(w9(record))).unwrap();
        assert_eq!(
            vault_tin(s.ledger(), &vault, acme).unwrap().digits(),
            "123456789",
            "{record:?}"
        );
    }
}
