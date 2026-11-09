use crate::*;

impl Books<'_> {
    pub(crate) fn answer_vendor_1099_paid(&self, q: &Query) -> Result<Value, Refusal> {
        let (from, to) = self.q_period(q)?;
        let year = chrono::Datelike::year(&to);
        let l = self.l();
        let sources = self.payment_accounts()?;
        let r = vendor_1099_report(l, year, None, &sources, &self.party_members, Some(to))
            .map_err(|e| match e {
                // A year whose threshold is not published is BS-1099-2's
                // refusal, never a guess (the engine picks
                // refuse-an-unknown-year).
                LedgerError::RuleNotKnown { .. } => Refusal::of_reason("unknown_1099_year"),
                other => engine_refusal(RuleId::new(Area::Tax1099, 1), other),
            })?;
        let _ = from;
        let mut m = JsonMap::new();
        for (name, vendor) in &self.payees {
            let paid: Decimal = r
                .vendors
                .iter()
                .filter(|v| v.vendor_id == *vendor)
                .map(|v| v.paid)
                .sum();
            m.insert(name.clone(), Value::String(figure(paid)));
        }
        Ok(Value::Object(m))
    }

    pub(crate) fn answer_vendor_1099(&self, q: &Query) -> Result<Value, Refusal> {
        let (_, to) = self.q_period(q)?;
        let year = chrono::Datelike::year(&to);
        let name = q.payee.as_deref().ok_or_else(|| self.missing("payee"))?;
        let v = *self.payees.get(name).ok_or_else(|| {
            Refusal::new(
                RuleId::new(Area::Tax1099, 1),
                format!("payee {name:?} not in setup"),
            )
        })?;
        let sources = self.payment_accounts()?;
        let r = vendor_1099_report(
            self.l(),
            year,
            None,
            &sources,
            &self.party_members,
            Some(to),
        )
        .map_err(|e| match e {
            LedgerError::RuleNotKnown { .. } => Refusal::of_reason("unknown_1099_year"),
            other => engine_refusal(RuleId::new(Area::Tax1099, 1), other),
        })?;
        let line = r.vendors.iter().find(|l| l.vendor_id == v).ok_or_else(|| {
            Refusal::new(
                RuleId::new(Area::Tax1099, 1),
                format!("the 1099 worksheet has no line for {name}"),
            )
        })?;
        Ok(json_figures([
            ("status".to_string(), line.status.key().to_string()),
            ("threshold".to_string(), figure(r.threshold)),
            ("paid".to_string(), figure(line.paid)),
            ("excluded_card".to_string(), figure(line.excluded_card)),
            ("undetermined".to_string(), figure(line.undetermined)),
            ("not_reportable".to_string(), figure(line.not_reportable)),
        ]))
    }

    pub(crate) fn answer_party_part(&self, q: &Query) -> Result<Value, Refusal> {
        let event = q.entry.ok_or_else(|| self.missing("entry"))?;
        let n = self.entry_or_refuse(event)?;
        let name = q.party.as_deref().ok_or_else(|| self.missing("party"))?;
        let party = *self.parties.get(name).ok_or_else(|| {
            Refusal::new(
                RuleId::new(Area::Partner, 3),
                format!("party {name:?} not in setup"),
            )
        })?;
        let ex = &self.ex;
        let r = ledger_core::shares::resolve(
            self.l(),
            n,
            self.l().entry_parties(n),
            &party_money(ex, self.l()),
            &self.party_members,
        )
        .map_err(|why| {
            Refusal::new(
                RuleId::new(Area::Partner, 3),
                format!("entry {n}'s shares: {why}"),
            )
        })?;
        let part = r.part(party).ok_or_else(|| {
            Refusal::new(
                RuleId::new(Area::Partner, 3),
                format!("entry {n} doesn't name {name:?}"),
            )
        })?;
        Ok(json_figures([
            ("resolved".to_string(), figure(part.amount)),
            (
                "direction".to_string(),
                r.target.direction.key().to_string(),
            ),
            ("cash".to_string(), figure(part.cash)),
            ("in_kind".to_string(), figure(part.in_kind)),
        ]))
    }

    pub(crate) fn answer_party_totals(&self, q: &Query) -> Result<Value, Refusal> {
        let (from, to) = self.q_period(q)?;
        let basis = q
            .basis
            .as_deref()
            .ok_or_else(|| {
                Refusal::new(
                    RuleId::new(Area::Partner, 3),
                    "party_totals names its basis",
                )
            })
            .and_then(|k| {
                Basis::parse(k).filter(|b| b.key() == k).ok_or_else(|| {
                    Refusal::new(RuleId::new(Area::Partner, 3), format!("basis {k:?}"))
                })
            })?;
        let name = q.party.as_deref().ok_or_else(|| self.missing("party"))?;
        let party = *self.parties.get(name).ok_or_else(|| {
            Refusal::new(
                RuleId::new(Area::Partner, 3),
                format!("party {name:?} not in setup"),
            )
        })?;
        let ex = &self.ex;
        let a = ledger_core::parties::activity_with(
            self.l(),
            party,
            (from, to),
            &party_money(ex, self.l()),
            &self.party_members,
        );
        if !a.unresolved.is_empty() {
            return Err(Refusal::new(
                RuleId::new(Area::Partner, 3),
                format!("{name:?}'s entries don't resolve"),
            ));
        }
        let (paid, received) = a.totals(basis);
        Ok(json_figures([
            ("paid".to_string(), figure(paid)),
            ("received".to_string(), figure(received)),
        ]))
    }

    pub(crate) fn answer_party_tin_last4(&self, q: &Query) -> Result<Value, Refusal> {
        let name = q.party.as_deref().ok_or_else(|| self.missing("party"))?;
        let party = *self.parties.get(name).ok_or_else(|| {
            Refusal::new(
                RuleId::new(Area::Partner, 2),
                format!("party {name:?} not in setup"),
            )
        })?;
        let got = self
            .l()
            .get_party(party)
            .and_then(|p| p.tin_last4())
            .unwrap_or_default();
        Ok(json_figures([("tin_last4".to_string(), got.to_string())]))
    }

    /// The full TIN a return produced for `party` carries (BS-partner-8,
    /// outside-the-books): read from the vault outside the books, never
    /// from the books' records, which hold only the last four. A party
    /// whose number the vault does not hold is refused `tin_not_held`.
    pub(crate) fn return_tin(&self, party: PartyId) -> Result<String, Refusal> {
        let vault = self
            .vault
            .as_ref()
            .ok_or_else(|| Refusal::of_reason("tin_not_held"))?;
        let tin = vault
            .tin(party)
            .map_err(|_| Refusal::of_reason("tin_not_held"))?
            .ok_or_else(|| Refusal::of_reason("tin_not_held"))?;
        Ok(tin.formatted())
    }

    pub(crate) fn answer_return_tin(&self, q: &Query) -> Result<Value, Refusal> {
        let name = q.party.as_deref().ok_or_else(|| self.missing("party"))?;
        let party = *self.parties.get(name).ok_or_else(|| {
            Refusal::new(
                RuleId::new(Area::Partner, 8),
                format!("party {name:?} not in setup"),
            )
        })?;
        let tin = self.return_tin(party)?;
        Ok(json_figures([("tin".to_string(), tin)]))
    }

    pub(crate) fn answer_project(&self, q: &Query) -> Result<Value, Refusal> {
        let name = q
            .project
            .as_deref()
            .ok_or_else(|| self.missing("project"))?;
        let id = *self.projects.get(name).ok_or_else(|| {
            Refusal::new(
                RuleId::new(Area::Statement, 5),
                format!("project {name:?} not in setup.projects"),
            )
        })?;
        let (from, to) = self.q_period(q)?;
        let f = ledger_core::project_figures(self.l(), id, from, to)
            .map_err(|e| engine_refusal(RuleId::new(Area::Statement, 5), e))?;
        let opt = |d: Option<Decimal>| d.map_or("none".to_string(), figure);
        Ok(json_figures([
            ("revenue".to_string(), figure(f.period.total_revenue)),
            ("expenses".to_string(), figure(f.period.total_expenses)),
            ("net".to_string(), figure(f.period.net_income())),
            ("to_date.revenue".to_string(), figure(f.to_date.revenue)),
            ("to_date.expenses".to_string(), figure(f.to_date.expenses)),
            ("to_date.net".to_string(), figure(f.to_date.net())),
            (
                "unrecovered_cost".to_string(),
                opt(f.to_date.unrecovered_cost),
            ),
        ]))
    }
}

/// The 1099 report a `vendor_1099_paid` or `vendor_1099` expectation
/// asks for: the calendar year of `to`, from January 1 through `to`, out
/// of the cash and financing accounts, an entry's shares resolved with
/// the members who are parties (BS-1099-1, BS-statement-4).
pub(crate) fn report_1099(
    b: &Books<'_>,
    x: &Expect,
) -> (NaiveDate, NaiveDate, ledger_core::Vendor1099Report) {
    let (ex, l) = (b.ex, b.l());
    // A calendar-year count, never the fiscal default (BS-period-3):
    // both dates given.
    assert!(
        x.from.is_some() && x.to.is_some(),
        "{}: {} gives from and to",
        ex.id,
        x.query
    );
    let (from, to) = period(ex, x);
    let year = chrono::Datelike::year(&to);
    assert_eq!(
        Some(from),
        NaiveDate::from_ymd_opt(year, 1, 1),
        "{}: a 1099 counts a calendar year: from is January 1 of to's year",
        ex.id
    );
    assert!(
        !ex.setup.cash_accounts.is_empty(),
        "{}: {} needs cash_accounts (what payees are paid from)",
        ex.id,
        x.query
    );
    let sources = PaymentAccounts {
        cash: codes(l, &ex.setup.cash_accounts, &ex.id),
        financing: codes(l, &ex.setup.financing_accounts, &ex.id),
    };
    let r = vendor_1099_report(l, year, None, &sources, &b.party_members, Some(to))
        .unwrap_or_else(|e| panic!("{}: {e}", ex.id));
    (from, to, r)
}

/// Check one `vendor_1099_paid` expectation: each payee's payments that
/// count toward its 1099 ([`report_1099`]), a payment card's left out,
/// every payee listed (BS-1099-1, BS-statement-4).
pub(crate) fn check_vendor_1099_paid(b: &Books<'_>, x: &Expect) {
    let ex = b.ex;
    let (from, to, r) = report_1099(b, x);
    exactly(ex, x, &b.payees.keys().cloned().collect());
    for (name, want) in x.figures(&ex.id) {
        let want = exact(want, &format!("{} {name}", ex.id));
        let got: Decimal = r
            .vendors
            .iter()
            .filter(|v| v.vendor_id == b.payees[name])
            .map(|v| v.paid)
            .sum();
        assert_eq!(got, want, "{} 1099 paid [{from} .. {to}]: {name}", ex.id);
    }
}

/// Check one `vendor_1099` expectation: one payee's line of the 1099
/// worksheet ([`report_1099`]): `status` (`reportable`,
/// `under_threshold`, `not_reportable`, `undetermined`), the `threshold`
/// applied, and what was `paid` (counts toward its 1099),
/// `excluded_card` (a card or payment network's, Form 1099-K),
/// `undetermined` (its payment method not recorded) and `not_reportable`
/// (paid while decided not reportable); every one given (BS-1099-1).
pub(crate) fn check_vendor_1099(b: &Books<'_>, x: &Expect) {
    let ex = b.ex;
    let name = x
        .payee
        .as_deref()
        .unwrap_or_else(|| panic!("{}: vendor_1099 names its payee", ex.id));
    let v = *b
        .payees
        .get(name)
        .unwrap_or_else(|| panic!("{}: payee {name:?} not in setup", ex.id));
    let (from, to, r) = report_1099(b, x);
    let line = r
        .vendors
        .iter()
        .find(|l| l.vendor_id == v)
        .unwrap_or_else(|| panic!("{}: the 1099 worksheet has no line for {name}", ex.id));
    let keys = [
        "status",
        "threshold",
        "paid",
        "excluded_card",
        "undetermined",
        "not_reportable",
    ];
    exactly(ex, x, &keys.iter().map(|k| k.to_string()).collect());
    let what = |k: &str| format!("{} 1099 [{from} .. {to}] {name}: {k}", ex.id);
    for (key, want) in x.figures(&ex.id) {
        if key == "status" {
            assert_eq!(line.status.key(), want, "{}", what(key));
            continue;
        }
        let got = match key.as_str() {
            "threshold" => r.threshold,
            "paid" => line.paid,
            "excluded_card" => line.excluded_card,
            "undetermined" => line.undetermined,
            _ => line.not_reportable,
        };
        assert_eq!(got, exact(want, &what(key)), "{}", what(key));
    }
}

/// The money party shares resolve against: the cash accounts (every
/// asset account but receivables when none are named), the cards and the
/// financing accounts (BS-partner-3 statement 7).
pub(crate) fn party_money(ex: &Example, l: &Ledger) -> MoneyAccounts {
    ledger_core::parties::money_accounts(
        l,
        &codes(l, &ex.setup.cash_accounts, &ex.id),
        &codes(l, &ex.setup.financing_accounts, &ex.id),
    )
}

/// The party an expectation names.
pub(crate) fn expect_party(b: &Books<'_>, x: &Expect) -> PartyId {
    let name = x
        .party
        .as_deref()
        .unwrap_or_else(|| panic!("{}: {} names its party", b.ex.id, x.query));
    *b.parties
        .get(name)
        .unwrap_or_else(|| panic!("{}: party {name:?} not in setup.parties", b.ex.id))
}

/// Check one `party_part` expectation: entry N's part for the party
/// (BS-partner-3): `resolved`, signed as a line on the party's own
/// account (debit-positive: paid or given positive, received negative),
/// the entry's `direction` (`paid`, `received` or `neither`), and what
/// of it moved as money (`cash`) and as property (`in_kind`); every one
/// given.
pub(crate) fn check_party_part(b: &Books<'_>, x: &Expect) {
    let (ex, l) = (b.ex, b.l());
    assert!(
        x.from.is_none() && x.to.is_none() && x.as_of.is_none(),
        "{}: party_part asks about one entry, no dates",
        ex.id
    );
    let event = x
        .entry
        .unwrap_or_else(|| panic!("{}: a party_part query names its event", ex.id));
    let n = b.entry_of(event, &format!("{} party_part", ex.id));
    let party = expect_party(b, x);
    let r = ledger_core::shares::resolve(
        l,
        n,
        l.entry_parties(n),
        &party_money(ex, l),
        &b.party_members,
    )
    .unwrap_or_else(|why| panic!("{}: entry {n}'s shares: {why}", ex.id));
    let part = r
        .part(party)
        .unwrap_or_else(|| panic!("{}: entry {n} doesn't name {:?}", ex.id, x.party));
    let keys = ["resolved", "direction", "cash", "in_kind"];
    exactly(ex, x, &keys.iter().map(|k| k.to_string()).collect());
    let what = |k: &str| format!("{} entry {n} {:?}: {k}", ex.id, x.party);
    for (key, want) in x.figures(&ex.id) {
        if key == "direction" {
            assert_eq!(r.target.direction.key(), want, "{}", what(key));
            continue;
        }
        let got = match key.as_str() {
            "resolved" => part.amount,
            "cash" => part.cash,
            _ => part.in_kind,
        };
        assert_eq!(got, exact(want, &what(key)), "{}", what(key));
    }
}

/// Check one `party_totals` expectation: what was `paid` to and
/// `received` from the party from `from` through `to` on `basis` (`cash`:
/// money; `all`: money and property in kind), each entry's part on its
/// root original's side (BS-partner-3 statement 5); both given.
pub(crate) fn check_party_totals(b: &Books<'_>, x: &Expect) {
    let (ex, l) = (b.ex, b.l());
    assert!(
        x.from.is_some() && x.to.is_some(),
        "{}: party_totals gives from and to",
        ex.id
    );
    let (from, to) = period(ex, x);
    let basis = x
        .basis
        .as_deref()
        .map(|k| {
            Basis::parse(k)
                .filter(|b| b.key() == k)
                .unwrap_or_else(|| panic!("{}: basis {k:?}", ex.id))
        })
        .unwrap_or_else(|| panic!("{}: party_totals names its basis", ex.id));
    let party = expect_party(b, x);
    let a = ledger_core::parties::activity_with(
        l,
        party,
        (from, to),
        &party_money(ex, l),
        &b.party_members,
    );
    assert!(
        a.unresolved.is_empty(),
        "{}: {:?}'s entries {:?} don't resolve",
        ex.id,
        x.party,
        a.unresolved
    );
    let (paid, received) = a.totals(basis);
    exactly(ex, x, &["paid".to_string(), "received".to_string()].into());
    let f = x.figures(&ex.id);
    let what = |k: &str| {
        format!(
            "{} {:?} [{from} .. {to}] {}: {k}",
            ex.id,
            x.party,
            basis.key()
        )
    };
    assert_eq!(paid, exact(&f["paid"], &what("paid")), "{}", what("paid"));
    assert_eq!(
        received,
        exact(&f["received"], &what("received")),
        "{}",
        what("received")
    );
}

/// Check one `project` expectation (BS-statement-5): the project's
/// `revenue`, `expenses` and `net` for the period (BS-period-3 resolving omitted
/// dates), the same `to_date` (every tagged line through `to`), and its
/// `unrecovered_cost` (`"none"` unless it is on spec); every one given.
/// Through ledger_core::project_figures, as the tools and the viewer.
pub(crate) fn check_project(b: &Books<'_>, x: &Expect) {
    let (ex, l) = (b.ex, b.l());
    let name = x
        .project
        .as_ref()
        .unwrap_or_else(|| panic!("{}: a project query names its project", ex.id));
    let id = *b
        .projects
        .get(name)
        .unwrap_or_else(|| panic!("{}: project {name:?} not in setup.projects", ex.id));
    let (from, to) = period(ex, x);
    let f =
        ledger_core::project_figures(l, id, from, to).unwrap_or_else(|e| panic!("{}: {e}", ex.id));
    let keys: BTreeSet<String> = [
        "revenue",
        "expenses",
        "net",
        "to_date.revenue",
        "to_date.expenses",
        "to_date.net",
        "unrecovered_cost",
    ]
    .iter()
    .map(|k| k.to_string())
    .collect();
    exactly(ex, x, &keys);
    let what = |k: &str| format!("{} project {name:?} [{from} .. {to}]: {k}", ex.id);
    for (key, want) in x.figures(&ex.id) {
        let got = match key.as_str() {
            "revenue" => Some(f.period.total_revenue),
            "expenses" => Some(f.period.total_expenses),
            "net" => Some(f.period.net_income()),
            "to_date.revenue" => Some(f.to_date.revenue),
            "to_date.expenses" => Some(f.to_date.expenses),
            "to_date.net" => Some(f.to_date.net()),
            _ => f.to_date.unrecovered_cost,
        };
        match (want.as_str(), got) {
            ("none", None) => {}
            ("none", Some(g)) => panic!("{}: {g}, expected none", what(key)),
            (w, Some(g)) => assert_eq!(g, exact(w, &what(key)), "{}", what(key)),
            (w, None) => panic!("{}: none, expected {w}", what(key)),
        }
    }
}

/// Check one `party_tin_last4` expectation: the last four of the party's
/// TIN the books keep (its TIN facts', else its W-9's; `""` for none:
/// BS-partner-2).
/// Check one `return_tin` expectation: the full TIN a return produced for
/// the party carries, taken from the vault outside the books
/// (BS-partner-8's outside-the-books option).
pub(crate) fn check_return_tin(b: &Books<'_>, x: &Expect) {
    let ex = b.ex;
    assert!(
        x.from.is_none() && x.to.is_none() && x.as_of.is_none(),
        "{}: return_tin takes no dates",
        ex.id
    );
    let party = expect_party(b, x);
    exactly(ex, x, &["tin".to_string()].into());
    let got = b
        .return_tin(party)
        .unwrap_or_else(|r| panic!("{}: return_tin refused ({})", ex.id, r.reason));
    assert_eq!(
        got,
        x.figures(&ex.id)["tin"],
        "{}: {:?}'s return TIN",
        ex.id,
        x.party
    );
}

pub(crate) fn check_party_tin_last4(b: &Books<'_>, x: &Expect) {
    let ex = b.ex;
    assert!(
        x.from.is_none() && x.to.is_none() && x.as_of.is_none(),
        "{}: party_tin_last4 takes no dates",
        ex.id
    );
    let party = expect_party(b, x);
    exactly(ex, x, &["tin_last4".to_string()].into());
    let got = b
        .l()
        .get_party(party)
        .and_then(|p| p.tin_last4())
        .unwrap_or_default();
    assert_eq!(
        got,
        x.figures(&ex.id)["tin_last4"],
        "{}: {:?}'s TIN last four",
        ex.id,
        x.party
    );
}
