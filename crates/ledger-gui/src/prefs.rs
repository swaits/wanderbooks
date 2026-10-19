//! Where the viewer keeps its preferences (window size, color scheme and
//! its filter): the platform's own place for an app's settings, in a
//! folder of the viewer's own.
//!
//! ```text
//! GOAL: the viewer's preferences live where the platform keeps app
//!       settings, and never in the books' private folder.
//! SOURCE: owner, 2026-10-19 (preferences "in the OS's platform-native
//!         location (like XDG on Linux)"); plan I-8 (the viewer writes
//!         nothing under the data root; only eframe preferences under
//!         $HOME). Found 2026-10-19: eframe's default, named after the app
//!         id "wanderbooks", is $XDG_DATA_HOME/wanderbooks, the data
//!         root's own default place.
//! INVARIANTS:
//!   1. Linux and the BSDs: $XDG_CONFIG_HOME/wanderbooks-viewer/viewer.ron
//!      ($XDG_CONFIG_HOME only if absolute, else ~/.config); macOS:
//!      ~/Library/Application Support/wanderbooks-viewer/viewer.ron;
//!      Windows: %APPDATA%\wanderbooks-viewer\viewer.ron.
//!   2. Preferences eframe kept in its old place (app.ron in
//!      $XDG_DATA_HOME/wanderbooks and the like) are moved once, when the
//!      new place has none: only that one file, nothing else there is
//!      read or touched.
//! ```

use std::path::{Path, PathBuf};

/// The viewer's own folder for its preferences.
pub const DIR: &str = "wanderbooks-viewer";
/// Its preferences file.
pub const FILE: &str = "viewer.ron";
/// eframe's own file name, in its old place.
const OLD_FILE: &str = "app.ron";

/// The preferences file for an account whose home is `home`, given
/// `$XDG_CONFIG_HOME` (or `%APPDATA%` on Windows) (INVARIANT 1).
pub fn path_in(home: &Path, config_home: Option<&Path>) -> PathBuf {
    let base = if cfg!(target_os = "macos") {
        home.join("Library").join("Application Support")
    } else if cfg!(windows) {
        config_home
            .map(Path::to_path_buf)
            .unwrap_or_else(|| home.join("AppData").join("Roaming"))
    } else {
        config_home
            .filter(|p| p.is_absolute())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| home.join(".config"))
    };
    base.join(DIR).join(FILE)
}

/// The preferences file for this account, if it has a home.
pub fn path() -> Option<PathBuf> {
    let home = std::env::home_dir()?;
    let var = if cfg!(windows) {
        "APPDATA"
    } else {
        "XDG_CONFIG_HOME"
    };
    let config = std::env::var_os(var).map(PathBuf::from);
    Some(path_in(&home, config.as_deref()))
}

/// Move preferences from `old_dir/app.ron` to `new` if `new` doesn't
/// exist yet (INVARIANT 2). Failures change nothing that matters: the
/// viewer starts with its defaults.
pub fn move_old(old_dir: &Path, new: &Path) {
    let old = old_dir.join(OLD_FILE);
    if new.exists() || !old.is_file() {
        return;
    }
    let Some(dir) = new.parent() else {
        return;
    };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    if std::fs::copy(&old, new).is_ok() {
        let _ = std::fs::remove_file(&old);
    }
}

/// Where the viewer keeps its preferences, its old ones moved in
/// (INVARIANT 2): None when the account has no home.
pub fn prepare() -> Option<PathBuf> {
    let new = path()?;
    if let Some(old_dir) = eframe::storage_dir("wanderbooks") {
        move_old(&old_dir, &new);
    }
    Some(new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn on_linux_it_is_the_xdg_config_folder_of_its_own() {
        let home = Path::new("/home/someone");
        assert_eq!(
            path_in(home, None),
            Path::new("/home/someone/.config/wanderbooks-viewer/viewer.ron")
        );
        assert_eq!(
            path_in(home, Some(Path::new("/cfg"))),
            Path::new("/cfg/wanderbooks-viewer/viewer.ron")
        );
        // A relative XDG_CONFIG_HOME is ignored (the spec says so).
        assert_eq!(
            path_in(home, Some(Path::new("cfg"))),
            Path::new("/home/someone/.config/wanderbooks-viewer/viewer.ron")
        );
        // Never the data root's place.
        assert!(!path_in(home, None).starts_with("/home/someone/.local/share/wanderbooks"));
    }

    #[test]
    fn old_preferences_are_moved_once_and_nothing_else_is_touched() {
        let dir = crate::test_support::TempDir(
            std::env::temp_dir().join(format!("ledger-gui-prefs-{}", std::process::id())),
        );
        let _ = std::fs::remove_dir_all(&dir.0);
        std::fs::create_dir_all(&dir.0).unwrap();
        let old = dir.0.join("old");
        std::fs::create_dir(&old).unwrap();
        std::fs::write(old.join("app.ron"), "(scheme: \"Ayu/Mirage\")").unwrap();
        std::fs::write(old.join("other"), "kept").unwrap();
        let new = dir.0.join("cfg/wanderbooks-viewer/viewer.ron");
        move_old(&old, &new);
        assert_eq!(
            std::fs::read_to_string(&new).unwrap(),
            "(scheme: \"Ayu/Mirage\")"
        );
        assert!(!old.join("app.ron").exists(), "moved, not copied");
        assert_eq!(std::fs::read_to_string(old.join("other")).unwrap(), "kept");
        // Once: new preferences are never overwritten by old ones.
        std::fs::write(old.join("app.ron"), "older").unwrap();
        std::fs::write(&new, "newer").unwrap();
        move_old(&old, &new);
        assert_eq!(std::fs::read_to_string(&new).unwrap(), "newer");
        assert!(old.join("app.ron").exists());
        // Nothing to move: nothing made.
        let none = dir.0.join("none/viewer.ron");
        move_old(&dir.0.join("missing"), &none);
        assert!(!none.exists() && !dir.0.join("none").exists());
    }
}
