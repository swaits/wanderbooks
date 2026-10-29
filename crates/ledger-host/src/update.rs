//! `wanderbooks update` and `wanderbooks rollback` (plan P11a step 2,
//! 3.9, D-24, D-26).
//!
//! The owner's way to a newer version of the program, for an install that
//! updates itself (an AppImage, or the executable put somewhere by hand).
//! A package-managed install refuses and says to use the package manager.
//!
//! ```text
//! GOAL: a newer version replaces the running program only if it is the
//!       one we released, whole, and the books never see a half-written
//!       or unverified program.
//! SOURCE: plan/served.md 3.9 (Restarting; Format changes are the
//!         server's gate, not the updater's), D-24 (every self-update is
//!         verified against our own release keys), D-26 (the feed is a
//!         manifest at a fixed address).
//! INVARIANTS:
//!   1. The manifest names a version, and for this platform a file's
//!      address, its sha256 and its minisign signature. An update goes
//!      ahead only if the version is higher than this one's, the bytes
//!      downloaded have that sha256 AND verify against the release key
//!      this program carries; anything else is refused before a byte is
//!      written (nothing staged, nothing swapped).
//!   2. The program file is replaced by a rename of a complete file made
//!      in its own folder: a reader sees the old file or the new, never a
//!      part.
//!   3. The version replaced is kept under `<root>/updates/previous/` with
//!      the journal format it understood, until a rollback uses it or the
//!      next update replaces it.
//!   4. The new program must answer `--help` once swapped in; if not, the
//!      old one is put back and the update is an error.
//!   5. The running books are never told to stop: the host notices its
//!      file was replaced and restarts as it does for a package upgrade
//!      (host.rs INVARIANT 7); calls in progress finish first.
//!   6. A rollback refuses while any book declares a journal format or
//!      feature the kept version doesn't know (the owner consented to a
//!      newer one: the only way back is a restore, plan 3.9).
//!   7. A package-managed install is never touched.
//!   8. A program with no release key compiled in updates from nothing
//!      but a local (`file://`) feed whose key the owner names.
//! SCENARIOS:
//!   1. IF the feed offers a higher version, signed by our key: SHOULD
//!      replace the program and keep the old one.
//!   2. IF the file's sha256 differs from the manifest's, or its signature
//!      is by another key or missing: SHOULD refuse, changing nothing.
//!   3. IF the new program can't start: SHOULD put the old one back.
//!   4. IF a book consented to a newer format: SHOULD refuse to roll back.
//! ```

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

type Failure = Box<dyn std::error::Error>;

/// Where the release manifest is (D-26): the fixed address GitHub
/// Releases serves the newest release's files at.
pub const DEFAULT_FEED: &str =
    "https://github.com/swaits/wanderbooks/releases/latest/download/manifest.json";

/// The release public key (minisign, base64) this program trusts, put in
/// at build time by the release workflow (P11a step 5). A build without
/// one trusts nothing.
const RELEASE_KEY: Option<&str> = option_env!("WANDERBOOKS_RELEASE_PUBKEY");

/// The updates folder in the data root, and what it holds.
const UPDATES: &str = "updates";
const PREVIOUS: &str = "previous";
const PROGRAM: &str = "wanderbooks";
const META: &str = "meta.json";

const MAX_MANIFEST: usize = 1 << 20;
const MAX_PROGRAM: u64 = 1 << 30;
/// How long the new program has to answer `--help`.
const HEALTH_WAIT: Duration = Duration::from_secs(30);

/// The release manifest (the shape `cargo-packager-updater` reads: a
/// version and a file per platform, its signature the base64 of the
/// `.sig` file), with the file's sha256 added.
#[derive(Debug, Deserialize)]
struct Manifest {
    version: String,
    #[serde(default)]
    notes: String,
    platforms: BTreeMap<String, Entry>,
}

#[derive(Debug, Deserialize)]
struct Entry {
    /// The file's address: absolute, or relative to the manifest's.
    url: String,
    sha256: String,
    /// The minisign signature file's text, or its base64.
    signature: String,
}

/// How this program was installed.
#[derive(Debug, PartialEq, Eq)]
pub enum Install {
    /// A file this program may replace: the AppImage, or the executable.
    SelfUpdating(PathBuf),
    /// A package manager owns the file (`deb`, `rpm`, `aur`, ...).
    PackageManaged(String),
}

/// What kind of install the executable at `exe` is. A package puts
/// `<prefix>/share/wanderbooks/install-method` beside `<prefix>/bin`, with
/// its manager's name in it; an AppImage says where it is in `APPIMAGE`.
pub fn install_kind(exe: &Path, appimage: Option<&Path>) -> Install {
    if let Some(image) = appimage.filter(|p| p.is_file()) {
        return Install::SelfUpdating(image.to_path_buf());
    }
    if let Some(prefix) = exe.parent().and_then(Path::parent)
        && let Ok(text) = std::fs::read_to_string(prefix.join("share/wanderbooks/install-method"))
    {
        let manager = text.split_whitespace().next().unwrap_or("package");
        return Install::PackageManaged(manager.to_string());
    }
    Install::SelfUpdating(exe.to_path_buf())
}

fn this_install() -> Result<Install, String> {
    let exe = crate::start::this_executable()?;
    let image = std::env::var_os("APPIMAGE").map(PathBuf::from);
    Ok(install_kind(&exe, image.as_deref()))
}

fn refuse_package_managed(manager: &str, what: &str) -> Failure {
    format!(
        "this install is managed by {manager}, which also {what} it: use the package manager (our apt and dnf repositories keep every release, and the AUR keeps old PKGBUILDs)"
    )
    .into()
}

/// `1.2.3` as numbers; anything else (a pre-release, a `v`) is no version
/// of ours.
pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.split('.');
    let mut next = || -> Option<u64> {
        let p = parts.next()?;
        (!p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
            .then(|| p.parse().ok())
            .flatten()
    };
    let v = (next()?, next()?, next()?);
    parts.next().is_none().then_some(v)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// The flags of `update`.
#[derive(Default)]
struct Flags {
    root: Option<PathBuf>,
    feed: Option<String>,
    key_file: Option<PathBuf>,
    check: bool,
}

fn parse_flags(
    prog: &str,
    args: impl IntoIterator<Item = String>,
    update: bool,
) -> Result<Option<Flags>, Failure> {
    let mut flags = Flags::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => {
                flags.root = Some(PathBuf::from(args.next().ok_or("--root needs a folder")?))
            }
            "--feed" if update => flags.feed = Some(args.next().ok_or("--feed needs an address")?),
            "--trusted-key" if update => {
                flags.key_file = Some(PathBuf::from(
                    args.next().ok_or("--trusted-key needs a file")?,
                ))
            }
            "--check" if update => flags.check = true,
            "--help" | "-h" => {
                if update {
                    eprintln!("usage: {prog} [--check] [--feed ADDRESS [--trusted-key FILE]]");
                    eprintln!(
                        "  Brings this program to the newest release: downloads it, checks it"
                    );
                    eprintln!(
                        "  against our release key, keeps the version it replaces, and swaps"
                    );
                    eprintln!("  it in; the books restart onto it when idle. --check only looks.");
                    eprintln!(
                        "  --feed reads a manifest from elsewhere (a file:// folder, to update"
                    );
                    eprintln!(
                        "  from a local release); only then may --trusted-key name the minisign"
                    );
                    eprintln!("  public key to check against.");
                } else {
                    eprintln!("usage: {prog}");
                    eprintln!("  Goes back to the version the last update replaced, unless a book");
                    eprintln!("  has consented to a newer journal format that version can't open.");
                }
                return Ok(None);
            }
            other => {
                eprintln!("unknown argument: {other}");
                eprintln!(
                    "usage: {prog} {}",
                    if update {
                        "[--check] [--feed ADDRESS [--trusted-key FILE]]"
                    } else {
                        ""
                    }
                );
                return Err(Box::new(crate::Exit(2)));
            }
        }
    }
    Ok(Some(flags))
}

/// `wanderbooks update`: bring this program to the newest release.
/// `current` is this program's version.
pub fn update(
    prog: &str,
    current: &str,
    args: impl IntoIterator<Item = String>,
) -> Result<(), Failure> {
    let Some(flags) = parse_flags(prog, args, true)? else {
        return Ok(());
    };
    let target = match this_install()? {
        Install::PackageManaged(manager) => {
            return Err(refuse_package_managed(&manager, "updates"));
        }
        Install::SelfUpdating(target) => target,
    };
    let current_v = parse_version(current).ok_or("this program's version isn't one of ours")?;
    let feed = flags.feed.as_deref().unwrap_or(DEFAULT_FEED);
    let key = trusted_key(feed, flags.key_file.as_deref())?;
    let manifest_bytes = fetch(feed, MAX_MANIFEST as u64)?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|e| format!("the release manifest isn't one we can read: {e}"))?;
    let offered = parse_version(&manifest.version).ok_or_else(|| {
        format!(
            "the manifest's version {:?} isn't one of ours",
            manifest.version
        )
    })?;
    if offered == current_v {
        println!("wanderbooks {current} is the newest release.");
        return Ok(());
    }
    if offered < current_v {
        return Err(format!(
            "the feed offers {}, older than this {current}: not installing it (to go back, `wanderbooks rollback`)",
            manifest.version
        )
        .into());
    }
    let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    let entry = manifest
        .platforms
        .get(&platform)
        .ok_or_else(|| format!("release {} has no build for {platform}", manifest.version))?;
    if flags.check {
        println!(
            "wanderbooks {} is available (this is {current}). Run `{}` to install it.",
            manifest.version,
            prog.trim_end_matches(" --check")
        );
        if !manifest.notes.trim().is_empty() {
            println!("{}", manifest.notes.trim());
        }
        return Ok(());
    }
    let root = ledger_store::root::Root::find(flags.root.as_deref()).map_err(|e| e.to_string())?;
    let updates = updates_dir(&root)?;
    let _lock = take_lock(&updates)?;

    let bytes = fetch(&resolve(feed, &entry.url)?, MAX_PROGRAM)?;
    verify(&bytes, entry, &key)?;

    // Verified: from here the files are ours to write. Staged, then the
    // old one kept, then swapped in.
    let staged_dir = updates.join(&manifest.version);
    private_dir(&staged_dir)?;
    let staged = staged_dir.join(PROGRAM);
    write_program(&staged, &bytes)?;
    keep_previous(&updates, &target, current)?;
    swap_in(&target, &staged)?;
    if let Err(why) = healthy(&target) {
        // INVARIANT 4: put the old one back.
        let old = updates.join(PREVIOUS).join(PROGRAM);
        let back = swap_in(&target, &old);
        let _ = std::fs::remove_dir_all(&staged_dir);
        return Err(match back {
            Ok(()) => format!(
                "the new version didn't start ({why}); the old one is back and nothing changed"
            ),
            Err(e) => format!(
                "the new version didn't start ({why}) and the old one couldn't be put back ({e}); it is kept in the updates folder"
            ),
        }
        .into());
    }
    let _ = std::fs::remove_dir_all(&staged_dir);
    println!(
        "Updated wanderbooks from {current} to {}.",
        manifest.version
    );
    println!(
        "The books that are open restart onto it by themselves when they are idle; nothing in progress is cut off. `wanderbooks rollback` goes back."
    );
    Ok(())
}

/// `wanderbooks rollback`: go back to the version the last update
/// replaced.
pub fn rollback(prog: &str, args: impl IntoIterator<Item = String>) -> Result<(), Failure> {
    let Some(flags) = parse_flags(prog, args, false)? else {
        return Ok(());
    };
    let target = match this_install()? {
        Install::PackageManaged(manager) => {
            return Err(refuse_package_managed(&manager, "downgrades"));
        }
        Install::SelfUpdating(target) => target,
    };
    let root = ledger_store::root::Root::find(flags.root.as_deref()).map_err(|e| e.to_string())?;
    let updates = updates_dir(&root)?;
    let _lock = take_lock(&updates)?;
    let previous = updates.join(PREVIOUS);
    let meta: serde_json::Value = std::fs::read(previous.join(META))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or("there is no earlier version kept to go back to")?;
    if !previous.join(PROGRAM).is_file() {
        return Err("there is no earlier version kept to go back to".into());
    }
    let version = meta["version"].as_str().unwrap_or("the earlier version");
    let known = ledger_store::format::JournalFormat {
        format: meta["format"].as_u64().unwrap_or(0) as u32,
        features: meta["features"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|f| f.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
    };
    refuse_if_consented(&root, version, &known)?;
    // The newer program is set aside while the older one is tried.
    let aside = updates.join("rolling-back");
    let newer = std::fs::read(&target)
        .map_err(|e| format!("can't read this program to set it aside: {}", e.kind()))?;
    write_program(&aside, &newer)?;
    drop(newer);
    swap_in(&target, &previous.join(PROGRAM))?;
    if let Err(why) = healthy(&target) {
        let back = swap_in(&target, &aside);
        let _ = std::fs::remove_file(&aside);
        return Err(match back {
            Ok(()) => format!("version {version} didn't start ({why}); the newer one is back and nothing changed"),
            Err(e) => format!("version {version} didn't start ({why}) and the newer one couldn't be put back ({e})"),
        }
        .into());
    }
    let _ = std::fs::remove_file(&aside);
    let _ = std::fs::remove_dir_all(&previous);
    println!("Went back to wanderbooks {version}.");
    println!("The books that are open restart onto it by themselves when they are idle.");
    Ok(())
}

/// INVARIANT 6: refuse while a book declares more than `known`.
fn refuse_if_consented(
    root: &ledger_store::root::Root,
    version: &str,
    known: &ledger_store::format::JournalFormat,
) -> Result<(), Failure> {
    for id in root.list_books().map_err(|e| e.to_string())? {
        let journal = root.book_dir(&id).join(ledger_store::JOURNAL_FILE);
        let declared = match ledger_store::journal::read_journal_and_format(&journal) {
            Ok((_, declared)) => declared,
            Err(e) => {
                return Err(format!(
                    "can't tell what format the books {id} are in ({e}), so not going back"
                )
                .into());
            }
        };
        let newer_format = declared.format > known.format;
        let newer: Vec<&String> = declared
            .features
            .iter()
            .filter(|f| !known.features.contains(f))
            .collect();
        if newer_format || !newer.is_empty() {
            return Err(format!(
                "the books {id} were moved to a newer journal format, which version {version} can't open: going back would leave them unreadable. The only way back is restoring the backup made before the upgrade (it says how many entries would be lost)"
            )
            .into());
        }
    }
    Ok(())
}

/// The key to check against: the one compiled in, or, for a `file://`
/// feed only, the one the owner names (INVARIANT 8).
fn trusted_key(feed: &str, key_file: Option<&Path>) -> Result<minisign_verify::PublicKey, Failure> {
    let text = match (key_file, RELEASE_KEY) {
        (Some(file), _) => {
            if !feed.starts_with("file://") {
                return Err("--trusted-key goes with a file:// feed only: the key a release is checked against is the one built into this program".into());
            }
            std::fs::read_to_string(file).map_err(|e| format!("can't read the key: {e}"))?
        }
        (None, Some(key)) => key.to_string(),
        (None, None) => {
            return Err("this build carries no release key, so it can't check an update: install a release build, or update from a local release with --feed file://… --trusted-key FILE".into());
        }
    };
    parse_public_key(&text)
        .map_err(|e| format!("the release key isn't a minisign public key: {e}").into())
}

/// A minisign public key as `minisign -G` writes it (two lines), as
/// `cargo packager signer generate` writes it (those lines in base64), or
/// as the bare key line.
pub fn parse_public_key(text: &str) -> Result<minisign_verify::PublicKey, minisign_verify::Error> {
    use base64::Engine;
    let text = text.trim();
    if text.starts_with("untrusted comment:") {
        return minisign_verify::PublicKey::decode(text);
    }
    if let Ok(raw) = base64::engine::general_purpose::STANDARD.decode(text)
        && let Ok(inner) = String::from_utf8(raw)
        && inner.starts_with("untrusted comment:")
    {
        return minisign_verify::PublicKey::decode(&inner);
    }
    minisign_verify::PublicKey::from_base64(text)
}

/// INVARIANT 1: the sha256 AND the signature.
fn verify(bytes: &[u8], entry: &Entry, key: &minisign_verify::PublicKey) -> Result<(), Failure> {
    let have = sha256_hex(bytes);
    if !have.eq_ignore_ascii_case(entry.sha256.trim()) {
        return Err(
            "the download doesn't match the checksum in the manifest: refused, nothing changed"
                .into(),
        );
    }
    let text = entry.signature.trim();
    let text = if text.starts_with("untrusted comment:") {
        text.to_string()
    } else {
        use base64::Engine;
        let raw = base64::engine::general_purpose::STANDARD
            .decode(text)
            .map_err(
                |_| "the manifest's signature isn't a minisign signature: refused, nothing changed",
            )?;
        String::from_utf8(raw).map_err(|_| "the manifest's signature isn't text")?
    };
    let signature = minisign_verify::Signature::decode(&text).map_err(|e| {
        format!(
            "the manifest's signature isn't a minisign signature ({e}): refused, nothing changed"
        )
    })?;
    key.verify(bytes, &signature, false).map_err(|e| {
        Box::<dyn std::error::Error>::from(format!(
            "the download isn't signed by our release key ({e}): refused, nothing changed"
        ))
    })
}

/// The address of `url` as the manifest at `feed` means it.
fn resolve(feed: &str, url: &str) -> Result<String, Failure> {
    if url.contains("://") {
        return Ok(url.to_string());
    }
    if let Some(path) = feed.strip_prefix("file://") {
        let dir = Path::new(path).parent().unwrap_or(Path::new("/"));
        return Ok(format!("file://{}", dir.join(url).display()));
    }
    let base = reqwest::Url::parse(feed).map_err(|e| format!("the feed address is bad: {e}"))?;
    Ok(base
        .join(url)
        .map_err(|e| format!("the manifest's file address is bad: {e}"))?
        .to_string())
}

/// The bytes at `address` (`file://` or `https://`), at most `max`.
fn fetch(address: &str, max: u64) -> Result<Vec<u8>, Failure> {
    if let Some(path) = address.strip_prefix("file://") {
        let file = std::fs::File::open(path).map_err(|e| format!("can't read {address}: {e}"))?;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut std::io::Read::take(file, max + 1), &mut bytes)
            .map_err(|e| format!("can't read {address}: {e}"))?;
        if bytes.len() as u64 > max {
            return Err(format!("{address} is larger than a release can be").into());
        }
        return Ok(bytes);
    }
    if !address.starts_with("https://") {
        return Err(format!("{address}: only https:// and file:// feeds are read").into());
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async {
        let mut response = reqwest::Client::new()
            .get(address)
            .timeout(Duration::from_secs(600))
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("can't fetch {address}: {}", e.without_url()))?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| format!("can't fetch {address}: {}", e.without_url()))?
        {
            bytes.extend_from_slice(&chunk);
            if bytes.len() as u64 > max {
                return Err(format!("{address} is larger than a release can be"));
            }
        }
        Ok(bytes)
    })
    .map_err(Into::into)
}

/// `<root>/updates`, made 0700 if missing.
fn updates_dir(root: &ledger_store::root::Root) -> Result<PathBuf, Failure> {
    let dir = root.path().join(UPDATES);
    private_dir(&dir)?;
    Ok(dir)
}

fn private_dir(dir: &Path) -> Result<(), Failure> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .map_err(|e| format!("can't make the updates folder: {}", e.kind()))?;
    Ok(())
}

/// One update or rollback at a time.
fn take_lock(updates: &Path) -> Result<std::fs::File, Failure> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(updates.join("update.lock"))
        .map_err(|e| format!("can't open the update lock: {}", e.kind()))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => {
            Err("another update or rollback is running".into())
        }
        Err(std::fs::TryLockError::Error(e)) => {
            Err(format!("can't take the update lock: {}", e.kind()).into())
        }
    }
}

/// Write a program file whole and executable, readable by its owner only
/// until the swap.
fn write_program(path: &Path, bytes: &[u8]) -> Result<(), Failure> {
    let partial = path.with_extension("partial");
    let _ = std::fs::remove_file(&partial);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o700)
        .open(&partial)
        .map_err(|e| format!("can't stage the update: {}", e.kind()))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| format!("can't stage the update: {}", e.kind()))?;
    drop(file);
    std::fs::rename(&partial, path).map_err(|e| format!("can't stage the update: {}", e.kind()))?;
    Ok(())
}

/// INVARIANT 3: the program about to be replaced, with what it understood
/// of the journal.
fn keep_previous(updates: &Path, target: &Path, version: &str) -> Result<(), Failure> {
    let previous = updates.join(PREVIOUS);
    let _ = std::fs::remove_dir_all(&previous);
    private_dir(&previous)?;
    let bytes = std::fs::read(target)
        .map_err(|e| format!("can't read this program to keep it: {}", e.kind()))?;
    write_program(&previous.join(PROGRAM), &bytes)?;
    let known = ledger_store::format::current();
    let meta = serde_json::json!({
        "version": version,
        "format": known.format,
        "features": known.features,
    });
    std::fs::write(previous.join(META), meta.to_string())
        .map_err(|e| format!("can't keep this program: {}", e.kind()))?;
    Ok(())
}

/// INVARIANT 2: replace `target` with a copy of `from`, by renaming a
/// complete file made beside it.
fn swap_in(target: &Path, from: &Path) -> Result<(), Failure> {
    let dir = target.parent().ok_or("this program has no folder")?;
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temp = dir.join(format!(".{name}.update-{}", std::process::id()));
    let result = (|| -> std::io::Result<()> {
        let _ = std::fs::remove_file(&temp);
        let mut source = std::fs::File::open(from)?;
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o755)
            .open(&temp)?;
        std::io::copy(&mut source, &mut out)?;
        out.sync_all()?;
        drop(out);
        std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o755))?;
        std::fs::rename(&temp, target)
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&temp);
        return Err(format!(
            "can't replace the program ({}): is its folder yours to write? Nothing was changed",
            e.kind()
        )
        .into());
    }
    Ok(())
}

/// INVARIANT 4: the program at `path` starts and answers `--help`.
fn healthy(path: &Path) -> Result<(), String> {
    let deadline = Instant::now() + HEALTH_WAIT;
    let mut child = loop {
        match Command::new(path)
            .arg("--help")
            .env("APPIMAGE_EXTRACT_AND_RUN", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => break child,
            // The file was just written; a process forked meanwhile may
            // still hold it open for writing.
            Err(e)
                if e.kind() == std::io::ErrorKind::ExecutableFileBusy
                    && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("can't run it: {}", e.kind())),
        }
    };
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => return Err(format!("it exited with {status}")),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("it didn't answer in time".into());
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_three_numbers_and_order_by_number() {
        assert_eq!(parse_version("1.2.3"), Some((1, 2, 3)));
        assert!(parse_version("0.10.0") > parse_version("0.9.9"));
        for bad in ["", "1.2", "1.2.3.4", "v1.2.3", "1.2.3-rc1", "1..3", "1.2.x"] {
            assert_eq!(parse_version(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_package_marker_beside_the_program_makes_it_package_managed() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let exe = bin.join("wanderbooks");
        std::fs::write(&exe, b"x").unwrap();
        assert_eq!(install_kind(&exe, None), Install::SelfUpdating(exe.clone()));
        let share = dir.path().join("share/wanderbooks");
        std::fs::create_dir_all(&share).unwrap();
        std::fs::write(share.join("install-method"), "deb\n").unwrap();
        assert_eq!(
            install_kind(&exe, None),
            Install::PackageManaged("deb".into())
        );
        // An AppImage is its own file, whatever is beside the mount.
        let image = dir.path().join("Wanderbooks.AppImage");
        std::fs::write(&image, b"x").unwrap();
        assert_eq!(
            install_kind(&exe, Some(&image)),
            Install::SelfUpdating(image)
        );
    }

    #[test]
    fn a_public_key_is_read_in_every_form_minisign_and_the_packager_write_it() {
        let bare = "RWQdrs7W5/jf6drdD0G/ymxFmRWYAIOyggYlubz4NQzdQm5e/Fj0nrb0";
        let two = format!("untrusted comment: minisign public key: E9DFF8E7D6CEAE1D\n{bare}\n");
        use base64::Engine;
        let wrapped = base64::engine::general_purpose::STANDARD.encode(&two);
        for form in [bare.to_string(), two, wrapped] {
            parse_public_key(&form).unwrap_or_else(|e| panic!("{form}: {e}"));
        }
        assert!(parse_public_key("not a key").is_err());
    }

    #[test]
    fn a_manifest_address_is_resolved_beside_the_manifest() {
        assert_eq!(
            resolve("file:///rel/manifest.json", "wb").unwrap(),
            "file:///rel/wb"
        );
        assert_eq!(
            resolve("https://h.example/a/b/manifest.json", "wb").unwrap(),
            "https://h.example/a/b/wb"
        );
        assert_eq!(
            resolve("file:///x/m.json", "https://o/wb").unwrap(),
            "https://o/wb"
        );
    }

    #[test]
    fn a_swap_leaves_the_whole_new_file_and_no_temporary() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("wanderbooks");
        let from = dir.path().join("new");
        std::fs::write(&target, b"old").unwrap();
        std::fs::write(&from, b"new").unwrap();
        swap_in(&target, &from).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
    }
}
