//! SQLite schema and migrations.
//!
//! The projection schema. Amounts are TEXT (decimal strings) — never REAL.
//! Migrations are numbered SQL scripts applied in order; `SCHEMA_VERSION`
//! is the current version.

/// Current schema version.
pub const SCHEMA_VERSION: u32 = 19;

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
    r#"
-- v5: withdrawn attachments. Withdrawal retires a receipt or document from
-- listings and linking; the bytes stay in the blob store and the journal
-- keeps the filing and the withdrawal.
ALTER TABLE attachments ADD COLUMN withdrawn_at TEXT;
ALTER TABLE attachments ADD COLUMN withdrawn_reason TEXT;
"#,
    r#"
-- v6: tax categories on accounts (where each account lands on the return,
-- and how much of it is deductible). Percent is TEXT, never REAL.
ALTER TABLE accounts ADD COLUMN tax_category TEXT;
ALTER TABLE accounts ADD COLUMN deductible_percent TEXT;
"#,
    r#"
-- v7: 1099 tracking: which return each vendor gets, whether its W-9 is on
-- file (never the TIN itself), and the vendor each entry paid.
ALTER TABLE vendors ADD COLUMN form_1099 TEXT;
ALTER TABLE vendors ADD COLUMN tin_on_file INTEGER NOT NULL DEFAULT 0;
CREATE TABLE IF NOT EXISTS entry_vendors(
  entry_id INTEGER PRIMARY KEY,
  vendor_id INTEGER NOT NULL
);
"#,
    r#"
-- v8: each filing keeps its own file name and type. Before, they came from
-- the blob, so filing the same bytes again under another name listed the
-- first name (found by the spec-blind store suite, 2026-10-13). Existing
-- rows take the blob's; a database whose filings differed then fails
-- verify's table check and is rebuilt from the journal.
ALTER TABLE attachments ADD COLUMN file_name TEXT;
ALTER TABLE attachments ADD COLUMN mime_type TEXT;
UPDATE attachments SET
  file_name = (SELECT original_name FROM blobs WHERE sha256 = attachments.blob_sha),
  mime_type = (SELECT mime_type FROM blobs WHERE sha256 = attachments.blob_sha);
"#,
    r#"
-- v9: asset tags, serial numbers, and each asset's acquisition entry. assets.tag is the current tag;
-- asset_tags_log keeps every tagging in journal order, because a tag is
-- never reused by another asset even after a retag.
ALTER TABLE assets ADD COLUMN tag TEXT;
ALTER TABLE assets ADD COLUMN serial TEXT;
ALTER TABLE assets ADD COLUMN acquisition_entry INTEGER;
CREATE TABLE IF NOT EXISTS asset_tags_log(
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  asset_id INTEGER NOT NULL,
  tag TEXT NOT NULL
);
"#,
    r#"
-- v10: each asset's federal tax basis: the rule (cost, carryover,
-- personal_use_conversion) and the contributing member's adjusted basis
-- (TEXT, exact decimal, never REAL).
ALTER TABLE assets ADD COLUMN tax_rule TEXT;
ALTER TABLE assets ADD COLUMN contributor_basis TEXT;
"#,
    r#"
-- v11: attachments link to any subject (docs/parties-and-attachments.md):
-- entries beyond the first, accounts, assets, parties, invoices, bills,
-- periods, payroll runs, schedules, reconciliations, the company. Bank and
-- card statements carry their account, period, and balances; a
-- reconciliation records the statement, its balance, and the lines it
-- cleared. Amounts are TEXT, exact decimal.
CREATE TABLE IF NOT EXISTS attachment_links(
  attachment_id INTEGER NOT NULL,
  subject_kind TEXT NOT NULL,
  subject_id INTEGER NOT NULL,
  PRIMARY KEY(attachment_id, subject_kind, subject_id)
);
CREATE INDEX IF NOT EXISTS idx_attachment_links_subject ON attachment_links(subject_kind, subject_id);
-- Payroll lines name their employee party (Ledger PayrollLine.employee).
ALTER TABLE payroll_lines ADD COLUMN employee_id INTEGER;
CREATE TABLE IF NOT EXISTS statements(
  attachment_id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL,
  period_start TEXT NOT NULL,
  period_end TEXT NOT NULL,
  opening_balance TEXT NOT NULL,
  closing_balance TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS reconciliations(
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL,
  statement_date TEXT NOT NULL,
  statement_balance TEXT NOT NULL,
  attachment_id INTEGER NOT NULL,
  voided_reason TEXT NULL
);
CREATE TABLE IF NOT EXISTS reconciliation_lines(
  reconciliation_id INTEGER NOT NULL,
  entry_id INTEGER NOT NULL,
  line_index INTEGER NOT NULL,
  PRIMARY KEY(reconciliation_id, entry_id, line_index)
);
"#,
    r#"
-- v12: the books' own record of how they are kept (journal format 2,
-- plan/served.md 5.2): settings in force (each value as canonical JSON,
-- with the seq and source that set it), conventions (the current
-- revision of each, and its retirement), decisions, open items, and
-- approvals of draft entries.
CREATE TABLE IF NOT EXISTS settings(
  key TEXT PRIMARY KEY,
  value_json TEXT NOT NULL,
  seq INTEGER NOT NULL,
  source_json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS conventions(
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  title TEXT NOT NULL,
  body_md TEXT NOT NULL,
  structured_json TEXT NULL,
  source_json TEXT NOT NULL,
  seq INTEGER NOT NULL,
  revision INTEGER NOT NULL,
  retired_reason TEXT NULL,
  retired_source_json TEXT NULL,
  retired_seq INTEGER NULL
);
CREATE TABLE IF NOT EXISTS decisions(
  id INTEGER PRIMARY KEY,
  subject TEXT NOT NULL,
  decision_md TEXT NOT NULL,
  decided_by_json TEXT NOT NULL,
  decided_on TEXT NOT NULL,
  evidence_attachment INTEGER NULL,
  supersedes INTEGER NULL,
  citation TEXT NULL,
  seq INTEGER NOT NULL,
  superseded_by INTEGER NULL
);
CREATE TABLE IF NOT EXISTS open_items(
  id INTEGER PRIMARY KEY,
  title TEXT NOT NULL,
  detail_md TEXT NULL,
  due TEXT NULL,
  source_json TEXT NOT NULL,
  seq INTEGER NOT NULL,
  resolution TEXT NULL,
  resolved_source_json TEXT NULL,
  resolved_seq INTEGER NULL
);
CREATE TABLE IF NOT EXISTS entry_approvals(
  seq INTEGER PRIMARY KEY,
  entry_id INTEGER NOT NULL,
  approver TEXT NOT NULL,
  fingerprint TEXT NOT NULL,
  note TEXT NULL,
  source_json TEXT NOT NULL
);
"#,
    r#"
-- v13: 1099 decisions and payment methods (plan P3, defects 12, 18 and
-- 23; D-40). Each 1099 decision recorded with who decided (journal
-- feature vendor_1099_decisions), line by line: the date it takes effect
-- (NULL: from the beginning), nec, misc, not_reportable or NULL
-- (undetermined), and for not_reportable its kind and reason. A decision
-- from the beginning also sets vendors.form_1099, as every decision
-- written before these did. Payment methods (feature payment_instruments)
-- and the method each entry was paid with. Rows only come from lines
-- written under the features, so books written before them project the
-- same as before.
CREATE TABLE IF NOT EXISTS vendor_1099_decisions(
  seq INTEGER PRIMARY KEY,
  vendor_id INTEGER NOT NULL,
  effective TEXT NULL,
  form_1099 TEXT NULL,
  reason_kind TEXT NULL,
  reason TEXT NULL,
  source_json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS instruments(
  id INTEGER PRIMARY KEY,
  kind TEXT NOT NULL,
  label TEXT NOT NULL,
  last4 TEXT NULL,
  owner_party INTEGER NULL,
  account_id INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS entry_instruments(
  entry_id INTEGER PRIMARY KEY,
  instrument_id INTEGER NOT NULL
);
"#,
    r#"
-- v14: assets' basis facts and their depreciation runs keyed by entry
-- (plan P3, defect 13; D-41). Its own migration: v13 shipped on main
-- before these columns, so a v13 file already built never gains them by
-- an edit to v13 (each schema's file is rebuilt from the journal, and a
-- migration's text never changes once shipped).
--
-- An asset's basis facts beyond the rule and contributor basis
-- (asset.basis_set): the fair market value at contribution (TEXT, exact
-- decimal), the §704(c) method, and who gave the facts (JSON). tax_rule
-- keeps the rule as journaled, by its pre-P3 name or its wire name.
-- Depreciation runs keyed by their entry too: the journal can hold a
-- second run for a month whose first was reversed, and the projection
-- keeps every run, as replay does.
ALTER TABLE assets ADD COLUMN fmv_at_contribution TEXT;
ALTER TABLE assets ADD COLUMN sec704c_method TEXT;
ALTER TABLE assets ADD COLUMN basis_source_json TEXT;
CREATE TABLE asset_depreciation_v13(
  asset_id INTEGER NOT NULL,
  period TEXT NOT NULL,
  entry_id INTEGER NOT NULL,
  amount TEXT NOT NULL,
  PRIMARY KEY (asset_id, period, entry_id)
);
INSERT INTO asset_depreciation_v13(asset_id, period, entry_id, amount)
  SELECT asset_id, period, entry_id, amount FROM asset_depreciation;
DROP TABLE asset_depreciation;
ALTER TABLE asset_depreciation_v13 RENAME TO asset_depreciation;
"#,
    r#"
-- v15: a document keeps no figure of its own (plan P4b S1, D-45; rules.md
-- B-38): what an invoice or bill has been paid, and its status, are read
-- from the ledger lines tagged with it, as of any date, and a reversed
-- payment no longer counts. The columns that summed its payment events
-- (and so went on counting a reversed one) go.
ALTER TABLE invoices DROP COLUMN amount_paid;
ALTER TABLE invoices DROP COLUMN status;
ALTER TABLE bills DROP COLUMN amount_paid;
ALTER TABLE bills DROP COLUMN status;
"#,
    r#"
-- v16: a document's new fields (plan P4b S4.3, feature documents_v2): the
-- number it was issued under, its terms, the bill-to and seller snapshots,
-- the customer's reference and its memo; the bill's vendor reference, its
-- receipt date, terms and memo, and why a duplicate was entered anyway.
-- Terms and snapshots are canonical JSON TEXT; a document written before
-- this schema names none and its columns stay NULL. A line's account,
-- quantity and unit price ride its entry in lines_json, as its project
-- does.
ALTER TABLE invoices ADD COLUMN number TEXT;
ALTER TABLE invoices ADD COLUMN terms TEXT;
ALTER TABLE invoices ADD COLUMN bill_to TEXT;
ALTER TABLE invoices ADD COLUMN seller TEXT;
ALTER TABLE invoices ADD COLUMN customer_ref TEXT;
ALTER TABLE invoices ADD COLUMN memo TEXT;
ALTER TABLE bills ADD COLUMN vendor_ref TEXT;
ALTER TABLE bills ADD COLUMN received_on TEXT;
ALTER TABLE bills ADD COLUMN terms TEXT;
ALTER TABLE bills ADD COLUMN memo TEXT;
ALTER TABLE bills ADD COLUMN duplicate_reason TEXT;
"#,
    r#"
-- v17: sent tracking (plan P4b S5, `invoice.sent`): one row per recorded
-- send of an invoice, in the order journaled. A send moves no figure and
-- no ledger line, so this table is only the projection's copy of the
-- journal's record (parity with replay). `on` and `by` are quoted: `on`
-- is an SQL keyword.
CREATE TABLE IF NOT EXISTS invoice_sends(
  invoice_id INTEGER NOT NULL,
  "on" TEXT NOT NULL,
  "by" TEXT NOT NULL,
  via TEXT NOT NULL,
  to_json TEXT NOT NULL,
  kind TEXT NOT NULL,
  reference TEXT NULL,
  note TEXT NULL
);
"#,
    r#"
-- v18: evidence (plan P6, feature attachment_text): what was read out of
-- a file (`attachment.text_extracted`), the facts a human annotated it
-- with (`attachment.annotated`), and the duplicate an attachment was
-- withdrawn as. Extraction is its own record per (attachment, extractor,
-- version): an extractor upgrade adds one, never replaces one. A text
-- hash is the extraction's blob; the text itself is content-addressed in
-- the blob store. Amounts stay TEXT (exact decimal), never REAL.
CREATE TABLE IF NOT EXISTS extractions(
  attachment_id INTEGER NOT NULL,
  extractor TEXT NOT NULL,
  version TEXT NOT NULL,
  text_sha256 TEXT NULL,
  pages INTEGER NOT NULL,
  status TEXT NOT NULL,
  seq INTEGER NOT NULL,
  PRIMARY KEY(attachment_id, extractor, version)
);
CREATE INDEX IF NOT EXISTS idx_extractions_text ON extractions(text_sha256);
CREATE TABLE IF NOT EXISTS annotations(
  attachment_id INTEGER PRIMARY KEY,
  receipt_number INTEGER NULL,
  payee TEXT NULL,
  amount TEXT NULL,
  date TEXT NULL,
  instrument TEXT NULL,
  category TEXT NULL,
  source TEXT NULL,
  reimbursable_to TEXT NULL,
  note TEXT NULL,
  seq INTEGER NOT NULL
);
ALTER TABLE attachments ADD COLUMN duplicate_of INTEGER NULL;
"#,
    r#"
-- v19: draft states and posting order (plan P4, `drafts_v2`). Drafts are
-- not rows of `entries` (they never touch the projection until posted),
-- so a draft's state has a table of its own: one row per draft awaiting
-- its evidence (a draft being built is the default and has no row), gone
-- when the draft posts or is discarded. `posted_seq` is the journal line
-- (`entry.posted` or `entry.reversed`) that posted the entry: what the
-- digest's `since_seq` reads. Old rows are never migrated in place: the
-- database file is named for its schema, so a v19 file is built from the
-- journal.
ALTER TABLE entries ADD COLUMN posted_seq INTEGER NULL;
CREATE TABLE IF NOT EXISTS draft_states(
  entry_id INTEGER PRIMARY KEY,
  state TEXT NOT NULL,
  note TEXT NULL
);
"#,
];
