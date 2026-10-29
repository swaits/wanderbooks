//! `wanderbooks setup`: the books' private folder, made from the command
//! line, with its location never printed.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn wanderbooks() -> Command {
    Command::new(env!("CARGO_BIN_EXE_wanderbooks"))
}

fn mode(path: &std::path::Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn setup_makes_the_private_folder_and_never_names_it() {
    let home = tempfile::Builder::new()
        .prefix("wb-setup-")
        .tempdir()
        .unwrap();
    let root = ledger_store::root::default_root(home.path());
    let out = wanderbooks()
        .args(["setup", "--home"])
        .arg(home.path())
        .env_remove("XDG_DATA_HOME")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("The books' private folder: made"), "{text}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--home isn't this account's home"),
        "{out:?}"
    );
    assert!(
        !text.contains(home.path().to_str().unwrap()),
        "names the folder: {text}"
    );
    assert!(!String::from_utf8_lossy(&out.stderr).contains(home.path().to_str().unwrap()));
    assert_eq!(mode(&root), 0o700);
    for sub in ["books", "run", "legacy"] {
        assert_eq!(mode(&root.join(sub)), 0o700, "{sub}");
    }
    // Again: already there.
    let out = wanderbooks()
        .args(["setup", "--home"])
        .arg(home.path())
        .env_remove("XDG_DATA_HOME")
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("already there."),
        "{out:?}"
    );
    // Others could open it: made private again.
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = wanderbooks()
        .args(["setup", "--home"])
        .arg(home.path())
        .env_remove("XDG_DATA_HOME")
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("made private again"),
        "{out:?}"
    );
    assert_eq!(mode(&root), 0o700);
}

#[test]
fn a_root_elsewhere_is_noted_for_next_time() {
    // (XDG_DATA_HOME is honoured only for this account's own home, which
    // a test never touches: Root::setup's tests cover it.)
    let home = tempfile::Builder::new()
        .prefix("wb-setup-else-")
        .tempdir()
        .unwrap();
    let elsewhere = home.path().join("elsewhere").join("wanderbooks");
    let out = wanderbooks()
        .args(["setup", "--home"])
        .arg(home.path())
        .arg("--root")
        .arg(&elsewhere)
        .env_remove("XDG_DATA_HOME")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("noted for next time"),
        "{out:?}"
    );
    assert_eq!(mode(&elsewhere), 0o700);
    let noted = std::fs::read_to_string(
        ledger_store::root::platform_data_dir(home.path()).join("wanderbooks.location"),
    )
    .unwrap();
    assert_eq!(noted.trim_end(), elsewhere.display().to_string());
}

#[test]
fn setup_refuses_what_it_does_not_know_and_unsafe_folders() {
    let out = wanderbooks().args(["setup", "--bogus"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let out = wanderbooks().args(["setup", "--root"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--root needs a path"));
    let home = tempfile::Builder::new()
        .prefix("wb-setup-link-")
        .tempdir()
        .unwrap();
    let real = home.path().join("real");
    std::fs::create_dir(&real).unwrap();
    std::os::unix::fs::symlink(&real, home.path().join("link")).unwrap();
    let out = wanderbooks()
        .args(["setup", "--home"])
        .arg(home.path())
        .arg("--root")
        .arg(home.path().join("link"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("is a symlink"));
    // A mode comes first.
    let out = wanderbooks().args(["--force", "setup"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}

/// With another home given (tests), this account's XDG_DATA_HOME is never
/// read: setup can't be aimed at this account's real books by it.
#[test]
fn another_home_ignores_this_accounts_data_home() {
    let home = tempfile::Builder::new()
        .prefix("wb-setup-xdg-")
        .tempdir()
        .unwrap();
    let elsewhere = home.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    let out = wanderbooks()
        .args(["setup", "--home"])
        .arg(home.path())
        .env("XDG_DATA_HOME", &elsewhere)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(
        ledger_store::root::default_root(home.path())
            .join(".wanderbooks-root")
            .is_file()
    );
    assert_eq!(
        std::fs::read_dir(&elsewhere).unwrap().count(),
        0,
        "XDG_DATA_HOME was used"
    );
}

/// Run `args` at a terminal (a pseudo-terminal, through util-linux
/// `script`), typing `lines` one at a time: its exit code and what the
/// terminal showed.
fn at_terminal(args: &[String], lines: &[&str]) -> (i32, String) {
    use std::io::Write;
    let command = std::iter::once(env!("CARGO_BIN_EXE_wanderbooks").to_string())
        .chain(args.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ");
    let mut child = Command::new("script")
        .args(["-qefc", &command, "/dev/null"])
        .env_remove("XDG_DATA_HOME")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("util-linux script");
    let mut stdin = child.stdin.take().unwrap();
    // What it shows, as it shows it: each line is typed only once it has
    // asked for it (a prompt ending ": "), never before it stops echoing.
    let shown_so_far = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let mut stdout = child.stdout.take().unwrap();
    let reader = {
        let shown = shown_so_far.clone();
        std::thread::spawn(move || {
            use std::io::Read;
            let mut buf = [0u8; 4096];
            while let Ok(n) = stdout.read(&mut buf) {
                if n == 0 {
                    break;
                }
                shown.lock().unwrap().extend_from_slice(&buf[..n]);
            }
        })
    };
    let prompts = |shown: &[u8]| {
        String::from_utf8_lossy(shown)
            .split(['\r', '\n'])
            .filter(|l| l.ends_with(": ") || l.ends_with(':'))
            .count()
    };
    for (i, line) in lines.iter().enumerate() {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while prompts(&shown_so_far.lock().unwrap()) <= i {
            if child.try_wait().unwrap().is_some() || std::time::Instant::now() > deadline {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // It turns its echo off just after asking.
        std::thread::sleep(std::time::Duration::from_millis(150));
        let _ = stdin.write_all(format!("{line}\n").as_bytes());
    }
    // Kept open until it ends: script stops when its input does.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while child.try_wait().unwrap().is_none() {
        assert!(std::time::Instant::now() < deadline, "it never ended");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    let _ = reader.join();
    let shown = format!(
        "{}{}",
        String::from_utf8_lossy(&shown_so_far.lock().unwrap()),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.code().unwrap_or(-1), shown)
}

const FIRST: &str = "correct horse battery";
const SECOND: &str = "a different staple phrase";

#[test]
fn setup_at_a_terminal_sets_the_owner_passphrase() {
    let home = tempfile::Builder::new()
        .prefix("wb-pass-")
        .tempdir()
        .unwrap();
    let args = vec![
        "setup".to_string(),
        "--home".into(),
        home.path().display().to_string(),
    ];
    let (code, shown) = at_terminal(&args, &[FIRST, FIRST]);
    assert_eq!(code, 0, "{shown}");
    assert!(shown.contains("The owner passphrase is set."), "{shown}");
    assert!(!shown.contains(FIRST), "the passphrase was shown: {shown}");
    assert!(!shown.contains(home.path().to_str().unwrap()), "{shown}");
    let auth = ledger_store::root::default_root(home.path()).join("auth");
    assert!(ledger_store::passphrase::verify(&auth, FIRST).unwrap());
    assert_eq!(mode(&auth), 0o700);
    assert_eq!(mode(&auth.join("owner.json")), 0o600);
    // Run again, it asks for nothing more.
    let (code, shown) = at_terminal(&args, &[]);
    assert_eq!(code, 0, "{shown}");
    assert!(!shown.contains("passphrase"), "{shown}");
}

#[test]
fn the_passphrase_is_typed_by_a_person_and_changed_only_with_the_one_in_force() {
    let home = tempfile::Builder::new()
        .prefix("wb-pass-")
        .tempdir()
        .unwrap();
    // Set up without a terminal: the folder, and a note.
    let out = wanderbooks()
        .args(["setup", "--home"])
        .arg(home.path())
        .env_remove("XDG_DATA_HOME")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("wanderbooks passphrase"));
    let root = ledger_store::root::default_root(home.path());
    let auth = root.join("auth");
    assert!(!ledger_store::passphrase::is_set(&auth).unwrap());
    let args = vec![
        "passphrase".to_string(),
        "--root".into(),
        root.display().to_string(),
    ];
    // No terminal (a script, an agent): refused, nothing set.
    let out = wanderbooks()
        .args(&args)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("typed by a person"));
    assert!(!ledger_store::passphrase::is_set(&auth).unwrap());
    // Too short, or the two not matching: nothing set.
    for lines in [["short", "short"], [FIRST, SECOND]] {
        let (code, shown) = at_terminal(&args, &lines);
        assert_eq!(code, 1, "{shown}");
        assert!(!ledger_store::passphrase::is_set(&auth).unwrap(), "{shown}");
    }
    let (code, shown) = at_terminal(&args, &[FIRST, FIRST]);
    assert_eq!(code, 0, "{shown}");
    // A change needs the one in force.
    let (code, shown) = at_terminal(&args, &["not the passphrase at all"]);
    assert_eq!(code, 1, "{shown}");
    assert!(shown.contains("isn't the owner passphrase"), "{shown}");
    assert!(ledger_store::passphrase::verify(&auth, FIRST).unwrap());
    let (code, shown) = at_terminal(&args, &[FIRST, SECOND, SECOND]);
    assert_eq!(code, 0, "{shown}");
    assert!(!shown.contains(FIRST) && !shown.contains(SECOND), "{shown}");
    assert!(ledger_store::passphrase::verify(&auth, SECOND).unwrap());
    assert!(!ledger_store::passphrase::verify(&auth, FIRST).unwrap());
    let kept = std::fs::read_to_string(auth.join("owner.json")).unwrap();
    assert!(!kept.contains("staple"), "the text itself was kept");
}
