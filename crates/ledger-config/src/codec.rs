//! Settings files as text in and out of the books (plan/served.md 5.4).
//!
//! Books keep their settings in the journal (`ledger_core::settings`).
//! A `wanderbooks.toml` is text to import from and export to: the server
//! never reads one once the books hold their settings.
//!
//! ```text
//! GOAL: move a company's settings file into its books without changing
//!       how a single setting reads, and write them back out as a file.
//! SOURCE: plan/served.md I-3, 5.4 (toml_round_trip_zero_events); the
//!         settings file's own rules (crate::parse).
//! INVARIANTS:
//!   1. An import is exactly the changes that make the books' settings
//!      what the file (and config.json's receipt policy, if given) says:
//!      keys the file sets and the books don't, or set differently, are
//!      set; keys the books set and the file doesn't are cleared. Nothing
//!      else.
//!   2. NEVER an import that reads differently than the file: the
//!      settings after it give the same company, receipt policy, cash
//!      flow roles, asset tag prefix, party policy and time zone that
//!      the file gives the server today (crate::parse,
//!      crate::effective_receipt_policy_text), or the import is refused.
//!   3. Nothing in the file is dropped silently: what isn't a books
//!      setting is a note (where it goes instead) or unknown (not
//!      imported; the caller shows it before anything is journaled).
//!   4. Round trip: importing an export changes nothing (identity values
//!      are exported masked, and a masked value imports as "unchanged").
//! ```

use crate::{BooksSettings, ReceiptPolicy};
use ledger_core::settings::{self, Change, KEYS, Kind, Sensitivity, Settings, Source};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// What importing a settings file would do.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Import {
    /// The changes that make the books' settings what the file says
    /// (INVARIANT 1); empty when they already are.
    pub changes: Vec<Change>,
    /// What the file says that isn't a books setting, and where it goes
    /// instead (INVARIANT 3).
    pub notes: Vec<String>,
    /// Entries the books don't know: not imported (INVARIANT 3).
    pub unknown: Vec<String>,
    /// `[vault] key_file`, if the file sets one: the server's to use
    /// (`--vault-key-file`), not a books setting.
    pub vault_key_file: Option<String>,
    /// The file's comments, in order: a settings file's notes (where a
    /// value came from, what it waits on) are not settings, so they are
    /// listed for the caller to keep elsewhere (a convention, a decision, a
    /// setting's note) rather than dropped unseen (INVARIANT 3).
    pub comments: Vec<String>,
}

/// The comments of TOML text: what follows each `#` outside a string, in
/// order, trimmed, blank ones left out.
pub fn comments(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut quote: Option<char> = None;
        let mut escaped = false;
        for (i, c) in line.char_indices() {
            match quote {
                Some(q) => {
                    if escaped {
                        escaped = false;
                    } else if c == '\\' && q == '"' {
                        escaped = true;
                    } else if c == q {
                        quote = None;
                    }
                }
                None if c == '"' || c == '\'' => quote = Some(c),
                None if c == '#' => {
                    let rest = line[i + 1..].trim();
                    if !rest.is_empty() {
                        out.push(rest.to_string());
                    }
                    break;
                }
                None => {}
            }
        }
    }
    out
}

/// A TOML value as the JSON a setting takes: strings stay strings; a
/// number, boolean or date in a text setting reads as its TOML spelling;
/// arrays and tables convert element by element.
fn json_of(v: &toml::Value) -> Value {
    match v {
        toml::Value::String(s) => json!(s),
        toml::Value::Integer(i) => json!(i.to_string()),
        toml::Value::Float(f) => json!(f.to_string()),
        toml::Value::Boolean(b) => json!(b.to_string()),
        toml::Value::Datetime(d) => json!(d.to_string()),
        toml::Value::Array(a) => Value::Array(a.iter().map(json_of).collect()),
        toml::Value::Table(t) => {
            Value::Object(t.iter().map(|(k, v)| (k.clone(), json_of(v))).collect())
        }
    }
}

fn key_at(table: &str, name: &str) -> Option<&'static settings::Key> {
    KEYS.iter().find(|k| k.toml == (table, name))
}

/// What importing `text` (a `wanderbooks.toml`) and, if given,
/// `config_json` (its `receipt_policy` wins over the file's receipt
/// settings, as it does for the server) would change in books holding
/// `current`, kept in `base_currency`.
pub fn import(
    text: &str,
    config_json: Option<&str>,
    current: &Settings,
    base_currency: &str,
) -> Result<Import, String> {
    // The file as the server reads it today: what the import must match.
    let legacy = crate::parse(text)?;
    let legacy_policy = crate::effective_receipt_policy_text(config_json, &legacy)?;
    let table: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
    let mut out = Import {
        comments: comments(text),
        ..Import::default()
    };
    let mut wanted: BTreeMap<&'static str, Value> = BTreeMap::new();
    for (section, body) in &table {
        let toml::Value::Table(body) = body else {
            out.unknown.push(format!("{section} (outside any table)"));
            continue;
        };
        for (name, v) in body {
            match (section.as_str(), name.as_str()) {
                ("company", "base_currency") => {
                    let c = v.as_str().unwrap_or_default().trim().to_uppercase();
                    if c != base_currency {
                        return Err(format!(
                            "[company] base_currency {c:?}: these books are kept in {base_currency}, fixed when they were created"
                        ));
                    }
                    out.notes.push(format!(
                        "[company] base_currency: the books keep it themselves ({base_currency}, fixed when they were created)"
                    ));
                }
                ("books", "data_dir") => out.notes.push(
                    "[books] data_dir: where the books live is the server's to know, not a setting; dropped".into(),
                ),
                ("vault", "key_file") => {
                    out.vault_key_file = v.as_str().map(str::to_string);
                    out.notes.push(
                        "[vault] key_file: the server's to know (start it with --vault-key-file), not a books setting; dropped".into(),
                    );
                }
                ("viewer", n) => out.notes.push(format!(
                    "[viewer] {n}: the agent's preference, not a books setting; dropped (record it as a convention if it matters)"
                )),
                (s, n) => match key_at(s, n) {
                    Some(k) => {
                        wanted.insert(k.name, json_of(v));
                    }
                    None => out.unknown.push(format!("[{s}] {n}")),
                },
            }
        }
    }
    if let Some(raw) = config_json {
        let j: Value = serde_json::from_str(raw).map_err(|e| format!("bad config: {e}"))?;
        let obj = j.as_object().ok_or("bad config: not a JSON object")?;
        for (k, v) in obj {
            match k.as_str() {
                "receipt_policy" => {
                    // Its policy replaces the file's, whole (crate::
                    // effective_receipt_policy_text).
                    let p: ReceiptPolicy = serde_json::from_value(v.clone())
                        .map_err(|e| format!("bad config: {e}"))?;
                    wanted.insert("policy.evidence.threshold", json!(p.require_receipts_above));
                    wanted.insert(
                        "policy.evidence.required_accounts",
                        json!(p.receipt_required_accounts),
                    );
                    wanted.insert(
                        "policy.evidence.exempt_accounts",
                        json!(p.receipt_exempt_accounts),
                    );
                }
                "base_currency" => {
                    let c = v.as_str().unwrap_or_default().trim().to_uppercase();
                    if c != base_currency {
                        return Err(format!(
                            "config.json base_currency {c:?}: these books are kept in {base_currency}"
                        ));
                    }
                }
                other => out.unknown.push(format!("config.json {other}")),
            }
        }
    }
    // Canonical values, and the diff against what the books hold. An
    // identity value exported masked ("**-***5893") never imports: it is
    // what the books hold already when its last four match, else a note,
    // and either way the books' own value stays (INVARIANT 4).
    let mut canon: BTreeMap<&'static str, Value> = BTreeMap::new();
    let mut keep: Vec<&'static str> = Vec::new();
    for (name, v) in &wanted {
        let k = settings::key(name).expect("taken from KEYS");
        let masked =
            k.sensitivity == Sensitivity::Identity && v.as_str().is_some_and(|s| s.contains('*'));
        if masked {
            let same = current.get(name).is_some_and(|r| {
                v.as_str() == Some(settings::display(k, &r.value, false).as_str())
            });
            if !same {
                out.notes.push(format!(
                    "[{}] {}: masked in this file, so not imported; set it with books.set_setting",
                    k.toml.0, k.toml.1
                ));
            }
            keep.push(k.name);
            continue;
        }
        let value =
            settings::check(k, v).map_err(|e| format!("[{}] {}: {e}", k.toml.0, k.toml.1))?;
        // An empty list is the same as no list: not a change.
        if value.as_array().is_some_and(Vec::is_empty) && k.kind != Kind::Members {
            continue;
        }
        canon.insert(k.name, value);
    }
    for k in KEYS {
        let have = current.get(k.name).map(|r| &r.value);
        match (canon.get(k.name), have) {
            (Some(want), Some(have)) if want == have => {}
            (Some(want), _) => out.changes.push(Change {
                key: k.name.into(),
                value: Some(want.clone()),
            }),
            (None, Some(_)) if keep.contains(&k.name) => {}
            (None, Some(_)) => out.changes.push(Change {
                key: k.name.into(),
                value: None,
            }),
            (None, None) => {}
        }
    }
    // INVARIANT 2: the books, after the import, read as the file does.
    let mut next = current.clone();
    let probe = Source {
        decided_by: "import".into(),
        decided_on: chrono::NaiveDate::from_ymd_opt(2000, 1, 1).expect("a date"),
        citation: None,
        note: None,
    };
    next.apply(&out.changes, &probe, u64::MAX)?;
    next.check_whole()?;
    let ours = books_settings(&next, base_currency);
    let our_policy = receipt_policy(&next);
    let differs = differences(&legacy, &legacy_policy, &ours, &our_policy);
    if !differs.is_empty() {
        return Err(format!(
            "importing would read these settings differently than the file does: {}",
            differs.join(", ")
        ));
    }
    Ok(out)
}

/// Where two readings of the settings differ, by field name.
fn differences(
    a: &BooksSettings,
    ap: &ReceiptPolicy,
    b: &BooksSettings,
    bp: &ReceiptPolicy,
) -> Vec<&'static str> {
    let (ca, cb) = (&a.company, &b.company);
    let threshold = |p: &ReceiptPolicy| crate::decimal(p.require_receipts_above.trim());
    [
        ("company name", ca.name == cb.name),
        (
            "fiscal year start",
            ca.fiscal_year_start == cb.fiscal_year_start,
        ),
        ("members", ca.members == cb.members),
        ("accounting basis", ca.basis == cb.basis),
        ("tax form", ca.tax_form == cb.tax_form),
        ("close_into", ca.close_into == cb.close_into),
        ("close_sweep", ca.close_sweep == cb.close_sweep),
        ("receipt threshold", threshold(ap) == threshold(bp)),
        (
            "receipt required accounts",
            ap.receipt_required_accounts == bp.receipt_required_accounts,
        ),
        (
            "receipt exempt accounts",
            ap.receipt_exempt_accounts == bp.receipt_exempt_accounts,
        ),
        ("cash accounts", a.cash_accounts == b.cash_accounts),
        (
            "financing accounts",
            a.financing_accounts == b.financing_accounts,
        ),
        (
            "investing accounts",
            a.investing_accounts == b.investing_accounts,
        ),
        (
            "asset tag prefix",
            a.asset_tag_prefix() == b.asset_tag_prefix(),
        ),
        ("parties", a.parties == b.parties),
        ("time zone", a.timezone == b.timezone),
    ]
    .into_iter()
    .filter(|(_, same)| !same)
    .map(|(n, _)| n)
    .collect()
}

/// The settings as the server and the viewer use them, from the books'
/// own (`ledger_core::settings`): what [`crate::parse`] gives for a
/// settings file. `[vault] key_file` is never a books setting.
pub fn books_settings(s: &Settings, base_currency: &str) -> BooksSettings {
    BooksSettings {
        company: s.company(base_currency),
        receipt_threshold: s.receipt_threshold().map(str::to_string),
        receipt_required_accounts: s.receipt_required_accounts(),
        receipt_exempt_accounts: s.receipt_exempt_accounts(),
        cash_accounts: s.cash_accounts(),
        financing_accounts: s.financing_accounts(),
        investing_accounts: s.investing_accounts(),
        asset_tag_prefix: s.asset_tag_prefix().map(str::to_string),
        parties: s.parties(),
        vault_key_file: None,
        timezone: s.timezone().map(str::to_string),
    }
}

/// The receipt policy the books' own settings set.
pub fn receipt_policy(s: &Settings) -> ReceiptPolicy {
    ReceiptPolicy {
        require_receipts_above: s
            .receipt_threshold()
            .unwrap_or(settings::DEFAULT_THRESHOLD)
            .to_string(),
        receipt_required_accounts: s.receipt_required_accounts(),
        receipt_exempt_accounts: s.receipt_exempt_accounts(),
    }
}

/// One TOML line `name = value`, the value spelled as TOML.
fn toml_line(name: &str, v: &Value) -> String {
    let tv = match v {
        Value::String(s) => toml::Value::String(s.clone()),
        Value::Array(a) => toml::Value::Array(
            a.iter()
                .map(|x| toml::Value::String(x.as_str().unwrap_or_default().to_string()))
                .collect(),
        ),
        other => toml::Value::String(other.to_string()),
    };
    let mut t = toml::Table::new();
    t.insert(name.to_string(), tv);
    toml::to_string(&t)
        .unwrap_or_default()
        .trim_end()
        .to_string()
}

fn provenance(out: &mut String, r: &settings::Recorded) {
    let s = &r.source;
    let _ = writeln!(
        out,
        "# {}, {} (journal line {})",
        s.decided_by, s.decided_on, r.seq
    );
    for extra in [&s.citation, &s.note].into_iter().flatten() {
        for line in extra.lines() {
            let _ = writeln!(out, "#   {line}");
        }
    }
}

/// The books' settings as a settings file (`wanderbooks.toml` text):
/// every setting set, under its table, with who decided it and when as a
/// comment; identity values masked (INVARIANT 4).
pub fn export(s: &Settings, base_currency: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# wanderbooks settings, exported from the books{}.",
        s.last_seq()
            .map(|q| format!(" (through journal line {q})"))
            .unwrap_or_default()
    );
    out.push_str(
        "# The books keep their own settings: this file is a copy, and editing it\n# changes nothing until it is imported (books.import_settings).\n",
    );
    for table in ["company", "books"] {
        let _ = write!(out, "\n[{table}]\n");
        if table == "company" {
            let _ = writeln!(
                out,
                "# Fixed when the books were created.\nbase_currency = {:?}",
                base_currency
            );
        }
        let mut members = None;
        for (k, r) in s.iter().filter(|(k, _)| k.toml.0 == table) {
            if k.kind == Kind::Members {
                members = Some((k, r));
                continue;
            }
            provenance(&mut out, r);
            let shown = match k.sensitivity {
                Sensitivity::Identity => Value::String(settings::display(k, &r.value, false)),
                Sensitivity::Public => r.value.clone(),
            };
            let _ = writeln!(out, "{}", toml_line(k.toml.1, &shown));
        }
        if let Some((_, r)) = members {
            let _ = writeln!(out);
            provenance(&mut out, r);
            for m in r.value.as_array().into_iter().flatten() {
                out.push_str("[[company.members]]\n");
                for f in [
                    "name",
                    "share",
                    "capital_account",
                    "distribution_account",
                    "files_jointly_with",
                ] {
                    if let Some(v) = m.get(f) {
                        let _ = writeln!(out, "{}", toml_line(f, v));
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FERNHILL: &str = r#"
[company]
name = "Fernhill Labs LLC"
entity_type = "LLC, multi-member, member-managed"
tax_classification = "partnership"
base_currency = "USD"
fiscal_year_start = "01-01"
formation_state = "UT"
formed = "2026-10-04"
state_entity_number = "14745117-0160"
ein = "28-8525893"
irs_name_control = "WALN"
duns = "149976972"
address = "100 Example Way, Midvale, UT 84047"
registered_agent = "Tobin Harlow"
partnership_representative = "Tobin Harlow"
cpa = "Casey Rowe"

[[company.members]]
name = "Tobin Harlow"
share = "50%"
capital_account = "3000"
files_jointly_with = "Morgan Ellis"

[[company.members]]
name = "Morgan Ellis"
share = "50%"
capital_account = "3001"
files_jointly_with = "Tobin Harlow"

[books]
data_dir = "books-data"
accounting_basis = "accrual"
receipt_threshold = "0.00"
cash_accounts = ["1000"]
financing_accounts = ["2100"]
parties = "require"

[viewer]
open_at_start = "ask"
"#;

    fn src() -> Source {
        Source {
            decided_by: "Owner".into(),
            decided_on: chrono::NaiveDate::from_ymd_opt(2026, 10, 17).unwrap(),
            citation: None,
            note: Some("imported".into()),
        }
    }

    fn imported(text: &str) -> Settings {
        let i = import(text, None, &Settings::default(), "USD").unwrap();
        let mut s = Settings::default();
        let changes = s.check_changes(&i.changes, &src()).unwrap();
        s.apply(&changes, &src(), 1).unwrap();
        s
    }

    /// INVARIANTS 1-3 on a real company's file.
    #[test]
    fn a_real_file_imports_whole() {
        let i = import(FERNHILL, None, &Settings::default(), "USD").unwrap();
        assert!(i.unknown.is_empty(), "{:?}", i.unknown);
        assert_eq!(i.notes.len(), 3, "{:?}", i.notes);
        // 14 company facts, the members, and 5 [books] settings.
        assert_eq!(i.changes.len(), 20);
        let s = imported(FERNHILL);
        assert_eq!(s.get("company.ein").unwrap().value, json!("28-8525893"));
        assert_eq!(s.parties(), crate::PartyPolicy::Require);
        // Already imported: nothing to change.
        let again = import(FERNHILL, None, &s, "USD").unwrap();
        assert!(again.changes.is_empty(), "{:?}", again.changes);
    }

    /// INVARIANT 4: an export imports as no change at all.
    #[test]
    fn export_round_trips() {
        let s = imported(FERNHILL);
        let text = export(&s, "USD");
        assert!(text.contains("ein = \"**-***5893\""), "{text}");
        assert!(!text.contains("2215893"), "{text}");
        let back = import(&text, None, &s, "USD").unwrap();
        assert!(back.changes.is_empty(), "{:?}\n{text}", back.changes);
        assert!(back.unknown.is_empty(), "{:?}", back.unknown);
        // And the export reads, as a settings file, as the books do.
        assert_eq!(
            crate::parse(&text).unwrap().company.name,
            "Fernhill Labs LLC"
        );
    }

    /// INVARIANT 1: keys the file leaves out are cleared.
    #[test]
    fn what_the_file_leaves_out_is_cleared() {
        let s = imported(FERNHILL);
        let i = import(
            "[company]\nname = \"Fernhill Labs LLC\"\n",
            None,
            &s,
            "USD",
        )
        .unwrap();
        assert!(
            i.changes
                .iter()
                .any(|c| c.key == "company.members" && c.value.is_none())
        );
        assert!(!i.changes.iter().any(|c| c.key == "company.name"));
    }

    /// INVARIANT 3, and the currency is the books'.
    #[test]
    fn unknown_and_currency() {
        let i = import(
            "[company]\nwebsite = \"x\"\n[extra]\na = 1\n",
            None,
            &Settings::default(),
            "USD",
        )
        .unwrap();
        assert_eq!(i.unknown, ["[company] website", "[extra] a"]);
        assert!(
            import(
                "[company]\nbase_currency = \"EUR\"\n",
                None,
                &Settings::default(),
                "USD"
            )
            .is_err()
        );
    }

    /// config.json's receipt policy wins, whole, as it does for the server.
    #[test]
    fn config_json_policy_wins() {
        let j = r#"{"receipt_policy": {"require_receipts_above": "10.00", "receipt_exempt_accounts": ["6999"]}}"#;
        let i = import(
            "[books]\nreceipt_threshold = \"0.00\"\n",
            Some(j),
            &Settings::default(),
            "USD",
        )
        .unwrap();
        let mut s = Settings::default();
        s.apply(&i.changes, &src(), 1).unwrap();
        let p = receipt_policy(&s);
        assert_eq!(p.require_receipts_above, "10.00");
        assert_eq!(p.receipt_exempt_accounts, ["6999"]);
    }

    /// INVARIANT 2: a value the books would read differently is refused
    /// (here: a time zone the file names that isn't one).
    #[test]
    fn never_reads_differently() {
        assert!(
            import(
                "[company]\ntimezone = \"Nowhere/Land\"\n",
                None,
                &Settings::default(),
                "USD"
            )
            .is_err()
        );
        assert!(
            import(
                "[company]\nfiscal_year_start = \"07-01\"\n",
                None,
                &Settings::default(),
                "USD"
            )
            .is_ok()
        );
    }

    /// A comment is what follows a `#` outside a string: a `#` inside a
    /// basic string (even after an escaped quote) or a literal string
    /// (where a backslash escapes nothing) is text; blank comments are
    /// left out.
    #[test]
    fn comments_are_what_follows_a_hash_outside_strings() {
        let text = "# top\n\
                    name = \"a # not\" # one\n\
                    esc = \"q \\\" # still\" # two\n\
                    lit = 'c:\\' # three\n\
                    #\n\
                    x = 1 #   \n\
                    y = 2#five\n";
        assert_eq!(comments(text), ["top", "one", "two", "three", "five"]);
        assert!(comments("a = 'x' \"y\"\n").is_empty());
    }

    /// The import lists the file's comments (INVARIANT 3).
    #[test]
    fn an_import_lists_the_comments() {
        let i = import(
            "# Kept settings\n[company]\nname = \"X LLC\"  # from the certificate\n",
            None,
            &Settings::default(),
            "USD",
        )
        .unwrap();
        assert_eq!(i.comments, ["Kept settings", "from the certificate"]);
    }

    /// config.json's base_currency must be the books'.
    #[test]
    fn config_json_currency_is_the_books() {
        let ok = import(
            "",
            Some(r#"{"base_currency": " usd "}"#),
            &Settings::default(),
            "USD",
        )
        .unwrap();
        assert!(ok.changes.is_empty() && ok.unknown.is_empty(), "{ok:?}");
        let err = import(
            "",
            Some(r#"{"base_currency": "EUR"}"#),
            &Settings::default(),
            "USD",
        )
        .unwrap_err();
        assert!(err.contains("config.json base_currency \"EUR\""), "{err}");
    }

    /// INVARIANT 4: a masked identity value never imports. It keeps the
    /// books' own value either way, with a note only when its last four
    /// differ from what the books hold.
    #[test]
    fn a_masked_identity_keeps_the_books_value() {
        let s = imported(FERNHILL);
        let masked =
            |ein: &str| FERNHILL.replace("ein = \"28-8525893\"", &format!("ein = \"{ein}\""));
        let note =
            "[company] ein: masked in this file, so not imported; set it with books.set_setting";
        let same = import(&masked("**-***5893"), None, &s, "USD").unwrap();
        assert!(same.changes.is_empty(), "{:?}", same.changes);
        assert!(!same.notes.iter().any(|n| n == note), "{:?}", same.notes);
        let other = import(&masked("**-***0000"), None, &s, "USD").unwrap();
        assert!(other.changes.is_empty(), "{:?}", other.changes);
        assert!(other.notes.iter().any(|n| n == note), "{:?}", other.notes);
        // Into books that hold none: still not imported, and noted.
        let empty = import(
            "[company]\nein = \"**-***5893\"\n",
            None,
            &Settings::default(),
            "USD",
        )
        .unwrap();
        assert!(empty.changes.is_empty(), "{:?}", empty.changes);
        assert_eq!(empty.notes, [note]);
    }

    /// An empty list is no list: not a change.
    #[test]
    fn an_empty_list_is_no_change() {
        let i = import(
            "[books]\ncash_accounts = []\nfinancing_accounts = []\n",
            None,
            &Settings::default(),
            "USD",
        )
        .unwrap();
        assert!(i.changes.is_empty(), "{:?}", i.changes);
    }

    /// A value the file changes is a change, exactly that one.
    #[test]
    fn a_changed_value_is_one_change() {
        let s = imported(FERNHILL);
        let text = FERNHILL.replace("\"Fernhill Labs LLC\"", "\"Other Labs LLC\"");
        let i = import(&text, None, &s, "USD").unwrap();
        assert_eq!(
            i.changes,
            [Change {
                key: "company.name".into(),
                value: Some(json!("Other Labs LLC")),
            }]
        );
    }

    /// INVARIANT 2's check names each field read differently.
    #[test]
    fn differences_name_each_field() {
        let a = BooksSettings::default();
        let p = ReceiptPolicy::default();
        assert!(differences(&a, &p, &a, &p).is_empty());
        let mut b = a.clone();
        b.company.name = "Other".into();
        b.timezone = Some("America/Denver".into());
        let mut q = p.clone();
        q.require_receipts_above = format!("{}0", p.require_receipts_above);
        // The default asset tag prefix is made from the company's name.
        let named = ["company name", "asset tag prefix", "time zone"];
        assert_eq!(differences(&a, &p, &b, &q), named);
        q.require_receipts_above = "1.00".into();
        assert_eq!(
            differences(&a, &p, &b, &q),
            [
                "company name",
                "receipt threshold",
                "asset tag prefix",
                "time zone"
            ]
        );
    }

    /// An export says, for each setting, who decided it, when, and its
    /// note.
    #[test]
    fn an_export_keeps_each_settings_source() {
        let text = export(&imported(FERNHILL), "USD");
        assert!(
            text.contains("\n# Owner, 2026-10-17 (journal line 1)\n#   imported\n"),
            "{text}"
        );
    }
}
