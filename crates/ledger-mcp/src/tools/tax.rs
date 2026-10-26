//! Tax categories through the server: `books.tax_categories` and
//! `books.set_tax_category`. Split whole out of `server.rs`
//! (plan/served.md 8.2 U0a).

use super::*;

#[tool_router(router = tax_router, vis = "pub(super)")]
impl BooksServer {
    #[tool(
        name = "books.tax_categories",
        title = "Tax categories",
        description = "The tax categories an account can map to (where it lands on the income tax return), filtered to the business's return when the settings say which (tax_form or tax_classification). Each has a default deductible percent (meals 50, nondeductible 0) and guidance. Categories are stable keys, not form line numbers.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn tax_categories(&self) -> Result<Json<TaxCategoriesOut>, String> {
        let form = self.settings().company.tax_form;
        let categories = ledger_core::tax::CATEGORIES
            .iter()
            .filter(|c| form.is_none_or(|f| c.forms.contains(&f)))
            .map(|c| TaxCategoryOut {
                key: c.key.to_string(),
                label: c.label.to_string(),
                kind: match c.kind {
                    ledger_core::TaxKind::Income => "income",
                    ledger_core::TaxKind::CostOfGoods => "cost_of_goods",
                    ledger_core::TaxKind::Deduction => "deduction",
                }
                .to_string(),
                default_deductible_percent: c.default_deductible_percent,
                note: c.note.to_string(),
            })
            .collect();
        Ok(Json(TaxCategoriesOut {
            form: form.map(|f| f.label().to_string()),
            categories,
        }))
    }

    #[tool(
        name = "books.set_tax_category",
        title = "Set an account's tax category",
        description = "Map a revenue or expense account to a tax category (see books.tax_categories), optionally overriding its deductible percent; category null clears it (refused under policy.require_tax_category = refuse, for a revenue or expense account that isn't a header). Drives the tax_summary report.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn set_tax_category(
        &self,
        params: Parameters<SetTaxCategoryParams>,
    ) -> Result<Json<SetTaxCategoryOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let id = resolve_account(&store, &p.account)?;
        let pct = match &p.deductible_percent {
            Some(s) => Some(parse_amount(s)?),
            None => None,
        };
        if let Some(cat) = &p.category {
            let t = store
                .ledger()
                .chart()
                .get(id)
                .expect("resolved")
                .account_type;
            ledger_core::tax::check_category(cat, t, self.settings().company.tax_form)
                .map_err(|e| e.to_string())?;
        }
        store
            .set_account_tax(id, p.category.as_deref(), pct)
            .map_err(|e| e.to_string())?;
        Ok(Json(SetTaxCategoryOut { ok: true }))
    }
}
