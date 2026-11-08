use crate::*;

impl Books<'_> {
    /// Event `n` (counting from 1): accepted, or refused for the reason it
    /// gives and changing nothing; the entries it posted kept as event
    /// n's, tagged with the parties, payee and payment method it names.
    /// Event `n`'s kind; panics on a shape the schema refuses.
    pub(crate) fn event_kind(&self, n: usize, e: &Event) -> &'static str {
        let kinds = [
            ("lines", !e.lines.is_empty()),
            ("reverses", e.reverses.is_some()),
            ("close", e.close.is_some()),
            ("bill", e.bill.is_some()),
            ("invoice", e.invoice.is_some()),
            ("write_off", e.write_off.is_some()),
            ("run_depreciation", e.run_depreciation.is_some()),
            ("dispose_asset", e.dispose_asset.is_some()),
            ("run_schedules", e.run_schedules.is_some()),
            ("register_schedule", e.register_schedule.is_some()),
        ];
        match kinds.iter().filter(|(_, given)| *given).collect::<Vec<_>>()[..] {
            [(kind, _)] => kind,
            _ => panic!(
                "{} event {n}: an event is one of lines, reverses, close, bill, invoice, write_off, run_depreciation, dispose_asset, run_schedules or register_schedule",
                self.ex.id
            ),
        }
    }

    /// The shape the event must have, checked before it is taken: what the
    /// schema says and what the loader's own words say.
    pub(crate) fn check_event_shape(&self, n: usize, e: &Event, kind: &str) {
        let ex = self.ex.id.as_str();
        // An entry of the example's own words: dated and described. A
        // disposal is dated; the other commands date their own entries,
        // and every command describes its own.
        let own = matches!(kind, "lines" | "reverses" | "close" | "bill" | "invoice");
        let dated = own || matches!(kind, "dispose_asset" | "write_off");
        assert_eq!(
            e.date.is_some(),
            dated,
            "{ex} event {n}: {kind} {}",
            if dated {
                "is dated (date)"
            } else {
                "dates its own entries: no date"
            }
        );
        assert_eq!(
            e.description.is_some(),
            own,
            "{ex} event {n}: {kind} {}",
            if own {
                "is described (description)"
            } else {
                "describes its own entries: no description"
            }
        );
        assert!(
            !e.older || kind == "lines",
            "{ex} event {n}: older is an entry's lines, as an earlier version wrote them"
        );
        assert!(
            e.payee.is_none() || kind == "lines",
            "{ex} event {n}: a payee is an entry's: a reversal takes its original's, a bill names its vendor, an invoice its customer, a close and a command have none"
        );
        assert!(
            (e.parties.is_empty() && e.instrument.is_none()) || kind != "reverses",
            "{ex} event {n}: a reversal takes its original's parties and payment method"
        );
        assert!(
            (e.parties.is_empty() && e.instrument.is_none()) || own,
            "{ex} event {n}: {kind} is tagged with no parties or payment method"
        );
        assert!(
            e.refused.is_none() || !matches!(kind, "close" | "bill" | "invoice"),
            "{ex} event {n}: refused is for a reversal, an entry or a command, which the books take whole or not at all"
        );
        assert!(
            !(e.older && e.refused.is_some()),
            "{ex} event {n}: an entry posted as an earlier version wrote it (older) can't also be refused"
        );
        assert!(
            e.write_off.is_none() || kind == "write_off",
            "{ex} event {n}: a write_off is its own event"
        );
    }

    /// Take event `n` as the books take it, keep what it posted and tag it
    /// with the parties, payee and payment method the example names; the
    /// engine's answer, never compared with what the example expects.
    pub(crate) fn apply_event(&mut self, n: usize, e: &Event) -> Result<(), StoreError> {
        let kind = self.event_kind(n, e);
        self.check_event_shape(n, e, kind);
        let what = format!("{} event {n}", self.ex.id);
        let when = e.date.as_deref().map(|d| date(d, &what));
        let description = e.description.as_deref().unwrap_or_default();
        let first = self.l().next_entry_id();
        let result = self.take(n, e, kind, when, description, &what);
        let next = self.l().next_entry_id();
        let posted: Vec<EntryId> = (first..next).collect();
        self.posted.push(posted);
        result?;
        // The assets this event's entry put on the books.
        for a in self
            .ex
            .setup
            .assets
            .iter()
            .filter(|a| a.acquired_by == Some(n))
        {
            let entry = self.entry_of(n, &format!("{what}: asset {}'s acquired_by", a.id));
            let asset = self.asset(&a.id, &what);
            took(self.store.set_asset_acquisition(asset, entry), &what);
        }
        if !e.parties.is_empty() {
            let entry = self.entry_of(n, &what);
            let shares: Vec<PartyShare> = e
                .parties
                .iter()
                .map(|sh| {
                    let party = *self
                        .parties
                        .get(&sh.party)
                        .unwrap_or_else(|| panic!("{what}: party {:?} not in setup", sh.party));
                    let share = match (&sh.amount, &sh.percent) {
                        (None, None) => Share::Derived,
                        (Some(a), None) => Share::Amount(exact(a, &self.ex.id)),
                        (None, Some(p)) => Share::Percent(exact(p, &self.ex.id)),
                        _ => panic!("{what}: a share is an amount or a percent"),
                    };
                    PartyShare { party, share }
                })
                .collect();
            took(self.store.set_entry_parties(entry, &shares), &what);
        }
        // The payee after the parties: a payee's party must be among
        // several named (the vendor tag follows the parties).
        if let Some(p) = &e.payee {
            let v = *self
                .payees
                .get(p)
                .unwrap_or_else(|| panic!("{what}: payee {p:?} not in setup"));
            let entry = self.entry_of(n, &what);
            took(self.store.set_entry_vendor(entry, Some(v)), &what);
        }
        if let Some(label) = &e.instrument {
            let i = *self
                .instruments
                .get(label)
                .unwrap_or_else(|| panic!("{what}: payment method {label:?} not in setup"));
            let entry = self.entry_of(n, &what);
            took(self.store.set_entry_instrument(entry, Some(i)), &what);
        }
        Ok(())
    }

    /// Apply event `n` as the example says and check it: accepted, or
    /// refused for the reason it gives and changing nothing.
    pub(crate) fn apply(&mut self, n: usize, e: &Event) {
        let ex = self.ex.id.as_str();
        let kind = self.event_kind(n, e);
        let what = format!("{ex} event {n}");
        let own = matches!(kind, "lines" | "reverses" | "close" | "bill" | "invoice");
        let head = self.store.head();
        let first = self.l().next_entry_id();
        let result = self.apply_event(n, e);
        match (result, &e.refused) {
            (Ok(()), None) => {}
            (Err(err), Some(want)) => {
                assert_eq!(store_refusal(&err), *want, "{what}: {kind} refused ({err})");
                assert_eq!(
                    self.store.head(),
                    head,
                    "{what}: a refused {kind} changes nothing"
                );
            }
            (Err(err), None) => panic!("{what}: {kind}: {err}"),
            (Ok(()), Some(want)) => panic!("{what}: {kind} accepted, expected refused: {want}"),
        }
        let next = self.l().next_entry_id();
        let posted: Vec<EntryId> = (first..next).collect();
        assert_eq!(
            &posted,
            &self.posted[n - 1],
            "{what}: what the event posted is kept"
        );
        for &id in &posted {
            assert_eq!(
                self.l().get_entry(id).map(|x| x.status()),
                Some(EntryStatus::Posted),
                "{what}: entry {id}, which it made, is posted"
            );
        }
        if (own || kind == "write_off") && e.refused.is_none() {
            assert_eq!(posted.len(), 1, "{what}: {kind} posts one entry");
        }
    }

    /// An invoice's or bill's lines: `amount` (one line, described as the
    /// event) or `lines`, exactly one; each project a `setup.projects`
    /// name.
    pub(crate) fn document_lines(
        &self,
        amount: &Option<String>,
        lines: &[DocumentLineSpec],
        description: &str,
        what: &str,
    ) -> Vec<DocumentLine> {
        let positive = |s: &str| {
            let a = exact(s, what);
            assert!(a > Decimal::ZERO, "{what}: amounts are positive");
            a
        };
        match (amount, lines) {
            (Some(a), []) => vec![DocumentLine {
                description: description.to_string(),
                amount: positive(a),
                class_id: None,
                account: None,
                quantity: None,
                unit_price: None,
            }],
            (None, [_, ..]) => lines
                .iter()
                .map(|l| DocumentLine {
                    description: l.description.clone().unwrap_or_else(|| description.into()),
                    amount: positive(&l.amount),
                    class_id: l.project.as_ref().map(|p| {
                        *self.projects.get(p).unwrap_or_else(|| {
                            panic!("{what}: project {p:?} not in setup.projects")
                        })
                    }),
                    account: None,
                    quantity: None,
                    unit_price: None,
                })
                .collect(),
            _ => panic!("{what}: a document has an amount or lines, exactly one"),
        }
    }

    /// Event `n`'s command, given to the books (`kind`, as [`Self::apply`]
    /// found it): whatever it posts is posted, or the books refuse it.
    pub(crate) fn take(
        &mut self,
        n: usize,
        e: &Event,
        kind: &str,
        when: Option<NaiveDate>,
        description: &str,
        what: &str,
    ) -> Result<(), StoreError> {
        let money = |s: &str| {
            let amount = exact(s, what);
            // Only a refused entry carries an amount the books refuse
            // (the schema checks it).
            assert!(
                amount > Decimal::ZERO || e.refused.is_some(),
                "{what}: amounts are positive"
            );
            amount
        };
        let day = || when.expect("checked: dated");
        match kind {
            "lines" if e.refused.is_some() => {
                // The books take an entry whole or not at all, so a
                // refused one leaves no draft behind: it is tried on a
                // clone, which is how the engine refuses it.
                let mut probe = self.store.ledger().clone();
                let entry = probe.create_entry(day(), description);
                for (i, line) in e.lines.iter().enumerate() {
                    let side = match line.side.as_str() {
                        "debit" => Side::Debit,
                        "credit" => Side::Credit,
                        other => panic!("{what}: side {other:?}"),
                    };
                    let amount = Money::new(money(&line.amount), self.currency);
                    let account = self.id(&line.account);
                    probe.add_line(entry, account, side, amount)?;
                    if let Some(p) = &line.project {
                        let class = *self.projects.get(p).unwrap_or_else(|| {
                            panic!("{what}: project {p:?} not in setup.projects")
                        });
                        probe.set_line_class(entry, i, Some(class))?;
                    }
                }
                Ok(probe.post_entry(entry)?)
            }
            "lines" => {
                let post = |this: &mut Self| -> Result<(), StoreError> {
                    let entry = this.store.create_entry(day(), description)?;
                    for (i, line) in e.lines.iter().enumerate() {
                        let side = match line.side.as_str() {
                            "debit" => Side::Debit,
                            "credit" => Side::Credit,
                            other => panic!("{what}: side {other:?}"),
                        };
                        let amount = Money::new(money(&line.amount), this.currency);
                        let account = this.id(&line.account);
                        this.store.add_line(entry, account, side, amount)?;
                        // Tagged as books.add_line tags it: after the line.
                        if let Some(p) = &line.project {
                            let class = *this.projects.get(p).unwrap_or_else(|| {
                                panic!("{what}: project {p:?} not in setup.projects")
                            });
                            this.store.set_line_class(entry, i, Some(class))?;
                        }
                    }
                    this.store.post_entry(entry)
                };
                if e.older {
                    ledger_core::parties::replaying(|| post(self))
                } else {
                    post(self)
                }
            }
            "reverses" => {
                let of = e.reverses.expect("checked: a reversal");
                // Events count from 1, as entries do: `reverses = 1` is
                // the first event (bookspec/README.md).
                assert!(of >= 1, "{what}: reverses = {of}: events count from 1");
                assert!(of < n, "{what}: reverses a later event");
                let entry = self.entry_of(of, what);
                self.store
                    .reverse_entry(entry, day(), description)
                    .map(|_| ())
            }
            "close" => {
                let c = e.close.as_ref().expect("checked: a close");
                let start = date(&c.start, what);
                let p = self
                    .store
                    .create_period(&format!("event {n}"), start, day())?;
                let members: Vec<MemberClose> = self
                    .members
                    .iter()
                    .map(|m| MemberClose {
                        capital: m.capital_account,
                        share: m.share,
                        distributions: None,
                    })
                    .collect();
                if c.to == "members" {
                    assert!(
                        !members.is_empty() && members.iter().all(|m| m.share > Decimal::ZERO),
                        "{what}: a close to the members needs their shares"
                    );
                }
                let to = match (c.to.as_str(), c.rounding.as_deref()) {
                    ("members", None) => CloseTo::Members(members),
                    ("members", Some("per-close")) => CloseTo::MembersPerClose(members),
                    ("retained_earnings", None) => CloseTo::RetainedEarnings { sweep: vec![] },
                    other => panic!("{what}: close {other:?}"),
                };
                let closed = self.store.close_period_to(p, &to)?;
                assert!(closed.is_some(), "{what}: the close has nothing to close");
                Ok(())
            }
            "bill" => {
                let b = e.bill.as_ref().expect("checked: a bill");
                let lines = self.document_lines(&b.amount, &b.lines, description, what);
                let vendor = match self.vendors.get(&b.vendor) {
                    Some(&v) => v,
                    None => {
                        let v = self.store.add_vendor(&b.vendor)?;
                        self.vendors.insert(b.vendor.clone(), v);
                        v
                    }
                };
                self.store
                    .create_bill(vendor, day(), day(), lines)
                    .map(|_| ())
            }
            "invoice" => {
                let i = e.invoice.as_ref().expect("checked: an invoice");
                let lines = self.document_lines(&i.amount, &i.lines, description, what);
                let customer = match self.customers.get(&i.customer) {
                    Some(&c) => c,
                    None => {
                        let c = self.store.add_customer(&i.customer)?;
                        self.customers.insert(i.customer.clone(), c);
                        c
                    }
                };
                self.store
                    .create_invoice(customer, day(), day(), lines)
                    .map(|_| ())
            }
            "write_off" => {
                let w = e.write_off.as_ref().expect("checked: a write-off");
                let invoice_entry = self.entry_of(w.invoice, what);
                let invoice = self
                    .store
                    .ledger()
                    .invoices()
                    .into_iter()
                    .find(|i| i.entry_id == invoice_entry)
                    .map(|i| i.id)
                    .unwrap_or_else(|| panic!("{what}: event {} posted no invoice", w.invoice));
                let amount = match &w.amount {
                    Some(a) => exact(a, what),
                    None => {
                        self.store
                            .ledger()
                            .invoice_state(invoice, day())
                            .expect("the invoice's figures")
                            .expect("dated by its own day")
                            .outstanding
                    }
                };
                // Each write-off's own notice: identical bytes would be one
                // record (plan P6 step 4), and a receipt evidences one entry
                // (B-38), so a second write-off with the first's letter is
                // refused before anything is journaled.
                let letter = format!(
                    "%PDF-1.4\nFinal demand for invoice {invoice}: {}\n%%EOF\n",
                    w.reason
                );
                self.store
                    .write_off_invoice(
                        invoice,
                        day(),
                        amount,
                        &w.reason,
                        Some(ledger_store::WriteOffEvidence::File {
                            file_name: "collection-letter.pdf",
                            mime_type: "application/pdf",
                            bytes: letter.as_bytes(),
                            description: Some("Evidence of worthlessness"),
                        }),
                    )
                    .map(drop)
            }
            "run_depreciation" => {
                let r = e.run_depreciation.as_ref().expect("checked: a run");
                let asset = self.asset(&r.asset, what);
                let run = self.store.run_depreciation(asset, &r.period)?;
                self.runs.insert(n, run);
                Ok(())
            }
            "dispose_asset" => {
                let d = e.dispose_asset.as_ref().expect("checked: a disposal");
                let asset = self.asset(&d.asset, what);
                let proceeds = exact(&d.proceeds, what);
                assert!(
                    proceeds >= Decimal::ZERO,
                    "{what}: proceeds aren't negative"
                );
                let proceeds = Money::new(proceeds, self.currency);
                let (into, gain_loss) =
                    (self.id(&d.proceeds_account), self.id(&d.gain_loss_account));
                // What the disposal takes, as the engine computes it before
                // posting; the disposal itself refused whenever its
                // computation is.
                let preview = self.store.dispose_preview(asset, day(), proceeds);
                let disposed = self
                    .store
                    .dispose_asset(asset, day(), proceeds, into, gain_loss);
                match (preview, disposed) {
                    (Ok(p), Ok(_)) => {
                        assert!(
                            self.disposals.insert(d.asset.clone(), p).is_none(),
                            "{what}: asset {} disposed of twice",
                            d.asset
                        );
                        Ok(())
                    }
                    (Err(p), Ok(_)) => {
                        panic!("{what}: disposed of, though its computation was refused: {p}")
                    }
                    (_, Err(err)) => Err(err),
                }
            }
            "run_schedules" => {
                let r = e.run_schedules.as_ref().expect("checked: a run");
                let through = date(&r.through, what);
                self.store.run_schedules(through).map(|_| ())
            }
            "register_schedule" => {
                let sc = e.register_schedule.as_ref().expect("checked: a schedule");
                let new = new_schedule(self.l(), sc, self.currency, what);
                let schedule = self.store.register_schedule(new)?;
                assert!(
                    self.schedules.insert(sc.id.clone(), schedule).is_none(),
                    "{what}: schedule id {} used twice",
                    sc.id
                );
                Ok(())
            }
            other => unreachable!("{other} is no event"),
        }
    }
}
impl Books<'_> {
    /// Make a change that posts no entry (after event `after`).
    /// Make a change that posts no entry (after event `after`), or say why
    /// the books refuse it.
    pub(crate) fn try_change(&mut self, c: &ChangeSpec) -> Result<(), Refusal> {
        let ex = &self.ex.id.clone();
        let after = c.after;
        let what = format!("{ex} change after {after}");
        match (&c.rename, &c.require_tax_category, &c.correct_asset) {
            (Some(r), None, None) => {
                let account = self.id(&r.code);
                self.store
                    .rename_account(account, &r.name, None)
                    .map(|_| ())
                    .map_err(|e| store_refusal_obj(&e))
            }
            (None, None, Some(f)) => {
                let id = self.asset(&f.asset, &what);
                let a = self.l().get_asset(id).expect("registered").clone();
                let amount = |s: &Option<String>, was: Money| {
                    s.as_deref()
                        .map_or(was, |v| Money::new(exact(v, &what), self.currency))
                };
                let fix = ledger_core::AssetCorrection {
                    cost: amount(&f.cost, a.cost),
                    salvage_value: amount(&f.salvage, a.salvage_value),
                    useful_life_months: f.life_months.unwrap_or(a.useful_life_months),
                    in_service_date: f
                        .in_service
                        .as_deref()
                        .map_or(a.in_service_date, |d| date(d, &what)),
                };
                self.store
                    .correct_asset(id, fix, &f.reason)
                    .map_err(|e| store_refusal_obj(&e))
            }
            (None, Some(policy), None) => {
                assert!(
                    matches!(policy.as_str(), "warn" | "refuse"),
                    "{what}: require_tax_category {policy:?}"
                );
                use ledger_core::settings::Change;
                let change = Change {
                    key: "policy.require_tax_category".into(),
                    value: Some(serde_json::json!(policy)),
                };
                self.store
                    .change_settings(&[change], &bookspec_source(ex))
                    .map(|_| ())
                    .map_err(|e| store_refusal_obj(&e))
            }
            _ => panic!("{what}: a change is one of rename, require_tax_category or correct_asset"),
        }
    }

    /// Make a change that posts no entry, checked: the books must take it.
    pub(crate) fn change(&mut self, c: &ChangeSpec) {
        let after = c.after;
        let what = format!("{} change after {after}", self.ex.id);
        self.try_change(c)
            .unwrap_or_else(|r| panic!("{what}: {}", r.reason));
    }
}
