use crate::*;

impl Books<'_> {
    /// Where the entry event `q.entry` made stands (BS-entry-5), read from
    /// the engine's own books: a draft's state (with its note when it awaits
    /// evidence), posted (a posted entry has no state of its own), or gone
    /// (a discarded draft leaves the ledger).
    pub(crate) fn answer_draft_state(&self, q: &Query) -> Result<Value, Refusal> {
        let n = q.entry.ok_or_else(|| self.missing("entry"))?;
        let id = self.entry_or_refuse(n)?;
        let mut m = JsonMap::new();
        let state = match self.l().get_entry(id) {
            None => "discarded",
            Some(e) if e.status() == EntryStatus::Posted => "posted",
            Some(e) => match e.draft_state() {
                ledger_core::DraftState::Building => "building",
                ledger_core::DraftState::AwaitingEvidence { note } => {
                    m.insert("note".to_string(), Value::String(note.clone()));
                    "awaiting_evidence"
                }
            },
        };
        m.insert("state".to_string(), Value::String(state.to_string()));
        Ok(Value::Object(m))
    }

    /// The engine's one decision whether the entry event `q.entry` made
    /// needs evidence (`ledger_core::evidence::requirement`, BS-entry-5): the
    /// decision as the engine gives it, its reason or exemption read into
    /// the examples' closed words.
    pub(crate) fn answer_evidence(&self, q: &Query) -> Result<Value, Refusal> {
        use ledger_core::evidence::{Requirement, requirement};
        let n = q.entry.ok_or_else(|| self.missing("entry"))?;
        let id = self.entry_or_refuse(n)?;
        let entry = self
            .l()
            .get_entry(id)
            .ok_or_else(|| self.missing("entry"))?;
        let (decision, because) = match requirement(self.l(), entry, self.l().settings()) {
            Requirement::Required { .. } => ("required", "policy"),
            Requirement::Exempt { source } => (
                "exempt",
                // The engine words the source of an exemption in sentences;
                // the examples' words are the closed list of BS-entry-5.
                match source {
                    s if s.starts_with("the receipt policy") => "policy",
                    s if s.starts_with("a reversal") => "reversal",
                    s if s.starts_with("a period's closing entry") => "closing_entry",
                    s if s.starts_with("a schedule month's posting") => "schedule_posting",
                    s if s.starts_with("a depreciation run") => "depreciation_run",
                    s if s.starts_with("an asset's disposal") => "asset_disposal",
                    s if s.starts_with("a document's entry") || s.starts_with("a payroll") => {
                        "document_entry"
                    }
                    other => {
                        return Err(engine_refusal(
                            RuleId::new(Area::Entry, 5),
                            format!("an exemption with no word in the examples: {other}"),
                        ));
                    }
                },
            ),
        };
        Ok(json_figures([
            ("decision".to_string(), decision.to_string()),
            ("because".to_string(), because.to_string()),
        ]))
    }
}

/// Check a `draft_state` expectation: the state, and the note when the draft
/// awaits evidence, and nothing else.
pub(crate) fn check_draft_state(b: &Books<'_>, x: &Expect) {
    check_figures(b, x, b.answer_draft_state(&x.query()));
}

/// Check an `evidence` expectation: the decision and why, both given.
pub(crate) fn check_evidence(b: &Books<'_>, x: &Expect) {
    check_figures(b, x, b.answer_evidence(&x.query()));
}
