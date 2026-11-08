use crate::*;

impl Books<'_> {
    pub(crate) fn answer_cash_flow(&self, q: &Query) -> Result<Value, Refusal> {
        let (start, end) = self.q_period(q)?;
        let l = self.l();
        let params = CashFlowParams {
            start,
            end,
            cash_accounts: codes(l, &self.ex.setup.cash_accounts, &self.ex.id),
            financing_accounts: codes(l, &self.ex.setup.financing_accounts, &self.ex.id),
            investing_accounts: codes(l, &self.ex.setup.investing_accounts, &self.ex.id),
            members: self.members.clone(),
        };
        let cf = cash_flow(l, &params)
            .map_err(|e| engine_refusal(RuleId::new(Area::Statement, 3), e))?;
        let mut m = JsonMap::new();
        let put = |m: &mut JsonMap, key: &str, d: Decimal| {
            m.insert(key.to_string(), Value::String(figure(d)));
        };
        put(&mut m, "net_change", cf.net_change_in_cash);
        put(&mut m, "cash_beginning", cf.cash_beginning);
        put(&mut m, "cash_ending", cf.cash_ending);
        put(&mut m, "difference", cf.reconciles.difference);
        put(&mut m, "net_income", cf.operating_reconciliation.net_income);
        m.insert(
            "noncash.count".to_string(),
            Value::String(cf.noncash_disclosures.len().to_string()),
        );
        for (name, section) in [
            ("operating", CashFlowSection::Operating),
            ("investing", CashFlowSection::Investing),
            ("financing", CashFlowSection::Financing),
        ] {
            put(&mut m, name, cf.total(section));
            let lines = cf.lines(section);
            let entries: BTreeSet<EntryId> = lines.iter().map(|l| l.entry).collect();
            m.insert(
                format!("{name}.count"),
                Value::String(entries.len().to_string()),
            );
            for l in lines {
                let Some(n) = self.event_of(l.entry) else {
                    continue;
                };
                let key = format!("{name}.{n}");
                let amount = m
                    .get(&key)
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse::<Decimal>().ok())
                    .unwrap_or_default();
                m.insert(key, Value::String(figure(amount + l.amount)));
                let key = format!("{name}.{n}.on_behalf");
                let amount = m
                    .get(&key)
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse::<Decimal>().ok())
                    .unwrap_or_default();
                m.insert(key, Value::String(figure(amount + l.on_behalf)));
            }
        }
        for d in &cf.noncash_disclosures {
            if let Some(n) = self.event_of(d.entry) {
                let key = format!("noncash.{n}");
                let amount = m
                    .get(&key)
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse::<Decimal>().ok())
                    .unwrap_or_default();
                m.insert(key, Value::String(figure(amount + d.amount)));
            }
        }
        for (class, amount) in cf.operating_by_class() {
            let letter = ["a", "b", "c", "d", "e", "f", "g"]
                .into_iter()
                .find(|l| operating_class(l) == Some(class))
                .expect("a class has its letter");
            put(&mut m, &format!("class.{letter}"), amount);
        }
        for line in &cf.operating_reconciliation.lines {
            put(&mut m, &reconciliation_key(line), line.amount);
        }
        Ok(Value::Object(m))
    }

    pub(crate) fn answer_trial_balance(&self, q: &Query) -> Result<Value, Refusal> {
        let as_of = self.q_day(q)?;
        let tb = trial_balance_as_of(self.l(), as_of);
        if !tb.balanced() {
            return Err(Refusal::new(
                RuleId::new(Area::Statement, 1),
                "the trial balance balances",
            ));
        }
        Ok(json_figures(self.ex.setup.accounts.iter().map(|a| {
            let (dr, cr) = tb
                .lines
                .iter()
                .find(|t| t.code == a.code)
                .map_or((Decimal::ZERO, Decimal::ZERO), |t| (t.debit, t.credit));
            (a.code.clone(), figure(dr - cr))
        })))
    }

    pub(crate) fn answer_profit_and_loss(&self, q: &Query) -> Result<Value, Refusal> {
        let (start, end) = self.q_period(q)?;
        let pl = profit_and_loss_for_period(self.l(), start, end, None)
            .map_err(|e| engine_refusal(RuleId::new(Area::Statement, 3), e))?;
        let mut m = JsonMap::new();
        for a in self
            .ex
            .setup
            .accounts
            .iter()
            .filter(|a| matches!(a.kind.as_str(), "revenue" | "expense"))
        {
            let amount: Decimal = pl
                .revenue
                .iter()
                .chain(&pl.expenses)
                .filter(|s| s.code == a.code)
                .map(|s| s.amount)
                .sum();
            m.insert(a.code.clone(), Value::String(figure(amount)));
        }
        m.insert(
            "net_income".to_string(),
            Value::String(figure(pl.net_income())),
        );
        Ok(Value::Object(m))
    }

    pub(crate) fn answer_entry(&self, q: &Query) -> Result<Value, Refusal> {
        let n = q.entry.ok_or_else(|| self.missing("entry"))?;
        let entry = self
            .l()
            .get_entry(self.entry_or_refuse(n)?)
            .expect("an entry an event posted");
        let mut got: BTreeMap<String, Decimal> = BTreeMap::new();
        for line in entry.lines() {
            let code = &self.l().chart().get(line.account).unwrap().code;
            let side = match line.side {
                Side::Debit => "debit",
                Side::Credit => "credit",
            };
            *got.entry(format!("{code}.{side}")).or_default() += line.amount.amount;
        }
        Ok(json_figures(got.into_iter().map(|(k, v)| (k, figure(v)))))
    }

    pub(crate) fn answer_balances(&self, q: &Query) -> Result<Value, Refusal> {
        let as_of = self.q_day(q)?;
        let tb = trial_balance_as_of(self.l(), as_of);
        let mut m = JsonMap::new();
        for account in self.l().chart().all() {
            let (dr, cr) = tb
                .lines
                .iter()
                .find(|t| t.code == account.code)
                .map_or((Decimal::ZERO, Decimal::ZERO), |t| (t.debit, t.credit));
            let got = match account.account_type.normal_side() {
                Side::Debit => dr - cr,
                Side::Credit => cr - dr,
            };
            m.insert(account.code.clone(), Value::String(figure(got)));
        }
        Ok(Value::Object(m))
    }

    pub(crate) fn answer_members_equity(&self, q: &Query) -> Result<Value, Refusal> {
        let (from, to) = self.q_period(q)?;
        let st = members_equity(self.l(), from, to, &self.members)
            .map_err(|e| engine_refusal(RuleId::new(Area::Partner, 1), e))?;
        let mut m = JsonMap::new();
        for member in &st.members {
            for (column, v) in [
                ("beginning", member.beginning),
                ("contributions", member.contributions),
                ("net_income", member.net_income),
                ("distributions", member.distributions),
                ("other", member.other),
                ("ending", member.ending),
            ] {
                m.insert(
                    format!("{}.{column}", member.name),
                    Value::String(figure(v)),
                );
            }
        }
        Ok(Value::Object(m))
    }
}

/// The `cash_flow` query's value for `key` (bookspec/README.md), or None
/// for a key it doesn't know. A key's `<N>` is event N, whose entry
/// `entry_of` gives.
pub(crate) fn cash_flow_value(
    cf: &CashFlowStatement,
    key: &str,
    entry_of: &dyn Fn(usize) -> EntryId,
) -> Option<Decimal> {
    let sections = [
        ("operating", CashFlowSection::Operating),
        ("investing", CashFlowSection::Investing),
        ("financing", CashFlowSection::Financing),
    ];
    let count = |n: usize| Decimal::from(n as u64);
    Some(match key {
        "net_change" => cf.net_change_in_cash,
        "cash_beginning" => cf.cash_beginning,
        "cash_ending" => cf.cash_ending,
        "difference" => cf.reconciles.difference,
        "net_income" => cf.operating_reconciliation.net_income,
        "noncash.count" => count(cf.noncash_disclosures.len()),
        _ => {
            if let Some(&(_, s)) = sections.iter().find(|(k, _)| *k == key) {
                return Some(cf.total(s));
            }
            let (head, tail) = key.split_once('.')?;
            if head == "noncash" {
                let n = entry_of(tail.parse().ok()?);
                return Some(
                    cf.noncash_disclosures
                        .iter()
                        .filter(|d| d.entry == n)
                        .map(|d| d.amount)
                        .sum(),
                );
            }
            if head == "class" {
                // `class.<a..g>`: the operating class's total (ASC
                // 230-10-45-25's letters).
                let c = operating_class(tail)?;
                return Some(
                    cf.operating_by_class()
                        .into_iter()
                        .filter(|(k, _)| *k == c)
                        .map(|(_, a)| a)
                        .sum(),
                );
            }
            if head == "reconciliation" {
                let (kind, account) = match tail.split_once('.') {
                    Some((k, a)) => (k, Some(a)),
                    None => (tail, None),
                };
                let kind = match kind {
                    "depreciation" => ReconcilingKind::Depreciation,
                    "investing_gain_loss" => ReconcilingKind::InvestingGainLoss,
                    "noncash_through" => ReconcilingKind::NoncashThrough,
                    "other_noncash" => ReconcilingKind::OtherNoncash,
                    "balance_change" => ReconcilingKind::OperatingBalanceChange,
                    _ => return None,
                };
                return Some(
                    cf.operating_reconciliation
                        .lines
                        .iter()
                        .filter(|l| l.kind == kind && l.account.as_deref() == account)
                        .map(|l| l.amount)
                        .sum(),
                );
            }
            let &(_, s) = sections.iter().find(|(k, _)| *k == head)?;
            let lines = cf.lines(s);
            if tail == "count" {
                let entries: BTreeSet<EntryId> = lines.iter().map(|l| l.entry).collect();
                return Some(count(entries.len()));
            }
            // `<section>.<N>`, or `<section>.<N>.on_behalf`: of event N's
            // entry's cash there, what a member or lender handled for the
            // company.
            let (n, on_behalf) = match tail.split_once('.') {
                Some((n, "on_behalf")) => (n, true),
                Some(_) => return None,
                None => (tail, false),
            };
            let n = entry_of(n.parse().ok()?);
            lines
                .iter()
                .filter(|l| l.entry == n)
                .map(|l| if on_behalf { l.on_behalf } else { l.amount })
                .sum()
        }
    })
}

/// The operating class ASC 230-10-45-25's letter names.
pub(crate) fn operating_class(letter: &str) -> Option<ledger_core::OperatingClass> {
    use ledger_core::OperatingClass::*;
    Some(match letter {
        "a" => FromCustomers,
        "b" => InterestAndDividendsReceived,
        "c" => OtherReceipts,
        "d" => ToSuppliersAndEmployees,
        "e" => InterestPaid,
        "f" => IncomeTaxesPaid,
        "g" => OtherPayments,
        _ => return None,
    })
}

/// The reconciliation key a line answers to.
pub(crate) fn reconciliation_key(l: &ledger_core::ReconcilingLine) -> String {
    let kind = match l.kind {
        ReconcilingKind::Depreciation => "depreciation",
        ReconcilingKind::InvestingGainLoss => "investing_gain_loss",
        ReconcilingKind::NoncashThrough => "noncash_through",
        ReconcilingKind::OtherNoncash => "other_noncash",
        ReconcilingKind::OperatingBalanceChange => "balance_change",
    };
    match &l.account {
        Some(a) => format!("reconciliation.{kind}.{a}"),
        None => format!("reconciliation.{kind}"),
    }
}

/// Check one `cash_flow` expectation, every value it gives.
pub(crate) fn check_cash_flow(b: &Books<'_>, x: &Expect) {
    let (ex, l, members) = (b.ex, b.l(), &b.members);
    let (start, end) = period(ex, x);
    let values = x.figures(&ex.id);
    let params = CashFlowParams {
        start,
        end,
        cash_accounts: codes(l, &ex.setup.cash_accounts, &ex.id),
        financing_accounts: codes(l, &ex.setup.financing_accounts, &ex.id),
        investing_accounts: codes(l, &ex.setup.investing_accounts, &ex.id),
        members: members.to_vec(),
    };
    let cf = cash_flow(l, &params).unwrap_or_else(|e| panic!("{}: {e}", ex.id));
    for (key, want) in values {
        let want = exact(want, &format!("{} {key}", ex.id));
        let entry_of = |n: usize| b.entry_of(n, &format!("{} cash_flow {key}", ex.id));
        let got = cash_flow_value(&cf, key, &entry_of)
            .unwrap_or_else(|| panic!("{}: unknown cash_flow key {key:?}", ex.id));
        assert_eq!(
            got, want,
            "{} [{} .. {}] {key}",
            ex.id, params.start, params.end
        );
    }
    // A count given with entries listed: those are all the entries.
    for section in ["operating", "investing", "financing", "noncash"] {
        let listed: BTreeSet<&str> = values
            .keys()
            .filter_map(|k| k.strip_prefix(section)?.strip_prefix('.'))
            .filter(|t| *t != "count")
            .map(|t| t.split_once('.').map_or(t, |(n, _)| n))
            .collect();
        if let Some(n) = values.get(&format!("{section}.count"))
            && !listed.is_empty()
        {
            assert_eq!(
                Decimal::from(listed.len() as u64),
                exact(n, &ex.id),
                "{}: {section}.count against the entries listed",
                ex.id
            );
        }
    }
    // Class keys given: every class the statement has is one.
    if values.keys().any(|k| k.starts_with("class.")) {
        for (c, amount) in cf.operating_by_class() {
            let letter = ["a", "b", "c", "d", "e", "f", "g"]
                .into_iter()
                .find(|l| operating_class(l) == Some(c))
                .unwrap();
            assert!(
                values.contains_key(&format!("class.{letter}")),
                "{}: the statement has class.{letter} ({amount}), the example doesn't",
                ex.id
            );
        }
    }
    // Reconciliation keys given: every line the statement has is one.
    if values.keys().any(|k| k.starts_with("reconciliation.")) {
        for line in &cf.operating_reconciliation.lines {
            let key = reconciliation_key(line);
            assert!(
                values.contains_key(&key),
                "{}: the statement has {key} ({}), the example doesn't",
                ex.id,
                line.amount
            );
        }
    }
}

/// Check one `trial_balance` expectation: each account's balance as of
/// the day, debits less credits (a credit balance negative), every
/// account listed (rules.md B-6, B-33).
pub(crate) fn check_trial_balance(ex: &Example, l: &Ledger, x: &Expect) {
    let as_of = day(ex, x);
    let tb = trial_balance_as_of(l, as_of);
    assert!(tb.balanced(), "{}: the trial balance balances", ex.id);
    let keys = ex.setup.accounts.iter().map(|a| a.code.clone()).collect();
    exactly(ex, x, &keys);
    for (code, want) in x.figures(&ex.id) {
        let want = exact(want, &format!("{} {code}", ex.id));
        let got: Decimal = tb
            .lines
            .iter()
            .filter(|t| &t.code == code)
            .map(|t| t.debit - t.credit)
            .sum();
        assert_eq!(got, want, "{} trial balance as of {as_of}: {code}", ex.id);
    }
}

/// Check one `profit_and_loss` expectation: each revenue and expense
/// account's activity over the period on its normal side, closing entries
/// left out, and `net_income`, revenue less expenses; every revenue and
/// expense account listed (rules.md B-14, B-33).
pub(crate) fn check_profit_and_loss(ex: &Example, l: &Ledger, x: &Expect) {
    let (start, end) = period(ex, x);
    let pl = profit_and_loss_for_period(l, start, end, None)
        .unwrap_or_else(|e| panic!("{}: {e}", ex.id));
    let mut keys: BTreeSet<String> = ex
        .setup
        .accounts
        .iter()
        .filter(|a| matches!(a.kind.as_str(), "revenue" | "expense"))
        .map(|a| a.code.clone())
        .collect();
    keys.insert("net_income".to_string());
    exactly(ex, x, &keys);
    for (key, want) in x.figures(&ex.id) {
        let want = exact(want, &format!("{} {key}", ex.id));
        let got = if key == "net_income" {
            pl.net_income()
        } else {
            pl.revenue
                .iter()
                .chain(&pl.expenses)
                .filter(|s| &s.code == key)
                .map(|s| s.amount)
                .sum()
        };
        assert_eq!(
            got, want,
            "{} profit and loss [{start} .. {end}]: {key}",
            ex.id
        );
    }
}

/// Check one `entry` expectation: the lines of event N's entry, `"<code>.debit"` and
/// `"<code>.credit"` each summing that account's lines on that side,
/// every line of the entry named.
pub(crate) fn check_entry(b: &Books<'_>, x: &Expect) {
    let (ex, l) = (b.ex, b.l());
    assert!(
        x.from.is_none() && x.to.is_none() && x.as_of.is_none(),
        "{}: entry asks about one entry, no dates",
        ex.id
    );
    let n = x
        .entry
        .unwrap_or_else(|| panic!("{}: an entry query names its event", ex.id));
    let entry = l
        .get_entry(b.entry_of(n, &format!("{} entry {n}", ex.id)))
        .expect("an entry an event posted");
    let mut got: BTreeMap<String, Decimal> = BTreeMap::new();
    for line in entry.lines() {
        let code = &l.chart().get(line.account).unwrap().code;
        let side = match line.side {
            Side::Debit => "debit",
            Side::Credit => "credit",
        };
        *got.entry(format!("{code}.{side}")).or_default() += line.amount.amount;
    }
    let keys = got.keys().cloned().collect();
    exactly(ex, x, &keys);
    for (key, want) in x.figures(&ex.id) {
        let want = exact(want, &format!("{} {key}", ex.id));
        assert_eq!(got[key], want, "{} entry {n}: {key}", ex.id);
    }
}

/// Check one `balances` expectation: each account named, its balance as
/// of the day on its normal side (so a member's capital after a loss is
/// negative); the accounts named are the example's choice.
pub(crate) fn check_balances(ex: &Example, l: &Ledger, x: &Expect) {
    let as_of = day(ex, x);
    let tb = trial_balance_as_of(l, as_of);
    for (code, want) in x.figures(&ex.id) {
        let want = exact(want, &format!("{} {code}", ex.id));
        let account = l
            .chart()
            .find_by_code(code)
            .unwrap_or_else(|| panic!("{}: no account {code}", ex.id));
        let (dr, cr) = tb
            .lines
            .iter()
            .find(|t| &t.code == code)
            .map_or((Decimal::ZERO, Decimal::ZERO), |t| (t.debit, t.credit));
        let got = match account.account_type.normal_side() {
            Side::Debit => dr - cr,
            Side::Credit => cr - dr,
        };
        assert_eq!(got, want, "{} balance as of {as_of}: {code}", ex.id);
    }
}

/// Check one `members_equity` expectation: the statement of changes in
/// members' equity over the period, `"<member>.<column>"` for the columns
/// `beginning`, `contributions`, `net_income`, `distributions`, `other`
/// and `ending`; the figures named are the example's choice (rules.md
/// B-13).
pub(crate) fn check_members_equity(
    ex: &Example,
    l: &Ledger,
    members: &[MemberCapital],
    x: &Expect,
) {
    let (from, to) = period(ex, x);
    assert!(
        !members.is_empty() && members.iter().all(|m| m.share > Decimal::ZERO),
        "{}: members_equity needs the members and their shares",
        ex.id
    );
    let st = members_equity(l, from, to, members).unwrap_or_else(|e| panic!("{}: {e}", ex.id));
    let mut got = BTreeMap::new();
    for m in &st.members {
        for (column, v) in [
            ("beginning", m.beginning),
            ("contributions", m.contributions),
            ("net_income", m.net_income),
            ("distributions", m.distributions),
            ("other", m.other),
            ("ending", m.ending),
        ] {
            got.insert(format!("{}.{column}", m.name), v);
        }
    }
    for (key, want) in x.figures(&ex.id) {
        let want = exact(want, &format!("{} {key}", ex.id));
        let got = got
            .get(key)
            .unwrap_or_else(|| panic!("{}: members_equity has no {key}", ex.id));
        assert_eq!(
            *got, want,
            "{} members' equity [{from} .. {to}]: {key}",
            ex.id
        );
    }
}
