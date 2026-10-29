//! Prepaid schedules and recurring entries through the server: create,
//! list and run each. Split whole out of `server.rs` (plan/served.md 8.2
//! U0a).

use super::*;

fn schedule_out(ledger: &Ledger, s: &ledger_core::AmortizationSchedule) -> ScheduleOut {
    // A schedule an earlier version journaled may run past the last date
    // chrono holds (rules.md B-28): shown, with why its plan can't be.
    let plan = s.plan();
    let code = |id| {
        ledger
            .chart()
            .get(id)
            .map(|a| a.code.clone())
            .unwrap_or_default()
    };
    ScheduleOut {
        schedule_id: s.id,
        description: s.description.clone(),
        kind: match s.kind {
            ledger_core::ScheduleKind::PrepaidExpense => "prepaid_expense",
            ledger_core::ScheduleKind::DeferredRevenue => "deferred_revenue",
        }
        .to_string(),
        balance_account: code(s.balance_account),
        pl_account: code(s.pl_account),
        total: money(s.total.amount),
        convention: s.convention.key().to_string(),
        start: s.first_day().to_string(),
        end: s.last_day().map(|d| d.to_string()),
        next: s.next().ok().flatten().map(|d| d.to_string()),
        remaining: plan.as_ref().ok().and(s.remaining().ok()).map(money),
        months: plan
            .as_ref()
            .map(|plan| {
                plan.iter()
                    .map(|(d, a)| ScheduleMonthOut {
                        date: d.to_string(),
                        amount: money(*a),
                        posted: s.posted.contains(d),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        // A null remaining is always explained (ScheduleOut::plan_error).
        plan_error: plan
            .err()
            .or_else(|| s.remaining().err())
            .map(|e| e.to_string()),
    }
}

fn recurring_out(r: &ledger_core::RecurringEntry) -> RecurringOut {
    RecurringOut {
        recurring_id: r.id,
        description: r.description.clone(),
        frequency: format!("{:?}", r.frequency).to_lowercase(),
        start: r.start.to_string(),
        end: r.end.map(|e| e.to_string()),
        generated: r.generated.iter().map(|d| d.to_string()).collect(),
    }
}

#[tool_router(router = schedules_router, vis = "pub(super)")]
impl BooksServer {
    #[tool(
        name = "books.create_schedule",
        title = "Create an amortization schedule",
        description = "Spread a prepaid expense (an asset used up as time passes, e.g. a year of insurance paid up front) or deferred revenue (a liability earned as time passes, e.g. an annual subscription collected up front) over the time it covers, to the cent. convention: daily (by the day from start: a mid-month start posts a small first and last month, one month end more than its months) or monthly_even (equal months; starts on the 1st); omitted, the schedule_convention setting, else daily. Record the original payment first (Dr prepaid / Cr cash, with its receipt; or Dr cash / Cr deferred revenue). Returns the plan; post it with books.run_schedules.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn create_schedule(
        &self,
        params: Parameters<CreateScheduleParams>,
    ) -> Result<Json<ScheduleOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let kind = match p.kind.as_str() {
            "prepaid_expense" => ledger_core::ScheduleKind::PrepaidExpense,
            "deferred_revenue" => ledger_core::ScheduleKind::DeferredRevenue,
            other => {
                return Err(format!(
                    "kind {other:?}: expected prepaid_expense or deferred_revenue"
                ));
            }
        };
        // The books' setting, else daily (D-11, D-37).
        let convention = match p.convention.as_deref() {
            None => store.ledger().settings().schedule_convention(),
            Some(c) => ledger_core::ScheduleConvention::parse(c)
                .ok_or_else(|| format!("convention {c:?}: expected daily or monthly_even"))?,
        };
        let currency = store.ledger().base_currency();
        let balance_account = resolve_account(&store, &p.balance_account)?;
        let pl_account = resolve_account(&store, &p.pl_account)?;
        let id = store
            .register_schedule(ledger_core::NewSchedule {
                description: p.description,
                kind,
                total: Money::new(parse_amount(&p.total)?, currency),
                start: parse_date(&p.start)?,
                months: p.months,
                balance_account,
                pl_account,
                convention,
            })
            .map_err(|e| e.to_string())?;
        Ok(Json(schedule_out(
            store.ledger(),
            store.ledger().get_schedule(id).expect("registered"),
        )))
    }

    #[tool(
        name = "books.list_schedules",
        title = "List amortization schedules",
        description = "Amortization schedules (prepaid expenses, deferred revenue): each one's convention (daily or monthly_even), the first and last day it covers, each month's amount, what has posted, the next month to post, and what remains.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_schedules(&self) -> Result<Json<ListSchedulesOut>, String> {
        let store = self.lock()?;
        let ledger = store.ledger();
        Ok(Json(ListSchedulesOut {
            schedules: ledger
                .schedules()
                .map(|s| schedule_out(ledger, s))
                .collect(),
        }))
    }

    #[tool(
        name = "books.run_schedules",
        title = "Post scheduled amortization",
        description = "Post every amortization schedule's months ending on or before `through` (default today) that haven't posted. Idempotent: a month posts once; a month reversed in its own month is due again. A month in a closed period, or reversed after it ended, is listed under skipped, not posted. Without confirmed=true, returns the postings it would make. Exempt from the receipt policy: the schedule is the evidence (the original payment carries the receipt).",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn run_schedules(
        &self,
        params: Parameters<RunSchedulesParams>,
    ) -> Result<Json<RunSchedulesOut>, String> {
        let p = params.0;
        let through = match &p.through {
            Some(s) => parse_date(s)?,
            None => today(),
        };
        let mut store = self.lock()?;
        // A schedule whose plan can't be computed refuses the run, naming
        // it, before any month posts.
        let mut due: Vec<(NaiveDate, u64, Decimal)> = Vec::new();
        for s in store.ledger().schedules() {
            let months = s
                .due(through)
                .map_err(|e| format!("schedule {}: {e}", s.id))?;
            due.extend(months.into_iter().map(|(d, a)| (d, s.id, a)));
        }
        due.sort();
        // A month dated in a closed period can't post, nor one whose
        // posting was reversed after the month ended (it would count twice
        // until then): left for a human, and said, never a failed run.
        let (held, due): (Vec<_>, Vec<_>) = due
            .into_iter()
            .partition(|(d, id, _)| store.ledger().schedule_month_hold(*id, *d).is_some());
        let skipped: Vec<ScheduleSkippedOut> = held
            .into_iter()
            .map(|(d, id, _)| ScheduleSkippedOut {
                schedule_id: id,
                date: d.to_string(),
                reason: store
                    .ledger()
                    .schedule_month_hold(id, d)
                    .unwrap_or_default(),
            })
            .collect();
        let describe = |store: &Store, id: u64, date: NaiveDate| -> Result<String, String> {
            Ok(store
                .ledger()
                .preview_schedule_month(id, date)
                .map_err(|e| e.to_string())?
                .1)
        };
        let digest = self.preview_of(
            &store,
            "books.run_schedules",
            p.confirmed,
            p.preview.as_deref(),
        )?;
        if !p.confirmed {
            let postings = due
                .into_iter()
                .map(|(date, id, amount)| {
                    Ok(SchedulePostingOut {
                        schedule_id: id,
                        date: date.to_string(),
                        description: describe(&store, id, date)?,
                        amount: money(amount),
                        entry_id: None,
                    })
                })
                .collect::<Result<_, String>>()?;
            return Ok(Json(RunSchedulesOut {
                status: "preview".to_string(),
                postings,
                skipped,
                preview: Some(digest),
            }));
        }
        let mut postings = Vec::new();
        for (date, id, amount) in due {
            let description = describe(&store, id, date)?;
            let entry = store
                .post_schedule_month(id, date)
                .map_err(|e| e.to_string())?;
            postings.push(SchedulePostingOut {
                schedule_id: id,
                date: date.to_string(),
                description,
                amount: money(amount),
                entry_id: Some(entry),
            });
        }
        Ok(Json(RunSchedulesOut {
            status: "posted".to_string(),
            postings,
            skipped,
            preview: Some(digest),
        }))
    }

    #[tool(
        name = "books.create_recurring",
        title = "Create a recurring entry",
        description = "A recurring entry (rent, a subscription, a loan payment): balanced lines and a frequency (weekly, monthly, quarterly, yearly) from a start date, optionally ending. books.run_recurring turns due occurrences into drafts, never posted entries: attach each actual receipt and post through the normal preview.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn create_recurring(
        &self,
        params: Parameters<CreateRecurringParams>,
    ) -> Result<Json<RecurringOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let currency = store.ledger().base_currency();
        let frequency = ledger_core::Frequency::parse(&p.frequency).ok_or_else(|| {
            format!(
                "frequency {:?}: expected weekly, monthly, quarterly, or yearly",
                p.frequency
            )
        })?;
        let lines = p
            .lines
            .iter()
            .map(|l| {
                Ok((
                    resolve_account(&store, &l.account)?,
                    parse_side(&l.side)?,
                    Money::new(parse_amount(&l.amount)?, currency),
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let id = store
            .register_recurring(ledger_core::NewRecurring {
                description: p.description,
                lines,
                frequency,
                start: parse_date(&p.start)?,
                end: match &p.end {
                    Some(e) => Some(parse_date(e)?),
                    None => None,
                },
            })
            .map_err(|e| e.to_string())?;
        let r = store
            .ledger()
            .recurring_entries()
            .find(|r| r.id == id)
            .expect("registered");
        Ok(Json(recurring_out(r)))
    }

    #[tool(
        name = "books.list_recurring",
        title = "List recurring entries",
        description = "Recurring entries and the occurrences generated so far.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_recurring(&self) -> Result<Json<ListRecurringOut>, String> {
        let store = self.lock()?;
        Ok(Json(ListRecurringOut {
            recurring: store
                .ledger()
                .recurring_entries()
                .map(recurring_out)
                .collect(),
        }))
    }

    #[tool(
        name = "books.run_recurring",
        title = "Draft recurring entries",
        description = "Turn every recurring entry's occurrences on or before `through` (default today) into drafts. Idempotent: an occurrence generates once. Occurrences dated in a closed period are not generated; they come back in `skipped_closed_period` for the human to decide. Each draft then goes through the normal flow: attach the receipt, preview, post.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn run_recurring(
        &self,
        params: Parameters<RunRecurringParams>,
    ) -> Result<Json<RunRecurringOut>, String> {
        let through = match &params.0.through {
            Some(s) => parse_date(s)?,
            None => today(),
        };
        let mut store = self.lock()?;
        let drafts = store
            .run_recurring(through)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|(id, date, entry)| RecurringDraftOut {
                recurring_id: id,
                date: date.to_string(),
                entry_id: entry,
            })
            .collect();
        let skipped_closed_period = store
            .ledger()
            .recurring_in_closed_periods(through)
            .into_iter()
            .map(|(id, date)| RecurringSkippedOut {
                recurring_id: id,
                date: date.to_string(),
            })
            .collect();
        Ok(Json(RunRecurringOut {
            drafts,
            skipped_closed_period,
        }))
    }
}
