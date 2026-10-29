//! The drafts group (plan P4; rules.md B-43): which drafts are open and
//! what they wait for, a draft's state, throwing one away, and the full
//! preview of the post. Also what every report says is still open as of
//! its end (`as_of_status`) and the open-draft refusal of the CPA package.
//!
//! ```text
//! GOAL: a draft is never lost and never mistaken for a posted entry; a
//!       report or a package says when drafts dated in it are missing
//!       from its figures.
//! SOURCE: docs/spec/rules.md B-43; plan/served.md 4.8, D-47.
//! INVARIANTS:
//!   1. A state is a note on the draft: no figure moves.
//!   2. A discard is previewed, then confirmed with the preview's digest;
//!      only a draft is discarded.
//!   3. `books.preview_entry` is `books.post_entry` without `confirmed`:
//!      it changes nothing and returns the digest a confirm needs.
//!   4. A strict report refuses, naming what is open, instead of warning.
//! ```

use super::*;
use ledger_core::DraftState;

impl BooksServer {
    /// A draft as the tools list it.
    pub(crate) fn draft_out(
        &self,
        store: &Store,
        e: &ledger_core::JournalEntry,
    ) -> Result<DraftOut, String> {
        let ledger = store.ledger();
        let required =
            ledger_core::evidence::requirement(ledger, e, ledger.settings()).is_required();
        let receipts = store
            .list_attachments(Some(e.id()))
            .map_err(|e| e.to_string())?;
        Ok(DraftOut {
            entry_id: e.id(),
            date: e.date().to_string(),
            description: e.description().to_string(),
            total: ledger_core::evidence::Policy::entry_debits(e)
                .map(money)
                .unwrap_or_else(|_| "too large".to_string()),
            state: e.draft_state().key().to_string(),
            note: e.draft_state().note().to_string(),
            missing_evidence: required && receipts.is_empty(),
        })
    }

    /// What is open as of `end`: drafts dated on or before it, accounts
    /// not reconciled through the month before it, settings problems.
    pub(crate) fn as_of_status(
        &self,
        store: &Store,
        end: NaiveDate,
    ) -> Result<AsOfStatusOut, String> {
        let ledger = store.ledger();
        let mut drafts = Vec::new();
        for e in ledger.drafts() {
            if e.date() <= end {
                drafts.push(self.draft_out(store, e)?);
            }
        }
        drafts.sort_by(|a, b| a.date.cmp(&b.date).then(a.entry_id.cmp(&b.entry_id)));
        Ok(AsOfStatusOut {
            drafts_open_on_or_before: drafts,
            unreconciled: self.unreconciled_accounts(store, end)?,
            settings_problems: settings_problems(ledger, &self.settings()),
        })
    }

    /// A report's `as_of_status`, and under `strict` the refusal that
    /// names what is open (INVARIANT 4).
    pub(crate) fn report_status(
        &self,
        store: &Store,
        end: NaiveDate,
        strict: bool,
    ) -> Result<Option<AsOfStatusOut>, String> {
        let status = self.as_of_status(store, end)?;
        let open = !status.drafts_open_on_or_before.is_empty()
            || !status.unreconciled.is_empty()
            || !status.settings_problems.is_empty();
        if strict && open {
            let mut what = Vec::new();
            if !status.drafts_open_on_or_before.is_empty() {
                what.push(format!(
                    "{} draft(s) dated on or before {end} are open (books.list_drafts)",
                    status.drafts_open_on_or_before.len()
                ));
            }
            if !status.unreconciled.is_empty() {
                what.push(format!(
                    "{} account(s) are not reconciled (books.digest)",
                    status.unreconciled.len()
                ));
            }
            if !status.settings_problems.is_empty() {
                what.push(format!(
                    "{} setting(s) disagree with the chart (books.settings)",
                    status.settings_problems.len()
                ));
            }
            return Err(format!(
                "strict: the figures as of {end} are not final: {}. Post or discard the drafts, or ask again without strict",
                what.join("; ")
            ));
        }
        Ok(Some(status))
    }
}

#[tool_router(router = drafts_router, vis = "pub(super)")]
impl BooksServer {
    #[tool(
        name = "books.list_drafts",
        title = "List draft entries",
        description = "The draft entries still open: id, date, description, total, state (building, or awaiting_evidence with a note saying what it waits for) and whether the receipt policy covers it and it has no receipt yet (missing_evidence). state filters to one state; as_of to drafts dated on or before a day. A draft is not in any report until it posts; a period holding an open draft cannot close.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_drafts(
        &self,
        params: Parameters<ListDraftsParams>,
    ) -> Result<Json<ListDraftsOut>, String> {
        let p = params.0;
        let state = match p.state.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(s) => Some(
                DraftState::parse(s, "")
                    .ok_or_else(|| format!("unknown state {s:?}: building or awaiting_evidence"))?,
            ),
        };
        let as_of = p.as_of.as_deref().map(parse_date).transpose()?;
        let store = self.lock()?;
        let mut drafts = Vec::new();
        for e in store.ledger().drafts() {
            if state
                .as_ref()
                .is_some_and(|s| s.key() != e.draft_state().key())
            {
                continue;
            }
            if as_of.is_some_and(|d| e.date() > d) {
                continue;
            }
            drafts.push(self.draft_out(&store, e)?);
        }
        drafts.sort_by(|a, b| a.date.cmp(&b.date).then(a.entry_id.cmp(&b.entry_id)));
        Ok(Json(ListDraftsOut { drafts }))
    }

    #[tool(
        name = "books.set_draft_state",
        title = "Set a draft's state",
        description = "Say where a draft stands: building (still being worked on) or awaiting_evidence with a note saying what it waits for (\"Figma invoice due 11/3\"). A state is a note: no line or figure changes, and the receipt policy is still checked when the draft posts. Only a draft has one. Needs the books' format upgrade (drafts_v2).",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn set_draft_state(
        &self,
        params: Parameters<SetDraftStateParams>,
    ) -> Result<Json<SetDraftStateOut>, String> {
        let p = params.0;
        let note = p.note.as_deref().map(str::trim).unwrap_or("");
        let state = DraftState::parse(p.state.trim(), note).ok_or_else(|| {
            format!(
                "unknown state {:?}: building or awaiting_evidence",
                p.state.trim()
            )
        })?;
        if matches!(state, DraftState::Building) && !note.is_empty() {
            return Err("a note goes with awaiting_evidence: say what the draft waits for".into());
        }
        let mut store = self.lock()?;
        store
            .set_draft_state(p.entry_id, state.clone())
            .map_err(|e| e.to_string())?;
        Ok(Json(SetDraftStateOut {
            entry_id: p.entry_id,
            state: state.key().to_string(),
            note: state.note().to_string(),
        }))
    }

    #[tool(
        name = "books.discard_draft",
        title = "Discard a draft",
        description = "Throw away a draft that will never post, with the reason (journaled). Only a draft: a posted entry is never removed, it is reversed. Nothing in any report moves; the draft's id is not reused; a period it held open can close. Without confirmed=true, a preview that changes nothing and returns the preview to pass back with confirmed=true. Needs the books' format upgrade (drafts_v2).",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn discard_draft(
        &self,
        params: Parameters<DiscardDraftParams>,
    ) -> Result<Json<DiscardDraftOut>, String> {
        let p = params.0;
        let confirmed = p.confirmed.unwrap_or(false);
        let mut store = self.lock()?;
        let e = store
            .ledger()
            .get_entry(p.entry_id)
            .ok_or_else(|| format!("unknown entry {}", p.entry_id))?
            .clone();
        if e.status() != ledger_core::EntryStatus::Draft {
            return Err(format!(
                "entry {} is posted: a posted entry is never removed, reverse it (books.reverse_entry)",
                p.entry_id
            ));
        }
        if p.reason.trim().is_empty() {
            return Err("give the reason the draft will not post".into());
        }
        // The draft as it stands, so a line added since is a stale preview.
        let payload = serde_json::json!({
            "tool": "books.discard_draft",
            "entry": &e,
            "reason": p.reason.trim(),
            "settings_seq": store.ledger().settings().last_seq(),
        });
        let digest = Preview::check(
            "books.discard_draft",
            &payload,
            confirmed,
            p.preview.as_deref(),
        )?;
        let holds_open: Vec<String> = store
            .ledger()
            .list_periods()
            .into_iter()
            .filter(|per| per.status == ledger_core::PeriodStatus::Open && per.contains(e.date()))
            .map(|per| per.name.clone())
            .collect();
        let mut out = DiscardDraftOut {
            status: "preview".into(),
            entry_id: p.entry_id,
            description: e.description().to_string(),
            total: ledger_core::evidence::Policy::entry_debits(&e)
                .map(money)
                .unwrap_or_else(|_| "too large".to_string()),
            holds_open,
            preview: Some(digest),
        };
        if !confirmed {
            return Ok(Json(out));
        }
        store
            .discard_draft(p.entry_id, &p.reason)
            .map_err(|e| e.to_string())?;
        out.status = "discarded".into();
        Ok(Json(out))
    }

    #[tool(
        name = "books.preview_entry",
        title = "Preview posting a draft",
        description = "What posting a draft would do, without posting: the lines, the receipt checklist and whether the receipt policy needs one, the party, any approvals missing, and the preview to pass to books.post_entry with confirmed=true. The same answer as books.post_entry without confirmed, as a read-only call.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn preview_entry(
        &self,
        params: Parameters<PreviewEntryParams>,
    ) -> Result<Json<PostEntryOut>, String> {
        self.post_entry(Parameters(PostEntryParams {
            entry_id: params.0.entry_id,
            confirmed: None,
            expected_total: None,
            preview: None,
        }))
        .await
    }
}
