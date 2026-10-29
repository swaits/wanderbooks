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

The agent path was Linux-only because of the keyring. Step 0 adds the
macOS login Keychain and the Windows Credential Manager behind
`ledger-host/src/tokens.rs::Keeper`; CI (`portable` job) is the evidence.
Beyond the keyring, the Windows host is a port, not a `cfg` gap. Today
Unix-only, found by grep (2026-10-29):

- `ledger-host`: `bridge`, `host`, `start`, `opener` and others are
  `#[cfg(unix)]` modules; `host.rs`/`start.rs` use `setsid`, `umask`,
  `close_range`, `geteuid`, Unix sockets and `libc` unguarded;
  `commands.rs` calls `isatty`/`tcgetpgrp`.
- `ledger-mcp`: the owner-only socket check reads peer credentials
  (`SO_PEERCRED`, `getpeereid`); `admin`/`local` listen on Unix sockets.
  Windows needs named pipes with a peer SID check (plan 3.9).
- `ledger-store`: `ops.rs` takes `flock` locks; `root.rs` has no owner-only
  DACL (`set_mode` is a no-op off Unix) and the data root falls back to
  `%USERPROFILE%`, not the known-folder `%LOCALAPPDATA%`; `refuse.rs`
  (seccomp) and `renameat2` are Linux-only (macOS needs its own
  equivalents or `cfg` fences).
- The `portable` job runs Windows as allowed-to-fail until that port lands.
