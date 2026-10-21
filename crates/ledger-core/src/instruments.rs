//! Payment instruments: how the company (or a member, for it) paid.
//!
//! A payment to a contractor by check counts toward its 1099; the same
//! payment by card or through a payment network is the card processor's
//! to report, on Form 1099-K, and never the company's (Treas. Reg.
//! §1.6041-1(a)(1)(iv)). The books record each way of paying once
//! ([`Instrument`]: the company's bank, its business card, a member's own
//! card, a member's own cash or check, a payment platform), each with the
//! ledger account it pays through, and tag the entries paid with one
//! ([`crate::Ledger::set_entry_instrument`]). The 1099 worksheet reads the
//! tag ([`crate::form1099`]), and the business cards' liability accounts
//! are the books' card accounts ([`crate::Ledger::card_accounts`]), known
//! from what the books record rather than an account's name.
//!
//! ```text
//! GOAL: every payment's method is recorded once, plainly, so the 1099
//!       worksheet never guesses whether a payment was by card.
//! SOURCE: docs/spec/rules.md B-19 (Treas. Reg. §1.6041-1(a)(1)(iv),
//!         §1.6050W-1(b)(1), (b)(3), (c)(3); Instructions for Form 1099-K:
//!         automated clearing houses are not payment networks;
//!         sources/irs-1099-payment-method.md); plan/served.md 4.9, 5.2.
//! INVARIANTS:
//!   1. A label is one line, at most settings::MAX_TEXT characters, holds
//!      no full TIN, and is unique among instruments (ignoring case and
//!      surrounding space).
//!   2. last4, when given, is exactly four ASCII digits: no full card or
//!      account number ever enters the books.
//!   3. The account is a real asset or liability account, not a header,
//!      and not the payables or receivables control account (a bill is not
//!      paid until it is paid).
//!   4. The owner party, when given, exists. It is informational: it never
//!      classifies a payment (the payment-card exception follows the
//!      instrument, whoever's card it is).
//! ```

use crate::{AccountId, PartyId};
use serde::{Deserialize, Serialize};

/// Opaque instrument handle.
pub type InstrumentId = u64;

/// What kind of payment an instrument makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentKind {
    /// The company's own bank account: its checks, ACH, wires, bill-pay.
    Bank,
    /// A company credit or debit card the payee charged.
    BusinessCard,
    /// A member's personal credit or debit card the payee charged.
    MemberCard,
    /// A member's own cash, check, ACH, Zelle, or a friends-and-family
    /// transfer from a member's own app or card.
    MemberCashOrCheck,
    /// A payment the payee received as a seller through a payment network
    /// that settles it (PayPal or Venmo goods and services).
    Platform,
}

impl InstrumentKind {
    /// Every kind, in order.
    pub const ALL: [InstrumentKind; 5] = [
        InstrumentKind::Bank,
        InstrumentKind::BusinessCard,
        InstrumentKind::MemberCard,
        InstrumentKind::MemberCashOrCheck,
        InstrumentKind::Platform,
    ];

    /// Parse a key ("bank", "business_card", ...), ignoring case and
    /// surrounding space.
    pub fn parse(s: &str) -> Option<InstrumentKind> {
        let s = s.trim().to_ascii_lowercase();
        InstrumentKind::ALL.into_iter().find(|k| k.key() == s)
    }

    /// Its key: "bank", "business_card", "member_card",
    /// "member_cash_or_check", "platform".
    pub fn key(self) -> &'static str {
        match self {
            InstrumentKind::Bank => "bank",
            InstrumentKind::BusinessCard => "business_card",
            InstrumentKind::MemberCard => "member_card",
            InstrumentKind::MemberCashOrCheck => "member_cash_or_check",
            InstrumentKind::Platform => "platform",
        }
    }

    /// In words, for people.
    pub fn label(self) -> &'static str {
        match self {
            InstrumentKind::Bank => "the company's bank (check, ACH, wire)",
            InstrumentKind::BusinessCard => "a company card",
            InstrumentKind::MemberCard => "a member's personal card",
            InstrumentKind::MemberCashOrCheck => "a member's own cash or check",
            InstrumentKind::Platform => "a payment network (goods and services)",
        }
    }

    /// Whether a payment made with it is a payment-card or third-party
    /// network transaction: reported by the processor on Form 1099-K and
    /// excluded from 1099-NEC and 1099-MISC (Treas. Reg.
    /// §1.6041-1(a)(1)(iv)), whoever's card it was.
    pub fn is_card_or_network(self) -> bool {
        matches!(
            self,
            InstrumentKind::BusinessCard | InstrumentKind::MemberCard | InstrumentKind::Platform
        )
    }
}

/// One way the company pays.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instrument {
    /// Opaque handle.
    pub id: InstrumentId,
    /// What kind of payment it makes.
    pub kind: InstrumentKind,
    /// Its name ("Harbor Bank checking", "Riley's Visa"), unique.
    pub label: String,
    /// The last four digits of its card or account number, if given.
    pub last4: Option<String>,
    /// Whose it is (a member's card), if given. Never used to classify.
    pub owner_party: Option<PartyId>,
    /// The ledger account it pays through: the bank account, the card's
    /// liability, or the members' financing liability (Due to Members).
    pub account: AccountId,
}

/// Check a label: one line, at most `MAX_TEXT` characters, no full TIN
/// (INVARIANT 1, all but uniqueness). Returns it trimmed.
pub fn check_label(label: &str) -> Result<String, String> {
    crate::settings::line_text("label", label, crate::settings::MAX_TEXT)
}

/// Check a last four (INVARIANT 2).
pub fn check_last4(last4: &str) -> Result<(), String> {
    if last4.len() == 4 && last4.bytes().all(|b| b.is_ascii_digit()) {
        Ok(())
    } else {
        Err(format!(
            "last4 {:?} must be exactly four digits (never a whole card or account number)",
            crate::parties::redact_full_tins(last4)
        ))
    }
}

/// The key two labels are compared by: trimmed, lower case.
pub fn label_key(label: &str) -> String {
    label.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip_and_only_cards_and_networks_are_excluded() {
        for k in InstrumentKind::ALL {
            assert_eq!(InstrumentKind::parse(k.key()), Some(k));
            assert_eq!(
                InstrumentKind::parse(&format!(" {} ", k.key().to_uppercase())),
                Some(k)
            );
            assert!(!k.label().is_empty());
        }
        assert_eq!(InstrumentKind::parse("card"), None);
        let excluded: Vec<_> = InstrumentKind::ALL
            .into_iter()
            .filter(|k| k.is_card_or_network())
            .collect();
        assert_eq!(
            excluded,
            [
                InstrumentKind::BusinessCard,
                InstrumentKind::MemberCard,
                InstrumentKind::Platform
            ]
        );
    }

    #[test]
    fn last4_is_four_digits_only() {
        assert!(check_last4("0042").is_ok());
        for bad in ["", "123", "12345", "12a4", "１２３４", "4111111111111111"] {
            assert!(check_last4(bad).is_err(), "{bad:?}");
        }
        // A refusal never echoes a full TIN-shaped number.
        let why = check_last4("123-45-6789").unwrap_err();
        assert!(!why.contains("6789"), "{why}");
    }

    #[test]
    fn labels_are_one_bounded_line() {
        assert_eq!(check_label("  Harbor Bank  ").unwrap(), "Harbor Bank");
        assert!(check_label("").is_err());
        assert!(check_label("a\nb").is_err());
        assert!(check_label(&"x".repeat(crate::settings::MAX_TEXT + 1)).is_err());
        assert!(check_label("card 123-45-6789").is_err());
        assert_eq!(label_key(" Riley's Visa "), "riley's visa");
    }
}
