//! The helpers the tool groups share: resolving a name or a code to an
//! id, reading a tool's amounts and dates, shaping a party, a document or
//! a statement for output, and the journal's own readings. They live here
//! so that no group file reaches into another group's (plan/served.md 8.2
//! U0a, Build step 2); `server.rs` re-exports them and keeps the request
//! machinery.

use super::*;
use ledger_core::InstrumentId;
use ledger_store::StoreError;

/// The book's id, as the commands given to agents name it: its folder's
/// name in the data root.
pub(crate) fn book_name(store: &Store) -> String {
    store
        .data_dir()
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// A filed file's name and type: those given, else the upload's own.
pub(crate) fn named(
    file_name: Option<String>,
    mime_type: Option<String>,
    file: &files::TakenFile,
) -> Result<(String, String), String> {
    let name = file_name.unwrap_or_else(|| file.file_name.clone());
    let mime = mime_type.unwrap_or_else(|| file.mime.clone());
    if let Some(why) = ledger_store::uploads::file_name_problem(&name)
        .or_else(|| ledger_store::uploads::mime_problem(&mime))
    {
        return Err(why);
    }
    Ok((name, mime))
}

/// A refusal of the store's evidence gate as the tool error the agent
/// reads: the structured `receipt_required` (plan P4 step 2: the error the
/// MCP built before the gate moved into the store, unchanged), any other
/// store error as its text.
pub(crate) fn receipt_required_text(e: StoreError, policy: &ReceiptPolicy) -> String {
    let StoreError::ReceiptRequired(info) = e else {
        return e.to_string();
    };
    let ledger_store::error::ReceiptRequiredInfo {
        entry_id,
        debits,
        split_group,
        ..
    } = *info;
    let split_note = if split_group.is_empty() {
        String::new()
    } else {
        format!(
            " Anti-splitting: this looks like part of a split group with entries {:?}; the policy applies to the group total.",
            split_group
        )
    };
    // A draft's debits may be past exact arithmetic (rules.md B-28): then
    // the refusal says so in place of the sum.
    let (total, debits) = match debits {
        Some(t) => (money(t), t.to_string()),
        None => (
            "an amount too large to add".to_string(),
            "too large".to_string(),
        ),
    };
    ReceiptRequiredError {
        code: "receipt_required".to_string(),
        message: format!(
            "entry {entry_id} moves {total} in debits ({}); attach a receipt with books.attach_receipt before posting.{split_note}",
            policy.describe()
        ),
        entry_id: Some(entry_id),
        asset_id: None,
        debits,
        policy: policy.describe(),
        remedy_tool: "books.attach_receipt".to_string(),
        remedy_hint: format!(
            "attach a receipt to entry {entry_id} with books.attach_receipt, then retry books.post_entry with confirmed=true"
        ),
        split_group,
    }
    .to_tool_error()
}

/// The settings as ledger_package reads them: the company, and the
/// `[books]` cash, financing, and investing roles.
pub(crate) fn package_settings(settings: &Settings) -> ledger_package::Settings<'_> {
    ledger_package::Settings {
        company: &settings.company,
        cash_accounts: &settings.roles.cash,
        financing_accounts: &settings.roles.financing,
        investing_accounts: &settings.roles.investing,
    }
}

/// A decimal string, exactly `-?digits[.digits]` (INVARIANT 3).
pub(crate) fn parse_amount(s: &str) -> Result<Decimal, String> {
    ledger_config::decimal(s)
        .ok_or_else(|| format!("bad amount {s:?}: expected a decimal string like \"123.45\""))
}

pub(crate) fn parse_side(s: &str) -> Result<Side, String> {
    match s.to_lowercase().as_str() {
        "debit" => Ok(Side::Debit),
        "credit" => Ok(Side::Credit),
        _ => Err(format!("bad side {s:?}: expected \"debit\" or \"credit\"")),
    }
}

/// Every account code a live convention names: (convention id, code).
pub(crate) fn convention_codes(ledger: &Ledger) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for c in ledger
        .conventions()
        .conventions()
        .filter(|c| c.retired.is_none())
    {
        if let Some(t) = c.template() {
            for l in t.lines {
                if ledger_core::conventions::slot(&l.account).is_none() {
                    out.push((c.id.clone(), l.account));
                }
            }
        }
        if let Some(r) = c.approval_rule() {
            for a in r.accounts {
                out.push((c.id.clone(), a));
            }
        }
    }
    out.dedup();
    out
}

pub(crate) fn settings_problems(ledger: &Ledger, cfg: &Settings) -> Vec<String> {
    let mut out = Vec::new();
    // Named as the books keep them.
    let how = " with books.set_setting";
    // The cash flow roles: codes the chart lacks here; the rest is the
    // engine's own check, the one the statement refuses on (rules.md B-14
    // rule 2), so the two never disagree.
    let lists = [
        (
            ledger_core::RoleList::Cash,
            "books.cash_accounts",
            &cfg.roles.cash,
        ),
        (
            ledger_core::RoleList::Financing,
            "books.financing_accounts",
            &cfg.roles.financing,
        ),
        (
            ledger_core::RoleList::Investing,
            "books.investing_accounts",
            &cfg.roles.investing,
        ),
    ];
    let mut ids: [Vec<ledger_core::AccountId>; 3] = Default::default();
    for ((_, setting, codes), ids) in lists.iter().zip(&mut ids) {
        for code in codes.iter() {
            match ledger.chart().find_by_code(code) {
                Some(a) => ids.push(a.id),
                None => out.push(format!(
                    "{setting} names {code}, which isn't in the chart: add that account, or change {setting}{how}"
                )),
            }
        }
    }
    let named = |l: ledger_core::RoleList| {
        lists
            .iter()
            .find(|(list, _, _)| *list == l)
            .map_or(String::new(), |(_, setting, _)| setting.to_string())
    };
    for p in ledger_core::role_problems(ledger, &ids[0], &ids[1], &ids[2]) {
        out.push(format!("{}{how}", p.message(named)));
    }
    let mut check = |setting: &str, code: &str, want: &[AccountType]| {
        match ledger.chart().find_by_code(code) {
            None => out.push(format!(
                "{setting} names {code}, which isn't in the chart: add that account, or change {setting}{how}"
            )),
            Some(a) if !want.contains(&a.account_type) => out.push(format!(
                "{setting} names {code} \"{}\", a {:?} account, but needs {}: change {setting}{how}",
                a.name,
                a.account_type,
                want.iter().map(|t| format!("{t:?}")).collect::<Vec<_>>().join(" or "),
            )),
            Some(_) => {}
        }
    };
    use AccountType::*;

    let c = &cfg.company;
    for m in &c.members {
        check(
            &format!("{} {} capital_account", "company.members", m.name),
            &m.capital_account,
            &[Equity],
        );
        if let Some(d) = &m.distribution_account {
            check(
                &format!("{} {} distribution_account", "company.members", m.name),
                d,
                &[Equity],
            );
        }
    }
    if let Some(code) = &c.close_into {
        check("books.close_into", code, &[Equity]);
    }
    for code in &c.close_sweep {
        check("books.close_sweep", code, &[Equity]);
    }
    // Conventions naming an account no longer in the chart (renumbered or
    // never there): a template can't draft, a rule covers nothing.
    for (id, code) in convention_codes(ledger) {
        if ledger.chart().find_by_code(&code).is_none() {
            out.push(format!(
                "convention {id} names account {code}, which isn't in the chart: revise it with books.set_convention"
            ));
        }
    }
    out
}

/// An asset named by a string that is its current tag or its id. A tag
/// wins: tags like "1001" are allowed, and the label is what the human
/// reads.
pub(crate) fn resolve_asset_ref(store: &Store, r: &str) -> Result<u64, String> {
    if let Some(a) = store.ledger().asset_by_tag(r) {
        return Ok(a.id);
    }
    match r.parse::<u64>() {
        Ok(id) if store.ledger().get_asset(id).is_some() => Ok(id),
        _ => Err(format!("no asset has the tag or id {r:?}")),
    }
}

/// Parse a calendar-month period "YYYY-MM".
pub(crate) fn parse_period_mcp(period: &str) -> Result<(i32, u32), String> {
    let err = || format!("bad period {period:?}: expected YYYY-MM");
    let mut parts = period.split('-');
    let year: i32 = parts.next().ok_or_else(err)?.parse().map_err(|_| err())?;
    let month: u32 = parts.next().ok_or_else(err)?.parse().map_err(|_| err())?;
    // A month of the calendar the books keep (ledger_core::calendar).
    if parts.next().is_some() || !(1..=9999).contains(&year) || !(1..=12).contains(&month) {
        return Err(err());
    }
    Ok((year, month))
}

/// Last calendar day of a month: refused, never a panic, for one chrono
/// can't reach (P0 review round 3, S1).
pub(crate) fn last_day_of_month(year: i32, month: u32) -> Result<NaiveDate, String> {
    NaiveDate::from_ymd_opt(year, month, 1)
        .and_then(|first| first.checked_add_months(chrono::Months::new(1)))
        .and_then(|next| next.pred_opt())
        .ok_or_else(|| format!("bad period {year:04}-{month:02}: no such month"))
}

/// Resolve an account reference: an account code, or a numeric id. A
/// number that is one account's code and another's id is refused
/// (INVARIANT 6).
pub(crate) fn resolve_account(store: &Store, account: &str) -> Result<AccountId, String> {
    let chart = store.ledger().chart();
    let by_code = chart.find_by_code(account).map(|a| a.id);
    let by_id = account
        .parse::<u64>()
        .ok()
        .filter(|id| chart.get(*id).is_some());
    match (by_code, by_id) {
        (Some(c), Some(i)) if c != i => Err(format!(
            "account {account:?} is ambiguous: it is the code of account {c} and the id of account {i}; pass the code of the one you mean"
        )),
        (Some(c), _) => Ok(c),
        (None, Some(i)) => Ok(i),
        (None, None) => Err(format!(
            "unknown account {account:?}: use an id or code from books.chart"
        )),
    }
}

/// "invoice" or "bill" and an id, as a document.
pub(crate) fn doc_ref(kind: &str, id: u64) -> Result<ledger_core::DocRef, String> {
    match kind.trim().to_ascii_lowercase().as_str() {
        "invoice" => Ok(ledger_core::DocRef::Invoice(id)),
        "bill" => Ok(ledger_core::DocRef::Bill(id)),
        other => Err(format!("bad kind {other:?}: expected invoice or bill")),
    }
}

/// The control check as `books.control_check` answers it.
pub(crate) fn control_check_out(
    ledger: &Ledger,
    as_of: chrono::NaiveDate,
    checks: &[ledger_core::ControlCheck],
) -> ControlCheckOut {
    let accounts: Vec<ControlCheckRowOut> = checks
        .iter()
        .map(|c| {
            let acct = ledger.chart().get(c.account);
            ControlCheckRowOut {
                role: c.role.clone(),
                code: acct.map(|a| a.code.clone()).unwrap_or_default(),
                name: acct.map(|a| a.name.clone()).unwrap_or_default(),
                balance: money(c.balance),
                documents_total: money(c.documents_total),
                unattached: c
                    .unattached
                    .iter()
                    .map(|u| UnattachedLineOut {
                        entry_id: u.entry,
                        line: u.line,
                        date: u.date.to_string(),
                        amount: money(u.amount),
                    })
                    .collect(),
                difference: money(c.difference),
                attention: c
                    .attention
                    .iter()
                    .map(|a| {
                        let (kind, id) = match a.doc {
                            ledger_core::DocRef::Invoice(id) => ("invoice", id),
                            ledger_core::DocRef::Bill(id) => ("bill", id),
                        };
                        DocAttentionOut {
                            kind: kind.to_string(),
                            id,
                            status: a.status.key().to_string(),
                            total: money(a.total),
                            outstanding: money(a.outstanding),
                        }
                    })
                    .collect(),
            }
        })
        .collect();
    let agrees = checks
        .iter()
        .all(|c| c.difference.is_zero() && c.unattached.is_empty() && c.attention.is_empty());
    ControlCheckOut {
        as_of: as_of.to_string(),
        agrees,
        accounts,
    }
}

/// Resolve a party by id or by name: the name ignoring case and
/// surrounding space, among parties not archived (an archived party goes
/// by id). A number that is one party's id and another's name is refused
/// (INVARIANT 6).
pub(crate) fn resolve_party(store: &Store, party: &str) -> Result<ledger_core::PartyId, String> {
    let ledger = store.ledger();
    let by_id = party
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|id| ledger.get_party(*id).is_some());
    let by_name = ledger.party_by_name(party).map(|p| p.id);
    match (by_id, by_name) {
        (Some(i), Some(n)) if i != n => Err(format!(
            "party {party:?} is ambiguous: it is party {i}'s id and party {n}'s name; pass the id"
        )),
        (Some(i), _) | (None, Some(i)) => Ok(i),
        (None, None) => Err(format!(
            "unknown party {party:?}: use an id or name from books.parties (an archived party goes by id), or add it with books.add_party"
        )),
    }
}

/// A stored attachment's metadata, by id.
pub(crate) fn find_attachment(store: &Store, id: u64) -> Result<ledger_store::Attachment, String> {
    store
        .list_attachments(None)
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|a| a.id == id)
        .ok_or_else(|| format!("unknown attachment {id}"))
}

/// An attachment for a listing, with its links.
pub(crate) fn receipt_out(
    store: &Store,
    a: ledger_store::Attachment,
) -> Result<ReceiptOut, String> {
    Ok(ReceiptOut {
        links: links_out(store, a.id)?,
        attachment_id: a.id,
        sha256: a.blob_sha,
        entry_id: a.entry_id,
        category: a.category,
        file_name: a.file_name,
        mime_type: a.mime_type,
        size_bytes: a.size_bytes,
        description: a.description,
        withdrawn: a.withdrawn_at.is_some(),
        withdrawn_reason: a.withdrawn_reason,
    })
}

/// An account's code and name.
pub(crate) fn code_name(store: &Store, id: AccountId) -> (String, String) {
    store
        .ledger()
        .chart()
        .get(id)
        .map(|a| (a.code.clone(), a.name.clone()))
        .unwrap_or_default()
}

/// The latest statement date an account is reconciled through
/// (reconciliations not voided), if any.
pub(crate) fn reconciled_through(
    store: &Store,
    account: AccountId,
) -> Result<Option<NaiveDate>, String> {
    Ok(store
        .reconciliations(Some(account))
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|r| r.voided.is_none())
        .map(|r| r.statement_date)
        .max())
}

/// The last four digits of every TIN the books know of, from the journal
/// (each party's TIN and W-9 facts), never the vault: the filing scan's
/// probe bound (tin_files.rs INVARIANT 4).
pub(crate) fn known_tin_last4(store: &Store) -> Vec<u32> {
    store
        .ledger()
        .parties()
        .flat_map(|p| {
            [
                p.tin.as_ref().map(|t| t.last4.as_str()),
                p.w9.as_ref().map(|w| w.tin_last4.as_str()),
            ]
        })
        .flatten()
        .filter_map(|l| l.parse().ok())
        .collect()
}

/// An optional party text from a tool: trimmed, and "" clears it.
pub(crate) fn optional_text(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// The sum of `values`, each computed or refused: refused (as text) when
/// one is, or when the sum is past exact arithmetic. Figures an earlier
/// version journaled are as journaled (rules.md B-28).
pub(crate) fn checked_sum(
    values: impl IntoIterator<Item = Result<Decimal, ledger_core::LedgerError>>,
    what: &str,
) -> Result<Decimal, String> {
    values.into_iter().try_fold(Decimal::ZERO, |sum, v| {
        let v = v.map_err(|e| e.to_string())?;
        sum.checked_add(v).ok_or_else(|| {
            ledger_core::LedgerError::TooLargeToCompute(what.to_string()).to_string()
        })
    })
}

/// The `party_required` refusal for an engine-posted entry (payroll, a
/// remittance, a disposal) that names no counterparty under `[books]
/// parties = "require"`: `what` says what to pass.
pub(crate) fn party_required(what: &str) -> String {
    serde_json::json!({
        "code": "party_required",
        "message": format!(
            "these books require a counterparty on every entry (their parties setting is \"require\"): {what} (add the party with books.add_party first if it's new)"
        ),
    })
    .to_string()
}

/// The chart's ids for these account codes (codes it lacks are skipped).
pub(crate) fn cash_ids(ledger: &Ledger, codes: &[String]) -> Vec<AccountId> {
    codes
        .iter()
        .filter_map(|c| ledger.chart().find_by_code(c).map(|a| a.id))
        .collect()
}

/// What party shares resolve with ([`ledger_core::shares::resolve`]): the
/// money accounts over the `[books] cash_accounts` and
/// `financing_accounts` (with the cards), and the members.
pub(crate) fn share_context(
    ledger: &Ledger,
    settings: &Settings,
) -> (
    ledger_core::parties::MoneyAccounts,
    Vec<ledger_core::PartyMember>,
) {
    (
        ledger_core::parties::money_accounts(
            ledger,
            &cash_ids(ledger, &settings.roles.cash),
            &cash_ids(ledger, &settings.roles.financing),
        ),
        settings.company.party_members(ledger),
    )
}

/// `parties` params as shares: each party resolved, `amount` or `percent`
/// parsed (not both), neither meaning derived.
pub(crate) fn parse_shares(
    store: &Store,
    params: &[PartyShareParam],
) -> Result<Vec<ledger_core::PartyShare>, String> {
    params
        .iter()
        .map(|p| {
            let party = resolve_party(store, &p.party)?;
            let share = match (&p.amount, &p.percent) {
                (Some(_), Some(_)) => {
                    return Err(format!(
                        "party {}: give an amount or a percent, not both",
                        p.party
                    ));
                }
                (Some(a), None) => ledger_core::Share::Amount(parse_amount(a)?),
                (None, Some(pc)) => ledger_core::Share::Percent(parse_amount(pc)?),
                (None, None) => ledger_core::Share::Derived,
            };
            Ok(ledger_core::PartyShare { party, share })
        })
        .collect()
}

/// Whether an entry's lines are final enough to check shares against:
/// posted, or a draft with lines whose debits equal its credits.
pub(crate) fn lines_settled(entry: &JournalEntry) -> bool {
    if entry.status() == ledger_core::EntryStatus::Posted {
        return true;
    }
    // A draft's lines are as journaled, of any size (rules.md B-28): sides
    // past exact arithmetic are not settled lines.
    let side = |s: Side| -> Option<Decimal> {
        entry
            .lines()
            .iter()
            .filter(|l| l.side == s)
            .try_fold(Decimal::ZERO, |sum, l| sum.checked_add(l.amount.amount))
    };
    match (side(Side::Debit), side(Side::Credit)) {
        (Some(debits), Some(credits)) => !entry.lines().is_empty() && debits == credits,
        _ => false,
    }
}

/// Fix a posted entry's derived split among several parties: journal the
/// exact amounts they resolve to now, so a later change to
/// `[[company.members]]` (a new income split, a member archived) never
/// re-splits posted history.
///
/// ```text
/// GOAL: posted is forever, the split between its parties included.
/// SOURCE: AGENTS.md ("Posted is forever"); docs/parties-and-attachments.md
///         "Shares"; ledger_core::shares INVARIANT 10 (derived several:
///         the members' capital lines, else their income shares, both read
///         from the settings).
/// INVARIANTS:
///   1. Only a posted entry naming two or more parties with derived shares
///      changes: its shares become Amount(|part|) for each party, in
///      order, journaled as entry.parties_set (the reversal chain too), so
///      each party's resolved part is exactly what it was.
///   2. Nothing changes when the shares don't resolve, or a part is zero
///      (an amount must be positive): the entry keeps its derived shares.
/// ```
pub(crate) fn fix_derived_shares(
    store: &mut Store,
    settings: &Settings,
    entry_id: u64,
) -> Result<(), String> {
    let ledger = store.ledger();
    let Some(entry) = ledger.get_entry(entry_id) else {
        return Ok(());
    };
    let shares = ledger.entry_parties(entry_id);
    if entry.status() != ledger_core::EntryStatus::Posted
        || shares.len() < 2
        || !shares
            .iter()
            .any(|s| matches!(s.share, ledger_core::Share::Derived))
    {
        return Ok(());
    }
    let (money, members) = share_context(ledger, settings);
    let Ok(resolved) = ledger_core::shares::resolve(ledger, entry_id, shares, &money, &members)
    else {
        return Ok(());
    };
    if resolved.parts.iter().any(|p| p.amount.is_zero()) {
        return Ok(());
    }
    let fixed: Vec<ledger_core::PartyShare> = resolved
        .parts
        .iter()
        .map(|p| ledger_core::PartyShare {
            party: p.party,
            share: ledger_core::Share::Amount(p.amount.abs()),
        })
        .collect();
    store
        .set_entry_parties(entry_id, &fixed)
        .map_err(|e| e.to_string())
}

/// Resolve a class (a project) by id or name, trimmed, the name ignoring
/// capitals as the duplicate check does; a number that is one class's id
/// and another's name is refused (INVARIANT 6). New names can't be digits
/// only, so only old books can hold such a pair.
pub(crate) fn resolve_class(store: &Store, class: &str) -> Result<u32, String> {
    let class = class.trim();
    let by_id = class
        .parse::<u32>()
        .ok()
        .filter(|id| store.ledger().get_class(*id).is_some());
    let by_name = store.ledger().find_class_by_name(class).map(|c| c.id);
    match (by_id, by_name) {
        (Some(i), Some(n)) if i != n => {
            let name = store
                .ledger()
                .get_class(i)
                .map(|c| c.name.clone())
                .unwrap_or_default();
            Err(format!(
                "project {class:?} is ambiguous: it is project {i}'s id and project {n}'s name; pass {n} for project {n}, or {name:?} (project {i}'s name) for project {i}"
            ))
        }
        (Some(i), _) | (None, Some(i)) => Ok(i),
        (None, None) => Err(format!(
            "unknown project {class:?}: use an id or name from books.projects"
        )),
    }
}

/// A tool description that ends with the one sentence for its kind of
/// dates (rules.md B-30), so no tool says other than the resolver does.
pub(crate) fn described(what: &str, kind: ledger_core::calendar::PeriodKind) -> String {
    format!("{what} {}", kind.describe())
}

/// The warning for a revenue or expense account created without a tax
/// category (rules.md B-32).
pub(crate) fn uncategorized_warning(code: &str, name: &str) -> String {
    format!(
        "{} {} has no tax category: the tax summary will list it as uncategorized. Propose one from books.tax_categories and set it with books.set_tax_category once the human agrees.",
        code.trim(),
        name.trim()
    )
}

/// The warnings for the revenue and expense accounts an operation created
/// (control accounts: Revenue, General Expenses, payroll's expenses)
/// without a tax category: every account from `first_new` on.
pub(crate) fn created_uncategorized(
    ledger: &Ledger,
    first_new: ledger_core::AccountId,
) -> Vec<String> {
    let mut new: Vec<&ledger_core::Account> = ledger
        .chart()
        .all()
        .filter(|a| a.id >= first_new && a.tax_category.is_none())
        .filter(|a| ledger_core::tax::needs_category(a.account_type, a.placeholder))
        .collect();
    new.sort_by_key(|a| a.id);
    new.into_iter()
        .map(|a| uncategorized_warning(&a.code, &a.name))
        .collect()
}

/// Everything an attachment is linked to, for a tool's output.
pub(crate) fn links_out(store: &Store, attachment_id: u64) -> Result<Vec<SubjectOut>, String> {
    Ok(store
        .attachment_links(attachment_id)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|s| subject_out(store, s))
        .collect())
}

/// A subject for a tool's output, with human words for it.
pub(crate) fn subject_out(store: &Store, sub: ledger_store::Subject) -> SubjectOut {
    use ledger_store::Subject as S;
    let l = store.ledger();
    let label = match sub {
        S::Entry(i) => l
            .get_entry(i)
            .map(|e| format!("entry #{i} {}", e.description())),
        S::Account(i) => l
            .chart()
            .get(i)
            .map(|a| format!("account {} {}", a.code, a.name)),
        S::Asset(i) => l.get_asset(i).map(|a| match &a.tag {
            Some(t) => format!("asset {t} {}", a.description),
            None => format!("asset {i} {}", a.description),
        }),
        S::Party(i) => l.get_party(i).map(|p| format!("party {i} {}", p.label())),
        S::Project(i) => u32::try_from(i)
            .ok()
            .and_then(|c| l.get_class(c))
            .map(|c| format!("project {i} {}", c.name)),
        S::Period(i) => l
            .list_periods()
            .iter()
            .find(|p| p.id == i)
            .map(|p| format!("period {}", p.name)),
        S::PayrollRun(i) => l
            .get_payroll_run(i)
            .map(|r| format!("payroll run {i} of {}", r.date)),
        S::Schedule(i) => l
            .get_schedule(i)
            .map(|s| format!("schedule {i} {}", s.description)),
        S::Reconciliation(i) => store.reconciliations(None).ok().and_then(|rs| {
            rs.into_iter().find(|r| r.id == i).map(|r| {
                let code = l
                    .chart()
                    .get(r.account_id)
                    .map(|a| a.code.clone())
                    .unwrap_or_default();
                format!("reconciliation {i} of {code} through {}", r.statement_date)
            })
        }),
        S::Invoice(_) | S::Bill(_) | S::Company => None,
    }
    .unwrap_or_else(|| sub.describe());
    SubjectOut {
        kind: sub.kind().to_string(),
        id: (sub != S::Company).then(|| sub.id()),
        label,
    }
}

/// Resolve a payment method by id or label (INVARIANT 5).
pub(crate) fn resolve_instrument(store: &Store, r: &str) -> Result<InstrumentId, String> {
    let ledger = store.ledger();
    let by_id = r
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|id| ledger.get_instrument(*id).is_some());
    let by_label = ledger.instrument_by_label(r).map(|i| i.id);
    match (by_id, by_label) {
        (Some(i), Some(l)) if i != l => Err(format!(
            "payment method {r:?} is ambiguous: it is method {i}'s id and method {l}'s label; pass the id"
        )),
        (Some(i), _) | (None, Some(i)) => Ok(i),
        (None, None) => Err(format!(
            "unknown payment method {r:?}: use an id or label from books.instruments (books.add_instrument adds one)"
        )),
    }
}

/// A recorded send on the wire (plan P4b S5).
pub(crate) fn sent_out(s: &ledger_core::arap::Sent) -> SentOut {
    SentOut {
        on: s.on.to_string(),
        by: s.by.clone(),
        via: s.via.key(),
        to: s.to.clone(),
        kind: s.kind.key().to_string(),
        reference: s.reference.clone(),
        note: s.note.clone(),
    }
}

/// The day an invoice last went out, when a send is recorded.
pub(crate) fn last_sent(sends: &[ledger_core::arap::Sent]) -> Option<String> {
    sends.iter().map(|s| s.on).max().map(|d| d.to_string())
}

/// An annotation as a tool answers with it (plan P6 step 5).
pub(crate) fn annotation_out(a: &ledger_core::Annotation) -> AnnotationOut {
    AnnotationOut {
        receipt_number: a.fields.receipt_number,
        payee: a.fields.payee.clone(),
        amount: a.fields.amount.map(money),
        date: a.fields.date.map(|d| d.to_string()),
        instrument: a.fields.instrument.clone(),
        category: a.fields.category.clone(),
        source: a.fields.source.clone(),
        reimbursable_to: a.fields.reimbursable_to.clone(),
        note: a.note.clone(),
    }
}

/// A file name's stem, for a page picture's or a text file's name: what is
/// before the last dot, or "attachment" when there is nothing.
pub(crate) fn stem(file_name: &str) -> String {
    let stem = file_name
        .rsplit_once('.')
        .map_or(file_name, |(stem, _)| stem)
        .trim();
    if stem.is_empty() {
        "attachment".to_string()
    } else {
        stem.chars().take(80).collect()
    }
}

/// The largest character boundary at or below `at` in `text`.
pub(crate) fn floor_char_boundary(text: &str, at: usize) -> usize {
    let mut cut = at.min(text.len());
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    cut
}

/// The preview digest (plan P4 step 4, D-50, amending I-6): the SHA-256 of
/// the canonical JSON of what a tool's preview shows. A confirmed call
/// recomputes it from the books as they are now and refuses a different
/// one (`stale_preview`): a changed argument, a post by another session,
/// a changed setting. It detects drift; it does not prove a human saw the
/// preview. No key: a same-user key would prove nothing.
///
/// ```text
/// GOAL: a commit acts on the thing that was previewed, or not at all.
/// SOURCE: plan/served.md I-6, D-50 (owner decision 3: a post by another
///         session in between forces a re-preview).
/// INVARIANTS:
///   1. For a draft entry the payload is the entry's canonical state
///      (id, date, description, lines with their classes, party, vendor,
///      instrument, the evidence linked) and the settings seq: other
///      sessions' unrelated posts do not disturb it.
///   2. For every other operation it is the tool, its canonical arguments
///      (without `confirmed` and `preview`), the settings seq and the
///      journal head seq: any write in between does.
///   3. Equal payloads give equal digests whatever the key order.
/// ```
pub struct Preview;

impl Preview {
    /// The digest of `payload`.
    pub fn of(payload: &serde_json::Value) -> String {
        ledger_store::journal::sha256_hex(ledger_store::journal::canonical_json(payload).as_bytes())
    }

    /// The payload of an operation without a draft: the tool, the
    /// arguments it was called with (without `confirmed` and `preview`),
    /// the settings seq and the journal head seq.
    pub fn operation(store: &Store, tool: &str) -> serde_json::Value {
        let mut args = ARGS.with(|a| a.borrow().clone()).unwrap_or_default();
        args.remove("confirmed");
        args.remove("preview");
        args.retain(|_, v| !v.is_null());
        serde_json::json!({
            "tool": tool,
            "args": args,
            "settings_seq": store.ledger().settings().last_seq(),
            "head_seq": store.head().0,
        })
    }

    /// The preview check every tool with `confirmed` makes: the digest of
    /// `payload` now. Without `confirmed` it is what the preview returns;
    /// with it, `given` must equal it, or the call is refused with the
    /// structured `stale_preview` guard before anything is written.
    pub fn check(
        tool: &str,
        payload: &serde_json::Value,
        confirmed: bool,
        given: Option<&str>,
    ) -> Result<String, String> {
        let now = Self::of(payload);
        if !confirmed {
            return Ok(now);
        }
        let message = match given.map(str::trim) {
            None | Some("") => format!(
                "{tool} with confirmed=true needs the preview of what is being confirmed: call {tool} without confirmed, show it to the human, then pass its preview"
            ),
            Some(g) if g == now => return Ok(now),
            Some(_) => format!(
                "the books or the arguments changed since {tool} was previewed (a post by another session, a changed setting or argument): call {tool} without confirmed again and confirm that preview"
            ),
        };
        Err(serde_json::json!({
            "code": "stale_preview",
            "message": message,
            "remedy_tool": tool,
        })
        .to_string())
    }
}

impl BooksServer {
    /// [`Preview::check`] for an operation without a draft: the payload is
    /// built from the tool, its arguments and the books' seqs.
    pub(crate) fn preview_of(
        &self,
        store: &Store,
        tool: &str,
        confirmed: bool,
        given: Option<&str>,
    ) -> Result<String, String> {
        Preview::check(tool, &Preview::operation(store, tool), confirmed, given)
    }
}
