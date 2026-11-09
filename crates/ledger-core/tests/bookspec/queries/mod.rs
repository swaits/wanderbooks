use crate::*;

pub(crate) mod assets;
pub(crate) mod book;
pub(crate) mod commands;
pub(crate) mod dates;
pub(crate) mod documents;
pub(crate) mod drafts;
pub(crate) mod formats;
pub(crate) mod parties;
pub(crate) mod statements;
pub(crate) mod tax;

impl Books<'_> {
    /// A refusal of a question the example did not fill in.
    pub(crate) fn missing(&self, what: &str) -> Refusal {
        Refusal::new(
            RuleId::new(Area::Ledger, 1),
            format!("{} names its {what}", "the query"),
        )
    }

    /// The period a query covers, or the rule's refusal.
    pub(crate) fn q_period(&self, q: &Query) -> Result<(NaiveDate, NaiveDate), Refusal> {
        resolved_q_period(self.ex, q).map_err(|e| Refusal::of_reason(refusal(&e)))
    }

    /// The day an as-of query is made as of.
    pub(crate) fn q_day(&self, q: &Query) -> Result<NaiveDate, Refusal> {
        resolved_q_day(self.ex, q).map_err(|e| Refusal::of_reason(refusal(&e)))
    }

    /// The event whose entry `entry` is, counting from 1.
    pub(crate) fn event_of(&self, entry: EntryId) -> Option<usize> {
        self.posted
            .iter()
            .position(|entries| entries.contains(&entry))
            .map(|i| i + 1)
    }

    /// The entry event `n` posted.
    pub(crate) fn entry_or_refuse(&self, n: usize) -> Result<EntryId, Refusal> {
        assert!(n >= 1 && n <= self.posted.len(), "event {n} not applied");
        match self.posted[n - 1].as_slice() {
            [entry] => Ok(*entry),
            other => Err(Refusal::new(
                RuleId::new(Area::Ledger, 1),
                format!("event {n} posted {} entries", other.len()),
            )),
        }
    }

    /// The account `code` names.
    pub(crate) fn account_or_refuse(&self, code: &str) -> Result<ledger_core::AccountId, Refusal> {
        self.l()
            .chart()
            .find_by_code(code)
            .map(|a| a.id)
            .ok_or_else(|| Refusal::new(RuleId::new(Area::Ledger, 1), format!("no account {code}")))
    }

    /// The payment accounts the example names.
    pub(crate) fn payment_accounts(&self) -> Result<PaymentAccounts, Refusal> {
        let ex = &self.ex;
        Ok(PaymentAccounts {
            cash: codes(self.l(), &ex.setup.cash_accounts, &ex.id),
            financing: codes(self.l(), &ex.setup.financing_accounts, &ex.id),
        })
    }

    /// The engine's answer to one query, as the runner compares it: exact
    /// decimal strings, never floats; dates `YYYY-MM-DD`.
    pub(crate) fn answer(&self, q: &Query) -> Result<Value, Refusal> {
        match q.name.as_str() {
            "cash_flow" => self.answer_cash_flow(q),
            "trial_balance" => self.answer_trial_balance(q),
            "profit_and_loss" => self.answer_profit_and_loss(q),
            "entry" => self.answer_entry(q),
            "balances" => self.answer_balances(q),
            "members_equity" => self.answer_members_equity(q),
            "k1" => self.answer_k1(q),
            "k1_tax_basis" => self.answer_k1_tax_basis(q),
            "k1_reconciliation" => self.answer_k1_reconciliation(q),
            "vendor_1099_paid" => self.answer_vendor_1099_paid(q),
            "vendor_1099" => self.answer_vendor_1099(q),
            "party_part" => self.answer_party_part(q),
            "party_totals" => self.answer_party_totals(q),
            "party_tin_last4" => self.answer_party_tin_last4(q),
            "return_tin" => self.answer_return_tin(q),
            "project" => self.answer_project(q),
            "aging" => self.answer_aging(q),
            "document_dates" => self.answer_document_dates(q),
            "document" => self.answer_document(q),
            "invoice_numbers" => self.answer_invoice_numbers(q),
            "draft_state" => self.answer_draft_state(q),
            "evidence" => self.answer_evidence(q),
            "setting" => self.answer_setting(q),
            "role_account" => self.answer_role_account(q),
            "tax_summary" => self.answer_tax_summary(q),
            "run" => self.answer_run(q),
            "disposal" => self.answer_disposal(q),
            "depreciation_schedule" => self.answer_depreciation_schedule(q),
            "schedule_plan" => self.answer_schedule_plan(q),
            "register_row" => self.answer_register_row(q),
            "register_totals" => self.answer_register_totals(q),
            "asset_basis" => self.answer_asset_basis(q),
            "period" => {
                let (from, to) = self.q_period(q)?;
                Ok(json_figures([
                    ("from".to_string(), from.to_string()),
                    ("to".to_string(), to.to_string()),
                ]))
            }
            "day" => Ok(json_figures([(
                "as_of".to_string(),
                self.q_day(q)?.to_string(),
            )])),
            "calendar_year" => Ok(json_figures([(
                "year".to_string(),
                defaults(self.ex, &q.name).calendar_year().to_string(),
            )])),
            "account_names" => Ok(json_figures(
                self.l()
                    .chart()
                    .all()
                    .map(|a| (a.code.clone(), a.name.clone())),
            )),
            "uncategorized_accounts" => Ok(Value::Array(
                ledger_core::tax::uncategorized_accounts(self.l())
                    .into_iter()
                    .map(|a| Value::String(format!("{} {}", a.code, a.name)))
                    .collect(),
            )),
            "iif_lines" | "iif_changes" | "iif_line_ending" => self.answer_iif(q),
            "rename_account" | "add_account" | "set_tax_category" => self.answer_command(q),
            other => Err(Refusal::new(
                RuleId::new(Area::Ledger, 1),
                format!("no answer for query {other:?}"),
            )),
        }
    }
}
