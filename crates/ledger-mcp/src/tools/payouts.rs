//! Payouts through the server (`books.record_payout`). Split whole out
//! of `server.rs` (plan/served.md 8.2 U0a).

use super::*;

#[tool_router(router = payouts_router, vis = "pub(super)")]
impl BooksServer {
    #[tool(
        name = "books.record_payout",
        title = "Record a platform payout",
        description = "Record a platform payout (app store, payment processor, marketplace) grossed up: Dr the deposit account the net deposit, Dr fee accounts the fees withheld, Dr/Cr adjustments (refunds, chargebacks, reserves, taxes withheld), Cr revenue the gross sales (several lines and classes allowed). Refuses unless gross - fees +/- adjustments equals the deposit to the cent, and says by how much it's off. Creates a draft: attach the payout statement as its receipt, preview, and post.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn record_payout(
        &self,
        params: Parameters<RecordPayoutParams>,
    ) -> Result<Json<RecordPayoutOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let class = |store: &Store, c: &Option<String>| -> Result<Option<u32>, String> {
            c.as_deref().map(|c| resolve_class(store, c)).transpose()
        };
        let lines = |store: &Store, ls: &[PayoutLineParam]| {
            ls.iter()
                .map(|l| {
                    Ok(ledger_core::PayoutLine {
                        account: resolve_account(store, &l.account)?,
                        amount: parse_amount(&l.amount)?,
                        class: class(store, &l.class)?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()
        };
        let payout = ledger_core::Payout {
            deposit_account: resolve_account(&store, &p.deposit_account)?,
            deposit: parse_amount(&p.deposit)?,
            revenue: lines(&store, &p.revenue)?,
            fees: lines(&store, &p.fees)?,
            adjustments: p
                .adjustments
                .iter()
                .map(|a| {
                    Ok(ledger_core::PayoutAdjustment {
                        account: resolve_account(&store, &a.account)?,
                        side: parse_side(&a.side)?,
                        amount: parse_amount(&a.amount)?,
                        class: class(&store, &a.class)?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?,
        };
        let (plan, totals) =
            ledger_core::payout_plan(&payout, store.ledger().chart()).map_err(|e| e.to_string())?;
        let currency = store.base_currency();
        let entry_id = store
            .create_entry(parse_date(&p.date)?, &p.description)
            .map_err(|e| e.to_string())?;
        let mut described = Vec::new();
        let mut tagged: Vec<u32> = Vec::new();
        for (index, (account, side, amount, class)) in plan.into_iter().enumerate() {
            store
                .add_line(entry_id, account, side, Money::new(amount, currency))
                .map_err(|e| e.to_string())?;
            if let Some(class) = class {
                store
                    .set_line_class(entry_id, index, Some(class))
                    .map_err(|e| e.to_string())?;
                if !tagged.contains(&class) {
                    tagged.push(class);
                }
            }
            let code = store
                .ledger()
                .chart()
                .get(account)
                .map(|a| a.code.clone())
                .unwrap_or_default();
            described.push(format!("{side:?} {code} {}", money(amount)));
        }
        Ok(Json(RecordPayoutOut {
            entry_id,
            status: "draft".to_string(),
            gross: money(totals.gross),
            fees: money(totals.fees),
            adjustments: money(totals.adjustments),
            deposit: money(totals.deposit),
            lines: described,
            notes: tagged
                .into_iter()
                .filter_map(|c| projects::late_tag_note(store.ledger(), c))
                .collect(),
            next: "Attach the payout statement (books.attach_receipt), then preview and post with books.post_entry.".to_string(),
        }))
    }
}
