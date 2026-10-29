//! The receipt policy: server-side enforcement, not prose.
//!
//! Posting an entry whose debits are strictly greater than the configured
//! threshold requires at least one attached receipt, unless every line is in
//! an exempt account. Selected accounts can require receipts regardless of
//! amount. Reversals are exempt: the original entry carries the evidence.
//!
//! The decision itself lives in the engine
//! (`ledger_core::evidence::requirement`, plan P4, I-5): this is the
//! policy as the settings carry it, and the words that describe it.
//!
//! ```text
//! GOAL: no entry posts without the document that supports it.
//! SOURCE: docs/spec/sources/irs-records.md (Pub. 583: "you must keep
//!         supporting documents"; they "support the entries in your
//!         books"); DECISION (owner: the threshold, strictly greater
//!         than; Fernhill's is 0.00, every entry, 2026-10-12).
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

use ledger_core::{JournalEntry, Ledger};
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

    /// The policy as the engine evaluates it
    /// ([`ledger_core::evidence::Policy`], plan P4): one decision, in the
    /// core. A threshold that fails to parse requires a receipt on
    /// everything, as an unvalidated policy always did.
    pub fn engine(&self) -> ledger_core::evidence::Policy {
        ledger_core::evidence::Policy {
            threshold: self.threshold(),
            required_accounts: self.receipt_required_accounts.clone(),
            exempt_accounts: self.receipt_exempt_accounts.clone(),
        }
    }

    /// Total debits of the entry (one side of the balanced entry). A
    /// draft's lines are as journaled, of any size (rules.md B-28): a total
    /// past exact arithmetic is refused (TooLargeToCompute), never a panic.
    pub fn entry_debits(entry: &JournalEntry) -> Result<Decimal, ledger_core::LedgerError> {
        ledger_core::evidence::Policy::entry_debits(entry)
    }

    /// True when posting this entry requires at least one receipt by the
    /// policy alone (the engine's [`ledger_core::evidence::requirement`]
    /// also knows what posted it). Debits past exact arithmetic are past
    /// any threshold.
    pub fn receipt_required(&self, ledger: &Ledger, entry: &JournalEntry) -> bool {
        self.engine().receipt_required(ledger, entry)
    }

    /// True when an entry touching these account codes with this debit total
    /// requires at least one receipt. The code-level check behind
    /// [`Self::receipt_required`], for postings (like disposals) computed
    /// without a draft entry in hand.
    pub fn receipt_required_for(&self, codes: &[&str], debits: Decimal) -> bool {
        self.engine().receipt_required_for(codes, debits)
    }

    /// Human-readable policy summary for previews and errors.
    pub fn describe(&self) -> String {
        self.engine().describe()
    }

    /// The receipt rule in force, phrased for the agent's operating
    /// instructions, exactly: the threshold as configured, in the books'
    /// base currency (`currency`, e.g. "USD"), never rounded; the accounts
    /// that always need a receipt and those exempt; and that reversals
    /// need none (the original carries the evidence).
    pub fn rule(&self, currency: &str) -> String {
        self.rule_listing(currency, RULE_CODES)
    }

    /// [`Self::rule`], listing at most `codes` account codes of each kind
    /// (the rest counted, books.settings listing them); with `codes` 0,
    /// only how many accounts of each kind there are, and where they are
    /// listed: for a text with a budget.
    pub fn rule_listing(&self, currency: &str, codes: usize) -> String {
        let mut rule = if self.threshold().is_zero() {
            "every entry needs at least one receipt".to_string()
        } else {
            format!(
                "any entry moving more than {} {} in debits needs at least one receipt",
                self.threshold(),
                plain_code(currency)
            )
        };
        let (required, exempt) = (
            self.receipt_required_accounts.len(),
            self.receipt_exempt_accounts.len(),
        );
        if codes == 0 && required + exempt > 0 {
            rule.push_str(&format!(
                "; books.settings lists {required} account(s) whose entries always need one and {exempt} whose entries need none"
            ));
        } else {
            if required > 0 {
                rule.push_str(&format!(
                    "; so does any entry touching {}, whatever its amount",
                    accounts(&self.receipt_required_accounts, codes)
                ));
            }
            if exempt > 0 {
                rule.push_str(&format!(
                    "; an entry whose every line is in {} needs none{}",
                    accounts(&self.receipt_exempt_accounts, codes),
                    if required == 0 {
                        ""
                    } else {
                        " (unless it touches a required one)"
                    }
                ));
            }
        }
        rule.push_str("; reversals need none (the original carries the evidence)");
        rule
    }

    /// Detect a potential threshold split (see
    /// [`ledger_core::evidence::Policy::split_group`]): posted entries on
    /// the same date with the same normalized description and debit
    /// accounts whose combined debits with this entry exceed the threshold
    /// while this entry alone does not. Returns the matching posted entry
    /// ids.
    pub fn split_group(&self, ledger: &Ledger, entry_id: u64, entry: &JournalEntry) -> Vec<u64> {
        self.engine().split_group(ledger, entry_id, entry)
    }
}

/// The most account codes [`ReceiptPolicy::rule`] lists, and the most
/// characters of each: the rule goes into a connect text with a budget.
const RULE_CODES: usize = 6;
const RULE_CODE_CHARS: usize = 12;

/// A code as plain text for the rule: no control or format characters
/// (a code comes from the settings file, and must not carry text that
/// reads as the server's own), cut on a character.
fn plain_code(code: &str) -> String {
    let plain: String = code
        .chars()
        .filter(|c| !c.is_control() && !c.is_whitespace() && !is_format_char(*c))
        .collect();
    if plain.chars().count() > RULE_CODE_CHARS {
        let cut: String = plain.chars().take(RULE_CODE_CHARS - 1).collect();
        format!("{cut}…")
    } else {
        plain
    }
}

/// "account 6000", "accounts 6000, 6010", "accounts 6000, ... and 3 more
/// (books.settings lists them)": at most `max` codes, `max` at least 1.
fn accounts(codes: &[String], max: usize) -> String {
    let max = max.max(1);
    let shown: Vec<String> = codes.iter().take(max).map(|c| plain_code(c)).collect();
    let more = codes.len().saturating_sub(max);
    let list = shown.join(", ");
    match (codes.len(), more) {
        (1, _) => format!("account {list}"),
        (_, 0) => format!("accounts {list}"),
        _ => format!("accounts {list} and {more} more (books.settings lists them)"),
    }
}

/// Whether `c` is a Unicode format character (general category Cf):
/// invisible, but it reorders or joins the text around it (bidi
/// overrides, zero-width spaces and joiners, the byte-order mark).
/// SOURCE: Unicode 15.1, UnicodeData.txt, category Cf.
pub fn is_format_char(c: char) -> bool {
    matches!(
        c as u32,
        0x00AD
            | 0x0600..=0x0605
            | 0x061C
            | 0x06DD
            | 0x070F
            | 0x0890..=0x0891
            | 0x08E2
            | 0x180E
            | 0x200B..=0x200F
            | 0x202A..=0x202E
            | 0x2060..=0x2064
            | 0x2066..=0x206F
            | 0xFEFF
            | 0xFFF9..=0xFFFB
            | 0x110BD
            | 0x110CD
            | 0x13430..=0x1343F
            | 0x1BCA0..=0x1BCA3
            | 0x1D173..=0x1D17A
            | 0xE0001
            | 0xE0020..=0xE007F
    )
}

#[cfg(test)]
mod rule_tests {
    use super::*;

    fn policy(t: &str, required: &[&str], exempt: &[&str]) -> ReceiptPolicy {
        ReceiptPolicy {
            require_receipts_above: t.to_string(),
            receipt_required_accounts: required.iter().map(|s| s.to_string()).collect(),
            receipt_exempt_accounts: exempt.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// The rule the agent is told is the rule enforced: the threshold
    /// exactly as configured (0.001 stays 0.001, never "$0.00"), in the
    /// base currency; the required and exempt accounts; reversals exempt.
    #[test]
    fn the_rule_told_is_the_rule_enforced() {
        let p = policy("0.001", &[], &[]);
        assert_eq!(
            p.rule("EUR"),
            "any entry moving more than 0.001 EUR in debits needs at least one receipt; \
             reversals need none (the original carries the evidence)"
        );
        // And it is the threshold receipt_required_for applies.
        assert!(!p.receipt_required_for(&["6000"], Decimal::new(1, 3)));
        assert!(p.receipt_required_for(&["6000"], Decimal::new(2, 3)));
        let p = policy("25.00", &["6000"], &["1500", "1510"]);
        let rule = p.rule("USD");
        for must in [
            "more than 25.00 USD in debits",
            "so does any entry touching account 6000, whatever its amount",
            "an entry whose every line is in accounts 1500, 1510 needs none (unless it touches a required one)",
            "reversals need none",
        ] {
            assert!(rule.contains(must), "{must:?}: {rule}");
        }
        assert!(!rule.contains('$'), "{rule}");
        assert_eq!(
            policy("0.00", &[], &[]).rule("USD"),
            "every entry needs at least one receipt; reversals need none (the original carries the evidence)"
        );
    }

    /// A long list is cut, saying books.settings lists them (it does,
    /// every code uncut, in receipt_required_accounts and
    /// receipt_exempt_accounts: ledger-mcp's
    /// every_receipt_account_left_out_of_the_connect_text_is_in_books_settings),
    /// and a code carries no control or format characters.
    #[test]
    fn a_long_or_odd_account_list_stays_short_and_plain() {
        let many: Vec<String> = (0..20).map(|i| format!("{}", 6000 + i)).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        let rule = policy("1", &refs, &[]).rule("USD");
        assert!(
            rule.contains("6005 and 14 more (books.settings lists them)"),
            "{rule}"
        );
        assert!(!rule.contains("6006"), "{rule}");
        let rule = policy("1", &refs, &refs[..2]).rule_listing("USD", 0);
        assert!(
            rule.contains(
                "; books.settings lists 20 account(s) whose entries always need one and 2 whose entries need none;"
            ),
            "{rule}"
        );
        let rule = policy("1", &["60\n00\u{202e}\u{200b}XYZ-VERY-LONG-CODE"], &[]).rule("U\nSD");
        assert!(
            !rule.chars().any(|c| c.is_control() || is_format_char(c)),
            "{rule:?}"
        );
        assert!(rule.contains("account 6000XYZ-VER…"), "{rule}");
        assert!(rule.contains("1 USD"), "{rule}");
        // A code of exactly the most characters is kept whole; one more
        // is cut.
        let twelve = "ABCDEFGHIJKL";
        assert_eq!(twelve.chars().count(), RULE_CODE_CHARS);
        let rule = policy("1", &[twelve], &[]).rule("USD");
        assert!(
            rule.contains(&format!("account {twelve}, whatever")),
            "{rule}"
        );
        let rule = policy("1", &["ABCDEFGHIJKLM"], &[]).rule("USD");
        assert!(rule.contains("account ABCDEFGHIJK…, whatever"), "{rule}");
    }

    /// The short form (no codes) counts each kind of account, whichever
    /// kinds there are, and says nothing of accounts when there are none.
    #[test]
    fn the_short_rule_counts_each_kind_of_account() {
        for (required, exempt, counts) in [
            (&["6000"][..], &[][..], Some((1, 0))),
            (&[][..], &["1500", "1510"][..], Some((0, 2))),
            (&["6000", "6010"][..], &["1500"][..], Some((2, 1))),
            (&[][..], &[][..], None),
        ] {
            let rule = policy("1", required, exempt).rule_listing("USD", 0);
            let tail = "; reversals need none (the original carries the evidence)";
            let want = match counts {
                Some((r, e)) => format!(
                    "any entry moving more than 1 USD in debits needs at least one receipt; books.settings lists {r} account(s) whose entries always need one and {e} whose entries need none{tail}"
                ),
                None => format!(
                    "any entry moving more than 1 USD in debits needs at least one receipt{tail}"
                ),
            };
            assert_eq!(rule, want);
        }
    }

    /// A draft an earlier version journaled may hold debits exact
    /// arithmetic can't sum (rules.md B-28): its total is refused, never a
    /// panic, and for the policy it is past any threshold: a receipt is
    /// required, and it is no part of a split.
    #[test]
    fn debits_past_exact_arithmetic_are_past_any_threshold() {
        use ledger_core::{AccountType, Currency, Money, Side};
        let mut l = Ledger::new(Currency::USD);
        let cash = l
            .add_account("1000", "Cash", AccountType::Asset, None, false)
            .unwrap();
        let rent = l
            .add_account("6000", "Rent", AccountType::Expense, None, false)
            .unwrap();
        let usd = |a: Decimal| Money::new(a, Currency::USD);
        let date = chrono::NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let huge = l.create_entry(date, "huge");
        for _ in 0..2 {
            l.replay_add_line(huge, rent, Side::Debit, usd(Decimal::MAX))
                .unwrap();
        }
        let p = policy("1000000", &[], &["6000"]);
        let entry = l.get_entry(huge).unwrap().clone();
        assert!(matches!(
            ReceiptPolicy::entry_debits(&entry),
            Err(ledger_core::LedgerError::TooLargeToCompute(_))
        ));
        // Exempt accounts only, yet past any threshold: still exempt.
        assert!(!p.receipt_required(&l, &entry));
        let p = policy("1000000", &[], &[]);
        assert!(p.receipt_required(&l, &entry));
        assert!(p.split_group(&l, huge, &entry).is_empty());
        // Two posted entries of the same signature, one of them huge: the
        // group's total is past the threshold.
        let mut post = |amount: Decimal| {
            let e = l.create_entry(date, "Coffee");
            l.add_line(e, rent, Side::Debit, usd(amount)).unwrap();
            l.add_line(e, cash, Side::Credit, usd(amount)).unwrap();
            l.post_entry(e).unwrap();
            e
        };
        let small = post(Decimal::new(500, 2));
        let big = post(ledger_core::kernel::MAX_LINE_AMOUNT);
        let draft = l.create_entry(date, "coffee");
        l.add_line(draft, rent, Side::Debit, usd(Decimal::new(500, 2)))
            .unwrap();
        l.add_line(draft, cash, Side::Credit, usd(Decimal::new(500, 2)))
            .unwrap();
        let entry = l.get_entry(draft).unwrap().clone();
        let p = policy("10.00", &[], &[]);
        assert_eq!(p.split_group(&l, draft, &entry), vec![small, big]);
        assert_eq!(
            ReceiptPolicy::entry_debits(&entry),
            Ok(Decimal::new(500, 2))
        );
    }
}
