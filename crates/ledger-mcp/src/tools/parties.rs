//! Parties through the server: customers, vendors and other parties,
//! their roles, W-9s, the TIN vault, 1099s, and the entry's parties.
//! Split whole out of `server.rs` (plan/served.md 8.2 U0a).

use super::*;

/// The 1099 worksheet for `year`, the one every tool and the CPA package
/// use (ledger_package::vendor_1099_report): payments out of the cash and
/// financing roles, an entry's shares resolved with the settings' members
/// (as [`share_context`] checks them).
fn vendor_1099_report(
    ledger: &ledger_core::Ledger,
    settings: &Settings,
    year: i32,
    threshold: Option<Decimal>,
) -> Result<ledger_core::Vendor1099Report, ledger_core::LedgerError> {
    ledger_package::vendor_1099_report(ledger, &package_settings(settings), year, threshold, None)
}

/// A TIN type from a tool: ssn, ein, or itin.
fn parse_tin_type(s: &str) -> Result<ledger_vault::TinType, String> {
    match s.trim().to_lowercase().as_str() {
        "ssn" => Ok(ledger_vault::TinType::Ssn),
        "ein" => Ok(ledger_vault::TinType::Ein),
        "itin" => Ok(ledger_vault::TinType::Itin),
        _ => Err("tin_type is ssn, ein, or itin".into()),
    }
}

/// Parse role names (`Role::key`), refusing any unknown one.
fn parse_roles(roles: &[String]) -> Result<Vec<ledger_core::Role>, String> {
    roles
        .iter()
        .map(|r| {
            ledger_core::Role::parse(r.trim()).ok_or_else(|| {
                format!(
                    "unknown role {r:?}: expected one of {}",
                    ledger_core::Role::ALL.map(|r| r.key()).join(", ")
                )
            })
        })
        .collect()
}

/// A party's roles (vendor and customer included), in display order.
fn party_roles(p: &ledger_core::Party) -> Vec<String> {
    let all = p.all_roles();
    ledger_core::Role::ALL
        .into_iter()
        .filter(|r| all.contains(r))
        .map(|r| r.key().to_string())
        .collect()
}

/// A party's facts for a tool's output.
fn party_out(ledger: &Ledger, p: &ledger_core::Party) -> PartyOut {
    PartyOut {
        party_id: p.id,
        name: p.name.clone(),
        label: p.label().to_string(),
        display_name: p.display_name.clone(),
        roles: party_roles(p),
        website: p.website.clone(),
        address: p.address.clone(),
        notes: p.notes.clone(),
        email: p.email.clone(),
        phone: p.phone.clone(),
        contact: p.contact.clone(),
        billing_address: p.billing_address.clone(),
        shipping_address: p.shipping_address.clone(),
        billing_email: p.billing_email.clone(),
        default_terms_as_customer: p
            .default_terms_as_customer
            .as_ref()
            .map(ledger_core::Terms::describe),
        default_terms_as_vendor: p
            .default_terms_as_vendor
            .as_ref()
            .map(ledger_core::Terms::describe),
        archived: p.archived,
        vendor_id: p.vendor,
        customer_id: p.customer,
        w9: p.w9.as_ref().map(|w| W9Out {
            received_on: w.received.to_string(),
            entity_type: w.entity_type.clone(),
            exempt_payee_code: w.exempt_payee_code.clone(),
            fatca_code: w.fatca_code.clone(),
            tin_last4: w.tin_last4.clone(),
            kept_at: w.kept_at.clone(),
            in_vault: w.vault_record.is_some(),
        }),
        w4: p.w4.as_ref().map(|w| W4Out {
            received_on: w.received.to_string(),
            state: w.state.clone(),
            kept_at: w.kept_at.clone(),
        }),
        tin: p.tin.as_ref().map(|t| TinOut {
            tin_type: t.tin_type.key().to_string(),
            last4: t.last4.clone(),
            in_vault: t.vault_record.is_some(),
            purged: t.purged.clone(),
        }),
        creditor_of: p
            .creditor_of
            .iter()
            .filter_map(|a| ledger.chart().get(*a).map(|a| a.code.clone()))
            .collect(),
    }
}

/// A document's status for a tool's output.
fn status_key(s: DocumentStatus) -> String {
    match s {
        DocumentStatus::Open => "open",
        DocumentStatus::Partial => "partial",
        DocumentStatus::Paid => "paid",
    }
    .to_string()
}

/// An invoice or bill for `books.party`: its (date, due date), (total,
/// paid so far), status, and entry.
fn party_document(
    id: u64,
    (date, due): (NaiveDate, NaiveDate),
    (total, balance, paid): (
        Result<Decimal, ledger_core::LedgerError>,
        Result<Decimal, ledger_core::LedgerError>,
        Decimal,
    ),
    status: DocumentStatus,
    entry_id: u64,
) -> Result<PartyDocumentOut, String> {
    Ok(PartyDocumentOut {
        id,
        date: date.to_string(),
        due_date: due.to_string(),
        total: money(total.map_err(|e| e.to_string())?),
        paid: money(paid),
        balance: money(balance.map_err(|e| e.to_string())?),
        status: status_key(status),
        entry_id,
    })
}

/// A party-totals `basis` from a tool: "cash" (the default) or "all"
/// (rules.md B-35 item 4); any other word is refused.
fn parse_basis(basis: Option<&str>) -> Result<ledger_core::parties::Basis, String> {
    match basis {
        None => Ok(ledger_core::parties::Basis::default()),
        Some(b) => ledger_core::parties::Basis::parse(b)
            .ok_or_else(|| format!("basis is \"cash\" or \"all\", not {b:?}")),
    }
}

/// Everything the books did with one party (`PartyTotalsOut` defines it).
struct PartyActivity {
    /// The party's entries, drafts included.
    entries: std::collections::BTreeSet<u64>,
    /// Paid to it in the year, on the basis asked.
    paid: Decimal,
    /// Received from it in the year, on the basis asked.
    received: Decimal,
}

/// The party's entries and the year's paid and received on `basis`
/// (`PartyTotalsOut`): ledger_core::parties::activity_with over the
/// calendar year, with the money accounts of [`share_context`] and the
/// members (so an entry naming several parties counts this one's part).
fn party_activity(
    store: &Store,
    party: &ledger_core::Party,
    settings: &Settings,
    year: i32,
    basis: ledger_core::parties::Basis,
) -> Result<PartyActivity, String> {
    let ledger = store.ledger();
    let span = (
        NaiveDate::from_ymd_opt(year, 1, 1).ok_or("year out of range")?,
        NaiveDate::from_ymd_opt(year, 12, 31).ok_or("year out of range")?,
    );
    let (money, members) = share_context(ledger, settings);
    let a = ledger_core::parties::activity_with(ledger, party.id, span, &money, &members);
    let (paid, received) = a.totals(basis);
    Ok(PartyActivity {
        entries: a.entries,
        paid,
        received,
    })
}

/// Refuse shares that don't resolve against the entry's lines, once the
/// lines are settled ([`lines_settled`]).
fn check_shares_resolve(
    store: &Store,
    settings: &Settings,
    entry_id: u64,
    shares: &[ledger_core::PartyShare],
) -> Result<(), String> {
    let ledger = store.ledger();
    let entry = ledger
        .get_entry(entry_id)
        .ok_or_else(|| format!("unknown entry {entry_id}"))?;
    if !lines_settled(entry) {
        return Ok(());
    }
    let (money, members) = share_context(ledger, settings);
    ledger_core::shares::resolve(ledger, entry_id, shares, &money, &members).map(|_| ())
}

/// The party's customer record's invoices, by id.
fn party_invoices<'a>(
    ledger: &'a Ledger,
    party: &ledger_core::Party,
) -> Vec<&'a ledger_core::Invoice> {
    let Some(c) = party.customer else {
        return Vec::new();
    };
    ledger
        .invoices()
        .into_iter()
        .filter(|i| i.customer_id == c)
        .collect()
}

/// The party's vendor record's bills, by id.
fn party_bills<'a>(ledger: &'a Ledger, party: &ledger_core::Party) -> Vec<&'a ledger_core::Bill> {
    let Some(v) = party.vendor else {
        return Vec::new();
    };
    ledger
        .bills()
        .into_iter()
        .filter(|b| b.vendor_id == v)
        .collect()
}

#[tool_router(router = parties_router, vis = "pub(super)")]
impl BooksServer {
    #[tool(
        name = "books.list_vendors",
        title = "List vendors",
        description = "Vendors, each with its 1099 decision in force today (nec, misc, not_reportable with its reason, or undetermined; who decided and when) and every dated decision, whether its W-9 is on file, and the party it belongs to (books.party). undetermined with no decided_by: nobody has decided yet (books.set_vendor_1099).",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_vendors(&self) -> Result<Json<ListVendorsOut>, String> {
        let store = self.lock()?;
        Ok(Json(ListVendorsOut {
            vendors: store
                .ledger()
                .list_vendors()
                .into_iter()
                .map(|v| instruments::vendor_out(&store, v))
                .collect(),
        }))
    }

    #[tool(
        name = "books.set_vendor_1099",
        title = "Record a vendor's 1099 decision",
        description = "Record whether a vendor gets a 1099, with who decided (decided_by): nec (1099-NEC: contractors, attorneys), misc (1099-MISC: rent, royalties), not_reportable with reason_kind (corporation, tax_exempt, united_states, state, foreign_government, foreign_person, merchandise, employee, other with a citation) and reason in words, or undetermined. A corporation paid for legal or medical services stays nec/misc. Paid by card is never a reason (cards are excluded per payment). effective: only when the payee's status changed (an LLC's corporate election, from its date); a payee that incorporates is a new vendor. tin_on_file: a W-9 is on file (alone, it keeps the decision). Ask the human; CPA confirms.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn set_vendor_1099(
        &self,
        params: Parameters<SetVendor1099Params>,
    ) -> Result<Json<SetVendor1099Out>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let id = resolve_vendor(&store, &p.vendor)?;
        let decision = instruments::decision_in(&p)?;
        let (changed, warnings) = match decision {
            None => {
                let flag = p.tin_on_file.expect("decision_in: one of the two");
                store
                    .set_vendor_w9_flag(id, flag)
                    .map_err(|e| e.to_string())?;
                (ledger_core::form1099::Applied::Unchanged, Vec::new())
            }
            Some(d) => {
                let reached = ledger_core::form1099::decision_changes(store.ledger(), id, &d)
                    .map_err(|e| e.to_string())?;
                let applied = store
                    .set_vendor_1099(id, &d, p.tin_on_file)
                    .map_err(|e| e.to_string())?;
                let name = &store.ledger().get_vendor(id).expect("resolved").name;
                let warnings = instruments::filed_year_warnings(
                    store.ledger(),
                    &format!("{name}'s 1099 decision"),
                    &reached,
                    today(),
                );
                (applied, warnings)
            }
        };
        let v = store.ledger().get_vendor(id).expect("exists");
        Ok(Json(SetVendor1099Out {
            vendor: instruments::vendor_out(&store, v),
            changed: instruments::applied_key(changed).to_string(),
            warnings,
        }))
    }

    #[tool(
        name = "books.set_entry_vendor",
        title = "Tag an entry's vendor",
        description = "Tag an entry (draft or posted) with the vendor it paid, for 1099 tracking, or omit vendor to clear the tag. Reporting metadata only: amounts never change. On an entry naming one party or none, the vendor's party becomes its party; clearing also clears that party. On an entry naming several parties (books.set_entry_party parties) the parties stay as they are: the vendor of a party already named is tagged with every share kept; any other vendor, or clearing the tag while its party is named, is refused (change the parties instead). An entry and its reversal are tagged together. books.pay_bill tags its payments automatically; books.draft_entry takes a vendor too.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn set_entry_vendor(
        &self,
        params: Parameters<SetEntryVendorParams>,
    ) -> Result<Json<GetEntryOut>, String> {
        let p = params.0;
        {
            let mut store = self.lock()?;
            let vendor = p
                .vendor
                .as_deref()
                .map(|v| resolve_vendor(&store, v))
                .transpose()?;
            store
                .set_entry_vendor(p.entry_id, vendor)
                .map_err(|e| e.to_string())?;
        }
        self.get_entry(Parameters(GetEntryParams {
            entry_id: p.entry_id,
        }))
        .await
    }

    #[tool(
        name = "books.vendor_1099",
        title = "1099 worksheet",
        description = "The 1099 worksheet for a calendar year. Per vendor: what counts toward its 1099 (paid out of the cash accounts, or through a financing account with the method recorded, while not decided not_reportable; an entry with several parties counts only the vendor's share), what was paid by card or payment network (excluded: Form 1099-K's), what is undetermined (a member paid, method not recorded: books.set_entry_instrument), its 1099 decision, and its status (reportable at the threshold: $600 through 2025, $2,000 in 2026, indexed after; under_threshold; not_reportable; undetermined). Lists those payments, W-9s missing, each payee's TIN last four. include_tins adds full TINs from the vault; csv hands it over. cash_accounts and financing_accounts default to the settings' (as books.export_cpa_package's do); through stops the year at a day: given a package's accounts and its as_of, the CSV is the package's 1099 worksheet. A year whose threshold isn't known is refused: pass threshold from the IRS. For the CPA to confirm.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn vendor_1099(
        &self,
        params: Parameters<Vendor1099Params>,
    ) -> Result<Json<Vendor1099Out>, String> {
        let p = params.0;
        let threshold = p.threshold.as_deref().map(parse_amount).transpose()?;
        let through = p.through.as_deref().map(parse_date).transpose()?;
        if let Some(t) = through
            && chrono::Datelike::year(&t) != p.year
        {
            return Err(format!(
                "through ({t}) is a day of {}: the worksheet counts payments made in {} from January 1 through it",
                p.year, p.year
            ));
        }
        let store = self.lock()?;
        let ledger = store.ledger();
        for code in p.cash_accounts.iter().chain(&p.financing_accounts) {
            if ledger.chart().find_by_code(code).is_none() {
                return Err(format!(
                    "no account {code} in the chart (books.chart lists them)"
                ));
            }
        }
        // With include_tins, the vault is opened once (never creating a
        // key: a locked vault becomes a note on each line).
        let vault = p.include_tins.then(|| self.open_vault(&store, false));
        let tins = vault
            .as_ref()
            .map(|vault| move |id| vault_tin(ledger, vault, id));
        // The accounts given, else the settings': the CPA package's rule
        // (ledger_package::Settings::with_accounts), so its 1099 worksheet
        // is this one for the same accounts and `through` its as_of.
        let settings = self.settings();
        let accounts =
            package_settings(&settings).with_accounts(&p.cash_accounts, &p.financing_accounts);
        // A year past what wanderbooks knows is refused with guidance
        // (research or ask, pass `threshold`, then update the rule).
        let w = ledger_package::vendor_1099_worksheet(
            ledger,
            &accounts,
            p.year,
            threshold,
            through,
            tins.as_ref().map(|f| f as ledger_package::TinSource<'_>),
        )?;
        let warning = p.include_tins.then(|| {
            "This worksheet holds full TINs, for the CPA; keep it out of the books.".to_string()
        });
        let csv_file = match p.csv {
            false => None,
            true => {
                let csv = ledger_package::vendor_1099_csv(&w, p.include_tins);
                if p.include_tins {
                    self.wrote_tins(&store, csv.as_bytes())?;
                }
                let name = ledger_package::vendor_1099_csv_name(w.year, p.include_tins);
                Some(self.hand_over(&book_name(&store), &name, "text/csv", csv.into_bytes())?)
            }
        };
        let payments = |list: &[ledger_core::Payment1099]| -> Vec<Payment1099Out> {
            list.iter()
                .map(|p| Payment1099Out {
                    entry_id: p.entry_id,
                    date: p.date.to_string(),
                    vendor_id: p.vendor_id,
                    vendor: ledger
                        .get_vendor(p.vendor_id)
                        .map(|v| v.name.clone())
                        .unwrap_or_default(),
                    amount: money(p.amount),
                    instrument: p
                        .instrument
                        .and_then(|i| ledger.get_instrument(i))
                        .map(|i| i.label.clone()),
                })
                .collect()
        };
        let excluded_card_payments = payments(&w.excluded_card_payments);
        let undetermined_payments = payments(&w.undetermined_payments);
        let other_receipts = payments(&w.other_receipts);
        let vendors = w
            .lines
            .into_iter()
            .map(|l| Vendor1099LineOut {
                vendor_id: l.vendor_id,
                name: l.name,
                form_1099: l.form_1099,
                decision_1099: instruments::decision_out(Some(&ledger_core::Dated1099 {
                    effective: l.effective,
                    decision: l.decision.clone(),
                    source: l.decided.clone(),
                })),
                decisions_in_year: l
                    .decisions
                    .iter()
                    .map(|d| instruments::decision_out(Some(d)))
                    .collect(),
                tin_on_file: l.tin_on_file,
                paid: money(l.paid),
                excluded_card: money(l.excluded_card),
                undetermined: money(l.undetermined),
                not_reportable: money(l.not_reportable),
                status: l.status.key().to_string(),
                decision_needed: l.decision_needed,
                reportable: l.reportable,
                party_id: l.party_id,
                tin_last4: l.tin_last4,
                tin: l.tin,
                tin_type: l.tin_type,
                tin_note: l.tin_note,
            })
            .collect();
        Ok(Json(Vendor1099Out {
            year: w.year,
            threshold: money(w.threshold),
            vendors,
            missing_w9: w.missing_w9,
            excluded_card_payments,
            undetermined_payments,
            other_receipts,
            untagged_entries: w.untagged_payments,
            unresolved: w.unresolved,
            notes: w.notes,
            warning,
            csv: csv_file,
        }))
    }

    #[tool(
        name = "books.add_customer",
        title = "Add a customer",
        description = "Add a customer record for accounts receivable (invoices). It links to the party of the same name that has no customer record yet (ignoring case), else to a new party; the output names both ids.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn add_customer(
        &self,
        params: Parameters<AddRecordParams>,
    ) -> Result<Json<AddRecordOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let id = store.add_customer(&p.name).map_err(|e| e.to_string())?;
        let party_id = store
            .ledger()
            .party_of_customer(id)
            .ok_or_else(|| format!("customer {id} has no party"))?;
        Ok(Json(AddRecordOut { id, party_id }))
    }

    #[tool(
        name = "books.add_vendor",
        title = "Add a vendor",
        description = "Add a vendor record for accounts payable (bills) and 1099s. It links to the party of the same name that has no vendor record yet (ignoring case), else to a new party; the output names both ids.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn add_vendor(
        &self,
        params: Parameters<AddRecordParams>,
    ) -> Result<Json<AddRecordOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let id = store.add_vendor(&p.name).map_err(|e| e.to_string())?;
        let party_id = store
            .ledger()
            .party_of_vendor(id)
            .ok_or_else(|| format!("vendor {id} has no party"))?;
        Ok(Json(AddRecordOut { id, party_id }))
    }

    #[tool(
        name = "books.add_party",
        title = "Add a party",
        description = "Add a party to the directory: anyone the company deals with (a contractor, landlord, payout platform, tax agency, government agency, bank or card issuer, member, employee). Name it as on its documents (unique among parties not archived); give its roles: contractor, employee, member, payout_platform, tax_agency, government_agency, financial_institution, landlord. Vendor and customer aren't given by hand: books.add_vendor and books.add_customer add those records and link them to the party of the same name. Never put a full TIN or SSN in any party field: the engine refuses nine-digit numbers. Journaled as party.added; returns the party.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn add_party(
        &self,
        params: Parameters<AddPartyParams>,
    ) -> Result<Json<PartyOut>, String> {
        let p = params.0;
        let mut roles = parse_roles(&p.roles)?;
        roles.sort();
        roles.dedup();
        let mut store = self.lock()?;
        let id = store
            .add_party(&p.name, &roles)
            .map_err(|e| e.to_string())?;
        let party = store.ledger().get_party(id).expect("just added");
        Ok(Json(party_out(store.ledger(), party)))
    }

    #[tool(
        name = "books.update_party",
        title = "Update a party",
        description = "Change a party's facts: name, display_name, website, address, notes, email, default_terms_as_customer, default_terms_as_vendor, archived, creditor_of. Give only the fields to change; \"\" clears display_name, website, address, notes, email or the terms. email and the terms are copied onto each invoice or bill at issue (A-13); terms (\"Net 30\") set a due date when the document gives none. creditor_of replaces the list of liabilities the company owes the party itself. Every field is checked before any is written; each is journaled as party.updated. Returns the party.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn update_party(
        &self,
        params: Parameters<UpdatePartyParams>,
    ) -> Result<Json<PartyOut>, String> {
        use ledger_core::PartyField as F;
        let p = params.0;
        let mut store = self.lock()?;
        let id = resolve_party(&store, &p.party)?;
        let mut fields = Vec::new();
        if let Some(n) = p.name {
            fields.push(F::Name(n));
        }
        if let Some(v) = p.display_name {
            fields.push(F::DisplayName(optional_text(&v)));
        }
        if let Some(v) = p.website {
            fields.push(F::Website(optional_text(&v)));
        }
        if let Some(v) = p.address {
            fields.push(F::Address(optional_text(&v)));
        }
        if let Some(v) = p.notes {
            fields.push(F::Notes(optional_text(&v)));
        }
        if let Some(v) = p.email {
            fields.push(F::Email(optional_text(&v)));
        }
        // "" clears the terms; any other text is trade terms this party
        // grants or is granted (B-39).
        let terms = |v: String| -> Result<Option<ledger_core::Terms>, String> {
            if v.trim().is_empty() {
                return Ok(None);
            }
            ledger_core::Terms::parse(&v)
                .map(Some)
                .map_err(|e| e.to_string())
        };
        if let Some(v) = p.default_terms_as_customer {
            fields.push(F::DefaultTermsAsCustomer(terms(v)?));
        }
        if let Some(v) = p.default_terms_as_vendor {
            fields.push(F::DefaultTermsAsVendor(terms(v)?));
        }
        if let Some(v) = p.phone {
            fields.push(F::Phone(optional_text(&v)));
        }
        if let Some(v) = p.contact {
            fields.push(F::Contact(optional_text(&v)));
        }
        if let Some(v) = p.billing_address {
            fields.push(F::BillingAddress(optional_text(&v)));
        }
        if let Some(v) = p.shipping_address {
            fields.push(F::ShippingAddress(optional_text(&v)));
        }
        if let Some(v) = p.billing_email {
            fields.push(F::BillingEmail(optional_text(&v)));
        }
        if let Some(a) = p.archived {
            fields.push(F::Archived(a));
        }
        // creditor_of replaces the list: each account added or dropped is
        // one change.
        let mut creditor: Vec<(AccountId, bool)> = Vec::new();
        if let Some(codes) = &p.creditor_of {
            let want: std::collections::BTreeSet<AccountId> = codes
                .iter()
                .map(|c| resolve_account(&store, c.trim()))
                .collect::<Result<_, _>>()?;
            let has = &store.ledger().get_party(id).expect("resolved").creditor_of;
            creditor.extend(want.difference(has).map(|a| (*a, true)));
            creditor.extend(has.difference(&want).map(|a| (*a, false)));
        }
        if fields.is_empty() && p.creditor_of.is_none() {
            return Err(
                "give at least one field to change: name, display_name, website, address, notes, email, default_terms_as_customer, default_terms_as_vendor, phone, contact, billing_address, shipping_address, billing_email, archived, creditor_of"
                    .into(),
            );
        }
        // Every change, in order, before any is written (INVARIANT 2).
        let mut rehearsal = store.ledger().clone();
        for f in &fields {
            rehearsal
                .update_party(id, f.clone())
                .map_err(|e| e.to_string())?;
        }
        for (a, on) in &creditor {
            rehearsal
                .set_party_creditor(id, *a, *on)
                .map_err(|e| e.to_string())?;
        }
        // The journal features these lines need, asked for before this
        // command's first line, never between its lines (rules.md B-27:
        // books waiting for their format upgrade must not be left with the
        // name changed and the rest not).
        if fields.iter().any(|f| f.needs_documents_v2()) {
            store
                .ensure_feature(ledger_store::format::DOCUMENTS_V2)
                .map_err(|e| e.to_string())?;
        }
        if !creditor.is_empty() {
            store
                .ensure_feature(ledger_store::format::PARTY_CREDITORS)
                .map_err(|e| e.to_string())?;
        }
        for f in fields {
            store.update_party(id, f).map_err(|e| e.to_string())?;
        }
        for (a, on) in creditor {
            store
                .set_party_creditor(id, a, on)
                .map_err(|e| e.to_string())?;
        }
        let party = store.ledger().get_party(id).expect("resolved");
        Ok(Json(party_out(store.ledger(), party)))
    }

    #[tool(
        name = "books.set_party_roles",
        title = "Set a party's roles",
        description = "Give a party roles (add) or take them away (remove): contractor, employee, member, payout_platform, tax_agency, government_agency, financial_institution, landlord. Vendor and customer follow their records and can't be set by hand. Adds apply before removes; a role it already has (or lacks) is left alone. Removing a party's last 1099 role (contractor, landlord) needs its W-9 facts cleared first (books.set_w9 clear), and removing employee its W-4 facts. All or nothing: a refusal changes nothing. Each change is journaled as party.role_set. Returns the party.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn set_party_roles(
        &self,
        params: Parameters<SetPartyRolesParams>,
    ) -> Result<Json<PartyOut>, String> {
        let p = params.0;
        let add = parse_roles(&p.add)?;
        let remove = parse_roles(&p.remove)?;
        if add.is_empty() && remove.is_empty() {
            return Err("give at least one role to add or remove".into());
        }
        if let Some(r) = add.iter().find(|r| remove.contains(r)) {
            return Err(format!("role {} is both added and removed", r.key()));
        }
        if add
            .iter()
            .chain(&remove)
            .any(|r| matches!(r, ledger_core::Role::Vendor | ledger_core::Role::Customer))
        {
            return Err(
                "vendor and customer roles follow their records: books.add_vendor or books.add_customer"
                    .into(),
            );
        }
        let mut store = self.lock()?;
        let id = resolve_party(&store, &p.party)?;
        let has = store
            .ledger()
            .get_party(id)
            .expect("resolved")
            .roles
            .clone();
        let mut changes: Vec<(ledger_core::Role, bool)> = Vec::new();
        for r in add {
            if !has.contains(&r) && !changes.contains(&(r, true)) {
                changes.push((r, true));
            }
        }
        for r in remove {
            if has.contains(&r) && !changes.contains(&(r, false)) {
                changes.push((r, false));
            }
        }
        // The whole change, in order, before any is written (INVARIANT 2).
        let mut rehearsal = store.ledger().clone();
        for (r, on) in &changes {
            rehearsal
                .set_party_role(id, *r, *on)
                .map_err(|e| e.to_string())?;
        }
        for (r, on) in changes {
            store.set_party_role(id, r, on).map_err(|e| e.to_string())?;
        }
        let party = store.ledger().get_party(id).expect("resolved");
        Ok(Json(party_out(store.ledger(), party)))
    }

    /// ```text
    /// GOAL: the books know a W-9 arrived and what it said, and the full
    ///       TIN (and the form, which shows it) is kept where it can be
    ///       handed to the CPA and purged, never in the journal.
    /// SOURCE: docs/parties-and-attachments.md "The TIN vault"; B-26;
    ///         Treas. Reg. §301.6109-4 (truncated TINs); DECISION (owner,
    ///         2026-10-15): full TINs only in the encrypted vault.
    /// INVARIANTS:
    ///   1. A full TIN goes only to the vault (`vault/` in the book's own
    ///      folder, its key in `keys/` beside it, created on first use);
    ///      the facts journaled (`party.w9_set`) carry its last four and
    ///      the vault record's hash. The output never holds more than the
    ///      last four.
    ///   2. The W-9 file goes only to the vault, never the attachment store.
    ///   3. The facts are checked (on a copy of the ledger) before the vault
    ///      is written; a refusal journals nothing and writes no record.
    ///   4. The facts never disagree with the vault: a TIN in the vault has
    ///      the facts' last four.
    /// SCENARIOS:
    ///   1. IF tin is given: SHOULD validate it (Tin::new, with tin_type),
    ///      take tin_last4 from it (refusing a given one that differs), and
    ///      default kept_at to "wanderbooks vault".
    ///   2. IF no tin is given but the vault holds one whose last four
    ///      differ: SHOULD refuse (pass the new TIN, or books.purge_tin).
    ///   3. IF clearing while the vault holds a record: SHOULD refuse
    ///      (books.purge_tin first).
    /// ```
    #[tool(
        name = "books.set_w9",
        title = "Record W-9 facts",
        description = "Record what a contractor's or landlord's W-9 says, or clear it (clear: true alone): the date received (received_on), line 3 entity type, line 4 exempt payee and FATCA codes, the TIN's last four digits, and where the form is kept. Give the full TIN as tin with tin_type (ssn, ein, itin) and it goes only to the encrypted TIN vault (its key kept apart from it, created on first use): the books keep its last four (tin_last4 is then optional, and refused if it disagrees) and a hash of the vault record, in the W-9 facts and the party's TIN facts alike (party.tin_set, as books.set_tin), and kept_at defaults to \"wanderbooks vault\". The CPA package carries the full TIN to the CPA. Give the signed form (a PDF, PNG, or JPEG) as w9_upload_id (books.begin_upload purpose w9, then `wanderbooks upload`: a file never passes through you), and it goes to the vault too, never to the receipts and documents; read it back with books.get_w9. Never put a TIN in any other field, note, description, or file: the engine refuses any nine-digit number, and no tool output echoes one. The party needs a 1099 role (contractor or landlord: books.set_party_roles). Record it before paying them. A party with a vendor record also gets its W-9 marked on file there (books.vendor_1099 missing_w9). Clearing W-9 facts whose TIN is in the vault needs books.purge_tin first. Journaled as party.w9_set. Returns the party (the TIN's last four only).",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn set_w9(&self, params: Parameters<SetW9Params>) -> Result<Json<PartyOut>, String> {
        let p = params.0;
        let w9_given = p.w9_upload_id.is_some();
        if p.w9_file_name.is_some() && !w9_given {
            return Err("w9_file_name names the W-9 given with it (w9_upload_id)".into());
        }
        let vault_input = p.tin.is_some() || p.tin_type.is_some() || w9_given;
        let tin = match (&p.tin, &p.tin_type) {
            (Some(t), Some(k)) => Some(
                ledger_vault::Tin::new(t.0.trim(), parse_tin_type(k)?)
                    .map_err(|e| e.to_string())?,
            ),
            (Some(_), None) => return Err("tin needs tin_type: ssn, ein, or itin".into()),
            (None, Some(_)) => return Err("tin_type goes with tin".into()),
            (None, None) => None,
        };
        let facts = if p.clear {
            if p.received_on.is_some()
                || p.entity_type.is_some()
                || p.exempt_payee_code.is_some()
                || p.fatca_code.is_some()
                || p.tin_last4.is_some()
                || p.kept_at.is_some()
                || vault_input
            {
                return Err("clear: true takes no other W-9 fact".into());
            }
            None
        } else {
            let need = |v: &Option<String>, name: &str| {
                v.as_deref()
                    .map(|s| s.trim().to_string())
                    .ok_or_else(|| format!("{name} is required (or pass clear: true)"))
            };
            let tin_last4 = match (&tin, p.tin_last4.as_deref().map(str::trim)) {
                (Some(t), Some(given)) if given != t.last4() => {
                    return Err(
                        "tin_last4 doesn't match the TIN's last four digits (leave it out: it comes from tin)"
                            .into(),
                    );
                }
                (Some(t), _) => t.last4().to_string(),
                (None, _) => need(&p.tin_last4, "tin_last4")?,
            };
            let kept_at = match &p.kept_at {
                Some(k) => k.trim().to_string(),
                None if vault_input => "wanderbooks vault".to_string(),
                None => need(&p.kept_at, "kept_at")?,
            };
            Some(ledger_core::W9Facts {
                received: parse_date(&need(&p.received_on, "received_on")?)?,
                entity_type: need(&p.entity_type, "entity_type")?,
                exempt_payee_code: p.exempt_payee_code.as_deref().and_then(optional_text),
                fatca_code: p.fatca_code.as_deref().and_then(optional_text),
                tin_last4,
                kept_at,
                vault_record: None,
            })
        };
        let (w9_file, mut w9_claim) = if let Some(upload_id) = &p.w9_upload_id {
            let (file, claim) = self.w9_file(&p, upload_id)?;
            (Some(file), claim)
        } else {
            (None, None)
        };
        let mut store = self.lock()?;
        let id = resolve_party(&store, &p.party)?;
        let existing = store.ledger().get_party(id).and_then(|p| p.w9.clone());
        let party_tin = store.ledger().get_party(id).and_then(|p| p.tin.clone());
        // A new TIN with another last four replaces the party's TIN facts:
        // cleared first, so the two never disagree on the record (B-26).
        let replace_tin = tin
            .as_ref()
            .is_some_and(|t| party_tin.as_ref().is_some_and(|pt| pt.last4 != t.last4()));
        let mut tin_facts: Option<ledger_core::TinFacts> = None;
        let vault_dir = self.vault_dir(&store);
        let (held_tin, held_w9) =
            ledger_vault::records_held(&vault_dir, id).map_err(|e| e.to_string())?;
        let facts = match facts {
            None => {
                if held_tin
                    || held_w9
                    || existing.as_ref().is_some_and(|f| f.vault_record.is_some())
                {
                    return Err(format!(
                        "the TIN vault holds party {id}'s TIN or W-9: purge it first (books.purge_tin, with the reason), then clear"
                    ));
                }
                None
            }
            Some(mut f) => {
                // The facts must agree with a TIN the vault keeps
                // (INVARIANT 4).
                if tin.is_none() && held_tin {
                    let held = self
                        .open_vault(&store, false)?
                        .tin(id)
                        .map_err(|e| e.to_string())?;
                    if let Some(t) = held
                        && t.last4() != f.tin_last4
                    {
                        return Err(format!(
                            "the TIN vault holds a TIN ending in {} for party {id}: pass the new tin (with tin_type), or purge the old one first (books.purge_tin)",
                            t.last4()
                        ));
                    }
                }
                // Rehearse with a stand-in hash before writing the vault
                // (INVARIANT 3).
                let mut rehearsal = store.ledger().clone();
                f.vault_record = Some("0".repeat(64));
                let stand_in = tin.as_ref().map(|t| ledger_core::TinFacts {
                    tin_type: tin_kind(t.kind),
                    last4: t.last4().to_string(),
                    vault_record: Some("0".repeat(64)),
                    purged: None,
                });
                if replace_tin {
                    rehearsal
                        .set_party_tin(id, None)
                        .map_err(|e| e.to_string())?;
                }
                rehearsal
                    .set_party_w9(id, Some(f.clone()))
                    .map_err(|e| e.to_string())?;
                if stand_in.is_some() {
                    rehearsal
                        .set_party_tin(id, stand_in.clone())
                        .map_err(|e| e.to_string())?;
                }
                f.vault_record = if tin.is_some() || w9_file.is_some() {
                    let vault = self.open_vault(&store, true)?;
                    // The W-9 read is recorded before the vault keeps it:
                    // never filed into the books (tin_files.rs INVARIANT
                    // 1). Only now: every check has passed.
                    if let Some(w) = &w9_file {
                        self.wrote_tins(&store, &w.bytes)?;
                    }
                    let tin_hash = match &tin {
                        Some(t) => Some(vault.put_tin(id, t).map_err(|e| e.to_string())?),
                        None => None,
                    };
                    let w9_hash = match &w9_file {
                        Some(w) => Some(vault.put_w9(id, w).map_err(|e| e.to_string())?),
                        None => None,
                    };
                    // The W-9 is in the vault: its upload is used up.
                    files::filed(w9_claim.take());
                    match tin_hash {
                        Some(h) => Some(h),
                        None if held_tin => vault.record_hash(id).map_err(|e| e.to_string())?,
                        None => w9_hash,
                    }
                } else if held_tin {
                    ledger_vault::Vault::open(&vault_dir, &self.key_file(&store), false)
                        .ok()
                        .and_then(|v| v.record_hash(id).ok().flatten())
                        .or_else(|| existing.as_ref().and_then(|e| e.vault_record.clone()))
                } else if held_w9 {
                    existing.as_ref().and_then(|e| e.vault_record.clone())
                } else {
                    None
                };
                // The TIN's facts carry the same record as the W-9's.
                tin_facts = stand_in.map(|t| ledger_core::TinFacts {
                    vault_record: f.vault_record.clone(),
                    ..t
                });
                Some(f)
            }
        };
        let on_file = facts.is_some();
        // The vendor record's W-9 flag follows (below): its feature is
        // asked for before this command's first line, never after it
        // (B-27: books waiting for their format upgrade must not be left
        // with the W-9 recorded and the flag not).
        let vendor = store.ledger().get_party(id).and_then(|p| p.vendor);
        if vendor
            .and_then(|v| store.ledger().get_vendor(v))
            .is_some_and(|v| v.tin_on_file != on_file)
        {
            store
                .ensure_feature(ledger_store::format::VENDOR_1099_DECISIONS)
                .map_err(|e| e.to_string())?;
        }
        if replace_tin {
            store.set_party_tin(id, None).map_err(|e| e.to_string())?;
        }
        store.set_party_w9(id, facts).map_err(|e| e.to_string())?;
        if tin_facts.is_some() {
            store
                .set_party_tin(id, tin_facts)
                .map_err(|e| e.to_string())?;
        }
        // Keep the vendor record's W-9 flag (the 1099 worksheet's) in
        // step, its 1099 decision and who made it kept (P3 review R8).
        if let Some(v) = vendor {
            store
                .set_vendor_w9_flag(v, on_file)
                .map_err(|e| e.to_string())?;
        }
        let party = store.ledger().get_party(id).expect("resolved");
        Ok(Json(party_out(store.ledger(), party)))
    }

    /// ```text
    /// GOAL: any party's full TIN (a member's SSN for their K-1, a payee's
    ///       from their W-9) kept where the CPA package can carry it, and
    ///       nowhere else.
    /// SOURCE: docs/parties-and-attachments.md "The TIN vault"; B-26;
    ///         DECISION (owner, 2026-10-15): "In the vault, that's the
    ///         WHOLE reason we built it. And it needs to be in the damn CPA
    ///         package."
    /// INVARIANTS:
    ///   1. The full TIN goes only to the vault (set_w9's path: key outside
    ///      the books, created on first use; books in git must ignore
    ///      vault/); `party.tin_set` journals its type, last four, and the
    ///      record's hash. The output never holds more than the last four.
    ///   2. Any role.
    ///   3. The facts are checked (on a copy of the ledger) before the vault
    ///      is written; a refusal journals nothing and writes no record.
    ///   4. W-9 facts on file must have the same last four (else refused:
    ///      a payee's new TIN comes with its W-9, books.set_w9); when they
    ///      do, their vault record follows the new record (`party.w9_set`).
    /// ```
    #[tool(
        name = "books.set_tin",
        title = "Store a TIN in the vault",
        description = "Put a party's full taxpayer identification number in the encrypted TIN vault: tin (nine digits, hyphens allowed) and tin_type (ssn, ein, itin). Any role: collect every member's (their K-1 needs it) and every 1099 payee's (books.set_w9 tin records a W-9's TIN the same way). The books keep only its type, last four, and a hash of the vault record (journaled as party.tin_set); books.export_cpa_package carries the full number to the CPA. A party whose W-9 facts name another last four is refused: a payee's new TIN comes with its new W-9 (books.set_w9). Never repeat a TIN in chat, descriptions, notes, version-history messages, or file names. Returns the party (the last four only).",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn set_tin(&self, params: Parameters<SetTinParams>) -> Result<Json<PartyOut>, String> {
        let p = params.0;
        let tin = ledger_vault::Tin::new(p.tin.0.trim(), parse_tin_type(&p.tin_type)?)
            .map_err(|e| e.to_string())?;
        let mut store = self.lock()?;
        let id = resolve_party(&store, &p.party)?;
        let w9 = store.ledger().get_party(id).and_then(|p| p.w9.clone());
        let stand_in = ledger_core::TinFacts {
            tin_type: tin_kind(tin.kind),
            last4: tin.last4().to_string(),
            vault_record: Some("0".repeat(64)),
            purged: None,
        };
        // Rehearse on a copy before the vault is written (INVARIANT 3).
        let mut rehearsal = store.ledger().clone();
        rehearsal
            .set_party_tin(id, Some(stand_in.clone()))
            .map_err(|e| e.to_string())?;
        if let Some(f) = &w9 {
            rehearsal
                .set_party_w9(
                    id,
                    Some(ledger_core::W9Facts {
                        vault_record: Some("0".repeat(64)),
                        ..f.clone()
                    }),
                )
                .map_err(|e| e.to_string())?;
        }
        let vault = self.open_vault(&store, true)?;
        let hash = vault.put_tin(id, &tin).map_err(|e| e.to_string())?;
        store
            .set_party_tin(
                id,
                Some(ledger_core::TinFacts {
                    vault_record: Some(hash.clone()),
                    ..stand_in
                }),
            )
            .map_err(|e| e.to_string())?;
        if let Some(f) = w9
            && f.vault_record.as_deref() != Some(hash.as_str())
        {
            store
                .set_party_w9(
                    id,
                    Some(ledger_core::W9Facts {
                        vault_record: Some(hash),
                        ..f
                    }),
                )
                .map_err(|e| e.to_string())?;
        }
        let party = store.ledger().get_party(id).expect("resolved");
        Ok(Json(party_out(store.ledger(), party)))
    }

    /// ```text
    /// GOAL: hand the human (for the CPA) the signed W-9 the vault keeps,
    ///       without it passing through the books or the conversation.
    /// SOURCE: docs/parties-and-attachments.md "The TIN vault"; D-29.
    /// INVARIANTS:
    ///   1. The file is handed over (tools/files.rs INVARIANT 6: an export
    ///      for the caller's token), never written to a path; a name kept
    ///      before names were checked is made plain, never refused. It is
    ///      recorded (tin_files.rs) first, so it is never filed back into
    ///      the books.
    ///   2. Nothing is journaled.
    /// SCENARIOS:
    ///   1. IF the vault is locked or holds no W-9 file for the party:
    ///      SHOULD refuse, saying which.
    /// ```
    #[tool(
        name = "books.get_w9",
        title = "Write out a W-9",
        description = "A party's signed W-9 file from the TIN vault, for the CPA or the human: run the download command it returns to save it in a folder. Never file it into the books (it shows the full TIN). Nothing is journaled.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn get_w9(&self, params: Parameters<GetW9Params>) -> Result<Json<GetW9Out>, String> {
        let p = params.0;
        let store = self.lock()?;
        let id = resolve_party(&store, &p.party)?;
        let file = self
            .open_vault(&store, false)?
            .w9(id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| {
                format!(
                    "the TIN vault holds no W-9 file for party {id}: store one with books.set_w9 w9_upload_id"
                )
            })?;
        self.wrote_tins(&store, &file.bytes)?;
        let handed = self.hand_over(
            &book_name(&store),
            &file.file_name,
            &file.mime_type,
            file.bytes,
        )?;
        Ok(Json(GetW9Out {
            party_id: id,
            file: handed,
        }))
    }

    /// ```text
    /// GOAL: a TIN kept no longer than needed can be destroyed, on the
    ///       record.
    /// SOURCE: docs/parties-and-attachments.md "The TIN vault" ("Purge");
    ///         IRS Pub. 583 (keep records while needed; then dispose).
    /// INVARIANTS:
    ///   1. Without confirmed=true: a preview, changing nothing.
    ///   2. Confirmed: the party's vault records (TIN and W-9 file) are
    ///      deleted, needing no key; when the party has TIN facts,
    ///      `party.tin_set` journals them with the vault record cleared
    ///      and `purged` saying when and why, and when it has W-9 facts,
    ///      `party.w9_set` likewise in kept_at (the last four stay).
    ///   3. The new facts are checked before anything is deleted; a refusal
    ///      changes nothing.
    /// SCENARIOS:
    ///   1. IF the reason is blank: SHOULD refuse.
    ///   2. IF the vault holds nothing for the party and its facts name no
    ///      vault record: SHOULD refuse (nothing to purge).
    /// ```
    #[tool(
        name = "books.purge_tin",
        title = "Purge a TIN from the vault",
        description = "Delete a party's full TIN and W-9 file from the TIN vault (after the retention period, at the payee's request, or to replace a wrong one), with the reason. Without confirmed=true it previews what the vault holds and changes nothing. Confirmed, the records are deleted from the vault (backups made before keep their copies) and the books journal it: the party's TIN facts (party.tin_set: purged says when and why) and W-9 facts (party.w9_set: kept_at says so) keep the last four and drop the vault record. Works without the vault key. The next CPA package then lacks that TIN (and says so).",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn purge_tin(
        &self,
        params: Parameters<PurgeTinParams>,
    ) -> Result<Json<PurgeTinOut>, String> {
        let p = params.0;
        let reason = p.reason.trim();
        if reason.is_empty() {
            return Err("give the reason for purging".into());
        }
        let mut store = self.lock()?;
        let id = resolve_party(&store, &p.party)?;
        let party = store.ledger().get_party(id).expect("resolved").clone();
        let vault_dir = self.vault_dir(&store);
        let (held_tin, held_w9) =
            ledger_vault::records_held(&vault_dir, id).map_err(|e| e.to_string())?;
        let recorded = party.w9.as_ref().is_some_and(|f| f.vault_record.is_some())
            || party.tin.as_ref().is_some_and(|t| t.vault_record.is_some());
        if !held_tin && !held_w9 && !recorded {
            return Err(format!(
                "the TIN vault holds nothing for party {id} ({}) and its W-9 and TIN facts name no vault record",
                party.name
            ));
        }
        let why = format!("purged from the wanderbooks vault on {}: {reason}", today());
        let new_facts = party.w9.clone().map(|f| ledger_core::W9Facts {
            vault_record: None,
            kept_at: format!("not kept: {why}"),
            ..f
        });
        let new_tin = party.tin.clone().map(|t| ledger_core::TinFacts {
            vault_record: None,
            purged: Some(why.clone()),
            ..t
        });
        // Both checked before anything is deleted (INVARIANT 3).
        let mut rehearsal = store.ledger().clone();
        if let Some(f) = &new_facts {
            rehearsal
                .set_party_w9(id, Some(f.clone()))
                .map_err(|e| e.to_string())?;
        }
        if let Some(t) = &new_tin {
            rehearsal
                .set_party_tin(id, Some(t.clone()))
                .map_err(|e| e.to_string())?;
        }
        let mut out = PurgeTinOut {
            status: "preview".into(),
            party_id: id,
            name: party.name.clone(),
            tin_in_vault: held_tin,
            w9_file_in_vault: held_w9,
            tin_last4: party.tin_last4().map(str::to_string),
            journaled: false,
            note: format!(
                "Confirming deletes {}{}; backups made before keep their copies. Make sure the CPA has what they need first.",
                match (held_tin, held_w9) {
                    (true, true) => "the TIN and the W-9 file",
                    (true, false) => "the TIN",
                    (false, true) => "the W-9 file",
                    (false, false) => "nothing in the vault (the books' record of it is cleared)",
                },
                match (new_facts.is_some(), new_tin.is_some()) {
                    (true, true) =>
                        " and journals the purge in the TIN and W-9 facts (party.tin_set, party.w9_set; the last four stay)",
                    (false, true) =>
                        " and journals the purge in the TIN facts (party.tin_set; the last four stay)",
                    (true, false) =>
                        " and journals the purge in the W-9 facts (party.w9_set; the last four stay)",
                    (false, false) => "",
                }
            ),
        };
        if !p.confirmed.unwrap_or(false) {
            return Ok(Json(out));
        }
        ledger_vault::purge_records(&vault_dir, id).map_err(|e| e.to_string())?;
        if let Some(f) = new_facts {
            store.set_party_w9(id, Some(f)).map_err(|e| e.to_string())?;
            out.journaled = true;
        }
        if let Some(t) = new_tin {
            store
                .set_party_tin(id, Some(t))
                .map_err(|e| e.to_string())?;
            out.journaled = true;
        }
        out.status = "purged".into();
        out.note = format!(
            "Purged from the vault{}.",
            if out.journaled {
                "; the books record when and why"
            } else {
                ""
            }
        );
        Ok(Json(out))
    }

    #[tool(
        name = "books.set_w4",
        title = "Record W-4 facts",
        description = "Record what an employee's W-4 says, or clear it (clear: true alone): the date received (received_on), the work state (two letters), and where the form is kept. Never the SSN. The party needs the employee role. Record it before their first payroll. Journaled as party.w4_set. Returns the party.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn set_w4(&self, params: Parameters<SetW4Params>) -> Result<Json<PartyOut>, String> {
        let p = params.0;
        let facts = if p.clear {
            if p.received_on.is_some() || p.state.is_some() || p.kept_at.is_some() {
                return Err("clear: true takes no other W-4 fact".into());
            }
            None
        } else {
            let need = |v: &Option<String>, name: &str| {
                v.as_deref()
                    .map(|s| s.trim().to_string())
                    .ok_or_else(|| format!("{name} is required (or pass clear: true)"))
            };
            Some(ledger_core::W4Facts {
                received: parse_date(&need(&p.received_on, "received_on")?)?,
                state: need(&p.state, "state")?,
                kept_at: need(&p.kept_at, "kept_at")?,
            })
        };
        let mut store = self.lock()?;
        let id = resolve_party(&store, &p.party)?;
        store.set_party_w4(id, facts).map_err(|e| e.to_string())?;
        let party = store.ledger().get_party(id).expect("resolved");
        Ok(Json(party_out(store.ledger(), party)))
    }

    #[tool(
        name = "books.parties",
        title = "List parties",
        description = described(
            "The party directory, by id: each party's name, label, roles (vendor and customer included), vendor and customer record ids, whether W-9 and W-4 facts are on file, the TIN's last four when the books know it (TIN or W-9 facts), archived, and what was paid to and received from it in the year on the basis asked: cash by default (money), or all (adds property contributed or distributed in kind); defined as in books.party, the basis echoed. Filter by role; archived parties only with include_archived.",
            ledger_core::calendar::PeriodKind::CalendarYear
        ),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn parties(
        &self,
        params: Parameters<ListPartiesParams>,
    ) -> Result<Json<ListPartiesOut>, String> {
        let p = params.0;
        let role = p
            .role
            .as_deref()
            .map(|r| parse_roles(&[r.to_string()]).map(|v| v[0]))
            .transpose()?;
        let year = p.year.unwrap_or_else(|| self.periods().calendar_year());
        let basis = parse_basis(p.basis.as_deref())?;
        let settings = self.settings();
        let store = self.lock()?;
        let mut parties = Vec::new();
        for party in store.ledger().parties() {
            if party.archived && !p.include_archived {
                continue;
            }
            if role.is_some_and(|r| !party.all_roles().contains(&r)) {
                continue;
            }
            let act = party_activity(&store, party, &settings, year, basis)?;
            parties.push(PartySummaryOut {
                party_id: party.id,
                name: party.name.clone(),
                label: party.label().to_string(),
                roles: party_roles(party),
                vendor_id: party.vendor,
                customer_id: party.customer,
                w9_on_file: party.w9.is_some(),
                tin_last4: party.tin_last4().map(str::to_string),
                w4_on_file: party.w4.is_some(),
                archived: party.archived,
                paid: money(act.paid),
                received: money(act.received),
            });
        }
        Ok(Json(ListPartiesOut {
            year,
            basis: basis.key().to_string(),
            parties,
        }))
    }

    #[tool(
        name = "books.party",
        title = "Get a party",
        description = described(
            "One party, by id or name: its facts and roles, vendor and customer record ids, the year's totals (paid to it and received from it on the basis asked, cash by default or all with property in kind, the basis echoed; invoiced, billed, the 1099 amount for a vendor, payroll gross and net), what it is owed and owes now (open bills and invoices), every entry it is on (tagged with it or its vendor record, its invoices and their payments, and their reversals), its invoices and bills, its projects, and the payroll lines naming it. Totals come from the journal: the output's schema says exactly how.",
            ledger_core::calendar::PeriodKind::CalendarYear
        ),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn party(&self, params: Parameters<PartyParams>) -> Result<Json<PartyDetailOut>, String> {
        let p = params.0;
        let year = p.year.unwrap_or_else(|| self.periods().calendar_year());
        let basis = parse_basis(p.basis.as_deref())?;
        let settings = self.settings();
        let store = self.lock()?;
        let ledger = store.ledger();
        let id = resolve_party(&store, &p.party)?;
        let party = ledger.get_party(id).expect("resolved");
        let act = party_activity(&store, party, &settings, year, basis)?;
        let in_year = |d: NaiveDate| chrono::Datelike::year(&d) == year;
        let mut entries: Vec<PartyEntryOut> = act
            .entries
            .iter()
            .filter_map(|&e| ledger.get_entry(e))
            .map(|e| PartyEntryOut {
                entry_id: e.id(),
                date: e.date().to_string(),
                description: e.description().to_string(),
                status: match e.status() {
                    ledger_core::EntryStatus::Draft => "draft",
                    ledger_core::EntryStatus::Posted => "posted",
                }
                .to_string(),
                total: ReceiptPolicy::entry_debits(e).ok().map(money),
                reversal_of: e.reversal_of(),
            })
            .collect();
        entries.sort_by(|a, b| b.date.cmp(&a.date).then(b.entry_id.cmp(&a.entry_id)));
        let invoices = party_invoices(ledger, party);
        let bills = party_bills(ledger, party);
        // The worksheet's figure; none when it can't be made (no cash
        // accounts named), never a failed answer.
        let amount_1099 = party.vendor.and_then(|v| {
            let r = vendor_1099_report(ledger, &self.settings(), year, Some(Decimal::ZERO)).ok()?;
            Some(money(
                r.vendors
                    .iter()
                    .find(|l| l.vendor_id == v)
                    .map(|l| l.paid)
                    .unwrap_or_default(),
            ))
        });
        let decided_now = party
            .vendor
            .and_then(|v| ledger.get_vendor(v))
            .map(|v| v.decision_on(today()).cloned().unwrap_or_default());
        // Documents and runs are restored as journaled (rules.md B-28): a
        // total past exact arithmetic refuses the answer, never a panic.
        let mut payroll = Vec::new();
        let (mut gross, mut net) = (Decimal::ZERO, Decimal::ZERO);
        for run in ledger.list_payroll_runs() {
            for line in run.lines.iter().filter(|l| l.employee == Some(id)) {
                let line_net = line.net_pay().map_err(|e| e.to_string())?;
                if in_year(run.date) {
                    gross = checked_sum([Ok(gross), Ok(line.gross_pay)], "gross pay")?;
                    net = checked_sum([Ok(net), Ok(line_net)], "net pay")?;
                }
                payroll.push(PartyPayrollOut {
                    run_id: run.id,
                    date: run.date.to_string(),
                    entry_id: run.entry_id,
                    gross_pay: money(line.gross_pay),
                    net_pay: money(line_net),
                });
            }
        }
        let totals = PartyTotalsOut {
            basis: basis.key().to_string(),
            paid: money(act.paid),
            received: money(act.received),
            invoiced: money(checked_sum(
                invoices
                    .iter()
                    .filter(|i| in_year(i.date))
                    .map(|i| i.total()),
                "the year's invoices",
            )?),
            billed: money(checked_sum(
                bills.iter().filter(|b| in_year(b.date)).map(|b| b.total()),
                "the year's bills",
            )?),
            owed: money(checked_sum(
                bills.iter().map(|b| b.balance()),
                "what is owed",
            )?),
            receivable: money(checked_sum(
                invoices.iter().map(|i| i.balance()),
                "what is receivable",
            )?),
            amount_1099,
            form_1099: form_1099_label(decided_now.as_ref().and_then(|d| d.form())),
            decision_1099: decided_now.as_ref().map(|d| d.key().to_string()),
            gross_pay: money(gross),
            net_pay: money(net),
        };
        Ok(Json(PartyDetailOut {
            party: party_out(ledger, party),
            year,
            totals,
            entries,
            invoices: invoices
                .iter()
                .map(|i| {
                    party_document(
                        i.id,
                        (i.date, i.due_date),
                        (i.total(), i.balance(), i.amount_paid),
                        i.status,
                        i.entry_id,
                    )
                })
                .collect::<Result<_, _>>()?,
            bills: bills
                .iter()
                .map(|b| {
                    party_document(
                        b.id,
                        (b.date, b.due_date),
                        (b.total(), b.balance(), b.amount_paid),
                        b.status,
                        b.entry_id,
                    )
                })
                .collect::<Result<_, _>>()?,
            payroll,
            projects: ledger
                .projects_of_party(id)
                .into_iter()
                .map(|c| projects::project_ref(ledger, c))
                .collect(),
        }))
    }

    #[tool(
        name = "books.set_entry_party",
        title = "Tag an entry's parties",
        description = "Tag an entry (draft or posted) with the party it was with (id or name), or pass party null (or omit it) to clear the tag. A party with a vendor record also tags the entry's vendor, so a payment to a contractor or landlord counts in books.vendor_1099; a party without one, or clearing, clears the vendor tag. For an entry with several counterparties pass parties instead: [{party, amount?|percent?}], each share an exact amount (cents), a percent (summing to 100; leftover cents to the first party), or neither (derived: a sole party gets all; members named together get what their capital or distributions lines say, else their [[company.members]] share; once the entry is posted, a derived split among several parties is recorded as the exact amounts it resolves to, so a later change to [[company.members]] never re-splits posted history). Name both members on a joint member transaction (a reimbursement on 2100, a joint contribution) and let the shares derive; give amounts when a split is uneven. Refused: a party twice, mixed kinds of share, two parties with vendor records, and, once the entry has balanced lines, shares that don't add up to what it moved. Metadata only: amounts never change. An entry and its reversal carry the same parties: tagging either tags both, so each party's part nets to zero. Journaled as entry.party_set (one party) or entry.parties_set (parties). Returns the entry (as books.get_entry), each party's share resolved, with its direction and what of it was money or property in kind.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn set_entry_party(
        &self,
        params: Parameters<SetEntryPartyParams>,
    ) -> Result<Json<GetEntryOut>, String> {
        let p = params.0;
        {
            let settings = self.settings();
            let mut store = self.lock()?;
            let party = p
                .party
                .as_deref()
                .map(|r| resolve_party(&store, r))
                .transpose()?;
            let shares = p
                .parties
                .as_deref()
                .map(|ps| parse_shares(&store, ps))
                .transpose()?;
            let reason = p
                .no_party_reason
                .as_deref()
                .map(str::trim)
                .filter(|r| !r.is_empty());
            match (party, reason, shares) {
                (Some(_), _, Some(_)) => {
                    return Err(
                        "pass party (one) or parties (several, with shares), not both".into(),
                    );
                }
                (_, Some(_), Some(_)) => {
                    return Err("pass parties or a no_party_reason, not both".into());
                }
                (Some(_), Some(_), None) => {
                    return Err("pass a party or a no_party_reason, not both".into());
                }
                (None, Some(r), _) => {
                    if !store.ledger().entry_parties(p.entry_id).is_empty() {
                        // Check the mark before clearing, so a refusal
                        // journals nothing.
                        if r.trim().is_empty() {
                            return Err("say why the entry has no counterparty".into());
                        }
                        ledger_core::parties::check_text("reason", r)?;
                        store
                            .set_entry_party(p.entry_id, None)
                            .map_err(|e| e.to_string())?;
                    }
                    store
                        .set_entry_no_party(p.entry_id, r)
                        .map_err(|e| e.to_string())?;
                }
                (None, None, Some(sh)) => {
                    store
                        .ledger()
                        .check_set_entry_parties(p.entry_id, &sh)
                        .map_err(|e| e.to_string())?;
                    check_shares_resolve(&store, &settings, p.entry_id, &sh)?;
                    store
                        .set_entry_parties(p.entry_id, &sh)
                        .map_err(|e| e.to_string())?;
                    fix_derived_shares(&mut store, &settings, p.entry_id)?;
                }
                (party, None, None) => {
                    store
                        .set_entry_party(p.entry_id, party)
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        self.get_entry(Parameters(GetEntryParams {
            entry_id: p.entry_id,
        }))
        .await
    }
}
