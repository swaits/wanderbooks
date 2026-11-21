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
| `.dmg` | `deploy/packaging/macos/build-macos.sh` (P15 C1; it replaced the cargo-packager `dmg` config, unrun) | `Wanderbooks.app` dragged to Applications; one universal program at `Contents/MacOS/wanderbooks`; no admin password | the newer app dragged over the old (the program tells the person one exists) | hardened runtime; Developer ID and notarization when the owner's secrets are in the release workflow, else ad hoc |
| Windows `.exe` | `deploy/packaging/windows/build-windows.sh` with NSIS (P15 C1; not cargo-packager's `nsis`) | `%LOCALAPPDATA%\Programs\Wanderbooks\wanderbooks.exe`, Start Menu entry, Settings > Apps uninstaller; per user, no admin | the newer installer run over the old one (the program tells the person one exists) | Authenticode when the owner's certificate secrets are in the release workflow, else unsigned |

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
7. **The AppImage updates itself, and (since step 4) the books it holds
   restart onto the new image.** `update` replaces `$APPIMAGE`. The host
   that is open used to run from the old image's mount
   (`/tmp/.mount_*/usr/bin/wanderbooks`), a path that does not change, so
   it never saw a replaced file. The host now reads `APPIMAGE` (it is in
   the allow-list of `start.rs`'s scrubbed environment, only when it names
   an absolute file; a path the host reads about itself and never tells an
   agent), runs and restarts from that file, and tells a replaced program
   by its inode, size and last write. Tested without FUSE by
   `a_replaced_appimage_restarts_the_books` (the programs run from one
   copy while `APPIMAGE` names another, as a mount does); not run against
   a real image mount in this wave.

   **That reading trusted the variable alone, and it was wrong**
   (found 2026-11-10, when the gate ran in a session started from another
   app's AppImage). Every program an AppImage app starts inherits
   `APPIMAGE` and `APPDIR`, so a `wanderbooks` run from such an app's
   terminal or agent took that app's image for its own: it started that
   image as the host ("the books server keeps failing to start"), would
   have named it in an agent's pointer file, and `wanderbooks update`
   replaced that app's file with wanderbooks. The image is now believed
   only when the running executable is inside `APPDIR` (`start.rs`
   INVARIANT 6; `an_appimage_is_ours_only_when_the_program_runs_from_its_mount`,
   `another_apps_appimage_is_never_run_as_the_host`,
   `an_update_never_replaces_another_apps_appimage`). Still not run
   against a real image mount: the first trial of a built AppImage checks
   that `update` and a restart find the image there.
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

## The macOS bundle (step 6; REPLACED by P15 unit C1, see "P15 unit C1" below)

`deploy/packaging/packager-macos.toml` (cargo-packager: the `dmg` format,
which makes the `.app` first; the icon is the 512 px PNG, made into an
`.icns` by the packager; `minimum-system-version` 11.0), `Info.plist`
(merged into the generated one: one viewer at a time, no discrete GPU),
`build-macos.sh` (the same shape as the Linux script, in portable shell
because macOS has BSD `sed`) and `just package-macos`. The `macos` job of
`.github/workflows/release.yml` runs on `macos-latest` (Apple silicon: the
manifest key is `macos-aarch64`; an Intel build is not made), uploads the
`.dmg` as a run artifact (named `macos-dmg-unsigned`) and, on a tag,
attaches it to the release. **It is not signed or notarized** (step 5: the
owner's Developer ID and Q-4), so Gatekeeper refuses it on a Mac that
downloaded it; the owner opens it with right-click, Open. What was checked
here, on Linux: the config parses under cargo-packager 0.11.8's strict
schema (unknown keys are errors) and the script's `sed` is the portable
subset. **Not checked: that the `.dmg` builds, mounts or runs**: that needs
the CI run (Actions, release, Run workflow, this branch) and then a Mac.
Open items for the trial on a Mac: whether `wanderbooks update` replacing
the program inside a signed `.app` breaks its signature (it will: step 5
must decide, probably by updating a whole `.app`). **Translocation
(P11a step 6):** a `.dmg` downloaded from the internet keeps its
quarantine bit, so Gatekeeper may run the app from a read-only copy under
`/private/var/folders/…/AppTranslocation/…` that goes away when it
closes. `wanderbooks connect` (and the viewer's Connect, which is the same
function) refuses there, before a token is made or an agent's list is
touched, and says to move the app out of Downloads and open it again:
`ledger_host::start::translocation_refusal` reads the path the program
would give the agent. Checked on Linux by standing in for that path
(`translocated_app_refuses_to_connect`, `only_a_translocated_path_refuses_an_agent`);
the real Gatekeeper copy still needs the trial on a Mac.

## Updates staged by the host (step 4)

The host looks at the feed at most once a day while the books are open
(`update_check`), and downloads and verifies a newer release into
`<root>/updates/staged/` with a `meta.json` (version, sha256, signature,
notes). It never swaps. The next opener that starts a host applies it
first; `wanderbooks update` applies it too and reuses the staged copy;
`update --check` says "already downloaded". A local release is named by
`<root>/updates/feed` (a `file://` address, then the key file). Without a
compiled-in release key and without that file, the look does nothing but
log why.

The owner accepts a staged update **in the app** (P11a step 4). The
viewer reads `<root>/updates/staged/meta.json` every few seconds
(`Owner::update_waiting`) and shows a standing notice above the page,
"Version X of wanderbooks is ready", with "Update now"; Books also has
"Check for updates" (`Owner::check_for_updates`, which looks at the feed
at once whatever the daily look did) and "Update to X" while one waits.
Accepting sends the host's lease intent `apply_update`; the host installs
the staged copy exactly as `wanderbooks update` does and its existing
watch for a replaced program (host.rs INVARIANT 7) restarts onto it once
no call is running, the books staying open and every holder's session
carrying on. With no host running there is no call to cut off, so the
viewer applies it itself. `books.start` tells an agent the same thing
(`update_waiting`) and adds a next step to tell the human; no tool can
accept it. The accepted update replaces the program file, not a whole
`.app`, which step 5 must revisit for a signed bundle.

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
- Windows file semantics the port met (P11c): an `append(true)` handle has
  no right to truncate, so the chain log and the journal cut a torn tail
  or roll back a failed write through a short-lived write handle
  (`chain::truncate`); a held `LockFileEx` lock is mandatory, so locks sit
  on dedicated `*.lock` files and tests never read those; git refuses the
  `\\?\` path `canonicalize` returns (`root::plain_path`). std opens every
  file with read, write and delete sharing, so a rename never meets an
  open handle of ours.
- The `portable` job runs Windows as a gate (`experimental: false`), since the port landed (P11c).


## P15 unit C1: the installed app on the new shape (2026-11-21)

Written on Linux. **Nothing here has been run on Windows or macOS**: the
`release` workflow's first run on GitHub is the evidence, and it is built to
fail loudly at each step. What Linux checked: `crates/ledger-cli/tests/packaging.rs`
reads the installer script, the plist, the entitlements, the desktop entries and
the workflow; builds the macOS bundle with a stand-in program; compiles the
installer with `makensis` when it is installed; and a real `.deb` and `.rpm`
were built with a stand-in program and their file lists read (program, desktop
entry, icon, licences).

| | Windows | macOS | Linux |
|---|---|---|---|
| Package | NSIS installer, per user | `.app` in a `.dmg`, one universal program | `.deb`, `.rpm`, pacman tarball, AppImage (unchanged tools) |
| Program path (the bridge's) | `%LOCALAPPDATA%\Programs\Wanderbooks\wanderbooks.exe` | `/Applications/Wanderbooks.app/Contents/MacOS/wanderbooks` | `/usr/bin/wanderbooks`; the AppImage's own file |
| Books | `%LOCALAPPDATA%\wanderbooks` | `~/Library/Application Support/wanderbooks` | `~/.local/share/wanderbooks` (unchanged) |
| Removal leaves the books | the uninstaller names no data folder | dragging to Trash runs nothing | no package script runs |
| No console window | built for the GUI subsystem; attaches to a terminal for commands | n/a | n/a |
| Without git | says how to get Git for Windows | says `xcode-select --install`; the system's stub is not taken for git | says the package manager's command |

**Why NSIS written by hand, not cargo-packager's.** The installer's rules
(per user, one program, an uninstaller that can never reach the books) are
short and exact; a script of our own states them in a file a test can read, and
needs only `makensis` (free, zlib-licensed, `choco install nsis`). cargo-packager's
NSIS template downloads helper plugins at build time and ships an uninstaller
option that removes application data. **Why the macOS bundle is assembled by a
script:** signing order (inside out, hardened runtime, then the disk image, then
notarization) and the entitlements are exact, and the secrets contract is ours.
cargo-packager stays for Linux.

**Updating** is installing the newer package over the old one; the program says
when one exists (`ledger_host::newer`: a version number from `latest.json`,
once a day, a fixed download address) and never installs anything. The earlier
staged self-update (`update`, `update.lock`, the daily staged download) still
exists for an AppImage or a program put somewhere by hand and needs a release
key that does not exist yet; on the desktop the detached host it restarted
through is gone (P15 A5). A macOS `.app` is never updated one file at a time.

**Signing.** Windows (secrets `WINDOWS_CERTIFICATE_PFX_BASE64`,
`WINDOWS_CERTIFICATE_PASSWORD`): `sign-windows.ps1` signs the program, then the
installer. macOS (`APPLE_CERTIFICATE_P12_BASE64`, `APPLE_CERTIFICATE_PASSWORD`,
`APPLE_SIGNING_IDENTITY`; to notarize `APPLE_ID`, `APPLE_TEAM_ID`,
`APPLE_APP_SPECIFIC_PASSWORD`): `import-certificate.sh` puts the certificate in
a temporary keychain, `build-macos.sh` signs with the hardened runtime,
notarizes and staples. With none, each step prints a notice and the package is
unsigned (macOS: ad hoc, so it runs, but Gatekeeper refuses a download).

**The Claude desktop extension** (`ledger_host::extension`): an MCP Bundle
(`.mcpb`, the renamed `.dxt`; manifest version 0.3), one per company, made by the
Connect page's button; it runs the installed program as `mcp --book <id>`.

**Left out, on purpose:** an auto-installer; a Windows executable icon resource
(the shortcut and the uninstaller entry carry the icon, and the running window
sets its own from the 512 px PNG; Explorer shows the `.exe` with the default
icon);
`wanderbooks.cmd`/PATH setup so a terminal waits for a command on Windows (a
GUI-subsystem program returns the prompt at once); a macOS refusal for a program
run from a mounted disk image (a path under `/Volumes` may be a real install on
another disk); bundling git on Windows and macOS (the owner's decision, below); the signed `manifest.json` of the earlier self-update
(needs the release key, Q-4).

### Bundling git (an owner decision, not made here)

Backups run the system's git. **Linux:** the packages now depend on git, so the
package manager fetches it (nothing is bundled). **Windows and macOS:** the app
detects a missing git and says how to get it (WB-pkg-4); nothing is bundled.

What bundling would mean. Git is GPL-2.0-only free software. Shipping an
unmodified git as a separate file that our program starts as its own process
(never linked) is what the GPL calls aggregation: our MIT OR Apache-2.0 code is
unaffected, and AGENTS.md's clean-room rule (never copy GnuCash's code) is not
touched, since nothing of git is copied into ours; `deny.toml` governs Rust
crates only and does not see it. It still requires, for each release that
carries git: git's GPL text and copyright notices beside it (Git for Windows'
MinGit adds the notices of the components it carries: OpenSSL/curl, the MSYS2
runtime); the corresponding source of exactly that git, or a written offer for it
valid three years (GPLv2 section 3), most simply the git version's source
archive attached to the same release; no term of ours that forbids the
recipient what the GPL permits; and a note of any change if git were modified.
`THIRD-PARTY-NOTICES.txt` (cargo-about) covers Rust crates only, so it would
need an addition. On Windows the candidate is MinGit from Git for Windows
(about 40 MB, unpacked beside the program; `Git::find` would look there first);
on macOS Apple's command line tools' git is Apple's and cannot be bundled, so
it would be a build of our own (Homebrew's, or from source), universal, signed
inside the app, with the same obligations.
