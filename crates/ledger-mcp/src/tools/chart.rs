//! The chart of accounts through the server: `books.chart`,
//! `books.add_account`, the chart template and CSV import, renaming and
//! renumbering, and the control roles. Split whole out of `server.rs`
//! (plan/served.md 8.2 U0a).

use super::*;

/// The settings that disagree with the books: each names the setting and
/// what to change. A code must name an account in the chart, of the kind
/// the setting needs (cash and investing: assets; financing: liabilities;
/// capital, distributions, close_into, close_sweep: equity).
/// The live conventions that name account `code` (a posting template's
/// line, an approval rule's accounts): after a renumber, each still names
/// the old code until revised.
fn convention_mentions(ledger: &Ledger, code: &str) -> Vec<String> {
    convention_codes(ledger)
        .into_iter()
        .filter(|(_, c)| c == code)
        .map(|(id, _)| format!("convention {id} (revise it with books.set_convention)"))
        .collect()
}

/// Every control role and the account filling it, for books.roles.
fn roles_out(ledger: &Ledger) -> Vec<RoleOut> {
    ledger_core::control_accounts()
        .map(|c| {
            let account = ledger.role_account(c).and_then(|id| ledger.chart().get(id));
            RoleOut {
                role: c.key.to_string(),
                purpose: c.role.to_string(),
                account_type: format!("{:?}", c.account_type),
                default_code: c.code.to_string(),
                assigned: ledger.role_assignment(c).is_some(),
                account_code: account.map(|a| a.code.clone()),
                account_name: account.map(|a| a.name.clone()),
            }
        })
        .collect()
}

fn parse_account_type(s: &str) -> Result<AccountType, String> {
    match s.to_lowercase().as_str() {
        "asset" => Ok(AccountType::Asset),
        "liability" => Ok(AccountType::Liability),
        "equity" => Ok(AccountType::Equity),
        "revenue" => Ok(AccountType::Revenue),
        "expense" => Ok(AccountType::Expense),
        _ => Err(format!(
            "bad account_type {s:?}: expected asset, liability, equity, revenue, or expense"
        )),
    }
}

#[tool_router(router = chart_router, vis = "pub(super)")]
impl BooksServer {
    #[tool(
        name = "books.chart",
        title = "Chart of accounts",
        description = "List the chart of accounts with current balances. Start here: every line you post must use a real account from this chart.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub(super) async fn chart(&self) -> Result<Json<ChartOut>, String> {
        let store = self.lock()?;
        let ledger = store.ledger();
        let mut accounts = Vec::new();
        let mut all: Vec<_> = ledger.chart().all().collect();
        all.sort_by(|a, b| a.code.cmp(&b.code));
        for a in all {
            let balance = ledger.account_balance(a.id).map_err(|e| e.to_string())?;
            accounts.push(ChartAccount {
                id: a.id,
                code: a.code.clone(),
                name: a.name.clone(),
                account_type: format!("{:?}", a.account_type).to_lowercase(),
                parent: a.parent,
                placeholder: a.placeholder,
                balance: money(if balance.is_zero() {
                    Decimal::ZERO
                } else {
                    balance
                }),
                tax_category: a.tax_category.clone(),
                deductible_percent: a.deductible_percent.map(|p| p.to_string()),
            });
        }
        Ok(Json(ChartOut {
            accounts,
            base_currency: store.base_currency().code().to_string(),
        }))
    }

    #[tool(
        name = "books.add_account",
        title = "Add an account",
        description = "Add an account to the chart. Codes must be unique; check books.chart first. Give a revenue or expense account its tax_category (books.tax_categories): without one the result warns, or, under policy.require_tax_category = refuse, the account is refused. A name another account under the same parent has (ignoring capitals), one holding nine digits in a row, or one that isn't a single line of at most 200 characters, is refused.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn add_account(
        &self,
        params: Parameters<AddAccountParams>,
    ) -> Result<Json<AddAccountOut>, String> {
        let p = params.0;
        let account_type = parse_account_type(&p.account_type)?;
        let placeholder = p.placeholder.unwrap_or(false);
        let pct = match &p.deductible_percent {
            Some(s) => Some(parse_amount(s)?),
            None => None,
        };
        let category = p.tax_category.as_deref();
        if pct.is_some() && category.is_none() {
            return Err("deductible_percent needs a tax_category".into());
        }
        if let Some(cat) = category {
            ledger_core::tax::check_category(cat, account_type, self.settings().company.tax_form)
                .map_err(|e| e.to_string())?;
        }
        let mut store = self.lock()?;
        let id = store
            .add_account_with_tax(
                &p.code,
                &p.name,
                account_type,
                p.parent,
                placeholder,
                category.map(|c| (c, pct)),
            )
            .map_err(|e| e.to_string())?;
        let warnings =
            if category.is_none() && ledger_core::tax::needs_category(account_type, placeholder) {
                vec![uncategorized_warning(&p.code, &p.name)]
            } else {
                Vec::new()
            };
        Ok(Json(AddAccountOut {
            account_id: id,
            warnings,
        }))
    }

    #[tool(
        name = "books.chart_template",
        title = "Starter chart of accounts",
        description = "A starter chart of accounts for a kind of business (sole proprietorship, partnership or multi-member LLC, S corporation, C corporation): the right equity and owner-pay accounts (draws; per-partner capital and distributions plus guaranteed payments; officer compensation and distributions; dividends and income tax), common income and expense accounts with tax categories assigned, and the settings that go with it (changes for books.set_setting). A proposal: review it with the human, adjust, then books.import_chart_csv and books.set_setting.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn chart_template(
        &self,
        params: Parameters<ChartTemplateParams>,
    ) -> Result<Json<ChartTemplateOut>, String> {
        let p = params.0;
        let entity = ledger_import::EntityType::parse(&p.entity_type).ok_or_else(|| {
            format!(
                "entity_type {:?}: expected sole_proprietorship, partnership, s_corporation, or c_corporation (an LLC is one of these for tax purposes)",
                p.entity_type
            )
        })?;
        let t = ledger_import::chart_template(entity, &p.owners)?;
        // The template's settings, as the changes that set them in new
        // books (the one way settings change: books.set_setting).
        let settings = ledger_config::codec::import(
            &t.settings_toml,
            None,
            &ledger_core::settings::Settings::default(),
            self.currency(),
        )?
        .changes
        .into_iter()
        .filter_map(|c| {
            Some(TemplateSettingOut {
                value: c.value?,
                key: c.key,
            })
        })
        .collect();
        Ok(Json(ChartTemplateOut {
            csv: t.csv(),
            settings,
            notes: t.notes,
        }))
    }

    #[tool(
        name = "books.renumber_account",
        title = "Renumber an account",
        description = "Give an account a new code. Only the code changes: its id, name, and every entry posted to it stay, so balances and reports don't move. Refuses a code in use, or a control account's code (invoices, bills, payroll, closes) for an account of the wrong type. Lists any settings that still name the old code. Never renumber the human's accounts on your own: only when they ask.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn renumber_account(
        &self,
        params: Parameters<RenumberAccountParams>,
    ) -> Result<Json<RenumberAccountOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let id = resolve_account(&store, &p.account)?;
        let previous_code = store
            .ledger()
            .chart()
            .get(id)
            .expect("resolved")
            .code
            .clone();
        store
            .renumber_account(id, &p.code)
            .map_err(|e| e.to_string())?;
        Ok(Json(RenumberAccountOut {
            id,
            config_mentions: self
                .config_mentions(&previous_code)
                .into_iter()
                .chain(convention_mentions(store.ledger(), &previous_code))
                .collect(),
            previous_code,
            code: p.code,
        }))
    }

    #[tool(
        name = "books.rename_account",
        title = "Rename an account",
        description = "Give an account a new name. Only the name changes: its id, code, type, tax category and every entry posted to it stay, so no entry, balance or statement amount changes; every report, past periods included, prints the new name, and the journal keeps the old one. Refuses a blank name, the name it has, a name another account under the same parent has (ignoring capitals), one holding nine digits in a row, or one that isn't a single line of at most 200 characters. QuickBooks matches accounts by name: if the CPA imported an earlier IIF, tell them. Never rename the human's accounts on your own: only when they ask.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn rename_account(
        &self,
        params: Parameters<RenameAccountParams>,
    ) -> Result<Json<RenameAccountOut>, String> {
        use ledger_store::reconcilable;
        let p = params.0;
        let mut store = self.lock()?;
        let id = resolve_account(&store, &p.account)?;
        let reason = p.reason.as_deref().map(str::trim).filter(|r| !r.is_empty());
        // The sets the books pick by name, before and after (rules.md
        // B-31: a rename moves no amount, but it can move an account into
        // or out of one of these).
        let configured_cash = self.settings().roles.cash.clone();
        let picked = |ledger: &Ledger| {
            (
                configured_cash.is_empty()
                    && reconcilable::cash_accounts(ledger, &[]).contains(&id),
                reconcilable::card_accounts(ledger).contains(&id),
            )
        };
        let before = picked(store.ledger());
        let previous_name = store
            .ledger()
            .chart()
            .get(id)
            .expect("resolved")
            .name
            .clone();
        store
            .rename_account(id, &p.name, reason)
            .map_err(|e| e.to_string())?;
        let ledger = store.ledger();
        let a = ledger.chart().get(id).expect("resolved");
        let after = picked(ledger);
        let mut notes = vec![format!(
            "Every report, past periods included, prints \u{201c}{}\u{201d} now; the journal keeps \u{201c}{previous_name}\u{201d}. QuickBooks matches accounts by name: if the CPA imported an earlier IIF, tell them.",
            a.name
        )];
        let children = ledger
            .chart()
            .all()
            .filter(|c| c.parent == Some(id))
            .count();
        if children > 0 {
            notes.push(format!(
                "Its {children} sub-account{} carry the new name in their QuickBooks full names (IIF) too.",
                if children == 1 { "" } else { "s" }
            ));
        }
        for (was, is, what) in [
            (
                before.0,
                after.0,
                "a cash account (books.cash_accounts is unset, so cash accounts are the asset accounts whose names say cash, bank, checking, savings or money market): the reconciliation reminders and the viewer's cash figures, Cash page and cash flow statement follow it. Set books.cash_accounts to choose them by code",
            ),
            (
                before.1,
                after.1,
                "a card account (a liability whose name says card): the reconciliation reminders follow it",
            ),
        ] {
            if was != is {
                notes.push(format!(
                    "{} {} is {} {what}.",
                    a.code,
                    a.name,
                    if is { "now" } else { "no longer" }
                ));
            }
        }
        // A blank old name (an earlier version wrote one) is in every
        // text: it names no convention.
        let old = previous_name.trim().to_lowercase();
        for c in ledger
            .conventions()
            .conventions()
            .filter(|c| c.retired.is_none() && !old.is_empty())
            .filter(|c| {
                c.title.to_lowercase().contains(&old) || c.body_md.to_lowercase().contains(&old)
            })
        {
            notes.push(format!(
                "Convention {} names \u{201c}{previous_name}\u{201d} in its text: if it means this account, revise it with books.set_convention.",
                c.id
            ));
        }
        Ok(Json(RenameAccountOut {
            id,
            code: a.code.clone(),
            previous_name,
            name: a.name.clone(),
            notes,
        }))
    }

    #[tool(
        name = "books.roles",
        title = "Control account roles",
        description = "The control roles invoices, bills, payroll, and closes post to (receivables, revenue, payables, general expense, retained earnings, payroll expense and withholdings), and the account filling each: the one assigned with books.assign_role, else the account at the role's default code.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn roles(&self) -> Result<Json<RolesOut>, String> {
        let store = self.lock()?;
        Ok(Json(RolesOut {
            roles: roles_out(store.ledger()),
        }))
    }

    #[tool(
        name = "books.assign_role",
        title = "Assign a control role",
        description = "Assign a control role (see books.roles) one of the chart's own accounts, so invoices, bills, payroll, or closes post there instead of the default code; account null returns the role to its default code. Journaled. Refuses an account of the wrong type, a placeholder, one already filling another role, and moving a receivables, payables, withholding, or retained earnings role while its account holds a balance (move the balance with an entry first). Posted entries never change. Only when the human agrees.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn assign_role(
        &self,
        params: Parameters<AssignRoleParams>,
    ) -> Result<Json<RolesOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let account = match &p.account {
            Some(a) => Some(resolve_account(&store, a)?),
            None => None,
        };
        store
            .assign_role(&p.role, account)
            .map_err(|e| e.to_string())?;
        Ok(Json(RolesOut {
            roles: roles_out(store.ledger()),
        }))
    }

    #[tool(
        name = "books.import_chart_csv",
        title = "Import a chart of accounts",
        description = "Import a chart of accounts from CSV (header: code,name,type,parent_code,tax_category; the last is optional), as upload_id: upload it with books.begin_upload purpose import and `wanderbooks upload` (a file never passes through you). Types: asset, liability, equity, revenue, expense. Parents must come before children. Fails on the first bad row; nothing is half-imported. A revenue or expense row without a tax_category is listed in warnings, or, under policy.require_tax_category = refuse, refuses the file.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn import_chart_csv(
        &self,
        params: Parameters<ImportChartCsvParams>,
    ) -> Result<Json<ImportChartCsvOut>, String> {
        let p = params.0;
        let (csv, claim) = self.csv_given(&p.upload_id)?;
        let rows = ledger_import::parse_chart_csv(&csv).map_err(|e| e.to_string())?;
        let mut store = self.lock()?;
        let parse_type = |t: &str| match t {
            "asset" => Ok(AccountType::Asset),
            "liability" => Ok(AccountType::Liability),
            "equity" => Ok(AccountType::Equity),
            "revenue" => Ok(AccountType::Revenue),
            "expense" => Ok(AccountType::Expense),
            _ => Err(format!("bad account type: {t}")),
        };
        // Rehearse the whole import on a copy of the ledger first, so a bad
        // row anywhere (a duplicate within the file, a parent listed after
        // its child or of another type, a blank code, a name a sibling
        // has, a missing category the policy requires) refuses the file
        // and nothing is half-imported. The old up-front checks covered
        // only clashes with the existing chart and tax categories.
        let mut scratch = store.ledger().clone();
        let mut warnings = Vec::new();
        for row in &rows {
            let account_type = parse_type(&row.account_type)?;
            let parent = match &row.parent_code {
                Some(code) => Some(
                    scratch
                        .chart()
                        .find_by_code(code)
                        .map(|a| a.id)
                        .ok_or_else(|| format!("{}: unknown parent code {code}", row.code))?,
                ),
                None => None,
            };
            let id = scratch
                .add_account(&row.code, &row.name, account_type, parent, false)
                .map_err(|e| format!("{}: {e}", row.code))?;
            // A row is never a header account (the file can't say so), so
            // a revenue or expense row needs a category under refuse.
            scratch
                .check_tax_category_required(
                    &row.code,
                    &row.name,
                    account_type,
                    false,
                    row.tax_category.as_deref(),
                )
                .map_err(|e| {
                    format!(
                        "{}: {e}, as the file's fifth column (tax_category); a row of the file is never a header account",
                        row.code
                    )
                })?;
            if let Some(cat) = &row.tax_category {
                ledger_core::tax::check_category(
                    cat,
                    account_type,
                    self.settings().company.tax_form,
                )
                .map_err(|e| format!("{}: {e}", row.code))?;
                // A category an earlier version doesn't know waits for the
                // books' format upgrade: refused here, with the file whole,
                // never after the rows before it were written.
                if let Some(f) = ledger_store::format::tax_category_feature(cat) {
                    store
                        .ensure_feature(f)
                        .map_err(|e| format!("{}: {e}", row.code))?;
                }
                scratch
                    .set_account_tax(id, Some(cat), None)
                    .map_err(|e| format!("{}: {e}", row.code))?;
            } else if ledger_core::tax::needs_category(account_type, false) {
                warnings.push(uncategorized_warning(&row.code, &row.name));
            }
        }
        // Rehearsed whole: the upload is used up as the writing begins.
        files::filed(claim);
        let mut created = Vec::new();
        for row in rows {
            let account_type = match row.account_type.as_str() {
                "asset" => AccountType::Asset,
                "liability" => AccountType::Liability,
                "equity" => AccountType::Equity,
                "revenue" => AccountType::Revenue,
                "expense" => AccountType::Expense,
                _ => return Err(format!("bad account type: {}", row.account_type)),
            };
            let parent = match &row.parent_code {
                Some(code) => Some(
                    store
                        .ledger()
                        .chart()
                        .find_by_code(code)
                        .map(|a| a.id)
                        .ok_or_else(|| format!("unknown parent code: {code}"))?,
                ),
                None => None,
            };
            store
                .add_account_with_tax(
                    &row.code,
                    &row.name,
                    account_type,
                    parent,
                    false,
                    row.tax_category.as_deref().map(|c| (c, None)),
                )
                .map_err(|e| e.to_string())?;
            created.push(row.code.clone());
        }
        let count = created.len();
        Ok(Json(ImportChartCsvOut {
            created,
            count,
            warnings,
        }))
    }
}

#[cfg(test)]
mod tests {
    use crate::server::test_support::*;

    /// Settings naming a code are found: cash/financing/investing roles,
    /// close_into, and each member's capital and distributions accounts.
    #[test]
    fn config_mentions_finds_every_setting_naming_a_code() {
        let books = unit_dir("mentions");
        let server = served(settled(
            store(&books),
            &[
                members(),
                ("books.cash_accounts", serde_json::json!(["1000"])),
                ("books.financing_accounts", serde_json::json!(["2100"])),
                ("books.investing_accounts", serde_json::json!(["1400"])),
            ],
        ));
        assert_eq!(server.config_mentions("1000"), ["books.cash_accounts"]);
        assert_eq!(server.config_mentions("2100"), ["books.financing_accounts"]);
        assert_eq!(server.config_mentions("1400"), ["books.investing_accounts"]);
        assert_eq!(
            server.config_mentions("3000"),
            ["company.members Ada capital_account"]
        );
        assert_eq!(
            server.config_mentions("3001"),
            ["company.members Bo capital_account"]
        );
        assert_eq!(
            server.config_mentions("3100"),
            ["company.members Ada distribution_account"]
        );
        assert!(server.config_mentions("6000").is_empty());
        let books = unit_dir("mentions-solo");
        let solo = served(settled(
            store(&books),
            &[("books.close_into", serde_json::json!("3000"))],
        ));
        assert_eq!(solo.config_mentions("3000"), ["books.close_into"]);
    }
}
