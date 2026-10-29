// Fuzz target: the books' own record of how they are kept (journal format
// 2: settings.set, convention.*, decision.recorded, open_item.*,
// entry.approved), journaled by the real JournalWriter with fuzz-chosen
// keys, values and shapes after a real store and chart.
// Oracles:
//   1. Opening never panics or hangs (an Err is fine: a bad record is
//      refused as corrupt, never misread).
//   2. Books that open verify (live = replay = projection, row for row).
//   3. The viewer (Snapshot::load) accepts exactly the journals the writer
//      does, and then holds the same settings and conventions.
//   4. Whatever opens can be read every way the server and viewer read it:
//      the company, the receipt policy, the dates, the approvals, the
//      settings file export, never a panic.
#![no_main]

use arbitrary::{Arbitrary, Unstructured};
use libfuzzer_sys::fuzz_target;
use serde_json::{Map, Value, json};

const TYPES: &[&str] = &[
    "settings.set",
    "convention.set",
    "convention.retired",
    "decision.recorded",
    "open_item.raised",
    "open_item.resolved",
    "entry.approved",
    "journal.format_set",
];

const KEYS: &[&str] = &[
    "company.name", "company.fiscal_year_start", "company.members", "company.tax_form",
    "company.tax_classification", "company.timezone", "company.ein", "company.formed",
    "books.accounting_basis", "books.cash_accounts", "books.close_into", "books.close_sweep",
    "books.asset_tag_prefix", "policy.evidence.threshold", "policy.evidence.required_accounts",
    "policy.parties", "policy.approval_rules", "no.such.key",
];

fn scalar(u: &mut Unstructured) -> arbitrary::Result<Value> {
    Ok(match u.int_in_range(0..=11)? {
        0 => json!(u.int_in_range(0u64..=5)?),
        1 => json!(format!("{}.{:02}", u.int_in_range(0u32..=999)?, u.int_in_range(0u32..=99)?)),
        11 => json!(*u.choose(&["0.5", "1", "0.3333333333333333333333333333", "79228162514264337593543950335", "-1", "0"])?),
        2 => json!(format!("{:02}-{:02}", u.int_in_range(0u32..=13)?, u.int_in_range(0u32..=32)?)),
        3 => json!(*u.choose(&["America/Denver", "Mars/Base", "UTC", ""])?),
        4 => json!(*u.choose(&["1065", "accrual", "cash", "enforce", "off", "require", "x"])?),
        5 => json!(*u.choose(&["1000", "2100", "3000", "3001", "6000", "9999"])?),
        6 => json!(format!("2026-{:02}-{:02}", u.int_in_range(0u32..=13)?, u.int_in_range(0u32..=32)?)),
        7 => Value::Null,
        8 => json!(bool::arbitrary(u)?),
        _ => json!(String::arbitrary(u)?),
    })
}

fn value(u: &mut Unstructured, depth: u32) -> arbitrary::Result<Value> {
    Ok(match u.int_in_range(0..=4)? {
        0 if depth < 2 => Value::Array(
            (0..u.int_in_range(0..=3)?)
                .map(|_| value(u, depth + 1))
                .collect::<arbitrary::Result<_>>()?,
        ),
        1 if depth < 2 => {
            let mut m = Map::new();
            for _ in 0..u.int_in_range(0..=5)? {
                let k = *u.choose(&[
                    "name", "share", "capital_account", "files_jointly_with", "lines",
                    "account", "side", "approver", "accounts", "amount_over", "description",
                    "share",
                ])?;
                m.insert(k.into(), value(u, depth + 1)?);
            }
            Value::Object(m)
        }
        _ => scalar(u)?,
    })
}

fn source(u: &mut Unstructured) -> arbitrary::Result<Value> {
    Ok(match u.int_in_range(0..=5)? {
        0 => value(u, 1)?,
        _ => json!({"decided_by": String::arbitrary(u)?, "decided_on": scalar(u)?}),
    })
}

fn payload(u: &mut Unstructured, t: &str) -> arbitrary::Result<Value> {
    let mut m = Map::new();
    match t {
        "settings.set" => {
            let changes: Vec<Value> = (0..u.int_in_range(0..=4)?)
                .map(|_| Ok(json!({"key": *u.choose(KEYS)?, "value": value(u, 0)?})))
                .collect::<arbitrary::Result<_>>()?;
            m.insert("changes".into(), json!(changes));
        }
        "convention.set" => {
            m.insert("id".into(), json!(format!("c{}", u.int_in_range(0..=3)?)));
            m.insert(
                "kind".into(),
                json!(*u.choose(&["posting_template", "approval_rule", "note", "nope"])?),
            );
            m.insert("title".into(), json!(String::arbitrary(u)?));
            m.insert("body_md".into(), json!(String::arbitrary(u)?));
            m.insert("structured".into(), value(u, 0)?);
        }
        "convention.retired" => {
            m.insert("id".into(), json!(format!("c{}", u.int_in_range(0..=3)?)));
            m.insert("reason".into(), json!(String::arbitrary(u)?));
        }
        "decision.recorded" => {
            m.insert("id".into(), json!(u.int_in_range(0u64..=4)?));
            m.insert("subject".into(), json!(String::arbitrary(u)?));
            m.insert("decision_md".into(), json!(String::arbitrary(u)?));
            m.insert("decided_by".into(), value(u, 0)?);
            m.insert("decided_on".into(), scalar(u)?);
            m.insert("supersedes".into(), scalar(u)?);
        }
        "open_item.raised" | "open_item.resolved" => {
            m.insert("id".into(), json!(u.int_in_range(0u64..=4)?));
            m.insert("title".into(), json!(String::arbitrary(u)?));
            m.insert("resolution".into(), json!(String::arbitrary(u)?));
            m.insert("due".into(), scalar(u)?);
        }
        "entry.approved" => {
            m.insert("entry_id".into(), json!(u.int_in_range(0u64..=3)?));
            m.insert("approver".into(), json!(String::arbitrary(u)?));
            m.insert("fingerprint".into(), scalar(u)?);
        }
        _ => {
            m.insert("format".into(), json!(u.int_in_range(0u64..=3)?));
            m.insert("features".into(), json!([]));
        }
    }
    if u.ratio(4, 5)? {
        m.insert("source".into(), source(u)?);
    }
    Ok(Value::Object(m))
}

fuzz_target!(|data: &[u8]| {
    let mut u = Unstructured::new(data);
    let dir = std::env::temp_dir().join(format!("wb-fuzz-settings-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    {
        // Real books first: a chart and a draft for approvals and
        // templates to name.
        let mut s = ledger_store::Store::create(&dir, ledger_core::Currency::USD).unwrap();
        for (code, t) in [
            ("1000", ledger_core::AccountType::Asset),
            ("3000", ledger_core::AccountType::Equity),
            ("3001", ledger_core::AccountType::Equity),
            ("6000", ledger_core::AccountType::Expense),
        ] {
            s.add_account(code, code, t, None, false).unwrap();
        }
        s.create_entry(chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(), "draft")
            .unwrap();
        // Members the settings may name, for the figures that use shares.
        for name in ["A", "B"] {
            s.add_party(name, &[ledger_core::Role::Member]).unwrap();
        }
    }
    {
        let mut w =
            ledger_store::JournalWriter::open(&dir.join(ledger_store::JOURNAL_FILE)).unwrap();
        for _ in 0..u.int_in_range(0..=16).unwrap_or(0) {
            let Ok(t) = u.choose(TYPES) else { break };
            let Ok(p) = payload(&mut u, t) else { break };
            w.append(t, p).unwrap();
        }
    }
    let _ = std::fs::remove_file(dir.join(ledger_store::db_file()));
    let store = ledger_store::Store::open(&dir);
    let snap = ledger_store::Snapshot::load(&dir);
    match (store, snap) {
        (Ok(store), Ok(snap)) => {
            store.verify().expect("books that open must verify");
            let l = store.ledger();
            assert_eq!(l.settings(), snap.ledger().settings());
            assert_eq!(l.conventions(), snap.ledger().conventions());
            // Every way the books' record is read.
            let s = l.settings();
            let c = s.company("USD");
            let _ = c.fiscal_year_containing(chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap());
            let _ = c.party_members(l);
            // The statement of members' equity over the members as set.
            let caps: Vec<ledger_core::MemberCapital> = c
                .members
                .iter()
                .filter_map(|m| {
                    Some(ledger_core::MemberCapital {
                        name: m.name.clone(),
                        capital_account: l.chart().find_by_code(&m.capital_account)?.id,
                        distribution_account: None,
                        share: m.share,
                    })
                })
                .collect();
            let d = |y, m, day| chrono::NaiveDate::from_ymd_opt(y, m, day).unwrap();
            let _ = ledger_core::members_equity(l, d(2026, 1, 1), d(2026, 12, 31), &caps);
            let _ = ledger_config::codec::books_settings(s, "USD").asset_tag_prefix();
            let _ = ledger_config::codec::receipt_policy(s).threshold();
            let _ = ledger_config::codec::export(s, "USD");
            let _ = s.today();
            let _ = s.check_whole();
            for (k, r) in s.iter() {
                let _ = ledger_core::settings::display(k, &r.value, false);
            }
            for e in 1..l.next_entry_id() {
                let _ = l.approvals_missing(e);
            }
            for conv in l.conventions().conventions() {
                let _ = conv.summary();
                let slots = [("expense".to_string(), "6000".to_string())].into();
                let _ = l.template_lines(&conv.id, rust_decimal::Decimal::new(10001, 2), &slots);
                let _ = l.template_lines(&conv.id, rust_decimal::Decimal::new(10001, 2), &Default::default());
            }
        }
        (Err(a), Err(b)) => {
            let newer = |e: &ledger_store::StoreError| {
                matches!(e, ledger_store::StoreError::NewerJournal { .. })
            };
            assert_eq!(newer(&a), newer(&b), "the writer says {a}; the viewer says {b}");
        }
        (Ok(_), Err(e)) => panic!("the writer opens books the viewer refuses: {e}"),
        (Err(e), Ok(_)) => panic!("the viewer loads books the writer refuses: {e}"),
    }
});
