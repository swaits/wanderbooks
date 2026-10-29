//! Evidence beyond a file's bytes: finding it, annotating it, and
//! consolidating duplicates (plan/served.md P6 steps 4 and 5).
//!
//! ```text
//! GOAL: a receipt found by what it is or says, the facts a human read off
//!       it kept on the record, and identical files held once, linked
//!       wherever they are evidence.
//! SOURCE: plan/served.md P6 step 4 (defect 4) and step 5 (defect 10);
//!         IZ-18 (no FIFO guess: `entry.settles` only); Rev. Proc. 98-25
//!         (an audit trail); rules.md B-44.
//! ```

use super::*;

/// A date as the tool takes it.
fn date_of(text: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d")
        .map_err(|_| format!("{text:?} isn't a date: give YYYY-MM-DD"))
}

#[tool_router(router = evidence_router, vis = "pub(super)")]
impl BooksServer {
    /// ```text
    /// GOAL: find a filed file by its exact bytes, its receipt number, or
    ///       what its text says.
    /// SOURCE: plan/served.md P6 step 4; B-44.
    /// INVARIANTS:
    ///   1. EXACTLY one of sha256, receipt_number and text is given.
    ///   2. Withdrawn files are listed too, marked (a duplicate is found
    ///      before it is consolidated).
    ///   3. A text search is case-insensitive over the extracted text.
    /// SCENARIOS:
    ///   1. IF none or more than one is given: SHOULD refuse.
    ///   2. IF sha256 is not 64 hex digits: SHOULD refuse.
    ///   3. IF nothing matches: SHOULD answer with an empty list.
    /// ```
    #[tool(
        name = "books.find_attachment",
        title = "Find a filed file",
        description = "Find receipts and documents by exactly one of: sha256 (the file's exact hash), receipt_number (what books.annotate_attachment recorded), or text (words its extracted text says, case-insensitively). Withdrawn files are listed too, marked, so a duplicate is found before books.consolidate_duplicates. Each comes with all its links.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn find_attachment(
        &self,
        params: Parameters<FindAttachmentParams>,
    ) -> Result<Json<FindAttachmentOut>, String> {
        let p = params.0;
        let given = [
            p.sha256.is_some(),
            p.receipt_number.is_some(),
            p.text.is_some(),
        ]
        .into_iter()
        .filter(|g| *g)
        .count();
        if given != 1 {
            return Err(
                "give exactly one of sha256, receipt_number or text: they find a file three ways"
                    .into(),
            );
        }
        let store = self.lock()?;
        let mut found = if let Some(sha) = p.sha256 {
            let sha = sha.trim().to_lowercase();
            if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("sha256 is 64 hexadecimal digits".into());
            }
            store
                .list_attachments(None)
                .map_err(|e| e.to_string())?
                .into_iter()
                .filter(|a| a.blob_sha.eq_ignore_ascii_case(&sha))
                .collect()
        } else if let Some(n) = p.receipt_number {
            store
                .attachments_with_receipt_number(n)
                .map_err(|e| e.to_string())?
        } else {
            let text = p.text.unwrap_or_default();
            let text = text.trim();
            if text.chars().count() < 2 {
                return Err("text is at least two characters".into());
            }
            store
                .attachments_holding_text(text)
                .map_err(|e| e.to_string())?
        };
        found.sort_by_key(|a| a.id);
        let attachments = found
            .into_iter()
            .map(|a| receipt_out(&store, a))
            .collect::<Result<_, _>>()?;
        Ok(Json(FindAttachmentOut { attachments }))
    }

    /// ```text
    /// GOAL: record what a human read off a file, without touching the file
    ///       or the ledger's amounts.
    /// SOURCE: plan/served.md P6 step 5 (defect 10); B-44.
    /// INVARIANTS:
    ///   1. Journaled as `attachment.annotated`; a refusal journals nothing.
    ///   2. The fields are the closed list of `AnnotationFieldsParam`; at
    ///      least one field or the note is given.
    ///   3. A new annotation replaces the file's earlier one; the file
    ///      stays filed and its bytes unchanged.
    ///   4. The answer names the next receipt number (one past the highest
    ///      annotated).
    /// SCENARIOS:
    ///   1. IF the attachment is unknown or withdrawn: SHOULD refuse.
    ///   2. IF nothing is given, an amount is negative or malformed, or a
    ///      date is outside the calendar: SHOULD refuse.
    /// ```
    #[tool(
        name = "books.annotate_attachment",
        title = "Annotate a filed file",
        description = "Record what a human read off a receipt or document: receipt_number, payee, amount (decimal string), date (YYYY-MM-DD), instrument, category, source, reimbursable_to, and a note. It changes no amount and no file; a later annotation replaces this one. The answer gives next_receipt_number (one past the highest annotated), so receipt numbers run without reuse. Amounts are exact decimal strings.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn annotate_attachment(
        &self,
        params: Parameters<AnnotateAttachmentParams>,
    ) -> Result<Json<AnnotateAttachmentOut>, String> {
        let p = params.0;
        let currency = self.currency();
        let amount = match p.fields.amount.as_deref() {
            None => None,
            Some(a) => Some(a.trim().parse::<Decimal>().map_err(|_| {
                format!("{a:?} isn't an amount: give a decimal string like \"12.34\"")
            })?),
        };
        let date = match p.fields.date.as_deref() {
            None => None,
            Some(d) => Some(date_of(d)?),
        };
        let annotation = ledger_core::Annotation {
            attachment_id: p.attachment_id,
            fields: ledger_core::AnnotationFields {
                receipt_number: p.fields.receipt_number,
                payee: p.fields.payee,
                amount,
                date,
                instrument: p.fields.instrument,
                category: p.fields.category,
                source: p.fields.source,
                reimbursable_to: p.fields.reimbursable_to,
            },
            note: p.note,
        };
        let _ = currency;
        let mut store = self.lock()?;
        let stored = store
            .annotate_attachment(annotation)
            .map_err(|e| e.to_string())?;
        let next = store.ledger().next_receipt_number();
        Ok(Json(AnnotateAttachmentOut {
            attachment_id: stored.attachment_id,
            fields: annotation_out(&stored),
            next_receipt_number: next,
        }))
    }

    /// ```text
    /// GOAL: record which entries a payment settles, so "the member was
    ///       paid back" is read from the books and never guessed.
    /// SOURCE: plan/served.md P6 step 5 (IZ-18: no FIFO guess).
    /// INVARIANTS:
    ///   1. Journaled as `entry.settles`; a refusal journals nothing.
    ///   2. Every named entry exists, none is the paying entry itself, none
    ///      is named twice.
    ///   3. An empty list clears the record.
    /// SCENARIOS:
    ///   1. IF the paying entry or a settled entry is unknown, or the list
    ///      names the paying entry or one twice: SHOULD refuse.
    /// ```
    #[tool(
        name = "books.set_entry_settles",
        title = "Record what a payment settles",
        description = "Record which entries an entry pays back, such as a reimbursement to a member settling the expenses they paid personally. Needs attention stops asking about a member-paid entry once a payment names it here. Empty clears the record. It changes no amount: it names what the payment settles.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn set_entry_settles(
        &self,
        params: Parameters<SetEntrySettlesParams>,
    ) -> Result<Json<SetEntrySettlesOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        store
            .set_entry_settles(p.entry_id, &p.settles)
            .map_err(|e| e.to_string())?;
        Ok(Json(SetEntrySettlesOut {
            ok: true,
            entry_id: p.entry_id,
            settles: p.settles,
        }))
    }

    /// ```text
    /// GOAL: hold identical files once, linked wherever they were evidence,
    ///       without ever leaving an entry the receipt policy covers with
    ///       none.
    /// SOURCE: plan/served.md P6 step 4 (defect 4); rules.md B-44.
    /// INVARIANTS:
    ///   1. Records sharing one blob SHA are one set; the oldest is kept
    ///      and linked to every subject the others had.
    ///   2. Every other record is unlinked, then withdrawn as a duplicate
    ///      (`duplicate_of`), journaled; nothing is deleted.
    ///   3. Every touched entry's evidence is re-checked before the first
    ///      journal append: an entry that would be left with none and
    ///      needs one refuses the whole call, journaling nothing.
    ///   4. Without `confirmed`, a preview that changes nothing.
    /// SCENARIOS:
    ///   1. IF the kept record is withdrawn and cannot stand in for a
    ///      duplicate that is a posted entry's only evidence: SHOULD
    ///      refuse (receipt_required).
    ///   2. IF no two records share bytes: SHOULD answer with no groups.
    /// ```
    #[tool(
        name = "books.consolidate_duplicates",
        title = "Consolidate duplicate files",
        description = "Find receipts and documents filed more than once with the same bytes. For each set it keeps the oldest, links it to every subject the others had, unlinks the others, and withdraws them as duplicates of it (nothing is deleted). Refused whole when it would leave a posted entry the receipt policy covers with no evidence. Without confirmed, a preview that changes nothing.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn consolidate_duplicates(
        &self,
        params: Parameters<ConsolidateDuplicatesParams>,
    ) -> Result<Json<ConsolidateDuplicatesOut>, String> {
        use std::collections::{BTreeMap, BTreeSet};
        let confirmed = params.0.confirmed.unwrap_or(false);
        let mut store = self.lock()?;
        let all = store.list_attachments(None).map_err(|e| e.to_string())?;
        let mut by_sha: BTreeMap<String, Vec<ledger_store::Attachment>> = BTreeMap::new();
        for a in all {
            by_sha.entry(a.blob_sha.clone()).or_default().push(a);
        }
        // One plan per set of two or more records.
        struct Group {
            sha: String,
            keep: ledger_store::Attachment,
            dups: Vec<ledger_store::Attachment>,
            link: BTreeSet<ledger_store::Subject>,
            unlink: Vec<(u64, ledger_store::Subject)>,
        }
        let mut groups: Vec<Group> = Vec::new();
        for (sha, mut records) in by_sha {
            if records.len() < 2 {
                continue;
            }
            records.sort_by_key(|a| a.id);
            let keep = records.remove(0);
            let mut link: BTreeSet<ledger_store::Subject> = BTreeSet::new();
            let mut unlink = Vec::new();
            let mut live_dups = Vec::new();
            for d in records {
                if d.withdrawn_at.is_some() {
                    continue;
                }
                for sub in subjects_of(&store, &d)? {
                    if !covered(&store, &keep, sub)? {
                        link.insert(sub);
                    }
                    if !matches!(sub, ledger_store::Subject::Entry(e) if Some(e) == d.entry_id) {
                        unlink.push((d.id, sub));
                    }
                }
                live_dups.push(d);
            }
            if live_dups.is_empty() {
                continue;
            }
            groups.push(Group {
                sha,
                keep,
                dups: live_dups,
                link,
                unlink,
            });
        }
        // Consolidating writes `duplicate_of`: the books must read the
        // feature before anything at all is journaled (plan P6, R-A).
        if !groups.is_empty() {
            store
                .ensure_feature(ledger_store::format::ATTACHMENT_TEXT)
                .map_err(|e| e.to_string())?;
        }
        // The evidence gate, before anything is journaled: an entry that
        // would be left with none, and needs one, refuses the whole call.
        for g in &groups {
            let keep_stands = g.keep.withdrawn_at.is_none();
            for sub in &g.link {
                if let ledger_store::Subject::Entry(e) = sub {
                    let mut left = store
                        .list_attachments(Some(*e))
                        .map_err(|e| e.to_string())?
                        .into_iter()
                        .filter(|a| a.withdrawn_at.is_none())
                        .filter(|a| !g.dups.iter().any(|d| d.id == a.id))
                        .count();
                    if keep_stands && !covered(&store, &g.keep, *sub)? {
                        left += 1;
                    }
                    if left == 0 {
                        self.ensure_receipt_not_required(&store, *e)
                            .map_err(|why| {
                                format!(
                                    "not consolidated: entry {e}'s only evidence is a duplicate, and the kept file can't stand in for it ({why})"
                                )
                            })?;
                    }
                }
            }
        }
        let out_groups: Vec<DuplicateGroupOut> = groups
            .iter()
            .map(|g| DuplicateGroupOut {
                sha256: g.sha.clone(),
                keep: g.keep.id,
                withdraw: g.dups.iter().map(|d| d.id).collect(),
                links: g.link.iter().map(|s| subject_out(&store, *s)).collect(),
            })
            .collect();
        let withdrawn = out_groups.iter().map(|g| g.withdraw.len()).sum();
        let digest = self.preview_of(
            &store,
            "books.consolidate_duplicates",
            confirmed,
            params.0.preview.as_deref(),
        )?;
        if !confirmed {
            return Ok(Json(ConsolidateDuplicatesOut {
                status: "preview".into(),
                groups: out_groups,
                withdrawn,
                preview: Some(digest),
            }));
        }
        for g in &groups {
            for sub in &g.link {
                store
                    .link_attachment(g.keep.id, *sub)
                    .map_err(|e| e.to_string())?;
            }
            let reason = format!("duplicate of #{}", g.keep.id);
            for (id, sub) in &g.unlink {
                // A link a previous step took off (it was on the kept file
                // already) isn't there to take off again.
                if store
                    .attachment_links(*id)
                    .map_err(|e| e.to_string())?
                    .contains(sub)
                {
                    store
                        .unlink_attachment(*id, *sub, &reason)
                        .map_err(|e| e.to_string())?;
                }
            }
            for d in &g.dups {
                if d.entry_id.is_some() {
                    store
                        .unlink_receipt(d.id, &reason)
                        .map_err(|e| e.to_string())?;
                }
                store
                    .withdraw_attachment_duplicate(d.id, &reason, g.keep.id)
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(Json(ConsolidateDuplicatesOut {
            status: "consolidated".into(),
            groups: out_groups,
            withdrawn,
            preview: Some(digest),
        }))
    }
}

/// Every subject a filing is evidence of: the entry it is filed on, and
/// every explicit link.
fn subjects_of(
    store: &Store,
    a: &ledger_store::Attachment,
) -> Result<Vec<ledger_store::Subject>, String> {
    let mut out: Vec<ledger_store::Subject> = a
        .entry_id
        .map(ledger_store::Subject::Entry)
        .into_iter()
        .collect();
    out.extend(store.attachment_links(a.id).map_err(|e| e.to_string())?);
    out.sort();
    out.dedup();
    Ok(out)
}

/// Whether `a` is already evidence of `sub`.
fn covered(
    store: &Store,
    a: &ledger_store::Attachment,
    sub: ledger_store::Subject,
) -> Result<bool, String> {
    if let ledger_store::Subject::Entry(e) = sub
        && a.entry_id == Some(e)
    {
        return Ok(true);
    }
    Ok(store
        .attachment_links(a.id)
        .map_err(|e| e.to_string())?
        .contains(&sub))
}
