//! A TIN in the vault copies from the party page, and is never drawn.
//!
//! ```text
//! GOAL: people can use a member's or payee's full TIN (paste it into a
//!       form) without the viewer ever showing it.
//! SOURCE: DECISION (owner, 2026-10-16: "the TIN on the party page
//!         should have a click to copy too ... people still need to
//!         actually access these things").
//! INVARIANTS: Books::tin_from_vault 1-4, and
//!   5. Clicking the masked TIN puts the full, formatted number on the
//!      clipboard; nothing the viewer draws holds more than the last four.
//!   6. A failed read (locked vault) copies nothing and says why.
//! ```

use egui::OutputCommand;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ledger_core::{PartyId, Role, TinFacts, TinType};
use ledger_gui::books::Books;
use ledger_gui::nav::Route;
use ledger_gui::{App, Source, data};
use ledger_store::Store;
use ledger_vault::{Tin, Vault};
use std::path::{Path, PathBuf};

const SSN: &str = "123-45-6789";

struct Fixture {
    root: PathBuf,
    books: PathBuf,
    key: PathBuf,
    party: PartyId,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Books with one member whose SSN is in the vault; the key lives outside
/// the books dir, as the server requires.
fn fixture(tag: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("ledger-gui-tin-{tag}-{}", std::process::id()));
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
    Fixture {
        root,
        books,
        key,
        party,
    }
}

fn load(f: &Fixture, key: Option<&Path>) -> Books {
    let mut b = data::load_once(&Source {
        data_dir: f.books.join("books-data"),
        books_toml: None,
        today: Some(chrono::NaiveDate::from_ymd_opt(2026, 10, 16).unwrap()),
    })
    .unwrap();
    b.settings.vault_key_file = key.map(Path::to_path_buf);
    b
}

fn copied(h: &Harness<'_, App>) -> Vec<String> {
    h.output()
        .platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            OutputCommand::CopyText(t) => Some(t.clone()),
            _ => None,
        })
        .collect()
}

fn no_digits(s: &str) {
    let digits: String = s.chars().filter(char::is_ascii_digit).collect();
    assert!(!digits.contains("12345"), "{s}");
}

/// INVARIANTS 1, 3, 4: the vault's record, formatted; a party without
/// one is "not in the vault".
#[test]
fn reads_the_vault_with_the_key_in_force() {
    let f = fixture("read");
    let b = load(&f, Some(&f.key));
    assert_eq!(b.tin_from_vault(f.party).unwrap(), SSN);
    let e = b.tin_from_vault(f.party + 1).unwrap_err();
    assert!(e.contains("Not in the vault"), "{e}");
}

/// INVARIANT 2: a missing key is a locked vault, and none is created.
#[test]
fn a_missing_key_is_locked_and_never_created() {
    let f = fixture("locked");
    let gone = f.root.join("elsewhere/vault.key");
    let b = load(&f, Some(&gone));
    let e = b.tin_from_vault(f.party).unwrap_err();
    assert!(e.contains("locked"), "{e}");
    no_digits(&e);
    assert!(!gone.exists(), "reading must never create a key");
}

/// INVARIANT 3: a key file inside the books dir is refused.
#[test]
fn a_key_inside_the_books_is_refused() {
    let f = fixture("inside");
    let inside = f.books.join("vault.key");
    std::fs::copy(&f.key, &inside).unwrap();
    let e = load(&f, Some(&inside)).tin_from_vault(f.party).unwrap_err();
    assert!(e.contains("inside the books folder"), "{e}");
}

/// INVARIANT 4: a tampered record is unreadable, and says nothing of it.
#[test]
fn a_bad_record_is_an_error_without_digits() {
    let f = fixture("tamper");
    let rec = f.books.join(format!("vault/tin-{}.bin", f.party));
    let mut raw = std::fs::read(&rec).unwrap();
    let last = raw.len() - 1;
    raw[last] ^= 1;
    std::fs::write(&rec, raw).unwrap();
    let e = load(&f, Some(&f.key)).tin_from_vault(f.party).unwrap_err();
    no_digits(&e);
}

fn party_page(b: Books, party: PartyId) -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 1000.0))
        .build_eframe(move |cc| App::with_books(&cc.egui_ctx, b, false, Route::Party(party)));
    h.run_steps(3);
    h
}

/// INVARIANTS 5, 6, through the party page.
#[test]
fn clicking_the_masked_tin_copies_the_full_number() {
    let f = fixture("click");
    let shown = "TIN on file ••••6789 (SSN)";
    let mut h = party_page(load(&f, Some(&f.key)), f.party);
    assert!(
        h.query_by_label_contains("12345").is_none(),
        "the full number is never drawn"
    );
    h.get_by_label(shown).click();
    h.step();
    assert_eq!(copied(&h), [SSN]);
    h.run_steps(2);
    assert!(h.query_by_label_contains("12345").is_none());

    // Locked: nothing copied; the reason shows beside it.
    let gone = f.root.join("elsewhere/vault.key");
    let mut h = party_page(load(&f, Some(&gone)), f.party);
    h.get_by_label(shown).click();
    h.step();
    assert!(copied(&h).is_empty());
    h.run_steps(2);
    assert!(h.query_by_label_contains("The vault is locked").is_some());
}
