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

    /// The decimal places a share of an amount in this currency is split
    /// to (members' shares of earnings, periods::allocate_cumulative): its
    /// minor units (cents, whole yen, thousandths of a dinar), or, for a
    /// code ISO 4217 gives none (gold, SDRs), the most a line may carry
    /// (kernel::MAX_LINE_SCALE). Every posted amount is then a whole number
    /// of the unit split to, so no member's running share is ever a unit
    /// from exact (rules.md B-13); split to cents, a finer amount's
    /// leftover fraction would all go to one member.
    pub fn allocation_places(&self) -> u32 {
        self.minor_units().unwrap_or(crate::kernel::MAX_LINE_SCALE)
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

/// A non-negative amount as a whole number of steps, for exact proration
/// by days (kernel::prorate_by_days): `units` steps of 10^-`scale` each,
/// where `scale` is the amount's own places or its currency's minor unit,
/// whichever is more, and `unit` is one minor unit in steps.
///
/// ```text
/// GOAL: depreciation and daily schedules round to the currency's own
///       smallest unit (cents, whole yen, thousandths of a dinar), never a
///       fixed two places.
/// SOURCE: docs/spec/rules.md B-3 (no fraction of the minor unit; ISO 4217,
///         sources/iso-4217.md).
/// INVARIANTS:
///   1. amount = units × 10^-scale exactly; unit = 10^(scale − minor).
///   2. A currency with no minor unit (gold, SDRs) counts as two places.
///   3. NEVER rounds: an amount that doesn't fit is None.
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Steps {
    /// The amount, in steps.
    pub units: i128,
    /// One minor unit, in steps.
    pub unit: i128,
    /// Decimal places of one step.
    pub scale: u32,
    /// The currency's minor unit (two when it has none).
    pub minor: u32,
}

impl Steps {
    /// `amount` (not negative) in `currency`'s steps.
    pub fn of(amount: Decimal, currency: Currency) -> Option<Steps> {
        if amount.is_sign_negative() && !amount.is_zero() {
            return None;
        }
        let minor = currency.minor_units().unwrap_or(2);
        let n = amount.normalize();
        let scale = n.scale().max(minor);
        let units = n
            .mantissa()
            .checked_mul(10i128.checked_pow(scale - n.scale())?)?;
        Some(Steps {
            units,
            unit: 10i128.checked_pow(scale - minor)?,
            scale,
            minor,
        })
    }

    /// `n` steps as a decimal, with the currency's minor places at least
    /// (12,000 dollars prints 12000.00, 8,333 yen 8333). `None` past
    /// Decimal's range.
    pub fn decimal(&self, n: i128) -> Option<Decimal> {
        let d = Decimal::try_from_i128_with_scale(n, self.scale).ok()?;
        if self.scale == self.minor {
            return Some(d);
        }
        let mut d = d.normalize();
        if d.scale() < self.minor {
            d.rescale(self.minor);
        }
        Some(d)
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

    /// ISO 4217 list one (docs/spec/sources/iso-4217.md): one code from
    /// each exponent class, and the codes with no minor unit.
    #[test]
    fn minor_units_follow_iso_4217() {
        let units = |c: &str| Currency::new(c).unwrap().minor_units();
        assert_eq!(units("USD"), Some(2));
        assert_eq!(units("JPY"), Some(0));
        assert_eq!(units("BHD"), Some(3));
        assert_eq!(units("KWD"), Some(3));
        assert_eq!(units("CLF"), Some(4));
        assert_eq!(units("UYW"), Some(4));
        assert_eq!(units("XAU"), None);
        assert_eq!(units("XDR"), None);
    }

    #[test]
    fn currency_and_money_print_their_code_and_amount() {
        assert_eq!(Currency::new("usd").unwrap().to_string(), "USD");
        assert_eq!(format!("{:?}", Currency::EUR), "Currency(EUR)");
        assert_eq!(usd(d(1250)).to_string(), "12.50 USD");
        assert_eq!(format!("{:?}", usd(d(-5))), "-0.05 USD");
    }

    #[test]
    fn is_zero_means_exactly_zero() {
        assert!(Money::zero(Currency::USD).is_zero());
        assert!(usd(d(0)).is_zero());
        assert!(!usd(d(1)).is_zero());
        assert!(!usd(d(-1)).is_zero());
    }

    /// Steps INVARIANTS 1-3: cents for dollars, whole yen, thousandths of
    /// a dinar, two places for gold; an amount finer than its currency
    /// keeps its own places, with the unit a minor unit.
    #[test]
    fn steps_are_the_currencys_minor_unit() {
        let s =
            |a: &str, c: &str| Steps::of(a.parse().unwrap(), Currency::new(c).unwrap()).unwrap();
        let usd = s("12000", "USD");
        assert_eq!((usd.units, usd.unit, usd.scale), (1_200_000, 1, 2));
        assert_eq!(usd.decimal(1_148_387).unwrap().to_string(), "11483.87");
        assert_eq!(usd.decimal(1_200_000).unwrap().to_string(), "12000.00");
        let jpy = s("100000", "JPY");
        assert_eq!((jpy.units, jpy.unit, jpy.scale), (100_000, 1, 0));
        assert_eq!(jpy.decimal(8_333).unwrap().to_string(), "8333");
        let kwd = s("1000.000", "KWD");
        assert_eq!((kwd.units, kwd.unit, kwd.scale), (1_000_000, 1, 3));
        let gold = s("5", "XAU");
        assert_eq!((gold.units, gold.unit, gold.scale), (500, 1, 2));
        let fine = s("100.006", "USD");
        assert_eq!((fine.units, fine.unit, fine.scale), (100_006, 10, 3));
        assert_eq!(fine.decimal(100_010).unwrap().to_string(), "100.01");
        assert_eq!(fine.decimal(100_006).unwrap().to_string(), "100.006");
        assert_eq!(fine.decimal(0).unwrap().to_string(), "0.00");
        assert_eq!(Steps::of(d(-1), Currency::USD), None);
        assert_eq!(usd.decimal(i128::MAX), None);
    }

    #[test]
    fn checked_arithmetic_rejects_currency_mismatch() {
        let eur = Money::new(d(100), Currency::EUR);
        assert!(usd(d(100)).checked_add(eur).is_err());
        assert!(usd(d(100)).checked_sub(eur).is_err());
    }
}
