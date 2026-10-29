//! Journal entries through the server: `books.draft_entry`,
//! `books.add_line`, `books.post_entry`, `books.reverse_entry`,
//! `books.get_entry`, `books.search_entries`, and the bank CSV import.
//! Split whole out of `server.rs` (plan/served.md 8.2 U0a): nothing about
//! the tools changed.

use super::*;

/// Likely counterparties for an entry, best first (at most three): parties
/// named in its description, then in its receipts' names and
/// descriptions, then the party of an earlier entry with the same
/// description. Names match as whole words, ignoring case; a party also
/// matches by the first word of its name when that word has 4+ letters
/// ("Apple" for "Apple Inc.").
fn suggest_parties(store: &Store, entry: &JournalEntry) -> Result<Vec<PartySuggestionOut>, String> {
    let ledger = store.ledger();
    let words = |t: &str| -> Vec<String> {
        t.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_string)
            .collect()
    };
    let contains = |hay: &[String], needle: &[String]| {
        !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
    };
    let names_of = |p: &ledger_core::Party| -> Vec<Vec<String>> {
        let mut out = vec![words(&p.name)];
        if let Some(d) = &p.display_name {
            out.push(words(d));
        }
        if let Some(first) = words(&p.name).first()
            && first.len() >= 4
        {
            out.push(vec![first.clone()]);
        }
        out
    };
    let receipts: String = store
        .attachments_for(ledger_store::Subject::Entry(entry.id()))
        .map_err(|e| e.to_string())?
        .iter()
        .map(|a| {
            format!(
                "{} {} ",
                a.file_name,
                a.description.clone().unwrap_or_default()
            )
        })
        .collect();
    let desc = words(entry.description());
    let rec = words(&receipts);
    let mut out: Vec<PartySuggestionOut> = Vec::new();
    let push = |out: &mut Vec<PartySuggestionOut>, p: &ledger_core::Party, why: String| {
        if !out.iter().any(|s| s.party_id == p.id) {
            out.push(PartySuggestionOut {
                party_id: p.id,
                name: p.name.clone(),
                why,
            });
        }
    };
    let live: Vec<&ledger_core::Party> = ledger.parties().filter(|p| !p.archived).collect();
    for p in &live {
        if names_of(p).iter().any(|n| contains(&desc, n)) {
            push(&mut out, p, "named in the description".into());
        }
    }
    for p in &live {
        if names_of(p).iter().any(|n| contains(&rec, n)) {
            push(&mut out, p, "named in a receipt".into());
        }
    }
    let key = entry.description().trim().to_lowercase();
    for e in ledger.posted_entries() {
        if e.id() != entry.id()
            && e.description().trim().to_lowercase() == key
            && let Some(p) = ledger
                .entry_parties(e.id())
                .first()
                .and_then(|s| ledger.get_party(s.party))
            && !p.archived
        {
            push(
                &mut out,
                p,
                format!("tagged on entry {} with the same description", e.id()),
            );
        }
    }
    out.truncate(3);
    Ok(out)
}

/// An entry's parties for output, each share resolved, and why they don't
/// resolve when they don't.
fn entry_parties_out(
    store: &Store,
    settings: &Settings,
    entry: &JournalEntry,
) -> (Vec<EntryPartyOut>, Option<String>) {
    let ledger = store.ledger();
    let shares = ledger.entry_parties(entry.id());
    let (accounts, members) = share_context(ledger, settings);
    let (resolved, error) =
        match ledger_core::shares::resolve(ledger, entry.id(), shares, &accounts, &members) {
            Ok(r) => (Some(r), None),
            Err(e) => (None, Some(e)),
        };
    let out = shares
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let (kind, amount, percent) = match s.share {
                ledger_core::Share::Amount(a) => ("amount", Some(money(a)), None),
                ledger_core::Share::Percent(p) => {
                    ("percent", None, Some(p.normalize().to_string()))
                }
                ledger_core::Share::Derived => ("derived", None, None),
            };
            EntryPartyOut {
                party_id: s.party,
                party: ledger
                    .get_party(s.party)
                    .map(|p| p.label().to_string())
                    .unwrap_or_else(|| format!("party {}", s.party)),
                share: kind.to_string(),
                amount,
                percent,
                resolved: resolved.as_ref().map(|r| money(r.parts[i].amount)),
                direction: resolved
                    .as_ref()
                    .map(|r| r.target.direction.key().to_string()),
                cash: resolved.as_ref().map(|r| money(r.parts[i].cash)),
                in_kind: resolved.as_ref().map(|r| money(r.parts[i].in_kind)),
            }
        })
        .collect();
    (out, error)
}

/// Preview lines with resulting balances for an entry.
fn preview_lines(store: &Store, entry: &JournalEntry) -> Result<Vec<PreviewLine>, String> {
    let lines: Vec<(AccountId, Side, Money)> = entry
        .lines()
        .iter()
        .map(|l| (l.account, l.side, l.amount))
        .collect();
    preview_lines_for(store, &lines)
}

fn preview_receipts(store: &Store, entry_id: u64) -> Result<Vec<PreviewReceipt>, String> {
    Ok(store
        .list_attachments(Some(entry_id))
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|a| PreviewReceipt {
            attachment_id: a.id,
            file_name: a.file_name,
            sha256: a.blob_sha,
        })
        .collect())
}

#[tool_router(router = entries_router, vis = "pub(super)")]
impl BooksServer {
    #[tool(
        name = "books.draft_entry",
        title = "Draft an entry",
        description = "Start a draft journal entry. Drafts are journaled and survive restarts, but don't affect reports until posted; add lines with books.add_line. Pass party (id or name) for the counterparty: every entry with a vendor, platform, agency, bank, member, or contractor names its party (a party with a vendor record tags the vendor too, for 1099s). An entry with several counterparties passes parties instead, each with an amount, a percent, or neither (derived): name both members on a joint member transaction and let their shares derive.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn draft_entry(
        &self,
        params: Parameters<DraftEntryParams>,
    ) -> Result<Json<DraftEntryOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        // Resolve every reference before writing anything (INVARIANT 2).
        let date = parse_date(&p.date)?;
        let vendor = p
            .vendor
            .as_deref()
            .map(|v| resolve_vendor(&store, v))
            .transpose()?;
        let instrument = p
            .instrument
            .as_deref()
            .map(|r| resolve_instrument(&store, r))
            .transpose()?;
        let party = p
            .party
            .as_deref()
            .map(|r| resolve_party(&store, r))
            .transpose()?;
        let no_party = p
            .no_party_reason
            .as_deref()
            .map(str::trim)
            .filter(|r| !r.is_empty());
        let shares = p
            .parties
            .as_deref()
            .map(|ps| parse_shares(&store, ps))
            .transpose()?;
        if no_party.is_some() && (party.is_some() || vendor.is_some() || shares.is_some()) {
            return Err(
                "no_party_reason is for an entry with no counterparty: pass it without party, parties, or vendor".into(),
            );
        }
        if shares.is_some() && (party.is_some() || vendor.is_some()) {
            return Err(
                "pass party (one) or parties (several, with shares), not both; a vendor party in parties tags its vendor record".into(),
            );
        }
        if let Some(sh) = &shares {
            store
                .ledger()
                .check_party_shares(sh)
                .map_err(|e| e.to_string())?;
        }
        if let (Some(v), Some(pid)) = (vendor, party) {
            let party_vendor = store.ledger().get_party(pid).and_then(|p| p.vendor);
            if party_vendor != Some(v) {
                return Err(format!(
                    "vendor {v} isn't party {pid}'s vendor record: pass only the party (its vendor record is tagged with it), or the vendor alone"
                ));
            }
        }
        // A payment method exists only in books that declare the
        // payment_instruments feature (books.add_instrument needs it), so
        // tagging the draft with one below can't be refused for the format
        // after the draft's first line (B-27).
        let id = store
            .create_entry(date, &p.description)
            .map_err(|e| e.to_string())?;
        if let Some(sh) = shares.filter(|sh| !sh.is_empty()) {
            store
                .set_entry_parties(id, &sh)
                .map_err(|e| e.to_string())?;
        } else if party.is_some() {
            store
                .set_entry_party(id, party)
                .map_err(|e| e.to_string())?;
        } else if vendor.is_some() {
            store
                .set_entry_vendor(id, vendor)
                .map_err(|e| e.to_string())?;
        } else if let Some(r) = no_party {
            store.set_entry_no_party(id, r).map_err(|e| e.to_string())?;
        }
        if instrument.is_some() {
            store
                .set_entry_instrument(id, instrument)
                .map_err(|e| e.to_string())?;
        }
        Ok(Json(DraftEntryOut {
            entry_id: id,
            status: "draft".to_string(),
        }))
    }

    #[tool(
        name = "books.add_line",
        title = "Add a line to a draft",
        description = "Add one line to a draft entry. Amount is a decimal string, never a float. The entry must balance before posting. For foreign-currency lines, pass foreign_currency, foreign_amount, and fx_rate (foreign -> base); the base amount is computed as foreign_amount * fx_rate. Pass class (a project: name or id) to tag the line; see books.projects.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn add_line(
        &self,
        params: Parameters<AddLineParams>,
    ) -> Result<Json<AddLineOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let account = resolve_account(&store, &p.account)?;
        let side = parse_side(&p.side)?;
        // Resolve the class before writing anything (INVARIANT 2).
        let class_id = match &p.class {
            Some(c) => Some(resolve_class(&store, c)?),
            None => None,
        };
        // Foreign-currency line?
        if p.foreign_currency.is_some() || p.foreign_amount.is_some() || p.fx_rate.is_some() {
            let fc = p
                .foreign_currency
                .ok_or("foreign_currency is required for fx lines")?;
            let fa = p
                .foreign_amount
                .ok_or("foreign_amount is required for fx lines")?;
            let rate = p.fx_rate.ok_or("fx_rate is required for fx lines")?;
            let currency = Currency::new(&fc).map_err(|e| e.to_string())?;
            let foreign_amount = parse_amount(&fa)?;
            let fx_rate = parse_amount(&rate)?;
            store
                .add_line_fx(p.entry_id, account, side, currency, foreign_amount, fx_rate)
                .map_err(|e| e.to_string())?;
        } else {
            let amount = parse_amount(&p.amount)?;
            if amount <= Decimal::ZERO {
                return Err("amount must be positive".to_string());
            }
            let currency = store.base_currency();
            store
                .add_line(p.entry_id, account, side, Money::new(amount, currency))
                .map_err(|e| e.to_string())?;
        }
        // Class tag, if requested. The line just added is the last one.
        if let Some(class_id) = class_id {
            let line_index = store
                .ledger()
                .get_entry(p.entry_id)
                .ok_or_else(|| format!("unknown entry {}", p.entry_id))?
                .lines()
                .len()
                .checked_sub(1)
                .ok_or_else(|| "entry has no lines".to_string())?;
            store
                .set_line_class(p.entry_id, line_index, Some(class_id))
                .map_err(|e| e.to_string())?;
        }
        let note = class_id.and_then(|c| projects::late_tag_note(store.ledger(), c));
        Ok(Json(AddLineOut { ok: true, note }))
    }

    #[tool(
        name = "books.post_entry",
        title = "Post an entry",
        description = "Post a draft entry. Without confirmed=true this returns a preview (lines, resulting balances, receipt checklist) and posts nothing. With confirmed=true it posts after enforcing the receipt policy.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    pub(super) async fn post_entry(
        &self,
        params: Parameters<PostEntryParams>,
    ) -> Result<Json<PostEntryOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let entry = store
            .ledger()
            .get_entry(p.entry_id)
            .ok_or_else(|| format!("unknown entry {}", p.entry_id))?
            .clone();
        if entry.status() != ledger_core::EntryStatus::Draft {
            return Err(format!("entry {} is not a draft", p.entry_id));
        }
        // Pure validation before anything is journaled.
        entry
            .validate(store.ledger().chart())
            .map_err(|e| e.to_string())?;
        // Validated: its lines are in a line's range, so their sum is exact.
        let total = ReceiptPolicy::entry_debits(&entry).map_err(|e| e.to_string())?;
        if let Some(expected) = p.expected_total {
            let want = parse_amount(&expected)?;
            if total != want {
                return Err(format!(
                    "expected_total mismatch: entry totals {total}, expected {want}"
                ));
            }
        }
        // One function decides (plan P4, I-5): the engine's. A draft that
        // joins a split group is covered too (anti-splitting).
        let receipt_required =
            ledger_core::evidence::requirement(store.ledger(), &entry, store.ledger().settings())
                .is_required();
        let receipts = preview_receipts(&store, p.entry_id)?;
        let split_group = ledger_core::evidence::Policy::from_settings(store.ledger().settings())
            .split_group(store.ledger(), p.entry_id, &entry);
        let receipt_missing = receipt_required && receipts.is_empty();
        let party_policy = self.settings().parties;
        let party_id = store.ledger().entry_party(p.entry_id);
        let (parties, parties_error) = entry_parties_out(&store, &self.settings(), &entry);
        let no_party_reason = store
            .ledger()
            .entry_no_party(p.entry_id)
            .map(str::to_string);
        // Any party, or a no-party mark, satisfies the policy.
        let party_missing = party_policy != ledger_config::PartyPolicy::Off
            && parties.is_empty()
            && no_party_reason.is_none();
        let party_suggestions = if party_missing {
            suggest_parties(&store, &entry)?
        } else {
            Vec::new()
        };
        let mut out = PostEntryOut {
            preview: None,
            status: "preview".to_string(),
            entry_id: p.entry_id,
            date: entry.date().to_string(),
            description: entry.description().to_string(),
            total: money(total),
            lines: preview_lines(&store, &entry)?,
            receipts,
            receipt_required,
            receipt_missing,
            split_detected: split_group.clone(),
            party_policy: party_policy.key().to_string(),
            party_id,
            party: party_id
                .and_then(|i| store.ledger().get_party(i))
                .map(|x| x.name.clone()),
            parties,
            parties_error,
            no_party_reason,
            party_missing,
            party_suggestions,
            approvals_missing: store
                .ledger()
                .approvals_missing(p.entry_id)
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|(rule, approver)| ApprovalNeededOut { rule, approver })
                .collect(),
        };
        // The digest of the draft as it stands (plan P4 step 4, D-50): its
        // lines, party, vendor, instrument and evidence, and the settings
        // seq. Another session's unrelated post does not disturb it.
        let payload = serde_json::json!({
            "tool": "books.post_entry",
            "entry": &entry,
            "party": out.party_id,
            "parties": &out.parties,
            "no_party_reason": &out.no_party_reason,
            "vendor": store.ledger().entry_vendor(p.entry_id),
            "instrument": store.ledger().entry_instrument(p.entry_id),
            "evidence": out.receipts.iter().map(|r| r.attachment_id).collect::<Vec<_>>(),
            "settings_seq": store.ledger().settings().last_seq(),
        });
        out.preview = Some(Preview::check(
            "books.post_entry",
            &payload,
            p.confirmed.unwrap_or(false),
            p.preview.as_deref(),
        )?);
        if !p.confirmed.unwrap_or(false) {
            return Ok(Json(out));
        }
        if !out.approvals_missing.is_empty() {
            let missing: Vec<String> = out
                .approvals_missing
                .iter()
                .map(|a| format!("{} (rule {})", a.approver, a.rule))
                .collect();
            return Err(serde_json::json!({
                "code": "approval_required",
                "message": format!(
                    "entry {} needs approval before it posts (policy.approval_rules is enforce): {}. Ask each approver; when they approve, record it with books.approve_entry, then post.",
                    p.entry_id,
                    missing.join("; ")
                ),
                "entry_id": p.entry_id,
                "remedy_tool": "books.approve_entry",
            })
            .to_string());
        }
        if let Some(why) = &out.parties_error {
            return Err(serde_json::json!({
                "code": "party_shares",
                "message": format!(
                    "entry {}'s party shares don't add up: {why}. Fix them with books.set_entry_party parties (amounts that sum to what it moved, or percents summing to 100).",
                    p.entry_id
                ),
                "entry_id": p.entry_id,
                "remedy_tool": "books.set_entry_party",
            })
            .to_string());
        }
        if party_missing && party_policy == ledger_config::PartyPolicy::Require {
            let names: Vec<String> = out
                .party_suggestions
                .iter()
                .map(|s| format!("{} (party {}, {})", s.name, s.party_id, s.why))
                .collect();
            return Err(serde_json::json!({
                "code": "party_required",
                "message": format!(
                    "entry {} names no counterparty, and these books require one (policy.parties = \"require\"). Tag it with books.set_entry_party (add the party with books.add_party first if it's new), or, if it truly has none, call books.set_entry_party with party null and no_party_reason.{}",
                    p.entry_id,
                    if names.is_empty() { String::new() } else { format!(" Likely: {}.", names.join("; ")) }
                ),
                "entry_id": p.entry_id,
                "remedy_tool": "books.set_entry_party",
                "party_suggestions": out.party_suggestions,
            })
            .to_string());
        }
        // The evidence gate, in the store (plan P4, I-5): the same
        // decision the preview showed, made again at the moment it posts.
        store
            .post_entry_checked(p.entry_id)
            .map_err(|e| receipt_required_text(e, &self.settings().policy))?;
        fix_derived_shares(&mut store, &self.settings(), p.entry_id)?;
        let mut posted = out;
        posted.status = "posted".to_string();
        Ok(Json(posted))
    }

    #[tool(
        name = "books.reverse_entry",
        title = "Reverse an entry",
        description = "Reverse a posted entry with an offsetting entry. Without confirmed=true returns a preview; reversals are exempt from the receipt policy.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn reverse_entry(
        &self,
        params: Parameters<ReverseEntryParams>,
    ) -> Result<Json<ReverseEntryOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let orig = store
            .ledger()
            .get_entry(p.entry_id)
            .ok_or_else(|| format!("unknown entry {}", p.entry_id))?
            .clone();
        if orig.status() != ledger_core::EntryStatus::Posted {
            return Err(format!("entry {} is not posted", p.entry_id));
        }
        let date = parse_date(&p.date)?;
        // Refused before the preview, as the post would be (an asset's
        // disposal, a run its disposal took as final, a month posted twice).
        store
            .check_reverse_entry(p.entry_id, date)
            .map_err(|e| e.to_string())?;
        // Build the preview from the flipped lines without mutating.
        let flipped: Vec<(AccountId, Side, Money)> = orig
            .lines()
            .iter()
            .map(|l| {
                let side = match l.side {
                    Side::Debit => Side::Credit,
                    Side::Credit => Side::Debit,
                };
                (l.account, side, l.amount)
            })
            .collect();
        let digest = self.preview_of(
            &store,
            "books.reverse_entry",
            p.confirmed.unwrap_or(false),
            p.preview.as_deref(),
        )?;
        let out = ReverseEntryOut {
            preview: Some(digest),
            status: "preview".to_string(),
            reversal_id: 0, // filled in on post
            reverses: p.entry_id,
            lines: preview_lines_for(&store, &flipped)?,
        };
        if !p.confirmed.unwrap_or(false) {
            return Ok(Json(out));
        }
        let reversal_id = store
            .reverse_entry(p.entry_id, date, &p.description)
            .map_err(|e| e.to_string())?;
        let mut posted = out;
        posted.status = "posted".to_string();
        posted.reversal_id = reversal_id;
        Ok(Json(posted))
    }

    #[tool(
        name = "books.get_entry",
        title = "Get an entry",
        description = "Full detail of one entry: lines, totals, status, and attached receipts.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub(super) async fn get_entry(
        &self,
        params: Parameters<GetEntryParams>,
    ) -> Result<Json<GetEntryOut>, String> {
        let p = params.0;
        let store = self.lock()?;
        let entry = store
            .ledger()
            .get_entry(p.entry_id)
            .ok_or_else(|| format!("unknown entry {}", p.entry_id))?
            .clone();
        let lines = entry
            .lines()
            .iter()
            .map(|l| {
                // A draft line an earlier version journaled may name an
                // account the chart doesn't have (rules.md B-28).
                let (code, name) = match store.ledger().chart().get(l.account) {
                    Some(a) => (a.code.clone(), a.name.clone()),
                    None => (String::new(), format!("(no account {})", l.account)),
                };
                EntryLineOut {
                    account_id: l.account,
                    code,
                    name,
                    side: match l.side {
                        Side::Debit => "debit".to_string(),
                        Side::Credit => "credit".to_string(),
                    },
                    amount: money(l.amount.amount),
                    class_id: l.class_id,
                    project: l
                        .class_id
                        .and_then(|c| store.ledger().get_class(c))
                        .map(|c| c.name.clone()),
                }
            })
            .collect();
        let (parties, parties_error) = entry_parties_out(&store, &self.settings(), &entry);
        let total = ReceiptPolicy::entry_debits(&entry);
        Ok(Json(GetEntryOut {
            entry_id: p.entry_id,
            date: entry.date().to_string(),
            description: entry.description().to_string(),
            status: match entry.status() {
                ledger_core::EntryStatus::Draft => "draft".to_string(),
                ledger_core::EntryStatus::Posted => "posted".to_string(),
            },
            reversal_of: entry.reversal_of(),
            lines,
            total: total.as_ref().ok().map(|t| money(*t)),
            total_error: total.err().map(|e| e.to_string()),
            receipts: preview_receipts(&store, p.entry_id)?,
            vendor: store
                .ledger()
                .entry_vendor(p.entry_id)
                .and_then(|v| store.ledger().get_vendor(v))
                .map(|v| v.name.clone()),
            instrument: store
                .ledger()
                .entry_instrument(p.entry_id)
                .and_then(|i| store.ledger().get_instrument(i))
                .map(|i| i.label.clone()),
            instrument_id: store.ledger().entry_instrument(p.entry_id),
            party_id: store.ledger().entry_party(p.entry_id),
            party: store
                .ledger()
                .entry_party(p.entry_id)
                .and_then(|id| store.ledger().get_party(id))
                .map(|p| p.label().to_string()),
            parties,
            parties_error,
            no_party_reason: store
                .ledger()
                .entry_no_party(p.entry_id)
                .map(str::to_string),
        }))
    }

    #[tool(
        name = "books.search_entries",
        title = "Search entries",
        description = "Search posted entries by account, date range, and description text. Newest first.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn search_entries(
        &self,
        params: Parameters<SearchEntriesParams>,
    ) -> Result<Json<SearchEntriesOut>, String> {
        let p = params.0;
        let store = self.lock()?;
        let rows = store
            .search_entries(
                p.account_id,
                p.date_from.as_deref(),
                p.date_to.as_deref(),
                p.text.as_deref(),
                p.limit.unwrap_or(50).min(500),
                p.offset.unwrap_or(0),
            )
            .map_err(|e| e.to_string())?;
        let mut entries = Vec::with_capacity(rows.len());
        for r in rows {
            let e = store
                .ledger()
                .get_entry(r.id)
                .ok_or_else(|| format!("entry {} missing from ledger", r.id))?;
            let total = ReceiptPolicy::entry_debits(e).map_err(|e| e.to_string())?;
            let receipt_count = store
                .list_attachments(Some(r.id))
                .map_err(|e| e.to_string())?
                .len() as u64;
            entries.push(EntrySummary {
                entry_id: r.id,
                date: r.date,
                description: r.description,
                total: money(total),
                reversal_of: r.reversal_of,
                receipt_count,
            });
        }
        Ok(Json(SearchEntriesOut { entries }))
    }

    #[tool(
        name = "books.import_bank_csv",
        title = "Import a bank CSV as drafts",
        description = "Import a QuickBooks-style bank/credit-card CSV: 3 columns Date,Description,Amount (payments negative), or 4 columns Date,Description, money out, money in (Intuit heads them Credit,Debit). Slash dates must settle month-first or day-first from the file itself or from date_order (\"month_first\" for mm/dd, \"day_first\" for dd/mm); ask the human when the tool says the dates read either way. Creates one DRAFT entry per transaction, all or none: bank account on one side, categorized offset on the other (the longest matching rule pattern, else default_offset_code). Give the CSV as upload_id: upload it with books.begin_upload purpose import and `wanderbooks upload` (a file never passes through you). Review with books.digest, then post. File the export too (books.file_document category import_source) and pass source_attachment_id: it is linked to the bank account, so the import traces to its source (never to the drafts: a bank export is not a receipt).",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn import_bank_csv(
        &self,
        params: Parameters<ImportBankCsvParams>,
    ) -> Result<Json<ImportBankCsvOut>, String> {
        let p = params.0;
        let order = match p.date_order.as_deref() {
            None => None,
            Some("month_first") => Some(ledger_import::DateOrder::MonthFirst),
            Some("day_first") => Some(ledger_import::DateOrder::DayFirst),
            Some(other) => {
                return Err(format!(
                    "date_order must be \"month_first\" or \"day_first\", got {other:?}"
                ));
            }
        };
        let (csv, claim) = self.csv_given(&p.upload_id)?;
        let txns = ledger_import::parse_bank_csv_with(&csv, order).map_err(|e| e.to_string())?;
        let mut store = self.lock()?;
        let bank = store
            .ledger()
            .chart()
            .find_by_code(&p.bank_account_code)
            .map(|a| (a.id, a.code.clone()))
            .ok_or_else(|| format!("unknown bank account code: {}", p.bank_account_code))?;
        // The source file links to the account, checked before any draft.
        let link_source = match p.source_attachment_id {
            None => false,
            Some(att) => {
                let a = find_attachment(&store, att)?;
                if a.withdrawn_at.is_some() {
                    return Err(format!("attachment {att} is withdrawn"));
                }
                !store
                    .attachment_links(att)
                    .map_err(|e| e.to_string())?
                    .contains(&ledger_store::Subject::Account(bank.0))
            }
        };
        let default_offset = store
            .ledger()
            .chart()
            .find_by_code(&p.default_offset_code)
            .map(|a| (a.id, a.code.clone()))
            .ok_or_else(|| format!("unknown offset account code: {}", p.default_offset_code))?;
        // Resolve rule account codes upfront.
        let mut rules: Vec<(String, (ledger_core::AccountId, String))> = Vec::new();
        for (pattern, code) in &p.rules {
            let (id, c) = store
                .ledger()
                .chart()
                .find_by_code(code)
                .map(|a| (a.id, a.code.clone()))
                .ok_or_else(|| format!("unknown rule account code: {code}"))?;
            rules.push((pattern.to_lowercase(), (id, c)));
        }
        // The most specific pattern wins (longest, then alphabetical): the
        // rules arrive as a map with no order, so "first match" would pick
        // a different account from run to run when two patterns match.
        rules.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
        let offset_for = |description: &str| {
            let desc_lower = description.to_lowercase();
            rules
                .iter()
                .find(|(pat, _)| desc_lower.contains(pat.as_str()))
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| default_offset.clone())
        };
        let base = store.ledger().base_currency();
        // Rehearse the whole file on a copy first, so a transaction that
        // can't become a draft (a date in a closed period, say) refuses the
        // file instead of leaving the ones before it behind.
        let mut scratch = store.ledger().clone();
        for txn in &txns {
            let (offset_id, _) = offset_for(&txn.description);
            let amount = Money::new(txn.amount.abs(), base);
            let (bank_side, offset_side) = if txn.amount > Decimal::ZERO {
                (Side::Debit, Side::Credit)
            } else {
                (Side::Credit, Side::Debit)
            };
            let e = scratch
                .try_create_entry(txn.date, &txn.description)
                .and_then(|e| scratch.add_line(e, bank.0, bank_side, amount).map(|()| e))
                .and_then(|e| scratch.add_line(e, offset_id, offset_side, amount))
                .map_err(|e| format!("{} {}: {e}", txn.date, txn.description));
            e?;
        }
        // Rehearsed whole: the upload is used up as the writing begins.
        files::filed(claim);
        let mut imported = Vec::new();
        for txn in txns {
            let (offset_id, offset_code) = offset_for(&txn.description);
            let amount = Money::new(txn.amount.abs(), store.ledger().base_currency());
            let entry_id = store
                .create_entry(txn.date, &format!("Bank import — {}", txn.description))
                .map_err(|e| e.to_string())?;
            // Money in: debit bank, credit offset. Money out: reverse.
            let (bank_side, offset_side) = if txn.amount > Decimal::ZERO {
                (Side::Debit, Side::Credit)
            } else {
                (Side::Credit, Side::Debit)
            };
            store
                .add_line(entry_id, bank.0, bank_side, amount)
                .map_err(|e| e.to_string())?;
            store
                .add_line(entry_id, offset_id, offset_side, amount)
                .map_err(|e| e.to_string())?;
            imported.push(ImportedTxnOut {
                entry_id,
                date: txn.date.to_string(),
                description: txn.description.clone(),
                amount: money(txn.amount),
                offset_account: offset_code,
            });
        }
        if link_source && let Some(att) = p.source_attachment_id {
            store
                .link_attachment(att, ledger_store::Subject::Account(bank.0))
                .map_err(|e| e.to_string())?;
        }
        let count = imported.len();
        Ok(Json(ImportBankCsvOut {
            imported,
            count,
            source_attachment_id: p.source_attachment_id,
            source_linked: link_source,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::test_support::*;

    /// lines_settled: posted, or a draft with lines whose debits equal its
    /// credits; a draft whose sides are past exact arithmetic (one an
    /// earlier version journaled, rules.md B-28) is not settled.
    #[test]
    fn settled_lines_are_posted_or_balanced_drafts() {
        let mut l = Ledger::new(Currency::USD);
        let cash = l
            .add_account("1000", "Cash", ledger_core::AccountType::Asset, None, false)
            .unwrap();
        let rent = l
            .add_account(
                "6000",
                "Rent",
                ledger_core::AccountType::Expense,
                None,
                false,
            )
            .unwrap();
        let mut draft = |lines: &[(u64, Side, Decimal)]| {
            let e = l.create_entry(d(2026, 3, 1), "x");
            for (a, side, amount) in lines {
                l.replay_add_line(e, *a, *side, Money::new(*amount, Currency::USD))
                    .unwrap();
            }
            e
        };
        let ten = Decimal::TEN;
        let empty = draft(&[]);
        let balanced = draft(&[(rent, Side::Debit, ten), (cash, Side::Credit, ten)]);
        let unbalanced = draft(&[(rent, Side::Debit, ten), (cash, Side::Credit, Decimal::ONE)]);
        let huge = draft(&[
            (rent, Side::Debit, Decimal::MAX),
            (rent, Side::Debit, Decimal::MAX),
            (cash, Side::Credit, Decimal::MAX),
            (cash, Side::Credit, Decimal::MAX),
        ]);
        let posted = draft(&[(rent, Side::Debit, ten), (cash, Side::Credit, ten)]);
        l.post_entry(posted).unwrap();
        let settled = |e: u64| lines_settled(l.get_entry(e).unwrap());
        assert!(!settled(empty));
        assert!(settled(balanced));
        assert!(!settled(unbalanced));
        assert!(!settled(huge));
        assert!(settled(posted));
    }
}
