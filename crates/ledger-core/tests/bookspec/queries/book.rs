use crate::*;

/// The engine's key for a bookspec setting key (`SETTING_KEYS`): the
/// engine's own settings carry the `company.` prefix. A key the schema
/// admits and this map lacks is a gap in the adapter, so it panics.
pub(crate) fn engine_setting_key(bookspec_key: &str) -> &'static str {
    match bookspec_key {
        "fiscal_year_start" => "company.fiscal_year_start",
        other => panic!("the adapter maps no engine key for the bookspec setting {other:?}"),
    }
}

impl Books<'_> {
    /// A `setting` query (BS-ledger-9): the value the books keep, with who
    /// decided it and when. The books hold their settings in the journal
    /// (wanderbooks' pick, facts-in-the-books), so a replay reaches them.
    pub(crate) fn answer_setting(&self, q: &Query) -> Result<Value, Refusal> {
        let rule = RuleId::new(Area::Ledger, 9);
        let key = q.key.as_deref().ok_or_else(|| self.missing("key"))?;
        let recorded = self
            .l()
            .settings()
            .get(engine_setting_key(key))
            .ok_or_else(|| {
                Refusal::new(rule.clone(), format!("no setting {key:?} in the books"))
            })?;
        let value = match &recorded.value {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        Ok(json_figures([
            ("value".to_string(), value),
            ("decided_by".to_string(), recorded.source.decided_by.clone()),
            (
                "decided_on".to_string(),
                recorded.source.decided_on.to_string(),
            ),
        ]))
    }

    /// A `role_account` query (BS-ledger-18): the account a control role
    /// posts to, read from the books' own journaled assignment.
    pub(crate) fn answer_role_account(&self, q: &Query) -> Result<Value, Refusal> {
        let rule = RuleId::new(Area::Ledger, 18);
        let role = q.role.as_deref().ok_or_else(|| self.missing("role"))?;
        let control = control_account(role)
            .ok_or_else(|| Refusal::new(rule.clone(), format!("{role:?} is no control role")))?;
        let account = self
            .l()
            .role_account(control)
            .and_then(|id| self.l().chart().get(id))
            .map(|a| a.code.clone())
            .unwrap_or_else(|| "unassigned".to_string());
        Ok(json_figures([("account".to_string(), account)]))
    }

    /// A `tax_summary` query (BS-ledger-21): each category's deductible
    /// amount, as `tax::tax_summary` figures it (one rounding, on the
    /// category's exact total: wanderbooks' pick, round-once-on-the-total).
    pub(crate) fn answer_tax_summary(&self, q: &Query) -> Result<Value, Refusal> {
        let rule = RuleId::new(Area::Ledger, 21);
        let (from, to) = self.q_period(q)?;
        let summary = tax_summary(self.l(), from, to, None).map_err(|e| engine_refusal(rule, e))?;
        let mut out = JsonMap::new();
        for line in summary
            .income
            .iter()
            .chain(&summary.cost_of_goods)
            .chain(&summary.deductions)
            .chain(&summary.separately_stated)
        {
            out.insert(line.category.clone(), Value::String(figure(line.tax)));
        }
        Ok(Value::Object(out))
    }
}

/// Check one `setting` expectation (BS-ledger-9): the value, who decided it
/// and when, as the books keep them.
pub(crate) fn check_setting(b: &Books<'_>, x: &Expect) {
    let ex = b.ex;
    undated(ex, x);
    exactly(
        ex,
        x,
        &["decided_by", "decided_on", "value"]
            .map(String::from)
            .into(),
    );
    let got = b
        .answer(&x.query())
        .unwrap_or_else(|e| panic!("{}: setting refused ({})", ex.id, e.reason));
    let got = got.as_object().expect("a setting answers figures");
    for (k, want) in x.figures(&ex.id) {
        assert_eq!(
            got.get(k).and_then(Value::as_str),
            Some(want.as_str()),
            "{}: setting {k}",
            ex.id
        );
    }
}

/// Check one `role_account` expectation (BS-ledger-18): the code the role's
/// account has in the books.
pub(crate) fn check_role_account(b: &Books<'_>, x: &Expect) {
    let ex = b.ex;
    undated(ex, x);
    exactly(ex, x, &["account".to_string()].into());
    let got = b
        .answer(&x.query())
        .unwrap_or_else(|e| panic!("{}: role_account refused ({})", ex.id, e.reason));
    let got = got.as_object().expect("a role account answers figures");
    for (k, want) in x.figures(&ex.id) {
        assert_eq!(
            got.get(k).and_then(Value::as_str),
            Some(want.as_str()),
            "{}: role_account {k}",
            ex.id
        );
    }
}

/// Check one `tax_summary` expectation (BS-ledger-21): each category's
/// deductible amount, by category key.
pub(crate) fn check_tax_summary(b: &Books<'_>, x: &Expect) {
    let ex = b.ex;
    let (from, to) = period(ex, x);
    assert!(
        from <= to,
        "{}: tax_summary from {from} after to {to}",
        ex.id
    );
    let got = b
        .answer(&x.query())
        .unwrap_or_else(|e| panic!("{}: tax_summary refused ({})", ex.id, e.reason));
    let got = got.as_object().expect("a tax summary answers figures");
    for (k, want) in x.figures(&ex.id) {
        assert_eq!(
            got.get(k).and_then(Value::as_str),
            Some(want.as_str()),
            "{}: tax_summary {k}",
            ex.id
        );
    }
}
