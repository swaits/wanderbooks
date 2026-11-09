use crate::*;

impl Books<'_> {
    pub(crate) fn answer_k1(&self, q: &Query) -> Result<Value, Refusal> {
        let (from, to) = self.q_period(q)?;
        let cash = codes(self.l(), &self.ex.setup.cash_accounts, &self.ex.id);
        let rows = ledger_core::k1::k1_worksheet(self.l(), from, to, &self.members, &cash)
            .map_err(|e| engine_refusal(RuleId::new(Area::Partner, 1), e))?;
        let mut m = JsonMap::new();
        for r in &rows {
            for (column, v) in [
                ("beginning", r.beginning),
                ("contributions_cash", r.contributions_cash),
                ("contributions_property", r.contributions_property),
                ("net_income", r.net_income),
                ("distributions", r.distributions),
                ("other", r.other),
                ("ending", r.ending),
            ] {
                m.insert(format!("{}.{column}", r.name), Value::String(figure(v)));
            }
        }
        Ok(Value::Object(m))
    }

    pub(crate) fn tax_rows(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<TaxCapitalRow>, Refusal> {
        let cash = codes(self.l(), &self.ex.setup.cash_accounts, &self.ex.id);
        ledger_core::tax_capital::tax_basis_capital(self.l(), from, to, &self.members, &cash)
            .map(|r| r.rows)
            .map_err(|e| engine_refusal(RuleId::new(Area::Partner, 4), e))
    }

    pub(crate) fn answer_k1_tax_basis(&self, q: &Query) -> Result<Value, Refusal> {
        let (from, to) = self.q_period(q)?;
        let mut m = JsonMap::new();
        for row in self.tax_rows(from, to)? {
            let name = &row.book.name;
            m.insert(
                format!("{name}.contributions_cash"),
                Value::String(figure(row.contributions_cash)),
            );
            m.insert(
                format!("{name}.contributions_property_note"),
                Value::String(property_note(&row.contributions_property).to_string()),
            );
            for (line, tax) in [
                ("beginning", &row.beginning),
                ("contributions_property", &row.contributions_property),
                ("net_income", &row.net_income),
                ("other", &row.other),
                ("distributions", &row.distributions),
                ("ending", &row.ending),
            ] {
                m.insert(
                    format!("{name}.{line}"),
                    Value::String(tax.tax().map_or("open".to_string(), figure)),
                );
                m.insert(
                    format!("{name}.known_{line}"),
                    Value::String(figure(tax.known)),
                );
            }
        }
        Ok(Value::Object(m))
    }

    pub(crate) fn answer_k1_reconciliation(&self, q: &Query) -> Result<Value, Refusal> {
        let (from, to) = self.q_period(q)?;
        let mut m = JsonMap::new();
        for row in self.tax_rows(from, to)? {
            let r = &row.reconciliation;
            let total = r.total().ok_or_else(|| {
                Refusal::new(
                    RuleId::new(Area::Partner, 4),
                    "the reconciliation's total is too large",
                )
            })?;
            for (cause, v) in RECONCILING_CAUSES.into_iter().zip([
                r.beginning,
                r.built_in_gain_704c,
                r.property_without_basis,
                r.property_distributed,
                total,
            ]) {
                m.insert(
                    format!("{}.{cause}", row.book.name),
                    Value::String(figure(v)),
                );
            }
        }
        Ok(Value::Object(m))
    }
}

/// Check one `k1` expectation: the K-1 worksheet over the period (
/// BS-partner-1; Schedule K-1 Item L), `"<member>.<column>"` for the columns
/// `beginning`, `contributions_cash`, `contributions_property`,
/// `net_income`, `distributions`, `other` and `ending`; the figures named
/// are the example's choice. It needs `cash_accounts` (what is money
/// rather than property).
pub(crate) fn check_k1(ex: &Example, l: &Ledger, members: &[MemberCapital], x: &Expect) {
    let (from, to) = period(ex, x);
    assert!(
        !members.is_empty() && members.iter().all(|m| m.share > Decimal::ZERO),
        "{}: k1 needs the members and their shares",
        ex.id
    );
    assert!(
        !ex.setup.cash_accounts.is_empty(),
        "{}: k1 needs cash_accounts (what is money rather than property)",
        ex.id
    );
    let cash = codes(l, &ex.setup.cash_accounts, &ex.id);
    let rows = ledger_core::k1::k1_worksheet(l, from, to, members, &cash)
        .unwrap_or_else(|e| panic!("{}: {e}", ex.id));
    let mut got = BTreeMap::new();
    for r in &rows {
        for (column, v) in [
            ("beginning", r.beginning),
            ("contributions_cash", r.contributions_cash),
            ("contributions_property", r.contributions_property),
            ("net_income", r.net_income),
            ("distributions", r.distributions),
            ("other", r.other),
            ("ending", r.ending),
        ] {
            got.insert(format!("{}.{column}", r.name), v);
        }
    }
    for (key, want) in x.figures(&ex.id) {
        let want = exact(want, &format!("{} {key}", ex.id));
        let got = got
            .get(key)
            .unwrap_or_else(|| panic!("{}: k1 has no {key}", ex.id));
        assert_eq!(*got, want, "{} K-1 [{from} .. {to}]: {key}", ex.id);
    }
}

/// The tax-basis capital worksheet for `[from, to]` (BS-partner-4), a
/// row a member, as `k1` reads the book one: it needs the members and
/// their shares, and `cash_accounts`.
pub(crate) fn tax_capital_rows(b: &Books<'_>, x: &Expect, what: &str) -> Vec<TaxCapitalRow> {
    let ex = b.ex;
    let (from, to) = period(ex, x);
    assert!(
        !b.members.is_empty() && b.members.iter().all(|m| m.share > Decimal::ZERO),
        "{what}: needs the members and their shares"
    );
    assert!(
        !ex.setup.cash_accounts.is_empty(),
        "{what}: the K-1 worksheet needs cash_accounts"
    );
    let cash = codes(b.l(), &ex.setup.cash_accounts, &ex.id);
    ledger_core::tax_capital::tax_basis_capital(b.l(), from, to, &b.members, &cash)
        .unwrap_or_else(|e| panic!("{what}: {e}"))
        .rows
}

/// Check one `k1_tax_basis` expectation: each member's capital account
/// for `[from, to]` on the tax basis, as Schedule K-1 Item L reports it
/// (BS-partner-4), `"<member>.<line>"` for the lines `beginning`,
/// `contributions_cash`, `contributions_property`, `net_income`, `other`,
/// `distributions` and `ending`, beside `k1`'s book figures; a line the
/// books can't know is `"open"`, with what they know so far as
/// `"<member>.known_<line>"`. The example names the figures it shows.
pub(crate) fn check_k1_tax_basis(b: &Books<'_>, x: &Expect) {
    let ex = b.ex;
    let what = format!("{} k1_tax_basis", ex.id);
    let mut figures = BTreeMap::new();
    for row in tax_capital_rows(b, x, &what) {
        let m = &row.book.name;
        figures.insert(
            format!("{m}.contributions_cash"),
            figure(row.contributions_cash),
        );
        figures.insert(
            format!("{m}.contributions_property_note"),
            property_note(&row.contributions_property).to_string(),
        );
        for (line, tax) in [
            ("beginning", &row.beginning),
            ("contributions_property", &row.contributions_property),
            ("net_income", &row.net_income),
            ("other", &row.other),
            ("distributions", &row.distributions),
            ("ending", &row.ending),
        ] {
            figures.insert(
                format!("{m}.{line}"),
                tax.tax().map_or("open".to_string(), figure),
            );
            figures.insert(format!("{m}.known_{line}"), figure(tax.known));
        }
    }
    named(ex, x, &what, &figures);
}

/// The note a property line carries (BS-partner-17): `prepaid_at_book` when
/// property contributed to a prepaid account with no fixed asset behind it
/// was taken at its book amount for the preparer to confirm, `none` when the
/// line carries no note.
fn property_note(line: &ledger_core::tax_capital::TaxLine) -> &'static str {
    use ledger_core::tax_capital::Note;
    if line
        .notes
        .iter()
        .any(|n| matches!(n, Note::PrepaidAtBook { .. }))
    {
        "prepaid_at_book"
    } else {
        "none"
    }
}

/// The causes `k1_reconciliation` reports, each member's every one.
pub(crate) const RECONCILING_CAUSES: [&str; 5] = [
    "beginning",
    "built_in_gain_704c",
    "property_without_basis",
    "property_distributed",
    "total",
];

/// Check one `k1_reconciliation` expectation: each member's book capital
/// less tax-basis capital at the end of `[from, to]`, by cause (
/// BS-partner-4), `"<member>.<cause>"` for `beginning`, `built_in_gain_704c`
/// (their share of the §704(c) gain or loss at contribution),
/// `property_without_basis`, `property_distributed` and `total`: the
/// example names the members it shows, each with every cause.
pub(crate) fn check_k1_reconciliation(b: &Books<'_>, x: &Expect) {
    let ex = b.ex;
    let what = format!("{} k1_reconciliation", ex.id);
    let mut figures = BTreeMap::new();
    for row in tax_capital_rows(b, x, &what) {
        let r = &row.reconciliation;
        let total = r
            .total()
            .unwrap_or_else(|| panic!("{what}: the total is too large"));
        for (cause, v) in RECONCILING_CAUSES.into_iter().zip([
            r.beginning,
            r.built_in_gain_704c,
            r.property_without_basis,
            r.property_distributed,
            total,
        ]) {
            figures.insert(format!("{}.{cause}", row.book.name), figure(v));
        }
    }
    let named_members: BTreeSet<&str> = x
        .figures(&ex.id)
        .keys()
        .map(|k| k.split_once('.').map_or(k.as_str(), |(m, _)| m))
        .collect();
    for m in named_members {
        for cause in RECONCILING_CAUSES {
            assert!(
                x.figures(&ex.id).contains_key(&format!("{m}.{cause}")),
                "{what}: {m} is shown with every cause: {m}.{cause} is missing"
            );
        }
    }
    named(ex, x, &what, &figures);
}
