//! The server's tests: the whole tool surface (`annotations_complete`,
//! the text and schema checks, the document/description budgets), the
//! access door, the request machinery and the resources. They sit beside
//! `server.rs` (a `#[path]` module of it) so `server.rs` stays under its
//! 3,000-line bar (plan/served.md 8.2 U0a); a tool group's own tests live
//! in its group file, and the fixtures both use are in
//! `server::test_support`.

use super::*;
use crate::server::test_support::*;

/// An upload's limit, 25 MiB exactly (plan 4.7); nothing comes inline
/// (D-46).
#[test]
fn the_file_limits_are_the_plans() {
    assert_eq!(ledger_store::uploads::MAX_UPLOAD, 26_214_400);
    // Plan 4.7: an upload lasts 15 minutes; 16 under way per token.
    assert_eq!(ledger_store::uploads::UPLOAD_TTL, 900);
    assert_eq!(ledger_store::uploads::MAX_LIVE, 16);
    assert_eq!(ledger_store::uploads::MAX_NAME_BYTES, 255);
    assert_eq!(crate::local::MAX_BODY, 1024 * 1024);
}

/// Every tool says what it is and what it does to the books (plan
/// 4.9, "Annotations (P0)"): a title and readOnlyHint on all; a
/// mutating tool also says whether it is destructive. None asks the
/// client for an interaction of its own (owner decision 2026-10-16:
/// no added prompts).
#[test]
fn annotations_complete() {
    // Every tool's hints, pinned (MCP's ToolAnnotations): R read-only;
    // W changes the books, additively; D destructive (posts, or
    // overwrites, unlinks, voids, purges what was there: a vault
    // record, a link, a reconciliation); I idempotent (a repeat with
    // the same arguments leaves the journal and the vault as they
    // were: spec_vault's idempotent_tools_change_nothing_when_repeated
    // calls each twice). export_cpa_package writes a file, so it is
    // not read-only.
    type Hints = (Option<bool>, Option<bool>, Option<bool>);
    const R: Hints = (Some(true), None, None);
    const W: Hints = (Some(false), Some(false), None);
    const D: Hints = (Some(false), Some(true), None);
    const WI: Hints = (Some(false), Some(false), Some(true));
    const DI: Hints = (Some(false), Some(true), Some(true));
    let table: [(&str, Hints); 124] = [
        ("books.handbook", R),
        ("books.start", R),
        ("books.set_setting", DI),
        ("books.export_settings", R),
        ("books.conventions", R),
        ("books.set_convention", W),
        ("books.retire_convention", D),
        ("books.record_decision", W),
        ("books.decisions", R),
        ("books.raise_item", W),
        ("books.resolve_item", W),
        ("books.open_items", R),
        ("books.draft_from_template", W),
        ("books.approve_entry", W),
        ("books.chart", R),
        ("books.add_account", W),
        ("books.trial_balance", R),
        ("books.profit_and_loss", R),
        ("books.balance_sheet", R),
        ("books.cash_flow", R),
        ("books.draft_entry", W),
        ("books.add_line", W),
        ("books.post_entry", D),
        ("books.reverse_entry", D),
        ("books.attach_receipt", W),
        ("books.file_document", W),
        ("books.link_receipt", W),
        ("books.unlink_receipt", D),
        ("books.withdraw_attachment", D),
        ("books.link_attachment", W),
        ("books.unlink_attachment", D),
        ("books.attachments", R),
        ("books.list_receipts", R),
        ("books.get_receipt", R),
        ("books.get_entry", R),
        ("books.search_entries", R),
        ("books.show", R),
        ("books.verify", WI),
        ("books.digest", R),
        ("books.import_bank_csv", W),
        ("books.create_schedule", W),
        ("books.list_schedules", R),
        ("books.run_schedules", DI),
        ("books.create_recurring", W),
        ("books.list_recurring", R),
        ("books.run_recurring", DI),
        ("books.record_payout", W),
        ("books.list_vendors", R),
        ("books.set_vendor_1099", WI),
        ("books.set_entry_vendor", W),
        ("books.vendor_1099", W),
        ("books.add_instrument", W),
        ("books.instruments", R),
        ("books.set_entry_instrument", WI),
        ("books.chart_template", R),
        ("books.tax_categories", R),
        ("books.renumber_account", D),
        ("books.rename_account", D),
        ("books.label_asset", W),
        ("books.correct_asset", D),
        ("books.set_asset_tax_basis", W),
        ("books.asset_labels", R),
        ("books.settings", R),
        ("books.roles", R),
        ("books.assign_role", D),
        ("books.set_tax_category", W),
        ("books.import_chart_csv", W),
        ("books.begin_upload", W),
        ("books.export", R),
        ("books.register_asset", W),
        ("books.asset_schedule", R),
        ("books.list_assets", R),
        ("books.run_depreciation", DI),
        ("books.dispose_asset", D),
        ("books.create_period", W),
        ("books.close_period", D),
        ("books.list_periods", R),
        ("books.mark_cleared", W),
        ("books.reconcile", D),
        ("books.record_statement", W),
        ("books.statements", R),
        ("books.reconciliations", R),
        ("books.void_statement", D),
        ("books.void_reconciliation", D),
        ("books.add_customer", W),
        ("books.add_vendor", W),
        ("books.add_party", W),
        ("books.update_party", W),
        ("books.set_party_roles", WI),
        ("books.set_w9", D),
        ("books.set_tin", D),
        ("books.get_w9", W),
        ("books.purge_tin", D),
        ("books.set_w4", W),
        ("books.parties", R),
        ("books.party", R),
        ("books.set_entry_party", W),
        ("books.create_invoice", D),
        ("books.create_bill", D),
        ("books.receive_payment", D),
        ("books.pay_bill", D),
        ("books.void_document", D),
        ("books.mark_sent", W),
        ("books.control_check", R),
        ("books.documents", R),
        ("books.document", R),
        ("books.ar_aging", R),
        ("books.ap_aging", R),
        ("books.bills_due", R),
        ("books.set_fx_rate", W),
        ("books.run_payroll", D),
        ("books.remit_payroll_taxes", D),
        ("books.list_payroll_runs", R),
        ("books.add_class", W),
        ("books.list_classes", R),
        ("books.add_project", W),
        ("books.update_project", WI),
        ("books.projects", R),
        ("books.project", R),
        ("books.profit_and_loss_by_class", R),
        ("books.export_cpa_package", W),
        ("books.access_log", R),
        ("books.backup", R),
        ("books.back_up_now", WI),
    ];
    let tools = BooksServer::tool_router().list_all();
    assert_eq!(tools.len(), table.len());
    let mut names: Vec<&str> = table.iter().map(|(n, _)| *n).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), table.len(), "a tool is in the table twice");
    for t in &tools {
        let name = &t.name;
        assert!(
            t.title.as_deref().is_some_and(|s| !s.trim().is_empty()),
            "{name} has no title"
        );
        let a = t
            .annotations
            .as_ref()
            .unwrap_or_else(|| panic!("{name} has no annotations"));
        let want = table
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("{name} is not in the table"))
            .1;
        assert_eq!(
            (a.read_only_hint, a.destructive_hint, a.idempotent_hint),
            want,
            "{name}: (readOnlyHint, destructiveHint, idempotentHint)"
        );
        let json = serde_json::to_string(t).unwrap().to_lowercase();
        for word in ["requiresuserinteraction", "requires_user_interaction"] {
            assert!(!json.contains(word), "{name} asks for user interaction");
        }
        // The books are a closed world: no tool reaches beyond them,
        // but the one that pushes a backup to the owner's targets.
        assert_eq!(
            a.open_world_hint,
            Some(*name == "books.back_up_now"),
            "{name}: openWorldHint"
        );
    }
    // docs/mcp-interface.md names the 29 destructive tools.
    assert_eq!(table.iter().filter(|(_, h)| h.1 == Some(true)).count(), 29);
}

/// Every refusal an agent can be given whose text is fixed here, or
/// in the crates under the server: the error templates of every
/// variant (scanned from their sources, so a new variant is linted
/// too), representative values of those whose text is built, and the
/// server's own refusal constants.
fn agent_refusals() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (krate, src) in [
        (
            "ledger-core",
            include_str!("../../ledger-core/src/error.rs"),
        ),
        (
            "ledger-store",
            include_str!("../../ledger-store/src/error.rs"),
        ),
        (
            "ledger-vault",
            include_str!("../../ledger-vault/src/lib.rs"),
        ),
        (
            "ledger-import bank",
            include_str!("../../ledger-import/src/bank_csv.rs"),
        ),
        (
            "ledger-import chart",
            include_str!("../../ledger-import/src/chart_csv.rs"),
        ),
        (
            "ledger-import iif",
            include_str!("../../ledger-import/src/iif.rs"),
        ),
        (
            "ledger-import pdf",
            include_str!("../../ledger-import/src/pdf.rs"),
        ),
        (
            "ledger-import xlsx",
            include_str!("../../ledger-import/src/xlsx.rs"),
        ),
        (
            "ledger-store uploads",
            include_str!("../../ledger-store/src/uploads.rs"),
        ),
    ] {
        let mut rest = src;
        let mut n = 0;
        while let Some(at) = rest.find("#[error(") {
            rest = &rest[at + 8..];
            let end = rest.find(")]").expect("an #[error] attribute ends");
            out.push((format!("{krate} error template"), rest[..end].to_string()));
            rest = &rest[end..];
            n += 1;
        }
        assert!(n > 0, "{krate}: no error templates found");
    }
    // The files and backup tools' refusals (and descriptions), and the
    // backups' own words: their sentences. (What git itself says of a
    // failed backup reaches the human as git said it: not ours.)
    for (name, src) in [
        ("tools/files.rs", include_str!("tools/files.rs")),
        ("tools/backup.rs", include_str!("tools/backup.rs")),
        ("backup.rs", include_str!("backup.rs")),
    ] {
        let src = src.split("#[cfg(test)]").next().unwrap_or(src);
        for line in src.lines() {
            let line = line.trim_start();
            if line.starts_with("//") {
                continue;
            }
            for (i, text) in line.split('"').enumerate() {
                if i % 2 == 1 && text.len() > 30 && text.contains(' ') {
                    out.push((format!("{name} text"), text.to_string()));
                }
            }
        }
    }
    let newer = |verified| ledger_store::StoreError::NewerJournal {
        seq: 7,
        event_type: "settings.set".into(),
        need: "journal format 2".into(),
        verified,
    };
    out.push(("NewerJournal".into(), newer(true).to_string()));
    out.push(("NewerJournal unverified".into(), newer(false).to_string()));
    out.push((
        "FormatUpgradePending".into(),
        ledger_store::StoreError::FormatUpgradePending {
            need: "journal format 2".into(),
        }
        .to_string(),
    ));
    out.push((
        "RuleNotKnown".into(),
        ledger_core::form1099::default_threshold(9999)
            .unwrap_err()
            .to_string(),
    ));
    for (what, text) in [
        ("STOPPED", STOPPED),
        ("tin_files::TIN_FILE", tin_files::TIN_FILE),
        ("tin_files::TIN_HELD", tin_files::TIN_HELD),
        ("tin_files::TIN_PROBE", tin_files::TIN_PROBE),
        ("tin_files::TIN_PRINTED", tin_files::TIN_PRINTED),
    ] {
        out.push((what.to_string(), text.to_string()));
    }
    // Refusals and notes built inline, where the books' own record is
    // kept: every string literal of these sources outside their tests.
    for (what, src) in [
        ("tools/kept.rs", include_str!("tools/kept.rs")),
        (
            "ledger-config codec",
            include_str!("../../ledger-config/src/codec.rs"),
        ),
        (
            "ledger-core settings",
            include_str!("../../ledger-core/src/settings.rs"),
        ),
        (
            "ledger-core conventions",
            include_str!("../../ledger-core/src/conventions.rs"),
        ),
        (
            "ledger-core ledger",
            include_str!("../../ledger-core/src/ledger.rs"),
        ),
        ("settings.rs", include_str!("settings.rs")),
    ] {
        let code = src.split("#[cfg(test)]").next().unwrap_or(src);
        let literals = string_literals(code);
        assert!(!literals.is_empty(), "{what}: no literals found");
        out.extend(literals.into_iter().map(|l| (format!("{what} literal"), l)));
    }
    out
}

/// The text of every plain string literal ("...") in Rust source,
/// skipping comments (a rough scanner: enough for the lint).
fn string_literals(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in src.lines() {
        let line = line.trim_start();
        if line.starts_with("//") {
            continue;
        }
        let mut chars = line.chars().peekable();
        let mut cur: Option<String> = None;
        while let Some(c) = chars.next() {
            match (&mut cur, c) {
                (None, '"') => cur = Some(String::new()),
                (None, '/') if chars.peek() == Some(&'/') => break,
                (Some(s), '\\') => {
                    if let Some(n) = chars.next() {
                        s.push('\\');
                        s.push(n);
                    }
                }
                (Some(s), '"') => {
                    out.push(std::mem::take(s));
                    cur = None;
                }
                (Some(s), c) => s.push(c),
                (None, _) => {}
            }
        }
    }
    out
}

/// No text an agent is given sends it to the books' files or to tools
/// outside these (plan I-12): no helper commands, home paths, data
/// dir names or flags, file names of the books, git, GitHub, pull
/// requests, repo docs, the old company's examples, or suggestions to
/// read files. Linted whole: every tool's JSON (title, description,
/// input and output schemas), every resource, the connect text, every
/// handbook section, and every refusal fixed in code
/// ([`agent_refusals`]). (No resource templates are advertised yet.)
/// What `tool_text_lint` finds in one text beyond its fixed list: a
/// helper command however it ends (`just` as a word before a
/// backtick, a line break, a tab or the end), and where the books are
/// ("beside the books", "books dir").
fn helper_or_location(what: &str, text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let mut found = Vec::new();
    for (at, _) in lower.match_indices("just") {
        let prev = lower[..at].chars().next_back();
        let next = lower[at + 4..].chars().next();
        let word_start = !prev.is_some_and(|c| c.is_alphanumeric() || c == '_');
        if word_start && matches!(next, None | Some('`' | '\n' | '\r' | '\t')) {
            found.push(format!("{what} names a helper command (`just`)"));
        }
    }
    if lower.contains("beside the books") {
        found.push(format!(
            "{what} says where the books are: \"beside the books\""
        ));
    }
    if lower.contains("books dir") {
        found.push(format!("{what} says where the books are: \"books dir\""));
    }
    found
}

/// The lint's own checks catch what they name, and pass ordinary words.
#[test]
fn tool_text_lint_catches_helpers_and_locations() {
    for (text, n) in [
        ("run `just`", 1),
        ("run just\nthen", 1),
        ("run just\tgui", 1),
        ("run just", 1),
        ("just`gui`", 1),
        ("Just\n", 1),
        ("adjust`", 0),
        ("just after posting", 0),
        ("just, then", 0),
        ("justly", 0),
        ("the file beside the books", 1),
        ("in the books dir", 1),
        ("a path outside the books dir", 1),
        ("the books directory", 1),
        ("bookshelf", 0),
    ] {
        assert_eq!(helper_or_location("text", text).len(), n, "{text:?}");
    }
    assert_eq!(
        helper_or_location("tool books.get_w9", "outside the books dir").len(),
        1
    );
    assert_eq!(
        helper_or_location("tool books.chart", "outside the books dir").len(),
        1
    );
}

/// No tool takes a path on the server's computer (plan 4.7, D-28,
/// D-29, D-46): files come in by upload, and go out as exports.
#[test]
fn no_tool_takes_a_path() {
    fn names(v: &serde_json::Value, out: &mut Vec<String>) {
        match v {
            serde_json::Value::Object(map) => {
                if let Some(serde_json::Value::Object(props)) = map.get("properties") {
                    out.extend(props.keys().cloned());
                }
                map.values().for_each(|v| names(v, out));
            }
            serde_json::Value::Array(items) => items.iter().for_each(|v| names(v, out)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    for t in BooksServer::tool_router().list_all() {
        let mut props = Vec::new();
        names(
            &serde_json::Value::Object((*t.input_schema).clone()),
            &mut props,
        );
        for p in props {
            let lower = p.to_ascii_lowercase();
            // Names that hold a place to read or write: whole words,
            // so `link_to`, `*_file_name`, `tin_on_file` and `date_to`
            // pass.
            let words: Vec<&str> = lower.split('_').collect();
            let pathlike = lower.starts_with("save")
                || lower.contains("path")
                || [
                    "dir",
                    "folder",
                    "root",
                    "dest",
                    "destination",
                    "output",
                    "location",
                    "local",
                ]
                .iter()
                .any(|w| words.contains(w))
                || (words.last() == Some(&"file") && !lower.ends_with("on_file"))
                || lower == "to"
                || lower == "from"
                || lower == "source";
            if pathlike {
                found.push(format!("{} {p}", t.name));
            }
        }
    }
    found.sort();
    found.dedup();
    assert!(found.is_empty(), "path-like parameters: {found:?}");
}

#[test]
fn tool_text_lint() {
    const NEVER: &[&str] = &[
        "just ",
        "~/",
        "/home",
        ".config/wanderbooks",
        "books-data/",
        "data dir",
        "--data-dir",
        "--import-root",
        "vault.key",
        "journal.jsonl",
        ".jsonl",
        "sqlite",
        ".gitignore",
        "github",
        "pull request",
        "docs/",
        ".md",
        "read the journal",
        "the journal is the evidence",
        "read the file",
        "open the file",
        "cat ",
        // Gone in P2 slice 5: files come by upload.
        "import_local_file",
        "w9_file_path",
        "import folder",
        "import root",
        "put_url",
        "human_url",
        "fl-0001",
        // Gone with D-46: a file never passes through the agent.
        "content_base64",
        "csv_text",
        "base64",
        "256 kib",
    ];
    // Whole words only: "digit" and "committee" are fine.
    const NEVER_WORDS: &[&str] = &["git", "commit"];
    let mut texts: Vec<(String, String)> = Vec::new();
    for t in BooksServer::tool_router().list_all() {
        texts.push((
            format!("tool {}", t.name),
            serde_json::to_string(&t).unwrap(),
        ));
    }
    for r in resource_list() {
        texts.push((
            format!("resource {}", r.uri),
            serde_json::to_string(&r).unwrap(),
        ));
    }
    texts.extend(agent_refusals());
    for threshold in ["0.00", "25.00"] {
        let settings = Settings {
            policy: ReceiptPolicy {
                require_receipts_above: threshold.into(),
                ..ReceiptPolicy::default()
            },
            ..Settings::default()
        };
        texts.push((
            "connect text".into(),
            handbook::connect_text(&settings, "USD"),
        ));
        for (id, _) in handbook::SECTIONS {
            for declared in [true, false] {
                texts.push((
                    format!("handbook {id}"),
                    handbook::section(id, &settings, "USD", declared).unwrap(),
                ));
            }
        }
    }
    // Every tool the text names is one there is.
    let tools: std::collections::HashSet<String> = BooksServer::tool_router()
        .list_all()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    let mut unknown: Vec<String> = Vec::new();
    // Settings keys (`books.cash_accounts`) are named like tools: a
    // name that is one is a setting, not a tool.
    let keys: std::collections::HashSet<&str> =
        ledger_core::settings::KEYS.iter().map(|k| k.name).collect();
    for (what, text) in texts
        .iter()
        .filter(|(w, _)| !w.contains("settings literal"))
    {
        for (i, _) in text.match_indices("books.") {
            let name: String = text[i..]
                .chars()
                .take_while(|c| {
                    c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_' || *c == '.'
                })
                .collect();
            let name = name.trim_end_matches('.');
            // "the books." ends a sentence.
            if name == "books" {
                continue;
            }
            let before = text[..i].chars().next_back();
            if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '/') {
                continue;
            }
            if !tools.contains(name) && !keys.contains(name) {
                unknown.push(format!("{what}: {name}"));
            }
        }
    }
    unknown.sort();
    unknown.dedup();
    assert!(
        unknown.is_empty(),
        "text names tools there aren't: {unknown:?}"
    );
    let words = |text: &str| {
        text.split(|c: char| !c.is_alphanumeric() && c != '_')
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    let mut found: Vec<String> = Vec::new();
    for (what, text) in &texts {
        // A handbook resource's own address is not a file name.
        let lower = text.to_lowercase().replace("books://handbook", "");
        for bad in NEVER {
            if let Some(at) = lower.find(bad) {
                let from = lower[..at]
                    .char_indices()
                    .rev()
                    .nth(40)
                    .map_or(0, |(i, _)| i);
                let near: String = lower[from..].chars().take(100).collect();
                found.push(format!("{what} contains {bad:?}: ...{near}..."));
            }
        }
        let w = words(&lower);
        for bad in NEVER_WORDS {
            if w.iter().any(|x| x == bad) {
                found.push(format!("{what} says {bad:?}"));
            }
        }
    }
    for (what, text) in &texts {
        found.extend(helper_or_location(what, text));
    }
    assert!(found.is_empty(), "{}", found.join("\n"));
    // The settings file may be named in schemas (its fields are edited
    // there until settings move into the books, P1), but no tool's
    // title or description, no refusal, the connect text and no
    // handbook section names it: it is the file books.settings names.
    let mut prose: Vec<(String, String)> = BooksServer::tool_router()
        .list_all()
        .iter()
        .map(|t| {
            (
                format!("tool {}", t.name),
                format!(
                    "{} {}",
                    t.title.as_deref().unwrap_or_default(),
                    t.description.as_deref().unwrap_or_default()
                ),
            )
        })
        .collect();
    prose.extend(
        texts
            .into_iter()
            .filter(|(what, _)| !what.starts_with("tool ")),
    );
    for (what, text) in &prose {
        assert!(
            !text.to_lowercase().contains("wanderbooks.toml"),
            "{what} names the settings file: {text}"
        );
    }
}

/// Tool descriptions are at most 700 characters (plan I-12). The
/// ones over it today are listed, each to be shortened in P5 when its
/// text moves into the handbook; no new one may join them, and one
/// that shrinks under the cap must leave the list.
#[test]
fn tool_descriptions_fit_700_characters() {
    // Shorten in P5.
    const OVER: &[&str] = &[
        "books.export",
        "books.export_cpa_package",
        "books.import_bank_csv",
        "books.reconcile",
        "books.set_entry_party",
        "books.set_w9",
        "books.show",
        "books.vendor_1099",
    ];
    let mut over: Vec<String> = BooksServer::tool_router()
        .list_all()
        .iter()
        .filter(|t| t.description.as_deref().unwrap_or_default().chars().count() > 700)
        .map(|t| t.name.to_string())
        .collect();
    over.sort();
    assert_eq!(over, OVER, "tool descriptions over 700 characters");
}

/// The tools docs/mcp-interface.md documents: a line (or a bullet)
/// that starts with `` `books.NAME` `` or `` `books.NAME { `` , and
/// any `` `books.NAME { `` signature within a line.
fn documented_tools(doc: &str) -> std::collections::BTreeSet<String> {
    let name_at = |s: &str| -> Option<(String, char)> {
        let rest = s.strip_prefix("`books.")?;
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_')
            .collect();
        let next = rest[name.len()..].chars().next()?;
        (!name.is_empty()).then(|| (format!("books.{name}"), next))
    };
    let mut out = std::collections::BTreeSet::new();
    for line in doc.lines() {
        let t = line.trim_start();
        let t = t.strip_prefix("- ").unwrap_or(t);
        if let Some((name, next)) = name_at(t)
            && (next == '`' || next == ' ')
        {
            out.insert(name);
        }
        for (i, _) in line.match_indices("`books.") {
            if let Some((name, ' ')) = name_at(&line[i..])
                && line[i + 1 + name.len()..].starts_with(" {")
            {
                out.insert(name);
            }
        }
    }
    out
}

/// Every tool the server has is documented in docs/mcp-interface.md,
/// and every tool the doc documents is one the server has.
#[test]
fn every_tool_is_documented_and_every_documented_tool_exists() {
    let doc = include_str!("../../../docs/mcp-interface.md");
    let documented = documented_tools(doc);
    let tools: std::collections::BTreeSet<String> = BooksServer::tool_router()
        .list_all()
        .iter()
        .map(|t| t.name.to_string())
        .collect();
    let undocumented: Vec<&String> = tools.difference(&documented).collect();
    let unknown: Vec<&String> = documented.difference(&tools).collect();
    assert!(
        undocumented.is_empty(),
        "tools docs/mcp-interface.md doesn't document: {undocumented:?}"
    );
    assert!(
        unknown.is_empty(),
        "docs/mcp-interface.md documents tools there aren't: {unknown:?}"
    );
    // The reader catches what it should, and only that.
    let sample = "- `books.chart` → x\n  `books.decisions { a? }`; \
                      `books.open_items { b? }`.\nsee `books.cash_accounts`, \
                      `books.show`.";
    let got: Vec<String> = documented_tools(sample).into_iter().collect();
    assert_eq!(got, ["books.chart", "books.decisions", "books.open_items"]);
}

/// rules.md B-30 (defect 16): every report tool says what its omitted
/// dates mean in the one sentence the resolver's kind gives
/// (PeriodKind::describe), and each date parameter in the schema's
/// own words (START_DOC, END_DOC, AS_OF_DOC, YEAR_DOC); the two tools
/// over the description budget (books.export, books.show) and the CPA
/// package say it in their parameters. No tool or schema says "all
/// time", "beginning of time" or "first entry" any more. The resources
/// read with dates omitted say which period they cover.
#[test]
fn period_tools_describe_the_one_default() {
    use ledger_core::calendar::{AS_OF_DOC, END_DOC, PeriodKind, START_DOC, YEAR_DOC};
    let tools = BooksServer::tool_router().list_all();
    let tool = |name: &str| {
        tools
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("no tool {name}"))
    };
    let field = |name: &str, f: &str| -> String {
        tool(name).input_schema["properties"][f]["description"]
            .as_str()
            .unwrap_or_else(|| panic!("{name}.{f} has no description"))
            .to_string()
    };
    for (name, kind, fields) in [
        (
            "books.profit_and_loss",
            PeriodKind::Statement,
            &[("start", START_DOC), ("end", END_DOC)][..],
        ),
        (
            "books.profit_and_loss_by_class",
            PeriodKind::Statement,
            &[("start", START_DOC), ("end", END_DOC)][..],
        ),
        (
            "books.project",
            PeriodKind::Statement,
            &[("start", START_DOC), ("end", END_DOC)][..],
        ),
        (
            "books.cash_flow",
            PeriodKind::Statement,
            &[("start", START_DOC), ("end", END_DOC)][..],
        ),
        (
            "books.trial_balance",
            PeriodKind::AsOf,
            &[("as_of", AS_OF_DOC)][..],
        ),
        (
            "books.balance_sheet",
            PeriodKind::AsOf,
            &[("as_of", AS_OF_DOC)][..],
        ),
        (
            "books.control_check",
            PeriodKind::AsOf,
            &[("as_of", AS_OF_DOC)][..],
        ),
        (
            "books.documents",
            PeriodKind::AsOf,
            &[("as_of", AS_OF_DOC)][..],
        ),
        (
            "books.document",
            PeriodKind::AsOf,
            &[("as_of", AS_OF_DOC)][..],
        ),
        (
            "books.ar_aging",
            PeriodKind::AsOf,
            &[("as_of", AS_OF_DOC)][..],
        ),
        (
            "books.ap_aging",
            PeriodKind::AsOf,
            &[("as_of", AS_OF_DOC)][..],
        ),
        (
            "books.parties",
            PeriodKind::CalendarYear,
            &[("year", YEAR_DOC)][..],
        ),
        (
            "books.party",
            PeriodKind::CalendarYear,
            &[("year", YEAR_DOC)][..],
        ),
    ] {
        let d = tool(name).description.as_deref().unwrap_or_default();
        assert!(d.ends_with(kind.describe()), "{name}: {d}");
        for (f, doc) in fields {
            assert_eq!(field(name, f), *doc, "{name}.{f}");
        }
    }
    // Over the budget, or the package: in the parameters' words.
    for (name, f, doc) in [
        ("books.export", "start", START_DOC),
        ("books.export", "end", END_DOC),
        ("books.export", "as_of", AS_OF_DOC),
        ("books.show", "start", START_DOC),
        ("books.show", "end", END_DOC),
        ("books.export_cpa_package", "as_of", AS_OF_DOC),
    ] {
        assert!(
            field(name, f).ends_with(doc),
            "{name}.{f}: {}",
            field(name, f)
        );
    }
    assert!(
        tool("books.export")
            .description
            .as_deref()
            .unwrap()
            .contains("omitted: the fiscal year to date")
    );
    // Nothing left of the old defaults, anywhere.
    for t in tools.iter() {
        let text = format!(
            "{} {}",
            t.description.as_deref().unwrap_or_default(),
            serde_json::Value::Object((*t.input_schema).clone())
        )
        .to_lowercase();
        for stale in [
            "all time",
            "beginning of time",
            "first entry",
            "first posted entry",
        ] {
            assert!(!text.contains(stale), "{} says {stale:?}", t.name);
        }
    }
    let resources = resource_list();
    let resource = |uri: &str| {
        resources
            .iter()
            .find(|r| r.uri == uri)
            .and_then(|r| r.description.clone())
            .unwrap()
    };
    assert!(resource("books://reports/profit-and-loss").contains("fiscal year to date"));
    assert!(resource("books://reports/trial-balance").contains("as of today"));
    assert!(resource("books://reports/balance-sheet").contains("as of today"));
}

/// How many masked TINs a text holds.
fn data_masked(text: &str) -> usize {
    text.matches("•••-••-••••").count() + text.matches("•••••••••").count()
}

/// redact_error (B-26): every full TIN in an error's message and data
/// is masked, wherever the data holds it (a string, in an array, in a
/// nested object), and what holds none is left as it was.
#[test]
fn redact_error_masks_every_tin_in_the_message_and_data() {
    let tin = "291-12-2846";
    let e = redact_error(rmcp::ErrorData::resource_not_found(
        format!("books://receipts/{tin}"),
        Some(serde_json::json!({
            "reason": format!("bad receipt resource URI: {tin}"),
            "list": ["plain", tin, ["deeper 291122846"]],
            "nested": {"uri": tin, "n": 5, "fine": "no tin here 2026-10-16"},
        })),
    ));
    let text = format!("{} {}", e.message, e.data.as_ref().unwrap());
    assert!(!text.contains(tin) && !text.contains("291122846"), "{text}");
    assert_eq!(data_masked(&text), 5, "every TIN masked: {text}");
    let data = e.data.unwrap();
    assert_eq!(data["list"][0], "plain");
    assert_eq!(data["nested"]["n"], 5);
    assert_eq!(data["nested"]["fine"], "no tin here 2026-10-16");
}

/// Fail closed (SharedStore INVARIANTS 1-2): a request that panics,
/// with the store held or not, stops the server, and every later
/// request, tool or lock, is refused with the plain STOPPED message,
/// never the panic's text (which could carry anything, a TIN
/// included).
#[test]
fn a_request_that_panics_stops_the_server() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    type Body = fn(BooksServer) -> Result<u64, String>;
    async fn run(server: &BooksServer, body: Body) -> Result<Result<u64, String>, rmcp::ErrorData> {
        let s = server.clone();
        run_blocking(&server.store, move || Box::pin(async move { body(s) })).await
    }
    let bodies: [(&str, Body); 2] = [
        ("holding the store", |s| {
            let _store = s.lock()?;
            panic!("mid-mutation 123-45-6789");
        }),
        ("without the store", |_| {
            panic!("before the lock 123-45-6789")
        }),
    ];
    for (how, body) in bodies {
        let books = unit_dir("stops");
        let parts = Served::open(store(&books)).unwrap();
        let server = BooksServer::new(&parts);
        rt.block_on(async {
            let seq: Body = |s| s.lock().map(|st| st.journal_seq());
            // New books: store.created, then their journal format.
            assert_eq!(run(&server, seq).await.unwrap(), Ok(2), "{how}");
            let died = run(&server, body).await.unwrap_err();
            assert_eq!(died.message, STOPPED, "{how}");
            // Every later request is refused, before it runs.
            let after = run(&server, seq).await.unwrap_err();
            assert_eq!(after.message, STOPPED, "{how}");
            // So is any other session with the books.
            let other = BooksServer::new(&parts);
            assert_eq!(run(&other, seq).await.unwrap_err().message, STOPPED);
        });
        // And the lock itself, from any thread.
        let s = server.clone();
        let locked = std::thread::spawn(move || s.lock().map(|_| ()))
            .join()
            .unwrap();
        assert_eq!(locked, Err(STOPPED.to_string()), "{how}");
    }

    // A guard dropped by a panic on a thread of its own stops it too.
    let books = unit_dir("stops-thread");
    let server = served(store(&books));
    let s = server.clone();
    let died = std::thread::spawn(move || {
        let _store = s.lock().unwrap();
        panic!("mid-mutation");
    })
    .join();
    assert!(died.is_err());
    assert_eq!(server.lock().map(|_| ()), Err(STOPPED.to_string()));
}

/// run_blocking drives a request to completion even when it waits: a
/// future that is pending until another thread wakes it completes,
/// with its own result (the waker unparks the request's thread).
#[test]
fn a_request_that_waits_is_woken_and_completes() {
    /// Pending on its first poll, woken from another thread shortly
    /// after; ready on the next.
    struct WakeLater(bool);
    impl std::future::Future for WakeLater {
        type Output = u64;
        fn poll(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<u64> {
            if self.0 {
                return std::task::Poll::Ready(7);
            }
            self.0 = true;
            let waker = cx.waker().clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(20));
                waker.wake();
            });
            std::task::Poll::Pending
        }
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let books = unit_dir("wakes");
    let shared = SharedStore::new(store(&books));
    let got = rt.block_on(async {
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            run_blocking(&shared, || Box::pin(WakeLater(false))),
        )
        .await
    });
    // A request never woken would hold its thread: don't wait for it.
    rt.shutdown_background();
    assert_eq!(got.ok().and_then(Result::ok), Some(7), "never woken");
}

// Calls through the access door (access.rs), driven straight into the
// handler with a context the HTTP door would have left: its Caller,
// or none.

/// A peer to make request contexts with: a client and a server over
/// an in-memory pipe, kept alive while it is.
struct Session {
    peer: rmcp::service::Peer<RoleServer>,
    _server: rmcp::service::RunningService<RoleServer, BooksServer>,
    _client: rmcp::service::RunningService<rmcp::service::RoleClient, ()>,
    _books: TestDir,
}

async fn session(tag: &str) -> Session {
    use rmcp::ServiceExt;
    let books = unit_dir(&format!("{tag}-peer"));
    let server = served(store(&books));
    let (a, b) = tokio::io::duplex(1 << 16);
    let served = tokio::spawn(async move { server.serve(a).await });
    let client = ().serve(b).await.unwrap();
    let server = served.await.unwrap().unwrap();
    Session {
        peer: server.peer().clone(),
        _server: server,
        _client: client,
        _books: books,
    }
}

/// A request's context: from a token with `scopes` (the door's
/// Caller), or with none.
fn context(
    peer: &rmcp::service::Peer<RoleServer>,
    scopes: Option<&[ledger_store::auth::Scope]>,
) -> RequestContext<RoleServer> {
    let mut cx = RequestContext::new(rmcp::model::NumberOrString::Number(7), peer.clone());
    if let Some(scopes) = scopes {
        let token = ledger_store::auth::Token {
            id: "tok_0123456789ab".into(),
            book: "acme".into(),
            name: "test".into(),
            scopes: scopes.iter().copied().collect(),
            created: "2026-10-20T00:00:00Z".into(),
            sha256: String::new(),
        };
        let (mut parts, ()) = axum::http::Request::new(()).into_parts();
        parts.extensions.insert(crate::access::Caller { token });
        cx.extensions.insert(parts);
    }
    cx
}

const ALL: &[ledger_store::auth::Scope] = &[
    ledger_store::auth::Scope::Read,
    ledger_store::auth::Scope::Write,
    ledger_store::auth::Scope::Post,
    ledger_store::auth::Scope::Admin,
];

/// Books as the books server serves them: calls are checked against
/// their caller's token and recorded in the access log beside them.
fn logged(tag: &str) -> (BooksServer, TestDir) {
    let books = unit_dir(tag);
    let server = served(store(&books));
    (server, books)
}

/// Books whose access log can't be written: its file is a FIFO no one
/// reads, so every record fails (as on a full disk).
fn unwritable(tag: &str) -> (BooksServer, TestDir) {
    let books = unit_dir(tag);
    let store = store(&books);
    let fifo = books.0.join(ledger_store::access::ACCESS_FILE);
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap();
    assert!(made.success());
    let path = fifo.clone();
    let other_end = std::thread::spawn(move || {
        // The log reads the file whole as it opens (empty: a writer
        // that goes at once), then opens it to append (a reader that
        // goes at once): nothing reads what is written after.
        drop(std::fs::OpenOptions::new().write(true).open(&path).unwrap());
        drop(std::fs::File::open(&path).unwrap());
    });
    let server = served(store);
    other_end.join().unwrap();
    (server, books)
}

/// The access log's records: (subject, outcome, whether it carries
/// the arguments' digest).
fn records(books: &TestDir) -> Vec<(String, ledger_store::access::Outcome, bool)> {
    ledger_store::access::read(&books.0)
        .unwrap()
        .0
        .into_iter()
        .map(|r| {
            (
                r.access.subject,
                r.access.result,
                r.access.args_digest.is_some(),
            )
        })
        .collect()
}

fn refused(code: &str) -> ledger_store::access::Outcome {
    ledger_store::access::Outcome::Refused(code.into())
}

fn tool_call(name: &str, args: serde_json::Value) -> rmcp::model::CallToolRequestParams {
    rmcp::model::CallToolRequestParams::new(name.to_string())
        .with_arguments(args.as_object().cloned().unwrap_or_default())
}

/// A tool's answer: Ok((is it a refusal, its text)), or the protocol
/// error.
fn answer(
    out: Result<rmcp::model::CallToolResponse, rmcp::ErrorData>,
) -> Result<(bool, String), rmcp::ErrorData> {
    match out? {
        rmcp::model::CallToolResponse::Complete(r) => Ok((
            r.is_error == Some(true),
            r.content
                .iter()
                .find_map(|c| c.as_text().map(|t| t.text.clone()))
                .unwrap_or_default(),
        )),
        other => panic!("not a complete answer: {other:?}"),
    }
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// access.rs INVARIANTS 1-3: a call that reaches the books without a
/// Caller, or names a tool no table knows, is refused unrun (fail
/// closed), told why in words, and recorded so.
#[test]
fn a_call_without_a_caller_or_to_no_tool_is_refused_unrun_and_recorded() {
    rt().block_on(async {
        let s = session("door").await;
        let (server, books) = logged("door");
        let add = || {
            tool_call(
                "books.add_account",
                serde_json::json!({"code": "1000", "name": "Cash", "account_type": "asset"}),
            )
        };
        let (refusal, text) =
            answer(server.call_tool(add(), context(&s.peer, None)).await).unwrap();
        assert!(refusal, "{text}");
        assert!(text.contains("without a token"), "{text}");
        // An unknown tool is a protocol error (mcp-tools.md), never run.
        let unknown = tool_call("books.no_such_tool", serde_json::json!({}));
        let err = server
            .call_tool(unknown, context(&s.peer, Some(ALL)))
            .await
            .unwrap_err();
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS, "{err:?}");
        assert!(err.message.contains("there is no tool"), "{err:?}");
        // Neither ran: the account isn't there.
        let chart = tool_call("books.chart", serde_json::json!({}));
        let (refusal, text) =
            answer(server.call_tool(chart, context(&s.peer, Some(ALL))).await).unwrap();
        assert!(!refusal, "{text}");
        assert!(!text.contains("Cash"), "{text}");
        assert_eq!(
            records(&books),
            vec![
                ("books.add_account".into(), refused("unauthorized"), true),
                ("books.no_such_tool".into(), refused("unknown_tool"), true),
                (
                    "books.chart".into(),
                    ledger_store::access::Outcome::Ok,
                    true
                ),
            ]
        );
    });
}

/// SharedStore INVARIANT 3: once the books are closing a call or a
/// read isn't run, and says so with the not-run code (a JSON-RPC
/// server error, which the agents' bridge sends again once the books
/// are back); each is recorded as refused.
#[test]
fn a_call_while_the_books_close_is_refused_as_not_run() {
    rt().block_on(async {
        let s = session("closing").await;
        let (server, books) = logged("closing");
        server.store.close().await;
        let chart = tool_call("books.chart", serde_json::json!({}));
        let e = server
            .call_tool(chart, context(&s.peer, Some(ALL)))
            .await
            .unwrap_err();
        assert_eq!(e.code.0, NOT_RUN_CLOSING_CODE);
        assert_eq!(e.message, NOT_RUN_CLOSING);
        // JSON-RPC 2.0 5.1: -32000 to -32099 is for the server's own
        // errors.
        assert!((-32099..=-32000).contains(&NOT_RUN_CLOSING_CODE));
        assert!((-32099..=-32000).contains(&NOT_RUN_STOPPED_CODE));
        let read = ReadResourceRequestParams::new("books://chart");
        let e = server
            .read_resource(read, context(&s.peer, Some(ALL)))
            .await
            .unwrap_err();
        assert_eq!(e.code.0, NOT_RUN_CLOSING_CODE);
        assert_eq!(
            records(&books),
            vec![
                ("books.chart".into(), refused("closing"), true),
                ("books://chart".into(), refused("closing"), false),
            ]
        );
    });
}

/// Reading a resource is reading the books (access.rs): only a token
/// with books:read reads one, and every read is recorded with what
/// came of it.
#[test]
fn a_resource_is_read_only_with_books_read_and_each_read_is_recorded() {
    use ledger_store::access::Outcome;
    rt().block_on(async {
        let s = session("resources").await;
        let (server, books) = logged("resources");
        let read = |uri: &str| ReadResourceRequestParams::new(uri);
        assert!(
            server
                .read_resource(read("books://chart"), context(&s.peer, Some(ALL)))
                .await
                .is_ok()
        );
        let writer = [ledger_store::auth::Scope::Write];
        let e = server
            .read_resource(read("books://chart"), context(&s.peer, Some(&writer[..])))
            .await
            .unwrap_err();
        assert_eq!(e.message, crate::access::INSUFFICIENT_SCOPE);
        let e = server
            .read_resource(read("books://chart"), context(&s.peer, None))
            .await
            .unwrap_err();
        assert_eq!(e.message, crate::access::UNAUTHORIZED);
        // One that ran and failed is an error, not a refusal.
        assert!(
            server
                .read_resource(read("books://nope"), context(&s.peer, Some(ALL)))
                .await
                .is_err()
        );
        let outcomes: Vec<_> = records(&books).into_iter().map(|r| r.1).collect();
        assert_eq!(
            outcomes,
            vec![
                Outcome::Ok,
                refused("insufficient_scope"),
                refused("unauthorized"),
                Outcome::Error,
            ]
        );
    });
}

/// SharedStore INVARIANTS 2-3 at the last moment: a call that passed
/// the checks, then found the books closing (or the server stopped)
/// as it went to run, isn't run either, and is answered and recorded
/// as one refused at the start would be. The settings are held so
/// the call waits between its checks and its run.
#[test]
fn a_call_the_books_close_or_stop_under_is_refused_as_not_run() {
    let rt = rt();
    for (how, stops) in [("closes", false), ("stops", true)] {
        let (server, books) = logged(&format!("late-{how}"));
        // The settings, held on a thread of their own until released.
        let (locked, is_locked) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel::<()>();
        let settings = server.settings.clone();
        let holder = std::thread::spawn(move || {
            let _held = settings.lock().unwrap();
            locked.send(()).unwrap();
            let _ = released.recv();
        });
        is_locked.recv().unwrap();
        let answered = rt.block_on(async {
            let s = session(&format!("late-{how}")).await;
            let (started, began) = tokio::sync::oneshot::channel();
            let calling = server.clone();
            let cx = context(&s.peer, Some(ALL));
            let call = tokio::spawn(async move {
                let _ = started.send(());
                let chart = tool_call("books.chart", serde_json::json!({}));
                calling.call_tool(chart, cx).await
            });
            began.await.unwrap();
            // Past its checks (no I/O between): waiting for the settings.
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            if stops {
                server.store.stop();
            } else {
                server
                    .store
                    .closing
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }
            release.send(()).unwrap();
            call.await.unwrap()
        });
        holder.join().unwrap();
        if stops {
            assert_eq!(answer(answered).unwrap(), (true, STOPPED.to_string()));
            // Refused after its arguments were taken: their digest too.
            assert_eq!(
                records(&books),
                vec![("books.chart".into(), refused("stopped"), true)]
            );
        } else {
            let e = answered.unwrap_err();
            assert_eq!(e.code.0, NOT_RUN_CLOSING_CODE, "{e:?}");
            // Its arguments went with the request: no digest.
            assert_eq!(
                records(&books),
                vec![("books.chart".into(), refused("closing"), false)]
            );
        }
    }
}

/// access.rs INVARIANT 3: a record that can't be written stops the
/// books, and the answer says so; a call or read refused as not run
/// (closing) keeps its not-run code, so the bridge still sends it
/// again; anything else is an error in words.
#[test]
fn an_access_record_that_cant_be_written_stops_the_books_and_says_so() {
    rt().block_on(async {
        let s = session("unrecorded").await;
        // A call refused as the books close.
        let (server, _books) = unwritable("unrecorded-call");
        server.store.close().await;
        let chart = tool_call("books.chart", serde_json::json!({}));
        let e = server
            .call_tool(chart, context(&s.peer, Some(ALL)))
            .await
            .unwrap_err();
        assert_eq!(e.code.0, NOT_RUN_CLOSING_CODE);
        assert!(e.message.contains(crate::access::NOT_RECORDED), "{e:?}");
        assert!(server.store.stopped());
        // Any other answer refused unrun becomes the words alone.
        let (server, _books) = unwritable("unrecorded-refusal");
        let call = crate::access::Call::new(
            &server.access,
            &context(&s.peer, Some(ALL)),
            "tool",
            "books.chart",
        );
        let out = server
            .refuse(
                &call,
                crate::access::STOPPED,
                Err(rmcp::ErrorData::internal_error("not run", None)),
            )
            .await;
        assert_eq!(
            answer(out).unwrap(),
            (true, crate::access::NOT_RECORDED.to_string())
        );
        assert!(server.store.stopped());
        // A read refused as the books close.
        let (server, _books) = unwritable("unrecorded-closing-read");
        server.store.close().await;
        let e = server
            .read_resource(
                ReadResourceRequestParams::new("books://chart"),
                context(&s.peer, Some(ALL)),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code.0, NOT_RUN_CLOSING_CODE);
        assert!(e.message.contains(crate::access::NOT_RECORDED), "{e:?}");
        assert!(server.store.stopped());
        // A read that ran and failed.
        let (server, _books) = unwritable("unrecorded-read");
        let e = server
            .read_resource(
                ReadResourceRequestParams::new("books://nope"),
                context(&s.peer, Some(ALL)),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, rmcp::model::ErrorCode::INTERNAL_ERROR);
        assert_eq!(e.message, crate::access::NOT_RECORDED);
        assert!(server.store.stopped());
    });
}

/// A request whose blocking work the runtime never ran (it was
/// shutting down) is answered as not run, in the words a stopped
/// server uses, and stops nothing: only a request that panicked
/// stops the books (run_blocking).
#[test]
fn a_request_the_runtime_never_ran_is_answered_not_run_and_stops_nothing() {
    let dead = rt();
    let handle = dead.handle().clone();
    drop(dead);
    rt().block_on(async {
        let s = session("never-ran").await;
        let (server, _books) = logged("never-ran");
        let chart = tool_call("books.chart", serde_json::json!({}));
        let mut call = std::pin::pin!(server.call_tool(chart, context(&s.peer, Some(ALL))));
        // Its blocking work goes to the runtime that shut down.
        let out = std::future::poll_fn(|cx| {
            let _in = handle.enter();
            call.as_mut().poll(cx)
        })
        .await;
        assert_eq!(answer(out).unwrap(), (true, STOPPED.to_string()));
        assert!(!server.store.stopped());
    });
}

/// A tool list differs by caller (scopes.rs): a client on 2026-07-28
/// or later is told to cache it for this caller alone, and not to keep
/// it; an older one, which knows neither hint, is told neither.
#[test]
fn a_tool_list_is_cached_only_for_its_caller() {
    rt().block_on(async {
        let s = session("hints").await;
        let books = unit_dir("hints");
        let server = served(store(&books));
        for (version, hinted) in [
            (rmcp::model::ProtocolVersion::V_2026_07_28, true),
            (rmcp::model::ProtocolVersion::V_2025_11_25, false),
        ] {
            let mut cx = context(&s.peer, Some(ALL));
            cx.meta.set_protocol_version(version.clone());
            let list = server.list_tools(None, cx).await.unwrap();
            assert!(!list.tools.is_empty());
            assert_eq!(list.ttl_ms, hinted.then_some(0), "{version:?}");
            assert_eq!(
                list.cache_scope,
                hinted.then_some(rmcp::model::CacheScope::Private),
                "{version:?}"
            );
        }
    });
}
