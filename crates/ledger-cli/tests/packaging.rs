//! The packages (plan P15 unit C1; WB-pkg-1 to WB-pkg-7): what can be checked
//! on Linux about the Windows installer, the macOS app, the Linux entries and
//! the one source of the version. For Windows and macOS the release workflow
//! is the real test (each packaging step fails loudly); these tests keep the
//! files it builds from true to the rules.
//!
//! ```text
//! GOAL: an installed Wanderbooks is one executable at a stable path, named
//!       Wanderbooks everywhere a person sees it, with the licences beside
//!       it; installing a newer one over it, or uninstalling it, never
//!       touches the person's books.
//! SOURCE: WB-pkg-1 (installs like a normal program), WB-pkg-2 (one
//!         executable, a stable bridge path), WB-pkg-6 (one version, one
//!         name), WB-pkg-7 (the release carries the licences); D-77.
//! INVARIANTS:
//!   1. The version is written once (ledger-cli's Cargo.toml); the packaging
//!      scripts read it through deploy/packaging/version.sh.
//!   2. The Windows installer is per user (no administrator), installs one
//!      program, and its uninstaller names nothing but what the installer
//!      put in its own folder: no recursive removal, no wildcard, no data
//!      folder.
//!   3. The macOS bundle holds one program at Contents/MacOS/wanderbooks, an
//!      Info.plist whose identifier, name, version and executable agree with
//!      the scripts, and the licences.
//!   4. The Linux desktop entries launch plain `wanderbooks` (an AppImage's
//!      launcher appends what it is given) with the icon, the name, and the
//!      window's app id.
//!   5. Every package carries LICENSE-MIT, LICENSE-APACHE and
//!      THIRD-PARTY-NOTICES.txt.
//! SCENARIOS:
//!   1. The bundle script run on Linux with a stand-in program makes a
//!      bundle whose plist and files match the rules above.
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(path: &str) -> String {
    std::fs::read_to_string(root().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The lines of `text` that are not comments (`;` for NSIS, `#` for shell
/// and TOML).
fn code(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with(';') && !l.starts_with('#'))
        .collect()
}

const LICENCE_FILES: [&str; 3] = ["LICENSE-MIT", "LICENSE-APACHE", "THIRD-PARTY-NOTICES.txt"];

// ---- the version, written once ---------------------------------------------------

#[cfg(unix)]
#[test]
fn the_version_is_written_once_and_every_script_reads_it_from_there() {
    let out = Command::new("sh")
        .arg(root().join("deploy/packaging/version.sh"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let said = String::from_utf8(out.stdout).unwrap();
    assert_eq!(said.trim(), env!("CARGO_PKG_VERSION"));

    // The Linux packager's own file carries a version the build script
    // overwrites; it must not drift from the source.
    let packager = read("deploy/packaging/packager.toml");
    let line = code(&packager)
        .into_iter()
        .find(|l| l.starts_with("version = "))
        .unwrap();
    assert_eq!(line, format!("version = \"{}\"", env!("CARGO_PKG_VERSION")));

    // No packaging script or workflow assigns a literal version.
    for file in [
        "deploy/packaging/windows/build-windows.sh",
        "deploy/packaging/macos/build-macos.sh",
        ".github/workflows/release.yml",
    ] {
        for line in code(&read(file)) {
            let assigns = line.starts_with("version=") || line.starts_with("version:");
            let literal = line.split(['=', ':']).nth(1).is_some_and(|v| {
                v.trim()
                    .trim_matches('"')
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_digit())
            });
            assert!(!(assigns && literal), "{file}: a literal version: {line}");
        }
    }
    for file in [
        "deploy/packaging/windows/build-windows.sh",
        "deploy/packaging/macos/build-macos.sh",
        "deploy/packaging/build-linux.sh",
    ] {
        let text = read(file);
        assert!(
            text.contains("version.sh") || text.contains("ledger-cli/Cargo.toml"),
            "{file} does not read the version from its one source"
        );
    }
}

#[cfg(unix)]
#[test]
fn a_version_that_is_not_three_numbers_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("deploy/packaging")).unwrap();
    std::fs::create_dir_all(dir.path().join("crates/ledger-cli")).unwrap();
    std::fs::copy(
        root().join("deploy/packaging/version.sh"),
        dir.path().join("deploy/packaging/version.sh"),
    )
    .unwrap();
    for (toml, ok) in [
        ("version = \"0.2.0\"\n", true),
        ("version = \"0.2.0-rc1\"\n", false),
        ("version = \"two\"\n", false),
        ("", false),
    ] {
        std::fs::write(dir.path().join("crates/ledger-cli/Cargo.toml"), toml).unwrap();
        let out = Command::new("sh")
            .arg(dir.path().join("deploy/packaging/version.sh"))
            .output()
            .unwrap();
        assert_eq!(out.status.success(), ok, "{toml:?}: {out:?}");
    }
}

// ---- Windows --------------------------------------------------------------------

#[test]
fn the_windows_installer_is_per_user_and_installs_one_program() {
    let nsi = read("deploy/packaging/windows/wanderbooks.nsi");
    let lines = code(&nsi);
    let has = |needle: &str| lines.iter().any(|l| l.contains(needle));
    // No administrator prompt; everything in the person's own profile.
    assert!(has("RequestExecutionLevel user"));
    assert!(has(r#"InstallDir "$LOCALAPPDATA\Programs\Wanderbooks""#));
    assert!(
        !lines.iter().any(|l| l.contains("HKLM")),
        "per-machine registry"
    );
    assert!(!has("SetShellVarContext all"), "all users' Start Menu");
    // A Start Menu entry named Wanderbooks, with the icon.
    assert!(has(
        r#"CreateShortCut "$SMPROGRAMS\${NAME}.lnk" "$INSTDIR\wanderbooks.exe""#
    ));
    assert!(has(r#"!define NAME "Wanderbooks""#));
    assert!(has("wanderbooks.ico"));
    // An uninstaller, listed in Settings > Apps, whose name is the product's.
    assert!(has("WriteUninstaller"));
    assert!(has(r#""DisplayName" "${NAME}""#));
    assert!(has("UninstallString"));
    // One program: the only .exe installed besides the uninstaller.
    let programs: Vec<&&str> = lines
        .iter()
        .filter(|l| l.starts_with("File ") && l.contains(".exe"))
        .collect();
    assert_eq!(programs.len(), 1, "{programs:?}");
    assert!(programs[0].contains("/oname=wanderbooks.exe"));
    // The licences travel with it.
    for file in LICENCE_FILES {
        assert!(has(&format!("/oname={file}")), "{file} is not installed");
    }
}

/// WB-pkg-1: the uninstaller removes what the installer put in its own
/// folder and nothing else; the books are in the data folder, which it never
/// names.
#[test]
fn the_uninstaller_never_removes_the_books() {
    let nsi = read("deploy/packaging/windows/wanderbooks.nsi");
    let lines = code(&nsi);
    let start = lines
        .iter()
        .position(|l| l.starts_with(r#"Section "Uninstall""#))
        .expect("an uninstall section");
    let section = &lines[start..];
    let mut removals = 0;
    for line in section {
        let word = line.split_whitespace().next().unwrap_or("");
        if matches!(word, "Delete" | "RMDir" | "DeleteRegKey" | "DeleteRegValue") {
            removals += 1;
            assert!(!line.contains("/r"), "a recursive removal: {line}");
            assert!(
                !line.contains('*') && !line.contains('?'),
                "a wildcard: {line}"
            );
            let ok = line.contains("$INSTDIR")
                || line.contains("$SMPROGRAMS")
                || line.contains("${UNINSTALL_KEY}")
                || line.contains("${APP_KEY}");
            assert!(
                ok,
                "removes something that is not the program's own: {line}"
            );
        }
    }
    assert!(removals >= 6, "the uninstaller removes the installed files");
    // The data folder, the roaming settings and the keys are named nowhere
    // in the script's code (the comments say where the books are).
    for line in &lines {
        for data in [
            "$LOCALAPPDATA\\wanderbooks",
            "$APPDATA",
            "\\link",
            "legacy",
            "books\\",
        ] {
            assert!(!line.contains(data), "{data} named: {line}");
        }
    }
    // Only the registry keys of its own.
    let keys: Vec<&&str> = lines
        .iter()
        .filter(|l| l.starts_with("!define") && l.contains("_KEY"))
        .collect();
    assert_eq!(keys.len(), 2, "{keys:?}");
    assert!(keys[0].contains(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Wanderbooks"));
    assert!(keys[1].contains(r"Software\Wanderbooks"));
}

/// If NSIS is installed here, the script compiles with stand-in files
/// (the release workflow compiles it for real on Windows).
#[test]
fn the_installer_script_compiles_when_nsis_is_here() {
    let Some(makensis) = std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p)
            .map(|d| d.join("makensis"))
            .find(|f| f.is_file())
    }) else {
        eprintln!(
            "makensis is not installed here: the Windows job of the release workflow compiles the installer"
        );
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    std::fs::write(p.join("wanderbooks.exe"), b"MZ").unwrap();
    for f in LICENCE_FILES {
        std::fs::write(p.join(f), b"x").unwrap();
    }
    std::fs::copy(
        root().join("deploy/packaging/icons/wanderbooks.ico"),
        p.join("wanderbooks.ico"),
    )
    .unwrap();
    let out = Command::new(makensis)
        .arg("-V2")
        .arg(format!("-DVERSION={}", env!("CARGO_PKG_VERSION")))
        .arg(format!("-DPROGRAM={}", p.join("wanderbooks.exe").display()))
        .arg(format!("-DICON={}", p.join("wanderbooks.ico").display()))
        .arg(format!("-DLICENSES={}", p.display()))
        .arg(format!("-DOUTFILE={}", p.join("setup.exe").display()))
        .arg(root().join("deploy/packaging/windows/wanderbooks.nsi"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(p.join("setup.exe").metadata().unwrap().len() > 0);
}

/// WB-pkg-2: on Windows the one executable is built for the GUI subsystem
/// (no console window for the app), and gives a terminal its words back.
#[test]
fn the_windows_program_opens_no_console_window_and_still_speaks_in_a_terminal() {
    let main = read("crates/ledger-cli/src/main.rs");
    assert!(
        main.contains(
            r#"#![cfg_attr(all(windows, feature = "gui"), windows_subsystem = "windows")]"#
        )
    );
    assert!(main.contains("console::attach_parent_console()"));
    let console = read("crates/ledger-cli/src/console.rs");
    assert!(console.contains("AttachConsole(ATTACH_PARENT_PROCESS)"));
    // The server child the app starts has no console window of its own.
    assert!(read("crates/ledger-host/src/app_server.rs").contains("NO_WINDOW"));
}

// ---- macOS ----------------------------------------------------------------------

/// The `<key>`/value pairs of a plist's top-level dict, as text. Good enough
/// for the plain plists here (no nesting).
fn plist(text: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut key: Option<String> = None;
    for line in text.lines().map(str::trim) {
        if let Some(k) = line
            .strip_prefix("<key>")
            .and_then(|l| l.strip_suffix("</key>"))
        {
            assert!(key.is_none(), "a key with no value: {k}");
            key = Some(k.to_string());
        } else if let Some(k) = key.take_if(|_| line.starts_with('<') && !line.starts_with("<!--"))
        {
            let value = line
                .strip_prefix("<string>")
                .and_then(|l| l.strip_suffix("</string>"))
                .map(str::to_string)
                .unwrap_or_else(|| line.to_string());
            pairs.push((k, value));
        }
    }
    assert!(key.is_none(), "a key with no value at the end");
    pairs
}

#[cfg(unix)]
#[test]
fn the_macos_bundle_has_one_program_and_a_plist_that_agrees_with_the_scripts() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let stand_in = dir.path().join("program");
    std::fs::write(&stand_in, b"not a real program").unwrap();
    let version = "0.0.7";
    let out = Command::new("bash")
        .arg(root().join("deploy/packaging/macos/build-macos.sh"))
        .arg(version)
        .env("WB_PROGRAM", &stand_in)
        .env("WB_NO_ICNS", "1")
        .env("WB_BUNDLE_ONLY", "1")
        .output()
        .unwrap();
    let app = root().join(format!(
        "target/packages/.stage-macos-{version}/Wanderbooks.app"
    ));
    let cleanup = Cleanup(root().join(format!("target/packages/.stage-macos-{version}")));
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let exe = app.join("Contents/MacOS/wanderbooks");
    assert!(exe.is_file());
    assert_ne!(
        exe.metadata().unwrap().permissions().mode() & 0o111,
        0,
        "not executable"
    );
    let programs: Vec<_> = std::fs::read_dir(app.join("Contents/MacOS"))
        .unwrap()
        .collect();
    assert_eq!(programs.len(), 1, "one program in the bundle");
    for file in LICENCE_FILES {
        assert!(
            app.join("Contents/Resources").join(file).is_file(),
            "{file}"
        );
    }

    let info = std::fs::read_to_string(app.join("Contents/Info.plist")).unwrap();
    assert!(!info.contains('@'), "an unfilled placeholder: {info}");
    let pairs = plist(&info);
    let get = |k: &str| {
        pairs
            .iter()
            .find(|(key, _)| key == k)
            .unwrap_or_else(|| panic!("{k} is missing from Info.plist"))
            .1
            .clone()
    };
    assert_eq!(get("CFBundleIdentifier"), "ai.wanderbooks.app");
    assert_eq!(get("CFBundleName"), "Wanderbooks");
    assert_eq!(get("CFBundleDisplayName"), "Wanderbooks");
    assert_eq!(get("CFBundleExecutable"), "wanderbooks");
    assert_eq!(get("CFBundleShortVersionString"), version);
    assert_eq!(get("CFBundleVersion"), version);
    assert_eq!(get("CFBundlePackageType"), "APPL");
    assert_eq!(get("LSMinimumSystemVersion"), "11.0");
    assert_eq!(get("LSMultipleInstancesProhibited"), "<true/>");
    assert!(get("NSDocumentsFolderUsageDescription").contains("Documents"));
    drop(cleanup);
}

/// Removes a folder when dropped (a test's own scratch under `target/`).
struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn the_entitlements_are_empty_on_purpose_and_say_why() {
    let text = read("deploy/packaging/macos/entitlements.plist");
    assert!(
        code(&text).contains(&"<dict/>"),
        "no entitlement is claimed without a reason"
    );
    assert!(text.contains("hardened runtime"));
    assert!(text.contains("NOT sandboxed"));
}

#[test]
fn the_macos_script_signs_only_with_an_identity_and_names_no_secret() {
    let script = read("deploy/packaging/macos/build-macos.sh");
    // Secrets come from the environment, by name; none is written here.
    for name in [
        "WB_SIGN_IDENTITY",
        "WB_NOTARY_APPLE_ID",
        "WB_NOTARY_TEAM_ID",
        "WB_NOTARY_PASSWORD",
    ] {
        assert!(script.contains(name), "{name}");
    }
    assert!(script.contains("--options runtime"), "the hardened runtime");
    assert!(script.contains("entitlements.plist"));
    assert!(script.contains("notarytool submit"));
    assert!(script.contains("stapler staple"));
    assert!(script.contains("lipo -create"), "universal");
    for line in code(&script) {
        assert!(
            !line.contains("password=") || line.contains("$("),
            "a literal password: {line}"
        );
    }
}

// ---- Linux ----------------------------------------------------------------------

/// The `Key=Value` lines of a desktop entry.
fn desktop(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn the_desktop_entries_launch_plain_wanderbooks_with_the_icon_and_the_window_id() {
    // The id the window announces, which the launcher matches (Wayland, and
    // X11's WM class).
    let gui = read("crates/ledger-gui/src/lib.rs");
    assert!(gui.contains(r#".with_app_id("wanderbooks")"#));
    for file in [
        "deploy/packaging/wanderbooks-static.desktop",
        "deploy/packaging/wanderbooks.desktop",
    ] {
        let entries = desktop(&read(file));
        let get = |k: &str| {
            entries
                .iter()
                .find(|(key, _)| key == k)
                .unwrap_or_else(|| panic!("{file}: {k} is missing"))
                .1
                .clone()
        };
        assert_eq!(get("Type"), "Application", "{file}");
        assert_eq!(get("Terminal"), "false", "{file}");
        assert_eq!(get("StartupWMClass"), "wanderbooks", "{file}");
        let categories = get("Categories");
        assert!(
            categories.starts_with("{{") || categories.ends_with(';'),
            "{file}: {categories}"
        );
        // An AppImage's launcher appends what it is given to the Exec line:
        // it names the program and nothing else.
        let exec = get("Exec");
        assert!(
            exec == "wanderbooks" || exec == "{{exec}}",
            "{file}: {exec}"
        );
        let name = get("Name");
        assert!(
            name == "Wanderbooks" || name == "{{name}}",
            "{file}: {name}"
        );
        let icon = get("Icon");
        assert!(
            icon == "wanderbooks" || icon == "{{icon}}",
            "{file}: {icon}"
        );
    }
    // The static entry (the .rpm's) is a valid desktop file.
    if let Some(tool) = std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p)
            .map(|d| d.join("desktop-file-validate"))
            .find(|f| f.is_file())
    }) {
        let out = Command::new(tool)
            .arg(root().join("deploy/packaging/wanderbooks-static.desktop"))
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
    }
}

/// WB-pkg-7: every package carries the licences, and the Linux packages
/// install the icon their desktop entry names.
#[test]
fn every_package_carries_the_licences_and_the_icon() {
    let deb_pacman = read("deploy/packaging/packager.toml");
    let rpm = read("crates/ledger-cli/Cargo.toml");
    let rpm_script = read("deploy/packaging/build-linux.sh");
    let windows = read("deploy/packaging/windows/wanderbooks.nsi");
    let mac = read("deploy/packaging/macos/build-macos.sh");
    for file in LICENCE_FILES {
        assert!(
            deb_pacman.contains(&format!("\"{file}\" = ")),
            "deb or pacman lacks {file}"
        );
        assert_eq!(
            deb_pacman.matches(&format!("\"{file}\" = ")).count(),
            3,
            "the deb, the AppImage and the pacman package each carry {file}"
        );
        assert!(
            rpm.contains(&format!("source = \"{file}\"")),
            "rpm lacks {file}"
        );
        assert!(
            rpm_script.contains(&format!("source = \"{file}\"")),
            "rpm script lacks {file}"
        );
        assert!(
            windows.contains(&format!("/oname={file}")),
            "windows lacks {file}"
        );
        assert!(mac.contains(file), "macos lacks {file}");
    }
    assert!(rpm.contains("512x512/apps/wanderbooks.png"));
    assert!(rpm_script.contains("512x512/apps/wanderbooks.png"));
    for file in LICENCE_FILES {
        assert!(
            root().join(file).is_file(),
            "{file} is not in the repository"
        );
    }
}

#[test]
fn no_package_runs_a_script_that_could_touch_the_books() {
    let packager = read("deploy/packaging/packager.toml");
    for key in [
        "postrm",
        "post-remove",
        "before-remove",
        "preinst",
        "postinst",
        "after-install",
        "scripts",
    ] {
        assert!(!packager.contains(key), "{key} in packager.toml");
    }
    let rpm = read("crates/ledger-cli/Cargo.toml");
    for key in [
        "post_uninstall",
        "pre_uninstall",
        "post_install_script",
        "pre_install_script",
    ] {
        assert!(!rpm.contains(key), "{key} in Cargo.toml");
    }
}

// ---- the release workflow -------------------------------------------------------

/// WB-pkg-7: the release builds all three packages, ends in a DRAFT with the
/// SHA-256 sums, runs only on a version tag or by hand, and publishes
/// nothing by itself.
#[test]
fn the_release_workflow_makes_a_draft_of_all_three_packages_and_publishes_nothing() {
    let text = read(".github/workflows/release.yml");
    let lines = code(&text);
    let has = |needle: &str| lines.iter().any(|l| l.contains(needle));

    // Triggers: a tag or by hand; never an ordinary push or a pull request.
    let on = lines
        .iter()
        .position(|l| *l == "on:")
        .expect("an `on:` block");
    let jobs = lines.iter().position(|l| *l == "jobs:").expect("jobs");
    let triggers: Vec<&&str> = lines[on + 1..jobs].iter().collect();
    assert!(
        triggers.iter().any(|l| l.starts_with("tags:")),
        "{triggers:?}"
    );
    assert!(triggers.iter().any(|l| l.starts_with("workflow_dispatch:")));
    assert!(
        !triggers
            .iter()
            .any(|l| l.starts_with("pull_request") || l.starts_with("schedule"))
    );
    let push = triggers
        .iter()
        .position(|l| l.starts_with("push:"))
        .unwrap();
    assert!(
        triggers[push + 1].starts_with("tags:"),
        "a push trigger without tags"
    );

    // All three systems, each tested on its own system before packaging.
    for runner in ["ubuntu-24.04", "windows-latest", "macos-latest"] {
        assert!(has(&format!("runs-on: {runner}")), "{runner}");
    }
    assert_eq!(
        lines
            .iter()
            .filter(|l| l.contains("cargo test --workspace --no-fail-fast"))
            .count(),
        3,
        "each system runs the workspace's tests"
    );
    // The three packages, the sums, the licences and the update note.
    for needle in [
        "just package-linux",
        "build-windows.sh",
        "build-macos.sh",
        "sha256sum",
        "SHA256SUMS.txt",
        "THIRD-PARTY-NOTICES.txt",
        "LICENSE-MIT",
        "LICENSE-APACHE",
        "latest.json",
        "scripts/notices.sh --check",
    ] {
        assert!(has(needle), "{needle} is not in the release workflow");
    }
    // A draft, never published, never a published release touched.
    assert!(has("gh release create"));
    assert!(has("--draft"));
    for never in [
        "--draft=false",
        "gh release edit",
        "gh-pages",
        "prerelease=false",
        "make_latest",
    ] {
        assert!(!has(never), "{never} would publish");
    }
    assert!(has("never changes a published release"));
    // Secrets are read by name from the secrets store, never written here.
    for secret in [
        "WINDOWS_CERTIFICATE_PFX_BASE64",
        "APPLE_CERTIFICATE_P12_BASE64",
        "APPLE_SIGNING_IDENTITY",
        "APPLE_APP_SPECIFIC_PASSWORD",
    ] {
        assert!(has(&format!("${{{{ secrets.{secret} }}}}")), "{secret}");
    }
    // Signing steps say so when they are skipped.
    assert!(has("macOS signing skipped"));
    assert!(read("deploy/packaging/windows/sign-windows.ps1").contains("Windows signing skipped"));
    assert!(read("deploy/packaging/macos/build-macos.sh").contains("macOS notarization skipped"));
    // The Windows job installs and uninstalls the installer, and checks the
    // books' folder survives.
    assert!(has("THE UNINSTALLER REMOVED THE BOOKS' FOLDER"));
    assert!(has("subsystem -ne 2"));
}

#[test]
fn only_a_person_runs_the_workflow_that_publishes_the_repositories() {
    let text = read(".github/workflows/publish-repos.yml");
    let lines = code(&text);
    let on = lines.iter().position(|l| *l == "on:").unwrap();
    let jobs = lines.iter().position(|l| *l == "jobs:").unwrap();
    for l in &lines[on + 1..jobs] {
        assert!(
            !(l.starts_with("push:") || l.starts_with("pull_request") || l.starts_with("schedule")),
            "{l}: only workflow_dispatch may start it"
        );
    }
    assert!(lines.iter().any(|l| l.starts_with("workflow_dispatch:")));
    assert!(
        lines.iter().any(|l| l.contains("isDraft")),
        "it refuses a draft"
    );
}

/// WB-pkg-4: a backup is plain files the program writes itself, so no
/// package depends on another program for it and none carries one.
#[test]
fn no_package_depends_on_another_program_for_backups() {
    let word = |text: &str| {
        text.to_lowercase()
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|w| w == "git")
    };
    for file in [
        "deploy/packaging/packager.toml",
        "deploy/packaging/aur/PKGBUILD",
        "crates/ledger-cli/Cargo.toml",
        "deploy/packaging/windows/wanderbooks.nsi",
        "deploy/packaging/macos/build-macos.sh",
    ] {
        let text = read(file);
        // (Cargo.toml may name a git *dependency* of the build, never a
        // package requirement.)
        let checked: String = text
            .lines()
            .filter(|l| !l.contains("git ="))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!word(&checked), "{file} depends on, or bundles, a git");
    }
}
