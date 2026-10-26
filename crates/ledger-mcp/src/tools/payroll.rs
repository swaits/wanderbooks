//! Payroll through the server: accounting only, the caller supplies every
//! number (no tax tables). Split whole out of `server.rs`
//! (plan/served.md 8.2 U0a).

use super::*;

#[tool_router(router = payroll_router, vis = "pub(super)")]
impl BooksServer {
    #[tool(
        name = "books.run_payroll",
        title = "Run payroll",
        description = "Run payroll for a pay date. Each line carries one employee's gross pay and every withholding/deduction as decimal strings — the engine cuts one balanced entry (Dr Salary & Wage Expense, Dr Payroll Tax Expense for the employer share, Cr the withholding payables, Cr cash for net pay). Auto-creates the 61xx/21xx accounts. All amounts are decimal strings, never floats. Name each line's employee party (employee: id or name, a party with the employee role; add one with books.add_party and record its W-4 with books.set_w4 first): new runs should, so the run shows on the employee's books.party.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn run_payroll(
        &self,
        params: Parameters<RunPayrollParams>,
    ) -> Result<Json<RunPayrollOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let cash = resolve_account(&store, &p.cash_account)?;
        let lines: Vec<ledger_store::PayrollLineInput> = p
            .lines
            .into_iter()
            .map(|l| {
                let employee = l
                    .employee
                    .as_deref()
                    .map(|r| resolve_party(&store, r))
                    .transpose()?;
                let employee_name = match (l.employee_name, employee) {
                    (Some(n), _) => n,
                    (None, Some(id)) => store
                        .ledger()
                        .get_party(id)
                        .map(|p| p.name.clone())
                        .unwrap_or_default(),
                    (None, None) => {
                        return Err(
                            "each payroll line names its employee: employee (a party) or employee_name"
                                .to_string(),
                        );
                    }
                };
                Ok(ledger_core::PayrollLine {
                    employee_name,
                    employee,
                    gross_pay: parse_amount(&l.gross_pay)?,
                    federal_withholding: parse_amount(&l.federal_withholding)?,
                    state_withholding: parse_amount(&l.state_withholding)?,
                    ss_employee: parse_amount(&l.ss_employee)?,
                    medicare_employee: parse_amount(&l.medicare_employee)?,
                    ss_employer: parse_amount(&l.ss_employer)?,
                    medicare_employer: parse_amount(&l.medicare_employer)?,
                    other_deductions: parse_amount(&l.other_deductions)?,
                })
            })
            .collect::<Result<_, String>>()?;
        if self.settings().parties == ledger_config::PartyPolicy::Require
            && lines
                .iter()
                .any(|l| l.employee.is_none() && l.net_pay().is_ok_and(|n| !n.is_zero()))
        {
            return Err(party_required(
                "name each payroll line's employee party (employee), so the entry names who was paid",
            ));
        }
        let first_new = store.ledger().next_account_id();
        let run_id = store
            .run_payroll(parse_date(&p.date)?, lines, cash)
            .map_err(|e| e.to_string())?;
        let run = store
            .ledger()
            .get_payroll_run(run_id)
            .ok_or_else(|| format!("payroll run {run_id} not found after creation"))?;
        let total_gross = checked_sum(run.lines.iter().map(|l| Ok(l.gross_pay)), "gross pay")?;
        let total_net = checked_sum(run.lines.iter().map(|l| l.net_pay()), "net pay")?;
        Ok(Json(RunPayrollOut {
            run_id,
            entry_id: run.entry_id,
            total_gross: money(total_gross),
            total_net: money(total_net),
            warnings: created_uncategorized(store.ledger(), first_new),
        }))
    }

    #[tool(
        name = "books.remit_payroll_taxes",
        title = "Remit payroll taxes",
        description = "Remit all outstanding payroll tax payables (federal/state withholding, SS, Medicare, other deductions) to cash in one balanced entry. Name the tax agency paid (agency: a party id or name, e.g. the IRS): it is the entry's party, and [books] parties = \"require\" refuses without it.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn remit_payroll_taxes(
        &self,
        params: Parameters<RemitPayrollTaxesParams>,
    ) -> Result<Json<RemitPayrollTaxesOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let cash = resolve_account(&store, &p.cash_account)?;
        let agency = p
            .agency
            .as_deref()
            .map(|r| resolve_party(&store, r))
            .transpose()?;
        if agency.is_none() && self.settings().parties == ledger_config::PartyPolicy::Require {
            return Err(party_required("pass agency: the tax agency paid"));
        }
        // Total before remitting, for the response.
        let total: Decimal = store
            .ledger()
            .payroll_tax_payables()
            .iter()
            .map(|(_, b)| *b)
            .sum();
        let entry_id = store
            .remit_payroll_taxes(parse_date(&p.date)?, cash)
            .map_err(|e| e.to_string())?;
        if let Some(a) = agency {
            store
                .set_entry_party(entry_id, Some(a))
                .map_err(|e| e.to_string())?;
        }
        Ok(Json(RemitPayrollTaxesOut {
            entry_id,
            total: money(total),
        }))
    }

    #[tool(
        name = "books.list_payroll_runs",
        title = "List payroll runs",
        description = "List payroll runs in id order: date, employee count, totals, and the posted entry id.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_payroll_runs(&self) -> Result<Json<ListPayrollRunsOut>, String> {
        let store = self.lock()?;
        let runs = store
            .list_payroll_runs()
            .into_iter()
            .map(|r| {
                // A run is as journaled (rules.md B-28): a total past exact
                // arithmetic refuses the list, never a panic.
                let total_gross =
                    checked_sum(r.lines.iter().map(|l| Ok(l.gross_pay)), "gross pay")?;
                Ok(PayrollRunOut {
                    id: r.id,
                    date: r.date.to_string(),
                    employee_count: r.lines.len(),
                    total_gross: money(total_gross),
                    entry_id: r.entry_id,
                })
            })
            .collect::<Result<_, String>>()?;
        Ok(Json(ListPayrollRunsOut { runs }))
    }
}
