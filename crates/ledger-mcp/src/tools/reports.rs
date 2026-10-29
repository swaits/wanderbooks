//! The books' reports through the server: the statements,
//! `books.show`, `books.verify`, `books.digest`, `books.export`, the CPA
//! package and the P&L by class, with the export builders they call.
//! Split whole out of `server.rs` (plan/served.md 8.2 U0a).

use super::*;

/// What the digest lists from the control check: each line on a control
/// account that is no document's, and each document needing attention.
fn control_problems(out: &ControlCheckOut) -> Vec<String> {
    let mut v = Vec::new();
    for a in &out.accounts {
        for u in &a.unattached {
            v.push(format!(
                "{} {}: entry {} line {} ({}, {}) is no invoice's or bill's",
                a.code, a.name, u.entry_id, u.line, u.date, u.amount
            ));
        }
        for d in &a.attention {
            v.push(format!(
                "{} {}: {} {} is {} (total {}, owes {})",
                a.code, a.name, d.kind, d.id, d.status, d.total, d.outstanding
            ));
        }
    }
    v
}

/// An account's net movement over `entries` in its normal direction.
fn movement_over(
    ledger: &Ledger,
    account: AccountId,
    entries: &[&ledger_core::JournalEntry],
) -> Decimal {
    let movement: Decimal = entries
        .iter()
        .flat_map(|e| e.lines())
        .filter(|l| l.account == account)
        .map(|l| match l.side {
            Side::Debit => l.amount.amount,
            Side::Credit => -l.amount.amount,
        })
        .sum();
    let account = ledger.chart().get(account).expect("account exists");
    match account.account_type.normal_side() {
        Side::Debit => movement,
        Side::Credit => -movement,
    }
}

fn lines_out(lines: &[ledger_core::StatementLine]) -> Vec<StatementLineOut> {
    lines
        .iter()
        .map(|l| StatementLineOut {
            code: l.code.clone(),
            name: l.name.clone(),
            amount: money(l.amount),
        })
        .collect()
}

/// `books.cash_flow`'s answer, and `books.export`'s cash flow JSON.
fn cash_flow_out(stmt: &ledger_core::CashFlowStatement) -> CashFlowOut {
    use ledger_core::CashFlowSection as S;
    let line = |l: &ledger_core::CashFlowLine| CashFlowLineOut {
        entry: l.entry,
        date: l.date.to_string(),
        label: l.label.clone(),
        kind: l.kind.clone(),
        amount: money(l.amount),
        on_behalf: (!l.on_behalf.is_zero()).then(|| money(l.on_behalf)),
        accounts: l.accounts.clone(),
        reverses: l.reverses,
        class: l.class.map(|c| c.key().to_string()),
    };
    let kinds = |sec: S| {
        stmt.kinds(sec)
            .into_iter()
            .map(|k| CashFlowKindOut {
                kind: k.kind,
                class: k.class.map(|c| c.key().to_string()),
                amount: money(k.amount),
                entries: k.entries,
            })
            .collect()
    };
    let r = &stmt.operating_reconciliation;
    CashFlowOut {
        as_of_status: None,
        method: "direct".to_string(),
        start: stmt.start.to_string(),
        end: stmt.end.to_string(),
        operating_by_class: kinds(S::Operating),
        operating: stmt.operating.iter().map(line).collect(),
        cash_from_operations: money(stmt.cash_from_operations),
        investing_by_kind: kinds(S::Investing),
        investing: stmt.investing.iter().map(line).collect(),
        cash_from_investing: money(stmt.cash_from_investing),
        financing_by_kind: kinds(S::Financing),
        financing: stmt.financing.iter().map(line).collect(),
        cash_from_financing: money(stmt.cash_from_financing),
        net_change_in_cash: money(stmt.net_change_in_cash),
        cash_beginning: money(stmt.cash_beginning),
        cash_ending: money(stmt.cash_ending),
        noncash_disclosures: stmt
            .noncash_disclosures
            .iter()
            .map(|n| NoncashDisclosureOut {
                entry: n.entry,
                date: n.date.to_string(),
                label: n.label.clone(),
                amount: money(n.amount),
                operating: money(n.operating),
                investing: money(n.investing),
                financing: money(n.financing),
                exchanged: money(n.exchanged),
                reverses: n.reverses,
            })
            .collect(),
        operating_reconciliation: OperatingReconciliationOut {
            net_income: money(r.net_income),
            lines: r
                .lines
                .iter()
                .map(|l| ReconcilingLineOut {
                    kind: l.kind.key().to_string(),
                    label: l.label.clone(),
                    amount: money(l.amount),
                    account: l.account.clone(),
                    entries: l.entries.clone(),
                })
                .collect(),
            net_cash_from_operating: money(r.net_cash_from_operating),
        },
        reconciles: CashFlowFootingOut {
            sections_sum: money(stmt.reconciles.sections_sum),
            net_change: money(stmt.reconciles.net_change),
            difference: money(stmt.reconciles.difference),
        },
        warnings: stmt
            .warnings
            .iter()
            .map(|w| CashFlowWarningOut {
                entry: w.entry,
                kind: w.kind.key().to_string(),
                message: w.message.clone(),
            })
            .collect(),
    }
}

/// `books.export`'s cash flow CSV:
/// `section,kind,class,entry,date,label,amount,on_behalf`, every line of
/// every section (`on_behalf`: of its amount, what a member or lender
/// paid or received for the company, D-42), the reconciliation, the
/// noncash disclosures, the footing and the warnings (`warning`, its
/// kind, its entry and its message; no amount). Only a section's lines
/// have an `on_behalf`.
fn cash_flow_csv(stmt: &ledger_core::CashFlowStatement) -> String {
    use ledger_core::CashFlowSection as S;
    let mut csv = String::from("section,kind,class,entry,date,label,amount,on_behalf\n");
    let mut row = |section: &str,
                   kind: &str,
                   class: &str,
                   entry: Option<u64>,
                   date: Option<chrono::NaiveDate>,
                   label: &str,
                   amount: rust_decimal::Decimal,
                   on_behalf: Option<rust_decimal::Decimal>| {
        csv.push_str(&format!(
            "{section},{},{class},{},{},{},{},{}\n",
            csv_escape(kind),
            entry.map_or(String::new(), |e| e.to_string()),
            date.map_or(String::new(), |d| d.to_string()),
            csv_escape(label),
            money(amount),
            on_behalf.map_or(String::new(), money)
        ));
    };
    for (sec, total) in [
        (S::Operating, "Net cash from operating activities"),
        (S::Investing, "Net cash from investing activities"),
        (S::Financing, "Net cash from financing activities"),
    ] {
        for l in stmt.lines(sec) {
            let class = l.class.map_or("", |c| c.key());
            row(
                sec.key(),
                &l.kind,
                class,
                Some(l.entry),
                Some(l.date),
                &l.label,
                l.amount,
                Some(l.on_behalf),
            );
        }
        row(sec.key(), "", "", None, None, total, stmt.total(sec), None);
    }
    let r = &stmt.operating_reconciliation;
    row(
        "reconciliation",
        "",
        "",
        None,
        None,
        "Net income (loss)",
        r.net_income,
        None,
    );
    for l in &r.lines {
        row(
            "reconciliation",
            l.kind.key(),
            "",
            None,
            None,
            &l.label,
            l.amount,
            None,
        );
    }
    row(
        "reconciliation",
        "",
        "",
        None,
        None,
        "Net cash from operating activities",
        r.net_cash_from_operating,
        None,
    );
    for n in &stmt.noncash_disclosures {
        row(
            "noncash",
            "",
            "",
            Some(n.entry),
            Some(n.date),
            &n.label,
            n.amount,
            None,
        );
    }
    let f = &stmt.reconciles;
    for (label, amount) in [
        ("Net increase (decrease) in cash", stmt.net_change_in_cash),
        ("Cash at beginning of period", stmt.cash_beginning),
        ("Cash at end of period", stmt.cash_ending),
        ("Sections sum", f.sections_sum),
        ("Difference", f.difference),
    ] {
        row("footing", "", "", None, None, label, amount, None);
    }
    // The warnings, as the tool gives them: no amount (figures are
    // unaffected).
    for w in &stmt.warnings {
        csv.push_str(&format!(
            "warning,{},,{},,{},,\n",
            w.kind.key(),
            w.entry,
            csv_escape(&w.message)
        ));
    }
    csv
}

#[tool_router(router = reports_router, vis = "pub(super)")]
impl BooksServer {
    #[tool(
        name = "books.trial_balance",
        title = "Trial balance",
        description = described(
            "The trial balance as of a date: every account with a non-zero balance, debit and credit columns and totals. Debits must equal credits.",
            ledger_core::calendar::PeriodKind::AsOf
        ),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub(super) async fn trial_balance(
        &self,
        params: Parameters<TrialBalanceParams>,
    ) -> Result<Json<TrialBalanceOut>, String> {
        let store = self.lock()?;
        let as_of = self.as_of(params.0.as_of.as_deref())?;
        let as_of_status = self.report_status(&store, as_of, params.0.strict.unwrap_or(false))?;
        let tb = reports::trial_balance_as_of(store.ledger(), as_of);
        Ok(Json(TrialBalanceOut {
            as_of_status,
            as_of: as_of.to_string(),
            lines: tb
                .lines
                .iter()
                .map(|l| TrialBalanceLineOut {
                    code: l.code.clone(),
                    name: l.name.clone(),
                    debit: money(l.debit),
                    credit: money(l.credit),
                })
                .collect(),
            total_debits: money(tb.total_debits),
            total_credits: money(tb.total_credits),
            balanced: tb.balanced(),
        }))
    }

    #[tool(
        name = "books.profit_and_loss",
        title = "Profit and loss",
        description = described(
            "Profit and loss for a period: revenue, expenses, net income. Pass class (a project: name or id; see books.projects) to filter to one project.",
            ledger_core::calendar::PeriodKind::Statement
        ),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub(super) async fn profit_and_loss(
        &self,
        params: Parameters<ProfitAndLossParams>,
    ) -> Result<Json<ProfitAndLossOut>, String> {
        let p = params.0;
        let store = self.lock()?;
        let class_id = match &p.class {
            Some(c) => Some(resolve_class(&store, c)?),
            None => None,
        };
        let pl = self.pnl_statement(
            store.ledger(),
            p.start.as_deref(),
            p.end.as_deref(),
            class_id,
        )?;
        let as_of_status = self.report_status(&store, pl.end, p.strict.unwrap_or(false))?;
        Ok(Json(ProfitAndLossOut {
            as_of_status,
            start: pl.start.to_string(),
            end: pl.end.to_string(),
            revenue: money(pl.total_revenue),
            expenses: money(pl.total_expenses),
            net_income: money(pl.net_income()),
            revenue_accounts: lines_out(&pl.revenue),
            expense_accounts: lines_out(&pl.expenses),
        }))
    }

    #[tool(
        name = "books.balance_sheet",
        title = "Balance sheet",
        description = described(
            "The balance sheet as of a date: assets, liabilities, equity (including current earnings).",
            ledger_core::calendar::PeriodKind::AsOf
        ),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub(super) async fn balance_sheet(
        &self,
        params: Parameters<BalanceSheetParams>,
    ) -> Result<Json<BalanceSheetOut>, String> {
        let p = params.0;
        let store = self.lock()?;
        let bs = self.bs_statement(store.ledger(), p.as_of.as_deref())?;
        let as_of_status = self.report_status(&store, bs.as_of, p.strict.unwrap_or(false))?;
        Ok(Json(BalanceSheetOut {
            as_of_status,
            as_of: bs.as_of.to_string(),
            assets: money(bs.total_assets),
            liabilities: money(bs.total_liabilities),
            equity: money(bs.total_equity),
            current_earnings: money(bs.current_earnings),
            balanced: bs.balanced(),
            asset_accounts: lines_out(&bs.assets),
            liability_accounts: lines_out(&bs.liabilities),
            equity_accounts: lines_out(&bs.equity),
        }))
    }

    #[tool(
        name = "books.cash_flow",
        title = "Cash flow statement",
        description = described(
            "Cash flows, direct method (ASC 230): each entry that moved cash, gross, by section and kind (operating by ASC 230-10-45-25 class; investing and financing by kind, such as a member's contributions); an entry and its reversal in one period drop out. What a member or lender paid for the company is borrowed and spent that day (on_behalf). Also noncash activities, net income reconciled to operating cash, and warnings. Sections sum to the change in cash. Roles default to the settings.",
            ledger_core::calendar::PeriodKind::Statement
        ),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn cash_flow(
        &self,
        params: Parameters<CashFlowParams>,
    ) -> Result<Json<CashFlowOut>, String> {
        let p = params.0;
        let store = self.lock()?;
        let ledger = store.ledger();
        let stmt = self.cash_flow_for(
            ledger,
            p.start.as_deref(),
            p.end.as_deref(),
            &p.cash_accounts,
            &p.financing_accounts,
            &p.investing_accounts,
        )?;
        let as_of_status = self.report_status(&store, stmt.end, p.strict.unwrap_or(false))?;
        let mut out = cash_flow_out(&stmt);
        out.as_of_status = as_of_status;
        Ok(Json(out))
    }

    #[tool(
        name = "books.show",
        title = "Show in the viewer",
        description = "Show something in the human's viewer (the read-only desktop app). Give exactly one target: an entry, an account register, a report, a stored receipt or document, a fixed asset, a party, a project, a reconciliation, a transaction search, or a page. Work out dates yourself (\"last quarter\" is its first and last day). Use it when the human asks to see something, and after posting so they can look at what you did. Changes nothing in the books. The targets and their options are in books.handbook section viewer.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn show(&self, params: Parameters<ShowParams>) -> Result<Json<ShowOut>, String> {
        use ledger_store::ops::{ViewerTarget, request, viewer_open};
        let p = params.0;
        let store = self.lock()?;
        let ledger = store.ledger();
        let given = [
            p.entry_id.is_some(),
            p.account.is_some(),
            p.report.is_some(),
            p.attachment_id.is_some(),
            p.asset.is_some(),
            p.party.is_some(),
            p.project.is_some(),
            p.reconciliation.is_some(),
            p.search.is_some(),
            p.page.is_some(),
        ]
        .iter()
        .filter(|x| **x)
        .count();
        if given != 1 {
            return Err(
                "give exactly one of entry_id, account, report, attachment_id, asset, party, project, reconciliation, search, page"
                    .into(),
            );
        }
        let start = p.start.as_deref().map(parse_date).transpose()?;
        let end = p.end.as_deref().map(parse_date).transpose()?;
        // Options only an account or a report takes are refused anywhere
        // else (the viewer would ignore them, and the agent believe it
        // showed what it didn't).
        let dated = p.account.is_some() || p.report.is_some();
        let options: Vec<&str> = [
            ("start", p.start.is_some()),
            ("end", p.end.is_some()),
            ("columns", p.columns.is_some()),
            ("compare", p.compare.is_some()),
        ]
        .into_iter()
        .filter(|(_, given)| *given)
        .map(|(name, _)| name)
        .collect();
        if !dated && !options.is_empty() {
            return Err(format!(
                "{} {} for an account or a report; this target takes none",
                options.join(" and "),
                if options.len() == 1 { "is" } else { "are" }
            ));
        }
        if p.account.is_some() && (p.columns.is_some() || p.compare.is_some()) {
            return Err(
                "columns and compare are for a report; an account's register takes only start and end"
                    .into(),
            );
        }
        const REPORTS: [&str; 11] = [
            "profit_and_loss",
            "balance_sheet",
            "cash_flow",
            "members_equity",
            "trial_balance",
            "general_ledger",
            "tax_summary",
            "vendor_1099",
            "ar_aging",
            "ap_aging",
            "fixed_asset_register",
        ];
        const PAGES: [&str; 12] = [
            "overview",
            "cash",
            "profit",
            "transactions",
            "reports",
            "accounts",
            "assets",
            "parties",
            "projects",
            "documents",
            "company",
            "journal",
        ];
        let (target, shown) = if let Some(r) = &p.asset {
            let id = resolve_asset_ref(&store, r)?;
            let a = ledger.get_asset(id).expect("resolved");
            (
                ViewerTarget::Asset { asset_id: id },
                format!(
                    "asset {}{}",
                    a.tag
                        .as_deref()
                        .map(|t| format!("{t}, "))
                        .unwrap_or_default(),
                    a.description
                ),
            )
        } else if let Some(r) = &p.party {
            let id = resolve_party(&store, r)?;
            let party = ledger.get_party(id).expect("resolved");
            (
                ViewerTarget::Party { party_id: id },
                format!("party {id}, {}", party.label()),
            )
        } else if let Some(r) = &p.project {
            let id = resolve_class(&store, r)?;
            let c = ledger.get_class(id).expect("resolved");
            (
                ViewerTarget::Project { class_id: id },
                format!("project {id}, {}", c.name),
            )
        } else if let Some(id) = p.reconciliation {
            let r = store
                .reconciliations(None)
                .map_err(|e| e.to_string())?
                .into_iter()
                .find(|r| r.id == id)
                .ok_or_else(|| format!("unknown reconciliation {id}"))?;
            let (code, name) = code_name(&store, r.account_id);
            (
                ViewerTarget::Reconciliation { id },
                format!(
                    "reconciliation {id}, {code} {name} through {}",
                    r.statement_date
                ),
            )
        } else if let Some(id) = p.entry_id {
            let e = ledger
                .get_entry(id)
                .ok_or_else(|| format!("unknown entry {id}"))?;
            (
                ViewerTarget::Entry { entry_id: id },
                format!("entry #{id}, {}", e.description()),
            )
        } else if let Some(code) = p.account {
            let a = ledger
                .chart()
                .find_by_code(&code)
                .ok_or_else(|| format!("unknown account {code}"))?;
            // The dates resolved (rules.md B-30), sent as resolved, so what
            // the agent is told and what the viewer shows can't differ.
            let (s, e) = self
                .periods()
                .statement(start, end)
                .map_err(|e| e.to_string())?;
            (
                ViewerTarget::Account {
                    code: code.clone(),
                    start: Some(s.to_string()),
                    end: Some(e.to_string()),
                },
                format!("the register of {} {}, {s} to {e}", a.code, a.name),
            )
        } else if let Some(report) = p.report {
            if !REPORTS.contains(&report.as_str()) {
                return Err(format!(
                    "report {report:?}: expected one of {}",
                    REPORTS.join(", ")
                ));
            }
            if let Some(c) = &p.columns
                && !["total", "month", "quarter", "year"].contains(&c.as_str())
            {
                return Err(format!(
                    "columns {c:?}: expected total, month, quarter, or year"
                ));
            }
            if let Some(c) = &p.compare
                && !["prior_period", "prior_year"].contains(&c.as_str())
            {
                return Err(format!(
                    "compare {c:?}: expected prior_period or prior_year"
                ));
            }
            let dates = show_report_dates(&report, p.columns.as_deref(), p.compare.as_deref())?;
            if p.start.is_some() {
                match dates {
                    ShownDates::Period => {}
                    ShownDates::AsOf => {
                        return Err(format!(
                            "{report}{} is as of a day: pass end (the day), not start",
                            if report == "balance_sheet" {
                                " in one total column"
                            } else {
                                ""
                            }
                        ));
                    }
                    ShownDates::Year => {
                        return Err(format!(
                            "{report} is for a calendar year: pass end (a day in that year), not start"
                        ));
                    }
                }
            }
            let (s, e) = self
                .periods()
                .statement(start, end)
                .map_err(|e| e.to_string())?;
            // A split past the viewer's columns is shown as one total (the
            // viewer says so): a split it would ignore.
            let per = match p.columns.as_deref() {
                Some("month") => Some(1),
                Some("quarter") => Some(3),
                Some("year") => Some(12),
                _ => None,
            };
            if let Some(per) = per {
                let n = ledger_store::ops::report_columns(s, e, per);
                let max = ledger_store::ops::MAX_REPORT_COLUMNS;
                if n > max {
                    return Err(format!(
                        "columns {} from {s} to {e} is {n} columns, and the viewer splits a report into at most {max} (past that it shows the total): pass a shorter range or a longer split",
                        p.columns.as_deref().unwrap_or_default()
                    ));
                }
            }
            let mut what = format!("the {} report", report.replace('_', " "));
            what.push_str(&match dates {
                ShownDates::Period => format!(", {s} to {e}"),
                ShownDates::AsOf => format!(", as of {e}"),
                ShownDates::Year => format!(", for {}", chrono::Datelike::year(&e)),
            });
            if let Some(c) = p.columns.as_deref().filter(|c| *c != "total") {
                what.push_str(&format!(", by {c}"));
            }
            if let Some(c) = &p.compare {
                what.push_str(&format!(", compared with the {}", c.replace('_', " ")));
            }
            (
                ViewerTarget::Report {
                    report: report.clone(),
                    start: Some(s.to_string()),
                    end: Some(e.to_string()),
                    columns: p.columns.clone(),
                    compare: p.compare.clone(),
                },
                what,
            )
        } else if let Some(id) = p.attachment_id {
            let a = store
                .list_attachments(None)
                .map_err(|e| e.to_string())?
                .into_iter()
                .find(|a| a.id == id)
                .ok_or_else(|| format!("unknown attachment {id}"))?;
            (
                ViewerTarget::Document { attachment_id: id },
                format!("document {id}, {}", a.file_name),
            )
        } else if let Some(query) = p.search {
            (
                ViewerTarget::Search {
                    query: query.clone(),
                },
                format!("transactions matching {query:?}"),
            )
        } else {
            let page = p.page.unwrap_or_default();
            if !PAGES.contains(&page.as_str()) {
                return Err(format!(
                    "page {page:?}: expected one of {}",
                    PAGES.join(", ")
                ));
            }
            (
                ViewerTarget::Page { page: page.clone() },
                format!("the {page} page"),
            )
        };
        request(store.data_dir(), target).map_err(|e| format!("couldn't reach the viewer: {e}"))?;
        // Whether a viewer has these books open (ledger_store::ops
        // INVARIANT 3): it shows it now; else it will when opened soon.
        let open = viewer_open(store.data_dir());
        Ok(Json(ShowOut {
            shown: shown.clone(),
            viewer_open: open,
            note: if open {
                format!("The viewer shows {shown}.")
            } else {
                format!(
                    "No viewer has these books open: ask the human to open it (opened within a couple of minutes, it shows {shown})."
                )
            },
        }))
    }

    #[tool(
        name = "books.verify",
        title = "Verify the books",
        description = "Check the books end to end: the journal's hash chain; a fresh replay against the live ledger; every table of the books' database, row for row, against a fresh projection of the journal (rebuilding the database from the journal if it drifted, keeping the old copy); and, by default, every stored receipt and document against its SHA-256. Run it whenever a number looks wrong or a human asks whether the books are intact.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn verify(&self, params: Parameters<VerifyParams>) -> Result<Json<VerifyOut>, String> {
        let mut store = self.lock()?;
        let report = store.verify_and_heal().map_err(|e| e.to_string())?;
        let blobs = if params.0.blobs {
            Some(store.verify_blobs().map_err(|e| e.to_string())?)
        } else {
            None
        };
        let mut checked = vec![
            format!("journal hash chain ({} events)", report.events),
            "fresh replay matches the live ledger (every statement, assets, classes, payroll)"
                .to_string(),
            format!(
                "SQLite database ({}) matches a fresh projection of the journal, every table, row for row",
                report.database
            ),
        ];
        if let Some(n) = blobs {
            checked.push(format!("{n} stored files match their SHA-256"));
        }
        // Receivables and payables equal their documents (rules.md B-38
        // INVARIANT 1): an identity, so a difference is an engine error.
        let checks = store
            .ledger()
            .control_check(chrono::NaiveDate::MAX)
            .map_err(|e| e.to_string())?;
        if let Some(c) = checks.iter().find(|c| !c.difference.is_zero()) {
            return Err(format!(
                "{}: its balance {} is not its documents' {} plus its other lines: an engine error, report it",
                c.role,
                money(c.balance),
                money(c.documents_total)
            ));
        }
        checked.push(format!(
            "receivables and payables equal their documents and other lines ({} control accounts)",
            checks.len()
        ));
        Ok(Json(VerifyOut {
            status: if report.healed.is_some() {
                "healed"
            } else {
                "ok"
            }
            .to_string(),
            events: report.events,
            head: report.head,
            healed: report.healed,
            blobs_verified: blobs,
            database: report.database,
            other_databases: report.other_databases,
            checked,
        }))
    }

    #[tool(
        name = "books.digest",
        title = "Activity digest",
        description = "What changed since a date: entries posted, per-account movement, and receipt coverage; plus what needs doing: rules due for review, settings problems, cash and card accounts not reconciled through the end of last month (unreconciled_accounts), revenue and expense accounts with no tax category (uncategorized_accounts), and receivables or payables lines that are no document's or documents needing attention (control_problems, books.control_check). Built for human review.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub(super) async fn digest(
        &self,
        params: Parameters<DigestParams>,
    ) -> Result<Json<DigestOut>, String> {
        let p = params.0;
        let since = p.since.as_deref().map(parse_date).transpose()?;
        if since.is_none() && p.since_seq.is_none() {
            return Err("give since (a date) or since_seq (a journal sequence number)".into());
        }
        let today = today();
        let as_of = p.as_of.as_deref().map(parse_date).transpose()?;
        let end = as_of.unwrap_or(today);
        let store = self.lock()?;
        let ledger = store.ledger();
        // The entries: after a journal line (by when they were posted,
        // backdated ones included), else by their date.
        let selected: Vec<&ledger_core::JournalEntry> = ledger
            .posted_entries()
            .filter(|e| match p.since_seq {
                Some(n) => {
                    ledger.posted_seq(e.id()).is_some_and(|s| s > n)
                        && as_of.is_none_or(|d| e.date() <= d)
                }
                None => since.is_some_and(|s| e.date() >= s) && as_of.is_none_or(|d| e.date() <= d),
            })
            .collect();
        let mut entries = Vec::new();
        for e in &selected {
            let receipts = store
                .list_attachments(Some(e.id()))
                .map_err(|e| e.to_string())?;
            // The one function that decides (plan P4, I-5): what posted
            // an entry is part of the answer.
            let required =
                ledger_core::evidence::requirement(ledger, e, ledger.settings()).is_required();
            entries.push(DigestEntry {
                entry_id: e.id(),
                date: e.date().to_string(),
                description: e.description().to_string(),
                total: money(ReceiptPolicy::entry_debits(e).map_err(|e| e.to_string())?),
                reversal_of: e.reversal_of(),
                receipt_count: receipts.len() as u64,
                receipt_required: required,
                receipt_missing: required && receipts.is_empty(),
            });
        }
        entries.sort_by(|a, b| b.date.cmp(&a.date).then(b.entry_id.cmp(&a.entry_id)));
        // Movement over the entries asked for (by date: those dated from
        // `since` through the end, as before).
        let in_range: Vec<&ledger_core::JournalEntry> = if p.since_seq.is_some() {
            selected.clone()
        } else {
            selected
                .iter()
                .copied()
                .filter(|e| e.date() <= end)
                .collect()
        };
        let touched: std::collections::BTreeSet<AccountId> = in_range
            .iter()
            .flat_map(|e| e.lines())
            .map(|l| l.account)
            .collect();
        let mut totals = Vec::new();
        let mut zero_net_accounts = Vec::new();
        for a in ledger.chart().all() {
            if a.placeholder {
                continue;
            }
            let m = movement_over(ledger, a.id, &in_range);
            if !m.is_zero() {
                totals.push(DigestTotal {
                    code: a.code.clone(),
                    name: a.name.clone(),
                    movement: money(m),
                });
            } else if touched.contains(&a.id) {
                zero_net_accounts.push(format!("{} {}", a.code, a.name));
            }
        }
        zero_net_accounts.sort();
        totals.sort_by(|a, b| a.code.cmp(&b.code));
        let stale_rules = ledger_core::rules::stale_rules(today)
            .into_iter()
            .map(|r| StaleRule {
                name: r.name.to_string(),
                known_through: r.known_through,
                source: r.authority.to_string(),
                why: r.review_why.to_string(),
            })
            .collect();
        let journal_repairs = store
            .tail_repairs()
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|(at, _, _)| {
                since.is_none_or(|s| at.get(..10).is_some_and(|d| d >= s.to_string().as_str()))
            })
            // The stamp only: the file's name would name the journal.
            .map(|(at, bytes, saved_as)| JournalRepair {
                at,
                bytes,
                set_aside: saved_as
                    .rsplit_once(".torn.")
                    .map(|(_, stamp)| stamp.to_string())
                    .unwrap_or_default(),
            })
            .collect();
        let unreconciled_accounts = self.unreconciled_accounts(&store, today)?;
        let uncategorized_accounts = ledger_core::tax::uncategorized_accounts(ledger)
            .into_iter()
            .map(|a| format!("{} {}", a.code, a.name))
            .collect();
        let control_problems = control_problems(&control_check_out(
            ledger,
            today,
            &ledger.control_check(today).map_err(|e| e.to_string())?,
        ));
        let mut drafts_by_state = std::collections::BTreeMap::new();
        for e in ledger.drafts() {
            *drafts_by_state
                .entry(e.draft_state().key().to_string())
                .or_insert(0u64) += 1;
        }
        let with_statement: std::collections::BTreeSet<AccountId> = store
            .statements(None)
            .map_err(|e| e.to_string())?
            .iter()
            .map(|s| s.account_id)
            .collect();
        let (mut reconciliation_tracked, mut never_reconciled) = (Vec::new(), Vec::new());
        for id in ledger_store::reconcilable::cash_accounts(ledger, &self.settings().roles.cash) {
            let code = code_name(&store, id).0;
            if with_statement.contains(&id) {
                reconciliation_tracked.push(code);
            } else {
                never_reconciled.push(code);
            }
        }
        reconciliation_tracked.sort();
        never_reconciled.sort();
        Ok(Json(DigestOut {
            since_seq: p.since_seq,
            head_seq: store.head().0,
            zero_net_accounts,
            drafts_by_state,
            reconciliation_tracked,
            never_reconciled,
            unreconciled_accounts,
            uncategorized_accounts,
            control_problems,
            since: since.map(|d| d.to_string()).unwrap_or_default(),
            entries,
            totals_by_account: totals,
            stale_rules,
            journal_repairs,
            settings_problems: settings_problems(store.ledger(), &self.settings()),
        }))
    }

    #[tool(
        name = "books.export",
        title = "Export a report",
        description = "Export a report as a file and hand it over (run the download it returns; never its bytes). Reports: profit_and_loss, balance_sheet, cash_flow, trial_balance, members_equity, tax_summary, general_ledger, general_journal, fixed_asset_register, ar_aging, ap_aging. Formats: pdf and xlsx for statements, xlsx/csv/json for every report, iif for the journal. Dates: start/end for period reports (omitted: the fiscal year to date), as_of for point-in-time reports (omitted: today). Each report's contents and the IIF limits are in books.handbook section files-in-out.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn export(&self, params: Parameters<ExportParams>) -> Result<Json<ExportOut>, String> {
        let p = params.0;
        let format = p.format.to_lowercase();
        if !["csv", "json", "xlsx", "pdf", "iif"].contains(&format.as_str()) {
            return Err(format!(
                "bad format {:?}: expected \"csv\", \"json\", \"xlsx\", \"pdf\", or \"iif\"",
                p.format
            ));
        }
        let report = p.report.to_lowercase();
        if format == "iif" {
            if !matches!(report.as_str(), "journal" | "general_journal") {
                return Err(
                    "iif format is only valid for the general_journal report (alias journal)"
                        .to_string(),
                );
            }
            if p.start.is_some() || p.end.is_some() {
                return Err("iif takes as_of, not start or end: the file holds every posted entry from the first through as_of, so QuickBooks' balances match the books'".to_string());
            }
            // As of today by default, like every other as-of export
            // (rules.md B-30); never a period, so the general journal's
            // default period never shortens it (D-36, B-34).
            let as_of = self.as_of(p.as_of.as_deref())?;
            let iif = self.export_iif(as_of, &p.cash_accounts)?;
            let warnings = iif.warnings();
            let file = self.hand_over_export(&report, &format, iif.text.into_bytes())?;
            return Ok(Json(ExportOut {
                format,
                report,
                file,
                warnings,
            }));
        }
        if format == "xlsx" || format == "pdf" {
            let bytes = if format == "xlsx" {
                self.export_xlsx(&report, &p)?
            } else {
                self.export_pdf(&report, &p)?
            };
            let file = self.hand_over_export(&report, &format, bytes)?;
            return Ok(Json(ExportOut {
                format,
                report,
                file,
                warnings: Vec::new(),
            }));
        }
        let content = match report.as_str() {
            "journal" | "general_journal" => self.export_journal(&format, &p)?,
            "trial_balance" => self.export_trial_balance(&format, p.as_of.as_deref())?,
            "general_ledger" => self.export_general_ledger(&format, &p)?,
            "members_equity" => self.export_members_equity(&format, &p)?,
            "tax_summary" => self.export_tax_summary(&format, &p)?,
            "profit_and_loss" => self.export_pnl(&format, p.start.as_deref(), p.end.as_deref())?,
            "balance_sheet" => self.export_balance_sheet(&format, p.as_of.as_deref())?,
            "cash_flow" => self.export_cash_flow(&format, &p)?,
            "fixed_asset_register" => {
                self.export_fixed_asset_register(&format, p.as_of.as_deref())?
            }
            "ar_aging" => self.export_aging(&format, true, p.as_of.as_deref())?,
            "ap_aging" => self.export_aging(&format, false, p.as_of.as_deref())?,
            _ => {
                return Err(format!(
                    "bad report {:?}: expected general_journal, general_ledger, trial_balance, profit_and_loss, balance_sheet, cash_flow, members_equity, tax_summary, fixed_asset_register, ar_aging, or ap_aging",
                    p.report
                ));
            }
        };
        let file = self.hand_over_export(&report, &format, content.into_bytes())?;
        Ok(Json(ExportOut {
            format,
            report,
            file,
            warnings: Vec::new(),
        }))
    }

    #[tool(
        name = "books.profit_and_loss_by_class",
        title = "Profit and loss by class",
        description = described(
            "P&L broken down by class (a project: see books.projects) for a period, plus an unclassified row for untagged lines.",
            ledger_core::calendar::PeriodKind::Statement
        ),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn profit_and_loss_by_class(
        &self,
        params: Parameters<ProfitAndLossParams>,
    ) -> Result<Json<ProfitAndLossByClassOut>, String> {
        let p = params.0;
        let store = self.lock()?;
        let ledger = store.ledger();
        let (start, end) = self.statement_range(p.start.as_deref(), p.end.as_deref())?;
        let as_of_status = self.report_status(&store, end, p.strict.unwrap_or(false))?;
        // One class, if asked (validated like every reference).
        let only = match &p.class {
            Some(c) => Some(resolve_class(&store, c)?),
            None => None,
        };
        // Per-class P&L over the date range: accumulate line by line.
        use std::collections::HashMap;
        #[derive(Default)]
        struct Acc {
            revenue: Decimal,
            expenses: Decimal,
        }
        let mut by_class: HashMap<Option<u32>, Acc> = HashMap::new();
        // A closing entry isn't income or spending (rules.md B-11): left
        // out, as profit_and_loss_for_period leaves it out (found by
        // pnl_by_class_excludes_closing_entries, 2026-10-14).
        let closings = ledger.closing_entry_ids();
        for entry in ledger.posted_entries() {
            if entry.date() < start || entry.date() > end || closings.contains(&entry.id()) {
                continue;
            }
            for line in entry.lines() {
                let account = match ledger.chart().get(line.account) {
                    Some(a) => a,
                    _ => continue,
                };
                let signed = match line.side {
                    Side::Debit => line.amount.amount,
                    Side::Credit => -line.amount.amount,
                };
                // Normal-side positive.
                let normal = match account.account_type.normal_side() {
                    Side::Debit => signed,
                    Side::Credit => -signed,
                };
                let acc = by_class.entry(line.class_id).or_default();
                match account.account_type {
                    AccountType::Revenue => acc.revenue += normal,
                    AccountType::Expense => acc.expenses += normal,
                    _ => {}
                }
            }
        }
        // Sort: named classes by name, unclassified last.
        let mut rows: Vec<(Option<String>, Acc)> = by_class
            .into_iter()
            .filter(|(cid, _)| only.is_none() || *cid == only)
            .map(|(cid, acc)| {
                let name = cid
                    .and_then(|id| ledger.get_class(id))
                    .map(|c| c.name.clone());
                (name, acc)
            })
            // Drop rows with no P&L movement.
            .filter(|(_, acc)| !acc.revenue.is_zero() || !acc.expenses.is_zero())
            .collect();
        rows.sort_by(|a, b| match (&a.0, &b.0) {
            (Some(x), Some(y)) => x.cmp(y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });
        let classes = rows
            .into_iter()
            .map(|(class, acc)| ClassPnlOut {
                class,
                revenue: money(acc.revenue),
                expenses: money(acc.expenses),
                net_income: money(acc.revenue - acc.expenses),
            })
            .collect();
        Ok(Json(ProfitAndLossByClassOut {
            as_of_status,
            start: start.to_string(),
            end: end.to_string(),
            classes,
        }))
    }

    /// ```text
    /// GOAL: everything the CPA needs for the return in one zip, the K-1s'
    ///       and 1099s' TINs included, without a full TIN ever landing in
    ///       the books (a git repository) or the conversation.
    /// SOURCE: docs/mcp-interface.md; B-26; DECISION (owner,
    ///         2026-10-15): "it needs to be in the damn CPA package."
    /// INVARIANTS:
    ///   1. The zip is handed over (tools/files.rs INVARIANT 6: an export
    ///      the caller fetches), never written to a path; with include_tins
    ///      (the default) it and each file in it holding a TIN are recorded
    ///      (tin_files.rs), so none is filed back in. include_tins false:
    ///      last fours only.
    ///   2. With members, k1-members.csv has one row per [[company.members]]
    ///      member: name, party, share, capital account, TIN and type, and
    ///      the K-1 worksheet's capital figures (ledger_core::k1) for the
    ///      fiscal year to date; k1-tax-basis-capital.csv the same capital on
    ///      the tax basis beside them (ledger_core::tax_capital). The 1099
    ///      worksheet adds each payee's party, last four, TIN, and type.
    ///   3. A locked vault or a missing TIN never fails the package: the
    ///      row says why (vault_tin), the README lists it, and so does
    ///      tin_gaps (members, and 1099 payees that are reportable).
    ///   4. Read-only: nothing is journaled.
    /// ```
    #[tool(
        name = "books.export_cpa_package",
        title = "CPA handoff package",
        description = "Build the CPA handoff package as a zip (statements as PDF and XLSX, general ledger and journal, IIF, 1099 worksheet, K-1 worksheets, receipts and documents, README with the journal's verified hash) and hand it over: run the download command it returns. cash_accounts is required; as_of defaults to today. Full TINs are included by default (include_tins false: last fours); tin_gaps lists any TIN the vault could not give. Never file the zip into the books. What the package holds, and the order of work, is in books.handbook section cpa-handoff.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn export_cpa_package(
        &self,
        params: Parameters<ExportCpaPackageParams>,
    ) -> Result<Json<ExportCpaPackageOut>, String> {
        let p = params.0;
        let as_of = self.as_of(p.as_of.as_deref())?;
        let mut store = self.lock()?;
        // A package no counted audit stands behind is not final either
        // (plan P8, D-9): on books that declared `audits`, refused under
        // policy.cpa_package_requires_audit = refuse unless a reason is
        // given, which is journaled as a decision; under warn it goes and
        // says so. Books that haven't declared the feature are as they
        // were: there is no audit to ask for.
        let mut audit_warning: Option<String> = None;
        if let Some(why) = self.cpa_audit_standing(&store)? {
            if store.ledger().settings().cpa_package_requires_audit() {
                let Some(reason) = p
                    .without_audit_reason
                    .as_deref()
                    .map(str::trim)
                    .filter(|r| !r.is_empty())
                else {
                    return Err(serde_json::json!({
                        "code": "audit_required",
                        "message": why,
                        "remedy_tool": "books.audit",
                    })
                    .to_string());
                };
                let by = CALLER
                    .with(|c| c.borrow().as_ref().map(|(_, name)| name.clone()))
                    .unwrap_or_else(|| ledger_store::auth::DEFAULT_NAME.to_string());
                store
                    .record_decision(&ledger_core::conventions::NewDecision {
                        subject: format!("CPA package as of {as_of} without a counted audit"),
                        decision_md: format!(
                            "The CPA package as of {as_of} was exported without an audit at the books' current head attested at the bar. Reason: {reason}"
                        ),
                        decided_by: vec![by],
                        decided_on: today(),
                        evidence_attachment: None,
                        supersedes: None,
                        citation: None,
                        projects: Vec::new(),
                    })
                    .map_err(|e| e.to_string())?;
            } else {
                audit_warning = Some(format!(
                    "No counted audit stands behind this package: {why}."
                ));
            }
        }
        // A package with a draft missing from its figures is not final
        // (plan P4, 4.8): refused unless the human says why it goes anyway,
        // and that is journaled as a decision.
        let open_drafts: Vec<(u64, String)> = store
            .ledger()
            .drafts()
            .into_iter()
            .filter(|e| e.date() <= as_of)
            .map(|e| (e.id(), e.description().to_string()))
            .collect();
        if !open_drafts.is_empty() {
            let named: Vec<String> = open_drafts
                .iter()
                .map(|(id, d)| format!("#{id} {d}"))
                .collect();
            let Some(allow) = p.allow_open_drafts.as_ref() else {
                return Err(serde_json::json!({
                    "code": "open_drafts",
                    "message": format!(
                        "{} draft(s) dated on or before {as_of} are open and missing from the package's figures: {}. Post them (books.post_entry) or discard them (books.discard_draft), or pass allow_open_drafts with the reason the package goes anyway",
                        open_drafts.len(),
                        named.join("; ")
                    ),
                    "remedy_tool": "books.list_drafts",
                })
                .to_string());
            };
            let by = CALLER
                .with(|c| c.borrow().as_ref().map(|(_, name)| name.clone()))
                .unwrap_or_else(|| ledger_store::auth::DEFAULT_NAME.to_string());
            store
                .record_decision(&ledger_core::conventions::NewDecision {
                    subject: format!("CPA package as of {as_of} with open drafts"),
                    decision_md: format!(
                        "The CPA package as of {as_of} was exported with {} draft(s) open: {}. Reason: {}",
                        open_drafts.len(),
                        named.join("; "),
                        allow.reason.trim()
                    ),
                    decided_by: vec![by],
                    decided_on: today(),
                    evidence_attachment: None,
                    supersedes: None,
                    citation: None,
                    projects: Vec::new(),
                })
                .map_err(|e| e.to_string())?;
        }
        let book = book_name(&store);
        // The vault, opened once and never creating a key: locked, each
        // TIN row says so and the package is built anyway (INVARIANT 3).
        let vault = p.include_tins.then(|| self.open_vault(&store, false));
        let ledger = store.ledger();
        let tins = vault
            .as_ref()
            .map(|vault| move |id| vault_tin(ledger, vault, id));
        // Built from the books under the lock, written nowhere
        // (ledger_package INVARIANT 1).
        let package = ledger_package::build_package(
            &*store,
            &package_settings(&self.settings()),
            &ledger_package::PackageRequest {
                as_of,
                prepared_on: today(),
                cash_accounts: &p.cash_accounts,
                financing_accounts: &p.financing_accounts,
                investing_accounts: &p.investing_accounts,
                include_receipts: p.include_receipts,
                tins: tins.as_ref().map(|f| f as ledger_package::TinSource<'_>),
            },
        )?;
        let vault_dir = if p.include_tins {
            self.tin_record_dir(&store)?
        } else {
            self.vault_dir(&store)
        };
        drop(store);

        if p.include_tins {
            // The zip and each file in it that holds a TIN (the K-1 and
            // 1099 worksheets): one given back to be filed is refused
            // (tin_files.rs INVARIANT 1).
            for (name, bytes) in &package.files {
                let filed = name.starts_with("receipts/") || name.starts_with("documents/");
                if !filed && ledger_core::parties::has_full_tin(&String::from_utf8_lossy(bytes)) {
                    tin_files::record(&vault_dir, bytes)?;
                }
            }
            tin_files::record(&vault_dir, &package.zip)?;
        }
        let tin_gaps = package.tin_gaps;
        let mut note = match (p.include_tins, tin_gaps.len()) {
            (false, _) => "The package, with last fours only.".to_string(),
            (true, 0) => "The package, with full TINs, for the CPA.".to_string(),
            (true, n) => format!(
                "The package, with full TINs, for the CPA. {n} TIN(s) are missing (tin_gaps): collect them with books.set_tin or books.set_w9 and export again."
            ),
        };
        let mut warnings = package.warnings;
        warnings.extend(audit_warning);
        if package.iif_changes > 0 {
            note.push_str(&format!(
                " {} account or class name(s), number(s), place(s) or memo(s) are written differently in general-journal.iif to fit QuickBooks Desktop (warnings says how; so does the README): tell the human.",
                package.iif_changes
            ));
        }
        let file = self.hand_over(&book, &package.file_name, "application/zip", package.zip)?;
        Ok(Json(ExportCpaPackageOut {
            file,
            tins_included: p.include_tins,
            tin_gaps,
            warnings,
            note,
        }))
    }
}

// The export builders the export tools above call: the journal, the
// statements laid out for PDF and XLSX, and the IIF.
impl BooksServer {
    /// An export's file, handed over (files.rs INVARIANT 6, D-46): named
    /// for its report and today, typed by its format.
    fn hand_over_export(
        &self,
        report: &str,
        format: &str,
        bytes: Vec<u8>,
    ) -> Result<HandedFile, String> {
        let mime = match format {
            "csv" => "text/csv",
            "json" => "application/json",
            "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "pdf" => "application/pdf",
            // IIF is tab-separated ASCII text.
            _ => "text/plain",
        };
        let book = book_name(&*self.lock()?);
        self.hand_over(
            &book,
            &format!("{report}-{}.{format}", today()),
            mime,
            bytes,
        )
    }

    /// Lay out a financial statement for rendering, from export params.
    fn layout_statement(
        &self,
        ledger: &Ledger,
        report: &str,
        p: &ExportParams,
    ) -> Result<ledger_import::Statement, String> {
        use ledger_import::report;
        let members = self.report_members();
        Ok(match report {
            "profit_and_loss" => report::profit_and_loss(&self.pnl_statement(
                ledger,
                p.start.as_deref(),
                p.end.as_deref(),
                None,
            )?),
            "balance_sheet" => {
                let bs = self.bs_statement(ledger, p.as_of.as_deref())?;
                let earnings = self.year_earnings(ledger, bs.as_of)?;
                report::balance_sheet(&bs, earnings, &members)
            }
            "cash_flow" => report::cash_flow(&self.cash_flow_for(
                ledger,
                p.start.as_deref(),
                p.end.as_deref(),
                &p.cash_accounts,
                &p.financing_accounts,
                &p.investing_accounts,
            )?),
            "trial_balance" => {
                let as_of = self.as_of(p.as_of.as_deref())?;
                report::trial_balance(&reports::trial_balance_as_of(ledger, as_of), as_of)
            }
            "tax_summary" => {
                let (start, end) = self.statement_range(p.start.as_deref(), p.end.as_deref())?;
                report::tax_summary(
                    &ledger_core::tax_summary(ledger, start, end, self.settings().company.tax_form)
                        .map_err(|e| e.to_string())?,
                )
            }
            "members_equity" => {
                let (start, end) = self.statement_range(p.start.as_deref(), p.end.as_deref())?;
                let me =
                    ledger_core::members_equity(ledger, start, end, &self.member_capitals(ledger)?)
                        .map_err(|e| e.to_string())?;
                report::members_equity(&me)
            }
            "ar_aging" => {
                ledger_package::aging_statement(ledger, true, self.as_of(p.as_of.as_deref())?)
                    .map_err(|e| e.to_string())?
            }
            "ap_aging" => {
                ledger_package::aging_statement(ledger, false, self.as_of(p.as_of.as_deref())?)
                    .map_err(|e| e.to_string())?
            }
            _ => {
                return Err(format!(
                    "bad report {report:?}: expected profit_and_loss, balance_sheet, cash_flow, trial_balance, members_equity, tax_summary, ar_aging, or ap_aging"
                ));
            }
        })
    }

    /// The receivables or payables aging as CSV or JSON (plan P4b S6,
    /// 8.3.10): one row per open document, then the tie-out line; the
    /// same CSV the CPA package carries.
    fn export_aging(
        &self,
        format: &str,
        invoices: bool,
        as_of: Option<&str>,
    ) -> Result<String, String> {
        let as_of = self.as_of(as_of)?;
        let store = self.lock()?;
        if format == "json" {
            let docs = store
                .ledger()
                .aging_documents(as_of, invoices)
                .map_err(|e| e.to_string())?;
            let rows: Vec<serde_json::Value> = docs
                .iter()
                .map(|d| {
                    serde_json::json!({
                        "number": match d.doc {
                            ledger_core::DocRef::Invoice(id) => store.ledger().get_invoice(id).and_then(|i| i.number.clone()),
                            ledger_core::DocRef::Bill(id) => store.ledger().get_bill(id).and_then(|b| b.vendor_ref.clone()),
                        },
                        "party": d.party,
                        "date": d.date.to_string(),
                        "due_date": d.due_date.to_string(),
                        "terms": d.terms,
                        "total": money(d.total),
                        "paid": money(d.paid),
                        "discounted": money(d.discounted),
                        "outstanding": money(d.outstanding),
                        "bucket": d.bucket,
                        "days_overdue": d.days_overdue,
                    })
                })
                .collect();
            return serde_json::to_string_pretty(&rows).map_err(|e| e.to_string());
        }
        ledger_package::aging_csv(store.ledger(), invoices, as_of)
    }

    /// Render a report to XLSX bytes via ledger-import.
    fn export_xlsx(&self, report: &str, p: &ExportParams) -> Result<Vec<u8>, String> {
        use ledger_import::{XlsxReport, report_to_xlsx};
        let store = self.lock()?;
        let meta = self.report_meta(&store);
        let ledger = store.ledger();
        if report == "fixed_asset_register" {
            let as_of = self.as_of(p.as_of.as_deref())?;
            let r = ledger_core::fixed_asset_register(ledger, as_of).map_err(|e| e.to_string())?;
            return ledger_import::asset_register_xlsx(&r, &meta).map_err(|e| e.to_string());
        }
        let xlsx = |r: XlsxReport<'_>| report_to_xlsx(&r, &meta).map_err(|e| e.to_string());
        match report {
            "general_ledger" => {
                let (start, end) = self.statement_range(p.start.as_deref(), p.end.as_deref())?;
                let gl =
                    ledger_core::general_ledger(ledger, start, end).map_err(|e| e.to_string())?;
                xlsx(XlsxReport::GeneralLedger(&gl))
            }
            "general_journal" | "journal" => {
                let (start, end) = self.statement_range(p.start.as_deref(), p.end.as_deref())?;
                xlsx(XlsxReport::GeneralJournal(ledger, start, end))
            }
            _ => xlsx(XlsxReport::Statement(
                &self.layout_statement(ledger, report, p)?,
            )),
        }
    }

    /// Render a financial statement to PDF bytes via ledger-import.
    fn export_pdf(&self, report: &str, p: &ExportParams) -> Result<Vec<u8>, String> {
        if matches!(report, "general_ledger" | "general_journal" | "journal") {
            return Err(format!("{report} exports as xlsx, csv, or json, not pdf"));
        }
        let store = self.lock()?;
        let meta = self.report_meta(&store);
        if report == "fixed_asset_register" {
            let as_of = self.as_of(p.as_of.as_deref())?;
            let r = ledger_core::fixed_asset_register(store.ledger(), as_of)
                .map_err(|e| e.to_string())?;
            return ledger_import::asset_register_pdf(&r, &meta).map_err(|e| e.to_string());
        }
        let st = self.layout_statement(store.ledger(), report, p)?;
        ledger_import::statement_pdf(&st, &meta).map_err(|e| e.to_string())
    }

    /// The journal through `as_of` as IIF via ledger-package (the CPA
    /// package's own general-journal.iif): the cash accounts (`cash`, else
    /// the settings') BANK, the receivable and payable control accounts AR
    /// and AP, the accounts of an asset bought by `as_of` FIXASSET, the
    /// rest by their wanderbooks type; every posted entry from the first,
    /// closes left out.
    fn export_iif(
        &self,
        as_of: chrono::NaiveDate,
        cash: &[String],
    ) -> Result<ledger_import::Iif, String> {
        let store = self.lock()?;
        let settings = self.settings();
        let cash = if cash.is_empty() {
            settings.roles.cash.as_slice()
        } else {
            cash
        };
        ledger_package::export_iif(store.ledger(), cash, as_of)
    }
    /// The general journal as JSON or CSV: each entry posted in the
    /// period (rules.md B-30, as the XLSX journal), in posting order.
    fn export_journal(&self, format: &str, p: &ExportParams) -> Result<String, String> {
        let (start, end) = self.statement_range(p.start.as_deref(), p.end.as_deref())?;
        let store = self.lock()?;
        let ledger = store.ledger();
        let in_period = |e: &&ledger_core::JournalEntry| (start..=end).contains(&e.date());
        if format == "json" {
            let rows: Vec<serde_json::Value> = ledger
                .posted_entries()
                .filter(in_period)
                .map(|e| {
                    serde_json::json!({
                        "entry_id": e.id(),
                        "date": e.date().to_string(),
                        "description": e.description(),
                        "reversal_of": e.reversal_of(),
                        "lines": e.lines().iter().map(|l| {
                            let a = ledger.chart().get(l.account).unwrap();
                            serde_json::json!({
                                "account_code": a.code,
                                "account_name": a.name,
                                "side": match l.side { Side::Debit => "debit", Side::Credit => "credit" },
                                "amount": money(l.amount.amount),
                                "currency": l.amount.currency.code(),
                            })
                        }).collect::<Vec<_>>(),
                    })
                })
                .collect();
            return serde_json::to_string_pretty(&rows).map_err(|e| e.to_string());
        }
        let mut csv = String::from(
            "entry_id,date,description,reversal_of,account_code,account_name,side,amount,currency\n",
        );
        for e in ledger.posted_entries().filter(in_period) {
            for l in e.lines() {
                let a = ledger.chart().get(l.account).unwrap();
                csv.push_str(&format!(
                    "{},{},{},{},{},{},{},{},{}\n",
                    e.id(),
                    e.date(),
                    csv_escape(e.description()),
                    e.reversal_of().map(|v| v.to_string()).unwrap_or_default(),
                    a.code,
                    csv_escape(&a.name),
                    match l.side {
                        Side::Debit => "debit",
                        Side::Credit => "credit",
                    },
                    money(l.amount.amount),
                    l.amount.currency.code(),
                ));
            }
        }
        Ok(csv)
    }

    fn export_trial_balance(&self, format: &str, as_of: Option<&str>) -> Result<String, String> {
        let store = self.lock()?;
        let as_of = self.as_of(as_of)?;
        let tb = reports::trial_balance_as_of(store.ledger(), as_of);
        if format == "json" {
            return serde_json::to_string_pretty(&serde_json::json!({
                "as_of": as_of.to_string(),
                "lines": tb.lines.iter().map(|l| serde_json::json!({
                    "code": l.code, "name": l.name,
                    "debit": money(l.debit), "credit": money(l.credit),
                })).collect::<Vec<_>>(),
                "total_debits": money(tb.total_debits),
                "total_credits": money(tb.total_credits),
                "balanced": tb.balanced(),
            }))
            .map_err(|e| e.to_string());
        }
        let mut csv = String::from("code,name,debit,credit\n");
        for l in &tb.lines {
            csv.push_str(&format!(
                "{},{},{},{}\n",
                l.code,
                csv_escape(&l.name),
                money(l.debit),
                money(l.credit)
            ));
        }
        csv.push_str(&format!(
            "TOTAL,,{},{}",
            money(tb.total_debits),
            money(tb.total_credits)
        ));
        Ok(csv)
    }

    /// The fixed asset register (ledger_core::fixed_asset_register) as
    /// JSON or CSV, reconciled to the books.
    fn export_fixed_asset_register(
        &self,
        format: &str,
        as_of: Option<&str>,
    ) -> Result<String, String> {
        let store = self.lock()?;
        let as_of = self.as_of(as_of)?;
        let r =
            ledger_core::fixed_asset_register(store.ledger(), as_of).map_err(|e| e.to_string())?;
        // The same words as the XLSX and PDF register.
        let status = |row: &ledger_core::AssetRegisterRow| match row.disposed {
            Some((d, p)) => format!("Disposed {d}, proceeds {}", money(p)),
            None => "Active".to_string(),
        };
        let source = |row: &ledger_core::AssetRegisterRow| {
            row.basis_source.as_ref().map(|s| {
                let mut t = format!("{}, {}", s.decided_by, s.decided_on);
                if let Some(c) = &s.citation {
                    t.push_str(&format!(": {c}"));
                }
                t
            })
        };
        if format == "json" {
            return serde_json::to_string_pretty(&serde_json::json!({
                "as_of": as_of.to_string(),
                "tax_figures_are": "at contribution (or purchase): the CPA adjusts them for tax depreciation, and the §704(c) gain or loss as it is reduced after (Treas. Reg. §1.704-3(a)(3)(ii))",
                "assets": r.rows.iter().map(|a| serde_json::json!({
                    "asset_id": a.asset_id,
                    "tag": a.tag,
                    "description": a.description,
                    "serial": a.serial,
                    "acquired_date": a.acquired_date.to_string(),
                    "in_service_date": a.in_service_date.to_string(),
                    "cost": money(a.cost),
                    "salvage": money(a.salvage),
                    "useful_life_months": a.useful_life_months,
                    "accumulated_scheduled": money(a.accumulated_scheduled),
                    "accumulated_posted": money(a.accumulated_posted),
                    "book_value": money(a.book_value),
                    "status": status(a),
                    "proceeds": a.disposed.map(|(_, p)| money(p)),
                    "basis_rule": a.basis_rule.map(|r| r.key()),
                    "contributor_adjusted_basis": a.contributor_adjusted_basis.map(money),
                    "fmv_at_contribution": a.fmv_at_contribution.map(money),
                    "fmv_recorded": a.fmv_recorded,
                    "tax_basis_723": a.tax_basis_723.map(money),
                    "depreciable_basis": a.depreciable_basis.map(money),
                    "built_in_gain_704c": a.built_in_gain_704c.map(money),
                    "sec704c_1c": a.sec704c_1c,
                    "sec704c_method": a.sec704c_method.map(|m| m.key()),
                    "basis_source": source(a),
                })).collect::<Vec<_>>(),
                "total_cost": money(r.total_cost),
                "total_accumulated_scheduled": money(r.total_accumulated_scheduled),
                "total_accumulated_posted": money(r.total_accumulated_posted),
                "total_book_value": money(r.total_book_value),
                "total_depreciable_basis": money(r.total_depreciable_basis),
                "total_tax_basis_723": money(r.total_tax_basis_723),
                "missing_basis": r.missing_basis,
                "ledger_cost": money(r.ledger_cost),
                "ledger_accumulated": money(r.ledger_accumulated),
                "unposted_depreciation": money(r.unposted_depreciation()),
                "unattributed_accumulated": money(r.unattributed_accumulated()),
                "unregistered_cost": money(r.unregistered_cost()),
            }))
            .map_err(|e| e.to_string());
        }
        let mut csv = String::from(
            "tag,description,serial,acquired,in_service,cost,salvage,useful_life_months,accumulated_scheduled,accumulated_posted,book_value,status,proceeds,basis_rule,contributor_adjusted_basis,fmv_at_contribution,fmv_recorded,tax_basis_723,depreciable_basis,built_in_gain_704c,sec704c_1c,sec704c_method,basis_source\n",
        );
        for a in &r.rows {
            let cells = [
                csv_escape(a.tag.as_deref().unwrap_or("")),
                csv_escape(&a.description),
                csv_escape(a.serial.as_deref().unwrap_or("")),
                a.acquired_date.to_string(),
                a.in_service_date.to_string(),
                money(a.cost),
                money(a.salvage),
                a.useful_life_months.to_string(),
                money(a.accumulated_scheduled),
                money(a.accumulated_posted),
                money(a.book_value),
                status(a),
                a.disposed.map(|(_, p)| money(p)).unwrap_or_default(),
                a.basis_rule
                    .map(|r| r.key())
                    .unwrap_or_default()
                    .to_string(),
                a.contributor_adjusted_basis.map(money).unwrap_or_default(),
                a.fmv_at_contribution.map(money).unwrap_or_default(),
                if a.basis_rule.is_some_and(|r| r.is_contribution()) {
                    a.fmv_recorded.to_string()
                } else {
                    String::new()
                },
                a.tax_basis_723.map(money).unwrap_or_default(),
                a.depreciable_basis.map(money).unwrap_or_default(),
                a.built_in_gain_704c.map(money).unwrap_or_default(),
                // IRC §704(c)(1)(C): a built-in loss counts only for the
                // contributor; blank for a purchase or no basis.
                a.built_in_gain_704c
                    .map(|g| (g < Decimal::ZERO).to_string())
                    .unwrap_or_default(),
                a.sec704c_method
                    .map(|m| m.key())
                    .unwrap_or_default()
                    .to_string(),
                csv_escape(&source(a).unwrap_or_default()),
            ];
            csv.push_str(&cells.join(","));
            csv.push('\n');
        }
        csv.push_str(&format!(
            "TOTAL,,,,,{},,,{},{},{},,,,,,,{},{},,,,",
            money(r.total_cost),
            money(r.total_accumulated_scheduled),
            money(r.total_accumulated_posted),
            money(r.total_book_value),
            money(r.total_tax_basis_723),
            money(r.total_depreciable_basis)
        ));
        Ok(csv)
    }

    fn export_general_ledger(&self, format: &str, p: &ExportParams) -> Result<String, String> {
        let store = self.lock()?;
        let ledger = store.ledger();
        let (start, end) = self.statement_range(p.start.as_deref(), p.end.as_deref())?;
        let gl = ledger_core::general_ledger(ledger, start, end).map_err(|e| e.to_string())?;
        if format == "json" {
            return serde_json::to_string_pretty(&serde_json::json!({
                "start": gl.start.to_string(), "end": gl.end.to_string(),
                "accounts": gl.accounts.iter().map(|a| serde_json::json!({
                    "code": a.code, "name": a.name,
                    "beginning_balance": money(a.beginning_balance),
                    "postings": a.postings.iter().map(|p| serde_json::json!({
                        "date": p.date.to_string(), "entry_id": p.entry_id,
                        "description": p.description, "split": p.split,
                        "debit": money(p.debit), "credit": money(p.credit),
                        "balance": money(p.balance),
                    })).collect::<Vec<_>>(),
                    "total_debits": money(a.total_debits),
                    "total_credits": money(a.total_credits),
                    "ending_balance": money(a.ending_balance),
                })).collect::<Vec<_>>(),
            }))
            .map_err(|e| e.to_string());
        }
        let mut csv = String::from("account,date,entry,description,split,debit,credit,balance\n");
        for a in &gl.accounts {
            let account = csv_escape(&format!("{} {}", a.code, a.name));
            csv.push_str(&format!(
                "{account},,,Beginning balance,,,,{}\n",
                money(a.beginning_balance)
            ));
            for p in &a.postings {
                csv.push_str(&format!(
                    "{account},{},{},{},{},{},{},{}\n",
                    p.date,
                    p.entry_id,
                    csv_escape(&p.description),
                    csv_escape(&p.split),
                    money(p.debit),
                    money(p.credit),
                    money(p.balance)
                ));
            }
            csv.push_str(&format!(
                "{account},,,Total,,{},{},{}\n",
                money(a.total_debits),
                money(a.total_credits),
                money(a.ending_balance)
            ));
        }
        Ok(csv)
    }

    fn export_tax_summary(&self, format: &str, p: &ExportParams) -> Result<String, String> {
        let store = self.lock()?;
        let ledger = store.ledger();
        let (start, end) = self.statement_range(p.start.as_deref(), p.end.as_deref())?;
        let ts = ledger_core::tax_summary(ledger, start, end, self.settings().company.tax_form)
            .map_err(|e| e.to_string())?;
        let line = |l: &ledger_core::TaxSummaryLine| {
            serde_json::json!({
                "category": l.category, "label": l.label, "accounts": l.accounts,
                "book": money(l.book), "tax": money(l.tax),
            })
        };
        if format == "json" {
            return serde_json::to_string_pretty(&serde_json::json!({
                "start": ts.start.to_string(), "end": ts.end.to_string(),
                "form": ts.form.map(|f| f.label()),
                "income": ts.income.iter().map(line).collect::<Vec<_>>(),
                "cost_of_goods": ts.cost_of_goods.iter().map(line).collect::<Vec<_>>(),
                "deductions": ts.deductions.iter().map(line).collect::<Vec<_>>(),
                "separately_stated": ts.separately_stated.iter().map(line).collect::<Vec<_>>(),
                // Book amounts only: Form 4797 figures the tax gain.
                "form_4797": ts.form_4797.iter().map(|l| serde_json::json!({
                    "category": l.category, "label": l.label, "accounts": l.accounts,
                    "book": money(l.book),
                })).collect::<Vec<_>>(),
                "uncategorized": ts.uncategorized.iter().map(|u| serde_json::json!({
                    "account": u.account, "book": money(u.book),
                })).collect::<Vec<_>>(),
                "book_net": money(ts.book_net()),
                "tax_net": money(ts.tax_net()),
            }))
            .map_err(|e| e.to_string());
        }
        let mut csv = String::from("section,category,label,accounts,book,tax\n");
        for (section, lines) in [
            ("income", &ts.income),
            ("cost_of_goods", &ts.cost_of_goods),
            ("deductions", &ts.deductions),
            ("separately_stated", &ts.separately_stated),
        ] {
            for l in lines {
                csv.push_str(&format!(
                    "{section},{},{},{},{},{}\n",
                    l.category,
                    csv_escape(&l.label),
                    csv_escape(&l.accounts.join("; ")),
                    money(l.book),
                    money(l.tax)
                ));
            }
        }
        for l in &ts.form_4797 {
            csv.push_str(&format!(
                "form_4797,{},{},{},{},\n",
                l.category,
                csv_escape(&l.label),
                csv_escape(&l.accounts.join("; ")),
                money(l.book)
            ));
        }
        for u in &ts.uncategorized {
            csv.push_str(&format!(
                "uncategorized,,,{},{},\n",
                csv_escape(&u.account),
                money(u.book)
            ));
        }
        Ok(csv)
    }

    fn export_members_equity(&self, format: &str, p: &ExportParams) -> Result<String, String> {
        let store = self.lock()?;
        let ledger = store.ledger();
        let (start, end) = self.statement_range(p.start.as_deref(), p.end.as_deref())?;
        let me = ledger_core::members_equity(ledger, start, end, &self.member_capitals(ledger)?)
            .map_err(|e| e.to_string())?;
        let col = |c: &ledger_core::MemberEquityColumn| {
            serde_json::json!({
                "name": c.name, "beginning": money(c.beginning),
                "contributions": money(c.contributions), "net_income": money(c.net_income),
                "distributions": money(c.distributions), "other": money(c.other),
                "ending": money(c.ending),
            })
        };
        if format == "json" {
            return serde_json::to_string_pretty(&serde_json::json!({
                "start": me.start.to_string(), "end": me.end.to_string(),
                "members": me.members.iter().map(col).collect::<Vec<_>>(),
                "total": col(&me.total),
            }))
            .map_err(|e| e.to_string());
        }
        let mut csv =
            String::from("member,beginning,contributions,net_income,distributions,other,ending\n");
        for c in me.members.iter().chain(std::iter::once(&me.total)) {
            csv.push_str(&format!(
                "{},{},{},{},{},{},{}\n",
                csv_escape(&c.name),
                money(c.beginning),
                money(c.contributions),
                money(c.net_income),
                money(c.distributions),
                money(c.other),
                money(c.ending)
            ));
        }
        Ok(csv)
    }

    fn export_pnl(
        &self,
        format: &str,
        start: Option<&str>,
        end: Option<&str>,
    ) -> Result<String, String> {
        let store = self.lock()?;
        let pl = self.pnl_statement(store.ledger(), start, end, None)?;
        if format == "json" {
            return serde_json::to_string_pretty(&serde_json::json!({
                "start": pl.start.to_string(), "end": pl.end.to_string(),
                "revenue_accounts": lines_out(&pl.revenue),
                "expense_accounts": lines_out(&pl.expenses),
                "revenue": money(pl.total_revenue), "expenses": money(pl.total_expenses),
                "net_income": money(pl.net_income()),
            }))
            .map_err(|e| e.to_string());
        }
        let mut csv = String::from("section,code,name,amount\n");
        for (section, lines) in [("revenue", &pl.revenue), ("expenses", &pl.expenses)] {
            for l in lines {
                csv.push_str(&format!(
                    "{section},{},{},{}\n",
                    csv_escape(&l.code),
                    csv_escape(&l.name),
                    money(l.amount)
                ));
            }
        }
        csv.push_str(&format!(
            "total,,revenue,{}\ntotal,,expenses,{}\ntotal,,net_income,{}\n",
            money(pl.total_revenue),
            money(pl.total_expenses),
            money(pl.net_income())
        ));
        Ok(csv)
    }

    fn export_balance_sheet(&self, format: &str, as_of: Option<&str>) -> Result<String, String> {
        let store = self.lock()?;
        let bs = self.bs_statement(store.ledger(), as_of)?;
        if format == "json" {
            return serde_json::to_string_pretty(&serde_json::json!({
                "as_of": bs.as_of.to_string(),
                "asset_accounts": lines_out(&bs.assets),
                "liability_accounts": lines_out(&bs.liabilities),
                "equity_accounts": lines_out(&bs.equity),
                "assets": money(bs.total_assets), "liabilities": money(bs.total_liabilities),
                "equity": money(bs.total_equity), "current_earnings": money(bs.current_earnings),
                "balanced": bs.balanced(),
            }))
            .map_err(|e| e.to_string());
        }
        let mut csv = String::from("section,code,name,amount\n");
        for (section, lines) in [
            ("assets", &bs.assets),
            ("liabilities", &bs.liabilities),
            ("equity", &bs.equity),
        ] {
            for l in lines {
                csv.push_str(&format!(
                    "{section},{},{},{}\n",
                    csv_escape(&l.code),
                    csv_escape(&l.name),
                    money(l.amount)
                ));
            }
        }
        csv.push_str(&format!(
            "equity,,current_earnings,{}\ntotal,,assets,{}\ntotal,,liabilities,{}\ntotal,,equity,{}\n",
            money(bs.current_earnings),
            money(bs.total_assets),
            money(bs.total_liabilities),
            money(bs.total_equity)
        ));
        Ok(csv)
    }

    fn export_cash_flow(&self, format: &str, p: &ExportParams) -> Result<String, String> {
        let store = self.lock()?;
        let ledger = store.ledger();
        let stmt = self.cash_flow_for(
            ledger,
            p.start.as_deref(),
            p.end.as_deref(),
            &p.cash_accounts,
            &p.financing_accounts,
            &p.investing_accounts,
        )?;
        if format == "json" {
            // The tool's own answer, from the one function (they can't
            // drift apart).
            return serde_json::to_string_pretty(&cash_flow_out(&stmt)).map_err(|e| e.to_string());
        }
        Ok(cash_flow_csv(&stmt))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::test_support::*;
    use std::str::FromStr;

    /// A period is "YYYY-MM": a year of the calendar the books keep
    /// (1-9999), month 1-12, nothing more; its last day is a date, never a
    /// panic (P0 review round 3, S1: "999999-01" and "262142-12" once
    /// stopped the server).
    #[test]
    fn periods_are_a_year_and_a_month() {
        assert_eq!(parse_period_mcp("2026-01"), Ok((2026, 1)));
        assert_eq!(parse_period_mcp("2026-12"), Ok((2026, 12)));
        assert_eq!(parse_period_mcp("0001-01"), Ok((1, 1)));
        assert_eq!(parse_period_mcp("9999-12"), Ok((9999, 12)));
        for bad in [
            "2026-13",
            "2026-00",
            "2026-01-05",
            "0-05",
            "2026",
            "x-01",
            "2026-",
            "",
            "10000-01",
            "999999-01",
            "262142-12",
            "262143-01",
            "-5-01",
        ] {
            assert!(parse_period_mcp(bad).is_err(), "{bad:?}");
        }
        assert_eq!(last_day_of_month(2028, 2), Ok(d(2028, 2, 29)));
        assert_eq!(last_day_of_month(9999, 12), Ok(d(9999, 12, 31)));
        assert_eq!(last_day_of_month(1, 1), Ok(d(1, 1, 31)));
        for (y, m) in [(262142, 12), (999999, 1), (2026, 13), (2026, 0)] {
            assert!(last_day_of_month(y, m).is_err(), "{y}-{m}");
        }
    }

    /// Reports name each member with their accounts, in order.
    #[test]
    fn report_members_carry_names_and_accounts() {
        let books = unit_dir("members");
        let server = served(settled(store(&books), &[members()]));
        let m = server.report_members();
        assert_eq!(m.len(), 2);
        assert_eq!(
            (m[0].name.as_str(), m[0].capital_code.as_str()),
            ("Ada", "3000")
        );
        assert_eq!(m[0].distribution_code.as_deref(), Some("3100"));
        assert_eq!(
            (m[1].name.as_str(), m[1].capital_code.as_str()),
            ("Bo", "3001")
        );
        assert_eq!(m[1].distribution_code, None);
    }

    /// The balance sheet's year earnings: net income for the fiscal year
    /// to date, and the part not yet closed into equity (closing entries
    /// included, so a closed month drops out).
    #[test]
    fn year_earnings_split_closed_from_unclosed() {
        let books = unit_dir("earnings");
        let mut s = store(&books);
        let cash = s
            .add_account("1000", "Cash", AccountType::Asset, None, false)
            .unwrap();
        let sales = s
            .add_account("4000", "Sales", AccountType::Revenue, None, false)
            .unwrap();
        let rent = s
            .add_account("6000", "Rent", AccountType::Expense, None, false)
            .unwrap();
        post(&mut s, d(2025, 12, 15), cash, sales, "999.00"); // last fiscal year
        post(&mut s, d(2026, 1, 10), cash, sales, "1000.00");
        post(&mut s, d(2026, 1, 20), rent, cash, "300.00");
        post(&mut s, d(2026, 2, 5), cash, sales, "50.00");
        post(&mut s, d(2026, 2, 6), rent, cash, "20.00");
        let server = served(s);
        let as_of = d(2026, 2, 28);
        {
            let store = server.lock().unwrap();
            let e = server.year_earnings(store.ledger(), as_of).unwrap();
            assert_eq!(e.net_income_ytd, Decimal::from_str("730.00").unwrap());
            assert_eq!(e.unclosed, Decimal::from_str("730.00").unwrap());
        }
        {
            let mut store = server.lock().unwrap();
            let jan = store
                .create_period("2026-01", d(2026, 1, 1), d(2026, 1, 31))
                .unwrap();
            store.close_period(jan).unwrap();
        }
        let store = server.lock().unwrap();
        let e = server.year_earnings(store.ledger(), as_of).unwrap();
        assert_eq!(
            e.net_income_ytd,
            Decimal::from_str("730.00").unwrap(),
            "still the year's income"
        );
        assert_eq!(
            e.unclosed,
            Decimal::from_str("30.00").unwrap(),
            "January's 700 is closed"
        );
    }

    /// Only a closed period moves where the unclosed part starts: an open
    /// one dated before as_of doesn't, and a close that ended before the
    /// fiscal year began leaves the year starting on its first day (last
    /// December's income is never this year's).
    #[test]
    fn year_earnings_count_only_closes_and_never_before_the_year() {
        let books = unit_dir("earnings-open");
        let mut s = store(&books);
        let cash = s
            .add_account("1000", "Cash", AccountType::Asset, None, false)
            .unwrap();
        let sales = s
            .add_account("4000", "Sales", AccountType::Revenue, None, false)
            .unwrap();
        post(&mut s, d(2025, 12, 15), cash, sales, "999.00");
        post(&mut s, d(2026, 1, 10), cash, sales, "1000.00");
        s.create_period("2026-01", d(2026, 1, 1), d(2026, 1, 31))
            .unwrap();
        let server = served(s);
        let as_of = d(2026, 2, 28);
        let thousand = Decimal::from_str("1000.00").unwrap();
        {
            let store = server.lock().unwrap();
            let e = server.year_earnings(store.ledger(), as_of).unwrap();
            assert_eq!(e.unclosed, thousand, "an open January closes nothing");
        }
        {
            let mut store = server.lock().unwrap();
            let to_nov = store
                .create_period("2025-01..11", d(2025, 1, 1), d(2025, 11, 30))
                .unwrap();
            store.close_period(to_nov).unwrap();
        }
        let store = server.lock().unwrap();
        let e = server.year_earnings(store.ledger(), as_of).unwrap();
        assert_eq!(e.net_income_ytd, thousand);
        assert_eq!(e.unclosed, thousand, "December 2025 is not this year's");
    }

    /// books.show (defect 15): how the viewer reads each report's dates,
    /// and the options a report ignores, refused (one match, so each arm
    /// is pinned).
    #[test]
    fn show_reads_each_reports_dates_as_the_viewer_does() {
        for (report, columns, compare, want) in [
            ("profit_and_loss", None, None, Ok(ShownDates::Period)),
            ("cash_flow", Some("month"), None, Ok(ShownDates::Period)),
            ("members_equity", None, None, Ok(ShownDates::Period)),
            ("general_ledger", None, None, Ok(ShownDates::Period)),
            (
                "tax_summary",
                None,
                Some("prior_year"),
                Ok(ShownDates::Period),
            ),
            ("trial_balance", None, None, Ok(ShownDates::AsOf)),
            ("ar_aging", None, None, Ok(ShownDates::AsOf)),
            ("ap_aging", None, None, Ok(ShownDates::AsOf)),
            ("fixed_asset_register", None, None, Ok(ShownDates::AsOf)),
            ("vendor_1099", None, None, Ok(ShownDates::Year)),
            ("balance_sheet", None, None, Ok(ShownDates::AsOf)),
            ("balance_sheet", Some("total"), None, Ok(ShownDates::AsOf)),
            (
                "balance_sheet",
                None,
                Some("prior_year"),
                Ok(ShownDates::AsOf),
            ),
            (
                "balance_sheet",
                None,
                Some("prior_period"),
                Ok(ShownDates::Period),
            ),
            (
                "balance_sheet",
                Some("quarter"),
                None,
                Ok(ShownDates::Period),
            ),
            ("trial_balance", Some("total"), None, Err("columns")),
            ("tax_summary", Some("month"), None, Err("columns")),
            ("vendor_1099", None, Some("prior_year"), Err("compare")),
            ("general_ledger", None, Some("prior_period"), Err("compare")),
            (
                "profit_and_loss",
                Some("year"),
                Some("prior_year"),
                Err("split"),
            ),
        ] {
            let got = show_report_dates(report, columns, compare);
            match want {
                Ok(dates) => assert_eq!(got, Ok(dates), "{report} {columns:?} {compare:?}"),
                Err(word) => {
                    let e = got.unwrap_err();
                    assert!(e.contains(word), "{report} {columns:?} {compare:?}: {e}");
                }
            }
        }
    }

    /// A bare books://digest, or one with an empty since=, covers this
    /// month so far: since is the 1st of today's month, on any day of it
    /// (not today, which only agrees on the 1st). A given since is kept.
    #[test]
    fn a_bare_digest_covers_this_month_so_far() {
        for (day, want) in [(16, "2026-10-01"), (1, "2026-10-01"), (31, "2026-10-01")] {
            let t = d(2026, 10, day);
            for q in ["", "since=", "since=%20", "x=1", "x=1&since="] {
                let q = q.replace("%20", " ");
                assert_eq!(digest_since(&q, t), want, "{q:?} on {t}");
            }
        }
        assert_eq!(
            digest_since("since=2026-01-05", d(2026, 10, 16)),
            "2026-01-05"
        );
        assert_eq!(
            digest_since("x=1&since=2026-01-05", d(2026, 10, 16)),
            "2026-01-05"
        );
        assert_eq!(digest_since("", d(2027, 1, 9)), "2027-01-01");
    }
}
