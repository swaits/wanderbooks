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
            ("payment", e.payment.is_some()),
            ("settlement", e.settlement.is_some()),
            ("recovery", e.recovery.is_some()),
            ("run_depreciation", e.run_depreciation.is_some()),
            ("dispose_asset", e.dispose_asset.is_some()),
            ("run_schedules", e.run_schedules.is_some()),
            ("register_schedule", e.register_schedule.is_some()),
            ("post_draft", e.post_draft.is_some()),
            ("set_draft_state", e.set_draft_state.is_some()),
            ("attach_receipt", e.attach_receipt.is_some()),
            ("discard_draft", e.discard_draft.is_some()),
            ("assign_role", e.assign_role.is_some()),
            ("set_setting", e.set_setting.is_some()),
            ("payout", e.payout.is_some()),
            ("tag_asset", e.tag_asset.is_some()),
        ];
        match kinds.iter().filter(|(_, given)| *given).collect::<Vec<_>>()[..] {
            [(kind, _)] => kind,
            _ => panic!(
                "{} event {n}: an event is one of lines, reverses, close, bill, invoice, write_off, payment, settlement, recovery, run_depreciation, dispose_asset, run_schedules, register_schedule, post_draft, set_draft_state, attach_receipt, discard_draft, assign_role, set_setting, payout or tag_asset",
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
        let own = matches!(
            kind,
            "lines" | "reverses" | "close" | "bill" | "invoice" | "payout"
        );
        let dated = own
            || matches!(
                kind,
                "dispose_asset" | "write_off" | "payment" | "settlement" | "recovery"
            );
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
            e.refused.is_none() || kind != "invoice",
            "{ex} event {n}: refused is for a reversal, an entry, a bill, a command or a close, which the books take whole or not at all"
        );
        assert!(
            !e.draft || matches!(kind, "lines" | "reverses"),
            "{ex} event {n}: draft is an entry's lines or a reversal, prepared and not yet posted"
        );
        assert!(
            !e.draft
                || (e.parties.is_empty()
                    && e.instrument.is_none()
                    && e.payee.is_none()
                    && !e.older),
            "{ex} event {n}: a draft carries no payee, parties or payment method"
        );
        assert!(
            !(e.older && e.refused.is_some()),
            "{ex} event {n}: an entry posted as an earlier version wrote it (older) can't also be refused"
        );
        assert!(
            e.write_off.is_none() || kind == "write_off",
            "{ex} event {n}: a write_off is its own event"
        );
        assert!(
            e.payment.is_none() || kind == "payment",
            "{ex} event {n}: a payment is its own event"
        );
        assert!(
            e.settlement.is_none() || kind == "settlement",
            "{ex} event {n}: a settlement is its own event"
        );
        assert!(
            e.recovery.is_none() || kind == "recovery",
            "{ex} event {n}: a recovery is its own event"
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
        let own = matches!(
            kind,
            "lines" | "reverses" | "close" | "bill" | "invoice" | "payout"
        );
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
        // What a draft event made stands as a draft (until a `post_draft`
        // posts it); what any other event made is posted.
        let made = if e.draft {
            EntryStatus::Draft
        } else {
            EntryStatus::Posted
        };
        for &id in &posted {
            assert_eq!(
                self.l().get_entry(id).map(|x| x.status()),
                Some(made),
                "{what}: entry {id}, which it made, is {}",
                if e.draft { "a draft" } else { "posted" }
            );
        }
        if (own || matches!(kind, "write_off" | "payment" | "settlement")) && e.refused.is_none() {
            assert_eq!(posted.len(), 1, "{what}: {kind} posts one entry");
        }
        // A recovery posts one entry or two, as the engine's pick under
        // BS-arap-11 says: a plain entry, or the write-off's reversal and
        // the payment.
        if kind == "recovery" && e.refused.is_none() {
            assert!(
                matches!(posted.len(), 1 | 2),
                "{what}: a recovery posts one entry or two"
            );
        }
    }

    /// The terms a document's example gives, and the due date the books take
    /// them to (BS-arap-3): the engine's own `Terms::due_date` for every
    /// kind, a date the parties agreed ("Due 2026-11-15") included. The
    /// example's spelling is the canonical one, so the engine must print it
    /// back as written. Nothing of the date is read out of the text here:
    /// the example can only pass if the engine computes it. Without terms a
    /// document is due on its own date.
    pub(crate) fn terms_due(
        &self,
        terms: Option<&str>,
        date: NaiveDate,
        what: &str,
    ) -> (NaiveDate, Option<Terms>) {
        let Some(text) = terms else {
            return (date, None);
        };
        let parsed = Terms::parse(text).unwrap_or_else(|e| panic!("{what}: terms {text:?}: {e}"));
        assert_eq!(
            parsed.describe(),
            text,
            "{what}: the engine prints terms {text:?} back as written"
        );
        let due = parsed
            .due_date(date)
            .unwrap_or_else(|e| panic!("{what}: terms {text:?} on {date}: {e}"));
        (due, Some(parsed))
    }

    /// The invoice or bill event `event` posted, and its customer or vendor.
    pub(crate) fn document_of(&self, event: usize, what: &str) -> (ledger_core::DocRef, u64) {
        let entry = self.entry_of(event, what);
        let l = self.l();
        match (
            l.invoices().into_iter().find(|i| i.entry_id == entry),
            l.bills().into_iter().find(|b| b.entry_id == entry),
        ) {
            (Some(i), _) => (ledger_core::DocRef::Invoice(i.id), i.customer_id),
            (None, Some(b)) => (ledger_core::DocRef::Bill(b.id), b.vendor_id),
            (None, None) => panic!("{what}: event {event} posted no invoice or bill"),
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
                    account: l.account.as_deref().map(|code| self.id(code)),
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
            // A draft (BS-entry-5, BS-entry-6): the entry prepared, and posted
            // by nothing here. The engine refuses a line on a header account
            // as it is added (it picks check-when-line-added), and the books
            // take an entry whole or not at all, so the draft is tried on a
            // clone first: a refusal leaves no half-made draft in the journal.
            "lines" if e.draft => {
                let lines: Vec<_> = e
                    .lines
                    .iter()
                    .map(|line| {
                        let side = match line.side.as_str() {
                            "debit" => Side::Debit,
                            "credit" => Side::Credit,
                            other => panic!("{what}: side {other:?}"),
                        };
                        let class = line.project.as_ref().map(|p| {
                            *self.projects.get(p).unwrap_or_else(|| {
                                panic!("{what}: project {p:?} not in setup.projects")
                            })
                        });
                        (
                            self.id(&line.account),
                            side,
                            Money::new(money(&line.amount), self.currency),
                            class,
                        )
                    })
                    .collect();
                let mut probe = self.store.ledger().clone();
                let entry = probe.try_create_entry(day(), description)?;
                for (i, &(account, side, amount, class)) in lines.iter().enumerate() {
                    probe.add_line(entry, account, side, amount)?;
                    if class.is_some() {
                        probe.set_line_class(entry, i, class)?;
                    }
                }
                let entry = self.store.create_entry(day(), description)?;
                for (i, &(account, side, amount, class)) in lines.iter().enumerate() {
                    self.store.add_line(entry, account, side, amount)?;
                    if class.is_some() {
                        self.store.set_line_class(entry, i, class)?;
                    }
                }
                Ok(())
            }
            // Not a panic: the example fails as an engine refusal (`another
            // refusal: ...`), and the harness goes on to report it.
            "reverses" if e.draft => Err(StoreError::Io(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                format!(
                    "{what}: wanderbooks has no draft reversal: it picks reversal-evidence-is-originals (BS-ledger-17), under which a reversal posts at once"
                ),
            ))),
            // A platform's payout (BS-revenue-1): the engine's own plan of its
            // lines (`payout_plan`, which refuses unless gross - fee equals the
            // deposit to the cent), posted as one entry. The plan is made
            // before anything is journaled, so a refused payout leaves nothing.
            "payout" => {
                let p = e.payout.as_ref().expect("checked: a payout");
                let line = |account: &str, amount: &str| ledger_core::PayoutLine {
                    account: self.id(account),
                    amount: exact(amount, what),
                    class: None,
                };
                let payout = ledger_core::Payout {
                    deposit_account: self.id(&p.deposit_account),
                    deposit: exact(&p.deposit, what),
                    revenue: vec![line(&p.revenue_account, &p.gross)],
                    fees: match (&p.fee, &p.fee_account) {
                        (Some(fee), Some(account)) => vec![line(account, fee)],
                        _ => Vec::new(),
                    },
                    adjustments: Vec::new(),
                };
                let (plan, _) = ledger_core::payout_plan(&payout, self.store.ledger().chart())?;
                let entry = self.store.create_entry(day(), description)?;
                for (account, side, amount, _class) in plan {
                    self.store
                        .add_line(entry, account, side, Money::new(amount, self.currency))?;
                }
                if self.has_policy() {
                    self.store.post_entry_checked(entry)
                } else {
                    self.store.post_entry(entry)
                }
            }
            // A tag put on an asset (BS-asset-3): the engine's own command,
            // which refuses a tag of a wrong form and one another asset holds
            // or ever held.
            "tag_asset" => {
                let t = e.tag_asset.as_ref().expect("checked: a tag");
                let asset = self.asset(&t.asset, what);
                self.store.set_asset_tag(asset, &t.tag)
            }
            "post_draft" => {
                let p = e.post_draft.as_ref().expect("checked: a post");
                let entry = self.entry_of(p.entry, what);
                // With an evidence policy the post is the gate's: the
                // server's path for a hand-made draft; without one, the
                // primitive.
                if self.has_policy() {
                    self.store.post_entry_checked(entry)
                } else {
                    self.store.post_entry(entry)
                }
            }
            "set_draft_state" => {
                let s = e.set_draft_state.as_ref().expect("checked: a state");
                let entry = self.entry_of(s.entry, what);
                let state = match (s.state.as_str(), &s.note) {
                    ("building", None) => ledger_core::DraftState::Building,
                    ("awaiting_evidence", Some(note)) => {
                        ledger_core::DraftState::AwaitingEvidence { note: note.clone() }
                    }
                    other => panic!("{what}: draft state {other:?}"),
                };
                self.store.set_draft_state(entry, state)
            }
            "attach_receipt" => {
                let r = e.attach_receipt.as_ref().expect("checked: a receipt");
                let entry = self.entry_of(r.entry, what);
                // The document, by its reference: its own bytes, so that two
                // references are two documents (identical bytes are one file,
                // plan P6 step 4).
                let bytes = format!("%PDF-1.4\n{}\n%%EOF\n", r.receipt);
                self.store
                    .attach_receipt(
                        Some(entry),
                        &format!("{}.pdf", r.receipt.replace([' ', ',', '/'], "-")),
                        "application/pdf",
                        bytes.as_bytes(),
                        Some(&r.receipt),
                    )
                    .map(drop)
            }
            "discard_draft" => {
                let d = e.discard_draft.as_ref().expect("checked: a discard");
                let entry = self.entry_of(d.entry, what);
                self.store.discard_draft(entry, &d.reason)
            }
            "lines" if e.refused.is_some() => {
                // The books take an entry whole or not at all, so a
                // refused one leaves no draft behind. The store journals a
                // draft the moment it is made, so the entry is made, lined
                // and posted on a rehearsal: the same books, replayed from
                // their journal into a store of their own, which the engine
                // refuses through its own gate (`post_entry_checked` under a
                // policy, the server's path for a hand entry) and which is
                // then thrown away. The refusal is the engine's, never
                // rebuilt here.
                let mut rehearsal = self.rehearsal(what);
                // The books' own new-work entry point, so a date past the
                // calendar (BS-entry-7) is refused there, not rebuilt here.
                let entry = rehearsal.create_entry(day(), description)?;
                for (i, line) in e.lines.iter().enumerate() {
                    let side = match line.side.as_str() {
                        "debit" => Side::Debit,
                        "credit" => Side::Credit,
                        other => panic!("{what}: side {other:?}"),
                    };
                    let amount = Money::new(money(&line.amount), self.currency);
                    let account = self.id(&line.account);
                    rehearsal.add_line(entry, account, side, amount)?;
                    if let Some(p) = &line.project {
                        let class = *self.projects.get(p).unwrap_or_else(|| {
                            panic!("{what}: project {p:?} not in setup.projects")
                        });
                        rehearsal.set_line_class(entry, i, Some(class))?;
                    }
                }
                if self.has_policy() {
                    rehearsal.post_entry_checked(entry)
                } else {
                    rehearsal.post_entry(entry)
                }
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
                    if this.has_policy() {
                        this.store.post_entry_checked(entry)
                    } else {
                        this.store.post_entry(entry)
                    }
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
                if e.refused.is_some() {
                    // A close the books refuse (a draft stands in the period,
                    // BS-period-2) opens no period and posts nothing: it is
                    // tried on a clone, with the same close below.
                    let mut probe = self.store.ledger().clone();
                    let p = probe.create_period(&format!("event {n}"), start, day())?;
                    return Ok(probe
                        .close_period_to(p, &self.close_to(c, what))
                        .map(drop)?);
                }
                let p = self
                    .store
                    .create_period(&format!("event {n}"), start, day())?;
                let to = self.close_to(c, what);
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
                let (due, terms) = self.terms_due(b.terms.as_deref(), day(), what);
                self.store
                    .create_bill_with(
                        vendor,
                        day(),
                        due,
                        lines,
                        BillFields {
                            terms,
                            vendor_ref: b.vendor_ref.clone(),
                            duplicate_reason: b.duplicate_reason.clone(),
                            ..BillFields::default()
                        },
                    )
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
                let (due, terms) = self.terms_due(i.terms.as_deref(), day(), what);
                // The books number every invoice in order of issue (BS-arap-4),
                // as their invoicing settings default to.
                let number = self.l().next_invoice_number("INV-", 1);
                self.store
                    .create_invoice_with(
                        customer,
                        day(),
                        due,
                        lines,
                        InvoiceFields {
                            number: Some(number),
                            terms,
                            ..InvoiceFields::default()
                        },
                    )
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
                // (BS-arap-2), so a second write-off with the first's letter is
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
            "payment" => {
                let p = e.payment.as_ref().expect("checked: a payment");
                let entry = self.entry_of(p.document, what);
                let amount = money(&p.amount);
                let cash = self.id(&p.account);
                let invoice = self
                    .store
                    .ledger()
                    .invoices()
                    .into_iter()
                    .find(|i| i.entry_id == entry)
                    .map(|i| i.id);
                let bill = self
                    .store
                    .ledger()
                    .bills()
                    .into_iter()
                    .find(|b| b.entry_id == entry)
                    .map(|b| b.id);
                match (invoice, bill) {
                    (Some(invoice), _) => self
                        .store
                        .receive_payment(invoice, day(), amount, cash)
                        .map(drop),
                    (None, Some(bill)) => self.store.pay_bill(bill, day(), amount, cash).map(drop),
                    (None, None) => {
                        panic!("{what}: event {} posted no invoice or bill", p.document)
                    }
                }
            }
            "settlement" => {
                let st = e.settlement.as_ref().expect("checked: a settlement");
                let first = &st.applications[0];
                let (first_doc, party) = self.document_of(first.document, what);
                let applications: Vec<ledger_core::Application> = st
                    .applications
                    .iter()
                    .map(|a| ledger_core::Application {
                        doc: self.document_of(a.document, what).0,
                        cash: exact(&a.cash, what),
                        discount: a
                            .discount
                            .as_deref()
                            .map_or(Decimal::ZERO, |d| exact(d, what)),
                        discount_reason: a.discount_reason.clone(),
                    })
                    .collect();
                debug_assert_eq!(applications[0].doc, first_doc);
                let params = ledger_core::SettleParams {
                    date: day(),
                    description: String::new(),
                    cash_account: self.id(&st.account),
                    party,
                    applications,
                    fee: st.fee.as_deref().map(|f| ledger_core::Fee {
                        amount: exact(f, what),
                        account: None,
                        vendor: None,
                    }),
                    instrument: None,
                    reference: None,
                };
                self.store.settle(params).map(drop)
            }
            "recovery" => {
                // BS-arap-11, the pick reverse-then-plain-entry: in the tax
                // year of the write-off the write-off is reversed and the
                // money is recorded as any payment; in a later year a plain
                // entry debits the deposit account and credits the recoveries
                // account, naming the invoice.
                let r = e.recovery.as_ref().expect("checked: a recovery");
                let amount = exact(&r.amount, what);
                let (cash, income) = (self.id(&r.account), self.id(&r.income_account));
                let write_off = self.entry_of(r.write_off, what);
                let written = &self.ex.events[r.write_off - 1];
                let invoice_event = written
                    .write_off
                    .as_ref()
                    .map(|w| w.invoice)
                    .unwrap_or_else(|| panic!("{what}: event {} is no write-off", r.write_off));
                let (doc, _) = self.document_of(invoice_event, what);
                let ledger_core::DocRef::Invoice(invoice) = doc else {
                    panic!("{what}: a write-off is of an invoice")
                };
                let written_on = date(written.date.as_deref().expect("a write-off is dated"), what);
                let year =
                    |d| ledger_core::calendar::fiscal_year_start_on(fiscal_year_start(self.ex), d);
                if year(written_on) == year(day()) {
                    self.store.reverse_entry(
                        write_off,
                        day(),
                        "Recovered: the write-off is reversed",
                    )?;
                    self.store
                        .receive_payment(invoice, day(), amount, cash)
                        .map(drop)
                } else {
                    let entry = self
                        .store
                        .create_entry(day(), &format!("Recovery of invoice {invoice}"))?;
                    let money = Money::new(amount, self.currency);
                    self.store.add_line(entry, cash, Side::Debit, money)?;
                    self.store.add_line(entry, income, Side::Credit, money)?;
                    self.store.post_entry(entry)
                }
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
            "assign_role" => {
                let a = e.assign_role.as_ref().expect("checked: a role assignment");
                let account = self.id(&a.account);
                // The books' own journaled command: it checks the role's
                // account before it writes anything (BS-ledger-19).
                self.store.assign_role(&a.role, Some(account))
            }
            "set_setting" => {
                let s = e.set_setting.as_ref().expect("checked: a setting");
                // The books' own journaled command: the fact and who decided
                // it are kept in the journal (BS-ledger-9).
                let change = Change {
                    key: engine_setting_key(&s.key).to_string(),
                    value: Some(Value::String(s.value.clone())),
                };
                let source = Source {
                    decided_by: s.decided_by.clone(),
                    decided_on: date(&s.decided_on, what),
                    citation: None,
                    note: None,
                };
                self.store.change_settings(&[change], &source).map(|_| ())
            }
            other => unreachable!("{other} is no event"),
        }
    }
}
impl Books<'_> {
    /// Whether the example states an evidence policy (`setup.evidence_over`):
    /// the books' own setting, which the server's post gate reads.
    pub(crate) fn has_policy(&self) -> bool {
        self.ex.setup.evidence_over.is_some()
    }

    /// Where a close sends the period's income: to retained earnings, or to
    /// the members by their shares.
    pub(crate) fn close_to(&self, c: &bookspec_checker::CloseSpec, what: &str) -> CloseTo {
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
        match (c.to.as_str(), c.rounding.as_deref()) {
            ("members", None) => CloseTo::Members(members),
            ("members", Some("per-close")) => CloseTo::MembersPerClose(members),
            ("retained_earnings", None) => CloseTo::RetainedEarnings { sweep: vec![] },
            other => panic!("{what}: close {other:?}"),
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

/// Every role the schema's list names is a role this engine's command
/// takes, spelled the same: the adapter passes `assign_role`'s key
/// straight to `Store::assign_role`, so a key this engine spelled
/// differently would make every example that named it a refused
/// assignment the example did not ask for.
#[test]
fn every_listed_role_is_the_engine_role() {
    for &(role, _) in bookspec_checker::ACCOUNT_ROLES {
        assert!(
            ledger_core::control_account(role).is_some(),
            "{role}: the schema names a role this engine does not have"
        );
    }
}
