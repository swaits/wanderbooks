//! The guards: the named ways the books refuse a change, each with the
//! tool that remedies it.
//!
//! ```text
//! GOAL: an agent that is refused is told which rule refused it and which
//!       tool to call next, in the same words every time.
//! SOURCE: plan/served.md 4.6 (the result envelope: `guards` carry a code,
//!         a message and a remedy tool); DECISION (P5).
//! INVARIANTS:
//!   1. Every guard names a remedy tool that exists (test
//!      `every_guard_names_an_existing_tool`).
//!   2. A code is spelled one way: the enum's `code()`; a refusal whose
//!      text is `{"code": .., "message": ..}` JSON or says a code in words
//!      becomes the guard of that code, and any other refusal has none.
//! ```

use serde_json::{Value, json};

/// Every guard code the books know.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Guard {
    /// The preview a confirmation carries is not the one the books would
    /// show now.
    StalePreview,
    /// The books wait for the owner's format upgrade.
    FormatUpgradePending,
    /// The evidence policy wants a receipt before this posts.
    ReceiptRequired,
    /// An approval rule wants an approver's go-ahead.
    ApprovalRequired,
    /// The entry's party shares do not add up.
    PartyShares,
    /// The parties setting requires a party (or a no-party reason).
    PartyRequired,
    /// Drafts stand in the way of a close.
    OpenDrafts,
    /// A line on a control account carries no document tag.
    UntaggedControlLine,
    /// A vendor already has a bill with this reference.
    DuplicateVendorRef,
    /// The first invoice number cannot change once one is issued.
    FirstNumberLocked,
    /// No audit stands at the books' current head to rest this on (plan
    /// P8): run `books.audit`.
    AuditRequired,
}

impl Guard {
    /// Every guard, for the registry test.
    pub(crate) const ALL: [Guard; 11] = [
        Guard::StalePreview,
        Guard::FormatUpgradePending,
        Guard::ReceiptRequired,
        Guard::ApprovalRequired,
        Guard::PartyShares,
        Guard::PartyRequired,
        Guard::OpenDrafts,
        Guard::UntaggedControlLine,
        Guard::DuplicateVendorRef,
        Guard::FirstNumberLocked,
        Guard::AuditRequired,
    ];

    /// The code an agent reads.
    pub(crate) fn code(self) -> &'static str {
        match self {
            Guard::StalePreview => "stale_preview",
            Guard::FormatUpgradePending => "format_upgrade_pending",
            Guard::ReceiptRequired => "receipt_required",
            Guard::ApprovalRequired => "approval_required",
            Guard::PartyShares => "party_shares",
            Guard::PartyRequired => "party_required",
            Guard::OpenDrafts => "open_drafts",
            Guard::UntaggedControlLine => "untagged_control_line",
            Guard::DuplicateVendorRef => "duplicate_vendor_ref",
            Guard::FirstNumberLocked => "first_number_locked",
            Guard::AuditRequired => "audit_required",
        }
    }

    /// The guard of a code.
    pub(crate) fn from_code(code: &str) -> Option<Guard> {
        Self::ALL.into_iter().find(|g| g.code() == code)
    }

    /// The tool that remedies it; `tool` is the call that was refused (a
    /// stale preview is remedied by previewing it again).
    pub(crate) fn remedy_tool(self, tool: &str) -> String {
        match self {
            Guard::StalePreview => return tool.to_string(),
            Guard::FormatUpgradePending => "books.start",
            Guard::ReceiptRequired => "books.attach_receipt",
            Guard::ApprovalRequired => "books.approve_entry",
            Guard::PartyShares | Guard::PartyRequired => "books.set_entry_party",
            Guard::OpenDrafts => "books.list_drafts",
            Guard::UntaggedControlLine => "books.control_check",
            Guard::DuplicateVendorRef => "books.search_entries",
            Guard::FirstNumberLocked => "books.settings",
            Guard::AuditRequired => "books.audit",
        }
        .to_string()
    }
}

/// The guards a refusal's text carries, as the envelope shows them:
/// `[{code, message, remedy_tool, remedy_args?}]`. `tool` is the refused
/// call.
pub(crate) fn from_refusal(tool: &str, text: &str) -> Vec<Value> {
    // A structured refusal: the JSON the tool built.
    if let Ok(Value::Object(o)) = serde_json::from_str::<Value>(text)
        && let Some(code) = o.get("code").and_then(Value::as_str)
    {
        let remedy = o
            .get("remedy_tool")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| Guard::from_code(code).map(|g| g.remedy_tool(tool)));
        if let Some(remedy) = remedy {
            let mut g = json!({
                "code": code,
                "message": o.get("message").and_then(Value::as_str).unwrap_or(code),
                "remedy_tool": remedy,
            });
            if let Some(entry) = o.get("entry_id") {
                g["remedy_args"] = json!({ "entry_id": entry });
            }
            return vec![g];
        }
        return Vec::new();
    }
    // A refusal that names its code in words (the store's).
    for g in Guard::ALL {
        if text.starts_with(g.code()) {
            return vec![json!({
                "code": g.code(),
                "message": text,
                "remedy_tool": g.remedy_tool(tool),
            })];
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::BooksServer;

    #[test]
    fn every_guard_names_an_existing_tool() {
        let names: Vec<String> = BooksServer::tool_router()
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        for g in Guard::ALL {
            let remedy = g.remedy_tool("books.post_entry");
            assert!(
                names.contains(&remedy),
                "guard {} names {remedy}, which is no tool",
                g.code()
            );
            assert_eq!(Guard::from_code(g.code()), Some(g));
        }
    }

    #[test]
    fn a_refusal_becomes_its_guard() {
        let text = json!({"code":"approval_required","message":"needs approval","entry_id":7,"remedy_tool":"books.approve_entry"}).to_string();
        let g = from_refusal("books.post_entry", &text);
        assert_eq!(g[0]["code"], "approval_required");
        assert_eq!(g[0]["remedy_args"]["entry_id"], 7);
        let g = from_refusal(
            "books.create_invoice",
            "format_upgrade_pending: this needs documents_v2",
        );
        assert_eq!(g[0]["remedy_tool"], "books.start");
        assert!(from_refusal("books.x", "no such account").is_empty());
    }
}
