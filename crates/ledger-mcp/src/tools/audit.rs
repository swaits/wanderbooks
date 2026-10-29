//! The audit group (plan P8; rules.md B-45): an audit run over the books,
//! its workpaper, and the claims that someone checked.
//!
//! ```text
//! GOAL: the whole re-audit through the tools: one programme of checks
//!       that gives the same findings and the same workpaper for the same
//!       books, and attestations that rest on a head that has not moved.
//! SOURCE: IRS Pub. 583 (records); Rev. Proc. 98-25 §5.01 (the audit
//!         trail); docs/spec/rules.md B-45; DECISION D-9 (the bar), D-53
//!         (an audit is a run, not a job), IZ-1k (the head).
//! INVARIANTS:
//!   1. `books.audit` previews (what it will check; nothing changes),
//!      then with the preview's digest runs the checks, files the
//!      workpaper (JSON and PDF) as documents and journals `audit.run`.
//!   2. The same books (the figure head), day, scope and programme give
//!      the run that exists: nothing is filed or journaled twice.
//!   3. An attestation is a claim by the caller named by its token; the
//!      method it gives is self-reported and the books say so.
//!   4. `counted` is read against `policy.attestation_bar` as it stands
//!      when asked, never stored.
//!   5. A period is attested only on an audit at the current head.
//!   6. Nothing here moves a figure.
//! ```

use super::*;
use ledger_core::audit::{self, AttestationBar, AuditInput, Effort, Finding, Method, Scope};
use ledger_store::audit::{self as saudit, Attested, AuditRun, BackupStanding, Claim};
use sha2::{Digest, Sha256};

fn sha_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// The caller's token name, else the default token's.
fn caller_name() -> String {
    CALLER
        .with(|c| c.borrow().as_ref().map(|(_, name)| name.clone()))
        .unwrap_or_else(|| ledger_store::auth::DEFAULT_NAME.to_string())
}

fn method_of(m: Option<AttestMethodParam>) -> Result<Method, String> {
    let m = m.unwrap_or_default();
    let reasoning_effort = match m.reasoning_effort.as_deref() {
        None => None,
        Some(e) => Some(Effort::parse(e).ok_or_else(|| {
            format!("reasoning_effort {e:?} is not low, medium, high, xhigh or max")
        })?),
    };
    if m.verifier_agents == Some(0) {
        return Err("verifier_agents is at least 1".into());
    }
    Ok(Method {
        model: m
            .model
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        reasoning_effort,
        adversarial: m.adversarial,
        verifier_agents: m.verifier_agents,
    })
}

fn method_out(m: &Method) -> AttestMethodParam {
    AttestMethodParam {
        model: m.model.clone(),
        reasoning_effort: m.reasoning_effort.map(|e| e.key().to_string()),
        adversarial: m.adversarial,
        verifier_agents: m.verifier_agents,
    }
}

fn finding_out(f: &Finding) -> AuditFindingOut {
    AuditFindingOut {
        severity: f.severity.key().into(),
        code: f.code.into(),
        subject: f.subject.describe(),
        message: f.message.clone(),
    }
}

/// Every check the programme runs.
fn checks() -> Vec<AuditCheckOut> {
    audit::CODES
        .iter()
        .chain(saudit::CODES)
        .map(|r| AuditCheckOut {
            code: r.code.into(),
            severity: r.severity.key().into(),
            predicate: r.predicate.into(),
        })
        .collect()
}

fn run_out(store: &Store, run: &AuditRun) -> Result<AuditRunOut, String> {
    Ok(AuditRunOut {
        id: run.id,
        through: run.through.to_string(),
        through_seq: run.through_seq,
        scope: run.scope.key().into(),
        program_version: run.program_version.clone(),
        tb_hash: run.tb_hash.clone(),
        findings_sha256: run.findings_sha256.clone(),
        workpaper_attachment: run.workpaper_attachment,
        workpaper_pdf: run.workpaper_pdf,
        at_current_head: store.audit_is_current(run).map_err(err)?,
    })
}

fn summary(findings: &[Finding]) -> String {
    let n = |s: audit::Severity| findings.iter().filter(|f| f.severity == s).count();
    format!(
        "{} failure(s), {} warning(s), {} note(s)",
        n(audit::Severity::Failure),
        n(audit::Severity::Warning),
        n(audit::Severity::Info)
    )
}

/// Why an attestation does not count at this bar, in words.
fn why_not(bar: &AttestationBar, m: &Method, full: bool, disagrees: bool) -> Option<String> {
    if bar.counts(m, full, disagrees) {
        return None;
    }
    if disagrees {
        return Some(
            "the amount or date read off the receipt differs from the entry's: it is recorded and never counts"
                .into(),
        );
    }
    let mut why = Vec::new();
    if !m
        .reasoning_effort
        .is_some_and(|e| e >= bar.reasoning_effort)
    {
        why.push(format!(
            "reasoning_effort must be at least {}",
            bar.reasoning_effort.key()
        ));
    }
    if full {
        if bar.adversarial && m.adversarial != Some(true) {
            why.push("the check must be adversarial".into());
        }
        if !m.verifier_agents.is_some_and(|n| n >= bar.verifier_agents) {
            why.push(format!(
                "at least {} independent verifier agents",
                bar.verifier_agents
            ));
        }
    }
    Some(format!(
        "below policy.attestation_bar ({}): {}",
        bar.describe(),
        why.join("; ")
    ))
}

/// An attestation as the tools list it.
fn attestation_out(
    store: &Store,
    a: &saudit::Attestation,
    bar: &AttestationBar,
) -> Result<AttestationOut, String> {
    let mut out = AttestationOut {
        seq: a.seq,
        kind: a.kind().into(),
        by: a.by.clone(),
        at: a.at.clone(),
        attachment_id: None,
        entry_id: None,
        period_id: None,
        audit_run_id: None,
        amount_checked: None,
        date_checked: None,
        disagrees: false,
        method: method_out(&a.method),
        note: a.note.clone(),
        counted: false,
    };
    let (full, disagrees) = match &a.what {
        Attested::Receipt {
            attachment_id,
            entry_id,
            amount_checked,
            date_checked,
            disagrees,
        } => {
            out.attachment_id = Some(*attachment_id);
            out.entry_id = Some(*entry_id);
            out.amount_checked = Some(amount_checked.to_string());
            out.date_checked = Some(date_checked.to_string());
            out.disagrees = *disagrees;
            (false, *disagrees)
        }
        Attested::Entry { entry_id } => {
            out.entry_id = Some(*entry_id);
            (false, false)
        }
        Attested::Period {
            period_id,
            audit_run_id,
        } => {
            out.period_id = Some(*period_id);
            out.audit_run_id = Some(*audit_run_id);
            let scope = store
                .audit_run(*audit_run_id)
                .map_err(err)?
                .map(|r| r.scope);
            (scope.is_some_and(Scope::full_bar), false)
        }
    };
    out.counted = bar.counts(&a.method, full, disagrees);
    Ok(out)
}

/// What an attestation's tool answers.
fn attest_out(store: &Store, a: &saudit::Attestation) -> Result<AttestOut, String> {
    let bar = store.ledger().settings().attestation_bar();
    let o = attestation_out(store, a, &bar)?;
    let full = o
        .audit_run_id
        .and_then(|id| store.audit_run(id).ok().flatten())
        .is_some_and(|r| r.scope.full_bar());
    let why_not_counted = why_not(&bar, &a.method, full, o.disagrees);
    Ok(AttestOut {
        attestation: o,
        bar: bar.describe(),
        why_not_counted,
    })
}

impl BooksServer {
    /// Whether the CPA package may be made (plan P8 step 4, D-9): on books
    /// that declared `audits` and a policy of refuse, an audit for the
    /// handoff (scope cpa_handoff or full) at the current head must have a
    /// period attested on it at the bar. `Ok(None)` when it may; the
    /// refusal's words otherwise.
    pub(crate) fn cpa_audit_standing(&self, store: &Store) -> Result<Option<String>, String> {
        if !store
            .journal_format()
            .has_feature(ledger_store::format::AUDITS)
        {
            return Ok(None);
        }
        let settings = store.ledger().settings();
        let bar = settings.attestation_bar();
        let attestations = store.attestations().map_err(err)?;
        for run in store.audit_runs().map_err(err)?.iter().rev() {
            if !run.scope.serves_handoff() || !store.audit_is_current(run).map_err(err)? {
                continue;
            }
            let counted = attestations.iter().any(|a| {
                matches!(a.what, Attested::Period { audit_run_id, .. } if audit_run_id == run.id)
                    && bar.counts(&a.method, run.scope.full_bar(), false)
            });
            if counted {
                return Ok(None);
            }
        }
        Ok(Some(format!(
            "no audit for the handoff (books.audit with scope cpa_handoff or full) stands at the books' current head with a period attested at the bar ({}); run the audit, have it verified (books.attest_receipt, books.review_entry, books.attest_period), or pass without_audit_reason",
            bar.describe()
        )))
    }

    /// The findings the programme gives the books through `through`: the
    /// statements, then what only the store, the backups and the settings
    /// can see. Reads only.
    fn audit_findings(
        &self,
        store: &Store,
        through: NaiveDate,
    ) -> Result<(Vec<Finding>, String), String> {
        let ledger = store.ledger();
        let cfg = self.settings();
        let ps = package_settings(&cfg);
        let receipts = store.entries_with_receipts().map_err(err)?;
        // The statement of cash flows, from the first entry, with the
        // books' role settings; none when they name no cash accounts.
        let first = ledger
            .posted_entries()
            .filter(|e| e.date() <= through)
            .map(|e| e.date())
            .min();
        let cash = match first {
            Some(start) if !ps.cash_accounts.is_empty() => Some(
                ledger_package::cash_flow_for(
                    ledger,
                    &ps,
                    Some(&start.to_string()),
                    Some(&through.to_string()),
                    &[],
                    &[],
                    &[],
                    today(),
                )
                .map(|s| s.reconciles),
            ),
            _ => None,
        };
        // Vendors paid in a way that could need a 1099 and undecided; a
        // year whose threshold isn't known is the stale-rule item's.
        use chrono::Datelike;
        let undecided: Vec<(String, rust_decimal::Decimal)> =
            ledger_package::vendor_1099_report(ledger, &ps, through.year(), None, Some(through))
                .map(|r| {
                    r.vendors
                        .iter()
                        .filter(|v| v.decision_needed)
                        .map(|v| (v.name.clone(), v.paid))
                        .collect()
                })
                .unwrap_or_default();
        let problems = settings_problems(ledger, &cfg);
        let mut findings = audit::statements(
            ledger,
            &AuditInput {
                settings: ledger.settings(),
                receipts: &receipts,
                cash_flow: cash.as_ref(),
                settings_problems: &problems,
                undecided_1099: &undecided,
            },
            through,
        );
        findings.extend(store.integrity_findings());
        let standing = self.backups.standing(store);
        findings.extend(saudit::backup_findings(
            &standing
                .targets
                .iter()
                .map(|t| BackupStanding {
                    target: format!("{} {}", t.target, t.at),
                    stale: t.stale,
                    last_error: t.last_error.clone(),
                    drill_error: t.drill_error.clone(),
                })
                .collect::<Vec<_>>(),
        ));
        audit::sort(&mut findings);
        let tb = ledger_core::reports::trial_balance_as_of(ledger, through);
        let tb_hash = sha_hex(&serde_json::to_vec(&tb).map_err(err)?);
        Ok((findings, tb_hash))
    }
}

#[tool_router(router = audit_router, vis = "pub(super)")]
impl BooksServer {
    /// ```text
    /// GOAL: the whole programme of checks over the books through a day,
    ///       its workpaper filed, the run journaled.
    /// SOURCE: plan/served.md P8; B-45; D-9, D-53.
    /// INVARIANTS:
    ///   1. Without `confirmed`, a preview that lists the checks and
    ///      changes nothing; with it, the preview's digest is required.
    ///   2. Idempotent on the figure head, day, scope and programme.
    ///   3. The workpaper is JSON and PDF, filed as documents (category
    ///      workpaper), the PDF with deterministic bytes.
    /// SCENARIOS:
    ///   1. IF the books haven't declared audits: the run is refused with
    ///      format_upgrade_pending; the preview still lists the checks.
    ///   2. IF the same audit is asked at the same head: SHOULD return
    ///      the first run, status already_run.
    /// ```
    #[tool(
        name = "books.audit",
        title = "Audit the books",
        description = "Audit the books through a day (through, default today) at a scope: month, annual, cpa_handoff (the year, for the CPA package) or full. It checks the statements, the A/R and A/P control accounts, closes, schedules, assets, each posted entry against the receipt policy, open drafts, tax categories, 1099 decisions and settings, then the journal, stored files, access log and backups. Without confirmed, a preview of the checks, changing nothing. With confirmed and the preview, it files the workpaper (books.workpaper) and journals the run; the same books, day and scope give the run that exists. Attestations rest on a run only while no entry, document, close, settings or role change follows it",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn audit(&self, params: Parameters<AuditParams>) -> Result<Json<AuditOut>, String> {
        let p = params.0;
        let scope = Scope::parse(&p.scope).ok_or_else(|| {
            format!(
                "bad scope {:?}: expected month, annual, cpa_handoff or full",
                p.scope
            )
        })?;
        let through = self.as_of(p.through.as_deref())?;
        let confirmed = p.confirmed.unwrap_or(false);
        let mut store = self.lock()?;
        let head = store.figure_head().map_err(err)?;
        let digest = self.preview_of(&store, "books.audit", confirmed, p.preview.as_deref())?;
        if let Some(run) = store.find_audit_run(through, head, scope).map_err(err)? {
            let findings = self.stored_findings(&store, &run)?;
            return Ok(Json(AuditOut {
                status: "already_run".into(),
                preview: Some(digest),
                head,
                checks: checks(),
                summary: Some(summary_of(&findings)),
                run: Some(run_out(&store, &run)?),
                findings,
            }));
        }
        if !confirmed {
            return Ok(Json(AuditOut {
                status: "preview".into(),
                preview: Some(digest),
                head,
                checks: checks(),
                run: None,
                findings: Vec::new(),
                summary: None,
            }));
        }
        store
            .ensure_feature(ledger_store::format::AUDITS)
            .map_err(err)?;
        let (findings, tb_hash) = self.audit_findings(&store, through)?;
        let findings_sha256 = sha_hex(&serde_json::to_vec(&findings).map_err(err)?);
        let summary = summary(&findings);
        let company = self.settings().company.name.clone();
        let rows: Vec<AuditFindingOut> = findings.iter().map(finding_out).collect();
        let check_rows = checks();
        let paper = serde_json::json!({
            "company": company,
            "title": "Audit workpaper",
            "scope": scope.key(),
            "through": through.to_string(),
            "through_seq": head,
            "program_version": saudit::PROGRAM_VERSION,
            "tb_hash": tb_hash,
            "findings_sha256": findings_sha256,
            "summary": summary,
            "findings": rows,
            "checks": check_rows,
        });
        let json_bytes = serde_json::to_vec_pretty(&paper).map_err(err)?;
        let pdf_bytes = ledger_import::workpaper_pdf(&ledger_import::Workpaper {
            company,
            title: "Audit workpaper".into(),
            scope: scope.key().into(),
            through: through.to_string(),
            through_seq: head,
            program_version: saudit::PROGRAM_VERSION.into(),
            tb_hash: tb_hash.clone(),
            findings_sha256: findings_sha256.clone(),
            summary: summary.clone(),
            findings: rows
                .iter()
                .map(|f| ledger_import::WorkpaperFinding {
                    severity: f.severity.clone(),
                    code: f.code.clone(),
                    subject: f.subject.clone(),
                    message: f.message.clone(),
                })
                .collect(),
            checks: check_rows
                .iter()
                .map(|c| ledger_import::WorkpaperCheck {
                    code: c.code.clone(),
                    severity: c.severity.clone(),
                    predicate: c.predicate.clone(),
                })
                .collect(),
        })
        .map_err(err)?;
        let stem = format!("workpaper-{}-{through}-{head}", scope.key());
        let describe = format!(
            "Audit workpaper, scope {}, through {through}, books at figure head {head} ({}).",
            scope.key(),
            saudit::PROGRAM_VERSION
        );
        let (wp_json, _, _) = store
            .file_document(
                "workpaper",
                &format!("{stem}.json"),
                "application/json",
                &json_bytes,
                Some(&describe),
            )
            .map_err(err)?;
        let (wp_pdf, _, _) = store
            .file_document(
                "workpaper",
                &format!("{stem}.pdf"),
                "application/pdf",
                &pdf_bytes,
                Some(&describe),
            )
            .map_err(err)?;
        // A month's or a year's workpaper belongs with its period.
        if matches!(scope, Scope::Month | Scope::Annual)
            && let Some(period) = store
                .ledger()
                .list_periods()
                .into_iter()
                .filter(|q| q.start <= through && through <= q.end)
                .min_by_key(|q| (q.end - q.start).num_days())
                .map(|q| q.id)
        {
            for a in [wp_json, wp_pdf] {
                let _ = store.link_attachment(a, ledger_store::Subject::Period(period));
            }
        }
        let run = store
            .record_audit_run(saudit::NewAuditRun {
                through,
                through_seq: head,
                scope,
                tb_hash,
                findings_sha256,
                workpaper_attachment: Some(wp_json),
                workpaper_pdf: Some(wp_pdf),
            })
            .map_err(err)?;
        Ok(Json(AuditOut {
            status: "audited".into(),
            preview: Some(digest),
            head,
            checks: check_rows,
            summary: Some(summary),
            run: Some(run_out(&store, &run)?),
            findings: rows,
        }))
    }

    /// ```text
    /// GOAL: an audit run's workpaper, whole.
    /// SOURCE: plan/served.md P8 step 3; B-45.
    /// INVARIANTS:
    ///   1. Read-only; the JSON is the one filed, the PDF is handed over.
    /// ```
    #[tool(
        name = "books.workpaper",
        title = "An audit's workpaper",
        description = "The workpaper of an audit run (books.audit's run.id): what was checked, through which day and state of the books, and every finding, as the JSON that was filed, with the PDF handed over (run its download command to save it in a folder). Changes nothing.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn workpaper(
        &self,
        params: Parameters<WorkpaperParams>,
    ) -> Result<Json<WorkpaperOut>, String> {
        let store = self.lock()?;
        let run = store.audit_run(params.0.id).map_err(err)?.ok_or_else(|| {
            format!(
                "no audit run {}: books.attestations lists the ones attested",
                params.0.id
            )
        })?;
        let json_id = run
            .workpaper_attachment
            .ok_or("this audit run filed no workpaper")?;
        let bytes = store.attachment_bytes(json_id).map_err(err)?;
        let workpaper: serde_json::Value = serde_json::from_slice(&bytes).map_err(err)?;
        let pdf = match run.workpaper_pdf {
            Some(id) => {
                let bytes = store.attachment_bytes(id).map_err(err)?;
                Some(self.hand_over(
                    &book_name(&store),
                    &format!(
                        "workpaper-{}-{}-{}.pdf",
                        run.scope.key(),
                        run.through,
                        run.through_seq
                    ),
                    "application/pdf",
                    bytes,
                )?)
            }
            None => None,
        };
        Ok(Json(WorkpaperOut {
            run: run_out(&store, &run)?,
            workpaper,
            pdf,
        }))
    }

    /// ```text
    /// GOAL: a checker's claim that a receipt supports an entry.
    /// SOURCE: plan/served.md P8 step 4; B-45; D-9.
    /// INVARIANTS:
    ///   1. The reading is compared with the entry's amount and date; a
    ///      difference is recorded, marked, and never counts.
    ///   2. The method is the client's claim, self-reported.
    /// ```
    #[tool(
        name = "books.attest_receipt",
        title = "Attest a receipt",
        description = "Record that you read a receipt and it supports a posted entry: amount and date as you read them off it (get_receipt text, then page_image), and how you checked (method: model, reasoning_effort, adversarial, verifier_agents; your own claim, which the books record but cannot verify). The books compare the reading with the entry's amounts and date; a difference is recorded as disagrees and never counts. Whether it counts is weighed against policy.attestation_bar when asked.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn attest_receipt(
        &self,
        params: Parameters<AttestReceiptParams>,
    ) -> Result<Json<AttestOut>, String> {
        let p = params.0;
        let amount = parse_amount(&p.amount)?;
        let date = NaiveDate::parse_from_str(p.date.trim(), "%Y-%m-%d")
            .map_err(|_| format!("{:?} isn't a date: give YYYY-MM-DD", p.date))?;
        let claim = Claim {
            by: caller_name(),
            method: method_of(p.method)?,
            note: p.note,
        };
        let mut store = self.lock()?;
        let a = store
            .attest_receipt(p.attachment_id, p.entry_id, amount, date, claim)
            .map_err(err)?;
        Ok(Json(attest_out(&store, &a)?))
    }

    /// ```text
    /// GOAL: a checker's claim that it reviewed an entry whole.
    /// SOURCE: plan/served.md P8 step 4; B-45.
    /// ```
    #[tool(
        name = "books.review_entry",
        title = "Review an entry",
        description = "Record that you reviewed a posted entry whole: its lines, accounts, party and evidence, and how you checked (method, as for books.attest_receipt; your own claim). Changes no figure.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn review_entry(
        &self,
        params: Parameters<ReviewEntryParams>,
    ) -> Result<Json<AttestOut>, String> {
        let p = params.0;
        let claim = Claim {
            by: caller_name(),
            method: method_of(p.method)?,
            note: p.note,
        };
        let mut store = self.lock()?;
        let a = store.review_entry(p.entry_id, claim).map_err(err)?;
        Ok(Json(attest_out(&store, &a)?))
    }

    /// ```text
    /// GOAL: a checker's claim that a period is right, resting on an
    ///       audit run at the books' current head.
    /// SOURCE: plan/served.md P8 step 4; B-45; D-9; IZ-1k.
    /// INVARIANTS:
    ///   1. Refused unless an audit through the period's end stands at
    ///      the current head.
    /// ```
    #[tool(
        name = "books.attest_period",
        title = "Attest a period",
        description = "Record that you checked a period (its id or name from books.list_periods), resting on an audit (books.audit) through its end that still stands: refused when an entry, document, close, settings or role change has been made since the audit, or no audit covers it. Say how you checked (method, as for books.attest_receipt; your own claim). A year-level audit (annual, cpa_handoff, full) is held to the whole policy.attestation_bar: reasoning effort, adversarial, and the number of independent verifier agents.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn attest_period(
        &self,
        params: Parameters<AttestPeriodParams>,
    ) -> Result<Json<AttestOut>, String> {
        let p = params.0;
        let claim = Claim {
            by: caller_name(),
            method: method_of(p.method)?,
            note: p.note,
        };
        let mut store = self.lock()?;
        let named = p.period.trim();
        let period = store
            .ledger()
            .list_periods()
            .into_iter()
            .find(|q| q.name == named || q.id.to_string() == named)
            .map(|q| q.id)
            .ok_or_else(|| format!("no period {named:?}: books.list_periods lists them"))?;
        let (a, _) = store.attest_period(period, claim).map_err(err)?;
        Ok(Json(attest_out(&store, &a)?))
    }

    /// ```text
    /// GOAL: every claim that someone checked, and whether it counts.
    /// SOURCE: plan/served.md P8 step 4; B-45; D-9.
    /// INVARIANTS:
    ///   1. `counted` is read against the bar as it stands now.
    /// ```
    #[tool(
        name = "books.attestations",
        title = "Attestations",
        description = "List the attestations (receipts read, entries reviewed, periods attested), oldest first, each with who made it, how they say they checked, and counted: whether it meets policy.attestation_bar as the bar stands now (a change to the bar changes what counts, earlier claims too). through limits to entries dated, and periods ending, on or before a day; kind to receipt, entry or period. Changes nothing.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn attestations(
        &self,
        params: Parameters<AttestationsParams>,
    ) -> Result<Json<AttestationsOut>, String> {
        let p = params.0;
        let through = p
            .through
            .as_deref()
            .map(|t| self.as_of(Some(t)))
            .transpose()?;
        if let Some(k) = p.kind.as_deref()
            && !["receipt", "entry", "period"].contains(&k)
        {
            return Err("kind is receipt, entry or period".into());
        }
        let store = self.lock()?;
        let ledger = store.ledger();
        let bar = ledger.settings().attestation_bar();
        let mut out = Vec::new();
        for a in store.attestations().map_err(err)? {
            if p.kind.as_deref().is_some_and(|k| k != a.kind()) {
                continue;
            }
            if let Some(t) = through {
                let on = match a.what {
                    Attested::Receipt { entry_id, .. } | Attested::Entry { entry_id } => {
                        ledger.get_entry(entry_id).map(|e| e.date())
                    }
                    Attested::Period { period_id, .. } => {
                        ledger.get_period(period_id).map(|q| q.end)
                    }
                };
                if on.is_none_or(|d| d > t) {
                    continue;
                }
            }
            out.push(attestation_out(&store, &a, &bar)?);
        }
        Ok(Json(AttestationsOut {
            bar: bar.describe(),
            counted: out.iter().filter(|a| a.counted).count(),
            attestations: out,
        }))
    }
}

impl BooksServer {
    /// The findings of a run, as its filed workpaper holds them.
    fn stored_findings(
        &self,
        store: &Store,
        run: &AuditRun,
    ) -> Result<Vec<AuditFindingOut>, String> {
        let Some(id) = run.workpaper_attachment else {
            return Ok(Vec::new());
        };
        let bytes = store.attachment_bytes(id).map_err(err)?;
        let paper: serde_json::Value = serde_json::from_slice(&bytes).map_err(err)?;
        serde_json::from_value(paper["findings"].clone()).map_err(err)
    }
}

fn summary_of(findings: &[AuditFindingOut]) -> String {
    let n = |s: &str| findings.iter().filter(|f| f.severity == s).count();
    format!(
        "{} failure(s), {} warning(s), {} note(s)",
        n("failure"),
        n("warning"),
        n("info")
    )
}
