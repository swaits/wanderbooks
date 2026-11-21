# Claims: packaging

Wanderbooks' own product rules for how the program is installed, named,
updated and removed on Windows, macOS and Linux (plan P15 unit C1,
`plan/p15-app-and-relay.md`). No authority outside this product states them.
They are `WB-pkg-<n>`.

Rules: none from bookspec's list; these are product rules only.

---

### Product rules

**WB-pkg-1 It installs like a normal program, and taking it away never takes
the books.** A person downloads one file and opens it. On Windows that is an
installer that asks for no administrator password, installs under their own
profile, adds a Start Menu entry named Wanderbooks with the icon, and lists
an uninstaller in Settings. On macOS it is a disk image holding the app and a
shortcut to Applications to drag it onto. On Linux it is the package manager's
package, or one AppImage file. The Windows uninstaller removes exactly the
files the installer put in its own folder, the Start Menu entry and two
registry keys of its own: it uses no recursive removal and no wildcard and
names the data folder nowhere, and it asks the person to quit the app first
rather than fail on a program that is running. Moving the macOS app to the
Trash, or removing a Linux package, never runs a script. Each leaves the
person's books, keys, backups and settings where they are (WB-pkg-8).
*Enforced:* `deploy/packaging/windows/wanderbooks.nsi`,
`deploy/packaging/macos/build-macos.sh`, `deploy/packaging/packager.toml`.
*Checked:* `ledger-cli` `tests/packaging.rs`
(`the_windows_installer_is_per_user_and_installs_one_program`,
`the_uninstaller_never_removes_the_books`,
`no_package_runs_a_script_that_could_touch_the_books`,
`the_installer_script_compiles_when_nsis_is_here`,
`the_macos_bundle_has_one_program_and_a_plist_that_agrees_with_the_scripts`);
the release workflow's Windows job installs the installer, installs it over
itself and uninstalls it with a marker in the data folder that must survive.

**WB-pkg-2 One executable is the window, the books server and the
assistants' bridge, and the path an assistant is given does not change.** The
installed program is one file, `wanderbooks`, started three ways: with
nothing to say, as the window; by the window with `server`, as its child; and
by an assistant with `mcp --book`, as the bridge. No second program is
installed. The path an assistant is registered with is the installed file's own:
`%LOCALAPPDATA%\Programs\Wanderbooks\wanderbooks.exe` on Windows, which the
installer replaces in place; `/Applications/Wanderbooks.app/Contents/MacOS/wanderbooks`
on macOS, which dragging a newer app over the old one replaces; `/usr/bin/wanderbooks`
for a Linux package, replaced by the package manager; and the AppImage's own
file, which the person keeps where they put it. On macOS a program running from
the temporary copy the system makes of a quarantined download still refuses to
register an assistant, in words, since that path goes away. On Windows the file
is built for the GUI subsystem, so starting it from the Start Menu opens no
console window; run from a terminal it attaches to that terminal so a
command's words appear there, and it never replaces a pipe an assistant gave
it. The two console programs the window starts (the assistant's own list and
the system's opener) are started without a console window.
*Enforced:* `ledger_host::start::translocation_refusal`,
`ledger_host::start::this_executable`, `crates/ledger-cli/src/console.rs` (the Windows
console), `ledger_host::app_server` (the server child).
*Checked:* `ledger-cli` `tests/one_executable.rs`
(`each_mode_names_itself_in_its_usage`), `tests/translocation.rs`
(`translocated_app_refuses_to_connect`),
`tests/packaging.rs` (`the_windows_program_opens_no_console_window_and_still_speaks_in_a_terminal`,
`the_macos_bundle_has_one_program_and_a_plist_that_agrees_with_the_scripts`);
the release workflow's Windows job checks the built file's subsystem is the GUI one
and that it still prints through a pipe.

**WB-pkg-3 An update is the newer package installed over the old one, and the
app says when one exists.** The program installs nothing by itself. Once a day
at most, on a thread of its own, it reads one small public file a release
publishes and takes the version number from it and nothing else: where to get
the update is an address fixed in the program, never one the file names. A
higher version shows a notice in the window with a button to the download page,
and says the books, keys and connections stay as they are, because they live in
the data folder and the bridge's path is the same (WB-pkg-2). "Check for
updates" looks at once. The look sends no version, company or identifier and
a failed look is silent. A macOS app is never updated one file at a time
(that would break its signature): the program treats it as managed and says to
drag the newer app over the old one. Where the program may replace itself with
a release signed by our key (an AppImage, a program put somewhere by hand), the
earlier staged update still works; no release key exists yet, so today none does.
*Enforced:* `ledger_host::newer`, `ledger_host::update::install_kind`,
`ledger_host::commands::check_for_updates`, `ledger_gui::owner::NewerVersion`.
*Checked:* `ledger-host` unit tests in `newer.rs`
(`only_a_higher_plain_version_is_newer`,
`the_feed_gives_a_version_and_nothing_else`,
`a_look_is_noted_and_the_next_is_a_day_away`,
`a_failed_look_says_nothing_and_still_counts`, `the_note_is_private`) and
`update.rs` (`a_macos_app_is_never_updated_in_place`); `ledger-gui`
`tests/update_notice.rs` (`a_newer_version_is_told_with_where_to_get_it`).

**WB-pkg-4 Without git, backups are off and the person is told how to get
it.** Backups run the system's git; the program carries none. With no git found, a
backup says so in plain words for the system it runs on (Git for Windows or winget; Apple's
command line tools or Homebrew; the package manager) and the books work as
ever; the next backup looks again, so installing git needs no restart. On a Mac
without the developer tools, the stub at the system's git path is not taken for a
git. Whether to ship a git with the app is the owner's decision, since git is
GPL-2.0-only free software and shipping it carries a source offer and notices.
*Enforced:* `ledger_store::backup::how_to_get_git`,
`ledger_store::backup::Git::find`.
*Checked:* `ledger-store` unit tests in `backup.rs`
(`a_missing_git_says_how_to_get_it`,
`the_mac_stub_is_not_a_git_until_the_developer_tools_are_there`);
`ledger-cli` `tests/packaging.rs`
(`the_linux_packages_depend_on_git_and_none_carries_one`).

**WB-pkg-5 The Connect page can write a one-click Claude desktop extension
for a company.** The "Claude desktop app" row has one button. It makes the
company's assistant key if there is none and writes
`wanderbooks-<id>.mcpb` in a folder named Extensions in Documents/Wanderbooks: an
MCP Bundle (Anthropic's extension format, a zip file with a manifest, manifest
version 0.3) whose server is a binary that runs this program's installed path
as `mcp --book <id>` (WB-pkg-2). The package carries no program, no key, no
passphrase and no path of the data root. The page then says in words where in Claude's
settings to pick the file (Settings, Extensions, Advanced settings, Install
Extension), to keep Wanderbooks open, and offers to show the file. A program
running from a translocated copy writes nothing and says to move it first.
*Enforced:* `ledger_host::extension::claude_desktop_extension`,
`ledger_host::extension::manifest`.
*Checked:* `ledger-cli` `tests/extension.rs`
(`the_manifest_is_one_the_published_schema_accepts`,
`every_file_the_manifest_names_is_in_the_package`,
`the_server_runs_this_programs_bridge_for_the_company`); `ledger-host` unit
tests in `extension.rs`
(`no_secret_and_no_data_root_is_in_the_package`); `ledger-gui`
`tests/assistants.rs`
(`the_claude_desktop_row_makes_the_extension_and_says_what_to_do_next`).

**WB-pkg-6 One version, and the product is named Wanderbooks wherever a
person sees it.** The version is written once, in the program's own manifest
(`crates/ledger-cli/Cargo.toml`); every package script and the release workflow read it
through `deploy/packaging/version.sh`, which refuses anything that is not three
numbers, and the program reports it to the window, the update look and the
extension. The window's title, the Start Menu entry, the app bundle's name,
the uninstaller's entry and the installer are all "Wanderbooks". The
lower-case `wanderbooks` stays where it is an identifier a person does not
read: the program's file name, the window's app id, the data folder.
*Enforced:* `deploy/packaging/version.sh`, `ledger_host::program_version`.
*Checked:* `ledger-cli` `tests/packaging.rs`
(`the_version_is_written_once_and_every_script_reads_it_from_there`,
`a_version_that_is_not_three_numbers_is_refused`,
`the_desktop_entries_launch_plain_wanderbooks_with_the_icon_and_the_window_id`);
`ledger-gui` `tests/tabs.rs` (the window title tests).

**WB-pkg-7 A release is a draft that carries the licences and the sums.**
The release workflow runs on a version tag or by hand and never on an
ordinary push. It builds all three packages, runs each system's tests on that
system, and attaches the packages, the licence files, the third-party
notices (every package also carries them), the update note and a file of
SHA-256 sums to a draft release. It never publishes a release, never changes
one already published, and never pushes the apt or dnf repositories (a
separate workflow, by hand, from a published release does). Signing runs only
when the owner's secrets are present and says so in the run's notices when not.
*Enforced:* `.github/workflows/release.yml`,
`.github/workflows/publish-repos.yml`.
*Checked:* `ledger-cli` `tests/packaging.rs`
(`the_release_workflow_makes_a_draft_of_all_three_packages_and_publishes_nothing`,
`only_a_person_runs_the_workflow_that_publishes_the_repositories`,
`every_package_carries_the_licences_and_the_icon`).

**WB-pkg-8 The books live where each system keeps an app's own data, apart
from the program.** macOS: Library/Application Support/wanderbooks in the person's
home. Windows: the local application data folder, `wanderbooks` (never the
roaming one: the books are large, hold locks and belong to one machine).
Linux: the XDG data folder, `wanderbooks`, where the books have always been
(unchanged). The folder with the private keys and certificate for the web
address is owner-only on each system, as the data root is. No data folder is
inside any folder a program is installed to.
*Enforced:* `ledger_store::root::data_dir_for`,
`ledger_store::root::default_root`, `ledger_link::private::make_dir`.
*Checked:* `ledger-store` `tests/data_locations.rs`
(`each_system_keeps_its_data_where_it_keeps_an_apps_data`,
`no_data_folder_is_inside_an_install_folder`); `ledger-link` unit tests in
`private.rs` (`a_private_file_reads_back_and_is_mode_0600`,
`a_private_file_and_its_folder_are_owner_only_on_windows`).
