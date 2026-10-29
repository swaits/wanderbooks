//! Bank statements and reconciliations through the server: recording a
//! statement, clearing lines, reconciling, and voiding either. Split
//! whole out of `server.rs` (plan/served.md 8.2 U0a).

use super::*;

/// A recorded statement for a tool's output.
fn statement_out(store: &Store, st: &ledger_store::Statement) -> Result<StatementOut, String> {
    let (account_code, account_name) = code_name(store, st.account_id);
    let file_name = find_attachment(store, st.attachment_id)?.file_name;
    let reconciliation_id = store
        .reconciliations(Some(st.account_id))
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|r| r.attachment_id == st.attachment_id && r.voided.is_none())
        .map(|r| r.id);
    Ok(StatementOut {
        attachment_id: st.attachment_id,
        file_name,
        account_code,
        account_name,
        period_start: st.period_start.to_string(),
        period_end: st.period_end.to_string(),
        opening_balance: money(st.opening_balance),
        closing_balance: money(st.closing_balance),
        reconciliation_id,
    })
}

/// A recorded reconciliation for a tool's output.
fn reconciliation_out(store: &Store, r: &ledger_store::Reconciliation) -> ReconciliationOut {
    let (account_code, account_name) = code_name(store, r.account_id);
    ReconciliationOut {
        reconciliation_id: r.id,
        account_code,
        account_name,
        statement_date: r.statement_date.to_string(),
        statement_balance: money(r.statement_balance),
        statement_attachment: r.attachment_id,
        cleared: r
            .cleared
            .iter()
            .map(|&(entry_id, line_index)| ClearedRefOut {
                entry_id,
                line_index,
            })
            .collect(),
        voided: r.voided.is_some(),
        voided_reason: r.voided.clone(),
    }
}

/// A line of a reconciliation report.
fn reconcile_line(
    entry_id: u64,
    line_index: usize,
    date: NaiveDate,
    description: &str,
    side: Side,
    amount: Decimal,
) -> ReconcileLineOut {
    ReconcileLineOut {
        entry_id,
        line_index,
        date: date.to_string(),
        description: description.to_string(),
        side: match side {
            Side::Debit => "debit",
            Side::Credit => "credit",
        }
        .to_string(),
        amount: money(amount),
    }
}

#[tool_router(router = reconciliation_router, vis = "pub(super)")]
impl BooksServer {
    #[tool(
        name = "books.mark_cleared",
        title = "Mark a line cleared",
        description = "Mark a posted entry's line as cleared (or uncleared) for bank reconciliation. Only posted lines can be marked.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn mark_cleared(
        &self,
        params: Parameters<MarkClearedParams>,
    ) -> Result<Json<MarkClearedOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        store
            .mark_cleared(p.entry_id, p.line_index, p.cleared)
            .map_err(|e| e.to_string())?;
        Ok(Json(MarkClearedOut { ok: true }))
    }

    /// ```text
    /// GOAL: a monthly reconciliation that leaves proof: which statement,
    ///       which balance, which lines it cleared.
    /// SOURCE: CONVENTION (bank reconciliation); docs/parties-and-
    ///         attachments.md "Reconciliations"; Store::complete_reconciliation.
    /// INVARIANTS:
    ///   1. Without confirmed=true nothing is recorded: the preview reports
    ///      the difference, the cleared and uncleared lines, and the
    ///      problems a confirmation would hit.
    ///   2. A reconciliation is recorded only against a recorded statement
    ///      (statement_attachment), only at a difference of exactly zero,
    ///      and only after the account's last reconciliation not voided.
    ///   3. A refusal journals nothing.
    /// SCENARIOS:
    ///   1. IF statement_attachment is given: the account, date, and
    ///      balance are the statement's; any given that disagree: SHOULD
    ///      refuse. IF its attachment is withdrawn (the statement retired)
    ///      or its statement voided: SHOULD refuse, preview or not.
    ///   2. IF it isn't: account, statement_date, and statement_balance are
    ///      required, and confirmed=true SHOULD refuse.
    ///   3. IF confirmed=true and problems is not empty: SHOULD refuse,
    ///      naming them.
    /// ```
    #[tool(
        name = "books.reconcile",
        title = "Reconcile an account",
        description = "Reconcile a bank, cash or card account against a statement: give statement_attachment (from books.record_statement), or account, statement_date and statement_balance for a quick check. Returns the book, cleared and statement balances, the difference (cleared minus statement; zero means reconciled), the cleared and uncleared lines and the problems that would stop it. Without confirmed it previews. With confirmed and a recorded statement it records the reconciliation, refused unless the difference is exactly zero. Mark lines first with books.mark_cleared.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn reconcile(
        &self,
        params: Parameters<ReconcileParams>,
    ) -> Result<Json<ReconcileOut>, String> {
        let p = params.0;
        let confirmed = p.confirmed.unwrap_or(false);
        let mut store = self.lock()?;
        let given_account = p
            .account
            .as_deref()
            .map(|a| resolve_account(&store, a))
            .transpose()?;
        let given_date = p.statement_date.as_deref().map(parse_date).transpose()?;
        let given_balance = p
            .statement_balance
            .as_deref()
            .map(parse_amount)
            .transpose()?;
        let (account, date, balance) = match p.statement_attachment {
            Some(att) => {
                if find_attachment(&store, att)?.withdrawn_at.is_some() {
                    return Err(format!(
                        "attachment {att} is withdrawn, so its statement is retired: file the statement again and record it (books.record_statement)"
                    ));
                }
                let st = store
                    .statement(att)
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| {
                        format!(
                            "attachment {att} records no statement: record it with books.record_statement first"
                        )
                    })?;
                if let Some(a) = given_account.filter(|a| *a != st.account_id) {
                    return Err(format!(
                        "statement {att} is for account {}, not {}",
                        code_name(&store, st.account_id).0,
                        code_name(&store, a).0
                    ));
                }
                if let Some(d) = given_date.filter(|d| *d != st.period_end) {
                    return Err(format!("statement {att} ends {}, not {d}", st.period_end));
                }
                if let Some(b) = given_balance.filter(|b| *b != st.closing_balance) {
                    return Err(format!(
                        "statement {att} closes at {}, not {}",
                        money(st.closing_balance),
                        money(b)
                    ));
                }
                (st.account_id, st.period_end, st.closing_balance)
            }
            None => {
                if confirmed {
                    return Err("a reconciliation is recorded only against a recorded statement: file it (books.file_document category statement), record what it says (books.record_statement), then pass statement_attachment".into());
                }
                match (given_account, given_date, given_balance) {
                    (Some(a), Some(d), Some(b)) => (a, d, b),
                    _ => {
                        return Err("give statement_attachment, or account, statement_date, and statement_balance".into());
                    }
                }
            }
        };
        let report = store
            .ledger()
            .reconciliation_report(account, date, balance)
            .map_err(|e| e.to_string())?;
        let ledger = store.ledger();
        let mut cleared_lines = Vec::new();
        for e in ledger.posted_entries() {
            if e.date() > date {
                continue;
            }
            for (i, l) in e.lines().iter().enumerate() {
                if l.account == account && ledger.is_cleared(e.id(), i) {
                    cleared_lines.push(reconcile_line(
                        e.id(),
                        i,
                        e.date(),
                        e.description(),
                        l.side,
                        l.amount.amount,
                    ));
                }
            }
        }
        cleared_lines.sort_by(|a, b| {
            a.date
                .cmp(&b.date)
                .then(a.entry_id.cmp(&b.entry_id))
                .then(a.line_index.cmp(&b.line_index))
        });
        let mut problems = Vec::new();
        if p.statement_attachment.is_none() {
            problems.push("no recorded statement: file it, books.record_statement, then pass statement_attachment".to_string());
        }
        if !report.difference.is_zero() {
            problems.push(format!(
                "the cleared balance differs from the statement by {}: mark the lines the statement shows with books.mark_cleared (or find what's missing)",
                money(report.difference)
            ));
        }
        if let Some(last) = reconciled_through(&store, account)?
            && last >= date
        {
            problems.push(format!(
                "already reconciled through {last}: void that reconciliation (books.void_reconciliation) to redo it"
            ));
        }
        let mut out = ReconcileOut {
            status: "preview".to_string(),
            account_code: code_name(&store, account).0,
            statement_date: date.to_string(),
            statement_attachment: p.statement_attachment,
            book_balance: money(report.book_balance),
            cleared_balance: money(report.cleared_balance),
            statement_balance: money(report.statement_balance),
            difference: money(report.difference),
            uncleared_lines: report
                .uncleared_lines
                .into_iter()
                .map(|l| {
                    reconcile_line(
                        l.entry_id,
                        l.line_index,
                        l.date,
                        &l.description,
                        l.side,
                        l.amount,
                    )
                })
                .collect(),
            cleared_lines,
            problems,
            reconciliation: None,
            preview: None,
        };
        let digest = self.preview_of(&store, "books.reconcile", confirmed, p.preview.as_deref())?;
        out.preview = Some(digest);
        if !confirmed {
            return Ok(Json(out));
        }
        if !out.problems.is_empty() {
            return Err(format!("not recorded: {}", out.problems.join("; ")));
        }
        let att = p.statement_attachment.expect("required when confirmed");
        let r = store
            .complete_reconciliation(att)
            .map_err(|e| e.to_string())?;
        out.status = "reconciled".to_string();
        out.reconciliation = Some(reconciliation_out(&store, &r));
        Ok(Json(out))
    }

    /// ```text
    /// GOAL: the bank and card statements are in the books, so a
    ///       reconciliation names the one it agrees with.
    /// SOURCE: Store::record_statement.
    /// INVARIANTS:
    ///   1. Journaled as `statement.recorded`, then `attachment.linked` to
    ///      the account (unless linked already). A refusal journals nothing.
    /// SCENARIOS:
    ///   1. IF the attachment is unknown, withdrawn, or already records a
    ///      statement; the account is unknown or not an asset or liability;
    ///      the period runs backwards; or a balance isn't a decimal string
    ///      in cents: SHOULD refuse.
    /// ```
    #[tool(
        name = "books.record_statement",
        title = "Record a statement",
        description = "Record what a filed bank or card statement says (file it first: books.file_document category statement): the account it is for (code or id; an asset or liability), the period it covers (period_start, period_end: the statement date), and its opening and closing balances as printed, in the account's normal direction (what the bank holds; what the card is owed), decimal strings. Links the statement to the account. Then reconcile to it: books.reconcile statement_attachment. Journaled as statement.recorded. Returns the statement.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn record_statement(
        &self,
        params: Parameters<RecordStatementParams>,
    ) -> Result<Json<StatementOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        find_attachment(&store, p.attachment_id)?;
        let st = ledger_store::Statement {
            attachment_id: p.attachment_id,
            account_id: resolve_account(&store, &p.account)?,
            period_start: parse_date(&p.period_start)?,
            period_end: parse_date(&p.period_end)?,
            opening_balance: parse_amount(&p.opening_balance)?,
            closing_balance: parse_amount(&p.closing_balance)?,
        };
        store
            .record_statement(st.clone())
            .map_err(|e| e.to_string())?;
        Ok(Json(statement_out(&store, &st)?))
    }

    #[tool(
        name = "books.statements",
        title = "List statements",
        description = "Recorded bank and card statements (books.record_statement), optionally one account's (code or id), by statement date: the attachment, account, period, opening and closing balances, and the reconciliation made against it, if any.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn statements(
        &self,
        params: Parameters<AccountFilterParams>,
    ) -> Result<Json<StatementsOut>, String> {
        let store = self.lock()?;
        let account = params
            .0
            .account
            .as_deref()
            .map(|a| resolve_account(&store, a))
            .transpose()?;
        let statements = store
            .statements(account)
            .map_err(|e| e.to_string())?
            .iter()
            .map(|st| statement_out(&store, st))
            .collect::<Result<_, _>>()?;
        Ok(Json(StatementsOut { statements }))
    }

    #[tool(
        name = "books.reconciliations",
        title = "List reconciliations",
        description = "Recorded reconciliations (books.reconcile confirmed), optionally one account's (code or id), by statement date: the account, the statement date and balance, the statement's attachment, the lines it found cleared, and whether it was voided (and why).",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn reconciliations(
        &self,
        params: Parameters<AccountFilterParams>,
    ) -> Result<Json<ReconciliationsOut>, String> {
        let store = self.lock()?;
        let account = params
            .0
            .account
            .as_deref()
            .map(|a| resolve_account(&store, a))
            .transpose()?;
        let reconciliations = store
            .reconciliations(account)
            .map_err(|e| e.to_string())?
            .iter()
            .map(|r| reconciliation_out(&store, r))
            .collect();
        Ok(Json(ReconciliationsOut { reconciliations }))
    }

    /// ```text
    /// GOAL: correct a mis-recorded statement on the record.
    /// SOURCE: Store::void_statement.
    /// SCENARIOS:
    ///   1. IF the attachment records no statement, a reconciliation not
    ///      voided rests on it, or the reason is blank: SHOULD refuse,
    ///      journaling nothing.
    ///   2. Otherwise: journaled as `statement.voided`; books.statements no
    ///      longer lists it, and the same attachment can record the
    ///      statement again (books.record_statement).
    /// ```
    #[tool(
        name = "books.void_statement",
        title = "Void a statement",
        description = "Void a recorded statement that was recorded wrong (a mistyped balance or period), with the reason recorded in the journal, so the same filed statement can be recorded again with books.record_statement. Refused while a reconciliation not voided rests on it (books.void_reconciliation first). Nothing is erased: the journal keeps what it said. Journaled as statement.voided. Returns the statement as it was recorded.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn void_statement(
        &self,
        params: Parameters<VoidStatementParams>,
    ) -> Result<Json<StatementOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let st = store
            .statement(p.attachment_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("attachment {} records no statement", p.attachment_id))?;
        let out = statement_out(&store, &st)?;
        store
            .void_statement(p.attachment_id, &p.reason)
            .map_err(|e| e.to_string())?;
        Ok(Json(out))
    }

    /// ```text
    /// GOAL: undo a mistaken reconciliation on the record, to redo it.
    /// SOURCE: Store::void_reconciliation.
    /// SCENARIOS:
    ///   1. IF it is unknown or already voided, or the reason is blank:
    ///      SHOULD refuse, journaling nothing.
    ///   2. Otherwise: journaled as `reconciliation.voided`; it stays
    ///      listed, marked voided, and the account can be reconciled
    ///      through that date again.
    /// ```
    #[tool(
        name = "books.void_reconciliation",
        title = "Void a reconciliation",
        description = "Void a recorded reconciliation that was a mistake, with the reason recorded in the journal, so the account can be reconciled through that date again. Nothing is erased: it stays listed in books.reconciliations, marked voided. Journaled as reconciliation.voided. Returns it.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn void_reconciliation(
        &self,
        params: Parameters<VoidReconciliationParams>,
    ) -> Result<Json<ReconciliationOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        store
            .void_reconciliation(p.reconciliation_id, &p.reason)
            .map_err(|e| e.to_string())?;
        let r = store
            .reconciliations(None)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|r| r.id == p.reconciliation_id)
            .ok_or_else(|| format!("unknown reconciliation {}", p.reconciliation_id))?;
        Ok(Json(reconciliation_out(&store, &r)))
    }
}
