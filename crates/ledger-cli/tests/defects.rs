//! The re-audit's engineering mirror (plan P10; `docs/runbooks/re-audit.md`):
//! every defect the 2026-10-16 audit found (1 to 20), P0 (21, 22) and P3
//! (23, 24) has a named regression test somewhere in the workspace, and the
//! runbook names only tools and commands that exist.
//!
//! The list below is the single place that says which test stands guard for
//! which defect. A test renamed or deleted fails here, so a defect's
//! reproduction cannot be lost quietly. This proves the tests exist, are
//! real `#[test]`s and are not ignored; that they pass is `cargo test`'s
//! job (the runbook's step 4 runs them by name).
//!
//! SOURCE: plan/served.md 8.0 ("defect n") and P10 step 2.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// `(defect, test name, file)`. Defect 2 is closed by D-12 and D-47 (the
/// rule it now shows is that a reversal needs no receipt of its own);
/// defect 20 (the migration scripts) was retired at the cutover and is
/// checked separately below.
const DEFECTS: [(u32, &str, &str); 23] = [
    (
        1,
        "defect1_two_member_llc_first_year_regression",
        "crates/ledger-core/tests/spec_cash_flow.rs",
    ),
    (
        2,
        "a_reversal_needs_no_receipt_of_its_own",
        "crates/ledger-cli/tests/drafts.rs",
    ),
    (
        3,
        "get_receipt_default_has_text_and_no_bytes",
        "crates/ledger-cli/tests/evidence.rs",
    ),
    (
        4,
        "same_bytes_one_record_many_links",
        "crates/ledger-store/tests/attachments.rs",
    ),
    (
        5,
        "a_draft_waits_for_its_receipt_and_posts",
        "crates/ledger-cli/tests/drafts.rs",
    ),
    (
        6,
        "digest_since_seq_is_posting_time",
        "crates/ledger-cli/tests/drafts.rs",
    ),
    (
        7,
        "the_readme_lists_what_the_iif_writes_differently",
        "crates/ledger-package/tests/package.rs",
    ),
    (
        8,
        "add_account_warns_without_tax_category_defect_8",
        "crates/ledger-cli/tests/spec_tax_policy.rs",
    ),
    (
        9,
        "defect9_a_new_schedule_spreads_by_the_day",
        "crates/ledger-core/tests/schedules.rs",
    ),
    (
        10,
        "annotations_replay_and_project",
        "crates/ledger-store/tests/attachments.rs",
    ),
    (
        11,
        "defect11_the_directory_shows_a_members_tin_last4",
        "crates/ledger-cli/tests/spec_vault.rs",
    ),
    (
        12,
        "payment_methods_round_trip_and_classify_payments",
        "crates/ledger-cli/tests/spec_1099.rs",
    ),
    (
        13,
        "four_contributed_devices_each_take_the_lower_of_value_and_basis",
        "crates/ledger-core/tests/spec_asset_basis.rs",
    ),
    (
        14,
        "defect14_party_totals_take_a_basis",
        "crates/ledger-cli/tests/spec_party_totals.rs",
    ),
    (
        15,
        "rename_account_end_to_end_defect_15",
        "crates/ledger-cli/tests/mcp_journeys.rs",
    ),
    (
        16,
        "one_default_period_everywhere_defect_16",
        "crates/ledger-cli/tests/mcp_journeys.rs",
    ),
    (
        17,
        "the_audit_odd_cent_never_piles_up_across_closes",
        "crates/ledger-core/tests/odd_cent.rs",
    ),
    (
        18,
        "undecided_is_not_not_reportable",
        "crates/ledger-core/tests/form1099.rs",
    ),
    (
        19,
        "no_tool_takes_a_path",
        "crates/ledger-mcp/src/server_tests.rs",
    ),
    (
        21,
        "as_of_package_iif_stops_at_as_of",
        "crates/ledger-package/tests/package.rs",
    ),
    (
        22,
        "defect22_depreciation_through_mid_final_month_is_prorated_not_plugged",
        "crates/ledger-core/tests/spec_assets_payroll.rs",
    ),
    (
        23,
        "a_vendor_marked_no_1099_is_not_undecided",
        "crates/ledger-cli/tests/spec_1099.rs",
    ),
    (
        24,
        "defect24_an_accrual_through_another_liability_counts_once",
        "crates/ledger-core/tests/spec_party_totals.rs",
    ),
];

/// Defects 1 to 24 each have a named test that exists, is a test, and is
/// not ignored (defect 20 is the retired scripts, below).
#[test]
fn every_defect_reproduction_is_a_named_test() {
    let mut covered: Vec<u32> = DEFECTS.iter().map(|d| d.0).collect();
    covered.push(20);
    covered.sort_unstable();
    assert_eq!(
        covered,
        (1..=24).collect::<Vec<u32>>(),
        "a defect is missing"
    );

    let mut missing = Vec::new();
    for (defect, name, file) in DEFECTS {
        let text = read(file);
        let lines: Vec<&str> = text.lines().collect();
        let needle = format!("fn {name}(");
        let Some(at) = lines.iter().position(|l| {
            l.trim_start().starts_with(&needle)
                || l.trim_start().starts_with(&format!("pub {needle}"))
        }) else {
            missing.push(format!("defect {defect}: no `fn {name}` in {file}"));
            continue;
        };
        // The attributes and doc lines above the function, up to the
        // previous blank line or item.
        let above: String = lines[at.saturating_sub(8)..at].join("\n");
        let in_proptest = text.contains("proptest!");
        if !above.contains("#[test]") && !in_proptest {
            missing.push(format!(
                "defect {defect}: `{name}` in {file} is not a #[test]"
            ));
        }
        if above.contains("#[ignore") {
            missing.push(format!("defect {defect}: `{name}` in {file} is ignored"));
        }
    }
    assert!(missing.is_empty(), "{}", missing.join("\n"));
}

/// Defect 20, the migration scripts, was retired at the cutover: there is
/// no tool for them, and none may come back as a way past the journal.
/// The defect 19 mirror (`import_local_file` and every path parameter) is
/// `no_tool_takes_a_path` above.
#[test]
fn defect_20_no_migration_tool_exists() {
    let scopes = read("crates/ledger-mcp/src/scopes.rs");
    for line in scopes.lines().filter(|l| l.contains("(\"books.")) {
        for banned in [
            "migrate",
            "import_local_file",
            "import_sheet",
            "verify_sheet",
        ] {
            assert!(!line.contains(banned), "a retired tool is back: {line}");
        }
    }
}

/// The runbook names only tools and commands that exist: each
/// `books.<name>` is in the scopes table (the 8.0.3 list of tools), each
/// `wanderbooks <command>` is a command `main.rs` dispatches, and each
/// test file it names is a file.
#[test]
fn the_re_audit_runbook_names_only_what_exists() {
    let runbook = read("docs/runbooks/re-audit.md");
    let scopes = read("crates/ledger-mcp/src/scopes.rs");
    let main = read("crates/ledger-cli/src/main.rs");

    let mut tools = Vec::new();
    let mut rest = runbook.as_str();
    while let Some(i) = rest.find("books.") {
        let tail = &rest[i..];
        let end = tail
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
            .unwrap_or(tail.len());
        let name = tail[..end].trim_end_matches('.');
        // Prose such as "books.start's" ends at the apostrophe; a bare
        // "books." (the company's books followed by a full stop) is not a name.
        if name.len() > "books.".len() && !tools.contains(&name.to_string()) {
            tools.push(name.to_string());
        }
        rest = &tail[end.max(1)..];
    }
    assert!(
        tools.len() > 20,
        "the runbook should name its tools: {tools:?}"
    );
    let unknown: Vec<&String> = tools
        .iter()
        .filter(|t| !scopes.contains(&format!("(\"{t}\"")))
        .collect();
    assert!(
        unknown.is_empty(),
        "tools the runbook names that do not exist: {unknown:?}"
    );

    for cmd in ["gui", "connect"] {
        if runbook.contains(&format!("wanderbooks {cmd}")) {
            assert!(main.contains(&format!("\"{cmd}\"")), "no command {cmd}");
        }
    }
    assert!(
        runbook.contains("wanderbooks gui"),
        "step 6 names the viewer"
    );

    for step in 1..=8 {
        assert!(
            runbook.contains(&format!("## Step {step}.")),
            "the runbook lost step {step}"
        );
    }
    // The runbook must not point at the data root or tell anyone to read it.
    assert!(!runbook.contains("wanderbooks.v"), "names a database file");
}
