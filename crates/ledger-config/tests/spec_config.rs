//! Spec-blind tests of `ledger_config`: every expected value here comes from
//! the doc comments and the GOAL/SOURCE/INVARIANTS block on `parse`, the
//! public types' docs, `ledger_core::TaxForm`'s docs, and the vendored IRS
//! sources (docs/spec/sources/irs-periods-and-methods.md,
//! irs-entity-classification.md, irs-partnerships.md). Where the spec is
//! silent, nothing is asserted.

use chrono::{Datelike, Months, NaiveDate};
use ledger_config::{BooksSettings, load, parse};
use ledger_core::TaxForm;
use proptest::prelude::*;
use rust_decimal::Decimal;
use std::str::FromStr;

fn ok(text: &str) -> BooksSettings {
    match parse(text) {
        Ok(s) => s,
        Err(e) => panic!("expected Ok for:\n{text}\ngot Err: {e}"),
    }
}

fn refused(text: &str) {
    assert!(parse(text).is_err(), "expected Err for:\n{text}");
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// `[company]` with one string setting.
fn company_kv(key: &str, value: &str) -> String {
    format!("[company]\n{key} = {}\n", toml_str(value))
}

/// `[books]` with one string setting.
fn books_kv(key: &str, value: &str) -> String {
    format!("[books]\n{key} = {}\n", toml_str(value))
}

/// A TOML basic string literal.
fn toml_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn toml_array(items: &[String]) -> String {
    let parts: Vec<String> = items.iter().map(|s| toml_str(s)).collect();
    format!("[{}]", parts.join(", "))
}

/// A members table: (name, share, capital account).
fn members_toml(members: &[(String, String, String)]) -> String {
    let mut s = String::from("[company]\nname = \"Co\"\n");
    for (name, share, cap) in members {
        s.push_str(&format!(
            "\n[[company.members]]\nname = {}\nshare = {}\ncapital_account = {}\n",
            toml_str(name),
            toml_str(share),
            toml_str(cap)
        ));
    }
    s
}

fn two_members(a_share: &str, b_share: &str) -> String {
    members_toml(&[
        ("A".into(), a_share.into(), "3000".into()),
        ("B".into(), b_share.into(), "3001".into()),
    ])
}

fn classification(text: &str) -> Result<Option<TaxForm>, String> {
    parse(&company_kv("tax_classification", text)).map(|s| s.company.tax_form)
}

// ---------------------------------------------------------------------------
// INVARIANT 1: every setting optional; empty file defaults.
// ---------------------------------------------------------------------------

/// INVARIANT 1: an empty file is a calendar-year, accrual-basis
/// "Unnamed company" with no members.
#[test]
fn inv1_empty_file_defaults() {
    let s = ok("");
    assert_eq!(s.company.name, "Unnamed company");
    assert_eq!(s.company.fiscal_year_start, (1, 1));
    assert_eq!(s.company.basis, "Accrual");
    assert!(s.company.members.is_empty());
}

/// INVARIANT 1: with nothing set, the optional settings are unset (no tax
/// form known, no threshold, no close_into, no account roles).
#[test]
fn inv1_empty_file_leaves_options_unset() {
    let s = ok("");
    assert_eq!(s.company.tax_form, None);
    assert_eq!(s.company.close_into, None);
    assert!(s.company.close_sweep.is_empty());
    assert_eq!(s.receipt_threshold, None);
    assert!(s.cash_accounts.is_empty());
    assert!(s.financing_accounts.is_empty());
    assert!(s.investing_accounts.is_empty());
}

/// INVARIANT 1: empty `[company]` and `[books]` tables read as the defaults.
#[test]
fn inv1_empty_tables_are_defaults() {
    assert_eq!(ok("[company]\n[books]\n"), ok(""));
}

/// INVARIANT 1: setting only the name leaves the other defaults in place.
#[test]
fn inv1_name_only() {
    let s = ok(&company_kv("name", "Acme LLC"));
    assert_eq!(s.company.name, "Acme LLC");
    assert_eq!(s.company.fiscal_year_start, (1, 1));
    assert_eq!(s.company.basis, "Accrual");
}

/// Company doc: basis is "Accrual" or "Cash"; mcp-interface: the file says
/// "accrual" or "cash".
#[test]
fn basis_accrual_and_cash() {
    assert_eq!(
        ok(&books_kv("accounting_basis", "accrual")).company.basis,
        "Accrual"
    );
    assert_eq!(
        ok(&books_kv("accounting_basis", "cash")).company.basis,
        "Cash"
    );
}

/// Company doc: `profile` is every simple `[company]` setting, in file
/// order, as written.
#[test]
fn profile_in_file_order_as_written() {
    let s = ok("[company]\n\
         name = \"Acme LLC\"\n\
         ein = \"00-0000000\"\n\
         formed = \"2025-06-02\"\n\
         address = \"1 Main St\"\n");
    let want: Vec<(String, String)> = [
        ("name", "Acme LLC"),
        ("ein", "00-0000000"),
        ("formed", "2025-06-02"),
        ("address", "1 Main St"),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    assert_eq!(s.company.profile, want);
}

// ---------------------------------------------------------------------------
// INVARIANT 2: fiscal_year_start is the first of a month.
// ---------------------------------------------------------------------------

/// INVARIANT 2: every "MM-01" is accepted as (MM, 1).
#[test]
fn inv2_first_of_each_month_accepted() {
    for m in 1..=12u32 {
        let s = ok(&company_kv("fiscal_year_start", &format!("{m:02}-01")));
        assert_eq!(s.company.fiscal_year_start, (m, 1));
    }
}

/// INVARIANT 2 (IRS Pub. 538: a fiscal year ends on the last day of a
/// month): any day other than the 1st is refused.
#[test]
fn inv2_not_first_of_month_refused() {
    for v in ["01-02", "07-15", "12-31", "02-28", "06-30"] {
        refused(&company_kv("fiscal_year_start", v));
    }
}

/// INVARIANT 2: a month that doesn't exist is refused.
#[test]
fn inv2_bad_month_refused() {
    for v in ["00-01", "13-01", "99-01"] {
        refused(&company_kv("fiscal_year_start", v));
    }
}

/// INVARIANT 2 / 7: text that isn't "MM-DD" is refused.
#[test]
fn inv2_garbage_refused() {
    for v in ["", "January", "01/01", "2026-01-01", "xx-01"] {
        refused(&company_kv("fiscal_year_start", v));
    }
}

/// fiscal_year_containing doc + Pub. 538: a calendar year is Jan 1 to
/// Dec 31.
#[test]
fn fiscal_year_calendar() {
    let c = ok("").company;
    assert_eq!(
        c.fiscal_year_containing(date(2026, 10, 14)),
        (date(2026, 1, 1), date(2026, 12, 31))
    );
    assert_eq!(
        c.fiscal_year_containing(date(2026, 1, 1)),
        (date(2026, 1, 1), date(2026, 12, 31))
    );
    assert_eq!(
        c.fiscal_year_containing(date(2026, 12, 31)),
        (date(2026, 1, 1), date(2026, 12, 31))
    );
}

/// fiscal_year_containing doc + Pub. 538: a July start runs to June 30
/// (last day of a month), and a date before July belongs to the year that
/// started the previous July.
#[test]
fn fiscal_year_july_start() {
    let c = ok(&company_kv("fiscal_year_start", "07-01")).company;
    assert_eq!(
        c.fiscal_year_containing(date(2026, 3, 1)),
        (date(2025, 7, 1), date(2026, 6, 30))
    );
    assert_eq!(
        c.fiscal_year_containing(date(2026, 6, 30)),
        (date(2025, 7, 1), date(2026, 6, 30))
    );
    assert_eq!(
        c.fiscal_year_containing(date(2026, 7, 1)),
        (date(2026, 7, 1), date(2027, 6, 30))
    );
}

/// A March start ends on the last day of February, Feb 29 in a leap year.
#[test]
fn fiscal_year_march_start_leap() {
    let c = ok(&company_kv("fiscal_year_start", "03-01")).company;
    assert_eq!(
        c.fiscal_year_containing(date(2024, 2, 29)),
        (date(2023, 3, 1), date(2024, 2, 29))
    );
    assert_eq!(
        c.fiscal_year_containing(date(2025, 2, 1)),
        (date(2024, 3, 1), date(2025, 2, 28))
    );
}

// ---------------------------------------------------------------------------
// INVARIANT 3: members.
// ---------------------------------------------------------------------------

/// INVARIANT 3: two members, 50/50, read in order with their accounts.
#[test]
fn inv3_two_members_accepted() {
    let s = ok(&two_members("50%", "50%"));
    let m = &s.company.members;
    assert_eq!(m.len(), 2);
    assert_eq!(m[0].name, "A");
    assert_eq!(m[0].share, dec("0.5"));
    assert_eq!(m[0].capital_account, "3000");
    assert_eq!(m[0].distribution_account, None);
    assert_eq!(m[1].name, "B");
    assert_eq!(m[1].share, dec("0.5"));
    assert_eq!(m[1].capital_account, "3001");
}

/// INVARIANT 3: shares as "50%" or "0.5" read the same.
#[test]
fn inv3_percent_and_decimal_agree() {
    assert_eq!(
        ok(&two_members("50%", "50%")),
        ok(&two_members("0.5", "0.5"))
    );
    assert_eq!(
        ok(&two_members("25%", "0.75")).company.members[1].share,
        dec("0.75")
    );
    assert_eq!(
        ok(&two_members("25%", "0.75")).company.members[0].share,
        dec("0.25")
    );
}

/// INVARIANT 3: shares that don't sum to exactly 1 are refused.
#[test]
fn inv3_shares_must_sum_to_one() {
    refused(&two_members("50%", "49%"));
    refused(&two_members("0.5", "0.51"));
    refused(&two_members("0.5", "0.499999"));
    refused(&two_members("60%", "60%"));
}

/// INVARIANT 3: each share must be in (0, 1]: zero, negative, or above 1
/// is refused even when the sum is 1.
#[test]
fn inv3_share_out_of_range_refused() {
    refused(&two_members("0", "1"));
    refused(&two_members("0%", "100%"));
    refused(&two_members("1.5", "-0.5"));
    refused(&two_members("150%", "-50%"));
}

/// INVARIANT 3: a share of exactly 1 is in (0, 1].
#[test]
fn inv3_single_member_full_share() {
    let s = ok(&members_toml(&[(
        "Solo".into(),
        "100%".into(),
        "3000".into(),
    )]));
    assert_eq!(s.company.members.len(), 1);
    assert_eq!(s.company.members[0].share, Decimal::ONE);
}

/// INVARIANT 3 / 7: a share that is neither "NN%" nor a decimal is refused.
#[test]
fn inv3_share_garbage_refused() {
    refused(&two_members("half", "0.5"));
    refused(&two_members("", "1"));
}

/// INVARIANT 3: a member without a name is refused.
#[test]
fn inv3_member_needs_name() {
    refused(
        "[[company.members]]\nshare = \"50%\"\ncapital_account = \"3000\"\n\
         [[company.members]]\nname = \"B\"\nshare = \"50%\"\ncapital_account = \"3001\"\n",
    );
}

/// INVARIANT 3: a member without a capital account is refused.
#[test]
fn inv3_member_needs_capital_account() {
    refused(
        "[[company.members]]\nname = \"A\"\nshare = \"50%\"\n\
         [[company.members]]\nname = \"B\"\nshare = \"50%\"\ncapital_account = \"3001\"\n",
    );
}

/// INVARIANT 3: member names must be distinct.
#[test]
fn inv3_duplicate_names_refused() {
    refused(&members_toml(&[
        ("A".into(), "50%".into(), "3000".into()),
        ("A".into(), "50%".into(), "3001".into()),
    ]));
}

/// INVARIANT 3: capital accounts must be distinct.
#[test]
fn inv3_duplicate_capital_accounts_refused() {
    refused(&members_toml(&[
        ("A".into(), "50%".into(), "3000".into()),
        ("B".into(), "50%".into(), "3000".into()),
    ]));
}

/// INVARIANT 3: no member's distributions account may be another's capital
/// account.
#[test]
fn inv3_distribution_into_other_capital_refused() {
    refused(
        "[[company.members]]\nname = \"A\"\nshare = \"50%\"\ncapital_account = \"3000\"\n\
         distribution_account = \"3001\"\n\
         [[company.members]]\nname = \"B\"\nshare = \"50%\"\ncapital_account = \"3001\"\n",
    );
}

/// INVARIANT 3 + Member doc: a separate distributions account is read.
#[test]
fn inv3_distribution_account_read() {
    let s = ok(
        "[[company.members]]\nname = \"A\"\nshare = \"50%\"\ncapital_account = \"3000\"\n\
         distribution_account = \"3100\"\n\
         [[company.members]]\nname = \"B\"\nshare = \"50%\"\ncapital_account = \"3001\"\n\
         distribution_account = \"3101\"\n",
    );
    assert_eq!(
        s.company.members[0].distribution_account.as_deref(),
        Some("3100")
    );
    assert_eq!(
        s.company.members[1].distribution_account.as_deref(),
        Some("3101")
    );
}

// ---------------------------------------------------------------------------
// INVARIANT 4: tax form and classification.
// ---------------------------------------------------------------------------

/// INVARIANT 4 + TaxForm::parse doc: tax_form takes "schedule_c", "1065",
/// "1120s", "1120" and the labels.
#[test]
fn inv4_tax_form_values() {
    let cases = [
        ("schedule_c", TaxForm::ScheduleC),
        ("1065", TaxForm::Form1065),
        ("1120s", TaxForm::Form1120S),
        ("1120", TaxForm::Form1120),
        ("Schedule C", TaxForm::ScheduleC),
        ("Form 1065", TaxForm::Form1065),
        ("Form 1120-S", TaxForm::Form1120S),
        ("Form 1120", TaxForm::Form1120),
    ];
    for (text, want) in cases {
        let s = ok(&company_kv("tax_form", text));
        assert_eq!(s.company.tax_form, Some(want), "{text}");
    }
}

/// INVARIANT 4: tax_form wins over tax_classification.
#[test]
fn inv4_tax_form_wins() {
    let s = ok("[company]\ntax_form = \"1120s\"\ntax_classification = \"partnership\"\n");
    assert_eq!(s.company.tax_form, Some(TaxForm::Form1120S));
    let s = ok("[company]\ntax_classification = \"partnership\"\ntax_form = \"schedule_c\"\n");
    assert_eq!(s.company.tax_form, Some(TaxForm::ScheduleC));
}

/// INVARIANT 4 (source table): each row's phrasings map to its return.
#[test]
fn inv4_classification_table() {
    for (text, want) in CLASSIFICATIONS {
        assert_eq!(classification(text), Ok(Some(*want)), "{text}");
    }
}

/// INVARIANT 4: read by whole words, so "business corporation" (which
/// contains the letters "s corporation") is a corporation, Form 1120.
#[test]
fn inv4_business_corporation_is_1120() {
    assert_eq!(
        classification("business corporation"),
        Ok(Some(TaxForm::Form1120))
    );
}

/// INVARIANT 4 (whole words): "business corp" is not an S corporation.
#[test]
fn inv4_business_corp_not_s_corp() {
    assert_ne!(
        classification("business corp"),
        Ok(Some(TaxForm::Form1120S))
    );
}

/// INVARIANT 4: "corp" alone is in the table's Form 1120 row (with
/// "corporation" and "association"), so "business corp" is Form 1120.
#[test]
fn inv4_bare_corp_is_1120() {
    assert_eq!(classification("business corp"), Ok(Some(TaxForm::Form1120)));
}

/// INVARIANT 4: "corporation" or "association" (Form 1120) beside an S
/// corporation phrase (Form 1120-S) names two returns: refused.
#[test]
fn inv4_corporation_beside_s_corporation_refused() {
    for t in [
        "corporation or S corporation",
        "S corporation or corporation",
        "S corporation, formerly a corporation",
        "association or S corp",
        "S-corp association",
        "scorp corporation",
    ] {
        assert!(classification(t).is_err(), "{t}");
    }
}

/// INVARIANT 4 (whole words): "partnerships" is not the word
/// "partnership", and nothing else in the table matches, so it names no
/// return: refused. (A decision from the spec's "read by whole words".)
#[test]
fn inv4_partnerships_plural_refused() {
    assert!(classification("partnerships").is_err());
}

/// INVARIANT 4: "multi-member LLC" contains no table word ("single-member"
/// is the table's word, and the source's default classification of a
/// multi-member LLC is not something the table reads).
#[test]
fn inv4_multi_member_refused() {
    assert!(classification("multi-member LLC").is_err());
}

/// INVARIANT 4: two words for the same return still name exactly one.
#[test]
fn inv4_two_words_same_return() {
    assert_eq!(
        classification("disregarded entity, sole proprietorship"),
        Ok(Some(TaxForm::ScheduleC))
    );
}

/// INVARIANT 4: a classification naming two returns is refused.
#[test]
fn inv4_two_returns_refused() {
    for t in [
        "partnership or S corporation",
        "partnership, corporation",
        "single-member association",
        "S corp or C corporation",
    ] {
        assert!(classification(t).is_err(), "{t}");
    }
}

/// INVARIANT 8: an unknown classification is refused.
#[test]
fn inv8_unknown_classification_refused() {
    for t in ["nonprofit", "trust", "LLC", "cooperative"] {
        assert!(classification(t).is_err(), "{t}");
    }
}

/// INVARIANT 8: an unknown tax form is refused, even with a valid
/// classification beside it.
#[test]
fn inv8_unknown_tax_form_refused() {
    refused(&company_kv("tax_form", "990"));
    refused(&company_kv("tax_form", "1040-ES"));
    refused("[company]\ntax_form = \"990\"\ntax_classification = \"partnership\"\n");
}

/// INVARIANT 8: an unknown accounting basis is refused.
#[test]
fn inv8_unknown_basis_refused() {
    for b in ["modified cash", "tax", "", "hybrid"] {
        refused(&books_kv("accounting_basis", b));
    }
}

// ---------------------------------------------------------------------------
// INVARIANT 5: receipt_threshold.
// ---------------------------------------------------------------------------

/// INVARIANT 5: a decimal at least zero is accepted, zero included.
#[test]
fn inv5_threshold_accepted() {
    for t in ["0.00", "0", "25.00", "75", "1000.5"] {
        let s = ok(&books_kv("receipt_threshold", t));
        let got = s.receipt_threshold.expect("threshold set");
        assert_eq!(dec(&got), dec(t), "{t}");
    }
}

/// INVARIANT 5: a negative threshold is refused.
#[test]
fn inv5_negative_refused() {
    refused(&books_kv("receipt_threshold", "-0.01"));
    refused(&books_kv("receipt_threshold", "-25"));
}

/// INVARIANT 5 (and the MCP contract: decimal strings, never
/// locale-formatted): non-decimals are refused.
#[test]
fn inv5_not_decimal_refused() {
    for t in ["", "abc", "$25", "1,000", "twenty", "25 dollars"] {
        refused(&books_kv("receipt_threshold", t));
    }
}

// ---------------------------------------------------------------------------
// INVARIANT 6: account codes.
// ---------------------------------------------------------------------------

/// INVARIANT 6: cash, financing, and investing codes are trimmed.
#[test]
fn inv6_role_codes_trimmed() {
    let s = ok("[books]\n\
         cash_accounts = [\" 1000 \", \"1010\"]\n\
         financing_accounts = [\"\\t2100\"]\n\
         investing_accounts = [\"1500  \"]\n");
    assert_eq!(s.cash_accounts, vec!["1000", "1010"]);
    assert_eq!(s.financing_accounts, vec!["2100"]);
    assert_eq!(s.investing_accounts, vec!["1500"]);
}

/// INVARIANT 6: close_into and close_sweep are trimmed.
#[test]
fn inv6_close_codes_trimmed() {
    let s = ok("[books]\nclose_into = \" 3000 \"\nclose_sweep = [\" 3100\", \"3200 \"]\n");
    assert_eq!(s.company.close_into.as_deref(), Some("3000"));
    assert_eq!(s.company.close_sweep, vec!["3100", "3200"]);
}

/// INVARIANT 6: a blank code is refused in every list and in close_into.
#[test]
fn inv6_blank_code_refused() {
    for key in [
        "cash_accounts",
        "financing_accounts",
        "investing_accounts",
        "close_sweep",
    ] {
        refused(&format!("[books]\n{key} = [\"\"]\n"));
        refused(&format!("[books]\n{key} = [\"1000\", \"   \"]\n"));
    }
    refused(&books_kv("close_into", ""));
    refused(&books_kv("close_into", "  "));
}

/// INVARIANT 6: close_into is refused with members.
#[test]
fn inv6_close_into_with_members_refused() {
    let mut t = two_members("50%", "50%");
    t.push_str("\n[books]\nclose_into = \"3000\"\n");
    refused(&t);
}

// ---------------------------------------------------------------------------
// Real files.
// ---------------------------------------------------------------------------

/// A copy of the Wanderland Labs books dir's `wanderbooks.toml`.
const WANDERLAND: &str = r#"[company]
name = "Wanderland Labs LLC"
entity_type = "LLC, multi-member, member-managed"
tax_classification = "partnership"  # Form 1065; a Schedule K-1 per member
base_currency = "USD"
fiscal_year_start = "01-01"         # the year ends the day before: Dec 31
formation_state = "UT"
formed = "2026-10-04"

[[company.members]]
name = "Stephen Waits"
share = "50%"
capital_account = "3000"

[[company.members]]
name = "Katherine Pendergast"
share = "50%"
capital_account = "3001"

[books]
data_dir = "books-data"
accounting_basis = "accrual"
receipt_threshold = "0.00"
cash_accounts = ["1000"]
financing_accounts = ["2100"]

[viewer]
open_at_start = "ask"
"#;

/// The demo's `wanderbooks.toml` (ledger-gui demo).
const DEMO: &str = r#"[company]
name = "Juniper Studio LLC"
entity_type = "LLC, single-member"
tax_classification = "disregarded entity"  # Schedule C
fiscal_year_start = "01-01"
formation_state = "OR"
formed = "2025-06-02"
ein = "00-0000000"
address = "1200 NW Everett St, Portland, OR 97209"
owner = "Alex Rivera"

[books]
accounting_basis = "accrual"
receipt_threshold = "25.00"
close_into = "3000"
close_sweep = ["3100"]
cash_accounts = ["1000", "1010"]
"#;

/// Every invariant at once, on a real multi-member file.
#[test]
fn real_wanderland_file() {
    let s = ok(WANDERLAND);
    assert_eq!(s.company.name, "Wanderland Labs LLC");
    assert_eq!(s.company.fiscal_year_start, (1, 1));
    assert_eq!(s.company.basis, "Accrual");
    assert_eq!(s.company.tax_form, Some(TaxForm::Form1065));
    assert_eq!(s.company.members.len(), 2);
    assert_eq!(s.company.members[0].name, "Stephen Waits");
    assert_eq!(s.company.members[1].capital_account, "3001");
    assert_eq!(dec(s.receipt_threshold.as_deref().unwrap()), Decimal::ZERO);
    assert_eq!(s.cash_accounts, vec!["1000"]);
    assert_eq!(s.financing_accounts, vec!["2100"]);
    assert_eq!(s.company.close_into, None);
    let keys: Vec<&str> = s.company.profile.iter().map(|(k, _)| k.as_str()).collect();
    for k in ["name", "entity_type", "formation_state", "formed"] {
        assert!(keys.contains(&k), "{k} missing from {keys:?}");
    }
    assert!(
        s.company
            .profile
            .contains(&("formed".to_string(), "2026-10-04".to_string()))
    );
}

/// Every invariant at once, on a real single-owner file.
#[test]
fn real_demo_file() {
    let s = ok(DEMO);
    assert_eq!(s.company.name, "Juniper Studio LLC");
    assert_eq!(s.company.tax_form, Some(TaxForm::ScheduleC));
    assert!(s.company.members.is_empty());
    assert_eq!(s.company.close_into.as_deref(), Some("3000"));
    assert_eq!(s.company.close_sweep, vec!["3100"]);
    assert_eq!(s.cash_accounts, vec!["1000", "1010"]);
    assert_eq!(dec(s.receipt_threshold.as_deref().unwrap()), dec("25"));
}

// ---------------------------------------------------------------------------
// load.
// ---------------------------------------------------------------------------

/// `load` on a missing path is an error.
#[test]
fn load_missing_path_errors() {
    let dir = tempfile::tempdir().unwrap();
    assert!(load(&dir.path().join("nope/wanderbooks.toml")).is_err());
}

/// `load` reads a file the same as `parse` reads its text, refusals
/// included.
#[test]
fn load_matches_parse() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("wanderbooks.toml");
    for text in [WANDERLAND, DEMO, ""] {
        std::fs::write(&p, text).unwrap();
        assert_eq!(load(&p), parse(text));
    }
    std::fs::write(&p, two_members("50%", "49%")).unwrap();
    assert!(load(&p).is_err());
}

// ---------------------------------------------------------------------------
// Properties.
// ---------------------------------------------------------------------------

/// The table, every phrasing (source table and the doc on the reader).
const CLASSIFICATIONS: &[(&str, TaxForm)] = &[
    ("partnership", TaxForm::Form1065),
    ("S corporation", TaxForm::Form1120S),
    ("S corp", TaxForm::Form1120S),
    ("S-corp", TaxForm::Form1120S),
    ("scorp", TaxForm::Form1120S),
    ("C corporation", TaxForm::Form1120),
    ("corporation", TaxForm::Form1120),
    ("association", TaxForm::Form1120),
    ("disregarded entity", TaxForm::ScheduleC),
    ("disregarded", TaxForm::ScheduleC),
    ("sole proprietor", TaxForm::ScheduleC),
    ("sole proprietorship", TaxForm::ScheduleC),
    ("single-member", TaxForm::ScheduleC),
];

/// Filler words that name no return.
const FILLER: &[&str] = &[
    "LLC",
    "taxed",
    "as",
    "a",
    "an",
    "we",
    "are",
    "elected",
    "to",
    "be",
    "for",
    "federal",
    "tax",
    "purposes",
    "the",
    "company",
    "is",
    "filed",
    "(default)",
    "Utah",
    "member-managed",
];

fn filler() -> impl Strategy<Value = Vec<&'static str>> {
    prop::collection::vec(prop::sample::select(FILLER), 0..4)
}

/// n positive basis-point shares summing to 10_000.
fn bps_shares() -> impl Strategy<Value = Vec<u32>> {
    (1usize..=10).prop_flat_map(|n| {
        prop::sample::subsequence((1u32..10_000).collect::<Vec<_>>(), n - 1).prop_map(|mut cuts| {
            cuts.sort_unstable();
            let mut out = Vec::new();
            let mut prev = 0;
            for c in cuts.into_iter().chain([10_000]) {
                out.push(c - prev);
                prev = c;
            }
            out
        })
    })
}

/// n positive whole percents summing to 100.
fn pct_shares() -> impl Strategy<Value = Vec<u32>> {
    (1usize..=10).prop_flat_map(|n| {
        prop::sample::subsequence((1u32..100).collect::<Vec<_>>(), n - 1).prop_map(|mut cuts| {
            cuts.sort_unstable();
            let mut out = Vec::new();
            let mut prev = 0;
            for c in cuts.into_iter().chain([100]) {
                out.push(c - prev);
                prev = c;
            }
            out
        })
    })
}

fn members_from(shares: &[String]) -> String {
    let m: Vec<(String, String, String)> = shares
        .iter()
        .enumerate()
        .map(|(i, s)| (format!("Member {i}"), s.clone(), format!("30{i:02}")))
        .collect();
    members_toml(&m)
}

fn bps_str(b: u32) -> String {
    (Decimal::from(b) / Decimal::from(10_000)).to_string()
}

const KEYS_COMPANY: &[&str] = &[
    "name",
    "fiscal_year_start",
    "tax_form",
    "tax_classification",
    "ein",
    "entity_type",
];
const KEYS_BOOKS: &[&str] = &[
    "accounting_basis",
    "receipt_threshold",
    "cash_accounts",
    "financing_accounts",
    "investing_accounts",
    "close_into",
    "close_sweep",
];
const KEYS_MEMBER: &[&str] = &["name", "share", "capital_account", "distribution_account"];

/// A TOML value of any shape, biased toward ones that are near-valid.
fn toml_value() -> impl Strategy<Value = String> {
    let s = prop_oneof![
        any::<String>(),
        "[0-9]{0,2}-[0-9]{0,2}",
        "-?[0-9]{0,4}(\\.[0-9]{0,3})?%?",
        "[ \t]{0,2}[0-9]{0,5}[ \t]{0,2}",
        prop::sample::select(vec![
            "accrual",
            "cash",
            "partnership",
            "S corp",
            "1065",
            "schedule_c",
            "50%",
            "0.5",
            "",
            "01-01",
            "02-29",
        ])
        .prop_map(String::from),
    ];
    prop_oneof![
        s.clone().prop_map(|v| toml_str(&v)),
        prop::collection::vec(s, 0..4).prop_map(|v| toml_array(&v)),
        any::<i64>().prop_map(|n| n.to_string()),
        any::<f64>()
            .prop_filter("finite", |f| f.is_finite())
            .prop_map(|f| format!("{f:?}")),
        any::<bool>().prop_map(|b| b.to_string()),
        Just("{ a = 1 }".to_string()),
    ]
}

fn toml_shaped() -> impl Strategy<Value = String> {
    let entries = |keys: &'static [&'static str]| {
        prop::collection::vec((prop::sample::select(keys), toml_value()), 0..6)
    };
    (
        entries(KEYS_COMPANY),
        prop::collection::vec(entries(KEYS_MEMBER), 0..4),
        entries(KEYS_BOOKS),
    )
        .prop_map(|(company, members, books)| {
            let mut t = String::from("[company]\n");
            for (k, v) in company {
                t.push_str(&format!("{k} = {v}\n"));
            }
            for m in members {
                t.push_str("[[company.members]]\n");
                for (k, v) in m {
                    t.push_str(&format!("{k} = {v}\n"));
                }
            }
            t.push_str("[books]\n");
            for (k, v) in books {
                t.push_str(&format!("{k} = {v}\n"));
            }
            t
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// fiscal_year_containing doc + Pub. 538: for any first-of-month start
    /// and any date, the year contains the date, starts on that month's
    /// 1st, ends the day before the same month-day a year later, and is
    /// 365 or 366 days long.
    #[test]
    fn prop_fiscal_year_contains_date(
        month in 1u32..=12,
        days in 0i64..(NaiveDate::from_ymd_opt(2200, 12, 31).unwrap()
            - NaiveDate::from_ymd_opt(1900, 1, 1).unwrap()).num_days(),
    ) {
        let d = date(1900, 1, 1) + chrono::Duration::days(days);
        let c = ok(&company_kv("fiscal_year_start", &format!("{month:02}-01"))).company;
        let (start, end) = c.fiscal_year_containing(d);
        prop_assert!(start <= d && d <= end, "{d}: {start}..{end}");
        prop_assert_eq!((start.month(), start.day()), (month, 1));
        let y = if d.month() >= month { d.year() } else { d.year() - 1 };
        prop_assert_eq!(start, date(y, month, 1));
        let next = start.checked_add_months(Months::new(12)).unwrap();
        prop_assert_eq!(end, next.pred_opt().unwrap());
        let len = (end - start).num_days() + 1;
        prop_assert!(len == 365 || len == 366, "{len}");
    }

    /// Consecutive fiscal years tile: the day after one ends starts the
    /// next, and the day before one starts ends the previous.
    #[test]
    fn prop_fiscal_years_tile(
        month in 1u32..=12,
        days in 0i64..(NaiveDate::from_ymd_opt(2200, 12, 31).unwrap()
            - NaiveDate::from_ymd_opt(1900, 1, 1).unwrap()).num_days(),
    ) {
        let d = date(1900, 1, 1) + chrono::Duration::days(days);
        let c = ok(&company_kv("fiscal_year_start", &format!("{month:02}-01"))).company;
        let (start, end) = c.fiscal_year_containing(d);
        let after = end.succ_opt().unwrap();
        let (s2, e2) = c.fiscal_year_containing(after);
        prop_assert_eq!(s2, after);
        prop_assert_eq!(c.fiscal_year_containing(e2).0, after);
        let before = start.pred_opt().unwrap();
        prop_assert_eq!(c.fiscal_year_containing(before).1, before);
    }

    /// INVARIANT 2: any day but the 1st, of any month, is refused.
    #[test]
    fn prop_fiscal_start_not_first_refused(month in 1u32..=12, day in 2u32..=31) {
        refused(&company_kv("fiscal_year_start", &format!("{month:02}-{day:02}")));
    }

    /// INVARIANT 3: 1..=10 members whose decimal shares sum to 1 are
    /// accepted, with the shares as written.
    #[test]
    fn prop_shares_summing_to_one_accepted(bps in bps_shares()) {
        let shares: Vec<String> = bps.iter().map(|b| bps_str(*b)).collect();
        let s = ok(&members_from(&shares));
        prop_assert_eq!(s.company.members.len(), bps.len());
        for (m, b) in s.company.members.iter().zip(&bps) {
            prop_assert_eq!(m.share, Decimal::from(*b) / Decimal::from(10_000));
        }
        let total: Decimal = s.company.members.iter().map(|m| m.share).sum();
        prop_assert_eq!(total, Decimal::ONE);
    }

    /// INVARIANT 3: perturb one share by ±0.01 and the sum is no longer 1:
    /// refused.
    #[test]
    fn prop_perturbed_share_refused(bps in bps_shares(), pick in any::<prop::sample::Index>(), up in any::<bool>()) {
        let i = pick.index(bps.len());
        let mut shares: Vec<Decimal> =
            bps.iter().map(|b| Decimal::from(*b) / Decimal::from(10_000)).collect();
        let delta = dec("0.01");
        shares[i] = if up { shares[i] + delta } else { shares[i] - delta };
        let text: Vec<String> = shares.iter().map(|d| d.to_string()).collect();
        refused(&members_from(&text));
    }

    /// INVARIANT 3: shares written as "NN%" read the same as decimals.
    #[test]
    fn prop_percent_equals_decimal(pcts in pct_shares()) {
        let as_pct: Vec<String> = pcts.iter().map(|p| format!("{p}%")).collect();
        let as_dec: Vec<String> =
            pcts.iter().map(|p| (Decimal::from(*p) / Decimal::from(100)).to_string()).collect();
        let a = ok(&members_from(&as_pct));
        let b = ok(&members_from(&as_dec));
        prop_assert_eq!(a.company.members, b.company.members);
    }

    /// INVARIANT 6: whitespace padding around codes is trimmed.
    #[test]
    fn prop_codes_trimmed(
        codes in prop::collection::vec(("[ \t]{0,3}", "[0-9A-Za-z]{1,6}", "[ \t]{0,3}"), 1..5),
        into in ("[ \t]{0,3}", "[0-9]{1,6}", "[ \t]{0,3}"),
    ) {
        let padded: Vec<String> = codes.iter().map(|(a, c, b)| format!("{a}{c}{b}")).collect();
        let bare: Vec<String> = codes.iter().map(|(_, c, _)| c.clone()).collect();
        let arr = toml_array(&padded);
        let text = format!(
            "[books]\ncash_accounts = {arr}\nfinancing_accounts = {arr}\n\
             investing_accounts = {arr}\nclose_sweep = {arr}\nclose_into = {}\n",
            toml_str(&format!("{}{}{}", into.0, into.1, into.2))
        );
        let s = ok(&text);
        prop_assert_eq!(&s.cash_accounts, &bare);
        prop_assert_eq!(&s.financing_accounts, &bare);
        prop_assert_eq!(&s.investing_accounts, &bare);
        prop_assert_eq!(&s.company.close_sweep, &bare);
        prop_assert_eq!(s.company.close_into.as_deref(), Some(into.1.as_str()));
    }

    /// INVARIANT 6: a blank code anywhere in a list is refused.
    #[test]
    fn prop_blank_code_refused(
        key in prop::sample::select(vec!["cash_accounts", "financing_accounts", "investing_accounts", "close_sweep"]),
        good in prop::collection::vec("[0-9]{1,6}", 0..4),
        blank in "[ \t]{0,4}",
        at in any::<prop::sample::Index>(),
    ) {
        let mut items = good.clone();
        items.insert(at.index(good.len() + 1), blank);
        refused(&format!("[books]\n{key} = {}\n", toml_array(&items)));
    }

    /// INVARIANT 4: a table phrase among words that name no return maps
    /// per the source table.
    #[test]
    fn prop_classification_in_context(
        row in prop::sample::select(CLASSIFICATIONS),
        before in filler(),
        after in filler(),
    ) {
        let text = before.iter().chain([&row.0]).chain(after.iter()).copied().collect::<Vec<_>>().join(" ");
        prop_assert_eq!(classification(&text), Ok(Some(row.1)), "{}", text);
    }

    /// INVARIANT 4: a phrase naming two different returns is refused.
    #[test]
    fn prop_classification_two_returns_refused(
        a in prop::sample::select(CLASSIFICATIONS),
        b in prop::sample::select(CLASSIFICATIONS),
        mid in filler(),
    ) {
        prop_assume!(a.1 != b.1);
        let text = [a.0].iter().chain(mid.iter()).chain([&b.0]).copied().collect::<Vec<_>>().join(" ");
        prop_assert!(classification(&text).is_err(), "{}", text);
    }

    /// INVARIANT 4: tax_form wins over any classification beside it.
    #[test]
    fn prop_tax_form_wins(
        form in prop::sample::select(TaxForm::ALL.to_vec()),
        row in prop::sample::select(CLASSIFICATIONS),
    ) {
        let text = format!(
            "[company]\ntax_form = {}\ntax_classification = {}\n",
            toml_str(form.label()),
            toml_str(row.0)
        );
        prop_assert_eq!(ok(&text).company.tax_form, Some(form));
    }

    /// INVARIANT 5: any non-negative decimal is accepted, any negative one
    /// refused.
    #[test]
    fn prop_threshold_sign(units in 0i64..1_000_000_000, scale in 0u32..=4) {
        let d = Decimal::new(units, scale);
        let s = ok(&books_kv("receipt_threshold", &d.to_string()));
        prop_assert_eq!(dec(s.receipt_threshold.as_deref().unwrap()), d);
        if units > 0 {
            refused(&books_kv("receipt_threshold", &(-d).to_string()));
        }
    }

    /// Every refusal is an Err, never a panic: any text at all.
    #[test]
    fn prop_any_string_never_panics(text in any::<String>()) {
        let _ = parse(&text);
    }

    /// Every refusal is an Err, never a panic: TOML-shaped text with the
    /// real keys and arbitrary values.
    #[test]
    fn prop_toml_shaped_never_panics(text in toml_shaped()) {
        let _ = parse(&text);
    }
}

/// INVARIANT 7: the profile keeps numbers, booleans, and dates in TOML's
/// spelling, in file order, and leaves out tables and arrays.
#[test]
fn inv7_profile_keeps_simple_values_of_every_type() {
    let s = ledger_config::parse(
        "[company]\nname = \"Acme\"\nemployees = 3\nrate = 2.5\nactive = true\nformed = 2026-10-04\ncodes = [\"a\"]\n[company.address]\ncity = \"Ogden\"\n",
    )
    .unwrap();
    assert_eq!(
        s.company.profile,
        [
            ("name".to_string(), "Acme".to_string()),
            ("employees".to_string(), "3".to_string()),
            ("rate".to_string(), "2.5".to_string()),
            ("active".to_string(), "true".to_string()),
            ("formed".to_string(), "2026-10-04".to_string()),
        ]
    );
}

// ------------------------------------------------- [books] asset_tag_prefix
//
// Spec: `BooksSettings::asset_tag_prefix` doc ("`[books] asset_tag_prefix`,
// or else the initials of the company name's words, legal-form words (LLC,
// Inc, Corp, ...) left out, at most 8 ("Wanderland Labs LLC" → "WL"); "FA"
// (fixed asset) when the name gives none"), docs/mcp-interface.md
// (books.register_asset), and the parse refusal "1 to 8 letters or digits".

fn prefix_for_name(name: &str) -> String {
    ok(&company_kv("name", name)).asset_tag_prefix()
}

#[test]
fn asset_tag_prefix_explicit_setting_wins() {
    let s = ok("[company]\nname = \"Wanderland Labs LLC\"\n[books]\nasset_tag_prefix = \"LAB\"\n");
    assert_eq!(s.asset_tag_prefix.as_deref(), Some("LAB"));
    assert_eq!(s.asset_tag_prefix(), "LAB");
}

#[test]
fn asset_tag_prefix_accepts_one_to_eight_letters_or_digits() {
    for p in ["A", "7", "WL", "WL2026", "ABCDEFGH", "12345678"] {
        let s = ok(&books_kv("asset_tag_prefix", p));
        assert_eq!(s.asset_tag_prefix(), p, "{p:?}");
    }
}

#[test]
fn asset_tag_prefix_refuses_other_forms() {
    for p in [
        "",
        "ABCDEFGHI",
        "WL-",
        "W L",
        "W_L",
        "WL.",
        "ÉL",
        " WL",
        "WL/1",
    ] {
        refused(&books_kv("asset_tag_prefix", p));
    }
    // Not a string.
    refused("[books]\nasset_tag_prefix = 12\n");
}

#[test]
fn asset_tag_prefix_unset_is_none() {
    assert_eq!(ok("").asset_tag_prefix, None);
    assert_eq!(ok("[books]\n").asset_tag_prefix, None);
}

/// The doc's example.
#[test]
fn asset_tag_prefix_default_is_company_initials_without_legal_form() {
    assert_eq!(prefix_for_name("Wanderland Labs LLC"), "WL");
    assert_eq!(prefix_for_name("Acme Inc"), "A");
    assert_eq!(prefix_for_name("Acme Widgets Corp"), "AW");
    assert_eq!(prefix_for_name("Blue Sky Ventures"), "BSV");
}

/// Initials are uppercase.
#[test]
fn asset_tag_prefix_default_is_uppercase() {
    assert_eq!(prefix_for_name("wanderland labs LLC"), "WL");
}

/// At most 8 initials.
#[test]
fn asset_tag_prefix_default_takes_at_most_eight_initials() {
    assert_eq!(
        prefix_for_name("Alpha Bravo Charlie Delta Echo Foxtrot Golf Hotel India Juliet"),
        "ABCDEFGH"
    );
}

/// "FA" when the name gives none: only legal-form words, or the default
/// (unnamed) company.
#[test]
fn asset_tag_prefix_default_falls_back_to_fa() {
    assert_eq!(ok("").asset_tag_prefix(), "FA");
    assert_eq!(prefix_for_name("Unnamed company"), "FA");
    assert_eq!(prefix_for_name("LLC"), "FA");
    assert_eq!(prefix_for_name("Inc"), "FA");
}

proptest! {
    /// Whatever the name, the default is a prefix the setting itself would
    /// accept: 1 to 8 ASCII letters or digits, uppercase.
    #[test]
    fn prop_default_asset_tag_prefix_is_a_valid_setting(name in "[ -~]{1,60}") {
        let Ok(s) = parse(&company_kv("name", &name)) else { return Ok(()); };
        let p = s.asset_tag_prefix();
        prop_assert!((1..=8).contains(&p.len()), "{:?} -> {:?}", name, p);
        prop_assert!(p.chars().all(|c| c.is_ascii_alphanumeric()), "{:?} -> {:?}", name, p);
        prop_assert_eq!(p.to_ascii_uppercase(), p.clone());
        let back = ok(&books_kv("asset_tag_prefix", &p));
        prop_assert_eq!(back.asset_tag_prefix(), p);
    }
}
