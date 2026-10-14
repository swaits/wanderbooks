//! The receipt policy: server-side enforcement, not prose.
//!
//! Posting an entry whose debits are strictly greater than the configured
//! threshold requires at least one attached receipt, unless every line is in
//! an exempt account. Selected accounts can require receipts regardless of
//! amount. Reversals are exempt: the original entry carries the evidence.
//!
//! ```text
//! GOAL: no entry posts without the document that supports it.
//! SOURCE: docs/spec/sources/irs-records.md (Pub. 583: "you must keep
//!         supporting documents"; they "support the entries in your
//!         books"); DECISION (Stephen: the threshold, strictly greater
//!         than; Wanderland's is 0.00, every entry, 2026-10-12).
//! INVARIANTS:
//!   1. An entry touching a required account always needs a receipt.
//!   2. Otherwise it needs one exactly when its debits are strictly
//!      greater than the threshold and some line is outside the exempt
//!      accounts; at threshold 0.00 every entry with debits needs one.
//!   3. Splitting does not dodge it: posted entries the same day with the
//!      same description (whitespace and case aside) and the same debit
//!      accounts count together (split_group), reversals aside.
//!   4. A threshold that isn't a decimal, or is below zero, is refused at
//!      startup (validate).
//! ```

use ledger_core::{JournalEntry, Ledger, Side};
use rust_decimal::Decimal;
use serde::Deserialize;

/// Server-side receipt policy, loaded from config.
#[derive(Debug, Clone, Deserialize)]
pub struct ReceiptPolicy {
    /// Receipts required when an entry's debits are strictly greater than
    /// this amount. Decimal string, e.g. `"25.00"`.
    #[serde(default = "default_threshold")]
    pub require_receipts_above: String,
    /// Account codes that always require a receipt, regardless of amount.
    #[serde(default)]
    pub receipt_required_accounts: Vec<String>,
    /// Account codes exempt from the policy. The exemption applies only when
    /// *every* line of the entry is in an exempt account.
    #[serde(default)]
    pub receipt_exempt_accounts: Vec<String>,
}

fn default_threshold() -> String {
    "25.00".to_string()
}

impl Default for ReceiptPolicy {
    fn default() -> Self {
        Self {
            require_receipts_above: default_threshold(),
            receipt_required_accounts: Vec::new(),
            receipt_exempt_accounts: Vec::new(),
        }
    }
}

impl ReceiptPolicy {
    /// Refuse a policy the server can't honor: a threshold that isn't a
    /// decimal, or is below zero. Checked at startup, so a typo can't
    /// quietly become some other threshold.
    pub fn validate(&self) -> Result<(), String> {
        match crate::decimal(self.require_receipts_above.trim()) {
            Some(t) if !t.is_sign_negative() || t.is_zero() => Ok(()),
            _ => Err(format!(
                "receipt threshold {:?} must be a decimal at least zero",
                self.require_receipts_above
            )),
        }
    }

    /// The threshold: receipts are required strictly above it.
    pub fn threshold(&self) -> Decimal {
        // validate() runs at startup; an unvalidated policy (tests) that
        // fails to parse requires a receipt on everything.
        crate::decimal(self.require_receipts_above.trim()).unwrap_or(Decimal::ZERO)
    }

    fn account_codes(&self, ledger: &Ledger, entry: &JournalEntry) -> Vec<String> {
        entry
            .lines()
            .iter()
            .map(|l| {
                ledger
                    .chart()
                    .get(l.account)
                    .map(|a| a.code.clone())
                    .unwrap_or_default()
            })
            .collect()
    }

    /// Total debits of the entry (one side of the balanced entry).
    pub fn entry_debits(entry: &JournalEntry) -> Decimal {
        entry
            .lines()
            .iter()
            .filter(|l| l.side == Side::Debit)
            .map(|l| l.amount.amount)
            .sum()
    }

    /// True when posting this entry requires at least one receipt.
    pub fn receipt_required(&self, ledger: &Ledger, entry: &JournalEntry) -> bool {
        let codes = self.account_codes(ledger, entry);
        let refs: Vec<&str> = codes.iter().map(|s| s.as_str()).collect();
        self.receipt_required_for(&refs, Self::entry_debits(entry))
    }

    /// True when an entry touching these account codes with this debit total
    /// requires at least one receipt. The code-level check behind
    /// [`Self::receipt_required`], for postings (like disposals) computed
    /// without a draft entry in hand.
    pub fn receipt_required_for(&self, codes: &[&str], debits: Decimal) -> bool {
        // Per-account override: any line in a required account → required.
        if codes
            .iter()
            .any(|c| self.receipt_required_accounts.iter().any(|r| r == *c))
        {
            return true;
        }
        // Strictly greater than the threshold (Stephen: ">$25").
        if debits <= self.threshold() {
            return false;
        }
        // Exempt only when every line is in an exempt account.
        !codes
            .iter()
            .all(|c| self.receipt_exempt_accounts.iter().any(|e| e == *c))
    }

    /// Human-readable policy summary for previews and errors.
    pub fn describe(&self) -> String {
        if self.threshold().is_zero() {
            "receipts required on every entry".to_string()
        } else {
            format!(
                "receipts required when entry debits are strictly greater than {}",
                self.threshold()
            )
        }
    }

    /// The receipt rule, phrased for the agent's operating instructions.
    pub fn rule(&self) -> String {
        if self.threshold().is_zero() {
            "every entry needs at least one receipt".to_string()
        } else {
            format!(
                "any entry moving more than ${} in debits needs at least one receipt",
                self.threshold().round_dp(2)
            )
        }
    }

    /// Normalize a description for split detection.
    fn normalize_description(desc: &str) -> String {
        desc.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    }

    /// Debit account ids of an entry, sorted.
    fn debit_accounts(entry: &JournalEntry) -> Vec<u64> {
        let mut ids: Vec<u64> = entry
            .lines()
            .iter()
            .filter(|l| l.side == Side::Debit)
            .map(|l| l.account)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// Detect a potential threshold split: posted entries on the same date
    /// with the same normalized description and debit accounts, whose
    /// combined debits with this entry exceed the threshold while this
    /// entry alone does not. Returns the matching posted entry ids.
    ///
    /// This is the anti-gaming rule: splitting a $100 purchase into four
    /// $25 entries does not dodge the receipt policy. Same-day,
    /// same-description, same-accounts is the signature; recurring spending
    /// spread across days is not flagged.
    pub fn split_group(&self, ledger: &Ledger, entry_id: u64, entry: &JournalEntry) -> Vec<u64> {
        let debits = Self::entry_debits(entry);
        if debits > self.threshold() {
            return Vec::new(); // Normal policy already applies.
        }
        let desc = Self::normalize_description(entry.description());
        let accounts = Self::debit_accounts(entry);
        let date = entry.date();
        let mut group = Vec::new();
        let mut total = debits;
        for other in ledger.posted_entries() {
            if other.id() == entry_id {
                continue;
            }
            if other.date() != date {
                continue;
            }
            if Self::normalize_description(other.description()) != desc {
                continue;
            }
            if Self::debit_accounts(other) != accounts {
                continue;
            }
            // Reversals are exempt from the policy; don't count them.
            if other.reversal_of().is_some() {
                continue;
            }
            group.push(other.id());
            total += Self::entry_debits(other);
        }
        if !group.is_empty() && total > self.threshold() {
            group
        } else {
            Vec::new()
        }
    }
}
