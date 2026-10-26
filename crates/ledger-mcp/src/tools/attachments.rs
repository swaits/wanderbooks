//! Evidence through the server: receipts and filed documents, and their
//! links to whatever they belong to. Split whole out of `server.rs`
//! (plan/served.md 8.2 U0a).

use super::*;

/// Resolve a tool's subject: its kind, and its id resolved the way the
/// other tools resolve references (an account by code or id, an asset by
/// tag or id, a party by id or name), checked to exist.
fn resolve_subject(store: &Store, s: &SubjectParam) -> Result<ledger_store::Subject, String> {
    use ledger_store::Subject;
    let kind = s.kind.trim();
    let text = match &s.id {
        None => None,
        Some(RefId::Number(n)) => Some(n.to_string()),
        Some(RefId::Text(t)) => Some(t.trim().to_string()),
    };
    if Subject::from_parts(kind, 0).is_none() {
        return Err(format!(
            "unknown subject kind {kind:?}: expected one of {SUBJECT_KINDS}"
        ));
    }
    if kind == "company" {
        return match text {
            None => Ok(Subject::Company),
            Some(_) => Err("the company subject takes no id".into()),
        };
    }
    let t = text.ok_or_else(|| format!("a {kind} subject needs its id"))?;
    let sub = match kind {
        "account" => Subject::Account(resolve_account(store, &t)?),
        "asset" => Subject::Asset(resolve_asset_ref(store, &t)?),
        "party" => Subject::Party(resolve_party(store, &t)?),
        // A JSON number is an id, never a name.
        "project" => match &s.id {
            Some(RefId::Number(n)) => Subject::Project(*n),
            _ => Subject::Project(u64::from(resolve_class(store, &t)?)),
        },
        k => {
            let id = t
                .parse::<u64>()
                .map_err(|_| format!("a {k} subject's id is a number, got {t:?}"))?;
            Subject::from_parts(k, id).expect("kind checked")
        }
    };
    if !store.subject_exists(sub).map_err(|e| e.to_string())? {
        return Err(format!("{} doesn't exist", sub.describe()));
    }
    Ok(sub)
}

/// Check every `link_to` subject before anything is filed: each resolves,
/// its journal feature is declared, none twice, none the entry the file is
/// filed on (all or nothing).
fn check_link_to(
    store: &Store,
    filed_on: Option<u64>,
    link_to: &[SubjectParam],
) -> Result<Vec<ledger_store::Subject>, String> {
    let mut out: Vec<ledger_store::Subject> = Vec::new();
    for s in link_to {
        let sub = resolve_subject(store, s)?;
        // A link to a subject kind an older version doesn't know needs its
        // journal feature, asked here, before the file is filed: a refusal
        // leaves nothing half-filed (rules.md B-27).
        if let Some(f) = Store::subject_feature(sub) {
            store.ensure_feature(f).map_err(|e| e.to_string())?;
        }
        if out.contains(&sub) {
            return Err(format!("link_to names {} twice", sub.describe()));
        }
        if filed_on.map(ledger_store::Subject::Entry) == Some(sub) {
            return Err(format!(
                "link_to names {}, which it is filed on already",
                sub.describe()
            ));
        }
        out.push(sub);
    }
    Ok(out)
}

#[tool_router(router = attachments_router, vis = "pub(super)")]
impl BooksServer {
    #[tool(
        name = "books.attach_receipt",
        title = "Attach a receipt",
        description = "Attach a receipt to an entry, or file it unlinked and link later with books.link_receipt. The bytes: upload_id, from books.begin_upload and `wanderbooks upload` (a file never passes through you). Files are content-addressed and deduplicated. link_to links it to more subjects in the same call (another entry it also supports, an asset, a party, a project, ...: see books.link_attachment); every subject is checked first, so one bad subject files nothing. A file holding a full TIN is refused: one the server read or wrote with TINs, or text with a TIN the vault keeps or an SSN or EIN printed 3-2-4 or 2-7 with hyphens.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn attach_receipt(
        &self,
        params: Parameters<AttachReceiptParams>,
    ) -> Result<Json<AttachReceiptOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        if let Some(e) = p.entry_id
            && store.ledger().get_entry(e).is_none()
        {
            return Err(format!("unknown entry {e}"));
        }
        let link_to = check_link_to(&store, p.entry_id, &p.link_to)?;
        // Taken once everything else is known good.
        let file = self.take_file(files::Given {
            upload_id: &p.upload_id,
            purposes: files::FILED,
        })?;
        let (file_name, mime_type) = named(p.file_name, p.mime_type, &file)?;
        let (bytes, claim) = (file.bytes, file.claim);
        self.refuse_tin_file(&store, &bytes, &mime_type, &file_name)?;
        let (attachment_id, sha256, deduplicated) = store
            .attach_receipt(
                p.entry_id,
                &file_name,
                &mime_type,
                &bytes,
                p.description.as_deref(),
            )
            .map_err(|e| e.to_string())?;
        files::filed(claim);
        for sub in link_to {
            store
                .link_attachment(attachment_id, sub)
                .map_err(|e| e.to_string())?;
        }
        Ok(Json(AttachReceiptOut {
            links: links_out(&store, attachment_id)?,
            attachment_id,
            sha256,
            deduplicated,
        }))
    }

    #[tool(
        name = "books.file_document",
        title = "File a document",
        description = "File a document: category formation, legal, tax, accounting, statement (a bank or card statement: then books.record_statement), contract, photo, warranty, import_source (a bank export), or other. link_to links it to subjects in the same call (an account, an asset, a party, a project, the company: see books.link_attachment), all checked first. The bytes: upload_id, from books.begin_upload and `wanderbooks upload` (a file never passes through you). Never a W-9 (books.set_w9 w9_upload_id puts it in the vault). A file holding a full TIN is refused: one the server read or wrote with TINs, or text with a TIN the vault keeps or an SSN or EIN printed 3-2-4 or 2-7 with hyphens.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn file_document(
        &self,
        params: Parameters<FileDocumentParams>,
    ) -> Result<Json<AttachReceiptOut>, String> {
        let p = params.0;
        match p.category.as_str() {
            "formation" | "legal" | "tax" | "accounting" | "statement" | "contract" | "photo"
            | "warranty" | "import_source" | "other" => {}
            _ => {
                return Err(format!(
                    "bad category {:?}: expected formation, legal, tax, accounting, statement, contract, photo, warranty, import_source, or other",
                    p.category
                ));
            }
        }
        let mut store = self.lock()?;
        let link_to = check_link_to(&store, None, &p.link_to)?;
        let file = self.take_file(files::Given {
            upload_id: &p.upload_id,
            purposes: files::FILED,
        })?;
        let (file_name, mime_type) = named(p.file_name, p.mime_type, &file)?;
        let (bytes, claim) = (file.bytes, file.claim);
        self.refuse_tin_file(&store, &bytes, &mime_type, &file_name)?;
        let (attachment_id, sha256, deduplicated) = store
            .file_document(
                &p.category,
                &file_name,
                &mime_type,
                &bytes,
                p.description.as_deref(),
            )
            .map_err(|e| e.to_string())?;
        files::filed(claim);
        for sub in link_to {
            store
                .link_attachment(attachment_id, sub)
                .map_err(|e| e.to_string())?;
        }
        Ok(Json(AttachReceiptOut {
            links: links_out(&store, attachment_id)?,
            attachment_id,
            sha256,
            deduplicated,
        }))
    }

    #[tool(
        name = "books.link_receipt",
        title = "Link a receipt to an entry",
        description = "Link a previously filed (unlinked) receipt to an entry.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn link_receipt(
        &self,
        params: Parameters<LinkReceiptParams>,
    ) -> Result<Json<LinkReceiptOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        store
            .link_receipt(p.attachment_id, p.entry_id)
            .map_err(|e| e.to_string())?;
        Ok(Json(LinkReceiptOut { ok: true }))
    }

    #[tool(
        name = "books.unlink_receipt",
        title = "Unlink a receipt",
        description = "Detach a receipt from its entry, with a reason recorded in the journal. The receipt stays filed (unlinked); link it elsewhere or withdraw it with books.withdraw_attachment. The receipt policy still holds: an entry that requires a receipt cannot be left with none, so to replace a receipt attach the new one first, then unlink the old.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn unlink_receipt(
        &self,
        params: Parameters<UnlinkReceiptParams>,
    ) -> Result<Json<UnlinkReceiptOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let attachment = store
            .list_attachments(None)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|a| a.id == p.attachment_id)
            .ok_or_else(|| format!("unknown attachment {}", p.attachment_id))?;
        if let Some(entry_id) = attachment.entry_id {
            let remaining = store
                .list_attachments(Some(entry_id))
                .map_err(|e| e.to_string())?
                .len()
                - 1;
            if remaining == 0 {
                self.ensure_receipt_not_required(&store, entry_id)?;
            }
        }
        let entry_id = store
            .unlink_receipt(p.attachment_id, &p.reason)
            .map_err(|e| e.to_string())?;
        let remaining_receipts = store
            .list_attachments(Some(entry_id))
            .map_err(|e| e.to_string())?
            .len() as u64;
        Ok(Json(UnlinkReceiptOut {
            ok: true,
            entry_id,
            remaining_receipts,
        }))
    }

    #[tool(
        name = "books.withdraw_attachment",
        title = "Withdraw a receipt or document",
        description = "Withdraw an unlinked receipt or a filed document, with a reason recorded in the journal. It drops out of books.list_receipts and can never be linked again. Nothing is deleted: the bytes stay in the blob store and the journal keeps the filing and the withdrawal. Unlink it first (books.unlink_receipt from its entry, books.unlink_attachment from anything else).",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn withdraw_attachment(
        &self,
        params: Parameters<WithdrawAttachmentParams>,
    ) -> Result<Json<WithdrawAttachmentOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        store
            .withdraw_attachment(p.attachment_id, &p.reason)
            .map_err(|e| e.to_string())?;
        Ok(Json(WithdrawAttachmentOut { ok: true }))
    }

    /// ```text
    /// GOAL: one file, linked wherever it is evidence, without refiling.
    /// SOURCE: docs/parties-and-attachments.md "Attachments";
    ///         Store::link_attachment.
    /// INVARIANTS:
    ///   1. Journaled as `attachment.linked`; a refusal journals nothing.
    ///   2. A file linked to an entry counts as that entry's receipt.
    /// SCENARIOS:
    ///   1. IF the attachment is unknown or withdrawn, the subject kind is
    ///      unknown, or the subject doesn't exist: SHOULD refuse.
    ///   2. IF it is already linked there (or filed on that entry): SHOULD
    ///      refuse.
    /// ```
    #[tool(
        name = "books.link_attachment",
        title = "Link a file to a subject",
        description = "Link a stored receipt or document to a subject, so one file is evidence wherever it applies without filing it again. subject: {kind, id}: entry (id), account (code or id: its statements, the agreement that opened it), asset (tag or id: photos, warranty, manual), party (id or name: contracts, correspondence), project (id or name: its contract, spec, pitch), invoice, bill, period (its close package), payroll_run, schedule (the prepaid contract), reconciliation, or company (no id: formation documents). A file linked to an entry counts as one of its receipts. Journaled as attachment.linked. Returns the attachment's links.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn link_attachment(
        &self,
        params: Parameters<LinkAttachmentParams>,
    ) -> Result<Json<AttachmentLinksOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        find_attachment(&store, p.attachment_id)?;
        let sub = resolve_subject(&store, &p.subject)?;
        store
            .link_attachment(p.attachment_id, sub)
            .map_err(|e| e.to_string())?;
        Ok(Json(AttachmentLinksOut {
            attachment_id: p.attachment_id,
            links: links_out(&store, p.attachment_id)?,
        }))
    }

    /// ```text
    /// GOAL: take a link off, on the record, without leaving evidence
    ///       missing where a rule needs it.
    /// SOURCE: R-13a (receipt corrections are append-only events);
    ///         Store::unlink_attachment.
    /// INVARIANTS:
    ///   1. Journaled as `attachment.unlinked` with the reason; the file
    ///      stays filed. A refusal journals nothing.
    /// SCENARIOS:
    ///   1. IF the reason is blank, or it isn't linked to the subject:
    ///      SHOULD refuse.
    ///   2. IF it is filed on that entry (rather than linked): SHOULD refuse,
    ///      naming books.unlink_receipt.
    ///   3. IF it is a posted entry's last receipt and the receipt policy
    ///      covers the entry: SHOULD refuse (receipt_required).
    ///   4. IF it is the statement of a reconciliation not voided: SHOULD
    ///      refuse (void the reconciliation first).
    /// ```
    #[tool(
        name = "books.unlink_attachment",
        title = "Unlink a file from a subject",
        description = "Take a link (books.link_attachment) off a subject, with the reason recorded in the journal. The file stays filed and keeps its other links. A receipt filed on an entry comes off with books.unlink_receipt instead. Refused when it would leave a posted entry the receipt policy covers without a receipt (attach the replacement first), or take the statement off a reconciliation that isn't voided. Journaled as attachment.unlinked. Returns the attachment's links.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn unlink_attachment(
        &self,
        params: Parameters<UnlinkAttachmentParams>,
    ) -> Result<Json<AttachmentLinksOut>, String> {
        use ledger_store::Subject;
        let p = params.0;
        let mut store = self.lock()?;
        let a = find_attachment(&store, p.attachment_id)?;
        let sub = resolve_subject(&store, &p.subject)?;
        let linked = store
            .attachment_links(a.id)
            .map_err(|e| e.to_string())?
            .contains(&sub);
        match sub {
            Subject::Entry(e) if a.entry_id == Some(e) => {
                return Err(format!(
                    "attachment {} is filed on entry {e}, not linked to it: take it off with books.unlink_receipt",
                    a.id
                ));
            }
            Subject::Entry(e) if linked => {
                let others = store
                    .list_attachments(Some(e))
                    .map_err(|e| e.to_string())?
                    .iter()
                    .filter(|x| x.id != a.id)
                    .count();
                if others == 0 {
                    self.ensure_receipt_not_required(&store, e)?;
                }
            }
            Subject::Reconciliation(r)
                if store
                    .reconciliations(None)
                    .map_err(|e| e.to_string())?
                    .iter()
                    .any(|x| x.id == r && x.attachment_id == a.id && x.voided.is_none()) =>
            {
                return Err(format!(
                    "attachment {} is the statement reconciliation {r} rests on: void the reconciliation first (books.void_reconciliation)",
                    a.id
                ));
            }
            _ => {}
        }
        store
            .unlink_attachment(a.id, sub, &p.reason)
            .map_err(|e| e.to_string())?;
        Ok(Json(AttachmentLinksOut {
            attachment_id: a.id,
            links: links_out(&store, a.id)?,
        }))
    }

    #[tool(
        name = "books.attachments",
        title = "Files linked to a subject",
        description = "The files on a subject ({kind, id} as in books.link_attachment): for an entry, the receipts filed on it and the files linked to it; for anything else, the files linked to it. Withdrawn files are left out. Each comes with all its links.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn attachments(
        &self,
        params: Parameters<AttachmentsParams>,
    ) -> Result<Json<AttachmentsOut>, String> {
        let store = self.lock()?;
        let sub = resolve_subject(&store, &params.0.subject)?;
        let attachments = store
            .attachments_for(sub)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|a| receipt_out(&store, a))
            .collect::<Result<_, _>>()?;
        Ok(Json(AttachmentsOut {
            subject: subject_out(&store, sub),
            attachments,
        }))
    }

    #[tool(
        name = "books.list_receipts",
        title = "List receipts and documents",
        description = "List receipts and filed documents, optionally filtered to one entry (filed on it or linked to it), each with everything it is linked to (links). Withdrawn attachments are hidden unless include_withdrawn=true. For any other subject's files, books.attachments.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_receipts(
        &self,
        params: Parameters<ListReceiptsParams>,
    ) -> Result<Json<ListReceiptsOut>, String> {
        let p = params.0;
        let store = self.lock()?;
        let include_withdrawn = p.include_withdrawn.unwrap_or(false);
        let receipts = store
            .list_attachments(p.entry_id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|a| include_withdrawn || a.withdrawn_at.is_none())
            .map(|a| receipt_out(&store, a))
            .collect::<Result<_, _>>()?;
        Ok(Json(ListReceiptsOut { receipts }))
    }

    #[tool(
        name = "books.get_receipt",
        title = "Get a receipt",
        description = "A receipt or document: what it is and everything it is linked to. view bytes adds the file, handed over: run its download (wanderbooks download) to save a copy in a folder you name; its bytes never come back here.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    pub(super) async fn get_receipt(
        &self,
        params: Parameters<GetReceiptParams>,
    ) -> Result<Json<GetReceiptOut>, String> {
        let p = params.0;
        let bytes_wanted = match p.view.as_deref().unwrap_or("meta") {
            "meta" => false,
            "bytes" => true,
            other => {
                return Err(format!(
                    "view {:?}: meta (the default) or bytes",
                    other.chars().take(20).collect::<String>()
                ));
            }
        };
        let store = self.lock()?;
        let meta = find_attachment(&store, p.attachment_id)?;
        // Handed over, read hash-verified (D-46): never its bytes here.
        let file = if bytes_wanted {
            let bytes = store
                .attachment_bytes(p.attachment_id)
                .map_err(|e| e.to_string())?;
            Some(self.hand_over(&book_name(&store), &meta.file_name, &meta.mime_type, bytes)?)
        } else {
            None
        };
        Ok(Json(GetReceiptOut {
            file,
            links: links_out(&store, meta.id)?,
            description: meta.description.clone(),
            attachment_id: meta.id,
            sha256: meta.blob_sha,
            entry_id: meta.entry_id,
            category: meta.category,
            file_name: meta.file_name,
            mime_type: meta.mime_type,
            size_bytes: meta.size_bytes,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::test_support::*;

    /// Unlinking a receipt is refused only from a posted, non-reversal
    /// entry the policy covers (crate::policy).
    #[test]
    fn receipts_are_kept_only_where_the_policy_needs_them() {
        let books = unit_dir("unlink");
        let mut s = store(&books);
        let cash = s
            .add_account("1000", "Cash", AccountType::Asset, None, false)
            .unwrap();
        let rent = s
            .add_account("6000", "Rent", AccountType::Expense, None, false)
            .unwrap();
        let draft = s.create_entry(d(2026, 3, 1), "draft").unwrap();
        s.add_line(draft, rent, Side::Debit, usd("10.00")).unwrap();
        s.add_line(draft, cash, Side::Credit, usd("10.00")).unwrap();
        let posted = post(&mut s, d(2026, 3, 2), rent, cash, "10.00");
        let reversal = s.reverse_entry(posted, d(2026, 3, 3), "undo").unwrap();
        let server = served(settled(
            s,
            &[("policy.evidence.threshold", serde_json::json!("0.00"))],
        ));
        let store = server.lock().unwrap();
        assert!(
            server.ensure_receipt_not_required(&store, draft).is_ok(),
            "a draft"
        );
        assert!(
            server.ensure_receipt_not_required(&store, reversal).is_ok(),
            "a reversal"
        );
        assert!(
            server.ensure_receipt_not_required(&store, posted).is_err(),
            "a posted entry"
        );
    }
}
