//! Exact integer parsing of Kubernetes quantities (`250m`, `512Mi`, `129e6`, ...).
//!
//! No floating point is involved, so every valid quantity converts exactly. Negative
//! quantities are rejected because CPU and memory usage are never negative.

/// A mantissa digit count above this cannot be held in a `u128` mantissa with room to scale.
const MAX_MANTISSA_DIGITS: usize = 38;
/// `10^38` is the largest power of ten that fits in a `u128`.
const MAX_POWER_OF_TEN: u32 = 38;

const BINARY_SUFFIXES: [(&str, u32); 6] = [
    ("Ki", 10),
    ("Mi", 20),
    ("Gi", 30),
    ("Ti", 40),
    ("Pi", 50),
    ("Ei", 60),
];
const DECIMAL_SUFFIXES: [(&str, i32); 9] = [
    ("n", -9),
    ("u", -6),
    ("m", -3),
    ("k", 3),
    ("M", 6),
    ("G", 9),
    ("T", 12),
    ("P", 15),
    ("E", 18),
];

/// CPU in nanocores, from a Kubernetes quantity such as `250m`, `0.5`, `2`, or `1234567n`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CpuAmount {
    nanocores: u64,
}

impl CpuAmount {
    /// `None` for malformed text, a negative value, or a value above `u64::MAX` nanocores.
    pub fn parse(text: &str) -> Option<Self> {
        let nanocores = parse_quantity(text)?.to_unit(9)?;
        Some(Self { nanocores })
    }

    pub fn from_nanocores(nanocores: u64) -> Self {
        Self { nanocores }
    }

    pub fn nanocores(self) -> u64 {
        self.nanocores
    }

    pub fn cores(self) -> f64 {
        self.nanocores as f64 / 1e9
    }
}

/// Bytes, from a quantity such as `512Mi`, `1G`, `129e6`, or `16384256Ki`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ByteAmount {
    bytes: u64,
}

impl ByteAmount {
    /// `None` for malformed text, a negative value, or a value above `u64::MAX` bytes.
    pub fn parse(text: &str) -> Option<Self> {
        let bytes = parse_quantity(text)?.to_unit(0)?;
        Some(Self { bytes })
    }

    pub fn from_bytes(bytes: u64) -> Self {
        Self { bytes }
    }

    pub fn bytes(self) -> u64 {
        self.bytes
    }
}

/// `numerator / denominator` of two quantities of any unit (`180Gi` / `192Gi`, `9k` / `1k`).
/// `None` when either is malformed or the denominator is zero. The ratio is for comparing
/// and drawing, so it is a float, not exact.
pub fn quantity_ratio(numerator: &str, denominator: &str) -> Option<f64> {
    let denominator = parse_quantity(denominator)?.to_f64();
    if denominator == 0.0 {
        return None;
    }
    Some(parse_quantity(numerator)?.to_f64() / denominator)
}

/// `mantissa x 10^exponent x binary`, exact.
struct Quantity {
    mantissa: u128,
    exponent: i32,
    binary: u128,
}

impl Quantity {
    fn to_f64(&self) -> f64 {
        (self.mantissa as f64) * (self.binary as f64) * 10f64.powi(self.exponent)
    }

    /// The value in units of `10^unit_exponent`, rounded up, or `None` above `u64::MAX`.
    fn to_unit(&self, unit_exponent: i32) -> Option<u64> {
        let value = self.mantissa.checked_mul(self.binary)?;
        let shift = self.exponent.saturating_add(unit_exponent);
        let scaled = if shift >= 0 {
            if value == 0 {
                return Some(0);
            }
            let factor = power_of_ten(shift.unsigned_abs())?;
            value.checked_mul(factor)?
        } else {
            // A divisor beyond 10^38 exceeds any `u128` value, so only zero stays zero.
            match power_of_ten(shift.unsigned_abs()) {
                Some(divisor) => divide_up(value, divisor),
                None => u128::from(value != 0),
            }
        };
        u64::try_from(scaled).ok()
    }
}

fn power_of_ten(power: u32) -> Option<u128> {
    if power > MAX_POWER_OF_TEN {
        return None;
    }
    10u128.checked_pow(power)
}

fn divide_up(value: u128, divisor: u128) -> u128 {
    // Kubernetes rounds a quantity up to the unit, so `1.5n` of nanocores is `2n`.
    value / divisor + u128::from(!value.is_multiple_of(divisor))
}

fn parse_quantity(text: &str) -> Option<Quantity> {
    let text = text.trim();
    let text = text.strip_prefix('+').unwrap_or(text);
    let number_len = text
        .find(|character: char| !character.is_ascii_digit() && character != '.')
        .unwrap_or(text.len());
    let (number, suffix) = text.split_at(number_len);
    let (mantissa, fraction_exponent) = parse_number(number)?;
    let (exponent, binary) = parse_suffix(suffix)?;
    Some(Quantity {
        mantissa,
        exponent: fraction_exponent.saturating_add(exponent),
        binary,
    })
}

/// Digits with an optional fraction: the mantissa and the exponent the fraction implies.
fn parse_number(number: &str) -> Option<(u128, i32)> {
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    if whole.is_empty() && fraction.is_empty() {
        return None;
    }
    // Trailing zeros add nothing, and counting them would reject `1.0000...0Ki`.
    let fraction = fraction.trim_end_matches('0');
    let mut mantissa: u128 = 0;
    let mut significant_digits = 0;
    for character in whole.chars().chain(fraction.chars()) {
        let digit = character.to_digit(10)?;
        if mantissa != 0 || digit != 0 {
            significant_digits += 1;
        }
        if significant_digits > MAX_MANTISSA_DIGITS {
            return None;
        }
        mantissa = mantissa * 10 + u128::from(digit);
    }
    let fraction_digits = i32::try_from(fraction.len()).ok()?;
    Some((mantissa, -fraction_digits))
}

/// The decimal exponent and binary factor a suffix adds. Exactly one kind is allowed.
fn parse_suffix(suffix: &str) -> Option<(i32, u128)> {
    if suffix.is_empty() {
        return Some((0, 1));
    }
    if let Some((_, power)) = BINARY_SUFFIXES.iter().find(|(name, _)| *name == suffix) {
        return Some((0, 1u128 << power));
    }
    if let Some((_, exponent)) = DECIMAL_SUFFIXES.iter().find(|(name, _)| *name == suffix) {
        return Some((*exponent, 1));
    }
    // `E` alone is exa and was matched above; `E3` and `e3` are exponents.
    let digits = suffix.strip_prefix(['e', 'E'])?;
    Some((digits.parse::<i32>().ok()?, 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(text: &str) -> Option<u64> {
        ByteAmount::parse(text).map(ByteAmount::bytes)
    }

    fn nanocores(text: &str) -> Option<u64> {
        CpuAmount::parse(text).map(CpuAmount::nanocores)
    }

    #[test]
    fn parses_plain_and_fractional_numbers() {
        assert_eq!(bytes("2"), Some(2));
        assert_eq!(nanocores("2"), Some(2_000_000_000));
        assert_eq!(nanocores("0.5"), Some(500_000_000));
        assert_eq!(nanocores(".5"), Some(500_000_000));
        assert_eq!(nanocores("5."), Some(5_000_000_000));
        assert_eq!(nanocores("+1"), Some(1_000_000_000));
        assert_eq!(nanocores(" 1 "), Some(1_000_000_000));
    }

    #[test]
    fn parses_decimal_suffixes() {
        assert_eq!(nanocores("250m"), Some(250_000_000));
        assert_eq!(nanocores("1234567n"), Some(1_234_567));
        assert_eq!(nanocores("3u"), Some(3_000));
        assert_eq!(bytes("1k"), Some(1_000));
        assert_eq!(bytes("1G"), Some(1_000_000_000));
        assert_eq!(bytes("2M"), Some(2_000_000));
    }

    #[test]
    fn parses_binary_suffixes() {
        assert_eq!(bytes("512Mi"), Some(536_870_912));
        assert_eq!(bytes("16384256Ki"), Some(16_777_478_144));
        assert_eq!(bytes("1.5Gi"), Some(1_610_612_736));
        assert_eq!(bytes("1Ei"), Some(1 << 60));
    }

    #[test]
    fn parses_exponents() {
        assert_eq!(bytes("129e6"), Some(129_000_000));
        assert_eq!(bytes("1E3"), Some(1_000));
        assert_eq!(bytes("1E"), Some(1_000_000_000_000_000_000));
        assert_eq!(bytes("2e+3"), Some(2_000));
        assert_eq!(nanocores("1e-3"), Some(1_000_000));
    }

    #[test]
    fn rounds_sub_unit_values_up() {
        assert_eq!(nanocores("1.5n"), Some(2));
        assert_eq!(nanocores("1.4n"), Some(2));
        assert_eq!(nanocores("1.0n"), Some(1));
        assert_eq!(bytes("0.4"), Some(1));
        assert_eq!(bytes("0"), Some(0));
        // Far below one unit: the divisor no longer fits in a `u128`, yet a value is not zero.
        assert_eq!(bytes("1e-100"), Some(1));
        assert_eq!(bytes("0e-100"), Some(0));
    }

    #[test]
    fn long_fraction_zeros_do_not_overflow() {
        let zeros = "0".repeat(45);
        assert_eq!(bytes(&format!("1.{zeros}Ki")), Some(1_024));
        assert_eq!(bytes(&format!("2.{zeros}")), Some(2));
        assert_eq!(bytes(".0"), Some(0));
    }

    #[test]
    fn rejects_malformed_text() {
        for text in [
            "", " ", "-1", "1.2.3", "5x", "Mi", "1 Gi", ".", "+", "e3", "1e",
        ] {
            assert_eq!(bytes(text), None, "{text:?}");
        }
    }

    #[test]
    fn rejects_exponent_with_suffix() {
        assert_eq!(bytes("1e3Ki"), None);
        assert_eq!(nanocores("1.5e2m"), None);
    }

    #[test]
    fn rejects_values_beyond_u64() {
        assert_eq!(bytes("20Ei"), None);
        assert_eq!(bytes("18446744073709551616"), None);
        assert_eq!(bytes("18446744073709551615"), Some(u64::MAX));
        let digits = "9".repeat(39);
        assert_eq!(bytes(&digits), None);
        assert_eq!(bytes("1e999999999"), None);
    }

    #[test]
    fn quantity_ratio_mixes_units() {
        assert_eq!(quantity_ratio("9k", "1k"), Some(9.0));
        assert_eq!(quantity_ratio("500m", "1"), Some(0.5));
        let ratio = quantity_ratio("180Gi", "192Gi").unwrap_or_default();
        assert!((ratio - 0.9375).abs() < 1e-9);
        assert_eq!(quantity_ratio("1Gi", "512Mi"), Some(2.0));
    }

    #[test]
    fn quantity_ratio_rejects_zero_and_junk() {
        assert_eq!(quantity_ratio("1", "0"), None);
        assert_eq!(quantity_ratio("1", "junk"), None);
        assert_eq!(quantity_ratio("junk", "1"), None);
        assert_eq!(quantity_ratio("0", "1"), Some(0.0));
    }

    #[test]
    fn amounts_expose_their_unit() {
        assert_eq!(CpuAmount::from_nanocores(1_500_000_000).cores(), 1.5);
        assert_eq!(ByteAmount::from_bytes(7).bytes(), 7);
    }
}
