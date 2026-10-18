//! Spec-derived tests for the party directory in the store.
//!
//! Oracles: the `Store` party methods' doc comments (each "journaled as"
//! its event: `party.added {id, name, roles}`, `party.updated {id,
//! change}`, `party.role_set {id, role, on}`, `party.w9_set {id, facts}`,
//! `party.w4_set {id, facts}`, `entry.party_set {entry_id, party_id}`;
//! `payroll.run` lines carry `employee`), the `ledger_core::parties` spec
//! (INVARIANTS 1–4), `Store::verify` (a fresh replay equals the live
//! ledger), and `docs/parties-and-attachments.md` "Migration" (old journals
//! replay unchanged; vendors and customers get their parties by replay).
//! Expected values are worked by hand from those texts.

use chrono::NaiveDate;
use ledger_core::{
    AccountId, AccountType, Currency, EntryId, Ledger, Money, Party, PartyField, PartyId,
    PayrollLine, PayrollRun, Role, Side, W4Facts, W9Facts,
};
use ledger_store::journal::read_and_verify_journal;
use ledger_store::{DB_FILE, JOURNAL_FILE, JournalWriter, Store};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::str::FromStr;

// ---------------------------------------------------------------- helpers

/// A test's dir, removed when the test ends, pass or panic.
struct TestDir(PathBuf);

impl std::ops::Deref for TestDir {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl AsRef<std::path::Path> for TestDir {
    fn as_ref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn test_dir(name: &str) -> TestDir {
    let dir = std::env::temp_dir().join(format!("wb-parties-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TestDir(dir)
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn usd(s: &str) -> Money {
    Money::new(dec(s), Currency::USD)
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// The journal's (type, payload) pairs, verified.
fn journal(dir: &Path) -> Vec<(String, Value)> {
    read_and_verify_journal(&dir.join(JOURNAL_FILE))
        .unwrap()
        .into_iter()
        .map(|l| (l.line_type, l.payload))
        .collect()
}

fn journal_bytes(dir: &Path) -> Vec<u8> {
    std::fs::read(dir.join(JOURNAL_FILE)).unwrap()
}

/// Run `f`; it must journal exactly one line, of type `ty`. Returns that
/// line's payload.
fn one_event<T>(
    s: &mut Store,
    dir: &Path,
    ty: &str,
    f: impl FnOnce(&mut Store) -> T,
) -> (T, Value) {
    let seq = s.journal_seq();
    let len = journal(dir).len();
    let out = f(s);
    assert_eq!(s.journal_seq(), seq + 1, "{ty}: exactly one line");
    let j = journal(dir);
    assert_eq!(j.len(), len + 1, "{ty}: exactly one line on disk");
    let (t, p) = j.last().unwrap().clone();
    assert_eq!(t, ty);
    (out, p)
}

/// Run `f`; it must fail and journal nothing.
fn refused_nothing_journaled<T: std::fmt::Debug, E>(
    s: &mut Store,
    dir: &Path,
    what: &str,
    f: impl FnOnce(&mut Store) -> Result<T, E>,
) {
    let seq = s.journal_seq();
    let bytes = journal_bytes(dir);
    let before = s.ledger().clone();
    let r = f(s);
    assert!(r.is_err(), "{what}: expected refusal, got {:?}", r.ok());
    assert_eq!(s.journal_seq(), seq, "{what}: journaled");
    assert_eq!(journal_bytes(dir), bytes, "{what}: journal changed");
    assert!(*s.ledger() == before, "{what}: ledger changed");
}

fn w9(last4: &str) -> W9Facts {
    W9Facts {
        received: date(2026, 3, 1),
        entity_type: "individual/sole proprietor".to_string(),
        exempt_payee_code: None,
        fatca_code: None,
        tin_last4: last4.to_string(),
        kept_at: "wanderbooks vault".to_string(),
        vault_record: None,
    }
}

fn w4(state: &str) -> W4Facts {
    W4Facts {
        received: date(2026, 3, 1),
        state: state.to_string(),
        kept_at: "CPA portal".to_string(),
    }
}

fn pay_line(name: &str, employee: Option<PartyId>) -> PayrollLine {
    PayrollLine {
        employee_name: name.to_string(),
        employee,
        gross_pay: dec("1000.00"),
        federal_withholding: dec("100.00"),
        state_withholding: dec("0"),
        ss_employee: dec("62.00"),
        medicare_employee: dec("14.50"),
        ss_employer: dec("62.00"),
        medicare_employer: dec("14.50"),
        other_deductions: dec("0"),
    }
}

struct Fx {
    cash: AccountId,
    entry: EntryId,
}

/// A store with cash, capital, and one posted $500.00 entry.
fn fixture(dir: &Path) -> (Store, Fx) {
    let mut s = Store::create(dir, Currency::USD).unwrap();
    let cash = s
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let capital = s
        .add_account("3000", "Owner Capital", AccountType::Equity, None, false)
        .unwrap();
    let entry = s.create_entry(date(2026, 1, 1), "Opening").unwrap();
    s.add_line(entry, cash, Side::Debit, usd("500.00")).unwrap();
    s.add_line(entry, capital, Side::Credit, usd("500.00"))
        .unwrap();
    s.post_entry(entry).unwrap();
    (s, Fx { cash, entry })
}

/// What "replays to an equal ledger" compares: every party, every entry's
/// party and vendor tags, every payroll run.
#[derive(Debug, PartialEq)]
struct View {
    parties: Vec<Party>,
    tags: Vec<(EntryId, Option<PartyId>, Option<u64>)>,
    runs: Vec<PayrollRun>,
}

fn view(l: &Ledger) -> View {
    View {
        parties: l.parties().cloned().collect(),
        tags: (0..l.next_entry_id() + 2)
            .map(|e| (e, l.entry_party(e), l.entry_vendor(e)))
            .collect(),
        runs: l.list_payroll_runs().into_iter().cloned().collect(),
    }
}

fn delete_db(dir: &Path) {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let _ = std::fs::remove_file(dir.join(format!("{DB_FILE}{suffix}")));
    }
}

/// A store exercising every party event.
/// `named_employee`: whether the payroll run names Alice's party.
fn rich(dir: &Path, named_employee: bool) -> Store {
    let (mut s, fx) = fixture(dir);
    let irs = s.add_party("IRS", &[Role::TaxAgency]).unwrap();
    let dev = s.add_party("Dev Shop", &[Role::Contractor]).unwrap();
    let alice = s.add_party("Alice", &[Role::Employee]).unwrap();
    s.update_party(dev, PartyField::Website(Some("https://dev.example".into())))
        .unwrap();
    s.update_party(dev, PartyField::DisplayName(Some("Dev".into())))
        .unwrap();
    s.update_party(irs, PartyField::Notes(Some("EFTPS".into())))
        .unwrap();
    s.set_party_role(dev, Role::Landlord, true).unwrap();
    s.set_party_role(dev, Role::Landlord, false).unwrap();
    s.set_party_w9(dev, Some(w9("6789"))).unwrap();
    s.set_party_w4(alice, Some(w4("UT"))).unwrap();
    let v = s.add_vendor("dev shop").unwrap();
    let c = s.add_customer("Acme").unwrap();
    s.add_vendor("Acme").unwrap();
    let _ = c;
    s.set_entry_party(fx.entry, Some(irs)).unwrap();
    s.set_entry_vendor(fx.entry, Some(v)).unwrap();
    let old = s.add_party("Old Name", &[]).unwrap();
    s.update_party(old, PartyField::Archived(true)).unwrap();
    s.add_vendor("Old Name").unwrap();
    s.run_payroll(
        date(2026, 2, 27),
        vec![
            pay_line("Alice", named_employee.then_some(alice)),
            pay_line("Bob", None),
        ],
        fx.cash,
    )
    .unwrap();
    s
}

// ---------------------------------------------------------------- journaling

/// Oracle: Store::add_party doc ("journaled as party.added") and the
/// task's payload {id, name, roles}; roles by wire key.
#[test]
fn add_party_journals_party_added() {
    let dir = test_dir("added");
    let (mut s, _) = fixture(&dir);
    let next = s.ledger().next_party_id();
    let (id, p) = one_event(&mut s, &dir, "party.added", |s| {
        s.add_party("Internal Revenue Service", &[Role::TaxAgency])
            .unwrap()
    });
    assert_eq!(id, next);
    assert_eq!(p["id"], json!(id));
    assert_eq!(p["name"], json!("Internal Revenue Service"));
    assert_eq!(p["roles"], json!(["tax_agency"]));

    let (id2, p) = one_event(&mut s, &dir, "party.added", |s| {
        s.add_party("Tobin", &[Role::Member, Role::Employee])
            .unwrap()
    });
    assert_eq!(p["id"], json!(id2));
    let mut roles: Vec<String> = p["roles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap().to_string())
        .collect();
    roles.sort();
    assert_eq!(roles, vec!["employee".to_string(), "member".to_string()]);
}

/// Oracle: Store::update_party doc ("journaled as party.updated") with
/// {id, change}; PartyField's wire form is `{field, value}` (its serde
/// attributes: snake_case, tag "field", content "value").
#[test]
fn update_party_journals_party_updated() {
    let dir = test_dir("updated");
    let (mut s, _) = fixture(&dir);
    let id = s.add_party("Acme", &[]).unwrap();
    let cases = [
        (
            PartyField::Website(Some("https://acme.example".into())),
            json!({"field": "website", "value": "https://acme.example"}),
        ),
        (
            PartyField::DisplayName(Some("ACME".into())),
            json!({"field": "display_name", "value": "ACME"}),
        ),
        (
            PartyField::DisplayName(None),
            json!({"field": "display_name", "value": null}),
        ),
        (
            PartyField::Address(Some("1 Main St".into())),
            json!({"field": "address", "value": "1 Main St"}),
        ),
        (
            PartyField::Notes(Some("net 30".into())),
            json!({"field": "notes", "value": "net 30"}),
        ),
        (
            PartyField::Name("Acme LLC".into()),
            json!({"field": "name", "value": "Acme LLC"}),
        ),
        (
            PartyField::Archived(true),
            json!({"field": "archived", "value": true}),
        ),
    ];
    for (field, want) in cases {
        let ((), p) = one_event(&mut s, &dir, "party.updated", |s| {
            s.update_party(id, field.clone()).unwrap()
        });
        assert_eq!(p["id"], json!(id));
        assert_eq!(p["change"], want, "{field:?}");
    }
    let got = s.ledger().get_party(id).unwrap();
    assert_eq!(got.name, "Acme LLC");
    assert!(got.archived);
    assert_eq!(got.notes.as_deref(), Some("net 30"));
}

/// Oracle: Store::set_party_role doc ("journaled as party.role_set") with
/// {id, role, on}.
#[test]
fn set_party_role_journals_party_role_set() {
    let dir = test_dir("role");
    let (mut s, _) = fixture(&dir);
    let id = s.add_party("Mo", &[]).unwrap();
    let ((), p) = one_event(&mut s, &dir, "party.role_set", |s| {
        s.set_party_role(id, Role::PayoutPlatform, true).unwrap()
    });
    assert_eq!(p, json!({"id": id, "role": "payout_platform", "on": true}));
    let ((), p) = one_event(&mut s, &dir, "party.role_set", |s| {
        s.set_party_role(id, Role::PayoutPlatform, false).unwrap()
    });
    assert_eq!(p, json!({"id": id, "role": "payout_platform", "on": false}));
}

/// Oracle: Store::set_party_w9 / set_party_w4 docs (`party.w9_set` /
/// `party.w4_set` {id, facts}); facts null when cleared.
#[test]
fn w9_and_w4_journal_their_facts() {
    let dir = test_dir("facts");
    let (mut s, _) = fixture(&dir);
    let c = s.add_party("Contractor", &[Role::Contractor]).unwrap();
    let ((), p) = one_event(&mut s, &dir, "party.w9_set", |s| {
        s.set_party_w9(c, Some(w9("6789"))).unwrap()
    });
    assert_eq!(p["id"], json!(c));
    assert_eq!(p["facts"]["tin_last4"], json!("6789"));
    assert_eq!(p["facts"]["received"], json!("2026-03-01"));
    assert_eq!(
        p["facts"]["entity_type"],
        json!("individual/sole proprietor")
    );
    assert_eq!(p["facts"]["kept_at"], json!("wanderbooks vault"));
    let back: W9Facts = serde_json::from_value(p["facts"].clone()).unwrap();
    assert_eq!(back, w9("6789"));
    let ((), p) = one_event(&mut s, &dir, "party.w9_set", |s| {
        s.set_party_w9(c, None).unwrap()
    });
    assert_eq!(p, json!({"id": c, "facts": null}));

    let e = s.add_party("Employee", &[Role::Employee]).unwrap();
    let ((), p) = one_event(&mut s, &dir, "party.w4_set", |s| {
        s.set_party_w4(e, Some(w4("UT"))).unwrap()
    });
    assert_eq!(
        p,
        json!({"id": e, "facts": {"received": "2026-03-01", "state": "UT", "kept_at": "CPA portal"}})
    );
    let ((), p) = one_event(&mut s, &dir, "party.w4_set", |s| {
        s.set_party_w4(e, None).unwrap()
    });
    assert_eq!(p, json!({"id": e, "facts": null}));
}

/// Oracle: Store::set_entry_party doc (`entry.party_set {entry_id,
/// party_id}`, null when cleared).
#[test]
fn set_entry_party_journals_entry_party_set() {
    let dir = test_dir("entry");
    let (mut s, fx) = fixture(&dir);
    let p = s.add_party("Utah", &[Role::TaxAgency]).unwrap();
    let ((), pl) = one_event(&mut s, &dir, "entry.party_set", |s| {
        s.set_entry_party(fx.entry, Some(p)).unwrap()
    });
    assert_eq!(pl, json!({"entry_id": fx.entry, "party_id": p}));
    assert_eq!(s.ledger().entry_party(fx.entry), Some(p));
    let ((), pl) = one_event(&mut s, &dir, "entry.party_set", |s| {
        s.set_entry_party(fx.entry, None).unwrap()
    });
    assert_eq!(pl, json!({"entry_id": fx.entry, "party_id": null}));
    assert_eq!(s.ledger().entry_party(fx.entry), None);
}

/// Oracle: add_vendor / add_customer docs ("journaled as vendor.added" /
/// "customer.added"): one event each; the party comes from the core
/// auto-link (so from replay), not a second event.
#[test]
fn add_vendor_and_customer_journal_one_event_and_link_a_party() {
    let dir = test_dir("records");
    let (mut s, fx) = fixture(&dir);
    let (v, _) = one_event(&mut s, &dir, "vendor.added", |s| {
        s.add_vendor("Acme").unwrap()
    });
    let (c, _) = one_event(&mut s, &dir, "customer.added", |s| {
        s.add_customer("ACME").unwrap()
    });
    let pv = s.ledger().party_of_vendor(v).unwrap();
    assert_eq!(s.ledger().party_of_customer(c), Some(pv));
    // set_entry_vendor: one entry.vendor_set line, and the party is tagged.
    let ((), pl) = one_event(&mut s, &dir, "entry.vendor_set", |s| {
        s.set_entry_vendor(fx.entry, Some(v)).unwrap()
    });
    assert_eq!(pl["entry_id"], json!(fx.entry));
    assert_eq!(s.ledger().entry_party(fx.entry), Some(pv));
}

/// Oracle: payroll.run lines carry "employee" (the party id, or null).
#[test]
fn payroll_run_lines_carry_employee() {
    let dir = test_dir("payroll");
    let (mut s, fx) = fixture(&dir);
    let alice = s.add_party("Alice", &[Role::Employee]).unwrap();
    s.run_payroll(
        date(2026, 2, 27),
        vec![pay_line("Alice", Some(alice)), pay_line("Bob", None)],
        fx.cash,
    )
    .unwrap();
    let j = journal(&dir);
    let (_, p) = j.iter().rev().find(|(t, _)| t == "payroll.run").unwrap();
    let lines = p["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 2);
    let by_name = |n: &str| {
        lines
            .iter()
            .find(|l| l["employee_name"] == json!(n))
            .unwrap()
            .clone()
    };
    assert_eq!(by_name("Alice")["employee"], json!(alice));
    assert_eq!(by_name("Bob")["employee"], Value::Null);
}

// ---------------------------------------------------------------- refusals

/// Oracle: the core SCENARIOS (refusals) and the store's journal-first
/// contract: a refusal journals nothing.
#[test]
fn refusals_journal_nothing() {
    let dir = test_dir("refusals");
    let (mut s, fx) = fixture(&dir);
    let a = s.add_party("Alpha", &[Role::Member]).unwrap();
    s.add_party("Beta", &[]).unwrap();
    let emp = s.add_party("Emp", &[Role::Employee]).unwrap();
    let ghost = s.ledger().next_party_id() + 50;
    refused_nothing_journaled(&mut s, &dir, "blank", |s| s.add_party("  ", &[]));
    refused_nothing_journaled(&mut s, &dir, "tin", |s| s.add_party("Bob 123-45-6789", &[]));
    refused_nothing_journaled(&mut s, &dir, "dup", |s| s.add_party(" alpha", &[]));
    refused_nothing_journaled(&mut s, &dir, "vendor role", |s| {
        s.add_party("Gamma", &[Role::Vendor])
    });
    refused_nothing_journaled(&mut s, &dir, "update unknown", |s| {
        s.update_party(ghost, PartyField::Notes(None))
    });
    refused_nothing_journaled(&mut s, &dir, "update tin", |s| {
        s.update_party(a, PartyField::Notes(Some("12-3456789".into())))
    });
    refused_nothing_journaled(&mut s, &dir, "rename taken", |s| {
        s.update_party(a, PartyField::Name("BETA".into()))
    });
    refused_nothing_journaled(&mut s, &dir, "customer role", |s| {
        s.set_party_role(a, Role::Customer, true)
    });
    refused_nothing_journaled(&mut s, &dir, "w9 without 1099 role", |s| {
        s.set_party_w9(a, Some(w9("6789")))
    });
    refused_nothing_journaled(&mut s, &dir, "w4 without employee", |s| {
        s.set_party_w4(a, Some(w4("UT")))
    });
    refused_nothing_journaled(&mut s, &dir, "bad w4", |s| {
        s.set_party_w4(emp, Some(w4("Utah")))
    });
    s.set_party_w4(emp, Some(w4("UT"))).unwrap();
    refused_nothing_journaled(&mut s, &dir, "drop employee with w4", |s| {
        s.set_party_role(emp, Role::Employee, false)
    });
    refused_nothing_journaled(&mut s, &dir, "entry party unknown", |s| {
        s.set_entry_party(fx.entry, Some(ghost))
    });
    refused_nothing_journaled(&mut s, &dir, "entry unknown", |s| {
        s.set_entry_party(fx.entry + 99, Some(a))
    });
    refused_nothing_journaled(&mut s, &dir, "payroll non-employee", |s| {
        s.run_payroll(date(2026, 2, 27), vec![pay_line("Alpha", Some(a))], fx.cash)
    });
    refused_nothing_journaled(&mut s, &dir, "payroll unknown party", |s| {
        s.run_payroll(
            date(2026, 2, 27),
            vec![pay_line("Ghost", Some(ghost))],
            fx.cash,
        )
    });
}

// ---------------------------------------------------------------- replay

/// Oracle: Store::verify INVARIANT 2 (a fresh replay equals the live
/// ledger) and the journal as system of record: reopening, with or
/// without the database, gives the same directory, tags, and runs.
fn replay_check(name: &str, named_employee: bool, verify: bool) {
    let dir = test_dir(name);
    let s = rich(&dir, named_employee);
    if verify {
        s.verify().unwrap();
    }
    let live = view(s.ledger());
    let live_ledger = s.ledger().clone();
    assert!(live.parties.len() >= 5);
    drop(s);

    let s = Store::open(&dir).unwrap();
    assert_eq!(view(s.ledger()), live, "reopen");
    assert!(
        *s.ledger() == live_ledger,
        "reopen: {:?}",
        s.ledger().differs_from(&live_ledger)
    );
    if verify {
        s.verify().unwrap();
    }
    drop(s);

    delete_db(&dir);
    let s = Store::open(&dir).unwrap();
    assert_eq!(view(s.ledger()), live, "replay without the database");
    assert!(
        *s.ledger() == live_ledger,
        "no db: {:?}",
        s.ledger().differs_from(&live_ledger)
    );
    if verify {
        s.verify().unwrap();
    }
}

#[test]
fn replay_on_reopen_and_without_db_gives_an_equal_ledger() {
    replay_check("replay", false, true);
}

/// Same, with the payroll run naming its employee's party: replay must
/// still agree (verify is the next test).
#[test]
fn replay_with_a_named_employee_gives_an_equal_ledger() {
    replay_check("replay-employee", true, false);
}

/// Oracle: Store::verify INVARIANT 2 ("the SQLite read-back's ... payroll
/// runs equal the replay's") with a run whose line names its employee.
#[test]
fn verify_passes_with_a_named_employee() {
    replay_check("verify-employee", true, true);
}

// ---------------------------------------------------------------- corruption

/// Append `ty`/`payload` with a valid chain, then expect open to refuse.
fn assert_corrupt_on_open(name: &str, setup: impl FnOnce(&mut Store) -> Value, ty: &str) {
    let dir = test_dir(name);
    let (mut s, _) = fixture(&dir);
    let payload = setup(&mut s);
    drop(s);
    let mut w = JournalWriter::open(&dir.join(JOURNAL_FILE)).unwrap();
    w.append(ty, payload.clone()).unwrap();
    drop(w);
    if let Ok(_s) = Store::open(&dir) {
        panic!("{name}: a corrupt {ty} line {payload} opened");
    }
    delete_db(&dir);
    if let Ok(_s) = Store::open(&dir) {
        panic!("{name}: a corrupt {ty} line {payload} opened without the db");
    }
}

/// Oracle: the payloads' documented shapes (Role keys, W9Facts/W4Facts
/// and their check, integer ids, party.added's id = the next id) and the
/// invariants: a line that cannot replay is refused on open.
#[test]
fn corrupt_party_payloads_are_refused_on_open() {
    assert_corrupt_on_open(
        "bad-role",
        |s| json!({"id": s.add_party("A", &[]).unwrap(), "role": "boss", "on": true}),
        "party.role_set",
    );
    assert_corrupt_on_open(
        "record-role",
        |s| json!({"id": s.add_party("A", &[]).unwrap(), "role": "vendor", "on": true}),
        "party.role_set",
    );
    assert_corrupt_on_open(
        "role-on-not-bool",
        |s| json!({"id": s.add_party("A", &[]).unwrap(), "role": "member", "on": "yes"}),
        "party.role_set",
    );
    assert_corrupt_on_open(
        "w9-shape",
        |s| json!({"id": s.add_party("A", &[Role::Contractor]).unwrap(), "facts": {"nonsense": 1}}),
        "party.w9_set",
    );
    assert_corrupt_on_open(
        "w9-full-tin",
        |s| {
            let mut f = serde_json::to_value(w9("6789")).unwrap();
            f["tin_last4"] = json!("123456789");
            json!({"id": s.add_party("A", &[Role::Contractor]).unwrap(), "facts": f})
        },
        "party.w9_set",
    );
    assert_corrupt_on_open(
        "w9-no-1099-role",
        |s| {
            json!({"id": s.add_party("A", &[Role::Member]).unwrap(),
                   "facts": serde_json::to_value(w9("6789")).unwrap()})
        },
        "party.w9_set",
    );
    assert_corrupt_on_open(
        "w4-bad-state",
        |s| {
            json!({"id": s.add_party("A", &[Role::Employee]).unwrap(),
                   "facts": {"received": "2026-03-01", "state": "Utah", "kept_at": "x"}})
        },
        "party.w4_set",
    );
    assert_corrupt_on_open(
        "updated-bad-field",
        |s| json!({"id": s.add_party("A", &[]).unwrap(), "change": {"field": "ssn", "value": "x"}}),
        "party.updated",
    );
    for (i, bad) in [json!("3"), json!(-3), json!({"id": 1}), json!(true)]
        .into_iter()
        .enumerate()
    {
        assert_corrupt_on_open(
            &format!("party-id-{i}"),
            |s| {
                s.add_party("A", &[]).unwrap();
                json!({"entry_id": 0, "party_id": bad})
            },
            "entry.party_set",
        );
    }
    assert_corrupt_on_open(
        "added-id-mismatch",
        |s| json!({"id": s.ledger().next_party_id() + 5, "name": "X", "roles": []}),
        "party.added",
    );
    assert_corrupt_on_open(
        "added-bad-role",
        |s| json!({"id": s.ledger().next_party_id(), "name": "X", "roles": ["boss"]}),
        "party.added",
    );
    assert_corrupt_on_open(
        "added-full-tin",
        |s| json!({"id": s.ledger().next_party_id(), "name": "X 123456789", "roles": []}),
        "party.added",
    );
}

/// Control for the corruption test: a well-formed hand-appended
/// `party.added` and `entry.party_set` open and replay (so the refusals
/// above are about the payloads, not about appending by hand).
#[test]
fn a_well_formed_appended_party_line_opens() {
    let dir = test_dir("well-formed");
    let (s, fx) = fixture(&dir);
    let next = s.ledger().next_party_id();
    drop(s);
    let mut w = JournalWriter::open(&dir.join(JOURNAL_FILE)).unwrap();
    w.append(
        "party.added",
        json!({"id": next, "name": "Apple", "roles": ["payout_platform"]}),
    )
    .unwrap();
    w.append(
        "entry.party_set",
        json!({"entry_id": fx.entry, "party_id": next}),
    )
    .unwrap();
    drop(w);
    let s = Store::open(&dir).unwrap();
    let p = s.ledger().get_party(next).unwrap();
    assert_eq!(p.name, "Apple");
    assert_eq!(p.roles, [Role::PayoutPlatform].into_iter().collect());
    assert_eq!(s.ledger().entry_party(fx.entry), Some(next));
    s.verify().unwrap();
}

// ---------------------------------------------------------------- migration

/// Copy `from`'s journal into a fresh dir via `JournalWriter`, letting
/// `edit` drop or rewrite lines (a valid chain, old-format payloads).
fn rewrite_journal(from: &Path, to: &Path, edit: impl Fn(&str, Value) -> Option<Value>) {
    let mut w = JournalWriter::open(&to.join(JOURNAL_FILE)).unwrap();
    for (t, p) in journal(from) {
        if let Some(p) = edit(&t, p) {
            w.append(&t, p).unwrap();
        }
    }
}

/// Oracle: docs "Migration": old journals replay unchanged; vendor.added
/// and customer.added create parties; payroll lines from before carry the
/// name only (`employee` absent). INVARIANT 4 holds after replay.
#[test]
fn books_from_before_parties_open_and_get_parties_by_replay() {
    let src = test_dir("old-src");
    let (mut s, fx) = fixture(&src);
    let v1 = s.add_vendor("Namecheap").unwrap();
    let v2 = s.add_vendor("Apple").unwrap();
    let c1 = s.add_customer("apple").unwrap();
    let c2 = s.add_customer("Beta Tester").unwrap();
    s.set_entry_vendor(fx.entry, Some(v1)).unwrap();
    s.run_payroll(date(2026, 2, 27), vec![pay_line("Alice", None)], fx.cash)
        .unwrap();
    drop(s);

    let dst = test_dir("old-dst");
    rewrite_journal(&src, &dst, |t, mut p| {
        assert!(
            !t.starts_with("party.") && t != "entry.party_set",
            "{t} in old books"
        );
        if t == "payroll.run" {
            for l in p["lines"].as_array_mut().unwrap() {
                l.as_object_mut().unwrap().remove("employee");
            }
        }
        Some(p)
    });
    let s = Store::open(&dst).unwrap();
    let l = s.ledger();
    // INVARIANT 4: each record links to exactly one party.
    for (vid, name) in [(v1, "Namecheap"), (v2, "Apple")] {
        let pid = l.party_of_vendor(vid).expect("vendor party");
        let owners: Vec<_> = l.parties().filter(|p| p.vendor == Some(vid)).collect();
        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].id, pid);
        assert_eq!(owners[0].name, name);
        assert!(owners[0].all_roles().contains(&Role::Vendor));
    }
    for cid in [c1, c2] {
        let owners: Vec<_> = l.parties().filter(|p| p.customer == Some(cid)).collect();
        assert_eq!(owners.len(), 1);
        assert_eq!(l.party_of_customer(cid), Some(owners[0].id));
    }
    // The auto-link spec: "apple" the customer joins "Apple" the vendor.
    assert_eq!(l.party_of_customer(c1), l.party_of_vendor(v2));
    assert_eq!(l.parties().count(), 3);
    // entry.vendor_set replays as the party tag too.
    assert_eq!(l.entry_vendor(fx.entry), Some(v1));
    assert_eq!(l.entry_party(fx.entry), l.party_of_vendor(v1));
    // The old payroll run has the name only.
    let runs = l.list_payroll_runs();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].lines[0].employee_name, "Alice");
    assert_eq!(runs[0].lines[0].employee, None);
    s.verify().unwrap();
    let before = view(s.ledger());
    drop(s);
    delete_db(&dst);
    let s = Store::open(&dst).unwrap();
    assert_eq!(view(s.ledger()), before);
}

// ------------------------------------------------------- party shares

/// Oracle: Store::set_entry_parties doc (`entry.parties_set {entry_id,
/// parties: [{party_id, share}]}`, a share `{"amount": "30.00"}`,
/// `{"percent": "60"}`, or `"derived"`; refusals journal nothing) and
/// Store::verify (a fresh replay equals the live ledger).
mod shares {
    use super::*;
    use ledger_core::{PartyShare, Share};

    fn share(party: PartyId, share: Share) -> PartyShare {
        PartyShare { party, share }
    }

    #[test]
    fn set_entry_parties_journals_one_event_with_each_share() {
        let dir = test_dir("shares-event");
        let (mut s, fx) = fixture(&dir);
        let a = s.add_party("Tobin Harlow", &[Role::Member]).unwrap();
        let b = s
            .add_party("Morgan Ellis", &[Role::Member])
            .unwrap();
        let ((), pl) = one_event(&mut s, &dir, "entry.parties_set", |s| {
            s.set_entry_parties(fx.entry, &[PartyShare::whole(a), PartyShare::whole(b)])
                .unwrap()
        });
        assert_eq!(
            pl,
            json!({"entry_id": fx.entry, "parties": [
                {"party_id": a, "share": "derived"},
                {"party_id": b, "share": "derived"},
            ]})
        );
        let ((), pl) = one_event(&mut s, &dir, "entry.parties_set", |s| {
            s.set_entry_parties(
                fx.entry,
                &[
                    share(a, Share::Amount(dec("300.00"))),
                    share(b, Share::Amount(dec("200.00"))),
                ],
            )
            .unwrap()
        });
        assert_eq!(
            pl,
            json!({"entry_id": fx.entry, "parties": [
                {"party_id": a, "share": {"amount": "300.00"}},
                {"party_id": b, "share": {"amount": "200.00"}},
            ]})
        );
        let ((), pl) = one_event(&mut s, &dir, "entry.parties_set", |s| {
            s.set_entry_parties(
                fx.entry,
                &[
                    share(a, Share::Percent(dec("60"))),
                    share(b, Share::Percent(dec("40"))),
                ],
            )
            .unwrap()
        });
        assert_eq!(
            pl["parties"][0]["share"],
            json!({"percent": "60"}),
            "percent shares journal as strings"
        );
        assert_eq!(s.ledger().entry_parties(fx.entry).len(), 2);
        assert_eq!(s.ledger().entry_party(fx.entry), None);
    }

    #[test]
    fn refusals_journal_nothing() {
        let dir = test_dir("shares-refused");
        let (mut s, fx) = fixture(&dir);
        let a = s.add_party("A", &[Role::Member]).unwrap();
        let b = s.add_party("B", &[Role::Member]).unwrap();
        let v1 = s.add_vendor("V1").unwrap();
        let v2 = s.add_vendor("V2").unwrap();
        let (p1, p2) = (
            s.ledger().party_of_vendor(v1).unwrap(),
            s.ledger().party_of_vendor(v2).unwrap(),
        );
        let cases: Vec<(&str, EntryId, Vec<PartyShare>)> = vec![
            ("unknown entry", fx.entry + 99, vec![PartyShare::whole(a)]),
            ("unknown party", fx.entry, vec![PartyShare::whole(999)]),
            (
                "duplicate",
                fx.entry,
                vec![PartyShare::whole(a), PartyShare::whole(a)],
            ),
            (
                "mixed",
                fx.entry,
                vec![share(a, Share::Percent(dec("50"))), PartyShare::whole(b)],
            ),
            (
                "percent sum",
                fx.entry,
                vec![
                    share(a, Share::Percent(dec("50"))),
                    share(b, Share::Percent(dec("40"))),
                ],
            ),
            (
                "sub-cent",
                fx.entry,
                vec![
                    share(a, Share::Amount(dec("250.001"))),
                    share(b, Share::Amount(dec("249.999"))),
                ],
            ),
            (
                "two vendors",
                fx.entry,
                vec![
                    share(p1, Share::Amount(dec("250.00"))),
                    share(p2, Share::Amount(dec("250.00"))),
                ],
            ),
        ];
        for (what, e, shares) in cases {
            refused_nothing_journaled(&mut s, &dir, what, |s| s.set_entry_parties(e, &shares));
        }
    }

    #[test]
    fn replay_gives_equal_state() {
        let dir = test_dir("shares-replay");
        let (mut s, fx) = fixture(&dir);
        let a = s.add_party("Tobin Harlow", &[Role::Member]).unwrap();
        let b = s
            .add_party("Morgan Ellis", &[Role::Member])
            .unwrap();
        let v = s.add_vendor("Contractor").unwrap();
        let pv = s.ledger().party_of_vendor(v).unwrap();
        s.set_entry_parties(
            fx.entry,
            &[
                share(a, Share::Amount(dec("300.00"))),
                share(b, Share::Amount(dec("200.00"))),
            ],
        )
        .unwrap();
        // A second entry shared with a vendor party, then reversed.
        let other = s.create_entry(date(2026, 2, 1), "Split work").unwrap();
        let exp = s
            .add_account("6000", "Work", AccountType::Expense, None, false)
            .unwrap();
        s.add_line(other, exp, Side::Debit, usd("90.00")).unwrap();
        s.add_line(other, fx.cash, Side::Credit, usd("90.00"))
            .unwrap();
        s.post_entry(other).unwrap();
        s.set_entry_parties(
            other,
            &[
                share(pv, Share::Percent(dec("66.67"))),
                share(a, Share::Percent(dec("33.33"))),
            ],
        )
        .unwrap();
        assert_eq!(s.ledger().entry_vendor(other), Some(v));
        let r = s.reverse_entry(other, date(2026, 2, 2), "undo").unwrap();
        assert_eq!(s.ledger().entry_parties(r), s.ledger().entry_parties(other));
        // An old-style single tag after a multi-party one.
        let third = s.create_entry(date(2026, 3, 1), "Draft").unwrap();
        s.set_entry_parties(third, &[PartyShare::whole(a), PartyShare::whole(b)])
            .unwrap();
        s.set_entry_party(third, Some(b)).unwrap();
        s.verify().unwrap();
        let live = s.ledger().clone();
        drop(s);
        for db in [true, false] {
            if !db {
                delete_db(&dir);
            }
            let s = Store::open(&dir).unwrap();
            assert!(
                *s.ledger() == live,
                "replay (db {db}): {:?}",
                s.ledger().differs_from(&live)
            );
            assert_eq!(s.ledger().entry_parties(r).len(), 2);
            assert_eq!(s.ledger().entry_party(third), Some(b));
            s.verify().unwrap();
        }
    }
}
