//! Integration tests for `wanderbooks new`: one command lands a complete
//! books dir with the MCP server registered for it, the TOML and JSON
//! parse, and re-running is refused without `--force` (which never
//! touches the company profile).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_wanderbooks"))
}

/// Stand-ins for the installed binaries, so tests never run cargo install.
fn fake_bins(tmp: &Path) -> PathBuf {
    let dir = tmp.join("bin");
    fs::create_dir_all(&dir).unwrap();
    for b in ["ledger-mcp", "ledger-gui"] {
        fs::write(dir.join(b), "#!/bin/sh\n").unwrap();
    }
    dir
}

fn run(tmp: &Path, args: &[&str]) -> (bool, String) {
    let path = format!(
        "{}:{}",
        fake_bins(tmp).display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let out = Command::new(bin())
        .args(args)
        .arg("--no-start")
        .env("PATH", path)
        .output()
        .expect("run wanderbooks");
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.success(), s)
}

fn new(tmp: &Path, books: &Path, extra: &[&str]) -> (bool, String) {
    let mut args = vec!["new", books.to_str().unwrap()];
    args.extend(extra);
    run(tmp, &args)
}

#[test]
fn scaffold_lands_complete() {
    let tmp = tempfile::tempdir().unwrap();
    let books = tmp.path().join("acme-books");
    let (ok, out) = new(tmp.path(), &books, &[]);
    assert!(ok, "new should succeed: {out}");
    for f in [
        "justfile",
        "AGENTS.md",
        "CLAUDE.md",
        "wanderbooks.toml",
        ".gitignore",
        ".mcp.json",
        ".claude/settings.json",
        ".codex/hooks.json",
    ] {
        let p = books.join(f);
        assert!(p.exists(), "{f} exists");
        assert!(!fs::read_to_string(&p).unwrap().is_empty(), "{f} non-empty");
    }
    assert!(books.join("books-data").is_dir(), "books-data dir exists");
    assert!(books.join(".git").is_dir(), "fresh dir gets git init");
    let log = Command::new("git")
        .args(["log", "--oneline"])
        .current_dir(&books)
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&log.stdout).contains("init"),
        "scaffold committed"
    );
}

#[test]
fn the_server_is_registered_for_this_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let books = tmp.path().join("b");
    assert!(new(tmp.path(), &books, &[]).0);
    let books = books.canonicalize().unwrap();
    let mcp: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(books.join(".mcp.json")).unwrap()).unwrap();
    let server = &mcp["mcpServers"]["wanderbooks"];
    assert!(server["command"].as_str().unwrap().ends_with("ledger-mcp"));
    assert_eq!(
        server["args"],
        serde_json::json!(["--data-dir", books.join("books-data").to_string_lossy()])
    );
    let settings: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(books.join(".claude/settings.json")).unwrap())
            .unwrap();
    assert_eq!(
        settings["enabledMcpjsonServers"],
        serde_json::json!(["wanderbooks"])
    );
    // Every session starts by checking the viewer against open_at_start.
    let hook = settings["hooks"]["SessionStart"][0]["hooks"][0]["command"]
        .as_str()
        .expect("SessionStart hook command");
    assert!(hook.ends_with("just session-start"), "{hook}");
    let codex: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(books.join(".codex/hooks.json")).unwrap())
            .unwrap();
    let hook = codex["hooks"]["SessionStart"][0]["hooks"][0]["command"]
        .as_str()
        .expect("Codex SessionStart hook command");
    assert!(hook.ends_with("just session-start"), "{hook}");
    // The justfile opens this dir's books in the GUI.
    let just = fs::read_to_string(books.join("justfile")).unwrap();
    assert!(just.contains("ledger-gui\" --data-dir \"{{justfile_directory()}}/books-data\""));
    for recipe in [
        "agent:",
        "gui:",
        "gui-fg:",
        "gui-status:",
        "session-start:",
        "backup:",
        "update:",
    ] {
        assert!(just.contains(recipe), "justfile has {recipe}");
    }
    assert_eq!(
        fs::read_to_string(books.join("CLAUDE.md")).unwrap().trim(),
        "@AGENTS.md"
    );
    let toml: toml::Value =
        toml::from_str(&fs::read_to_string(books.join("wanderbooks.toml")).unwrap()).unwrap();
    assert_eq!(toml["books"]["data_dir"].as_str(), Some("books-data"));
}

#[test]
fn agents_md_carries_the_contract() {
    let tmp = tempfile::tempdir().unwrap();
    let books = tmp.path().join("b");
    assert!(new(tmp.path(), &books, &[]).0);
    let md = fs::read_to_string(books.join("AGENTS.md")).unwrap();
    for phrase in [
        "preview",
        "Posted is forever",
        "reversing entries",
        "receipt",
        "journal.jsonl",
        "onboard",
        "chart_template",
        "to the cent",
        "just gui",
        "just gui-status",
        "open_at_start",
    ] {
        assert!(md.contains(phrase), "AGENTS.md mentions {phrase:?}");
    }
}

#[test]
fn rerun_refused_without_force_and_never_clobbers_the_profile() {
    let tmp = tempfile::tempdir().unwrap();
    let books = tmp.path().join("b");
    assert!(new(tmp.path(), &books, &[]).0);
    let (ok, out) = new(tmp.path(), &books, &[]);
    assert!(!ok, "second new without --force fails");
    assert!(out.contains("--force"), "error points at --force: {out}");
    fs::write(
        books.join("wanderbooks.toml"),
        "[company]\nname = \"Acme LLC\"\n",
    )
    .unwrap();
    let (ok, _) = new(tmp.path(), &books, &["--force"]);
    assert!(ok, "second new with --force succeeds");
    assert!(
        fs::read_to_string(books.join("wanderbooks.toml"))
            .unwrap()
            .contains("Acme LLC"),
        "--force rewrites the scaffold, never the company profile"
    );
}

#[test]
fn init_is_an_alias_and_respects_an_existing_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let books = tmp.path().join("b");
    fs::create_dir_all(&books).unwrap();
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(&books)
            .status()
            .unwrap()
            .success()
    );
    let (ok, out) = run(tmp.path(), &["init", books.to_str().unwrap()]);
    assert!(ok, "{out}");
    assert!(books.join("wanderbooks.toml").exists());
}

#[test]
fn codex_gets_its_own_named_server() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = fake_bins(tmp.path());
    let log = tmp.path().join("codex-args");
    // A stand-in codex that records how it was called.
    fs::write(
        bin.join("codex"),
        format!("#!/bin/sh\necho \"$@\" > {}\n", log.display()),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(bin.join("codex"), fs::Permissions::from_mode(0o755)).unwrap();
    let books = tmp.path().join("Acme Books");
    let (ok, out) = new(tmp.path(), &books, &["--codex"]);
    assert!(ok, "{out}");
    let args = fs::read_to_string(&log).unwrap();
    let books = books.canonicalize().unwrap();
    assert!(
        args.starts_with("mcp add wanderbooks-acme-books -- "),
        "{args}"
    );
    assert!(
        args.trim_end().ends_with(&format!(
            "--data-dir {}",
            books.join("books-data").display()
        )),
        "{args}"
    );
    let just = fs::read_to_string(books.join("justfile")).unwrap();
    assert!(just.contains("codex mcp add \"wanderbooks-acme-books\""));
    assert!(out.contains("just codex"), "{out}");
}

/// The `wanderbooks.toml` a fresh `new` writes.
fn scaffolded_toml(tmp: &Path) -> (PathBuf, String) {
    let books = tmp.join("b");
    assert!(new(tmp, &books, &[]).0);
    let path = books.join("wanderbooks.toml");
    let text = fs::read_to_string(&path).unwrap();
    (path, text)
}

/// The example a commented line holds, if it is one: `# key = value`, or
/// `# [section]` / `# [[section]]`. Prose comments are not examples.
fn example(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("# ")?;
    let is_key = |k: &str| !k.is_empty() && k.bytes().all(|b| b.is_ascii_lowercase() || b == b'_');
    let header = rest
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split('.')
        .all(is_key);
    let assignment = rest
        .split_once(" = ")
        .is_some_and(|(k, v)| is_key(k) && !v.trim().is_empty());
    ((rest.starts_with('[') && rest.ends_with(']') && header) || assignment).then_some(rest)
}

/// The template with the examples `pick` chooses uncommented.
fn uncomment(text: &str, pick: impl Fn(&str) -> bool) -> String {
    text.lines()
        .map(|l| match example(l) {
            Some(ex) if pick(ex) => ex,
            _ => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_scaffolded_toml_loads_with_the_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let (path, text) = scaffolded_toml(tmp.path());
    let s = ledger_config::load(&path).expect("scaffolded wanderbooks.toml loads");
    let c = &s.company;
    assert_eq!(c.name, "Unnamed company", "name is left for onboarding");
    assert_eq!(c.fiscal_year_start, (1, 1));
    assert_eq!(c.basis, "Accrual");
    assert!(c.members.is_empty());
    assert_eq!(c.tax_form, None);
    assert_eq!(c.close_into, None);
    assert!(c.close_sweep.is_empty());
    assert_eq!(
        c.profile,
        [
            ("base_currency".to_string(), "USD".to_string()),
            ("fiscal_year_start".to_string(), "01-01".to_string()),
        ]
    );
    assert_eq!(s.receipt_threshold.as_deref(), Some("25.00"));
    assert!(s.cash_accounts.is_empty());
    assert!(s.financing_accounts.is_empty());
    assert!(s.investing_accounts.is_empty());
    let toml: toml::Value = toml::from_str(&text).unwrap();
    assert_eq!(toml["books"]["data_dir"].as_str(), Some("books-data"));
    // The justfile's session-start reads this line with sed, from column 0.
    assert!(
        text.lines().any(|l| l == "open_at_start = \"ask\""),
        "{text}"
    );
    assert!(text.contains("SSN"), "the template warns off storing SSNs");
}

#[test]
fn every_example_in_the_toml_is_valid() {
    let tmp = tempfile::tempdir().unwrap();
    let (_, text) = scaffolded_toml(tmp.path());
    let examples = text.lines().filter_map(example).count();
    assert!(examples >= 20, "only {examples} examples found");
    // close_into is for a single owner and refused with members, so it
    // gets a pass of its own; together the two passes cover every example.
    let single_owner = |ex: &str| ex.starts_with("close_into ");
    let all = ledger_config::parse(&uncomment(&text, |ex| !single_owner(ex)))
        .unwrap_or_else(|e| panic!("examples (without close_into) parse: {e}"));
    let c = &all.company;
    assert_eq!(c.name, "Acme Widgets LLC");
    assert_eq!(c.tax_form, Some(ledger_core::TaxForm::Form1065));
    assert_eq!(c.members.len(), 2);
    assert_eq!(c.members[1].distribution_account.as_deref(), Some("3101"));
    assert_eq!(c.close_sweep, ["3100"]);
    assert!(c.profile.iter().any(|(k, _)| k == "ein"));
    assert_eq!(all.cash_accounts, ["1000", "1010"]);
    assert_eq!(all.financing_accounts, ["2100"]);
    assert_eq!(all.investing_accounts, ["1400"]);
    let single = ledger_config::parse(&uncomment(&text, single_owner))
        .unwrap_or_else(|e| panic!("close_into example parses: {e}"));
    assert_eq!(single.company.close_into.as_deref(), Some("3000"));
    assert!(
        ledger_config::parse(&uncomment(&text, |_| true)).is_err(),
        "close_into with members is refused, as the template says"
    );
}

/// The serde field names of `struct name` in ledger-config's source.
fn fields(src: &str, name: &str) -> Vec<String> {
    let start = src
        .find(&format!("struct {name} {{"))
        .unwrap_or_else(|| panic!("ledger-config has struct {name}"));
    let body = &src[start..];
    let body = &body[..body.find("\n}").expect("struct ends")];
    let found: Vec<String> = body
        .lines()
        .skip(1)
        .map(str::trim)
        .filter(|l| !l.starts_with("//") && !l.starts_with("#["))
        .filter_map(|l| l.split_once(':'))
        .map(|(k, _)| k.trim_start_matches("pub ").to_string())
        .collect();
    assert!(!found.is_empty(), "struct {name} has fields");
    found
}

#[test]
fn the_toml_shows_every_setting_the_config_reads() {
    let tmp = tempfile::tempdir().unwrap();
    let (_, text) = scaffolded_toml(tmp.path());
    // (section, key) for every assignment, examples included.
    let mut section = String::new();
    let mut present = Vec::new();
    for line in uncomment(&text, |_| true).lines() {
        if line.starts_with('[') {
            section = line.trim_matches(['[', ']']).to_string();
        } else if let Some((k, _)) = line.split_once(" = ") {
            present.push((section.clone(), k.to_string()));
        }
    }
    let has = |s: &str, k: &str| present.iter().any(|(ps, pk)| ps == s && pk == k);
    // Read from the config crate's serde structs, so a new setting there
    // fails here until the template shows it.
    let src = include_str!("../../ledger-config/src/lib.rs");
    assert_eq!(fields(src, "BooksToml"), ["company", "books"]);
    for (strukt, sect) in [
        ("CompanyToml", "company"),
        ("MemberToml", "company.members"),
        ("BooksSection", "books"),
    ] {
        for key in fields(src, strukt) {
            if strukt == "CompanyToml" && key == "members" {
                assert!(text.contains("# [[company.members]]"));
                continue;
            }
            assert!(has(sect, &key), "[{sect}] {key} is in the template");
        }
    }
    // Read elsewhere: the justfile's session-start (main.rs), and the two
    // the scaffold has always written live.
    for (sect, key) in [
        ("viewer", "open_at_start"),
        ("company", "base_currency"),
        ("books", "data_dir"),
    ] {
        assert!(has(sect, key), "[{sect}] {key} is in the template");
    }
}
