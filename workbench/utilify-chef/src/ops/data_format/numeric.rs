//! Numbers: bytes as character codes, decimal, binary, octal and IEEE 754
//! floats, plus IP address formats. Arbitrary bases, BCD and text as an
//! integer live in [`radix`].

mod bignum;
pub(crate) mod js;
mod radix;

use super::CATEGORY;
use crate::util::{char_rep, from_hex, split_delim, strip_delim};
use crate::{Arg, ArgKind, Args, Dish, OpError, Operation};

pub(super) fn ops() -> Vec<&'static Operation> {
    let mut ops = vec![
        &TO_CHARCODE,
        &FROM_CHARCODE,
        &TO_DECIMAL,
        &FROM_DECIMAL,
        &TO_FLOAT,
        &FROM_FLOAT,
        &TO_BINARY,
        &FROM_BINARY,
        &TO_OCTAL,
        &FROM_OCTAL,
        &CHANGE_IP_FORMAT,
    ];
    ops.extend(radix::ops());
    ops
}

/// CyberChef's `DELIM_OPTIONS`.
const DELIMS: &[&str] = &["Space", "Comma", "Semi-colon", "Colon", "Line feed", "CRLF"];

/// CyberChef's `BIN_DELIM_OPTIONS`.
const BIN_DELIMS: &[&str] = &[
    "Space",
    "Comma",
    "Semi-colon",
    "Colon",
    "Line feed",
    "CRLF",
    "None",
];

const FROM_DECIMAL_DELIMS: &[&str] = &[
    "Space",
    "Comma",
    "Semi-colon",
    "Colon",
    "Line feed",
    "CRLF",
    "Auto",
];

const ENDIANNESS: &[&str] = &["Big Endian", "Little Endian"];
const FLOAT_SIZES: &[&str] = &["Float (4 bytes)", "Double (8 bytes)"];
const IP_FORMATS: &[&str] = &["Dotted Decimal", "Decimal", "Octal", "Hex"];

/// The text of a delimiter option, Space when unset.
fn delim(args: &Args, i: usize) -> &'static str {
    char_rep(match args.str(i) {
        "" => "Space",
        d => d,
    })
    .unwrap_or(" ")
}

/// A number argument checked the way CyberChef's `Ingredient.validate`
/// checks it before the operation runs, with its messages.
fn number_arg(
    args: &Args,
    i: usize,
    name: &str,
    min: Option<f64>,
    max: Option<f64>,
    integer: bool,
) -> Result<f64, OpError> {
    let n = args.num(i);
    if n.is_nan() {
        return Err(OpError::new(format!("{name} must be a number.")));
    }
    if integer && (!n.is_finite() || n.fract() != 0.0) {
        return Err(OpError::new(format!("{name} must be an integer.")));
    }
    if let Some(min) = min.filter(|m| n < *m) {
        return Err(OpError::new(format!(
            "{name} must be greater than or equal to {}.",
            js::number(min)
        )));
    }
    if let Some(max) = max.filter(|m| n > *m) {
        return Err(OpError::new(format!(
            "{name} must be less than or equal to {}.",
            js::number(max)
        )));
    }
    Ok(n)
}

/// An operation's `byteArray` result as bytes. Values outside 0-255 fail
/// CyberChef's dish check with its message; NaN (from `parseInt`) becomes
/// 0 the way it does when the array is stored.
fn byte_array(values: &[f64]) -> Result<Dish, OpError> {
    if values.iter().any(|v| *v < 0.0 || *v > 255.0) {
        return Err(OpError::new(format!(
            "Data is not a valid byteArray: {}",
            js::json_sample(values, 25)
        )));
    }
    Ok(Dish::Bytes(
        values
            .iter()
            .map(|v| if v.is_nan() { 0 } else { *v as u8 })
            .collect(),
    ))
}

/// A string of UTF-16 code units as bytes, CyberChef's `strToArrayBuffer`:
/// one byte per unit when they all fit, else UTF-8.
fn units_to_bytes(units: &[u16]) -> Vec<u8> {
    if units.iter().all(|u| *u < 256) {
        units.iter().map(|u| *u as u8).collect()
    } else {
        String::from_utf16_lossy(units).into_bytes()
    }
}

pub static TO_CHARCODE: Operation = Operation {
    name: "To Charcode",
    category: CATEGORY,
    description: "Converts text to its unicode character code equivalent.\n\ne.g. Γειά σου becomes 0393 03b5 03b9 03ac 20 03c3 03bf 03c5",
    info_url: Some("https://wikipedia.org/wiki/Plane_(Unicode)"),
    args: &[Arg::choice("Delimiter", DELIMS), Arg::number("Base", 16.0)],
    run: to_charcode,
};

fn to_charcode(input: Dish, args: &Args) -> Result<Dish, OpError> {
    let delim = delim(args, 0);
    let base = number_arg(args, 1, "Base", None, None, false)?;
    if !(2.0..=36.0).contains(&base) {
        return Err(OpError::new(
            "Error: Base argument must be between 2 and 36",
        ));
    }
    let codes: Vec<String> = input
        .text()
        .chars()
        .map(|c| {
            let ord = u32::from(c);
            if base == 16.0 {
                let width = match ord {
                    0..=0xff => 2,
                    0x100..=0xffff => 4,
                    _ => 6,
                };
                format!("{ord:0width$x}")
            } else {
                js::radix_string(f64::from(ord), base as u32)
            }
        })
        .collect();
    Ok(Dish::Text(codes.join(delim)))
}

pub static FROM_CHARCODE: Operation = Operation {
    name: "From Charcode",
    category: CATEGORY,
    description: "Converts unicode character codes back into text.\n\ne.g. 0393 03b5 03b9 03ac 20 03c3 03bf 03c5 becomes Γειά σου",
    info_url: Some("https://wikipedia.org/wiki/Plane_(Unicode)"),
    args: &[Arg::choice("Delimiter", DELIMS), Arg::number("Base", 16.0)],
    run: from_charcode,
};

fn from_charcode(input: Dish, args: &Args) -> Result<Dish, OpError> {
    let delim = delim(args, 0);
    let base = number_arg(args, 1, "Base", None, None, false)?;
    if !(2.0..=36.0).contains(&base) {
        return Err(OpError::new(
            "Error: Base argument must be between 2 and 36",
        ));
    }
    let input = input.text();
    if input.is_empty() {
        return Ok(Dish::Bytes(vec![]));
    }
    let mut bites: Vec<String> = input.split(delim).map(str::to_string).collect();
    // One long run of digits is read two characters at a time.
    let units: Vec<u16> = input.encode_utf16().collect();
    if bites.len() == 1 && units.len() > 17 {
        bites = units.chunks(2).map(String::from_utf16_lossy).collect();
    }
    let mut out: Vec<u16> = vec![];
    for b in &bites {
        let o = js::parse_int(b, base as u32);
        // `Utils.chr`: astral code points become surrogate pairs.
        if o > 65535.0 {
            let o = o - 65536.0;
            out.push(((js::to_uint32(o) >> 10) & 0x3ff | 0xd800) as u16);
            out.push((0xdc00 | (js::to_int32(o) & 0x3ff)) as u16);
        } else {
            out.push(js::to_uint32(o) as u16);
        }
    }
    Ok(Dish::Bytes(units_to_bytes(&out)))
}

pub static TO_DECIMAL: Operation = Operation {
    name: "To Decimal",
    category: CATEGORY,
    description: "Converts the input data to an ordinal integer array.\n\ne.g. Hello becomes 72 101 108 108 111",
    info_url: None,
    args: &[
        Arg::choice("Delimiter", DELIMS),
        Arg::flag("Support signed values", false),
    ],
    run: |input, args| {
        let delim = delim(args, 0);
        let signed = args.flag(1);
        let out: Vec<String> = input
            .bytes()
            .iter()
            .map(|b| {
                if signed {
                    (*b as i8).to_string()
                } else {
                    b.to_string()
                }
            })
            .collect();
        Ok(Dish::Text(out.join(delim)))
    },
};

pub static FROM_DECIMAL: Operation = Operation {
    name: "From Decimal",
    category: CATEGORY,
    description: "Converts the data from an ordinal integer array back into its raw form.\n\ne.g. 72 101 108 108 111 becomes Hello",
    info_url: None,
    args: &[
        Arg::choice("Delimiter", FROM_DECIMAL_DELIMS),
        Arg::flag("Support signed values", false),
    ],
    run: |input, args| {
        let input = input.text();
        let pieces: Vec<&str> = match args.str(0) {
            "Auto" => input
                .split(|c: char| !(c.is_ascii_digit() || c == '-'))
                .collect(),
            "" => split_delim(&input, "Space"),
            d => split_delim(&input, d),
        };
        let signed = args.flag(1);
        let values: Vec<f64> = pieces
            .into_iter()
            .filter(|p| !p.is_empty())
            .map(|p| js::parse_int(p, 10))
            .map(|v| if signed && v < 0.0 { 256.0 + v } else { v })
            .collect();
        byte_array(&values)
    },
};

pub static TO_FLOAT: Operation = Operation {
    name: "To Float",
    category: CATEGORY,
    description: "Convert to IEEE754 Floating Point Numbers",
    info_url: Some("https://wikipedia.org/wiki/IEEE_754"),
    args: &[
        Arg::choice("Endianness", ENDIANNESS),
        Arg::choice("Size", FLOAT_SIZES),
        Arg::choice("Delimiter", DELIMS),
    ],
    run: to_float,
};

fn to_float(input: Dish, args: &Args) -> Result<Dish, OpError> {
    let little = args.str(0) == "Little Endian";
    let size = if args.str(1) == "Double (8 bytes)" {
        8
    } else {
        4
    };
    let delim = delim(args, 2);
    let data = input.bytes();
    if !data.len().is_multiple_of(size) {
        return Err(OpError::new(format!("Input is not a multiple of {size}")));
    }
    let out: Vec<String> = data
        .chunks(size)
        .map(|chunk| {
            let mut be = chunk.to_vec();
            if little {
                be.reverse();
            }
            let v = match size {
                8 => f64::from_be_bytes(be.try_into().unwrap_or_default()),
                _ => f64::from(f32::from_be_bytes(be.try_into().unwrap_or_default())),
            };
            js::number(v)
        })
        .collect();
    Ok(Dish::Text(out.join(delim)))
}

pub static FROM_FLOAT: Operation = Operation {
    name: "From Float",
    category: CATEGORY,
    description: "Convert from IEEE754 Floating Point Numbers",
    info_url: Some("https://wikipedia.org/wiki/IEEE_754"),
    args: &[
        Arg::choice("Endianness", ENDIANNESS),
        Arg::choice("Size", FLOAT_SIZES),
        Arg::choice("Delimiter", DELIMS),
    ],
    run: from_float,
};

fn from_float(input: Dish, args: &Args) -> Result<Dish, OpError> {
    let input = input.text();
    if input.is_empty() {
        return Ok(Dish::Bytes(vec![]));
    }
    let little = args.str(0) == "Little Endian";
    let size = if args.str(1) == "Double (8 bytes)" {
        8
    } else {
        4
    };
    let delim = delim(args, 2);
    let mut out = vec![];
    for f in input.split(delim) {
        let mut buf = [0u8; 8];
        ieee754_write(&mut buf[..size], js::parse_float(f), little);
        out.extend_from_slice(&buf[..size]);
    }
    Ok(Dish::Bytes(out))
}

/// `value` in IEEE 754 binary32 or binary64 (`buf`'s length), big- or
/// little-endian. Narrowing to binary32 rounds to nearest, ties to even
/// (IEEE 754-2019 §4.3.1, the default rounding-direction attribute), which
/// Rust's `as f32` does; NaN is the default quiet NaN (§6.2.1: the first
/// bit of the trailing significand set).
fn ieee754_write(buf: &mut [u8], value: f64, little: bool) {
    let value = if value.is_nan() { f64::NAN } else { value };
    let mut bytes = if buf.len() == 4 {
        (value as f32).to_be_bytes().to_vec()
    } else {
        value.to_be_bytes().to_vec()
    };
    if little {
        bytes.reverse();
    }
    buf.copy_from_slice(&bytes);
}

pub static TO_BINARY: Operation = Operation {
    name: "To Binary",
    category: CATEGORY,
    description: "Displays the input data as a binary string.\n\ne.g. Hi becomes 01001000 01101001",
    info_url: Some("https://wikipedia.org/wiki/Binary_code"),
    args: &[
        Arg::choice("Delimiter", BIN_DELIMS),
        Arg::number_in("Byte Length", 8.0, 1.0, 256.0),
    ],
    run: |input, args| {
        let delim = delim(args, 0);
        let width = number_arg(args, 1, "Byte Length", Some(1.0), Some(256.0), true)? as usize;
        let out: Vec<String> = input
            .bytes()
            .iter()
            .map(|b| format!("{b:0width$b}"))
            .collect();
        Ok(Dish::Text(out.join(delim)))
    },
};

pub static FROM_BINARY: Operation = Operation {
    name: "From Binary",
    category: CATEGORY,
    description: "Converts a binary string back into its raw form.\n\ne.g. 01001000 01101001 becomes Hi",
    info_url: Some("https://wikipedia.org/wiki/Binary_code"),
    args: &[
        Arg::choice("Delimiter", BIN_DELIMS),
        Arg {
            name: "Byte Length",
            kind: ArgKind::Number {
                default: 8.0,
                min: Some(1.0),
                max: None,
                step: 1.0,
            },
        },
    ],
    run: |input, args| {
        let len = number_arg(args, 1, "Byte Length", Some(1.0), None, false)?;
        if len.is_finite() && len.fract() != 0.0 {
            return Err(OpError::new("Byte length must be a positive integer"));
        }
        let token = match args.str(0) {
            "" => "Space",
            d => d,
        };
        let bits: Vec<u16> = strip_delim(&input.text(), token).encode_utf16().collect();
        let len = (len as usize).max(1);
        let values: Vec<f64> = bits
            .chunks(len)
            .map(|c| js::parse_int(&String::from_utf16_lossy(c), 2))
            .collect();
        byte_array(&values)
    },
};

pub static TO_OCTAL: Operation = Operation {
    name: "To Octal",
    category: CATEGORY,
    description: "Converts the input string to octal bytes separated by the specified delimiter.\n\ne.g. The UTF-8 encoded string Γειά σου becomes 316 223 316 265 316 271 316 254 40 317 203 316 277 317 205",
    info_url: Some("https://wikipedia.org/wiki/Octal"),
    args: &[Arg::choice("Delimiter", DELIMS)],
    run: |input, args| {
        let out: Vec<String> = input.bytes().iter().map(|b| format!("{b:o}")).collect();
        Ok(Dish::Text(out.join(delim(args, 0))))
    },
};

pub static FROM_OCTAL: Operation = Operation {
    name: "From Octal",
    category: CATEGORY,
    description: "Converts an octal byte string back into its raw value.\n\ne.g. 316 223 316 265 316 271 316 254 40 317 203 316 277 317 205 becomes the UTF-8 encoded string Γειά σου",
    info_url: Some("https://wikipedia.org/wiki/Octal"),
    args: &[Arg::choice("Delimiter", DELIMS)],
    run: |input, args| {
        let input = input.text();
        if input.is_empty() {
            return Ok(Dish::Bytes(vec![]));
        }
        let values: Vec<f64> = input
            .split(delim(args, 0))
            .map(|v| js::parse_int(v, 8))
            .collect();
        byte_array(&values)
    },
};

pub static CHANGE_IP_FORMAT: Operation = Operation {
    name: "Change IP format",
    category: CATEGORY,
    description: "Convert an IP address from one format to another, e.g. 172.20.23.54 to ac141736",
    info_url: None,
    args: &[
        Arg::choice("Input format", IP_FORMATS),
        Arg::choice("Output format", IP_FORMATS),
    ],
    run: change_ip_format,
};

/// An address as four octets, the way `ChangeIPFormat.fromNumber` splits a
/// number (JS's 32-bit shifts).
fn ip_from_number(value: &str, radix: u32) -> Vec<f64> {
    let n = js::to_int32(js::parse_int(value, radix));
    [24, 16, 8, 0]
        .iter()
        .map(|s| f64::from((n >> s) & 255))
        .collect()
}

fn change_ip_format(input: Dish, args: &Args) -> Result<Dish, OpError> {
    let format = |i| match args.str(i) {
        "" => "Dotted Decimal",
        f => f,
    };
    let (in_format, out_format) = (format(0), format(1));
    let mut out = String::new();
    for line in input.text().split('\n').filter(|l| !l.is_empty()) {
        if in_format == out_format {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        // Octets as JS numbers: Dotted Decimal keeps whatever `parseInt` gives.
        let ip: Vec<f64> = match in_format {
            "Dotted Decimal" => line.split('.').map(|o| js::parse_int(o, 10)).collect(),
            "Decimal" => ip_from_number(line, 10),
            "Octal" => ip_from_number(line, 8),
            "Hex" => from_hex(line, "Auto", 2)?
                .into_iter()
                .map(f64::from)
                .collect(),
            _ => return Err(OpError::new("Unsupported input IP format")),
        };
        let as_u32 = || {
            let octet = |i: usize| js::to_int32(ip.get(i).copied().unwrap_or(0.0));
            ((octet(0) << 24) | (octet(1) << 16) | (octet(2) << 8) | octet(3)) as u32
        };
        match out_format {
            "Dotted Decimal" => {
                let parts: Vec<String> = ip.iter().map(|o| js::number(*o)).collect();
                out.push_str(&parts.join("."));
            }
            "Decimal" => out.push_str(&as_u32().to_string()),
            "Octal" => out.push_str(&format!("0{:o}", as_u32())),
            "Hex" => {
                for o in &ip {
                    out.push_str(&format!("{:0>2}", js::radix_string(*o, 16)));
                }
            }
            _ => return Err(OpError::new("Unsupported output IP format")),
        }
        out.push('\n');
    }
    out.pop();
    Ok(Dish::Text(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ArgValue;

    fn s(v: &str) -> ArgValue {
        ArgValue::Str(v.into())
    }

    fn run(op: &Operation, input: &str, args: &[ArgValue]) -> String {
        match op.apply(Dish::Text(input.into()), args) {
            Ok(d) => d.into_text(),
            Err(e) => e.0,
        }
    }

    fn run_bytes(op: &Operation, input: &str, args: &[ArgValue]) -> Vec<u8> {
        op.apply(Dish::Text(input.into()), args)
            .unwrap()
            .into_bytes()
    }

    #[test]
    fn charcodes() {
        let n = ArgValue::Num;
        assert_eq!(
            run(&TO_CHARCODE, "Γειά σου😀ÿ", &[s("Space"), n(16.)]),
            "0393 03b5 03b9 03ac 20 03c3 03bf 03c5 01f600 ff"
        );
        assert_eq!(
            run(&TO_CHARCODE, "abc😀", &[s("Comma"), n(2.5)]),
            "1100001,1100010,1100011,11111011000000000"
        );
        assert_eq!(
            run(&TO_CHARCODE, "abc", &[s("Line feed"), n(36.)]),
            "2p\n2q\n2r"
        );
        assert_eq!(
            run(&TO_CHARCODE, "abc", &[s("Comma"), n(37.)]),
            "Error: Base argument must be between 2 and 36"
        );
        assert_eq!(run(&TO_CHARCODE, "", &[]), "");
        assert_eq!(
            run(&FROM_CHARCODE, "0393 03b5 1f600 41", &[s("Space"), n(16.)]),
            "Γε😀A"
        );
        assert_eq!(run(&FROM_CHARCODE, "41 zz 42", &[]), "A\0B");
        assert_eq!(run(&FROM_CHARCODE, "414243444546474849", &[]), "ABCDEFGHI");
        assert_eq!(run(&FROM_CHARCODE, "41424344454647484", &[]), "\u{48000}");
        assert_eq!(run(&FROM_CHARCODE, "65 66", &[s("Comma"), n(10.)]), "A");
        assert_eq!(run(&FROM_CHARCODE, "-41", &[]), "\u{ffbf}");
        assert_eq!(run(&FROM_CHARCODE, "ff e9", &[]), "ÿé");
        assert_eq!(
            run(&FROM_CHARCODE, "", &[s("Space"), n(40.)]),
            "Error: Base argument must be between 2 and 36"
        );
        assert_eq!(run_bytes(&FROM_CHARCODE, "d800", &[]), [0xef, 0xbf, 0xbd]);
        assert_eq!(
            run_bytes(&FROM_CHARCODE, "100000041", &[]),
            [0xf4, 0x80, 0x81, 0x81]
        );
        assert_eq!(run_bytes(&FROM_CHARCODE, "ff", &[]), [0xff]);
    }

    #[test]
    fn decimal() {
        let t = ArgValue::Bool(true);
        assert_eq!(
            run(&TO_DECIMAL, "\u{ff}\u{80}A", &[s("Space"), t.clone()]),
            "-1 -128 65"
        );
        assert_eq!(
            run(&TO_DECIMAL, "é你", &[s("Comma")]),
            "195,169,228,189,160"
        );
        assert_eq!(
            run(&FROM_DECIMAL, "72 -1 300 x 1.5", &[]),
            "Data is not a valid byteArray: [72,-1,300,null,1]"
        );
        assert_eq!(
            run_bytes(&FROM_DECIMAL, "72 -1 -128 -129", &[s("Space"), t.clone()]),
            [72, 255, 128, 127]
        );
        assert_eq!(
            run_bytes(&FROM_DECIMAL, "a72b-3c", &[s("Auto"), t]),
            [72, 253]
        );
        assert_eq!(run(&FROM_DECIMAL, "72,,105", &[s("Comma")]), "Hi");
        assert_eq!(run_bytes(&FROM_DECIMAL, "72 x 105", &[]), [72, 0, 105]);
        assert_eq!(
            run(&FROM_DECIMAL, "1 2 3 4 5 6 7 8 9 10 11 12 13 14 300", &[]),
            "Data is not a valid byteArray: [1,2,3,4,5,6,7,8,9,10,..."
        );
    }

    #[test]
    fn binary_and_octal() {
        let n = ArgValue::Num;
        assert_eq!(
            run(&TO_BINARY, "Hi", &[s("None"), n(8.)]),
            "0100100001101001"
        );
        assert_eq!(
            run(&TO_BINARY, "Hi", &[s("Comma"), n(10.)]),
            "0001001000,0001101001"
        );
        assert_eq!(
            run(&TO_BINARY, "Hi", &[s("Comma"), n(4.)]),
            "1001000,1101001"
        );
        assert_eq!(
            run(&TO_BINARY, "Hi", &[s("Comma"), n(0.)]),
            "Byte Length must be greater than or equal to 1."
        );
        assert_eq!(
            run(&TO_BINARY, "Hi", &[s("Comma"), n(257.)]),
            "Byte Length must be less than or equal to 256."
        );
        assert_eq!(
            run(&TO_BINARY, "Hi", &[s("Comma"), n(7.5)]),
            "Byte Length must be an integer."
        );
        assert_eq!(
            run(&FROM_BINARY, "0100100001101001", &[s("None"), n(4.)]),
            "\u{4}\u{8}\u{6}\t"
        );
        assert_eq!(
            run(&FROM_BINARY, "01001000 01101001", &[s("Comma"), n(8.)]),
            "H4\u{1}"
        );
        assert_eq!(run(&FROM_BINARY, "0100100x", &[s("None"), n(8.)]), "$");
        assert_eq!(
            run(&FROM_BINARY, "0100", &[s("None"), n(1.5)]),
            "Byte length must be a positive integer"
        );
        assert_eq!(
            run(&FROM_BINARY, "111111111", &[s("None"), n(9.)]),
            "Data is not a valid byteArray: [511]"
        );
        assert_eq!(run(&TO_OCTAL, "Γ", &[]), "316 223");
        assert_eq!(
            run(&FROM_OCTAL, "316 223 777 8 x", &[]),
            "Data is not a valid byteArray: [206,147,511,null,null]"
        );
        assert_eq!(run_bytes(&FROM_OCTAL, "101  102", &[]), [65, 0, 66]);
        assert_eq!(run(&FROM_OCTAL, "", &[]), "");
    }

    #[test]
    fn floats() {
        let args = |e: &str, size: &str, d: &str| vec![s(e), s(size), s(d)];
        let f4 = args("Big Endian", "Float (4 bytes)", "Space");
        let hex = |b: Vec<u8>| b.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let write = |input: &str, a: &[ArgValue]| hex(run_bytes(&FROM_FLOAT, input, a));
        assert_eq!(
            write("0.1 1e30 -0 NaN", &f4),
            "3dcccccd7149f2ca800000007fc00000"
        );
        assert_eq!(
            write("abc 1.5x 3.4028236e38 7e-46", &f4),
            "7fc000003fc000007f80000000000000"
        );
        // 2^24 + 1 and 2^24 + 3 are ties: to even, 2^24 and 2^24 + 4.
        assert_eq!(
            write("1.17549435e-38 16777217 16777219 1e-45", &f4),
            "008000004b8000004b80000200000001"
        );
        assert_eq!(
            write("1.5 2.5", &args("Big Endian", "Float (4 bytes)", "Comma")),
            "3fc00000"
        );
        let d8 = args("Little Endian", "Double (8 bytes)", "Space");
        assert_eq!(
            write("0.1 -0 NaN", &d8),
            "9a9999999999b93f0000000000000080000000000000f87f"
        );
        assert_eq!(
            write("5e-324 -2.225073858507201e-308", &d8),
            "0100000000000000ffffffffffff0f80"
        );
        assert_eq!(write("", &f4), "");

        let read = |bytes: &[u8], a: &[ArgValue]| {
            TO_FLOAT
                .apply(Dish::Bytes(bytes.to_vec()), a)
                .map_or_else(|e| e.0, Dish::into_text)
        };
        assert_eq!(
            read(&[0x3d, 0xcc, 0xcc, 0xcd, 0, 0x80, 0, 0], &f4),
            "0.10000000149011612 1.1754943508222875e-38"
        );
        assert_eq!(
            read(
                &[
                    0x7f, 0x80, 0, 0, 0xff, 0xc0, 0, 1, 0, 0, 0, 1, 0x80, 0, 0, 0
                ],
                &f4
            ),
            "Infinity NaN 1.401298464324817e-45 0"
        );
        assert_eq!(
            read(
                b"abcd\xff\xff\xff\xff",
                &args("Big Endian", "Float (4 bytes)", "Line feed")
            ),
            "261007875622861540000\nNaN"
        );
        assert_eq!(read(b"abc", &f4), "Input is not a multiple of 4");
        assert_eq!(read(&[0, 0, 0, 0, 0, 0, 0xe0, 0x3f], &d8), "0.5");
    }

    #[test]
    fn ip_formats() {
        let f = |a: &str, b: &str| vec![s(a), s(b)];
        let dd = "Dotted Decimal";
        assert_eq!(
            run(
                &CHANGE_IP_FORMAT,
                "192.168.1.1\n\n10.0.0.1\n",
                &f(dd, "Decimal")
            ),
            "3232235777\n167772161"
        );
        assert_eq!(
            run(&CHANGE_IP_FORMAT, "192.168.1.1\n\n", &f("Hex", "Hex")),
            "192.168.1.1"
        );
        assert_eq!(
            run(&CHANGE_IP_FORMAT, "1.2.3", &f(dd, "Decimal")),
            "16909056"
        );
        assert_eq!(
            run(&CHANGE_IP_FORMAT, "300.1.x.4", &f(dd, "Hex")),
            "12c01NaN04"
        );
        assert_eq!(
            run(&CHANGE_IP_FORMAT, "300.-1.x.4", &f(dd, "Decimal")),
            "4294901764"
        );
        assert_eq!(
            run(&CHANGE_IP_FORMAT, "4294967296", &f("Decimal", dd)),
            "0.0.0.0"
        );
        assert_eq!(
            run(&CHANGE_IP_FORMAT, "-1", &f("Decimal", "Hex")),
            "ffffffff"
        );
        assert_eq!(
            run(&CHANGE_IP_FORMAT, "99999999999", &f("Decimal", "Hex")),
            "4876e7ff"
        );
        assert_eq!(
            run(&CHANGE_IP_FORMAT, "c0a8", &f("Hex", "Octal")),
            "030052000000"
        );
        assert_eq!(run(&CHANGE_IP_FORMAT, "zz", &f("Hex", "Decimal")), "0");
        assert_eq!(run(&CHANGE_IP_FORMAT, "0.0.0.0", &f(dd, "Octal")), "00");
        assert_eq!(
            run(&CHANGE_IP_FORMAT, "1.2.3.4\r\n5.6.7.8", &f(dd, "Hex")),
            "01020304\n05060708"
        );
        assert_eq!(run(&CHANGE_IP_FORMAT, "", &f(dd, "Hex")), "");
    }
}
