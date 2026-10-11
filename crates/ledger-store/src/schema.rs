//! SQLite schema and migrations.
//!
//! The projection schema. Amounts are TEXT (decimal strings) — never REAL.
//! Migrations are numbered SQL scripts applied in order; `SCHEMA_VERSION`
//! is the current version.

/// Current schema version.
pub const SCHEMA_VERSION: u32 = 1;

/// Migrations, in order. `MIGRATIONS[i]` migrates from version `i` to `i+1`.
pub const MIGRATIONS: &[&str] = &[r#"
CREATE TABLE IF NOT EXISTS meta(
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS accounts(
  id INTEGER PRIMARY KEY,
  code TEXT UNIQUE NOT NULL,
  name TEXT NOT NULL,
  type TEXT NOT NULL,
  parent INTEGER NULL,
  placeholder INTEGER NOT NULL,
  currency TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS entries(
  id INTEGER PRIMARY KEY,
  date TEXT NOT NULL,
  description TEXT NOT NULL,
  status TEXT NOT NULL,
  reversal_of INTEGER NULL
);
CREATE TABLE IF NOT EXISTS lines(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  entry_id INTEGER NOT NULL,
  account_id INTEGER NOT NULL,
  side TEXT NOT NULL,
  amount TEXT NOT NULL,
  currency TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS blobs(
  sha256 TEXT PRIMARY KEY,
  size_bytes INTEGER NOT NULL,
  mime_type TEXT NOT NULL,
  original_name TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS attachments(
  id INTEGER PRIMARY KEY,
  blob_sha TEXT NOT NULL,
  entry_id INTEGER NULL,
  category TEXT NOT NULL,
  description TEXT NULL,
  created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS events(
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  type TEXT NOT NULL,
  payload TEXT NOT NULL,
  created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_lines_entry ON lines(entry_id);
CREATE INDEX IF NOT EXISTS idx_entries_date ON entries(date);
CREATE INDEX IF NOT EXISTS idx_attachments_entry ON attachments(entry_id);
"#];
