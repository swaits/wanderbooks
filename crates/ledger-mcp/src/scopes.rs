//! What each tool needs of a token (plan 3.3).
//!
//! ```text
//! GOAL: a token does only what its scopes allow: reading needs
//!       books:read, changing what isn't posted books:write, posting and
//!       what is as final books:post, the access log books:admin.
//! SOURCE: plan 3.3's scope table ("books:post: confirmed post, close,
//!         reconcile, dispose, run_*, settings commit").
//! INVARIANTS:
//!   1. Every tool is in exactly one row of TABLE (test).
//!   2. A tool that changes anything (no readOnlyHint) needs more than
//!      books:read; a read-only one needs books:read, except reading the
//!      access log, which needs books:admin (test).
//!   3. A tool with a `confirmed` argument (a preview, then the act)
//!      needs books:write to preview and books:post when confirmed is
//!      true.
//!   4. What gates posting needs posting's scope: setting an approval
//!      rule, retiring any convention (one might be an approval rule),
//!      and approving an entry need books:post. Every other tool needs one
//!      scope whatever it is given.
//!   5. No token below books:admin gets a full TIN: until P7 moves TINs
//!      to the human pages, the tools that can put one out (vendor_1099
//!      with include_tins, export_cpa_package, get_w9) need books:admin,
//!      and sealing one into the vault (set_tin, set_w9, which can replace
//!      the one there) needs books:post (review of slice 4).
//!   6. Where the books' backups go is books:admin's, as backing up is
//!      (a backup is the whole book, its vault and key included):
//!      setting a `backup.*` key needs books:admin; other settings
//!      books:post as before (review of slice 7).
//! ```

use ledger_store::auth::Scope;

/// What a tool needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Needs {
    /// This scope, always.
    Always(Scope),
    /// books:write to preview, books:post when `confirmed` is true
    /// (INVARIANT 3).
    PostWhenConfirmed,
    /// books:post for an approval rule (the control that gates posting),
    /// books:write for any other convention (INVARIANT 4).
    PostForApprovalRules,
    /// books:admin for a change to a `backup.*` setting, books:post for
    /// any other (INVARIANT 6).
    AdminForBackupSettings,
}

use Needs::{AdminForBackupSettings, Always, PostForApprovalRules, PostWhenConfirmed};
const R: Needs = Always(Scope::Read);
const W: Needs = Always(Scope::Write);
const P: Needs = Always(Scope::Post);
const A: Needs = Always(Scope::Admin);
const C: Needs = PostWhenConfirmed;
const K: Needs = PostForApprovalRules;

/// Every tool and what it needs (INVARIANT 1).
pub const TABLE: [(&str, Needs); 134] = [
    ("books.handbook", R),
    ("books.start", R),
    // Settings are committed at once: the settings commit of the plan.
    ("books.set_setting", AdminForBackupSettings),
    ("books.export_settings", R),
    ("books.conventions", R),
    ("books.set_convention", K),
    ("books.retire_convention", P),
    ("books.record_decision", W),
    ("books.decisions", R),
    ("books.raise_item", W),
    ("books.resolve_item", W),
    ("books.open_items", R),
    ("books.draft_from_template", W),
    ("books.approve_entry", P),
    ("books.chart", R),
    ("books.add_account", W),
    ("books.trial_balance", R),
    ("books.profit_and_loss", R),
    ("books.balance_sheet", R),
    ("books.cash_flow", R),
    ("books.draft_entry", W),
    ("books.add_line", W),
    ("books.post_entry", C),
    ("books.reverse_entry", C),
    ("books.attach_receipt", W),
    ("books.file_document", W),
    ("books.link_receipt", W),
    ("books.unlink_receipt", W),
    ("books.withdraw_attachment", W),
    ("books.link_attachment", W),
    ("books.unlink_attachment", W),
    ("books.attachments", R),
    ("books.list_receipts", R),
    ("books.get_receipt", R),
    ("books.get_entry", R),
    ("books.search_entries", R),
    ("books.show", R),
    // It may rebuild the database from the journal: not read-only.
    ("books.verify", W),
    ("books.digest", R),
    ("books.import_bank_csv", W),
    ("books.create_schedule", W),
    ("books.list_schedules", R),
    ("books.run_schedules", C),
    ("books.create_recurring", W),
    ("books.list_recurring", R),
    ("books.run_recurring", P),
    ("books.record_payout", W),
    ("books.list_vendors", R),
    ("books.set_vendor_1099", W),
    ("books.set_entry_vendor", W),
    ("books.add_instrument", W),
    ("books.instruments", R),
    ("books.set_entry_instrument", W),
    // Until P7 these can put full TINs out (with include_tins, or a W-9
    // itself): only an admin token, never a plain one (plan 3.3, "no
    // scope unwraps the vault", holds for every other scope).
    ("books.vendor_1099", A),
    ("books.chart_template", R),
    ("books.tax_categories", R),
    ("books.renumber_account", P),
    ("books.rename_account", P),
    ("books.label_asset", W),
    ("books.correct_asset", P),
    ("books.set_asset_tax_basis", W),
    ("books.asset_labels", R),
    ("books.settings", R),
    ("books.roles", R),
    ("books.assign_role", P),
    ("books.set_tax_category", W),
    ("books.import_chart_csv", W),
    ("books.begin_upload", W),
    ("books.export", R),
    ("books.register_asset", W),
    ("books.asset_schedule", R),
    ("books.list_assets", R),
    ("books.run_depreciation", P),
    ("books.dispose_asset", C),
    ("books.create_period", W),
    ("books.close_period", C),
    ("books.list_periods", R),
    ("books.mark_cleared", W),
    ("books.reconcile", C),
    ("books.record_statement", W),
    ("books.statements", R),
    ("books.reconciliations", R),
    ("books.void_statement", P),
    ("books.void_reconciliation", P),
    ("books.add_customer", W),
    ("books.add_vendor", W),
    ("books.add_party", W),
    ("books.update_party", W),
    ("books.set_party_roles", W),
    // Sealing a TIN can replace the one in the vault.
    ("books.set_w9", P),
    ("books.set_tin", P),
    ("books.get_w9", A),
    ("books.purge_tin", C),
    ("books.set_w4", W),
    ("books.parties", R),
    ("books.party", R),
    ("books.set_entry_party", W),
    // These preview, then post when confirmed (P4b S4.4, A-1).
    ("books.create_invoice", C),
    ("books.create_bill", C),
    ("books.receive_payment", C),
    ("books.pay_bill", C),
    ("books.void_document", P),
    // Records a send: changes the books' record, never a figure.
    ("books.mark_sent", W),
    ("books.control_check", R),
    ("books.documents", R),
    ("books.document", R),
    ("books.ar_aging", R),
    ("books.ap_aging", R),
    ("books.bills_due", R),
    ("books.set_fx_rate", W),
    ("books.run_payroll", P),
    ("books.remit_payroll_taxes", P),
    ("books.list_payroll_runs", R),
    ("books.add_class", W),
    ("books.list_classes", R),
    ("books.profit_and_loss_by_class", R),
    ("books.add_project", W),
    ("books.update_project", W),
    ("books.projects", R),
    ("books.project", R),
    ("books.export_cpa_package", A),
    ("books.access_log", A),
    ("books.backup", R),
    ("books.back_up_now", A),
    // P4b S7: a bad-debt write-off (preview, then post).
    ("books.write_off_invoice", C),
    // P6: evidence (plan/served.md P6).
    ("books.find_attachment", R),
    ("books.annotate_attachment", W),
    ("books.set_entry_settles", W),
    ("books.consolidate_duplicates", C),
    // P4: drafts (plan P4; a discard previews, then acts).
    ("books.list_drafts", R),
    ("books.set_draft_state", W),
    ("books.discard_draft", C),
    // P5
    ("books.journal_history", R),
    ("books.preview_entry", R),
];

/// What `tool` needs, given its arguments; None for a tool this server
/// doesn't have.
pub fn needs(
    tool: &str,
    args: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Option<Scope> {
    let (_, needs) = TABLE.iter().find(|(name, _)| *name == tool)?;
    Some(match needs {
        Always(scope) => *scope,
        PostWhenConfirmed => {
            if confirmed(args) {
                Scope::Post
            } else {
                Scope::Write
            }
        }
        AdminForBackupSettings => {
            let backup = args
                .and_then(|a| a.get("changes"))
                .and_then(serde_json::Value::as_array)
                .is_some_and(|changes| {
                    changes.iter().any(|c| {
                        c.get("key")
                            .and_then(serde_json::Value::as_str)
                            .is_some_and(|k| k.trim().to_ascii_lowercase().starts_with("backup."))
                    })
                });
            if backup { Scope::Admin } else { Scope::Post }
        }
        PostForApprovalRules => {
            let kind = args
                .and_then(|a| a.get("kind"))
                .and_then(serde_json::Value::as_str);
            if kind == Some("approval_rule") {
                Scope::Post
            } else {
                Scope::Write
            }
        }
    })
}

fn confirmed(args: Option<&serde_json::Map<String, serde_json::Value>>) -> bool {
    args.and_then(|a| a.get("confirmed"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// Whether a token with `scopes` may list `tool` (it can call it in some
/// form: a tool that posts when confirmed shows to a books:write token,
/// which may preview).
pub fn shown(tool: &str, scopes: &std::collections::BTreeSet<Scope>) -> bool {
    match TABLE.iter().find(|(name, _)| *name == tool) {
        Some((_, Always(scope))) => scopes.contains(scope),
        Some((_, PostWhenConfirmed | PostForApprovalRules)) => {
            scopes.contains(&Scope::Write) || scopes.contains(&Scope::Post)
        }
        Some((_, AdminForBackupSettings)) => {
            scopes.contains(&Scope::Post) || scopes.contains(&Scope::Admin)
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::BooksServer;
    use serde_json::json;

    #[test]
    fn every_tool_is_in_the_table_once() {
        let tools = BooksServer::tool_router().list_all();
        let mut names: Vec<&str> = TABLE.iter().map(|(n, _)| *n).collect();
        names.sort_unstable();
        let len = names.len();
        names.dedup();
        assert_eq!(names.len(), len, "a tool is in the table twice");
        let mut have: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
        have.sort();
        assert_eq!(names, have, "the table and the tools differ");
    }

    #[test]
    fn only_read_only_tools_need_no_more_than_read() {
        for tool in BooksServer::tool_router().list_all() {
            let read_only = tool
                .annotations
                .as_ref()
                .and_then(|a| a.read_only_hint)
                .unwrap_or(false);
            let (_, needs) = TABLE.iter().find(|(n, _)| *n == tool.name).unwrap();
            let want = if !read_only {
                *needs != R
            } else if tool.name == "books.access_log" {
                *needs == A
            } else {
                *needs == R
            };
            assert!(
                want,
                "{}: read-only {read_only}, needs {needs:?}",
                tool.name
            );
        }
    }

    #[test]
    fn where_backups_go_is_admins() {
        let args = |v: serde_json::Value| v.as_object().cloned();
        let set = |keys: &[&str]| {
            let changes: Vec<_> = keys
                .iter()
                .map(|k| json!({"key": k, "value": "x"}))
                .collect();
            needs(
                "books.set_setting",
                args(json!({"changes": changes})).as_ref(),
            )
        };
        assert_eq!(set(&["company.name"]), Some(Scope::Post));
        assert_eq!(set(&["backup.folder"]), Some(Scope::Admin));
        assert_eq!(
            set(&["company.name", "backup.git_remote"]),
            Some(Scope::Admin)
        );
        assert_eq!(set(&[" Backup.Folder"]), Some(Scope::Admin));
        assert_eq!(needs("books.set_setting", None), Some(Scope::Post));
        // Shown to whoever can set some setting.
        let post: std::collections::BTreeSet<Scope> = [Scope::Post].into();
        let write: std::collections::BTreeSet<Scope> = [Scope::Write].into();
        let admin: std::collections::BTreeSet<Scope> = [Scope::Admin].into();
        assert!(shown("books.set_setting", &post));
        assert!(shown("books.set_setting", &admin));
        assert!(!shown("books.set_setting", &write));
    }

    #[test]
    fn a_tool_that_takes_confirmed_posts_only_when_confirmed() {
        // Exactly the tools whose arguments have `confirmed`.
        for tool in BooksServer::tool_router().list_all() {
            let takes = tool
                .input_schema
                .get("properties")
                .and_then(|p| p.as_object())
                .is_some_and(|p| p.contains_key("confirmed"));
            let (_, needs) = TABLE.iter().find(|(n, _)| *n == tool.name).unwrap();
            assert_eq!(takes, *needs == C, "{}", tool.name);
            let takes_kind = tool
                .input_schema
                .get("properties")
                .and_then(|p| p.as_object())
                .is_some_and(|p| p.contains_key("kind"));
            if *needs == K {
                assert!(takes_kind, "{}", tool.name);
            }
        }
        let args = |v: serde_json::Value| v.as_object().cloned();
        assert_eq!(needs("books.post_entry", None), Some(Scope::Write));
        assert_eq!(
            needs(
                "books.post_entry",
                args(json!({"confirmed": false})).as_ref()
            ),
            Some(Scope::Write)
        );
        assert_eq!(
            needs(
                "books.post_entry",
                args(json!({"confirmed": true})).as_ref()
            ),
            Some(Scope::Post)
        );
        // Only a real true confirms.
        assert_eq!(
            needs(
                "books.post_entry",
                args(json!({"confirmed": "true"})).as_ref()
            ),
            Some(Scope::Write)
        );
        assert_eq!(
            needs(
                "books.void_document",
                args(json!({"confirmed": false})).as_ref()
            ),
            Some(Scope::Post),
            "a tool that posts at once needs books:post whatever it is given"
        );
        // P4: a payment previews, then posts when confirmed.
        assert_eq!(needs("books.receive_payment", None), Some(Scope::Write));
        assert_eq!(
            needs("books.pay_bill", args(json!({"confirmed": true})).as_ref()),
            Some(Scope::Post)
        );
        assert_eq!(needs("books.access_log", None), Some(Scope::Admin));
        assert_eq!(needs("books.nonesuch", None), None);
        // INVARIANT 4.
        assert_eq!(
            needs(
                "books.set_convention",
                args(json!({"kind": "approval_rule"})).as_ref()
            ),
            Some(Scope::Post)
        );
        assert_eq!(
            needs(
                "books.set_convention",
                args(json!({"kind": "policy"})).as_ref()
            ),
            Some(Scope::Write)
        );
        assert_eq!(needs("books.set_convention", None), Some(Scope::Write));
        assert_eq!(needs("books.retire_convention", None), Some(Scope::Post));
        assert_eq!(needs("books.approve_entry", None), Some(Scope::Post));
        // INVARIANT 5.
        for tool in [
            "books.vendor_1099",
            "books.export_cpa_package",
            "books.get_w9",
        ] {
            assert_eq!(needs(tool, None), Some(Scope::Admin), "{tool}");
        }
        for tool in ["books.set_tin", "books.set_w9", "books.purge_tin"] {
            let confirmed = args(json!({"confirmed": true}));
            assert_eq!(needs(tool, confirmed.as_ref()), Some(Scope::Post), "{tool}");
        }
    }

    #[test]
    fn a_token_is_shown_what_it_can_call() {
        let set = |s: &[Scope]| s.iter().copied().collect::<std::collections::BTreeSet<_>>();
        let read = set(&[Scope::Read]);
        assert!(shown("books.chart", &read));
        assert!(!shown("books.post_entry", &read));
        assert!(!shown("books.draft_entry", &read));
        assert!(!shown("books.access_log", &read));
        let write = set(&[Scope::Read, Scope::Write]);
        assert!(shown("books.post_entry", &write), "it may preview");
        assert!(
            shown("books.create_invoice", &write),
            "it may preview an invoice"
        );
        assert!(!shown("books.void_document", &write));
        let post = set(&[Scope::Post]);
        assert!(shown("books.post_entry", &post));
        assert!(!shown("books.chart", &post));
        assert!(shown("books.access_log", &set(&[Scope::Admin])));
        assert!(!shown("books.nonesuch", &set(&Scope::ALL)));
    }
}
