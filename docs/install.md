# Installing Wanderbooks

For a person who wants to use it. Three short sections, one per system, then
what to do after. The release page has the files:
https://github.com/swaits/wanderbooks/releases/latest

Wanderbooks keeps your books on your own computer. An assistant (Claude,
ChatGPT, Codex and others) can reach them **only while Wanderbooks is open**,
and only the companies you have open in its tabs. When you are finished,
close the window: that quits Wanderbooks fully and closes your books.

## Windows

1. Download `Wanderbooks-…-windows-x64-setup.exe` and double-click it. It asks
   for no administrator password: it installs for you alone.
2. Until the installer is signed, Windows shows "Windows protected your PC".
   Choose **More info**, then **Run anyway**. (Unsigned software always does
   this the first time.)
3. Open **Wanderbooks** from the Start Menu. The first time, it makes your
   books' folder and helps you connect your assistant.
4. To update, run the newer installer over the old one. To remove it, use
   Settings, Apps, Wanderbooks. Neither touches your books.

Where things are: the program is in `%LOCALAPPDATA%\Programs\Wanderbooks`; your
books, keys and backups are in `%LOCALAPPDATA%\wanderbooks`.
Backups need nothing installed: choose a backup folder in Wanderbooks
(another disk, or a folder your cloud drive keeps in sync).

## macOS

1. Download `Wanderbooks-…-macos-universal.dmg` (one file for Apple silicon and
   Intel), open it, and drag **Wanderbooks** onto **Applications**.
2. Open it from Applications, not from the disk image or your Downloads folder
   (an assistant needs the program to stay in one place). Until the app is
   notarized by Apple, the first time: right-click it, choose **Open**, then
   **Open** again.
3. To update, quit Wanderbooks and drag the newer app over the old one. To
   remove it, drag it to the Trash. Neither touches your books.

Your books, keys and backups are in `~/Library/Application Support/wanderbooks`.
Backups need nothing installed: choose a backup folder in Wanderbooks
(another disk, or a folder iCloud, Dropbox, OneDrive or Google Drive keeps in
sync).

## Linux

Use the package for your system from the release page: the `.deb` (Debian,
Ubuntu), the `.rpm` (Fedora), the Arch package, or the AppImage (one file: mark
it executable, keep it where you put it, and run it). Open **Wanderbooks** from
the menu. To update, install the newer package, or replace the AppImage. Your
books are in `~/.local/share/wanderbooks` and are never touched by a package.
Backups need nothing installed: choose a backup folder in Wanderbooks
(another disk, or a folder your sync program keeps).

## After it is installed

Open the **Assistants** page of a company. Pick the assistant you use: Claude
Code and Codex have a button; **Claude desktop app** has a button that makes one
extension file to open in Claude (Settings, Extensions, Advanced settings,
Install Extension); web assistants have an address and the steps. Wanderbooks
tells you in the window when a newer version is out.

`wanderbooks serve` (a server with no window, for another computer) is for
people who run it themselves: see `deploy/README.md`.
