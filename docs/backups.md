# Backups

A backup of your books is an ordinary folder of ordinary files. Nothing needs
to be installed to make one or to read one back, and you never need to know how
it works. This page says where it is, what is in it, and how to get your books
back, on this computer, on a new one, or from a backup an older version made.

## Where it is

You choose the folder once (in the window: **Back up to...**): another disk, a
USB drive, a folder on your network, or a folder your cloud drive keeps in sync
(Dropbox, iCloud Drive, OneDrive, Google Drive). Your books then back
themselves up a little after each change, when they open and before they
close; **Back up now** does it at once. A backup that is more than a day behind
is flagged, and once a week your books prove they can be restored from it.

## What is in it

One folder for your books, inside the folder you chose:

```text
Backups/                          the folder you chose
  acme/                           your books (the name you gave them)
    READ-ME.txt                   a few words about the folder
    manifests/
      000001.json                 one per backup that changed something
      000002.json
    journal/000001-..., 000002-... the journal, in pieces
    access/000001-...             who reached the books, in pieces
    files-index/000001-...        which receipts and files there are
    files/ab/abcdef...            each receipt or file, once
    vault/...  keys/...           your tax-ID vault and its key
```

Files are only ever **added**. Nothing in the folder is changed or deleted by
Wanderbooks, so a sync service never sees a file change under it, and an earlier
state of your books is always still there. A backup counts as complete only
when its small manifest file is in place, written last; an interrupted backup
leaves a few unused files and the last complete backup is still whole. Two
computers backing up to one folder cannot damage it either: whichever writes
the next manifest first wins, and the other goes on from there. If the folder
turns out to hold a different set of books under the same name, or a newer
copy of these than this computer has, Wanderbooks says so and writes nothing.

If your disk is short of room, you may delete the whole folder of your books
(never parts of it) once you have another backup.

## A passphrase (optional)

When you set up backups you may protect them with a passphrase. It is off unless
you turn it on. With one, every file in the folder is sealed (XChaCha20-Poly1305;
the key comes from your passphrase through Argon2id) and the file names say
nothing about your books, only how many files there are and how big. Your
computer keeps the key privately, so backups stay automatic. **A forgotten
passphrase cannot be recovered**, by anyone: write it down somewhere safe.
On another computer, restoring asks for the passphrase.

Changing the passphrase starts a new set of sealed backups beside the old
ones, which stay as they are and stay sealed with the passphrase they were made
with. Turning it off makes later backups plain. If you want old backups gone
after a change, delete their folder yourself.

## One file instead of a folder

**Save a backup...** writes the same backup as a single file ending in
.wbbackup (a zip). Keep it, email it to yourself, or carry it on a USB stick. It
is sealed if your backups are. A file by the same name is never overwritten.

## Getting your books back

**Restore...** takes the backup folder you chose (or the folder of one set of
books inside it), or a .wbbackup file. It checks everything first and shows you
what it found: how far the books go, how many files, whether it needed to skip a
newer backup that was damaged. Only then does it put your books back, as new
books, or in place of books with the same name (those are kept whole in a
`legacy` folder, never deleted).

On a **new computer**: install Wanderbooks, open it, choose **Restore...**, point it
at the backup folder (connect the disk or let your cloud drive finish syncing
first) or at the .wbbackup file, and give the passphrase if it asks. Then choose
a backup folder again to keep backing up.

If the disk your backups were on is not connected, Wanderbooks says so and does
not start over on another disk by itself: connect the disk, or, if it is a new
disk, say so when you back up and it begins there again.

## A backup made with an older version

Older versions kept backups differently, in a way that needed another program
called git. Wanderbooks no longer writes those or reads them, and does not touch
them: they stay exactly where they are. If you still have one and want your books
from it, you can get them back with git yourself:

1. In a terminal, make a plain folder from the old backup (it is the folder named
   after your books, ending in .git, inside the folder you backed up to):

   ```text
   git clone /path/to/backups/acme.git acme-old
   ```

2. Adopt that folder as books (with your vault and its key, which the backup
   also kept), and check what it says:

   ```text
   wanderbooks books adopt acme-old --id acme --vault acme-old/vault --vault-key acme-old/keys/legacy.key
   ```

   It copies the books in, proves them whole, and leaves the original as it was.
   (If your books had a tax-ID vault you did not use, leave out the two vault
   options and add `--no-vault`.)

3. Choose a backup folder for the books you got back, so new backups are made in
   the new form beside the old one.

Your books still list the private repository they used to back up to; it is no
longer used, and you may clear that setting.
