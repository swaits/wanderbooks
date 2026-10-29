//! The books' own settings and guidance through the server:
//! `books.handbook`, `books.settings` and the recorded FX rates. Split
//! whole out of `server.rs` (plan/served.md 8.2 U0a).

use super::*;

#[tool_router(router = settings_router, vis = "pub(super)")]
impl BooksServer {
    #[tool(
        name = "books.handbook",
        title = "Bookkeeping handbook",
        description = "The guide to keeping these books: every procedure and rule, by section. Read the contract section first, then each section you need, before bookkeeping. Without section: the index; with section: that section's text (also at books://handbook/{id}); section \"all\": the whole handbook.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub(super) async fn handbook(
        &self,
        params: Parameters<HandbookParams>,
    ) -> Result<Json<HandbookOut>, String> {
        let settings = self.settings();
        let declared = self.document_line_projects();
        let documents_v2 = self.documents_v2();
        let sections = handbook::SECTIONS
            .iter()
            .map(|(id, _)| {
                let text =
                    handbook::section_with(id, &settings, self.currency(), declared, documents_v2)
                        .unwrap_or_default();
                HandbookSectionOut {
                    id: id.to_string(),
                    title: handbook::title(&text).to_string(),
                    uri: format!("{}/{id}", handbook::URI),
                    chars: text.chars().count(),
                }
            })
            .collect();
        let section = match params.0.section.as_deref().map(str::trim) {
            None => None,
            Some(id) => {
                let text = handbook::section_with(id, &settings, self.currency(), declared, documents_v2).ok_or_else(|| {
                    let ids: Vec<&str> = handbook::SECTIONS.iter().map(|(id, _)| *id).collect();
                    format!(
                        "no handbook section {id:?}: the sections are {}, or \"{}\" for all of them",
                        ids.join(", "),
                        handbook::ALL
                    )
                })?;
                let title = if id == handbook::ALL {
                    "The whole handbook".to_string()
                } else {
                    handbook::title(&text).to_string()
                };
                Some(HandbookTextOut {
                    id: id.to_string(),
                    title,
                    text,
                })
            }
        };
        Ok(Json(HandbookOut {
            read_first: "Read the contract section first: the rules every entry follows. Then each section you need, or section \"all\" for the whole handbook.".to_string(),
            sections,
            section,
        }))
    }

    #[tool(
        name = "books.settings",
        title = "Settings in force",
        description = "The settings in force (company, fiscal year, tax form, members, basis, receipt rule with every account it requires or exempts, cash flow roles, close target, time zone, approval rules), every key the books know with its value and who set it (change them with books.set_setting), and problems: settings or conventions that disagree with the chart, each saying what to change.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub(super) async fn settings_tool(&self) -> Result<Json<SettingsOut>, String> {
        // The books first: their settings sync as they lock, so the
        // settings read after are theirs.
        let store = self.lock()?;
        let cfg = self.settings();
        let c = &cfg.company;
        Ok(Json(SettingsOut {
            timezone: cfg.timezone.clone(),
            today: today().to_string(),
            approval_rules: match store.ledger().settings().approval_rules() {
                ledger_core::settings::ApprovalMode::Off => "off",
                ledger_core::settings::ApprovalMode::Enforce => "enforce",
            }
            .to_string(),
            keys: kept::keys_out(&store.ledger().settings().clone()),
            company: c.name.clone(),
            fiscal_year_start: format!("{:02}-{:02}", c.fiscal_year_start.0, c.fiscal_year_start.1),
            tax_form: c.tax_form.map(|f| f.label().to_string()),
            basis: c.basis.clone(),
            receipt_rule: cfg.policy.rule(self.currency()),
            receipt_required_accounts: cfg.policy.receipt_required_accounts.clone(),
            receipt_exempt_accounts: cfg.policy.receipt_exempt_accounts.clone(),
            parties: cfg.parties.key().to_string(),
            members: c
                .members
                .iter()
                .map(|m| MemberSettingOut {
                    name: m.name.clone(),
                    share: m.share.normalize().to_string(),
                    capital_account: m.capital_account.clone(),
                    distribution_account: m.distribution_account.clone(),
                    files_jointly_with: m.files_jointly_with.clone(),
                })
                .collect(),
            cash_accounts: cfg.roles.cash.clone(),
            financing_accounts: cfg.roles.financing.clone(),
            investing_accounts: cfg.roles.investing.clone(),
            close_into: c.close_into.clone(),
            close_sweep: c.close_sweep.clone(),
            problems: settings_problems(store.ledger(), &cfg),
        }))
    }

    #[tool(
        name = "books.set_fx_rate",
        title = "Set an exchange rate",
        description = "Record a historical FX rate: how much base currency one unit of the foreign currency buys on the given date. For reference; lines store their own rate at entry time.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn set_fx_rate(
        &self,
        params: Parameters<SetFxRateParams>,
    ) -> Result<Json<SetFxRateOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let currency = Currency::new(&p.currency).map_err(|e| e.to_string())?;
        store
            .set_fx_rate(currency, parse_date(&p.date)?, parse_amount(&p.rate)?)
            .map_err(|e| e.to_string())?;
        Ok(Json(SetFxRateOut { ok: true }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::test_support::*;

    /// A client that connects after the settings changed is told the rule
    /// now in force (plan P0): the connect text is composed from the
    /// books' settings as they are at initialize, not as they were when
    /// the server started.
    #[test]
    fn a_new_session_is_told_the_settings_in_force() {
        let books = unit_dir("info");
        let server = served(store(&books));
        let told = |s: &BooksServer| rmcp::ServerHandler::get_info(s).instructions.unwrap();
        let before = told(&server);
        assert!(before.contains("more than 25.00 USD in debits"), "{before}");
        {
            let mut store = server.lock().unwrap();
            store
                .change_settings(
                    &[ledger_core::settings::Change {
                        key: "policy.evidence.threshold".into(),
                        value: Some(serde_json::json!("0.00")),
                    }],
                    &ledger_core::settings::Source {
                        decided_by: "Test Owner".into(),
                        decided_on: d(2026, 10, 23),
                        citation: None,
                        note: None,
                    },
                )
                .unwrap();
        }
        let after = told(&server);
        assert!(
            after.contains("every entry needs at least one receipt"),
            "{after}"
        );
        assert!(!after.contains("25.00"), "{after}");
    }
}
