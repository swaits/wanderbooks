# Backups

- The books back themselves up: a little after each change, when they open, before they close. Each backup goes to the targets the settings name: backup.git_remote, a private repository the human owns, and backup.folder, a folder on another disk. Both is best. The backups aren't encrypted: the human's repository host and disk keep them private.
- No targets set: books.start says so. Ask the human for the repository's address and the folder, then set them with books.set_setting (who decided, and when); choosing where backups go needs books:admin, as backing up does.
- books.backup says how each target stands: when it last took the books, through which journal seq, why the last try failed, and whether its weekly restore check passed. A target that is stale, failing, or failed its check: tell the human what it says, in plain words (a disk not connected, a repository that refused).
- books.back_up_now backs up at once, for instance before the human changes computers. A backup never changes the books.
- A backup folder that took these books' backups but holds none now is a disk not connected: ask the human to connect it. Only if they say the disk is new, back up now with new_disk: true to begin the books' backup there again.
- Books restored as a copy of other books (under another name) don't back up to those books' repository (that would stop theirs): books.backup says so; ask the human for one of their own.
- Restoring is the human's to do, outside these tools: a backup comes back as new books, or replacing these (the old books are kept, never deleted), and each is proven whole before it is restored.
