use crate::*;

impl Books<'_> {
    pub(crate) fn answer_command(&self, q: &Query) -> Result<Value, Refusal> {
        let ex = &self.ex;
        let mut copy = self.l().clone();
        let result: Result<(), LedgerError> = match q.name.as_str() {
            "rename_account" => {
                let code = q.code.as_deref().ok_or_else(|| self.missing("account"))?;
                let name = q.new_name.as_deref().ok_or_else(|| self.missing("name"))?;
                let account = self.account_or_refuse(code)?;
                copy.rename_account(account, name)
            }
            "add_account" => {
                let a = q.account.as_ref().ok_or_else(|| self.missing("account"))?;
                let kind = account_type(a, &ex.id);
                let parent = a
                    .parent
                    .as_deref()
                    .map(|p| self.account_or_refuse(p))
                    .transpose()?;
                copy.add_account(&a.code, &a.name, kind, parent, a.header)
                    .and_then(|id| match &a.tax_category {
                        Some(c) => copy.set_account_tax(id, Some(c), None),
                        None => Ok(()),
                    })
                    .and_then(|()| {
                        self.l().check_tax_category_required(
                            &a.code,
                            &a.name,
                            kind,
                            a.header,
                            a.tax_category.as_deref(),
                        )
                    })
            }
            "set_tax_category" => {
                let code = q.code.as_deref().ok_or_else(|| self.missing("account"))?;
                let account = self.account_or_refuse(code)?;
                let category = q.category.as_deref();
                copy.set_account_tax(account, category, None)
                    .and_then(|()| self.l().check_tax_category_kept(account, category))
            }
            other => unreachable!("{other} is no command"),
        };
        result
            .map(|()| Value::Bool(true))
            .map_err(|e| Refusal::of_reason(refusal(&e)))
    }
}

/// The commands an example may try: each on a copy of the books, so it
/// changes nothing that is checked after it.
pub(crate) const COMMANDS: [&str; 3] = ["rename_account", "add_account", "set_tax_category"];

/// The account `code` names, for a command.
pub(crate) fn command_account(ex: &Example, l: &Ledger, x: &Expect) -> ledger_core::AccountId {
    let code = x
        .code
        .as_deref()
        .unwrap_or_else(|| panic!("{}: {} names its account (code)", ex.id, x.query));
    l.chart()
        .find_by_code(code)
        .unwrap_or_else(|| panic!("{}: no account {code}", ex.id))
        .id
}

/// Every balance, by code: what a change that moves no amount leaves as
/// it was.
pub(crate) fn every_balance(l: &Ledger) -> Vec<(String, Decimal, Decimal)> {
    trial_balance_as_of(l, ledger_core::calendar::LAST_DAY)
        .lines
        .into_iter()
        .map(|t| (t.code, t.debit, t.credit))
        .collect()
}

/// Try one command on a copy of the books, as the books' own command does
/// it (the engine's checks, then the policy's: rules.md B-31, B-32), and
/// check it is accepted or refused for the reason given.
pub(crate) fn check_command(ex: &Example, l: &Ledger, x: &Expect) {
    let mut copy = l.clone();
    let result: Result<(), LedgerError> = match x.query.as_str() {
        "rename_account" => {
            let account = command_account(ex, l, x);
            let name = x
                .name
                .as_deref()
                .unwrap_or_else(|| panic!("{}: rename_account gives the name", ex.id));
            copy.rename_account(account, name).inspect(|()| {
                // A rename changes the label, nothing else (rules.md B-31).
                assert_eq!(
                    copy.chart().get(account).unwrap().name,
                    name.trim(),
                    "{}: rename_account keeps the name trimmed",
                    ex.id
                );
                assert_eq!(
                    every_balance(&copy),
                    every_balance(l),
                    "{}: a rename moves no amount",
                    ex.id
                );
            })
        }
        "add_account" => {
            let a = x
                .account
                .as_ref()
                .unwrap_or_else(|| panic!("{}: add_account gives the account", ex.id));
            let kind = account_type(a, &ex.id);
            let parent = a.parent.as_deref().map(|p| {
                l.chart()
                    .find_by_code(p)
                    .unwrap_or_else(|| panic!("{}: no account {p}", ex.id))
                    .id
            });
            copy.add_account(&a.code, &a.name, kind, parent, a.header)
                .and_then(|id| match &a.tax_category {
                    Some(c) => copy.set_account_tax(id, Some(c), None),
                    None => Ok(()),
                })
                .and_then(|()| {
                    l.check_tax_category_required(
                        &a.code,
                        &a.name,
                        kind,
                        a.header,
                        a.tax_category.as_deref(),
                    )
                })
        }
        "set_tax_category" => {
            let account = command_account(ex, l, x);
            let category = x.category.as_deref();
            copy.set_account_tax(account, category, None)
                .and_then(|()| l.check_tax_category_kept(account, category))
        }
        q => unreachable!("{q} is no command"),
    };
    match (result, &x.refused) {
        (Ok(()), None) => {}
        (Err(e), Some(want)) => {
            assert_eq!(refusal(&e), *want, "{}: {} refused ({e})", ex.id, x.query)
        }
        (Err(e), None) => panic!("{}: {} refused ({e}), expected accepted", ex.id, x.query),
        (Ok(()), Some(want)) => panic!("{}: {} accepted, expected refused: {want}", ex.id, x.query),
    }
}
