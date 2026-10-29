//! The audit's journal and the integrity checks (plan P8; rules.md B-45):
//! `audit.run`, `receipt.attested`, `entry.reviewed` and `period.attested`,
//! what "the current head" means, and the findings only the store can see.
//!
//! ```text
//! GOAL: an audit run is a journaled fact keyed by the books' figure
//!       head, so the same books give the same run once, and an
//!       attestation rests on a head that has not moved.
//! SOURCE: IRS Pub. 583 (records); Rev. Proc. 98-25 §5.01 (the audit
//!         trail); docs/spec/rules.md B-45; DECISION D-9 (the bar), D-53
//!         (an audit is a run, not a job), IZ-1k (the head).
//! INVARIANTS:
//!   1. The figure head is the seq of the last journal line that bears on
//!      a figure: everything but the audit and attestation events, the
//!      evidence filings (an attachment, a link, a statement, a
//!      reconciliation, extraction, annotations) and the format
//!      declaration. An audit's own filing never moves it.
//!   2. A run is idempotent on (through, figure head, scope, program
//!      version): asking again returns the run that exists and journals
//!      nothing.
//!   3. A period is attested only on a run at the current figure head
//!      that covers it (its through date is on or after the period's
//!      end).
//!   4. A receipt attestation whose amount or date differs from the
//!      entry's is recorded, marked `disagrees`, and never counts.
//!   5. `counted` is never stored: it is read against
//!      `policy.attestation_bar` as it stands when asked.
//!   6. The method is the client's own claim; the books keep it and say
//!      so.
//!   7. Every write waits for the `audits` journal feature and journals
//!      nothing when refused.
//! SCENARIOS:
//!   1. IF the same audit is asked twice at one head: SHOULD return the
//!      first run both times.
//!   2. IF an entry posts after a run: SHOULD refuse attesting a period
//!      on that run.
//! ```

use std::collections::BTreeSet;

use chrono::NaiveDate;
use ledger_core::audit::{Finding, Method, Scope, Severity, Subject};
use rusqlite::params;
use rust_decimal::Decimal;
use serde_json::Value;

use super::{Store, apply_line_to_sqlite};
use crate::error::StoreError;

/// The audit programme's version: the crate's, and a revision of its
/// checks. A run at another version is another run.
pub const PROGRAM_VERSION: &str = concat!("audit-", env!("CARGO_PKG_VERSION"), "-1");

/// Journal lines that do not bear on a figure (INVARIANT 1).
pub const NOT_FIGURES: &[&str] = &[
    "audit.run",
    "receipt.attested",
    "entry.reviewed",
    "period.attested",
    "store.created",
    "journal.format_set",
    "journal.tail_repaired",
    "attachment.linked",
    "attachment.unlinked",
    "attachment.withdrawn",
    "attachment.text_extracted",
    "attachment.annotated",
    "receipt.attached",
    "receipt.linked",
    "receipt.unlinked",
    "document.filed",
    "statement.recorded",
    "statement.voided",
    "reconciliation.completed",
    "reconciliation.voided",
    "decision.recorded",
    "open_item.raised",
    "open_item.resolved",
    "entry.settles",
];

/// The integrity checks' codes, severities and predicates (the
/// statements checks are `ledger_core::audit::CODES`).
pub const CODES: &[ledger_core::audit::CodeRow] = &[
    ledger_core::audit::CodeRow {
        code: "journal_damaged",
        severity: Severity::Failure,
        predicate: "the journal's hash chain does not verify",
    },
    ledger_core::audit::CodeRow {
        code: "projection_differs",
        severity: Severity::Failure,
        predicate: "the live books or the database differ from a replay of the journal",
    },
    ledger_core::audit::CodeRow {
        code: "blob_missing",
        severity: Severity::Failure,
        predicate: "a stored file the books name is not on disk",
    },
    ledger_core::audit::CodeRow {
        code: "blob_damaged",
        severity: Severity::Failure,
        predicate: "a stored file's bytes do not match their SHA-256",
    },
    ledger_core::audit::CodeRow {
        code: "access_log_damaged",
        severity: Severity::Failure,
        predicate: "the access log's chain does not verify",
    },
    ledger_core::audit::CodeRow {
        code: "extraction_blob_missing",
        severity: Severity::Warning,
        predicate: "a file's extracted text is not on disk",
    },
    ledger_core::audit::CodeRow {
        code: "duplicate_blob",
        severity: Severity::Warning,
        predicate: "two filed attachments, neither withdrawn, hold the same bytes",
    },
    ledger_core::audit::CodeRow {
        code: "backup_stale",
        severity: Severity::Warning,
        predicate: "a backup target has not reached the books' head within a day",
    },
    ledger_core::audit::CodeRow {
        code: "drill_failed",
        severity: Severity::Failure,
        predicate: "a backup failed, or failed its restore drill",
    },
];

fn found(code: &'static str, subject: Subject, message: String) -> Finding {
    let severity = CODES
        .iter()
        .find(|r| r.code == code)
        .map_or(Severity::Failure, |r| r.severity);
    Finding {
        code,
        severity,
        subject,
        message,
    }
}

/// What the backups' standing says, one target at a time, for the audit
/// (the server reads the standing; the store does not know the backups).
#[derive(Clone, Debug)]
pub struct BackupStanding {
    /// The target's kind and name, as the backups name it.
    pub target: String,
    /// It has not reached the head within a day.
    pub stale: bool,
    /// Its last run's error, if it failed.
    pub last_error: Option<String>,
    /// Its last restore drill's error, if it failed.
    pub drill_error: Option<String>,
}

/// The backup findings: a target stale, failing, or failing its drill.
/// Books with no target set up have none (that is the owner's standing
/// choice, which `books.start` already says; an audit reports what the
/// backups it can see do).
pub fn backup_findings(targets: &[BackupStanding]) -> Vec<Finding> {
    let mut out = Vec::new();
    for t in targets {
        if t.stale {
            out.push(found(
                "backup_stale",
                Subject::Named(t.target.clone()),
                format!("{} has not reached the books' head within a day", t.target),
            ));
        }
        if let Some(e) = &t.last_error {
            out.push(found(
                "drill_failed",
                Subject::Named(t.target.clone()),
                format!("{}'s last backup failed: {e}", t.target),
            ));
        }
        if let Some(e) = &t.drill_error {
            out.push(found(
                "drill_failed",
                Subject::Named(t.target.clone()),
                format!("{}'s restore drill failed: {e}", t.target),
            ));
        }
    }
    out
}

/// An audit run as the journal records it (`audit.run`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditRun {
    /// Its id (1 for the first).
    pub id: u64,
    /// The audit covers the books through this date.
    pub through: NaiveDate,
    /// The figure head it was made at ([`Store::figure_head`]).
    pub through_seq: u64,
    /// What it covers.
    pub scope: Scope,
    /// The programme's version ([`PROGRAM_VERSION`]).
    pub program_version: String,
    /// SHA-256 of the trial balance through the date.
    pub tb_hash: String,
    /// SHA-256 of the findings.
    pub findings_sha256: String,
    /// The workpaper's JSON, filed as a document.
    pub workpaper_attachment: Option<u64>,
    /// The workpaper's PDF, filed as a document.
    pub workpaper_pdf: Option<u64>,
    /// The journal line that recorded it.
    pub seq: u64,
}

/// What a run is made from, before it has an id or a seq.
#[derive(Clone, Debug)]
pub struct NewAuditRun {
    /// Through this date.
    pub through: NaiveDate,
    /// The figure head the findings were computed at.
    pub through_seq: u64,
    /// What it covers.
    pub scope: Scope,
    /// SHA-256 of the trial balance.
    pub tb_hash: String,
    /// SHA-256 of the findings.
    pub findings_sha256: String,
    /// The filed workpaper JSON.
    pub workpaper_attachment: Option<u64>,
    /// The filed workpaper PDF.
    pub workpaper_pdf: Option<u64>,
}

/// What an attestation says it checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Attested {
    /// A receipt against an entry.
    Receipt {
        /// The filed receipt.
        attachment_id: u64,
        /// The entry it supports.
        entry_id: u64,
        /// The amount the checker read off it.
        amount_checked: Decimal,
        /// The date the checker read off it.
        date_checked: NaiveDate,
        /// The checker's reading differs from the entry's.
        disagrees: bool,
    },
    /// An entry, reviewed whole.
    Entry {
        /// The entry.
        entry_id: u64,
    },
    /// A period, on an audit run.
    Period {
        /// The period.
        period_id: u64,
        /// The run it rests on.
        audit_run_id: u64,
    },
}

/// A claim that someone checked, as the journal records it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attestation {
    /// The journal line.
    pub seq: u64,
    /// Who claims it: the caller's token name.
    pub by: String,
    /// When, UTC.
    pub at: String,
    /// How, as the client says.
    pub method: Method,
    /// A free note.
    pub note: Option<String>,
    /// What was checked.
    pub what: Attested,
}

impl Attestation {
    /// Its kind's word: `receipt`, `entry` or `period`.
    pub fn kind(&self) -> &'static str {
        match self.what {
            Attested::Receipt { .. } => "receipt",
            Attested::Entry { .. } => "entry",
            Attested::Period { .. } => "period",
        }
    }
}

/// Who claims an attestation, how, and what they add.
#[derive(Clone, Debug)]
pub struct Claim {
    /// The caller's token name.
    pub by: String,
    /// How, as the client says.
    pub method: Method,
    /// A free note.
    pub note: Option<String>,
}

impl Claim {
    fn into_attestation(self, what: Attested) -> Attestation {
        Attestation {
            seq: 0,
            by: self.by,
            at: now(),
            method: self.method,
            note: self
                .note
                .map(|n| n.trim().to_string())
                .filter(|n| !n.is_empty()),
            what,
        }
    }
}

fn refused(why: impl Into<String>) -> StoreError {
    StoreError::AuditRefused(why.into())
}

fn now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

impl Store {
    /// The seq of the last journal line that bears on a figure
    /// (INVARIANT 1); zero for books with none.
    pub fn figure_head(&self) -> Result<u64, StoreError> {
        let quoted: Vec<String> = NOT_FIGURES.iter().map(|t| format!("'{t}'")).collect();
        let sql = format!(
            "SELECT COALESCE(MAX(seq), 0) FROM events WHERE type NOT IN ({})",
            quoted.join(",")
        );
        let n: i64 = self.conn.query_row(&sql, [], |r| r.get(0))?;
        Ok(n as u64)
    }

    /// The entries with at least one receipt filed on them or linked to
    /// them, not withdrawn.
    pub fn entries_with_receipts(&self) -> Result<BTreeSet<u64>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT entry_id FROM attachments WHERE withdrawn_at IS NULL AND entry_id IS NOT NULL
             UNION
             SELECT l.subject_id FROM attachment_links l JOIN attachments a ON a.id = l.attachment_id
              WHERE l.subject_kind = 'entry' AND a.withdrawn_at IS NULL",
        )?;
        let ids = stmt
            .query_map([], |r| r.get::<_, i64>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids.into_iter().map(|i| i as u64).collect())
    }

    /// What only the store can see: the chain, the database against a
    /// replay, the stored files, the access log, extraction text and
    /// duplicate files. Reads; changes nothing.
    pub fn integrity_findings(&self) -> Vec<Finding> {
        let mut out = Vec::new();
        match self.verify() {
            Ok(()) => {}
            Err(StoreError::VerifyMismatch(why)) => out.push(found(
                "projection_differs",
                Subject::Books,
                format!("the books differ from a replay of the journal: {why}"),
            )),
            Err(e) => out.push(found(
                "journal_damaged",
                Subject::Books,
                format!("the journal does not verify: {e}"),
            )),
        }
        let shas = |sql: &str| -> Vec<String> {
            self.conn
                .prepare(sql)
                .and_then(|mut s| {
                    s.query_map([], |r| r.get::<_, String>(0))?
                        .collect::<Result<Vec<_>, _>>()
                })
                .unwrap_or_default()
        };
        let mut blobs = shas("SELECT sha256 FROM blobs ORDER BY sha256");
        blobs.dedup();
        for sha in blobs {
            if !self.blobs.exists(&sha) {
                out.push(found(
                    "blob_missing",
                    Subject::Named(sha.chars().take(12).collect()),
                    format!("the stored file {sha} is not on disk"),
                ));
            } else if let Err(StoreError::CorruptBlob(_)) = self.blobs.get(&sha) {
                out.push(found(
                    "blob_damaged",
                    Subject::Named(sha.chars().take(12).collect()),
                    format!("the stored file {sha} does not match its SHA-256"),
                ));
            }
        }
        if let Err(e) = crate::access::read(&self.data_dir) {
            out.push(found(
                "access_log_damaged",
                Subject::Named("access log".into()),
                format!("the access log does not verify: {e}"),
            ));
        }
        for sha in shas(
            "SELECT DISTINCT text_sha256 FROM extractions WHERE text_sha256 IS NOT NULL ORDER BY text_sha256",
        ) {
            if !self.blobs.exists(&sha) {
                out.push(found(
                    "extraction_blob_missing",
                    Subject::Named(sha.chars().take(12).collect()),
                    format!("the extracted text {sha} is not on disk"),
                ));
            }
        }
        let dups: Vec<(String, String)> = self
            .conn
            .prepare(
                "SELECT blob_sha, GROUP_CONCAT(id) FROM
                   (SELECT blob_sha, id FROM attachments WHERE withdrawn_at IS NULL ORDER BY id)
                 GROUP BY blob_sha HAVING COUNT(*) > 1 ORDER BY blob_sha",
            )
            .and_then(|mut s| {
                s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<Result<Vec<_>, _>>()
            })
            .unwrap_or_default();
        for (sha, ids) in dups {
            out.push(found(
                "duplicate_blob",
                Subject::Named(sha.chars().take(12).collect()),
                format!("attachments {ids} hold the same bytes ({sha})"),
            ));
        }
        out
    }

    /// Every audit run, oldest first.
    pub fn audit_runs(&self) -> Result<Vec<AuditRun>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, through, through_seq, scope, program_version, tb_hash, findings_sha256,
                    workpaper_attachment, workpaper_pdf, seq FROM audits ORDER BY id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, Option<i64>>(7)?,
                r.get::<_, Option<i64>>(8)?,
                r.get::<_, i64>(9)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, through, seq_at, scope, version, tb, fs, wp, pdf, seq) = row?;
            out.push(AuditRun {
                id: id as u64,
                through: NaiveDate::parse_from_str(&through, "%Y-%m-%d")
                    .map_err(|_| StoreError::JournalCorrupt("audits.through".into()))?,
                through_seq: seq_at as u64,
                scope: Scope::parse(&scope)
                    .ok_or_else(|| StoreError::JournalCorrupt("audits.scope".into()))?,
                program_version: version,
                tb_hash: tb,
                findings_sha256: fs,
                workpaper_attachment: wp.map(|v| v as u64),
                workpaper_pdf: pdf.map(|v| v as u64),
                seq: seq as u64,
            });
        }
        Ok(out)
    }

    /// One run.
    pub fn audit_run(&self, id: u64) -> Result<Option<AuditRun>, StoreError> {
        Ok(self.audit_runs()?.into_iter().find(|r| r.id == id))
    }

    /// The run that already answers an audit of `through` at `head`
    /// (INVARIANT 2).
    pub fn find_audit_run(
        &self,
        through: NaiveDate,
        head: u64,
        scope: Scope,
    ) -> Result<Option<AuditRun>, StoreError> {
        Ok(self.audit_runs()?.into_iter().find(|r| {
            r.through == through
                && r.through_seq == head
                && r.scope == scope
                && r.program_version == PROGRAM_VERSION
        }))
    }

    /// Whether the books' figures are where `run` left them
    /// (IZ-1k): nothing bearing on a figure has been journaled since.
    pub fn audit_is_current(&self, run: &AuditRun) -> Result<bool, StoreError> {
        Ok(run.through_seq == self.figure_head()?)
    }

    /// Record an audit run (`audit.run`). Waits for the `audits` feature.
    pub fn record_audit_run(&mut self, new: NewAuditRun) -> Result<AuditRun, StoreError> {
        self.ensure_feature(crate::format::AUDITS)?;
        if let Some(existing) = self.find_audit_run(new.through, new.through_seq, new.scope)? {
            return Ok(existing);
        }
        for a in [new.workpaper_attachment, new.workpaper_pdf]
            .into_iter()
            .flatten()
        {
            self.attachment_state(a)?;
        }
        let id = self.audit_runs()?.last().map_or(1, |r| r.id + 1);
        let mut run = AuditRun {
            id,
            through: new.through,
            through_seq: new.through_seq,
            scope: new.scope,
            program_version: PROGRAM_VERSION.into(),
            tb_hash: new.tb_hash,
            findings_sha256: new.findings_sha256,
            workpaper_attachment: new.workpaper_attachment,
            workpaper_pdf: new.workpaper_pdf,
            seq: 0,
        };
        let line = self.append_journal("audit.run", crate::meta::audit_run(&run))?;
        run.seq = line.seq;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(run)
    }

    /// Every attestation, oldest first.
    pub fn attestations(&self) -> Result<Vec<Attestation>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, kind, attachment_id, entry_id, period_id, audit_run_id, by_who, at_ts,
                    method, amount_checked, date_checked, disagrees, note
               FROM attestations ORDER BY seq",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, Option<i64>>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, Option<String>>(9)?,
                r.get::<_, Option<String>>(10)?,
                r.get::<_, Option<i64>>(11)?,
                r.get::<_, Option<String>>(12)?,
            ))
        })?;
        let bad = |w: &str| StoreError::JournalCorrupt(format!("attestations.{w}"));
        let mut out = Vec::new();
        for row in rows {
            let (seq, kind, att, entry, period, run, by, at, method, amount, date, dis, note) =
                row?;
            let method: Value = serde_json::from_str(&method)?;
            let what = match kind.as_str() {
                "receipt" => Attested::Receipt {
                    attachment_id: att.ok_or_else(|| bad("attachment_id"))? as u64,
                    entry_id: entry.ok_or_else(|| bad("entry_id"))? as u64,
                    amount_checked: amount
                        .and_then(|a| a.parse().ok())
                        .ok_or_else(|| bad("amount_checked"))?,
                    date_checked: date
                        .and_then(|d| NaiveDate::parse_from_str(&d, "%Y-%m-%d").ok())
                        .ok_or_else(|| bad("date_checked"))?,
                    disagrees: dis.unwrap_or(0) != 0,
                },
                "entry" => Attested::Entry {
                    entry_id: entry.ok_or_else(|| bad("entry_id"))? as u64,
                },
                "period" => Attested::Period {
                    period_id: period.ok_or_else(|| bad("period_id"))? as u64,
                    audit_run_id: run.ok_or_else(|| bad("audit_run_id"))? as u64,
                },
                _ => return Err(bad("kind")),
            };
            out.push(Attestation {
                seq: seq as u64,
                by,
                at,
                method: crate::meta::method_from_value(&method)?,
                note,
                what,
            });
        }
        Ok(out)
    }

    fn journal_attestation(
        &mut self,
        line_type: &str,
        a: Attestation,
    ) -> Result<Attestation, StoreError> {
        let (line_type, payload) = (line_type, crate::meta::attestation(&a));
        let line = self.append_journal(line_type, payload)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(Attestation { seq: line.seq, ..a })
    }

    /// Attest that a receipt supports an entry (`receipt.attested`): the
    /// checker's reading of its amount and date against the entry's. A
    /// reading that differs is recorded as disagreeing and never counts
    /// (INVARIANT 4).
    pub fn attest_receipt(
        &mut self,
        attachment_id: u64,
        entry_id: u64,
        amount: Decimal,
        date: NaiveDate,
        claim: Claim,
    ) -> Result<Attestation, StoreError> {
        self.ensure_feature(crate::format::AUDITS)?;
        let entry = self
            .ledger
            .get_entry(entry_id)
            .ok_or(ledger_core::LedgerError::UnknownEntry(entry_id))?;
        if entry.status() != ledger_core::EntryStatus::Posted {
            return Err(refused(format!("entry {entry_id} is not posted")));
        }
        let (_, withdrawn) = self.attachment_state(attachment_id)?;
        if withdrawn {
            return Err(StoreError::AttachmentWithdrawn(attachment_id));
        }
        if !self
            .attachment_links(attachment_id)?
            .contains(&crate::store::Subject::Entry(entry_id))
        {
            return Err(refused(format!(
                "attachment {attachment_id} is not filed on or linked to entry {entry_id}"
            )));
        }
        let debits = entry
            .lines()
            .iter()
            .filter(|l| l.side == ledger_core::Side::Debit)
            .try_fold(Decimal::ZERO, |s, l| s.checked_add(l.amount.amount));
        let amount_agrees =
            debits == Some(amount) || entry.lines().iter().any(|l| l.amount.amount == amount);
        let disagrees = !amount_agrees || date != entry.date();
        self.journal_attestation(
            "receipt.attested",
            claim.into_attestation(Attested::Receipt {
                attachment_id,
                entry_id,
                amount_checked: amount,
                date_checked: date,
                disagrees,
            }),
        )
    }

    /// Record that an entry was reviewed whole (`entry.reviewed`).
    pub fn review_entry(&mut self, entry_id: u64, claim: Claim) -> Result<Attestation, StoreError> {
        self.ensure_feature(crate::format::AUDITS)?;
        let entry = self
            .ledger
            .get_entry(entry_id)
            .ok_or(ledger_core::LedgerError::UnknownEntry(entry_id))?;
        if entry.status() != ledger_core::EntryStatus::Posted {
            return Err(refused(format!("entry {entry_id} is not posted")));
        }
        self.journal_attestation(
            "entry.reviewed",
            claim.into_attestation(Attested::Entry { entry_id }),
        )
    }

    /// The run a period can be attested on: the latest at the current
    /// figure head whose through date is on or after the period's end
    /// (INVARIANT 3).
    pub fn run_for_period(&self, period_id: u64) -> Result<AuditRun, StoreError> {
        let period = self
            .ledger
            .get_period(period_id)
            .ok_or_else(|| refused(format!("no period {period_id}")))?;
        let head = self.figure_head()?;
        let runs = self.audit_runs()?;
        runs.into_iter()
            .rev()
            .find(|r| r.through_seq == head && r.through >= period.end)
            .ok_or_else(|| {
                refused(format!(
                    "no audit through {} or later stands at the books' current head: run books.audit first (an entry, a document, a close or a settings change since the last one moves the head)",
                    period.end
                ))
            })
    }

    /// Attest a period on an audit run at the current head
    /// (`period.attested`).
    pub fn attest_period(
        &mut self,
        period_id: u64,
        claim: Claim,
    ) -> Result<(Attestation, AuditRun), StoreError> {
        self.ensure_feature(crate::format::AUDITS)?;
        let run = self.run_for_period(period_id)?;
        let a = self.journal_attestation(
            "period.attested",
            claim.into_attestation(Attested::Period {
                period_id,
                audit_run_id: run.id,
            }),
        )?;
        Ok((a, run))
    }
}

/// Read an `audits` row for a replayed line.
pub(crate) fn insert_run(conn: &rusqlite::Connection, run: &AuditRun) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO audits(id, through, through_seq, scope, program_version, tb_hash, findings_sha256,
                            workpaper_attachment, workpaper_pdf, seq)
         VALUES (?,?,?,?,?,?,?,?,?,?)",
        params![
            run.id as i64,
            run.through.to_string(),
            run.through_seq as i64,
            run.scope.key(),
            run.program_version,
            run.tb_hash,
            run.findings_sha256,
            run.workpaper_attachment.map(|v| v as i64),
            run.workpaper_pdf.map(|v| v as i64),
            run.seq as i64,
        ],
    )?;
    Ok(())
}

/// Write an `attestations` row for a replayed line.
pub(crate) fn insert_attestation(
    conn: &rusqlite::Connection,
    a: &Attestation,
) -> Result<(), StoreError> {
    let (mut att, mut entry, mut period, mut run) = (None, None, None, None);
    let (mut amount, mut date, mut dis) = (None, None, None);
    match &a.what {
        Attested::Receipt {
            attachment_id,
            entry_id,
            amount_checked,
            date_checked,
            disagrees,
        } => {
            att = Some(*attachment_id as i64);
            entry = Some(*entry_id as i64);
            amount = Some(amount_checked.to_string());
            date = Some(date_checked.to_string());
            dis = Some(i64::from(*disagrees));
        }
        Attested::Entry { entry_id } => entry = Some(*entry_id as i64),
        Attested::Period {
            period_id,
            audit_run_id,
        } => {
            period = Some(*period_id as i64);
            run = Some(*audit_run_id as i64);
        }
    }
    conn.execute(
        "INSERT INTO attestations(seq, kind, attachment_id, entry_id, period_id, audit_run_id, by_who, at_ts,
                                  method, amount_checked, date_checked, disagrees, note)
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
        params![
            a.seq as i64,
            a.kind(),
            att,
            entry,
            period,
            run,
            a.by,
            a.at,
            crate::meta::method_json(&a.method).to_string(),
            amount,
            date,
            dis,
            a.note,
        ],
    )?;
    Ok(())
}
