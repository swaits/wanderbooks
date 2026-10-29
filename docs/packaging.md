# Packaging and updates: the trial (P11a step 1)

Status 2026-10-29: **the Linux half of the trial is run** (on the Arch
desktop that builds the project, with Debian, Fedora and Arch containers
for the installs); **the macOS and Windows halves are not**: they need
those machines (and for the `.dmg` and the NSIS `.exe`, the signing
identities of step 5), and this machine has neither a macOS nor a Windows
target. `cargo-packager` 0.11.8 and `cargo-generate-rpm` 0.21.1 are pinned in
`mise.toml`; `cargo-packager-updater` is **not** pinned: it is a library
(the program checks releases itself, `crates/ledger-host/src/update.rs`),
not a tool, so there is nothing for a recipe to call.

| format | built by | installs as | updated by | signed by |
|---|---|---|---|---|
| `.deb` | `cargo packager` (`deb`) | `/usr/bin/wanderbooks`, a menu entry, `/usr/share/wanderbooks/install-method` = `deb`; 47 MB | apt repository (`repo-indexes.sh apt`, GitHub Pages) | unsigned until step 5 (the repository's index is `trusted=yes` in the test) |
| `.rpm` | `cargo-generate-rpm` (table in `crates/ledger-cli/Cargo.toml`) | same paths, marker `rpm`; 36 MB | dnf repository (`repo-indexes.sh dnf`) | unsigned until step 5 (`--signing-key` takes the PGP key then) |
| AUR `PKGBUILD` | `cargo packager` makes the tarball (`pacman` format) and a PKGBUILD for it; ours is `deploy/packaging/aur/PKGBUILD`, which repacks the release's tarball (package `wanderbooks-bin`) | same paths, marker `aur`; built with `makepkg` | pacman (the AUR keeps old PKGBUILDs) | n/a (pacman trusts the AUR's checksums) |
| AppImage | `cargo packager` (`appimage`); 44 MB | one file the owner puts anywhere and runs; needs no admin password | `wanderbooks update` (`$APPIMAGE` is the file replaced) against the manifest; minisign | minisign by the release key (step 5) |
| `.dmg` | `cargo packager` (`dmg`) | (trial) needs a Mac | `wanderbooks update` | Developer ID and notarization (step 5) |
| NSIS `.exe` | `cargo packager` (`nsis`) | (trial) needs Windows, and native Windows is unit P11c (D-62) | `wanderbooks update` | the Windows identity (step 5) |

`just package-linux` builds the first four (`deploy/packaging/build-linux.sh`,
about a minute after the release build, itself 8 to 12 minutes on a
loaded 16-core machine); `just test-linux-packages` is the install-and-upgrade test.

## Linux results (2026-10-29)

What the trial found, each of which is built around now:

1. **A program linked here does not start on Debian.** This machine's
   glibc is 2.44; the binary asks for `GLIBC_2.43` and Debian 13 (trixie)
   has 2.41. A package must be built on the oldest system it claims.
   `.github/workflows/release.yml` builds on `ubuntu-24.04` (glibc 2.39);
   `just release-trixie` builds in `rust:1.99-trixie` for the local test. The
   claim is therefore "Debian 13, Ubuntu 24.04 and newer, current Fedora and
   Arch"; Debian 12 and Ubuntu 22.04 are not claimed (they would need a
   build container of their own).
2. **The books refuse to run under a seccomp filter** (`host.rs`
   INVARIANT 2), and Docker's default profile is one: in a container the
   host fails to start ("refusing to run under a seccomp filter"). The
   tests run containers with `--security-opt seccomp=unconfined`. Anyone
   running the books in Docker meets this; it is not a bug of the packages.
3. **An AppImage's launcher runs the desktop file's `Exec` line and appends
   the arguments it was given.** With `Exec=wanderbooks gui`,
   `Wanderbooks.AppImage update` ran `wanderbooks gui update`. The desktop
   files say plain `Exec=wanderbooks`, and a launch with nothing to say and
   no terminal is the viewer (`main.rs::is_bare_launch`); at a terminal
   it is still the usage. So the AppImage is also the command line:
   `Wanderbooks.AppImage update`, `... mcp --book acme`.
4. **cargo-packager's configuration** (0.11.8): paths in a config are
   relative to the *config file*, not the working directory (the build script
   writes an absolute-path copy); `files` maps *source to destination*;
   `category` is a fixed list (`Finance`); the AppImage needs a square icon
   (`deploy/packaging/icons/512x512.png`, **a placeholder the owner should
   replace**); the `pacman` format yields the tarball and PKGBUILD but not a
   package (that is `makepkg`'s); its `.deb` has `Depends` from the config.
5. **The AppImage tools are downloaded when it is built** (`AppRun`,
   `linuxdeploy` and its AppImage plugin from tauri-apps' and linuxdeploy's
   GitHub releases, unverified by cargo-packager): a supply-chain edge of
   the release build, not of the installed app. They run with
   `APPIMAGE_EXTRACT_AND_RUN=1`, so the build needs no FUSE.
6. **`cargo packager signer` signs in a format `update` verifies.** Its
   key and its `.sig` are the base64 of minisign's two-line text; the
   manifest's `signature` is that `.sig` as written, and `update` reads the
   public key in either form. Checked end to end by hand (below), and by
   `a_public_key_is_read_in_every_form_minisign_and_the_packager_write_it`.
7. **The AppImage updates itself but the books it holds do not restart
   onto the new image.** `update` replaces `$APPIMAGE`, and an AppImage run
   from a shell (`Wanderbooks.AppImage update`, or the one that was running)
   proves the swap and the rollback: after `update`, the AppImage on disk
   is the new file, the old one is kept, and `rollback` restores it. But the
   host that is open runs from the old image's mount
   (`/tmp/.mount_*/usr/bin/wanderbooks`), a path that does not change, so
   `host.rs` never sees a replaced file and the open books keep the old
   version until they are closed and opened again (`wanderbooks close`).
   Fixing it means the host learning the image's path (its environment is
   scrubbed on purpose; `APPIMAGE` would join the allow-list of `start.rs`,
   or the path would come from the mount table) and starting its successor
   from it: a decision about what the host may trust, left to the EM (the
   unit's report names it). Package installs and a hand-placed executable
   restart as designed (`bridge_survives_update`).
8. **Onboarding with no terminal does not exist yet.** The unit's Done-when
   (1) says an AppImage "opens, onboards and posts an entry with no
   terminal". The AppImage opens (a bare launch is the viewer), but the viewer
   opens only an existing book (`ledger_gui::book_in_root`), and a data root
   and a first book come from `wanderbooks setup` and `books create`, which
   need a terminal. A first-run path in the viewer ("New books…" is in its
   menu, but the menu is not reachable until a book is open) is not in P11a's
   steps. It is the gap between this wave's AppImage and Done-when (1).

By hand, on this machine, against a throwaway release folder made by `just
local-release` (a key from `cargo packager signer generate`; the feed
`file://…/manifest.json`; a scratch home made by `wanderbooks setup --home`,
nothing of the owner's):

```text
Wanderbooks.AppImage --help                       -> the usage (FUSE mount, and with APPIMAGE_EXTRACT_AND_RUN=1)
Wanderbooks.AppImage setup --home H               -> the private folder; books create acme -> books list: "acme  open"
Wanderbooks.AppImage update --check --feed F --trusted-key K
                                                  -> "wanderbooks 0.2.0 is available (this is 0.1.0)"
Wanderbooks.AppImage update --feed F --trusted-key K
                                                  -> "Updated wanderbooks from 0.1.0 to 0.2.0."; the file's sha256
                                                     is the release's; updates/previous/ holds the 0.1.0 image
Wanderbooks.AppImage rollback                     -> "Went back to wanderbooks 0.1.0."; the file is the original again
```

`linux_packages_install_and_upgrade` (`deploy/packaging/test-linux-packages.sh`),
2026-10-29, 0.1.0 then 0.1.1, all three passing: in `debian:trixie-slim`
and `fedora:latest` the package installs from a repository made by
`repo-indexes.sh` (the release workflow runs the same script), in
`archlinux:latest` from `makepkg` over `deploy/packaging/aur/PKGBUILD`; then
books are made as an ordinary user and held open, 0.1.1 is added and
upgraded to by the package manager (`apt-get install --only-upgrade`, `dnf
upgrade`, `pacman -U`), the host restarts onto the new file (a new pid whose
`/proc/PID/exe` is not deleted), the journal's last line (the head) is
byte-for-byte unchanged, and `wanderbooks update` exits non-zero naming the
package manager. Not tested: Ubuntu, openSUSE, a package-manager
*downgrade* (the repositories keep every release by construction), and the
`.rpm` and `.deb` under their real signatures (step 5).

## What `update` and `rollback` need from a release

`manifest.json` at `releases/latest/download/` (D-26):

```json
{ "version": "0.2.0", "notes": "…",
  "platforms": { "linux-x86_64": { "url": "wanderbooks-0.2.0-linux-x86_64",
                                   "sha256": "…hex…", "signature": "…base64 of the .sig file…" } } }
```

`url` is absolute or relative to the manifest; the platform key is
`std::env::consts::OS-ARCH`; the file is the whole program (the AppImage
on Linux). The program checks the version is higher, the sha256, and the
minisign signature against `WANDERBOOKS_RELEASE_PUBKEY` (compiled in by the
release workflow once the owner has a key: **step 5, Q-4**; today no build
carries one, so a build updates only from a `file://` feed with
`--trusted-key`). The two-key release manifest of D-25 would wrap this
(rotation, expiry, a monotone version) and is step 5's.

## What step 0 found about the platforms (read before the macOS and Windows rows)

The agent path was Linux-only because of the keyring; D-63 removed the
keyring on every OS (tokens are 0600 files under the data root, unit U0e),
so nothing about tokens is OS-specific now. The Windows host is a port,
not a `cfg` gap. Today Unix-only, found by grep (2026-10-29):

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
