//! "A newer version exists, and where to get it" (plan P15 unit C1, 3.1:
//! update is an ordinary app update).
//!
//! The installed program is replaced by installing the newer package over
//! it (the installer on Windows, the `.dmg` on macOS, the package manager
//! or a new AppImage on Linux). The program's part is to tell the person
//! that one exists. This module is that part, on every platform: a look at
//! one small file a release publishes, at most once a day, and a note of
//! what it found in the data root.
//!
//! ```text
//! GOAL: a person who installed Wanderbooks is told, in the window, when a
//!       newer version is out, and where to get it; nothing is downloaded
//!       or installed by the program.
//! SOURCE: WB-pkg-3; plan/p15-app-and-relay.md 3.1 ("Update becomes an
//!         ordinary app update"), D-24 (anything the program installs by
//!         itself must be verified against our own release keys: this
//!         installs nothing, so it needs none).
//! INVARIANTS:
//!   1. Only a version number is read from the feed: the place to get the
//!      update is a fixed address in this program ([`DOWNLOAD_PAGE`]),
//!      never one the feed names. A feed that lies can at most say a
//!      number.
//!   2. The feed is looked at at most once a day ([`EVERY`]); a failed look
//!      counts as a look (no hammering) and is never an error the person
//!      sees. The look is never in the window's own thread.
//!   3. A version is "newer" only when it is a higher `x.y.z` than this
//!      program's; anything else is no version of ours.
//!   4. The note is a small file in the data root's `updates/` folder,
//!      private to the account; the books are not read or changed.
//!   5. The look sends no version, company, key or identifier of the
//!      person: a plain request for a public file.
//! SCENARIOS:
//!   1. IF the feed says 0.2.0 and this is 0.1.0: SHOULD say 0.2.0 is
//!      newer, for a day.
//!   2. IF the feed says the same, an older, or a word: SHOULD say nothing.
//!   3. IF the feed can't be reached: SHOULD say nothing and look again
//!      tomorrow.
//! ```

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// The small file a release publishes: `{"version": "x.y.z"}`.
pub const FEED: &str = "https://github.com/swaits/wanderbooks/releases/latest/download/latest.json";

/// Where a person gets the newest version (INVARIANT 1).
pub const DOWNLOAD_PAGE: &str = "https://github.com/swaits/wanderbooks/releases/latest";

/// How often the feed is looked at (INVARIANT 2).
pub const EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// The note's name in `<root>/updates/`.
const NOTE: &str = "newer.json";

/// The longest feed read, in bytes.
const MAX_FEED: usize = 4096;

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

/// Whether `theirs` is a version higher than `mine` (INVARIANT 3).
pub fn is_newer(mine: &str, theirs: &str) -> bool {
    parse_version(theirs)
        .zip(parse_version(mine))
        .is_some_and(|(theirs, mine)| theirs > mine)
}

/// The version the feed's text names (INVARIANT 1): nothing else of it is
/// read.
pub fn version_in_feed(text: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Feed {
        version: String,
    }
    let feed: Feed = serde_json::from_str(text).ok()?;
    parse_version(&feed.version).map(|_| feed.version)
}

/// What the last look found.
#[derive(Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
struct Note {
    /// Seconds since 1970 when the look was made.
    looked: u64,
    /// The version the feed named, when it named one.
    version: Option<String>,
}

fn note_path(root: &Path) -> PathBuf {
    root.join("updates").join(NOTE)
}

fn read_note(root: &Path) -> Option<Note> {
    let bytes = std::fs::read(note_path(root)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The newer version the last look found, if it found one newer than
/// `mine` (a note about a version this program already is, or is past, is
/// nothing).
pub fn found(root: &Path, mine: &str) -> Option<String> {
    read_note(root)?
        .version
        .filter(|theirs| is_newer(mine, theirs))
}

/// Whether the feed is due to be looked at at `now` (INVARIANT 2).
pub fn due(root: &Path, now: u64) -> bool {
    read_note(root)
        .is_none_or(|n| now.saturating_sub(n.looked) >= EVERY.as_secs() || n.looked > now)
}

/// Look at the feed through `fetch` (its text) and note what it said, at
/// `now`. A feed that can't be read is noted as a look with nothing found
/// (INVARIANT 2). The newer version, if there is one.
pub fn look(
    root: &Path,
    mine: &str,
    now: u64,
    fetch: impl FnOnce() -> Result<String, String>,
) -> Result<Option<String>, String> {
    let said = fetch();
    let version = said.as_deref().ok().and_then(version_in_feed);
    let note = Note {
        looked: now,
        version: version.clone(),
    };
    let dir = root.join("updates");
    ledger_link::private::make_dir(&dir).map_err(|e| e.kind().to_string())?;
    let bytes = serde_json::to_vec(&note).map_err(|e| e.to_string())?;
    ledger_link::private::write(&note_path(root), &bytes).map_err(|e| e.kind().to_string())?;
    match said {
        Ok(_) => Ok(version.filter(|theirs| is_newer(mine, theirs))),
        Err(why) => Err(why),
    }
}

/// The feed, over https: a plain request for a public file (INVARIANT 5).
pub fn fetch_feed() -> Result<String, String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async {
        let mut response = reqwest::Client::new()
            .get(FEED)
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("can't look for a newer version: {}", e.without_url()))?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| format!("can't look for a newer version: {}", e.without_url()))?
        {
            bytes.extend_from_slice(&chunk);
            if bytes.len() > MAX_FEED {
                return Err("the release note is larger than it can be".to_string());
            }
        }
        String::from_utf8(bytes).map_err(|_| "the release note is not text".to_string())
    })
}

/// "Check for updates", now: look whatever the last look was, and say
/// what came of it in plain words.
pub fn check_now(root: &Path, mine: &str) -> Result<String, String> {
    match look(root, mine, now_secs(), fetch_feed)? {
        Some(version) => Ok(format!(
            "Version {version} of Wanderbooks is available (this is {mine}). Get it at {DOWNLOAD_PAGE} and install it over this one: your books, keys and connections stay as they are."
        )),
        None => Ok(format!("You have the newest version ({mine}).")),
    }
}

/// Whether a background look is under way.
static LOOKING: AtomicBool = AtomicBool::new(false);

/// The newer version known, and, when a look is due, one begun on a thread
/// of its own (INVARIANT 2): cheap, for the window to call every few
/// seconds.
pub fn found_and_refresh(root: &Path, mine: &'static str) -> Option<String> {
    if due(root, now_secs())
        && LOOKING
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    {
        let root = root.to_path_buf();
        let started = std::thread::Builder::new()
            .name("wanderbooks-newer".into())
            .spawn(move || {
                // Whatever it came to, it was a look (noted), and the next
                // is a day away.
                let _ = look(&root, mine, now_secs(), fetch_feed);
                LOOKING.store(false, Ordering::SeqCst);
            });
        if started.is_err() {
            LOOKING.store(false, Ordering::SeqCst);
        }
    }
    found(root, mine)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_higher_plain_version_is_newer() {
        assert!(is_newer("0.1.0", "0.2.0"));
        assert!(is_newer("0.1.9", "0.1.10"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.2.0", "0.1.9"));
        for odd in ["v0.3.0", "0.3", "0.3.0-rc1", "", "latest", "1.2.3.4"] {
            assert!(!is_newer("0.1.0", odd), "{odd}");
        }
    }

    #[test]
    fn the_feed_gives_a_version_and_nothing_else() {
        assert_eq!(
            version_in_feed(r#"{"version":"0.2.0","url":"https://evil.example/x"}"#),
            Some("0.2.0".into())
        );
        assert_eq!(version_in_feed(r#"{"version":"soon"}"#), None);
        assert_eq!(version_in_feed("not json"), None);
        // Where to get it is this program's, never the feed's.
        assert!(DOWNLOAD_PAGE.starts_with("https://github.com/swaits/wanderbooks/"));
    }

    #[test]
    fn a_look_is_noted_and_the_next_is_a_day_away() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(due(root, 1_000));
        let said = look(root, "0.1.0", 1_000, || Ok(r#"{"version":"0.2.0"}"#.into())).unwrap();
        assert_eq!(said, Some("0.2.0".into()));
        assert_eq!(found(root, "0.1.0"), Some("0.2.0".into()));
        assert!(!due(root, 1_000 + EVERY.as_secs() - 1));
        assert!(due(root, 1_000 + EVERY.as_secs()));
        // A clock that went backwards is due, not stuck for years.
        assert!(due(root, 10));
        // Once this program is that version, nothing is newer.
        assert_eq!(found(root, "0.2.0"), None);
    }

    #[test]
    fn a_failed_look_says_nothing_and_still_counts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        look(root, "0.1.0", 1_000, || Ok(r#"{"version":"0.2.0"}"#.into())).unwrap();
        let err = look(root, "0.1.0", 2_000, || Err("offline".into())).unwrap_err();
        assert_eq!(err, "offline");
        assert_eq!(found(root, "0.1.0"), None, "what was found is forgotten");
        assert!(!due(root, 2_001), "the failed look counts");
    }

    #[cfg(unix)]
    #[test]
    fn the_note_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        look(dir.path(), "0.1.0", 1, || Ok("{}".into())).unwrap();
        let mode = |p: PathBuf| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(dir.path().join("updates")), 0o700);
        assert_eq!(mode(note_path(dir.path())), 0o600);
    }
}
