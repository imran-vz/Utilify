//! Numbers of any size: arbitrary bases, binary-coded decimal, and text read
//! as one big-endian integer. CyberChef does these with bignumber.js and
//! BigInt; [`super::bignum`] stands in for the first and `num-bigint` for
//! the second.

use num_traits::Zero;

use super::bignum::{BigNumber, BigUint, base_digits, from_digits};
use super::{CATEGORY, js, number_arg};
use crate::dish::{latin1_to_string, text_to_bytes};
use crate::{Arg, Args, Dish, OpError, Operation};

pub(super) fn ops() -> Vec<&'static Operation> {
    vec![
        &TO_BASE,
        &FROM_BASE,
        &TO_BCD,
        &FROM_BCD,
        &TEXT_INTEGER_CONVERSION,
    ]
}

pub static TO_BASE: Operation = Operation {
    name: "To Base",
    category: CATEGORY,
    description: "Converts a decimal number to a given numerical base.",
    info_url: Some("https://wikipedia.org/wiki/Radix"),
    args: &[Arg::number_in("Radix", 36.0, 2.0, 36.0)],
    run: |input, args| {
        let radix = number_arg(args, 0, "Radix", Some(2.0), Some(36.0), true)?;
        Ok(Dish::Text(
            BigNumber::parse(&input.text()).to_radix(radix as u32),
        ))
    },
};

pub static FROM_BASE: Operation = Operation {
    name: "From Base",
    category: CATEGORY,
    description: "Converts a number to decimal from a given numerical base.",
    info_url: Some("https://wikipedia.org/wiki/Radix"),
    args: &[Arg::number("Radix", 36.0)],
    run: from_base,
};

/// Once a fraction digit's place value passes this, it rounds to nothing at
/// 20 decimal places.
const NEGLIGIBLE_PLACE: u128 = 1_000_000_000_000_000_000_000_000;

/// bignumber.js's errors carry CyberChef's "From Base - " prefix for
/// thrown non-`OperationError`s; the radix range check is an
/// `OperationError` and has none.
fn from_base(input: Dish, args: &Args) -> Result<Dish, OpError> {
    let radix = number_arg(args, 0, "Radix", None, None, false)?;
    if !(2.0..=36.0).contains(&radix) {
        return Err(OpError::new(
            "Error: Radix argument must be between 2 and 36",
        ));
    }
    if radix.fract() != 0.0 {
        return Err(OpError::new(format!(
            "From Base - [BigNumber Error] Base not an integer: {}",
            js::number(radix)
        )));
    }
    let radix = radix as u32;
    let not_base = |s: &str| {
        OpError::new(format!(
            "From Base - [BigNumber Error] Not a base {radix} number: {s}"
        ))
    };
    let text: String = input.text().chars().filter(|c| !js::is_space(*c)).collect();
    let mut parts = text.split('.');
    let int_text = parts.next().unwrap_or_default();
    let int = base_digits(strip_plus(int_text), radix).ok_or_else(|| not_base(int_text))?;
    let whole = from_digits(int.int, radix);
    let neg = int.neg;

    // Each fraction digit adds digit / radix^place, that quotient rounded
    // (half up) to 20 decimal places: the sum in units of 1e-20.
    let mut frac_units: u128 = 0;
    let mut place = Some(1u128);
    for c in parts.next().unwrap_or_default().chars() {
        let s = c.to_string();
        let d = base_digits(strip_plus(&s), radix).ok_or_else(|| not_base(&s))?;
        let digit = u128::from(d.int.first().copied().unwrap_or(0));
        place = place
            .and_then(|p| p.checked_mul(u128::from(radix)))
            .filter(|p| *p <= NEGLIGIBLE_PLACE);
        if let Some(p) = place {
            frac_units += (2 * digit * 10u128.pow(20) + p) / (2 * p);
        }
    }

    let scaled = whole * BigUint::from(10u128.pow(20));
    let frac = BigUint::from(frac_units);
    let (neg, magnitude) = match (neg, scaled.cmp(&frac)) {
        (false, _) => (false, scaled + frac),
        (true, std::cmp::Ordering::Less) => (false, frac - scaled),
        (true, _) => (true, scaled - frac),
    };
    Ok(Dish::Text(fixed_point(neg, &magnitude)))
}

/// `str.replace(/^\+(?!-)/, "")`, what bignumber.js does to a string given
/// with a base once whitespace is gone.
fn strip_plus(s: &str) -> &str {
    match s.strip_prefix('+') {
        Some(rest) if !rest.starts_with('-') => rest,
        _ => s,
    }
}

/// A count of 1e-20 units printed like `BigNumber.toFixed()`.
fn fixed_point(neg: bool, units: &BigUint) -> String {
    let digits = units.to_str_radix(10);
    let digits = format!("{digits:0>21}");
    let (int, frac) = digits.split_at(digits.len() - 20);
    let frac = frac.trim_end_matches('0');
    let sign = if neg && !units.is_zero() { "-" } else { "" };
    if frac.is_empty() {
        format!("{sign}{int}")
    } else {
        format!("{sign}{int}.{frac}")
    }
}

const BCD_SCHEMES: &[&str] = &[
    "8 4 2 1",
    "7 4 2 1",
    "4 2 2 1",
    "2 4 2 1",
    "8 4 -2 -1",
    "Excess-3",
    "IBM 8 4 2 1",
];

const BCD_FORMATS: &[&str] = &["Nibbles", "Bytes", "Raw"];

/// Each scheme's nibble for the digits 0-9. CyberChef's `ENCODING_LOOKUP`.
fn bcd_encoding(scheme: &str) -> [u8; 10] {
    match scheme {
        "7 4 2 1" => [0, 1, 2, 3, 4, 5, 6, 8, 9, 10],
        "4 2 2 1" => [0, 1, 4, 5, 8, 9, 12, 13, 14, 15],
        "2 4 2 1" => [0, 1, 2, 3, 4, 11, 12, 13, 14, 15],
        "8 4 -2 -1" => [0, 7, 6, 5, 4, 11, 10, 9, 8, 15],
        "Excess-3" => [3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
        "IBM 8 4 2 1" => [10, 1, 2, 3, 4, 5, 6, 7, 8, 9],
        _ => [0, 1, 2, 3, 4, 5, 6, 7, 8, 9],
    }
}

pub static TO_BCD: Operation = Operation {
    name: "To BCD",
    category: CATEGORY,
    description: "Binary-Coded Decimal (BCD) is a class of binary encodings of decimal numbers where each decimal digit is represented by a fixed number of bits, usually four or eight. Special bit patterns are sometimes used for a sign",
    info_url: Some("https://wikipedia.org/wiki/Binary-coded_decimal"),
    args: &[
        Arg::choice("Scheme", BCD_SCHEMES),
        Arg::flag("Packed", true),
        Arg::flag("Signed", false),
        Arg::choice("Output format", BCD_FORMATS),
    ],
    run: to_bcd,
};

fn to_bcd(input: Dish, args: &Args) -> Result<Dish, OpError> {
    let n = BigNumber::parse(&input.text());
    let BigNumber::Finite { neg, digits, exp } = &n else {
        // CyberChef crashes on Infinity rather than saying so.
        return Err(OpError::new("Invalid input"));
    };
    if !digits.is_empty() && digits.len() as i64 > exp + 1 {
        return Err(OpError::new("Fractional values are not supported by BCD"));
    }
    let encoding = bcd_encoding(args.str(0));
    let (packed, signed) = (args.flag(1), args.flag(2));
    let fixed = n.to_fixed();
    let digits: Vec<u8> = fixed
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|d| d - b'0')
        .collect();
    let mut nibbles: Vec<u8> = digits.iter().map(|d| encoding[*d as usize]).collect();
    if signed {
        // A leading 0 keeps the sign nibble out of a byte of its own.
        if packed && digits.len().is_multiple_of(2) {
            nibbles.insert(0, encoding[0]);
        }
        let positive = !neg && !n.is_zero();
        nibbles.push(if positive { 12 } else { 13 });
    }
    let bytes: Vec<u8> = if packed {
        nibbles
            .chunks(2)
            .map(|p| (p[0] << 4) | p.get(1).copied().unwrap_or(0))
            .collect()
    } else {
        let bytes = nibbles.clone();
        nibbles = nibbles.iter().flat_map(|n| [0, *n]).collect();
        bytes
    };
    let out = match args.str(3) {
        "Nibbles" | "" => nibbles
            .iter()
            .map(|n| format!("{n:04b}"))
            .collect::<Vec<_>>()
            .join(" "),
        "Bytes" => bytes
            .iter()
            .map(|b| format!("{b:08b}"))
            .collect::<Vec<_>>()
            .join(" "),
        _ => latin1_to_string(&bytes),
    };
    Ok(Dish::Text(out))
}

pub static FROM_BCD: Operation = Operation {
    name: "From BCD",
    category: CATEGORY,
    description: "Binary-Coded Decimal (BCD) is a class of binary encodings of decimal numbers where each decimal digit is represented by a fixed number of bits, usually four or eight. Special bit patterns are sometimes used for a sign.",
    info_url: Some("https://wikipedia.org/wiki/Binary-coded_decimal"),
    args: &[
        Arg::choice("Scheme", BCD_SCHEMES),
        Arg::flag("Packed", true),
        Arg::flag("Signed", false),
        Arg::choice("Input format", BCD_FORMATS),
    ],
    run: from_bcd,
};

fn from_bcd(input: Dish, args: &Args) -> Result<Dish, OpError> {
    let encoding = bcd_encoding(args.str(0));
    let (packed, signed) = (args.flag(1), args.flag(2));
    let text = input.text();
    // NaN where a group of four isn't binary.
    let mut nibbles: Vec<f64> = match args.str(3) {
        "Nibbles" | "Bytes" | "" => {
            let units: Vec<u16> = text
                .chars()
                .filter(|c| !js::is_space(*c))
                .collect::<String>()
                .encode_utf16()
                .collect();
            units
                .chunks(4)
                .map(|c| js::parse_int(&String::from_utf16_lossy(c), 2))
                .collect()
        }
        _ => text_to_bytes(&text)
            .iter()
            .flat_map(|b| [f64::from(b >> 4), f64::from(b & 15)])
            .collect(),
    };
    if !packed {
        // Every high nibble goes, which CyberChef does by splicing out
        // every other element from the first.
        nibbles = nibbles.into_iter().skip(1).step_by(2).collect();
    }
    let mut out = String::new();
    if signed && matches!(nibbles.pop(), Some(s) if s == 13.0 || s == 11.0) {
        out.push('-');
    }
    for n in nibbles {
        if n.is_nan() {
            return Err(OpError::new("Invalid input"));
        }
        let Some(digit) = encoding.iter().position(|e| f64::from(*e) == n) else {
            return Err(OpError::new(format!(
                "Value {:04b} is not in the encoding scheme",
                n as u32
            )));
        };
        out.push(char::from(b'0' + digit as u8));
    }
    if out.is_empty() || out == "-" {
        // `new BigNumber("")` throws.
        return Err(OpError::new(format!(
            "[BigNumber Error] Not a number: {out}"
        )));
    }
    Ok(Dish::Text(BigNumber::parse(&out).to_fixed()))
}

pub static TEXT_INTEGER_CONVERSION: Operation = Operation {
    name: "Text-Integer Conversion",
    category: CATEGORY,
    description: "Converts between text strings and large integers (decimal or hexadecimal).\n\nText is interpreted as a big-endian sequence of character codes. For example:\nABC is 0x414243 (hex) is 4276803 (decimal)\nInput format detection:\nDecimal: digits 0-9 only\nHexadecimal: 0x... prefix\nQuoted or unquoted text: treated as string\n\nCharacter limitations:\nText input may only contain ASCII and Latin-1 characters (code point < 256).\nMulti-byte Unicode characters will generate an error.\n\n.",
    info_url: Some("https://wikipedia.org/wiki/Endianness"),
    args: &[Arg::choice(
        "Output format",
        &["String", "Decimal", "Hexadecimal"],
    )],
    run: text_integer,
};

/// Text's UTF-16 code units as big-endian bytes; CyberChef refuses any
/// unit past Latin-1.
fn text_to_integer(text: &str) -> Result<BigUint, OpError> {
    let mut bytes = Vec::with_capacity(text.len());
    for (i, unit) in text.encode_utf16().enumerate() {
        let b = u8::try_from(unit).map_err(|_| {
            OpError::new(format!(
                "Character at position {i} exceeds Latin-1 range (0-255).\nOnly ASCII and Latin-1 characters are supported."
            ))
        })?;
        bytes.push(b);
    }
    Ok(BigUint::from_bytes_be(&bytes))
}

/// `/^["'].*["']$/`: quotes at both ends, no line break between.
fn quoted(s: &str) -> Option<&str> {
    let b = s.as_bytes();
    let quote = |c: &u8| *c == b'"' || *c == b'\'';
    let breaks = |c: char| matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}');
    (b.len() >= 2 && quote(&b[0]) && quote(&b[b.len() - 1]) && !s.contains(breaks))
        .then(|| &s[1..s.len() - 1])
}

fn text_integer(input: Dish, args: &Args) -> Result<Dish, OpError> {
    let text = input.text();
    let trimmed = js::trim(&text);
    let digits =
        |s: &str, radix: u32| from_digits(s.chars().filter_map(|c| c.to_digit(radix)), radix);
    let hex = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .filter(|h| !h.is_empty() && h.chars().all(|c| c.is_ascii_hexdigit()));
    let (sign, unsigned) = match trimmed.as_bytes().first() {
        Some(b'-') => (true, &trimmed[1..]),
        Some(b'+') => (false, &trimmed[1..]),
        _ => (false, trimmed),
    };
    let (neg, value) = if let Some(h) = hex {
        (false, digits(h, 16))
    } else if !unsigned.is_empty() && unsigned.bytes().all(|c| c.is_ascii_digit()) {
        (sign, digits(unsigned, 10))
    } else if let Some(inner) = quoted(trimmed) {
        (false, text_to_integer(inner)?)
    } else {
        (false, text_to_integer(trimmed)?)
    };
    let neg = neg && !value.is_zero();
    let sign = if neg { "-" } else { "" };
    Ok(Dish::Text(match args.str(0) {
        "String" | "" if neg => String::new(),
        // num-bigint writes zero as one 0 byte; BigInt-to-text gives none.
        "String" | "" if value.is_zero() => String::new(),
        "String" | "" => latin1_to_string(&value.to_bytes_be()),
        "Decimal" => format!("{sign}{}", value.to_str_radix(10)),
        _ => format!("0x{sign}{}", value.to_str_radix(16)),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ArgValue;

    fn run(op: &Operation, input: &str, args: &[ArgValue]) -> String {
        match op.apply(Dish::Text(input.into()), args) {
            Ok(d) => d.into_text(),
            Err(e) => e.0,
        }
    }

    #[test]
    fn to_base() {
        let r = |input: &str, radix: f64| run(&TO_BASE, input, &[ArgValue::Num(radix)]);
        assert_eq!(r("255", 16.), "ff");
        assert_eq!(r("", 16.), "NaN");
        assert_eq!(r("-255.5", 16.), "-ff.8");
        assert_eq!(r("0.1", 3.), "0.00220022002200220022");
        assert_eq!(r("1e30", 36.), "2oy99wnkl1c76diocq9s");
        assert_eq!(r("0x1.8", 10.), "1.5");
        assert_eq!(
            r("0.123456789012345678901234567", 10.),
            "0.1234567890123456789"
        );
        assert_eq!(r("0.000000000000000000005", 10.), "0.00000000000000000001");
        assert_eq!(r("1.5e-3", 7.), "0.00034132126054162545");
        assert_eq!(r("Infinity", 10.), "Infinity");
        assert_eq!(r("12", 1.), "Radix must be greater than or equal to 2.");
        assert_eq!(r("12", 37.), "Radix must be less than or equal to 36.");
        assert_eq!(r("12", 2.5), "Radix must be an integer.");
    }

    #[test]
    fn from_base() {
        let r = |input: &str, radix: f64| run(&FROM_BASE, input, &[ArgValue::Num(radix)]);
        assert_eq!(r("", 36.), "0");
        assert_eq!(r("-", 10.), "0");
        assert_eq!(r("1_0", 10.), "10");
        assert_eq!(r("ZZ", 36.), "1295");
        assert_eq!(
            r("Zz", 36.),
            "From Base - [BigNumber Error] Not a base 36 number: Zz"
        );
        assert_eq!(
            r("zz", 10.),
            "From Base - [BigNumber Error] Not a base 10 number: zz"
        );
        assert_eq!(
            r("1.x", 10.),
            "From Base - [BigNumber Error] Not a base 10 number: x"
        );
        assert_eq!(r("1.-", 10.), "1");
        assert_eq!(r("+5", 10.), "5");
        assert_eq!(r("-1.5", 10.), "-0.5");
        assert_eq!(r("-.5", 10.), "0.5");
        assert_eq!(r("1 2\n3.4 5", 10.), "123.45");
        assert_eq!(r("1.2.3", 10.), "1.2");
        assert_eq!(
            r("0.12121212121212121212121212", 3.),
            "0.62499999999975411763"
        );
        assert_eq!(r("0.000000000000000000005", 10.), "0.00000000000000000001");
        assert_eq!(r("1.Z", 36.), "1.97222222222222222222");
        assert_eq!(
            r("123456789abcdefghijklmnopqrstuvwxyz", 36.),
            "86846823611197163108337531226495015298096208677436155"
        );
        assert_eq!(r("1", 1.), "Error: Radix argument must be between 2 and 36");
        assert_eq!(
            r("1", 10.5),
            "From Base - [BigNumber Error] Base not an integer: 10.5"
        );
    }

    #[test]
    fn bcd() {
        let a = |scheme: &str, packed: bool, signed: bool, format: &str| {
            vec![
                ArgValue::Str(scheme.into()),
                ArgValue::Bool(packed),
                ArgValue::Bool(signed),
                ArgValue::Str(format.into()),
            ]
        };
        let d = a("8 4 2 1", true, false, "Nibbles");
        assert_eq!(run(&TO_BCD, "", &d), "Invalid input");
        assert_eq!(
            run(&TO_BCD, "1.5", &d),
            "Fractional values are not supported by BCD"
        );
        assert_eq!(run(&TO_BCD, "1.0", &d), "0001");
        assert_eq!(
            run(&TO_BCD, "1e3", &a("8 4 2 1", true, true, "Bytes")),
            "00000001 00000000 00001100"
        );
        assert_eq!(
            run(&TO_BCD, "-0", &a("8 4 2 1", true, true, "Nibbles")),
            "0000 1101"
        );
        assert_eq!(
            run(&TO_BCD, "-12", &a("8 4 2 1", false, true, "Bytes")),
            "00000001 00000010 00001101"
        );
        assert_eq!(
            run(&TO_BCD, "123", &a("8 4 2 1", true, false, "Raw")),
            "\u{12}0"
        );
        assert_eq!(run(&TO_BCD, " 7 ", &a("8 4 2 1", true, true, "Raw")), "|");

        assert_eq!(run(&FROM_BCD, "0001 0010 001", &d), "121");
        assert_eq!(
            run(&FROM_BCD, "0001 1111", &d),
            "Value 1111 is not in the encoding scheme"
        );
        assert_eq!(run(&FROM_BCD, "0001 2222", &d), "Invalid input");
        assert_eq!(run(&FROM_BCD, "0001 0x10", &d), "10");
        assert_eq!(run(&FROM_BCD, "0000 0000 0001", &d), "1");
        assert_eq!(run(&FROM_BCD, "", &d), "[BigNumber Error] Not a number: ");
        let signed = a("8 4 2 1", true, true, "Nibbles");
        assert_eq!(
            run(&FROM_BCD, "1101", &signed),
            "[BigNumber Error] Not a number: -"
        );
        assert_eq!(run(&FROM_BCD, "0001 1011", &signed), "-1");
        let unpacked = a("8 4 2 1", false, false, "Bytes");
        assert_eq!(run(&FROM_BCD, "0000 0001 0000 0010 0000", &unpacked), "12");
        assert_eq!(
            run(&FROM_BCD, "\u{12}\u{34}", &a("8 4 2 1", true, false, "Raw")),
            "1234"
        );
        assert_eq!(
            run(&FROM_BCD, "é", &a("8 4 2 1", true, false, "Raw")),
            "Value 1110 is not in the encoding scheme"
        );
    }

    #[test]
    fn text_integer() {
        let r =
            |input: &str, f: &str| run(&TEXT_INTEGER_CONVERSION, input, &[ArgValue::Str(f.into())]);
        assert_eq!(r("", "Decimal"), "0");
        assert_eq!(r("  ", "Hexadecimal"), "0x0");
        assert_eq!(r("-5", "String"), "");
        assert_eq!(r("-5", "Hexadecimal"), "0x-5");
        assert_eq!(r("-0", "Decimal"), "0");
        assert_eq!(r("0x", "Decimal"), "12408");
        assert_eq!(r("0X1f", "Decimal"), "31");
        assert_eq!(r("\"a'", "Decimal"), "97");
        assert_eq!(r("\"a\nb\"", "Decimal"), "147656958498");
        assert_eq!(r("256", "String"), "\u{1}\u{0}");
        assert_eq!(r("\u{feff}42", "Hexadecimal"), "0x2a");
        assert_eq!(r("1.5", "Decimal"), "3223093");
        assert_eq!(
            r("x😀", "Decimal"),
            "Character at position 1 exceeds Latin-1 range (0-255).\nOnly ASCII and Latin-1 characters are supported."
        );
        assert_eq!(
            r(
                "113091951015816448506195587157728348242683688608116",
                "String"
            ),
            "Mary had a little cat"
        );
    }
}
