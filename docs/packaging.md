# Packaging and updates: the trial (P11a step 1)

Status 2026-10-29: **the trial has not been run.** It needs clean Linux,
macOS and Windows machines for the six formats (`.deb`, `.rpm`, AUR,
AppImage, `.dmg`, NSIS `.exe`), and the one machine that built step 0 is a
Linux desktop with neither a macOS nor a Windows target, no container
runtime and none of the packager tools installed. The tools are pinned in
`mise.toml` now (`cargo-packager` 0.11.8, `cargo-packager-updater` 0.2.3,
`cargo-generate-rpm` 0.21.1), so the trial is `mise install` and the VMs.
Nothing below is a result; it is what is known before the trial, and the
table is filled by the person who runs it.

| format | built by | installs as | updated by | signed by |
|---|---|---|---|---|
| `.deb` | `cargo packager` | (trial) | apt repository (step 3) | (trial) |
| `.rpm` | `cargo-generate-rpm` | (trial) | dnf repository (step 3) | (trial) |
| AUR `PKGBUILD` | hand-written | (trial) | the package manager | n/a |
| AppImage | `cargo packager` | (trial) | `cargo-packager-updater` | (trial) |
| `.dmg` | `cargo packager` | (trial) | `cargo-packager-updater` | Developer ID (step 5) |
| NSIS `.exe` | `cargo packager` | (trial) | `cargo-packager-updater` | Windows identity (step 5) |

## What step 0 found about the platforms (read before the macOS and Windows rows)

The agent path was Linux-only because of the keyring; D-63 removed the
keyring on every OS (tokens are 0600 files under the data root, unit U0e),
so nothing about tokens is OS-specific now. The Windows host is a port,
not a `cfg` gap. Today Unix-only, found by grep (2026-10-29):

- ~~`ledger-host`: `bridge`, `host`, `start`, `opener` and others are
  `#[cfg(unix)]` modules; `host.rs`/`start.rs` use `setsid`, `umask`,
  `close_range`, `geteuid`, Unix sockets and `libc` unguarded;
  `commands.rs` calls `isatty`/`tcgetpgrp`.~~ Landed (P11c): every module
  builds on every OS. The channels are `ledger_mcp::ipc` (below); the host
  starts detached (`DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP`, leaving
  the opener's job if it lets it) where Unix uses `setsid`, with a
  Windows environment list in `start.rs::host_env`; `close_range` and the
  `umask` have nothing to do there (a Windows child inherits no handle it
  is not given); the terminal checks use `IsTerminal`; the stop signals
  are Ctrl+C, Ctrl+Break, console close and shutdown
  (`ipc::stop_signals`); a replaced program is told by its size and last
  write, since a running program cannot be replaced in place.
- ~~`ledger-mcp`: the owner-only socket check reads peer credentials
  (`SO_PEERCRED`, `getpeereid`); `admin`/`local` listen on Unix sockets.
  Windows needs named pipes with a peer SID check (plan 3.9).~~ Landed
  (P11c): `ledger_mcp::ipc` is the one seam. A pipe is
  `\\.\pipe\wanderbooks-<hash of the root's run/ folder>-<name>`, made with
  a DACL of one allow entry (this account's SID), remote clients refused,
  the first instance made once, and 16 idle instances waiting; each end
  checks the other's process (`GetNamedPipeClientProcessId` /
  `GetNamedPipeServerProcessId`, then that process's token user) against
  its own SID. Clients open with `SECURITY_IDENTIFICATION`.
- ~~`ledger-store`: `ops.rs` takes `flock` locks; `root.rs` has no
  owner-only DACL (`set_mode` is a no-op off Unix) and the data root falls
  back to `%USERPROFILE%`, not the known-folder `%LOCALAPPDATA%`;
  `refuse.rs` (seccomp) and `renameat2` are Linux-only (macOS needs its
  own equivalents or `cfg` fences).~~ Landed (P11c): `lock.rs` is the one
  lock seam (std's `try_lock`, `LockFileEx` on Windows); `winsec.rs` sets
  and reads the owner-only DACL and finds the profile and `LocalAppData`
  from the shell's known folders; `rename_new` is `MoveFileExW` without
  the replace flag. `refuse.rs` (seccomp, a test fault injector) and the
  host's `close_range` stay Linux-only on purpose (D-32: nothing here
  confines a process, so nothing is promised where they are absent).
  macOS, which has no `renameat2`, still looks before it moves.
- The `portable` job runs Windows as a gate (`experimental: false`), since the port landed (P11c).
