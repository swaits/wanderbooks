use crate::*;

/// An example's books: its setup, then its events as they are applied,
/// kept by the engine's own store (each command journaled as the books
/// journal it), in a directory of their own that goes when they do.
pub(crate) struct Books<'a> {
    pub(crate) ex: &'a Example,
    pub(crate) store: Store,
    /// Where the store keeps them.
    pub(crate) _dir: tempfile::TempDir,
    pub(crate) currency: Currency,
    /// The members, with their shares (zero when the example gives none).
    pub(crate) members: Vec<MemberCapital>,
    /// The members who are parties (`setup.parties` with the member
    /// role), as the settings give them to party shares and the 1099.
    pub(crate) party_members: Vec<PartyMember>,
    /// The directory's parties, by name.
    pub(crate) parties: BTreeMap<String, PartyId>,
    /// The payees' vendor records, by name.
    pub(crate) payees: BTreeMap<String, VendorId>,
    /// The vendors bills came from, by name (a payee's record if one).
    pub(crate) vendors: BTreeMap<String, VendorId>,
    /// The customers invoices went to, by name.
    pub(crate) customers: BTreeMap<String, u64>,
    /// The payment methods, by label.
    pub(crate) instruments: BTreeMap<String, InstrumentId>,
    /// The fixed assets of `setup.assets`, by id.
    pub(crate) assets: BTreeMap<String, AssetId>,
    /// The projects of `setup.projects`, by name.
    pub(crate) projects: BTreeMap<String, ledger_core::ClassId>,
    /// The schedules of `setup.schedules` and `register_schedule` events,
    /// by id.
    pub(crate) schedules: BTreeMap<String, ScheduleId>,
    /// The entries each event applied so far posted, in order: event N's
    /// at N - 1.
    pub(crate) posted: Vec<Vec<EntryId>>,
    /// Each `run_depreciation` event's run, by event.
    pub(crate) runs: BTreeMap<usize, DepreciationRun>,
    /// What each disposal took, by asset id, as the engine computed it
    /// just before posting it.
    pub(crate) disposals: BTreeMap<String, DisposalPreview>,
}

/// A store command's result, unwrapped: the example's books take it.
pub(crate) fn took<T>(r: Result<T, StoreError>, what: &str) -> T {
    r.unwrap_or_else(|e| panic!("{what}: {e}"))
}

/// Who an example's decisions and records are made by.
pub(crate) fn bookspec_source(id: &str) -> Source {
    Source {
        decided_by: "bookspec".into(),
        decided_on: date("2026-01-01", id),
        citation: None,
        note: None,
    }
}

/// A payee's 1099 decision in an example's words (bookspec/README.md).
pub(crate) fn decision(
    ex: &str,
    what: &str,
    kind: Option<&str>,
    reason: Option<&str>,
) -> Decision1099 {
    match what {
        "not_reportable" => {
            // Exactly as the README spells it: the engine's parse forgives
            // case and spaces, an example doesn't.
            let kind = kind
                .and_then(ExemptReason::parse)
                .filter(|k| Some(k.key()) == kind)
                .unwrap_or_else(|| {
                    panic!("{ex}: not_reportable needs a reason_kind, got {kind:?}")
                });
            let reason = reason.unwrap_or_else(|| panic!("{ex}: not_reportable needs a reason"));
            Decision1099::not_reportable(kind, reason).unwrap_or_else(|e| panic!("{ex}: {e}"))
        }
        "undetermined" | "1099-NEC" | "1099-MISC" => {
            assert!(
                kind.is_none() && reason.is_none(),
                "{ex}: only not_reportable takes reason_kind and reason"
            );
            match Form1099::parse(what) {
                Some(f) if what.starts_with("1099-") => Decision1099::Files(f),
                _ => Decision1099::Undetermined,
            }
        }
        other => panic!("{ex}: decision {other:?}"),
    }
}

/// The example's directory (`setup.parties`), by name: each party with
/// its roles (a vendor or customer with its record), the liabilities it
/// is the creditor of, and its TIN and W-9 facts' last four.
pub(crate) fn add_parties(ex: &Example, store: &mut Store) -> BTreeMap<String, PartyId> {
    let mut out = BTreeMap::new();
    for p in &ex.setup.parties {
        let what = format!("{} party {}", ex.id, p.name);
        let roles: Vec<Role> = p
            .roles
            .iter()
            .map(|r| Role::parse(r).unwrap_or_else(|| panic!("{what}: role {r:?}")))
            .collect();
        let record = |r: Role| roles.contains(&r);
        let others: Vec<Role> = roles
            .iter()
            .copied()
            .filter(|r| !matches!(r, Role::Vendor | Role::Customer))
            .collect();
        let id = match (record(Role::Vendor), record(Role::Customer)) {
            (true, true) => panic!("{what}: a vendor or a customer, not both"),
            (true, false) => {
                let v = took(store.add_vendor(&p.name), &what);
                store.ledger().party_of_vendor(v).expect("a vendor's party")
            }
            (false, true) => {
                let c = took(store.add_customer(&p.name), &what);
                store
                    .ledger()
                    .party_of_customer(c)
                    .expect("a customer's party")
            }
            (false, false) => took(store.add_party(&p.name, &others), &what),
        };
        if record(Role::Vendor) || record(Role::Customer) {
            for r in &others {
                took(store.set_party_role(id, *r, true), &what);
            }
        }
        for code in &p.creditor_of {
            let account = store
                .ledger()
                .chart()
                .find_by_code(code)
                .unwrap_or_else(|| panic!("{what}: no account {code}"))
                .id;
            took(
                store.set_party_creditor(id, account, true),
                &format!("{what}: creditor of {code}"),
            );
        }
        assert!(
            !p.tin_purged || p.tin_last4.is_some(),
            "{what}: tin_purged needs tin_last4"
        );
        if let Some(last4) = &p.tin_last4 {
            let facts = TinFacts {
                tin_type: TinType::Ssn,
                last4: last4.clone(),
                vault_record: (!p.tin_purged).then(|| "0".repeat(64)),
                purged: p.tin_purged.then(|| "purged: retention ended".into()),
            };
            took(store.set_party_tin(id, Some(facts)), &what);
        }
        if let Some(last4) = &p.w9_tin_last4 {
            let facts = W9Facts {
                received: date("2026-01-01", &what),
                entity_type: "individual/sole proprietor".into(),
                exempt_payee_code: None,
                fatca_code: None,
                tin_last4: last4.clone(),
                kept_at: "wanderbooks vault".into(),
                vault_record: None,
            };
            took(store.set_party_w9(id, Some(facts)), &what);
        }
        assert!(out.insert(p.name.clone(), id).is_none(), "{what}: twice");
    }
    out
}

/// An asset's basis facts as the books record them.
pub(crate) fn basis(b: &BasisSpec, what: &str) -> ledger_core::AssetBasis {
    let amount = |s: &Option<String>| s.as_deref().map(|v| exact(v, what));
    ledger_core::AssetBasis {
        rule: ledger_core::BasisRule::parse(&b.rule)
            .unwrap_or_else(|| panic!("{what}: basis rule {:?}", b.rule)),
        contributor_adjusted_basis: amount(&b.contributor_adjusted_basis),
        fmv_at_contribution: amount(&b.fmv_at_contribution),
        sec704c_method: b.sec704c_method.as_deref().map(|m| {
            ledger_core::Sec704cMethod::parse(m)
                .unwrap_or_else(|| panic!("{what}: §704(c) method {m:?}"))
        }),
        source: None,
    }
}

/// A schedule as the books register it.
pub(crate) fn new_schedule(
    l: &Ledger,
    s: &ScheduleSpec,
    currency: Currency,
    what: &str,
) -> NewSchedule {
    let account = |code: &str| {
        l.chart()
            .find_by_code(code)
            .unwrap_or_else(|| panic!("{what}: no account {code}"))
            .id
    };
    NewSchedule {
        description: s.description.clone(),
        kind: match s.kind.as_str() {
            "prepaid_expense" => ScheduleKind::PrepaidExpense,
            "deferred_revenue" => ScheduleKind::DeferredRevenue,
            other => panic!("{what}: schedule kind {other:?}"),
        },
        total: Money::new(exact(&s.total, what), currency),
        start: date(&s.start, what),
        months: s.months,
        balance_account: account(&s.balance_account),
        pl_account: account(&s.pl_account),
        convention: ScheduleConvention::parse(&s.convention)
            .unwrap_or_else(|| panic!("{what}: schedule convention {:?}", s.convention)),
    }
}

impl<'a> Books<'a> {
    /// The books an example sets up, before any event: its shape checked,
    /// its accounts, fixed assets, schedules, parties, members, payees,
    /// their dated decisions, and payment methods.
    pub(crate) fn blank(ex: &'a Example) -> Self {
        let currency = Currency::new(&ex.setup.currency).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let store = took(
            Store::create(&dir.path().join("books"), currency),
            &format!("{}: the books", ex.id),
        );
        Books {
            ex,
            store,
            _dir: dir,
            currency,
            members: Vec::new(),
            party_members: Vec::new(),
            parties: BTreeMap::new(),
            payees: BTreeMap::new(),
            vendors: BTreeMap::new(),
            customers: BTreeMap::new(),
            instruments: BTreeMap::new(),
            assets: BTreeMap::new(),
            projects: BTreeMap::new(),
            schedules: BTreeMap::new(),
            posted: Vec::new(),
            runs: BTreeMap::new(),
            disposals: BTreeMap::new(),
        }
    }

    /// The books an example sets up, before any event: its shape checked,
    /// its accounts, fixed assets, schedules, parties, members, payees,
    /// their dated decisions, and payment methods.
    pub(crate) fn setup(&mut self) {
        let ex = self.ex;
        let s = &ex.setup;
        let currency = self.currency;
        assert!(
            ex.appendix.as_deref().is_none_or(|a| a == "formats"),
            "{}: appendix {:?}",
            ex.id,
            ex.appendix
        );
        assert!(
            matches!(s.basis.as_str(), "accrual" | "cash"),
            "{}: basis {:?}",
            ex.id,
            s.basis
        );
        fiscal_year_start(ex);
        if let Some(today) = &s.today {
            date(today, &format!("{} today", ex.id));
        }
        assert!(
            matches!(
                s.rounding.as_str(),
                "half-even" | "half-away-from-zero" | "largest-remainder"
            ),
            "{}: rounding {:?}",
            ex.id,
            s.rounding
        );
        // The engine splits a close among the members by largest
        // remainder (rules.md B-13): an example closing to the members
        // says so.
        assert!(
            s.rounding == "largest-remainder"
                || !ex
                    .events
                    .iter()
                    .any(|e| e.close.as_ref().is_some_and(|c| c.to == "members")),
            "{}: a close to the members rounds by largest-remainder",
            ex.id
        );
        let id = |l: &Ledger, code: &str| {
            l.chart()
                .find_by_code(code)
                .unwrap_or_else(|| panic!("{}: no account {code}", ex.id))
                .id
        };
        for a in &s.accounts {
            let what = format!("{}: account {}", ex.id, a.code);
            // A parent is declared before its subaccounts.
            let parent = a.parent.as_deref().map(|p| id(self.store.ledger(), p));
            let account = took(
                self.store
                    .add_account(&a.code, &a.name, account_type(a, &ex.id), parent, a.header),
                &what,
            );
            if let Some(c) = &a.tax_category {
                took(self.store.set_account_tax(account, Some(c), None), &what);
            }
        }
        let first = ex
            .events
            .iter()
            .find_map(|e| e.date.as_deref())
            .map_or(date("2026-01-01", "start"), |d| date(d, "date"));
        for fa in &s.fixed_assets {
            let new = NewAsset {
                description: format!("Fixed asset on {}", fa.cost_account),
                in_service_date: first,
                cost: Money::new(Decimal::ONE, currency),
                salvage_value: Money::new(Decimal::ZERO, currency),
                useful_life_months: 12,
                cost_account: id(self.store.ledger(), &fa.cost_account),
                accum_depr_account: id(self.store.ledger(), &fa.accumulated_account),
                expense_account: id(self.store.ledger(), &fa.expense_account),
            };
            took(
                self.store.register_asset(new),
                &format!("{}: fixed asset", ex.id),
            );
        }
        let mut assets = BTreeMap::new();
        for a in &s.assets {
            let what = format!("{}: asset {}", ex.id, a.id);
            assert_eq!(a.method, "straight_line", "{what}: method");
            let new = NewAsset {
                description: a.description.clone(),
                in_service_date: date(&a.in_service, &what),
                cost: Money::new(exact(&a.cost, &what), currency),
                salvage_value: Money::new(exact(&a.salvage, &what), currency),
                useful_life_months: a.life_months,
                cost_account: id(self.store.ledger(), &a.cost_account),
                accum_depr_account: id(self.store.ledger(), &a.accumulated_account),
                expense_account: id(self.store.ledger(), &a.expense_account),
            };
            let asset = took(self.store.register_asset(new), &what);
            assert!(
                assets.insert(a.id.clone(), asset).is_none(),
                "{what}: id used twice"
            );
            if let Some(b) = &a.basis {
                took(self.store.set_asset_basis(asset, basis(b, &what)), &what);
            }
            assert!(
                a.acquired_by.is_none_or(|n| n >= 1 && n <= ex.events.len()),
                "{what}: acquired_by = {:?}: events count from 1, to {}",
                a.acquired_by,
                ex.events.len()
            );
        }
        let mut schedules = BTreeMap::new();
        for sc in &s.schedules {
            let what = format!("{}: schedule {}", ex.id, sc.id);
            let new = new_schedule(self.store.ledger(), sc, currency, &what);
            let schedule = took(self.store.register_schedule(new), &what);
            assert!(
                schedules.insert(sc.id.clone(), schedule).is_none(),
                "{what}: id used twice"
            );
        }
        let shared = s.members.iter().filter(|m| m.share.is_some()).count();
        assert!(
            shared == 0 || shared == s.members.len(),
            "{}: every member has a share, or none does",
            ex.id
        );
        let members: Vec<MemberCapital> = s
            .members
            .iter()
            .map(|m| MemberCapital {
                name: m.name.clone(),
                capital_account: id(self.store.ledger(), &m.capital_account),
                distribution_account: m
                    .distributions_account
                    .as_deref()
                    .map(|c| id(self.store.ledger(), c)),
                share: m.share.as_deref().map_or(Decimal::ZERO, |x| {
                    let share = exact(x, &format!("{} {} share", ex.id, m.name));
                    assert!(share > Decimal::ZERO, "{}: {}'s share", ex.id, m.name);
                    share
                }),
            })
            .collect();
        if shared > 0 {
            let total: Decimal = members.iter().map(|m| m.share).sum();
            assert_eq!(total, Decimal::ONE, "{}: the shares sum to 1", ex.id);
        }
        let parties = add_parties(ex, &mut self.store);
        // A member who is a party (by name, with the member role), as the
        // settings' members are (settings::Company::party_members).
        let party_members: Vec<PartyMember> = members
            .iter()
            .filter_map(|m| {
                let p = self.store.ledger().party_by_name(&m.name)?;
                p.roles.contains(&Role::Member).then_some(PartyMember {
                    party: p.id,
                    share: m.share,
                    capital: m.capital_account,
                    distributions: m.distribution_account,
                })
            })
            .collect();
        let source = bookspec_source(&ex.id);
        let mut payees = BTreeMap::new();
        for p in &s.payees {
            let what = format!("{}: payee {}", ex.id, p.name);
            let form = Form1099::parse(&p.form)
                .filter(|f| f.label() == p.form)
                .unwrap_or_else(|| panic!("{what}: form {:?}", p.form));
            // A payee in the directory with the vendor role is that
            // party's vendor record.
            let known = parties
                .get(&p.name)
                .and_then(|&id| self.store.ledger().get_party(id)?.vendor);
            let v = match known {
                Some(v) => v,
                None => took(self.store.add_vendor(&p.name), &what),
            };
            let dated = Dated1099::from_beginning(Decision1099::Files(form), Some(source.clone()));
            took(self.store.set_vendor_1099(v, &dated, Some(false)), &what);
            assert!(
                payees.insert(p.name.clone(), v).is_none(),
                "{}: payee {} twice",
                ex.id,
                p.name
            );
        }
        for c in &s.status_changes {
            let what = format!("{} status change of {}", ex.id, c.payee);
            let &v = payees
                .get(&c.payee)
                .unwrap_or_else(|| panic!("{what}: payee not in setup"));
            let dated = Dated1099 {
                effective: Some(date(&c.effective, &what)),
                decision: decision(
                    &what,
                    &c.decision,
                    c.reason_kind.as_deref(),
                    c.reason.as_deref(),
                ),
                source: Some(source.clone()),
            };
            took(self.store.set_vendor_1099(v, &dated, Some(false)), &what);
        }
        let mut projects = BTreeMap::new();
        for p in &s.projects {
            let what = format!("{}: project {}", ex.id, p.name);
            let facts = ledger_core::ProjectFacts {
                kind: p.kind.as_deref().map(|k| {
                    ledger_core::ProjectKind::parse(k)
                        .unwrap_or_else(|| panic!("{what}: kind {k:?}"))
                }),
                status: p.status.as_deref().map_or(Default::default(), |k| {
                    ledger_core::ProjectStatus::parse(k)
                        .unwrap_or_else(|| panic!("{what}: status {k:?}"))
                }),
                party: p.party.as_ref().map(|n| {
                    *parties
                        .get(n)
                        .unwrap_or_else(|| panic!("{what}: party {n:?} not in setup.parties"))
                }),
                ..Default::default()
            };
            let id = took(self.store.add_project(&p.name, &facts), &what);
            assert!(
                projects.insert(p.name.clone(), id).is_none(),
                "{what}: named twice"
            );
        }
        let mut instruments = BTreeMap::new();
        for i in &s.instruments {
            let what = format!("{} instrument {}", ex.id, i.label);
            let kind = InstrumentKind::parse(&i.kind)
                .filter(|k| k.key() == i.kind)
                .unwrap_or_else(|| panic!("{what}: kind {:?}", i.kind));
            let account = id(self.store.ledger(), &i.account);
            let owner = i.owner.as_ref().map(|o| {
                *parties
                    .get(o)
                    .unwrap_or_else(|| panic!("{what}: owner {o:?} not in setup.parties"))
            });
            let id = took(
                self.store
                    .add_instrument(kind, &i.label, i.last4.as_deref(), owner, account),
                &what,
            );
            assert!(
                instruments.insert(i.label.clone(), id).is_none(),
                "{what}: twice"
            );
        }
        self.members = members;
        self.party_members = party_members;
        self.parties = parties;
        self.vendors = payees.clone();
        self.customers = BTreeMap::new();
        self.payees = payees;
        self.instruments = instruments;
        self.assets = assets;
        self.projects = projects;
        self.schedules = schedules;
    }

    /// The books an example sets up, already loaded.
    pub(crate) fn new(ex: &'a Example) -> Self {
        let mut books = Self::blank(ex);
        books.setup();
        books
    }

    /// The books as they stand.
    pub(crate) fn l(&self) -> &Ledger {
        self.store.ledger()
    }

    pub(crate) fn id(&self, code: &str) -> ledger_core::AccountId {
        self.l()
            .chart()
            .find_by_code(code)
            .unwrap_or_else(|| panic!("{}: no account {code}", self.ex.id))
            .id
    }

    /// The fixed asset `setup.assets` names `name`.
    pub(crate) fn asset(&self, name: &str, what: &str) -> AssetId {
        *self
            .assets
            .get(name)
            .unwrap_or_else(|| panic!("{what}: no asset {name:?} in setup.assets"))
    }

    /// The entry event `event` (counting from 1) posted: an event applied
    /// already, that posted exactly one.
    pub(crate) fn entry_of(&self, event: usize, what: &str) -> EntryId {
        assert!(
            event >= 1 && event <= self.posted.len(),
            "{what}: event {event}: events count from 1, to the last one applied ({})",
            self.posted.len()
        );
        match self.posted[event - 1].as_slice() {
            [e] => *e,
            other => panic!(
                "{what}: event {event} posted {} entries: only an event that posts one entry names it",
                other.len()
            ),
        }
    }
}
