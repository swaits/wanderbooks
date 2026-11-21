//! Where an installed app's books live on each system (plan P15 unit C1,
//! WB-pkg-8).
//!
//! ```text
//! GOAL: the books, keys and settings sit where each system keeps an app's
//!       own data, apart from the program, so installing a newer program
//!       over the old one, or uninstalling it, leaves them as they are.
//! SOURCE: WB-pkg-8; each system's convention for per-user application data
//!         (Apple's File System Programming Guide: Library/Application
//!         Support; Microsoft's known folders: LocalAppData; the XDG base
//!         directory specification: $XDG_DATA_HOME, default ~/.local/share).
//! INVARIANTS:
//!   1. macOS: <home>/Library/Application Support/wanderbooks.
//!   2. Windows: <home>\AppData\Local\wanderbooks (LocalAppData, the local
//!      folder, never the roaming one).
//!   3. Anywhere else: <home>/.local/share/wanderbooks, where books have
//!      always lived on Linux.
//!   4. No system's data folder is inside the folder its program installs
//!      to (the install folders are in docs/install.md and the packaging
//!      scripts), so removing the program cannot remove the books.
//! SCENARIOS:
//!   1. The three systems' folders, from one home.
//! ```

use ledger_store::root::{ROOT_NAME, data_dir_for, default_root};
use std::path::Path;

#[test]
fn each_system_keeps_its_data_where_it_keeps_an_apps_data() {
    let home = Path::new("/home/someone");
    assert_eq!(
        data_dir_for("macos", home),
        home.join("Library/Application Support")
    );
    assert_eq!(
        data_dir_for("windows", home),
        home.join("AppData").join("Local")
    );
    for other in ["linux", "freebsd", "openbsd"] {
        assert_eq!(
            data_dir_for(other, home),
            home.join(".local/share"),
            "{other}"
        );
    }
    assert_eq!(ROOT_NAME, "wanderbooks");
}

#[test]
fn this_systems_default_root_is_its_data_folder_and_the_name() {
    let home = Path::new("/home/someone");
    assert_eq!(
        default_root(home),
        data_dir_for(std::env::consts::OS, home).join(ROOT_NAME)
    );
}

/// The program's install folders (the Windows installer's, the macOS
/// bundle's, the Linux packages') are never above or below a data folder.
#[test]
fn no_data_folder_is_inside_an_install_folder() {
    let home = Path::new("/home/someone");
    let installs = [
        // Windows: the installer's InstallDir (deploy/packaging/windows).
        home.join("AppData/Local/Programs/Wanderbooks"),
        // macOS: the app (drag-to-Applications), and the per-user folder.
        Path::new("/Applications/Wanderbooks.app").to_path_buf(),
        home.join("Applications/Wanderbooks.app"),
        // Linux packages.
        Path::new("/usr/bin").to_path_buf(),
        Path::new("/usr/share/wanderbooks").to_path_buf(),
    ];
    for os in ["macos", "windows", "linux"] {
        let data = data_dir_for(os, home).join(ROOT_NAME);
        for install in &installs {
            assert!(
                !data.starts_with(install) && !install.starts_with(&data),
                "{os}: {} and {} overlap",
                data.display(),
                install.display()
            );
        }
    }
}
