//! Arbitrary-precision numbers for the base, BCD and text-integer
//! operations: the decimal BigNumber that CyberChef's `bignumber.js` parses
//! and prints (20 decimal places, rounding half up, when a conversion has a
//! fraction). The integer arithmetic is `num-bigint`'s [`BigUint`].
//!
//! The bignumber.js layer stays ours: no Rust crate parses its syntax
//! (`0x1.8`, `1_000`, case retries, `RANGE` overflow) or converts fractions
//! between bases with its 20-place `ROUND_HALF_UP` digit loop, and both
//! decide the output of valid input.

pub use num_bigint::BigUint;

/// bignumber.js's `DECIMAL_PLACES`.
pub const DECIMAL_PLACES: usize = 20;
/// bignumber.js's `RANGE`: larger exponents are Infinity, smaller zero.
const MAX_EXP: i64 = 10_000_000;

const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

/// Digit values (each below `radix`), most significant first, as one
/// integer.
pub fn from_digits(digits: impl IntoIterator<Item = u32>, radix: u32) -> BigUint {
    let digits: Vec<u8> = digits.into_iter().map(|d| d as u8).collect();
    BigUint::from_radix_be(&digits, radix).unwrap_or_default()
}

/// A digit's value in base `radix`, ASCII only like bignumber.js's alphabet.
fn digit(c: char, radix: u32) -> Option<u32> {
    c.to_digit(36)
        .filter(|d| *d < radix && !c.is_ascii_uppercase())
}

/// A number written in base `from` as integer and fraction digit values,
/// rewritten in base `to`: the fraction to [`DECIMAL_PLACES`] digits,
/// rounded up when the next digit is at least half the base (bignumber.js's
/// `convertBase` with `ROUND_HALF_UP`), trailing zeros dropped.
pub fn convert_base(int: &[u32], frac: &[u32], from: u32, to: u32) -> String {
    let mut whole = from_digits(int.iter().copied(), from);
    if frac.is_empty() {
        return whole.to_str_radix(to);
    }
    // Multiply the fraction by `to` again and again; what carries out of it
    // is the next digit.
    let mut f = frac.to_vec();
    let mut out = Vec::with_capacity(DECIMAL_PLACES + 1);
    for _ in 0..=DECIMAL_PLACES {
        let mut carry = 0;
        for d in f.iter_mut().rev() {
            let v = *d * to + carry;
            *d = v % from;
            carry = v / from;
        }
        out.push(carry);
    }
    let next = out.pop().unwrap_or(0);
    if 2 * next >= to {
        let mut i = out.len();
        loop {
            if i == 0 {
                whole += 1u32;
                break;
            }
            i -= 1;
            out[i] += 1;
            if out[i] < to {
                break;
            }
            out[i] = 0;
        }
    }
    while out.last() == Some(&0) {
        out.pop();
    }
    let mut s = whole.to_str_radix(to);
    if !out.is_empty() {
        s.push('.');
        s.extend(out.iter().map(|d| DIGITS[*d as usize] as char));
    }
    s
}

/// A bignumber.js value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BigNumber {
    NaN,
    Infinity {
        neg: bool,
    },
    /// `0.d₁d₂… × 10^(exp+1)`: `digits` without leading or trailing zeros
    /// (empty for zero), `exp` the power of ten of the first digit.
    Finite {
        neg: bool,
        digits: Vec<u8>,
        exp: i64,
    },
}

impl BigNumber {
    pub fn is_zero(&self) -> bool {
        matches!(self, BigNumber::Finite { digits, .. } if digits.is_empty())
    }

    /// `new BigNumber(str)`, NaN where it throws (as CyberChef's BigNumber
    /// dish catches it): decimal or exponential notation, `Infinity`,
    /// `NaN`, `0x`/`0b`/`0o` prefixes and `_` between digits.
    pub fn parse(s: &str) -> BigNumber {
        if let Some((neg, body)) = numeric(s) {
            return parse_valid(neg, body);
        }
        let s = strip_space_or_plus(s);
        match s {
            "Infinity" => return BigNumber::Infinity { neg: false },
            "-Infinity" => return BigNumber::Infinity { neg: true },
            "NaN" | "-NaN" => return BigNumber::NaN,
            _ => {}
        }
        let (neg, unsigned) = match s.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, s),
        };
        let b = unsigned.as_bytes();
        if b.len() > 2 && b[0] == b'0' && b[2] != b'.' {
            let radix = match b[1] {
                b'x' | b'X' => Some(16),
                b'b' | b'B' => Some(2),
                b'o' | b'O' => Some(8),
                _ => None,
            };
            if let Some(radix) = radix {
                let rest = format!("{}{}", if neg { "-" } else { "" }, &unsigned[2..]);
                return parse_base(&rest, radix).unwrap_or(BigNumber::NaN);
            }
        }
        let cleaned = strip_digit_underscores(s);
        match numeric(&cleaned) {
            Some((neg, body)) => parse_valid(neg, body),
            None => BigNumber::NaN,
        }
    }

    /// `toFixed()`: every digit, no exponent; `-0` prints as `0`.
    pub fn to_fixed(&self) -> String {
        match self {
            BigNumber::NaN => "NaN".into(),
            BigNumber::Infinity { neg } => if *neg { "-Infinity" } else { "Infinity" }.into(),
            BigNumber::Finite { digits, .. } if digits.is_empty() => "0".into(),
            BigNumber::Finite { neg, digits, exp } => {
                let (int, frac) = split_point(digits, *exp);
                let int: String = if int.is_empty() {
                    "0".into()
                } else {
                    int.iter().map(|d| char::from(b'0' + d)).collect()
                };
                let frac: String = frac.iter().map(|d| char::from(b'0' + d)).collect();
                let sign = if *neg { "-" } else { "" };
                if frac.is_empty() {
                    format!("{sign}{int}")
                } else {
                    format!("{sign}{int}.{frac}")
                }
            }
        }
    }

    /// `toString(radix)`: the fraction rounded to 20 digits in that base. A
    /// negative number keeps its sign even if it rounds to `0`.
    pub fn to_radix(&self, radix: u32) -> String {
        match self {
            BigNumber::Finite { neg, digits, exp } => {
                let (int, frac) = split_point(digits, *exp);
                let int: Vec<u32> = int.iter().map(|d| u32::from(*d)).collect();
                let frac: Vec<u32> = frac.iter().map(|d| u32::from(*d)).collect();
                let s = convert_base(&int, &frac, 10, radix);
                if *neg && !digits.is_empty() {
                    format!("-{s}")
                } else {
                    s
                }
            }
            other => other.to_fixed(),
        }
    }
}

/// Integer and fraction digits of a finite value, with the zeros its
/// exponent implies.
fn split_point(digits: &[u8], exp: i64) -> (Vec<u8>, Vec<u8>) {
    if exp < 0 {
        let mut frac = vec![0; (-exp - 1) as usize];
        frac.extend_from_slice(digits);
        return (vec![], frac);
    }
    let point = exp as usize + 1;
    if point >= digits.len() {
        let mut int = digits.to_vec();
        int.resize(point, 0);
        (int, vec![])
    } else {
        (digits[..point].to_vec(), digits[point..].to_vec())
    }
}

/// bignumber.js's `isNumeric`: `-?(\d+(\.\d*)?|\.\d+)(e[+-]?\d+)?`, split
/// into the sign and the rest.
fn numeric(s: &str) -> Option<(bool, &str)> {
    let (neg, body) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let b = body.as_bytes();
    let digits = |from: usize| {
        b[from.min(b.len())..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .count()
    };
    let int = digits(0);
    let mut i = int;
    if b.get(i) == Some(&b'.') {
        let frac = digits(i + 1);
        if int == 0 && frac == 0 {
            return None;
        }
        i += 1 + frac;
    } else if int == 0 {
        return None;
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        let sign = usize::from(matches!(b.get(i + 1), Some(b'+' | b'-')));
        let exp = digits(i + 1 + sign);
        if exp == 0 {
            return None;
        }
        i += 1 + sign + exp;
    }
    (i == b.len()).then_some((neg, body))
}

/// bignumber.js's `parseValidString` on an unsigned [`numeric`] string.
fn parse_valid(neg: bool, s: &str) -> BigNumber {
    let (mantissa, exp_part) = match s.find(['e', 'E']) {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    let point = mantissa.find('.');
    let digits: Vec<u8> = mantissa
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|c| c - b'0')
        .collect();
    let mut e = point.unwrap_or(digits.len()) as i64;
    if let Some(x) = exp_part {
        // Saturates like JS's `+str` turning huge exponents into Infinity.
        let (sign, mag) = match x.as_bytes().first() {
            Some(b'-') => (-1, &x[1..]),
            Some(b'+') => (1, &x[1..]),
            _ => (1, x),
        };
        let mag = mag.parse::<i64>().unwrap_or(i64::MAX / 4).min(i64::MAX / 4);
        e = e.saturating_add(sign * mag);
    }
    let lead = digits.iter().take_while(|d| **d == 0).count();
    let trail = digits.iter().rev().take_while(|d| **d == 0).count();
    if lead == digits.len() {
        return BigNumber::Finite {
            neg,
            digits: vec![],
            exp: 0,
        };
    }
    let e = e - lead as i64 - 1;
    if e > MAX_EXP {
        return BigNumber::Infinity { neg };
    }
    if e < -MAX_EXP {
        return BigNumber::Finite {
            neg,
            digits: vec![],
            exp: 0,
        };
    }
    BigNumber::Finite {
        neg,
        digits: digits[lead..digits.len() - trail].to_vec(),
        exp: e,
    }
}

/// `str.replace(/^\s*\+(?!-)|^\s+|\s+$/g, "")`.
fn strip_space_or_plus(s: &str) -> &str {
    let lead = s.trim_start_matches(super::js::is_space);
    let s = match lead.strip_prefix('+') {
        Some(rest) if !rest.starts_with('-') => rest,
        _ => lead,
    };
    s.trim_end_matches(super::js::is_space)
}

/// `str.replace(/(\d)_(?=\d)/g, "$1")`.
fn strip_digit_underscores(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut prev_digit_kept = false;
    for (i, c) in s.char_indices() {
        let keep = !(c == '_' && prev_digit_kept && b.get(i + 1).is_some_and(u8::is_ascii_digit));
        if keep {
            out.push(c);
            prev_digit_kept = c.is_ascii_digit();
        } else {
            prev_digit_kept = false;
        }
    }
    out
}

/// What bignumber.js's `parseBaseString` reads from a string: its sign and
/// digit values either side of an optional point.
pub struct BaseDigits {
    pub neg: bool,
    pub int: Vec<u32>,
    pub frac: Vec<u32>,
}

/// bignumber.js's `parseBaseString`: a sign, digits of the base (all upper
/// case is read as lower case), `_` between digits, one point. `None` where
/// it throws "Not a base N number".
pub fn base_digits(s: &str, radix: u32) -> Option<BaseDigits> {
    let (neg, body) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s),
    };
    let mut text: String = body.to_string();
    let mut case_changed = false;
    'scan: loop {
        let chars: Vec<char> = text.chars().collect();
        let (mut int, mut frac) = (vec![], vec![]);
        let (mut has_dot, mut prev_numeral) = (false, false);
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if let Some(d) = digit(c, radix) {
                if has_dot {
                    frac.push(d)
                } else {
                    int.push(d)
                }
                prev_numeral = true;
                i += 1;
                continue;
            }
            if c == '_' {
                if prev_numeral && i + 1 < chars.len() {
                    prev_numeral = false;
                    i += 1;
                    continue;
                }
            } else if c == '.' {
                if i == 0 || (!has_dot && prev_numeral) {
                    // A trailing or lone point is dropped.
                    if i + 1 == chars.len() {
                        break;
                    }
                    has_dot = true;
                    prev_numeral = false;
                    i += 1;
                    continue;
                }
            } else if !case_changed {
                // Allow `FF` as well as `ff`; the digits-only alphabets of
                // bases up to 10 also try upper case, to no avail.
                let upper = text.to_uppercase();
                let lower = text.to_lowercase();
                if text == upper && !lower.is_empty() {
                    text = lower;
                    case_changed = true;
                    continue 'scan;
                }
                if text == lower && radix <= 10 && !upper.is_empty() {
                    text = upper;
                    case_changed = true;
                    continue 'scan;
                }
            }
            return None;
        }
        return Some(BaseDigits { neg, int, frac });
    }
}

/// `new BigNumber(str, radix)` once whitespace and a leading `+` are gone.
pub fn parse_base(s: &str, radix: u32) -> Option<BigNumber> {
    let d = base_digits(s, radix)?;
    let decimal = convert_base(&d.int, &d.frac, radix, 10);
    Some(parse_valid(d.neg, &decimal))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed(s: &str) -> String {
        BigNumber::parse(s).to_fixed()
    }

    #[test]
    fn digits_make_integers() {
        let n = from_digits(
            "123456789012345678901234567890"
                .chars()
                .map(|c| c as u32 - 48),
            10,
        );
        assert_eq!(n.to_str_radix(16), "18ee90ff6c373e0ee4e3f0ad2");
        assert_eq!(from_digits([], 2).to_str_radix(2), "0");
    }

    #[test]
    fn parses_like_bignumber_js() {
        assert_eq!(fixed("1e3"), "1000");
        assert_eq!(fixed(" +12 "), "12");
        assert_eq!(fixed("-0"), "0");
        assert_eq!(fixed("1_000"), "1000");
        assert_eq!(fixed("0x1.8"), "1.5");
        assert_eq!(fixed("0xFF"), "255");
        assert_eq!(fixed("0xFf"), "NaN");
        assert_eq!(fixed("0x.8"), "NaN");
        assert_eq!(fixed("1e"), "NaN");
        assert_eq!(fixed("-Infinity"), "-Infinity");
        assert_eq!(fixed("1.50e-2"), "0.015");
    }

    #[test]
    fn converts_bases_with_twenty_places() {
        assert_eq!(BigNumber::parse("0.1").to_radix(2), "0.0001100110011001101");
        assert_eq!(
            BigNumber::parse("0.5").to_radix(3),
            "0.11111111111111111111"
        );
        assert_eq!(BigNumber::parse("-1e-30").to_radix(2), "-0");
        assert_eq!(
            BigNumber::parse("0.99999999999999999999999").to_radix(10),
            "1"
        );
    }
}
