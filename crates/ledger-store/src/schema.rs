//! SQLite schema and migrations.
//!
//! The projection schema. Amounts are TEXT (decimal strings) — never REAL.
//! Migrations are numbered SQL scripts applied in order; `SCHEMA_VERSION`
//! is the current version.

/// Current schema version.
pub const SCHEMA_VERSION: u32 = 4;

/// Migrations, in order. `MIGRATIONS[i]` migrates from version `i` to `i+1`.
pub const MIGRATIONS: &[&str] = &[
    r#"
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
"#,
    r#"
-- v2: fixed-asset register and depreciation runs.
CREATE TABLE IF NOT EXISTS assets(
  id INTEGER PRIMARY KEY,
  description TEXT NOT NULL,
  purchase_date TEXT NOT NULL,
  cost_amount TEXT NOT NULL,
  cost_currency TEXT NOT NULL,
  salvage_amount TEXT NOT NULL,
  salvage_currency TEXT NOT NULL,
  useful_life_months INTEGER NOT NULL,
  method TEXT NOT NULL,
  cost_account INTEGER NOT NULL,
  accum_depr_account INTEGER NOT NULL,
  expense_account INTEGER NOT NULL,
  status TEXT NOT NULL,
  disposed_date TEXT NULL,
  proceeds_amount TEXT NULL
);
CREATE TABLE IF NOT EXISTS asset_depreciation(
  asset_id INTEGER NOT NULL,
  period TEXT NOT NULL,
  entry_id INTEGER NOT NULL,
  amount TEXT NOT NULL,
  PRIMARY KEY (asset_id, period)
);
"#,
    r#"
-- v3: accounting periods, A/R + A/P, cleared-line flags, FX rates, and
-- foreign-currency columns on lines. Amounts are TEXT, never REAL.
CREATE TABLE IF NOT EXISTS periods(
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  start TEXT NOT NULL,
  end TEXT NOT NULL,
  status TEXT NOT NULL,
  closing_entry INTEGER NULL
);
CREATE TABLE IF NOT EXISTS customers(
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS vendors(
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS invoices(
  id INTEGER PRIMARY KEY,
  customer_id INTEGER NOT NULL,
  date TEXT NOT NULL,
  due_date TEXT NOT NULL,
  lines_json TEXT NOT NULL,
  amount_paid TEXT NOT NULL,
  status TEXT NOT NULL,
  entry_id INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS bills(
  id INTEGER PRIMARY KEY,
  vendor_id INTEGER NOT NULL,
  date TEXT NOT NULL,
  due_date TEXT NOT NULL,
  lines_json TEXT NOT NULL,
  amount_paid TEXT NOT NULL,
  status TEXT NOT NULL,
  entry_id INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS cleared_lines(
  entry_id INTEGER NOT NULL,
  line_index INTEGER NOT NULL,
  PRIMARY KEY (entry_id, line_index)
);
CREATE TABLE IF NOT EXISTS fx_rates(
  currency TEXT NOT NULL,
  date TEXT NOT NULL,
  rate TEXT NOT NULL,
  PRIMARY KEY (currency, date)
);
CREATE INDEX IF NOT EXISTS idx_invoices_customer ON invoices(customer_id);
CREATE INDEX IF NOT EXISTS idx_bills_vendor ON bills(vendor_id);
ALTER TABLE lines ADD COLUMN foreign_currency TEXT;
ALTER TABLE lines ADD COLUMN foreign_amount TEXT;
"#,
    r#"
-- v4: classes, payroll runs, and class_id on lines. Amounts are TEXT,
-- never REAL.
CREATE TABLE IF NOT EXISTS classes(
  id INTEGER PRIMARY KEY,
  name TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS payroll_runs(
  id INTEGER PRIMARY KEY,
  date TEXT NOT NULL,
  entry_id INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS payroll_lines(
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  run_id INTEGER NOT NULL,
  employee_name TEXT NOT NULL,
  gross_pay TEXT NOT NULL,
  federal_withholding TEXT NOT NULL,
  state_withholding TEXT NOT NULL,
  ss_employee TEXT NOT NULL,
  medicare_employee TEXT NOT NULL,
  ss_employer TEXT NOT NULL,
  medicare_employer TEXT NOT NULL,
  other_deductions TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_payroll_lines_run ON payroll_lines(run_id);
ALTER TABLE lines ADD COLUMN class_id INTEGER;
"#,
];
