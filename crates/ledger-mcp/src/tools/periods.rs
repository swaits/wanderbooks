//! Calendar periods through the server: `books.create_period`,
//! `books.close_period` and `books.list_periods`. Split whole out of
//! `server.rs` (plan/served.md 8.2 U0a).

use super::*;

#[tool_router(router = periods_router, vis = "pub(super)")]
impl BooksServer {
    #[tool(
        name = "books.create_period",
        title = "Create a period",
        description = "Create an accounting period (month, quarter, year). Dates are YYYY-MM-DD, inclusive.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn create_period(
        &self,
        params: Parameters<CreatePeriodParams>,
    ) -> Result<Json<CreatePeriodOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let id = store
            .create_period(&p.name, parse_date(&p.start)?, parse_date(&p.end)?)
            .map_err(|e| e.to_string())?;
        Ok(Json(CreatePeriodOut { period_id: id }))
    }

    #[tool(
        name = "books.close_period",
        title = "Close a period",
        description = "Close an accounting period. The closing entry, dated at period end, zeroes revenue and expense balances into Retained Earnings or, for a multi-member LLC (members in the settings), into each member's capital account by share, cumulatively: each close gives a member their share of everything closed to the members so far, less what earlier closes gave them (odd cents by largest remainder), so no member is ever a cent off their exact share; a close may carry a one-time correction of earlier rounding. Periods close in order. Without confirmed=true, returns the exact closing entry and members' split, closing nothing; with confirmed=true, posts it and closes the period to new entries.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn close_period(
        &self,
        params: Parameters<ClosePeriodParams>,
    ) -> Result<Json<ClosePeriodOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        // Verify the period exists.
        if store.ledger().get_period(p.period_id).is_none() {
            return Err(format!("unknown period {}", p.period_id));
        }
        let to = self.close_target(store.ledger())?;
        let describe = |ledger: &Ledger, eid: Option<ledger_core::EntryId>| -> Vec<String> {
            let Some(entry) = eid.and_then(|e| ledger.get_entry(e)) else {
                return Vec::new();
            };
            entry
                .lines()
                .iter()
                .map(|line| {
                    let code = ledger
                        .chart()
                        .get(line.account)
                        .map(|a| a.code.clone())
                        .unwrap_or_default();
                    format!("{:?} {} {}", line.side, code, money(line.amount.amount))
                })
                .collect()
        };
        // The members' split, from the closed ledger (rules.md B-13).
        let split = |ledger: &Ledger| -> Option<MembersCloseOut> {
            let (since_close, parts) = ledger.member_close_split(p.period_id)?;
            let names = self.member_capitals(ledger).unwrap_or_default();
            let members = parts
                .iter()
                .map(|m| MemberCloseOut {
                    name: names
                        .iter()
                        .find(|c| c.capital_account == m.capital)
                        .map(|c| c.name.clone())
                        .unwrap_or_default(),
                    capital_account: ledger
                        .chart()
                        .get(m.capital)
                        .map(|a| a.code.clone())
                        .unwrap_or_default(),
                    share: ledger_package::share_text(m.share),
                    this_close: money(m.part),
                    running: money(m.running),
                    exact: money(m.exact.normalize()),
                })
                .collect();
            Some(MembersCloseOut {
                since_close,
                members,
            })
        };
        let digest = self.preview_of(
            &store,
            "books.close_period",
            p.confirmed,
            p.preview.as_deref(),
        )?;
        if !p.confirmed {
            // Preview: run the real close on a copy of the ledger, so the
            // preview is exactly what confirming would post.
            let mut scratch = store.ledger().clone();
            let closing = scratch
                .close_period_to(p.period_id, &to)
                .map_err(|e| e.to_string())?;
            return Ok(Json(ClosePeriodOut {
                status: "preview".to_string(),
                period_id: p.period_id,
                closing_entry: None,
                lines: describe(&scratch, closing),
                members: split(&scratch),
                preview: Some(digest),
            }));
        }
        let closing = store
            .close_period_to(p.period_id, &to)
            .map_err(|e| e.to_string())?;
        let lines = describe(store.ledger(), closing);
        Ok(Json(ClosePeriodOut {
            status: "closed".to_string(),
            period_id: p.period_id,
            closing_entry: closing,
            lines,
            members: split(store.ledger()),
            preview: Some(digest),
        }))
    }

    #[tool(
        name = "books.list_periods",
        title = "List periods",
        description = "List all accounting periods with their status, and for a period closed to the members, the rounding rule it was closed under.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_periods(&self) -> Result<Json<ListPeriodsOut>, String> {
        let store = self.lock()?;
        let periods = store
            .ledger()
            .list_periods()
            .into_iter()
            .map(|p| PeriodOut {
                id: p.id,
                name: p.name.clone(),
                start: p.start.to_string(),
                end: p.end.to_string(),
                status: match p.status {
                    PeriodStatus::Open => "open".to_string(),
                    PeriodStatus::Closed => "closed".to_string(),
                },
                closing_entry: p.closing_entry,
                close_rule: p.members_closed.as_ref().map(|m| match m.rule {
                    ledger_core::MemberRule::Cumulative { since_seq } => {
                        format!("members_cumulative_since:{since_seq}")
                    }
                    ledger_core::MemberRule::Run => "members_run".to_string(),
                    ledger_core::MemberRule::PerClose => "members_per_close".to_string(),
                    ledger_core::MemberRule::LastAbsorbs => "members_last_absorbs".to_string(),
                }),
            })
            .collect();
        Ok(Json(ListPeriodsOut { periods }))
    }
}
