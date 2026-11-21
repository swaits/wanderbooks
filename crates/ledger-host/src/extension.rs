//! The Claude desktop extension for a company (plan P15 unit C1, 3.3).
//!
//! The Connect page's "Claude desktop app" row writes a one-click
//! extension package that makes the Claude desktop app run this program's
//! bridge (`wanderbooks mcp --book X`) for one company. The format is
//! Anthropic's MCP Bundle ("MCPB"): a zip archive, extension `.mcpb`,
//! holding a `manifest.json`. (The format was called "Desktop Extension",
//! `.dxt`, until it was renamed; the manifest spec's page says so.)
//!
//! ```text
//! GOAL: a person double-clicks one file, or picks it under Claude
//!       desktop's Settings > Extensions > Advanced settings > Install
//!       Extension, and Claude can keep that company's books, with no
//!       command typed and no file edited.
//! SOURCE: WB-pkg-5; Anthropic's MCPB manifest specification, version
//!         0.3 (github.com/anthropics/mcpb, MANIFEST.md and
//!         schemas/mcpb-manifest-v0.3.schema.json, read 2026-11-21) and
//!         Claude's help article "Getting started with Local MCP servers
//!         on Claude Desktop" (the install steps).
//! INVARIANTS:
//!   1. The manifest holds only fields the specification defines, with
//!      `manifest_version` "0.3", a `server` of type `binary` whose
//!      `mcp_config.command` is this program's own stable path (the one a
//!      terminal agent is registered with: `commands::bridge_serve_args`,
//!      which refuses a translocated app) and whose arguments are `mcp
//!      --book <id>` (and `--root` when the root was named). The package
//!      carries no program: the one executable is the installed one, so
//!      an update of the app is an update of the extension.
//!   2. No key, token, passphrase or path of the data root is in the
//!      package: the bridge reads its key from the token file the app
//!      keeps, as it does for a terminal agent (`ensure_agent_key` makes
//!      it, once).
//!   3. The package is written under `<Documents>/Wanderbooks/Extensions/`
//!      (a folder a person can open; not a book's home, which holds four
//!      files and nothing else), by a temporary file and a rename.
//!   4. The package is for the system it was made on (`platforms`).
//! SCENARIOS:
//!   1. IF the program is installed at a stable path: SHOULD write
//!      `wanderbooks-<id>.mcpb` that validates against the 0.3 schema.
//!   2. IF the program runs from a translocated macOS copy: SHOULD write
//!      nothing and say to move it first.
//!   3. IF the company's name holds quotes, markup or a very long text:
//!      SHOULD appear as text in the manifest, bounded.
//! ```
//!
//! Not run against a real Claude desktop app by the author (written on
//! Linux, with no Claude desktop at hand): the manifest is checked
//! against the published schema in `crates/ledger-cli/tests/extension.rs`;
//! that Claude accepts a `binary` extension whose command is an absolute
//! path to a program outside the package is the part to try first on a
//! real install.

use crate::commands::Common;
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};

/// The manifest specification this package follows.
pub const MANIFEST_VERSION: &str = "0.3";

/// The icon the package carries (the app's own, 512 px).
const ICON: &[u8] = include_bytes!("../../../deploy/packaging/icons/512x512.png");

/// The longest company name or description put in a manifest, in
/// characters.
const LONGEST: usize = 120;

/// What was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Made {
    /// The package.
    pub path: PathBuf,
    /// The folder it is in, for the person to open.
    pub folder: PathBuf,
}

/// `<documents>/Wanderbooks/Extensions`: where the packages go. A book's id
/// is lower case, so no company's books home can have this name.
pub fn extensions_dir_in(documents: &Path) -> PathBuf {
    documents.join("Wanderbooks").join("Extensions")
}

/// The file's name for a book: `wanderbooks-<id>.mcpb`.
pub fn file_name(book: &str) -> String {
    format!("wanderbooks-{book}.mcpb")
}

/// `process.platform`'s name for this system, as the manifest's
/// `compatibility.platforms` spells it.
pub fn platform_name() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    }
}

/// `text` cut to [`LONGEST`] characters, with control characters turned
/// into spaces: a name is text in the manifest and nothing else.
fn tidy(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(LONGEST)
        .collect::<String>()
        .trim()
        .to_string()
}

/// The manifest (INVARIANT 1) for `book`, kept by `company`, run by
/// `command` with `args`, made for `platform`.
pub fn manifest(
    book: &str,
    company: &str,
    version: &str,
    command: &str,
    args: &[String],
    platform: &str,
) -> Value {
    let company = match tidy(company) {
        name if name.is_empty() => book.to_string(),
        name => name,
    };
    json!({
        "manifest_version": MANIFEST_VERSION,
        "name": format!("wanderbooks-{book}"),
        "display_name": format!("Wanderbooks: {company}"),
        "version": version,
        "description": format!("Lets Claude keep the books of {company} in Wanderbooks."),
        "long_description": format!(
            "Connects the Claude desktop app to the books of {company}, which Wanderbooks keeps on this computer. Claude reads them, drafts entries and, when you let it, posts them. Wanderbooks must be open, with these books open in a tab, for Claude to reach them."
        ),
        "author": { "name": "Wanderbooks", "url": "https://wanderbooks.ai" },
        "homepage": "https://wanderbooks.ai",
        "icon": "icon.png",
        "license": "MIT OR Apache-2.0",
        "keywords": ["bookkeeping", "accounting", "wanderbooks"],
        "server": {
            "type": "binary",
            "entry_point": "server/README.txt",
            "mcp_config": { "command": command, "args": args }
        },
        "tools_generated": true,
        "compatibility": { "platforms": [platform] }
    })
}

/// What `server/README.txt` says (the manifest's entry point is a file in
/// the package; the program it names is the installed one).
const README: &str = "This extension carries no program of its own.\n\
It runs the Wanderbooks app you installed, as `wanderbooks mcp --book <id>`,\n\
so Claude can keep one company's books. Open Wanderbooks first, with those\n\
books open in a tab. If you move or reinstall Wanderbooks somewhere else,\n\
make this extension again from the app's Assistants page.\n";

/// The package's bytes: a zip of `manifest.json`, `icon.png` and
/// `server/README.txt` (INVARIANT 1).
pub fn package_bytes(manifest: &Value) -> Result<Vec<u8>, String> {
    use zip::write::SimpleFileOptions;
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let text = serde_json::to_vec_pretty(manifest).map_err(|e| e.to_string())?;
    let mut out = std::io::Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(&mut out);
    let mut add = |name: &str, bytes: &[u8]| -> Result<(), String> {
        zip.start_file(name, options)
            .map_err(|e| format!("can't make the package: {e}"))?;
        zip.write_all(bytes)
            .map_err(|e| format!("can't make the package: {e}"))
    };
    add("manifest.json", &text)?;
    add("icon.png", ICON)?;
    add("server/README.txt", README.as_bytes())?;
    zip.finish()
        .map_err(|e| format!("can't make the package: {e}"))?;
    Ok(out.into_inner())
}

/// Write the extension for `book` (company `company`) under `documents`,
/// for the program and arguments `bridge` (`program, args…`). The key is
/// the caller's to have made (INVARIANT 2).
pub fn write_in(
    documents: &Path,
    book: &str,
    company: &str,
    version: &str,
    bridge: &[std::ffi::OsString],
) -> Result<Made, String> {
    let book = ledger_store::root::BookId::parse(book)
        .map_err(|e| e.to_string())?
        .to_string();
    if !documents.is_absolute() {
        return Err("there is no documents folder to put the extension in".into());
    }
    let (command, args) = bridge
        .split_first()
        .ok_or("the bridge's command could not be worked out")?;
    let as_text = |part: &std::ffi::OsStr| {
        part.to_str()
            .map(str::to_string)
            .ok_or_else(|| "the program's path is not text Claude can be given".to_string())
    };
    let command = as_text(command)?;
    let args = args
        .iter()
        .map(|a| as_text(a))
        .collect::<Result<Vec<_>, _>>()?;
    let bytes = package_bytes(&manifest(
        &book,
        company,
        version,
        &command,
        &args,
        platform_name(),
    ))?;
    let folder = extensions_dir_in(documents);
    std::fs::create_dir_all(&folder)
        .map_err(|e| format!("can't make {}: {}", folder.display(), e.kind()))?;
    let path = folder.join(file_name(&book));
    let tmp = folder.join(format!(".{}.tmp{}", file_name(&book), std::process::id()));
    std::fs::write(&tmp, &bytes).map_err(|e| format!("can't write the package: {}", e.kind()))?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("can't write the package: {}", e.kind())
    })?;
    Ok(Made { path, folder })
}

/// The Claude desktop extension for `book`, in the person's Documents:
/// the agent key made if there is none, then the package (the words
/// above). What was made.
pub fn claude_desktop_extension(
    common: &Common,
    book: &str,
    company: &str,
) -> Result<Made, String> {
    let documents = crate::commands::documents_dir()
        .ok_or("there is no documents folder to put the extension in")?;
    // The stable path first: a translocated copy refuses here, before a
    // key is made or a file written.
    let bridge = crate::commands::bridge_serve_args(common, book)?;
    crate::commands::ensure_agent_key(common, book)?;
    write_in(&documents, book, company, crate::program_version(), &bridge)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bridge() -> Vec<std::ffi::OsString> {
        ["/opt/Wanderbooks/wanderbooks", "mcp", "--book", "acme"]
            .map(Into::into)
            .to_vec()
    }

    #[test]
    fn the_package_is_written_where_a_person_can_open_it() {
        let dir = tempfile::tempdir().unwrap();
        let made = write_in(dir.path(), "acme", "Acme LLC", "0.1.0", &bridge()).unwrap();
        assert_eq!(
            made.path,
            dir.path()
                .join("Wanderbooks/Extensions/wanderbooks-acme.mcpb")
        );
        assert_eq!(made.folder, dir.path().join("Wanderbooks/Extensions"));
        let file = std::fs::File::open(&made.path).unwrap();
        let mut zip = zip::ZipArchive::new(file).unwrap();
        let mut names: Vec<String> = (0..zip.len())
            .map(|i| zip.by_index(i).unwrap().name().unwrap().to_string())
            .collect();
        names.sort();
        assert_eq!(names, ["icon.png", "manifest.json", "server/README.txt"]);
        // Written again, it replaces itself; nothing is left beside it.
        write_in(dir.path(), "acme", "Acme LLC", "0.1.0", &bridge()).unwrap();
        let left: Vec<_> = std::fs::read_dir(&made.folder).unwrap().collect();
        assert_eq!(left.len(), 1);
    }

    #[test]
    fn a_name_is_text_and_bounded() {
        let long = "x".repeat(500);
        let m = manifest(
            "acme",
            &format!("A\"<b>\n{long}"),
            "1.0.0",
            "/p",
            &[],
            "linux",
        );
        let shown = m["display_name"].as_str().unwrap();
        assert!(
            shown.chars().count() <= "Wanderbooks: ".len() + LONGEST,
            "{shown}"
        );
        assert!(!shown.contains('\n'));
        let none = manifest("acme", "  ", "1.0.0", "/p", &[], "linux");
        assert_eq!(none["display_name"], "Wanderbooks: acme");
    }

    #[test]
    fn no_secret_and_no_data_root_is_in_the_package() {
        let dir = tempfile::tempdir().unwrap();
        let made = write_in(dir.path(), "acme", "Acme", "0.1.0", &bridge()).unwrap();
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&made.path).unwrap()).unwrap();
        let mut text = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("manifest.json").unwrap(), &mut text)
            .unwrap();
        for word in ["token", "passphrase", "secret", "password", "bearer"] {
            assert!(!text.to_lowercase().contains(word), "{word} in {text}");
        }
        assert!(text.contains("\"mcp\""), "{text}");
    }

    #[test]
    fn a_relative_documents_folder_is_refused() {
        let err = write_in(Path::new("Documents"), "acme", "Acme", "0.1.0", &bridge()).unwrap_err();
        assert!(err.contains("documents folder"), "{err}");
    }
}
