//! The viewer's command line: `--vault-key-file` names the vault key and
//! wins over a settings file's `[vault] key_file`, as it does for the
//! server; an unknown argument stops the viewer with status 2. (Its own
//! test binary: the key file is named once per process.)

use ledger_core::{Role, TinFacts, TinType};
use ledger_gui::{Source, data};
use ledger_store::Store;
use ledger_vault::{Tin, Vault};

const SSN: &str = "123-45-6789";

#[test]
fn the_key_named_on_the_command_line_wins() {
    let root = std::env::temp_dir().join(format!("ledger-gui-key-flag-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let books = root.join("books");
    let key = root.join("keys/vault.key");
    std::fs::create_dir_all(&books).unwrap();
    let mut s = Store::create(&books.join("books-data"), ledger_core::Currency::USD).unwrap();
    let party = s.add_party("Pat Lee", &[Role::Member]).unwrap();
    let v = Vault::open(&books.join("vault"), &key, true).unwrap();
    let hash = v
        .put_tin(party, &Tin::new(SSN, ledger_vault::TinType::Ssn).unwrap())
        .unwrap();
    s.set_party_tin(
        party,
        Some(TinFacts {
            tin_type: TinType::Ssn,
            last4: "6789".into(),
            vault_record: Some(hash),
            purged: None,
        }),
    )
    .unwrap();
    drop(s);

    // Named first, so the default key location is never consulted.
    assert_eq!(ledger_gui::vault_key_file(), None);
    ledger_gui::set_vault_key_file(key.clone());
    assert_eq!(ledger_gui::vault_key_file(), Some(key.clone()));
    // Named once: a second naming changes nothing.
    ledger_gui::set_vault_key_file(root.join("elsewhere.key"));
    assert_eq!(ledger_gui::vault_key_file(), Some(key.clone()));

    let mut b = data::load_once(&Source {
        data_dir: books.join("books-data"),
        books_toml: None,
        today: Some(chrono::NaiveDate::from_ymd_opt(2026, 10, 17).unwrap()),
    })
    .unwrap();
    // The settings name a key that isn't there; the command line wins.
    b.settings.vault_key_file = Some(root.join("missing/vault.key"));
    assert_eq!(b.tin_from_vault(party).unwrap(), SSN);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_unknown_argument_stops_the_viewer() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_ledger-gui"))
        .arg("--no-such-flag")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("unknown argument --no-such-flag"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let help = std::process::Command::new(env!("CARGO_BIN_EXE_ledger-gui"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("--vault-key-file PATH"));
}
