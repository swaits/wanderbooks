//! The Claude desktop extension the Connect page writes (plan P15 unit C1,
//! WB-pkg-5): its package is Anthropic's MCP Bundle, and its manifest is
//! checked against Anthropic's published schema for manifest version 0.3
//! (`tests/fixtures/mcpb/`, with where it came from).
//!
//! ```text
//! GOAL: a package the Claude desktop app can install, naming this
//!       program's bridge for one company, with nothing secret in it.
//! SOURCE: WB-pkg-5; github.com/anthropics/mcpb (MANIFEST.md, the 0.3
//!         schema); ledger-host extension.rs.
//! INVARIANTS:
//!   1. The package is a zip whose root holds `manifest.json`; every file
//!      the manifest names (`icon`, `server.entry_point`) is in the zip.
//!   2. The manifest validates against the 0.3 schema, which refuses any
//!      field the specification does not define.
//!   3. The server is a `binary` whose command is the program's path and
//!      whose arguments are `mcp --book <id>`.
//! SCENARIOS:
//!   1. A package made for a company with an awkward name validates.
//!   2. Every required field of the schema is present.
//! ```

use serde_json::Value;
use std::io::Read;

fn schema() -> jsonschema::Validator {
    let text = include_str!("fixtures/mcpb/mcpb-manifest-v0.3.schema.json");
    jsonschema::validator_for(&serde_json::from_str::<Value>(text).unwrap()).unwrap()
}

fn made(company: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let bridge: Vec<std::ffi::OsString> = [
        "/Applications/Wanderbooks.app/Contents/MacOS/wanderbooks",
        "mcp",
        "--book",
        "acme",
    ]
    .map(Into::into)
    .to_vec();
    let made = ledger_host::extension::write_in(dir.path(), "acme", company, "0.1.0", &bridge)
        .expect("the package is written");
    (dir, made.path)
}

fn read(path: &std::path::Path) -> (Value, Vec<String>) {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let names: Vec<String> = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().unwrap().to_string())
        .collect();
    let mut text = String::new();
    zip.by_name("manifest.json")
        .expect("manifest.json is at the root of the package")
        .read_to_string(&mut text)
        .unwrap();
    (serde_json::from_str(&text).unwrap(), names)
}

#[test]
fn the_manifest_is_one_the_published_schema_accepts() {
    for company in [
        "Wanderland Labs LLC",
        "Quote \" and <markup> & \u{e9}\u{4e2d}",
        &"long ".repeat(100),
        "",
    ] {
        let (_dir, path) = made(company);
        let (manifest, _) = read(&path);
        let errors: Vec<String> = schema()
            .iter_errors(&manifest)
            .map(|e| format!("{e} at {}", e.instance_path()))
            .collect();
        assert!(errors.is_empty(), "{company:?}: {errors:?}\n{manifest:#}");
        // The check is not vacuous: a field the specification lacks, or a
        // missing required one, is refused.
        let mut extra = manifest.clone();
        extra["no_such_field"] = Value::Bool(true);
        assert!(!schema().is_valid(&extra));
        let mut short = manifest.clone();
        short.as_object_mut().unwrap().remove("author");
        assert!(!schema().is_valid(&short));
    }
}

#[test]
fn every_file_the_manifest_names_is_in_the_package() {
    let (_dir, path) = made("Acme");
    let (manifest, names) = read(&path);
    assert_eq!(manifest["manifest_version"], "0.3");
    for named in [
        manifest["icon"].as_str().unwrap(),
        manifest["server"]["entry_point"].as_str().unwrap(),
    ] {
        assert!(names.iter().any(|n| n == named), "{named} in {names:?}");
    }
    // The schema's required fields, by the schema's own list.
    let schema: Value =
        serde_json::from_str(include_str!("fixtures/mcpb/mcpb-manifest-v0.3.schema.json")).unwrap();
    for field in schema["required"].as_array().unwrap() {
        assert!(manifest.get(field.as_str().unwrap()).is_some(), "{field}");
    }
    for field in schema["properties"]["server"]["required"]
        .as_array()
        .unwrap()
    {
        assert!(manifest["server"].get(field.as_str().unwrap()).is_some());
    }
}

#[test]
fn the_server_runs_this_programs_bridge_for_the_company() {
    let (_dir, path) = made("Acme");
    let (manifest, _) = read(&path);
    let server = &manifest["server"];
    assert_eq!(server["type"], "binary");
    assert_eq!(
        server["mcp_config"]["command"],
        "/Applications/Wanderbooks.app/Contents/MacOS/wanderbooks"
    );
    assert_eq!(
        server["mcp_config"]["args"],
        serde_json::json!(["mcp", "--book", "acme"])
    );
    // The package carries no program (the one executable is the installed
    // one) and no key.
    let (_, names) = read(&path);
    assert_eq!(names.len(), 3, "{names:?}");
    assert_eq!(manifest["name"], "wanderbooks-acme");
}

/// The 512 px icon is a PNG (the manifest's `icon` is "a png icon file").
#[test]
fn the_icon_is_a_png() {
    let (_dir, path) = made("Acme");
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut head = [0u8; 8];
    zip.by_name("icon.png")
        .unwrap()
        .read_exact(&mut head)
        .unwrap();
    assert_eq!(&head, b"\x89PNG\r\n\x1a\n");
}
