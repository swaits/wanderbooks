//! Exact money: [`Decimal`] amounts paired with an ISO 4217 currency code.
//!
//! There is no float anywhere in this module, and there never will be.
//! [`Money`] arithmetic is checked: adding different currencies is an error,
//! not a guess.

use crate::LedgerError;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fmt;

/// An ISO 4217 alpha-3 currency code, stored as three uppercase ASCII bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Currency([u8; 3]);

impl Currency {
    /// US dollar.
    pub const USD: Currency = Currency(*b"USD");
    /// Euro.
    pub const EUR: Currency = Currency(*b"EUR");
    /// British pound.
    pub const GBP: Currency = Currency(*b"GBP");
    /// Swiss franc.
    pub const CHF: Currency = Currency(*b"CHF");
    /// Japanese yen.
    pub const JPY: Currency = Currency(*b"JPY");

    /// Parse and validate a three-letter code. Input is uppercased, so
    /// `"usd"` and `"USD"` both work; `"US"` or `"USDD"` fail.
    pub fn new(code: &str) -> Result<Self, LedgerError> {
        let bytes = code.as_bytes();
        if bytes.len() != 3 || !bytes.iter().all(|b| b.is_ascii_alphabetic()) {
            return Err(LedgerError::InvalidCurrencyCode(code.to_string()));
        }
        let mut upper = [0u8; 3];
        for (i, b) in bytes.iter().enumerate() {
            upper[i] = b.to_ascii_uppercase();
        }
        Ok(Currency(upper))
    }

    /// The currency's minor units: how many decimal places its smallest
    /// unit has (2 for USD's cents, 0 for JPY, 3 for KWD), or `None` for
    /// codes ISO 4217 gives none (gold, SDRs). Unlisted codes are 2.
    ///
    /// SOURCE: ISO 4217 list one, published 2026-09-17
    /// (docs/spec/sources/iso-4217.md); generated from it, not typed in.
    pub fn minor_units(&self) -> Option<u32> {
        match self.code() {
            "BIF" | "CLP" | "DJF" | "GNF" | "ISK" | "JPY" | "KMF" | "KRW" | "PYG" | "RWF"
            | "UGX" | "UYI" | "VND" | "VUV" | "XAF" | "XOF" | "XPF" => Some(0),
            "BHD" | "IQD" | "JOD" | "KWD" | "LYD" | "OMR" | "TND" => Some(3),
            "CLF" | "UYW" => Some(4),
            "XAG" | "XAU" | "XBA" | "XBB" | "XBC" | "XBD" | "XDR" | "XPD" | "XPT" | "XSU"
            | "XTS" | "XUA" | "XXX" => None,
            _ => Some(2),
        }
    }

    /// The three-letter code, e.g. `"USD"`.
    pub fn code(&self) -> &str {
        std::str::from_utf8(&self.0).expect("currency codes are ASCII by construction")
    }
}

impl fmt::Debug for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Currency({})", self.code())
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// An exact decimal amount in a specific currency.
///
/// `Money` is `Copy` and cheap: pass it by value. All arithmetic is checked
/// for currency agreement and returns [`LedgerError::CurrencyMismatch`]
/// rather than silently converting.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Money {
    /// The exact amount. Never a float.
    pub amount: Decimal,
    /// The currency this amount is denominated in.
    pub currency: Currency,
}

impl Money {
    /// Build money from an exact decimal amount.
    pub fn new(amount: Decimal, currency: Currency) -> Self {
        Self { amount, currency }
    }

    /// Zero in the given currency.
    pub fn zero(currency: Currency) -> Self {
        Self {
            amount: Decimal::ZERO,
            currency,
        }
    }

    /// True when the amount is exactly zero.
    pub fn is_zero(&self) -> bool {
        self.amount.is_zero()
    }

    fn check_currency(&self, other: &Money) -> Result<(), LedgerError> {
        if self.currency != other.currency {
            return Err(LedgerError::CurrencyMismatch {
                expected: self.currency.code().to_string(),
                found: other.currency.code().to_string(),
            });
        }
        Ok(())
    }

    /// Add two money values. Fails on currency mismatch.
    pub fn checked_add(self, rhs: Money) -> Result<Money, LedgerError> {
        self.check_currency(&rhs)?;
        Ok(Money::new(self.amount + rhs.amount, self.currency))
    }

    /// Subtract two money values. Fails on currency mismatch.
    pub fn checked_sub(self, rhs: Money) -> Result<Money, LedgerError> {
        self.check_currency(&rhs)?;
        Ok(Money::new(self.amount - rhs.amount, self.currency))
    }
}

impl fmt::Debug for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.amount, self.currency)
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.amount, self.currency)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usd(amount: Decimal) -> Money {
        Money::new(amount, Currency::USD)
    }

    fn d(cents: i64) -> Decimal {
        Decimal::new(cents, 2)
    }

    #[test]
    fn checked_arithmetic() {
        assert_eq!(
            usd(d(1050)).checked_add(usd(d(425))).unwrap().amount,
            d(1475)
        );
        assert_eq!(
            usd(d(1050)).checked_sub(usd(d(425))).unwrap().amount,
            d(625)
        );
        // Subtraction is not division, and not addition.
        assert_ne!(
            usd(d(1000)).checked_sub(usd(d(200))).unwrap().amount,
            d(500)
        );
    }

    #[test]
    fn checked_arithmetic_rejects_currency_mismatch() {
        let eur = Money::new(d(100), Currency::EUR);
        assert!(usd(d(100)).checked_add(eur).is_err());
        assert!(usd(d(100)).checked_sub(eur).is_err());
    }
}
