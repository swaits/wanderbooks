//! Starting the host cleanly (plan 3.9, "Starting the host cleanly").
//!
//! ```text
//! GOAL: the host never inherits anything from whoever opened the books:
//!       not their environment, descriptors, session, umask, or the app's
//!       scope (which would take the host down with the app).
//! SOURCE: plan/served.md 3.9; test host_starts_clean, host_survives_app_scope.
//! INVARIANTS:
//!   1. The host's environment is a fixed list: PATH (fixed), HOME (from
//!      the account's record, never the opener's), and XDG_RUNTIME_DIR
//!      and DBUS_SESSION_BUS_ADDRESS only when they point at this
//!      account's own private runtime folder, and SSH_AUTH_SOCK only when
//!      it names a socket this account owns (the owner's ssh agent, so the
//!      server's backups reach a private git repository, D-30). Nothing
//!      else passes (a scope's own identifiers, such as INVOCATION_ID, are
//!      systemd's).
//!   2. Its standard input and output are null, its errors go to
//!      run/host.log (0600); it starts a new session with umask 077. Every
//!      other descriptor is closed (std opens them close-on-exec, and the
//!      host closes any stray one at start).
//!   3. Under a systemd app scope it starts through a transient
//!      `systemd-run --user --scope` (nothing installed or kept); if the
//!      scope couldn't be made (systemd-run failed without the host ever
//!      writing its first line), or there is no such scope, directly. A
//!      host that ran and stopped is never started a second time.
//!   4. It runs this executable's file as it is on disk now.
//!   5. The opener doesn't wait for it, and reaps it when it ends.
//! ```

use crate::knobs::Knobs;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The host's PATH (INVARIANT 1).
const PATH: &str = "/usr/local/bin:/usr/bin:/bin";

/// Where systemd-run may be (never looked up on the opener's PATH).
const SYSTEMD_RUN: [&str; 2] = ["/usr/bin/systemd-run", "/bin/systemd-run"];

/// The host's log, in the root's `run/`.
pub const HOST_LOG: &str = "host.log";
/// A log larger than this is moved aside when a host starts.
const LOG_LIMIT: u64 = 8 * 1024 * 1024;

/// This executable's file as it is on disk now (INVARIANT 4): on Linux a
/// replaced file reads as "… (deleted)"; the path, not the old file, is
/// what a new host runs.
pub fn this_executable() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("can't find this program: {e}"))?;
    let text = exe.to_string_lossy();
    if let Some(path) = text.strip_suffix(" (deleted)")
        && !exe.exists()
    {
        return Ok(PathBuf::from(path));
    }
    Ok(exe)
}

/// This account's user ID.
fn own_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and can't fail.
    unsafe { libc::geteuid() }
}

/// `XDG_RUNTIME_DIR` from `value`, if it is this account's own private
/// runtime folder: absolute, a real folder (no link), owned by this
/// account, and closed to everyone else.
pub fn own_runtime_dir(value: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    // Written plainly (no trailing slash, no `.`), as a bus address
    // names it.
    let dir: PathBuf = PathBuf::from(value?).components().collect();
    if !dir.is_absolute() {
        return None;
    }
    let meta = std::fs::symlink_metadata(&dir).ok()?;
    (meta.is_dir() && meta.uid() == own_uid() && meta.mode() & 0o077 == 0).then_some(dir)
}

/// `DBUS_SESSION_BUS_ADDRESS` from `value`, if it is the bus in `runtime`
/// (`unix:path=<runtime>/bus`, perhaps with a guid after a comma).
pub fn own_bus(value: Option<&std::ffi::OsStr>, runtime: &Path) -> Option<String> {
    let value = value?.to_str()?;
    let want = format!("unix:path={}/bus", runtime.display());
    let rest = value.strip_prefix(&want)?;
    (rest.is_empty() || rest.starts_with(',')).then(|| value.to_string())
}

/// This account's runtime folder: the one named in the environment if it
/// is ours and private; else, when the opener's environment has none (a
/// client that scrubs it, as Codex does), the system's for this account,
/// `/run/user/<uid>`, under the same checks.
fn runtime_dir() -> Option<PathBuf> {
    // The one named, if it passes; else (none named, or one that isn't
    // ours or private) the system's for this account.
    std::env::var_os("XDG_RUNTIME_DIR")
        .and_then(|named| own_runtime_dir(Some(&named)))
        .or_else(|| own_runtime_dir(Some(format!("/run/user/{}", own_uid()).as_ref())))
}

/// The session bus in `runtime`: the one named in the environment if it
/// is there, else its usual socket there, if one exists.
fn session_bus(runtime: &Path) -> Option<String> {
    match std::env::var_os("DBUS_SESSION_BUS_ADDRESS") {
        Some(named) => own_bus(Some(&named), runtime),
        None => {
            use std::os::unix::fs::FileTypeExt;
            let socket = runtime.join("bus");
            std::fs::symlink_metadata(&socket)
                .is_ok_and(|m| m.file_type().is_socket() && m.uid() == own_uid())
                .then(|| format!("unix:path={}", socket.display()))
        }
    }
}

/// The ssh agent's socket named in `named`, if it is a socket this
/// account owns at an absolute path (INVARIANT 1).
fn own_agent(named: Option<&std::ffi::OsStr>) -> Option<std::ffi::OsString> {
    use std::os::unix::fs::FileTypeExt;
    let path = Path::new(named?);
    (path.is_absolute()
        && std::fs::symlink_metadata(path)
            .is_ok_and(|m| m.file_type().is_socket() && m.uid() == own_uid()))
    .then(|| path.as_os_str().to_os_string())
}

/// The host's environment (INVARIANT 1), given the opener's
/// `SSH_AUTH_SOCK`.
fn host_env(home: &Path, ssh_agent: Option<&std::ffi::OsStr>) -> Vec<(String, std::ffi::OsString)> {
    let mut env = vec![
        ("PATH".to_string(), PATH.into()),
        ("HOME".to_string(), home.as_os_str().to_os_string()),
    ];
    if let Some(runtime) = runtime_dir() {
        if let Some(bus) = session_bus(&runtime) {
            env.push(("DBUS_SESSION_BUS_ADDRESS".into(), bus.into()));
        }
        env.push(("XDG_RUNTIME_DIR".into(), runtime.into_os_string()));
    }
    if let Some(agent) = own_agent(ssh_agent) {
        env.push(("SSH_AUTH_SOCK".into(), agent));
    }
    env
}

/// Whether this process sits in a unit under this account's user manager
/// (an app's scope or service, as uwsm makes for every app), whose stop
/// would take the host with it; going by the unified cgroup line of
/// `/proc/self/cgroup`. The manager's own `init.scope` is not such a
/// unit.
pub fn in_app_scope(cgroup: &str, uid: u32) -> bool {
    let manager = format!("/user@{uid}.service/");
    cgroup.lines().any(|line| {
        line.strip_prefix("0::").is_some_and(|path| {
            path.split_once(manager.as_str())
                .is_some_and(|(_, unit)| !unit.is_empty() && unit != "init.scope")
        })
    })
}

/// systemd-run, if this process is in an app scope and the user manager
/// can be reached (INVARIANT 3).
fn systemd_run() -> Option<&'static str> {
    let cgroup = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    if !in_app_scope(&cgroup, own_uid()) {
        return None;
    }
    // systemd-run reaches the user manager through the runtime folder,
    // which host_env gives it even when the opener's environment didn't.
    runtime_dir()?;
    SYSTEMD_RUN.into_iter().find(|p| Path::new(p).is_file())
}

/// Start a host for the root (`root`, else the one the host finds
/// itself), with these knobs (INVARIANTS 1-5).
pub fn start_host(root: Option<&Path>, knobs: &Knobs) -> Result<(), String> {
    let found = ledger_store::root::Root::find(root).map_err(|e| e.to_string())?;
    let run = found.run_dir().map_err(|e| e.to_string())?;
    let home = ledger_store::root::account_home().map_err(|e| e.to_string())?;
    let exe = this_executable()?;
    let mut args: Vec<std::ffi::OsString> = vec!["host".into()];
    // An explicit root goes on as found (absolute): the host starts in /.
    if root.is_some() {
        args.push("--root".into());
        args.push(found.path().as_os_str().to_os_string());
    }
    args.extend(knobs.args().into_iter().map(Into::into));
    let log = || {
        let path = run.join(HOST_LOG);
        // A large log is moved aside first (to `.1`), never from under a
        // host writing it.
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > LOG_LIMIT)
            && !crate::opener::host_lock_held(&run)
        {
            let _ = std::fs::rename(&path, path.with_extension("log.1"));
        }
        std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| format!("can't open the host's log: {}", e.kind()))
    };
    // Through systemd-run (`Some`), or directly.
    let command = |through: Option<&str>, log: std::fs::File| -> Result<Command, String> {
        let mut c = match through {
            Some(systemd_run) => {
                let mut c = Command::new(systemd_run);
                c.args(["--user", "--scope", "--quiet", "--collect", "--"])
                    .arg(&exe);
                c
            }
            None => Command::new(&exe),
        };
        c.args(&args)
            .env_clear()
            .envs(host_env(
                &home,
                std::env::var_os("SSH_AUTH_SOCK").as_deref(),
            ))
            .current_dir("/")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log);
        // SAFETY: setsid and umask are async-signal-safe and touch only
        // the child.
        unsafe {
            c.pre_exec(|| {
                libc::setsid();
                libc::umask(0o077);
                Ok(())
            });
        }
        Ok(c)
    };
    if let Some(systemd_run) = systemd_run() {
        // The log as this start opened it, and where it stands now: what
        // the host writes after this is read back from that same file,
        // even if another opener moves the log aside meanwhile.
        let file = log()?;
        let from = file.metadata().map_or(0, |m| m.len());
        let watch = file
            .try_clone()
            .map_err(|e| format!("can't open the host's log: {}", e.kind()))?;
        if let Ok(mut child) = command(Some(systemd_run), file)?.spawn()
            && scope_made(&mut child, &watch, from)
        {
            reap(child);
            return Ok(());
        }
    }
    let child = command(None, log()?)?
        .spawn()
        .map_err(|e| format!("can't start the books' host: {e}"))?;
    reap(child);
    Ok(())
}

/// How long systemd-run has to make its scope and run the host.
const SCOPE_WAIT: Duration = Duration::from_secs(2);

/// Whether systemd-run made its scope and ran the host (INVARIANT 3): the
/// host's first line is in its log after `from`, or systemd-run is still
/// running when SCOPE_WAIT is up (it became the host). Only systemd-run
/// failing with no host line means it didn't: then the host is started
/// directly. (systemd-run's own complaints go to the same log, so the
/// log growing proves nothing; the host's line does.)
fn scope_made(child: &mut std::process::Child, log: &std::fs::File, from: u64) -> bool {
    let host_ran = || {
        use std::os::unix::fs::FileExt;
        let len = log.metadata().map_or(0, |m| m.len());
        let mut tail = vec![0u8; len.saturating_sub(from).min(64 * 1024) as usize];
        log.read_exact_at(&mut tail, from).is_ok()
            && String::from_utf8_lossy(&tail).contains(crate::host::STARTED)
    };
    let deadline = Instant::now() + SCOPE_WAIT;
    loop {
        if host_ran() {
            return true;
        }
        match child.try_wait() {
            Ok(Some(status)) => return status.success() || host_ran(),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            _ => return true,
        }
    }
}

/// Reap a started host when it ends, without waiting for it (INVARIANT 5).
fn reap(mut child: std::process::Child) {
    let _ = std::thread::Builder::new()
        .name("reap-host".into())
        .spawn(move || {
            let _ = child.wait();
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn an_app_scope_is_known_by_its_unified_cgroup() {
        let uwsm = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-Hyprland-kitty-1234.scope\n";
        assert!(in_app_scope(uwsm, 1000));
        assert!(!in_app_scope(uwsm, 1001), "another account's manager");
        // An app run as a service unit (uwsm's app-…@….service) too.
        assert!(in_app_scope(
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-foo@x.service\n",
            1000
        ));
        for not in [
            "0::/user.slice/user-1000.slice/session-2.scope\n",
            "0::/user.slice/user-1000.slice/user@1000.service/init.scope\n",
            "0::/user.slice/user-1000.slice/user@1000.service/\n",
            "0::/\n",
            "1:name=systemd:/user.slice/user-1000.slice/user@1000.service/app.slice/x.scope\n",
            "",
        ] {
            assert!(!in_app_scope(not, 1000), "{not}");
        }
    }

    #[test]
    fn only_our_own_private_runtime_folder_passes() {
        let dir = tempfile::tempdir().unwrap();
        let rt = dir.path().join("rt");
        std::fs::create_dir(&rt).unwrap();
        std::fs::set_permissions(&rt, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(own_runtime_dir(Some(rt.as_os_str())), Some(rt.clone()));
        std::fs::set_permissions(&rt, std::fs::Permissions::from_mode(0o750)).unwrap();
        assert_eq!(
            own_runtime_dir(Some(rt.as_os_str())),
            None,
            "group can enter"
        );
        std::fs::set_permissions(&rt, std::fs::Permissions::from_mode(0o700)).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&rt, &link).unwrap();
        assert_eq!(own_runtime_dir(Some(link.as_os_str())), None, "a link");
        assert_eq!(own_runtime_dir(Some("relative/rt".as_ref())), None);
        assert_eq!(
            own_runtime_dir(Some(dir.path().join("none").as_os_str())),
            None
        );
        assert_eq!(own_runtime_dir(None), None);
        // Owned by root, not us.
        if own_uid() != 0 {
            assert_eq!(own_runtime_dir(Some("/".as_ref())), None);
        }
    }

    #[test]
    fn only_the_bus_in_our_runtime_folder_passes() {
        let rt = Path::new("/run/user/1000");
        assert_eq!(
            own_bus(Some("unix:path=/run/user/1000/bus".as_ref()), rt).as_deref(),
            Some("unix:path=/run/user/1000/bus")
        );
        assert!(own_bus(Some("unix:path=/run/user/1000/bus,guid=ab".as_ref()), rt).is_some());
        for bad in [
            "unix:path=/tmp/evil/bus",
            "unix:path=/run/user/1000/bus2",
            "unix:path=/run/user/1000/busx,guid=1",
            "tcp:host=evil.example,port=1",
            "unix:abstract=/run/user/1000/bus",
        ] {
            assert_eq!(own_bus(Some(bad.as_ref()), rt), None, "{bad}");
        }
        assert_eq!(own_bus(None, rt), None);
    }

    #[test]
    fn the_host_environment_carries_nothing_else() {
        let env = host_env(Path::new("/home/someone"), None);
        let names: Vec<&str> = env.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(&names[..2], ["PATH", "HOME"]);
        for n in &names[2..] {
            assert!(
                [
                    "XDG_RUNTIME_DIR",
                    "DBUS_SESSION_BUS_ADDRESS",
                    "SSH_AUTH_SOCK"
                ]
                .contains(n),
                "{n}"
            );
        }
        assert_eq!(env[0].1, PATH);
        assert_eq!(env[1].1, "/home/someone");
    }

    #[test]
    fn only_an_own_ssh_agent_socket_passes() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("agent.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        assert_eq!(
            own_agent(Some(sock.as_os_str())),
            Some(sock.clone().into_os_string())
        );
        // It reaches the host's environment, once.
        let env = host_env(Path::new("/home/someone"), Some(sock.as_os_str()));
        let agents: Vec<_> = env.iter().filter(|(n, _)| n == "SSH_AUTH_SOCK").collect();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].1, sock.as_os_str());
        // A link to it is not it (whoever made the link chose where it
        // points).
        let link = dir.path().join("link.sock");
        std::os::unix::fs::symlink(&sock, &link).unwrap();
        assert_eq!(own_agent(Some(link.as_os_str())), None);
        let env = host_env(Path::new("/home/someone"), Some(link.as_os_str()));
        assert!(env.iter().all(|(n, _)| n != "SSH_AUTH_SOCK"));
        // Named relative to here: not taken, even where it leads to one.
        let here = std::env::current_dir().unwrap();
        let up = here.components().count() - 1;
        let relative: std::path::PathBuf = std::iter::repeat_n("..", up)
            .collect::<std::path::PathBuf>()
            .join(sock.strip_prefix("/").unwrap());
        assert!(relative.exists() && relative.is_relative(), "{relative:?}");
        assert_eq!(own_agent(Some(relative.as_os_str())), None);
        // Another account's socket (the system's bus, owned by root, where
        // there is one): not ours.
        let system = Path::new("/run/dbus/system_bus_socket");
        if system.exists() && own_uid() != 0 {
            assert_eq!(own_agent(Some(system.as_os_str())), None);
        }
        let file = dir.path().join("plain");
        std::fs::write(&file, "x").unwrap();
        for bad in [
            file.as_os_str(),
            "agent.sock".as_ref(),
            "/nonexistent/agent.sock".as_ref(),
            "".as_ref(),
        ] {
            assert_eq!(own_agent(Some(bad)), None, "{bad:?}");
        }
        assert_eq!(own_agent(None), None);
    }

    #[test]
    fn this_executable_is_a_file_on_disk() {
        let exe = this_executable().unwrap();
        assert!(exe.is_file(), "{exe:?}");
    }
}
