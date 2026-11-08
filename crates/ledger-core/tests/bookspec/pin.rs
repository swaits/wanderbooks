use crate::*;

/// The Lean laws and the Rust checker must come from one bookspec rev
/// (D-69): `verify/lean` requires bookspec's Lean at a git rev, and the
/// crates depend on bookspec's checker and runner at a git rev. If the two
/// drift, `just prove` audits laws the tests do not run against — so this
/// test fails unless every pin names the same repository at the same commit.
///
/// The revs live in four files; the Lean one is in two of them (the
/// lakefile's `[[require]]` and the generated `lake-manifest.json`, which
/// Lake builds from, so both must agree).
fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A pin as one of the files states it: where it was found, the URL and
/// the commit.
struct Pin {
    source: String,
    url: String,
    rev: String,
}

fn lean_pins(root: &Path) -> Vec<Pin> {
    let mut pins = Vec::new();
    let lakefile = std::fs::read_to_string(root.join("verify/lean/lakefile.toml"))
        .expect("verify/lean/lakefile.toml");
    let doc: toml::Value = toml::from_str(&lakefile).expect("lakefile.toml parses");
    let require = doc
        .get("require")
        .and_then(toml::Value::as_array)
        .expect("lakefile.toml has [[require]]");
    let bookspec = require
        .iter()
        .find(|r| r.get("name").and_then(toml::Value::as_str) == Some("bookspec"))
        .expect("lakefile.toml requires bookspec");
    assert_eq!(
        bookspec.get("subDir").and_then(toml::Value::as_str),
        Some("lean"),
        "verify/lean/lakefile.toml requires bookspec from its lean/ package"
    );
    pins.push(Pin {
        source: "verify/lean/lakefile.toml".into(),
        url: bookspec
            .get("git")
            .and_then(toml::Value::as_str)
            .expect("the require names a git url")
            .into(),
        rev: bookspec
            .get("rev")
            .and_then(toml::Value::as_str)
            .expect("the require names a rev")
            .into(),
    });

    let manifest = std::fs::read_to_string(root.join("verify/lean/lake-manifest.json"))
        .expect("verify/lean/lake-manifest.json (commit it: Lake builds from it)");
    let doc: serde_json::Value = serde_json::from_str(&manifest).expect("manifest parses");
    let bookspec = doc
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .expect("manifest has packages")
        .iter()
        .find(|p| p.get("name").and_then(serde_json::Value::as_str) == Some("bookspec"))
        .expect("the manifest records bookspec");
    assert_eq!(
        bookspec.get("subDir").and_then(serde_json::Value::as_str),
        Some("lean"),
        "verify/lean/lake-manifest.json records bookspec's lean/ package"
    );
    pins.push(Pin {
        source: "verify/lean/lake-manifest.json".into(),
        url: bookspec
            .get("url")
            .and_then(serde_json::Value::as_str)
            .expect("the manifest records a url")
            .into(),
        rev: bookspec
            .get("rev")
            .and_then(serde_json::Value::as_str)
            .expect("the manifest records a rev")
            .into(),
    });
    pins
}

/// Every `bookspec-*` dependency in a crate's manifest, from whichever
/// table it is declared in.
fn cargo_pins(root: &Path, manifest: &str) -> Vec<Pin> {
    let path = root.join(manifest);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{manifest}"));
    let doc: toml::Value = toml::from_str(&text).unwrap_or_else(|_| panic!("{manifest} parses"));
    let mut pins = Vec::new();
    for table in ["dependencies", "dev-dependencies", "build-dependencies"] {
        let Some(deps) = doc.get(table).and_then(toml::Value::as_table) else {
            continue;
        };
        for (name, dep) in deps {
            if !name.starts_with("bookspec-") {
                continue;
            }
            let Some(dep) = dep.as_table() else { continue };
            let url = dep
                .get("git")
                .and_then(toml::Value::as_str)
                .unwrap_or_else(|| panic!("{manifest}: {name} is not a git dependency"));
            let rev = dep
                .get("rev")
                .and_then(toml::Value::as_str)
                .unwrap_or_else(|| panic!("{manifest}: {name} is pinned to a rev, not a branch"));
            pins.push(Pin {
                source: format!("{manifest}: {name}"),
                url: url.into(),
                rev: rev.into(),
            });
        }
    }
    assert!(!pins.is_empty(), "{manifest} pins no bookspec crate");
    pins
}

#[test]
pub(crate) fn bookspec_rev_is_pinned_once() {
    let root = repo_root();
    let mut pins = lean_pins(&root);
    pins.extend(cargo_pins(&root, "crates/ledger-core/Cargo.toml"));
    pins.extend(cargo_pins(&root, "crates/ledger-package/Cargo.toml"));

    let first = &pins[0];
    let differing: Vec<String> = pins
        .iter()
        .filter(|p| p.rev != first.rev || p.url != first.url)
        .map(|p| format!("  {}: {} @ {}", p.source, p.url, p.rev))
        .collect();
    assert!(
        differing.is_empty(),
        "the Lean laws and the Rust checker name different bookspec pins \
         (all four must be {} @ {}):\n{}",
        first.url,
        first.rev,
        differing.join("\n")
    );
    assert_eq!(first.url, "https://git.swaits.com/swaits/bookspec");
    assert_eq!(
        first.rev.len(),
        40,
        "a bookspec pin is a full commit: {}",
        first.rev
    );
}
