//! Exact money arithmetic.
//!
//! Kindly never uses floats for money. Amounts are stored as signed minor units
//! (cents) plus a currency. Percentages are integer basis points (1 % = 100 bps).
//! Rounding is half-away-from-zero and happens in exactly one place per
//! operation so that PayPal-facing values, ledger values and UI values agree.

use crate::error::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Currency {
    Usd,
    Eur,
    Gbp,
}

impl Currency {
    pub const ALL: [Currency; 3] = [Currency::Usd, Currency::Eur, Currency::Gbp];

    pub const fn code(self) -> &'static str {
        match self {
            Currency::Usd => "USD",
            Currency::Eur => "EUR",
            Currency::Gbp => "GBP",
        }
    }

    pub const fn symbol(self) -> &'static str {
        match self {
            Currency::Usd => "$",
            Currency::Eur => "€",
            Currency::Gbp => "£",
        }
    }

    pub const fn exponent(self) -> u32 {
        2
    }

    pub fn from_code(code: &str) -> CoreResult<Self> {
        match code.to_ascii_uppercase().as_str() {
            "USD" => Ok(Currency::Usd),
            "EUR" => Ok(Currency::Eur),
            "GBP" => Ok(Currency::Gbp),
            other => Err(CoreError::InvalidMoney(format!("unsupported currency {other}"))),
        }
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// An exact monetary amount: `minor` cents in `currency`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Money {
    pub minor: i64,
    pub currency: Currency,
}

impl Money {
    pub const fn new(minor: i64, currency: Currency) -> Self {
        Self { minor, currency }
    }

    pub const fn zero(currency: Currency) -> Self {
        Self { minor: 0, currency }
    }

    pub fn from_major(major: i64, currency: Currency) -> CoreResult<Self> {
        let scale = 10i64.pow(currency.exponent());
        major
            .checked_mul(scale)
            .map(|minor| Self { minor, currency })
            .ok_or(CoreError::Overflow { context: "from_major" })
    }

    /// Parse a decimal string such as `"1234.56"`, `"1,234.56"` or `"1234"`.
    pub fn parse(input: &str, currency: Currency) -> CoreResult<Self> {
        let cleaned: String = input.chars().filter(|c| *c != ',' && *c != ' ').collect();
        let cleaned = cleaned.trim();
        if cleaned.is_empty() {
            return Err(CoreError::InvalidMoney("empty amount".into()));
        }
        let (negative, body) = match cleaned.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, cleaned.strip_prefix('+').unwrap_or(cleaned)),
        };
        let mut parts = body.splitn(2, '.');
        let whole = parts.next().unwrap_or("0");
        let frac = parts.next().unwrap_or("");
        if whole.is_empty() && frac.is_empty() {
            return Err(CoreError::InvalidMoney(input.to_string()));
        }
        let whole: i64 = if whole.is_empty() {
            0
        } else {
            whole
                .parse()
                .map_err(|_| CoreError::InvalidMoney(input.to_string()))?
        };
        if frac.len() > currency.exponent() as usize {
            return Err(CoreError::InvalidMoney(format!(
                "too many decimal places for {}: {input}",
                currency.code()
            )));
        }
        let mut frac_digits = frac.to_string();
        while frac_digits.len() < currency.exponent() as usize {
            frac_digits.push('0');
        }
        let frac_value: i64 = if frac_digits.is_empty() {
            0
        } else {
            frac_digits
                .parse()
                .map_err(|_| CoreError::InvalidMoney(input.to_string()))?
        };
        let scale = 10i64.pow(currency.exponent());
        let minor = whole
            .checked_mul(scale)
            .and_then(|v| v.checked_add(frac_value))
            .ok_or(CoreError::Overflow { context: "parse" })?;
        Ok(Self {
            minor: if negative { -minor } else { minor },
            currency,
        })
    }

    pub const fn is_zero(self) -> bool {
        self.minor == 0
    }

    pub const fn is_positive(self) -> bool {
        self.minor > 0
    }

    pub const fn is_negative(self) -> bool {
        self.minor < 0
    }

    fn ensure_same(self, other: Money) -> CoreResult<()> {
        if self.currency != other.currency {
            return Err(CoreError::CurrencyMismatch {
                left: self.currency.code().to_string(),
                right: other.currency.code().to_string(),
            });
        }
        Ok(())
    }

    pub fn checked_add(self, other: Money) -> CoreResult<Money> {
        self.ensure_same(other)?;
        self.minor
            .checked_add(other.minor)
            .map(|minor| Money::new(minor, self.currency))
            .ok_or(CoreError::Overflow { context: "add" })
    }

    pub fn checked_sub(self, other: Money) -> CoreResult<Money> {
        self.ensure_same(other)?;
        self.minor
            .checked_sub(other.minor)
            .map(|minor| Money::new(minor, self.currency))
            .ok_or(CoreError::Overflow { context: "sub" })
    }

    pub fn sum<I: IntoIterator<Item = Money>>(amounts: I, currency: Currency) -> CoreResult<Money> {
        amounts.into_iter().try_fold(Money::zero(currency), |acc, m| acc.checked_add(m))
    }

    /// Multiply by a percentage expressed in basis points (100 bps = 1 %).
    /// Rounds half away from zero.
    pub fn bps(self, bps: i32) -> CoreResult<Money> {
        if !(-100_000..=100_000_000).contains(&bps) {
            return Err(CoreError::InvalidPercent(bps));
        }
        let scaled = (self.minor as i128) * (bps as i128);
        let rounded = divide_round_half_away_from_zero(scaled, 10_000);
        let minor = i64::try_from(rounded).map_err(|_| CoreError::Overflow { context: "bps" })?;
        Ok(Money::new(minor, self.currency))
    }

    /// Split into `n` parts as evenly as possible. The remainder lands on the
    /// earliest installments so the client is never asked to overpay.
    pub fn split_evenly(self, n: u8) -> CoreResult<Vec<Money>> {
        if n == 0 {
            return Err(CoreError::Validation("cannot split into zero parts".into()));
        }
        if self.minor < 0 {
            return Err(CoreError::Validation("cannot split a negative amount".into()));
        }
        let parts = n as i64;
        let base = self.minor / parts;
        let remainder = self.minor % parts;
        let mut out = Vec::with_capacity(n as usize);
        for i in 0..parts {
            let extra = if i < remainder { 1 } else { 0 };
            out.push(Money::new(base + extra, self.currency));
        }
        Ok(out)
    }

    /// PayPal-compatible decimal string, e.g. `"1234.56"`.
    pub fn paypal_value(self) -> String {
        let scale = 10i64.pow(self.currency.exponent());
        let sign = if self.minor < 0 { "-" } else { "" };
        let abs = self.minor.abs();
        format!("{}{}.{:0width$}", sign, abs / scale, abs % scale, width = self.currency.exponent() as usize)
    }

    /// Human display with thousands separators, e.g. `"1,234.56"`.
    pub fn display_value(self) -> String {
        let raw = self.paypal_value();
        let (sign, body) = match raw.strip_prefix('-') {
            Some(rest) => ("-", rest.to_string()),
            None => ("", raw),
        };
        let (whole, frac) = match body.split_once('.') {
            Some((w, f)) => (w.to_string(), Some(f.to_string())),
            None => (body, None),
        };
        let grouped = group_thousands(&whole);
        match frac {
            Some(frac) => format!("{sign}{grouped}.{frac}"),
            None => format!("{sign}{grouped}"),
        }
    }

    pub fn display_with_symbol(self) -> String {
        format!("{}{}", self.currency.symbol(), self.display_value())
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.display_value(), self.currency.code())
    }
}

impl FromStr for Money {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Money::parse(s, Currency::Usd)
    }
}

fn group_thousands(digits: &str) -> String {
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (idx, ch) in digits.chars().enumerate() {
        if idx > 0 && (digits.len() - idx) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn divide_round_half_away_from_zero(numerator: i128, denominator: i128) -> i128 {
    debug_assert!(denominator > 0);
    let sign = if numerator < 0 { -1 } else { 1 };
    let magnitude = numerator.abs();
    let quotient = magnitude / denominator;
    let remainder = magnitude % denominator;
    let rounded = if remainder * 2 >= denominator { quotient + 1 } else { quotient };
    sign * rounded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_format_round_trip() {
        let m = Money::parse("1,234.56", Currency::Usd).unwrap();
        assert_eq!(m.minor, 123_456);
        assert_eq!(m.paypal_value(), "1234.56");
        assert_eq!(m.display_value(), "1,234.56");
        assert_eq!(m.display_with_symbol(), "$1,234.56");
    }

    #[test]
    fn parse_handles_short_and_negative_values() {
        assert_eq!(Money::parse("12", Currency::Usd).unwrap().minor, 1200);
        assert_eq!(Money::parse("0.5", Currency::Usd).unwrap().minor, 50);
        assert_eq!(Money::parse("-3.25", Currency::Usd).unwrap().minor, -325);
        assert!(Money::parse("1.234", Currency::Usd).is_err());
        assert!(Money::parse("abc", Currency::Usd).is_err());
    }

    #[test]
    fn bps_rounds_half_away_from_zero() {
        let m = Money::new(240_000, Currency::Usd); // $2,400.00
        assert_eq!(m.bps(5000).unwrap(), Money::new(120_000, Currency::Usd)); // 50 %
        assert_eq!(m.bps(1000).unwrap(), Money::new(24_000, Currency::Usd)); // 10 %
        // Exactly half a cent rounds away from zero.
        assert_eq!(Money::new(100, Currency::Usd).bps(150).unwrap().minor, 2);
        assert_eq!(Money::new(100, Currency::Usd).bps(149).unwrap().minor, 1);
        assert!(Money::new(100, Currency::Usd).bps(200_000_000).is_err());
    }

    #[test]
    fn add_and_sub_reject_currency_mismatch() {
        let usd = Money::new(100, Currency::Usd);
        let eur = Money::new(100, Currency::Eur);
        assert!(usd.checked_add(eur).is_err());
        assert!(usd.checked_sub(eur).is_err());
    }

    #[test]
    fn splitting_never_loses_money() {
        let parts = Money::new(10_000, Currency::Usd).split_evenly(3).unwrap();
        assert_eq!(parts.len(), 3);
        let total = Money::sum(parts.clone(), Currency::Usd).unwrap();
        assert_eq!(total.minor, 10_000);
        assert_eq!(parts[0].minor, 3334);
        assert_eq!(parts[2].minor, 3333);
    }
}
