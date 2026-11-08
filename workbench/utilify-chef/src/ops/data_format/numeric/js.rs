//! JavaScript number semantics the numeric operations depend on: how JS
//! prints numbers, `parseInt`/`parseFloat`, the `ToInt32` family and
//! `String.prototype.trim`'s idea of whitespace.

use num_bigint::BigUint;
use num_traits::FromPrimitive;

/// `Number.prototype.toString()`.
pub fn number(n: f64) -> String {
    crate::format_number(n)
}

/// `Number.prototype.toString(radix)` for a whole number, which is all the
/// operations here print in other bases.
pub fn radix_string(n: f64, radix: u32) -> String {
    if !n.is_finite() {
        return number(n);
    }
    let sign = if n < 0.0 { "-" } else { "" };
    // Exact for every whole double, however large.
    let digits = BigUint::from_f64(n.abs().trunc())
        .unwrap_or_default()
        .to_str_radix(radix);
    format!("{sign}{digits}")
}

/// The whitespace `String.prototype.trim` and `\s` know about.
pub fn is_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

pub fn trim(s: &str) -> &str {
    s.trim_matches(is_space)
}

/// JavaScript's `parseInt(s, radix)` for a radix of 2 to 36: optional sign,
/// `0x` for radix 16, then the longest run of digits; NaN when there are none.
pub fn parse_int(s: &str, radix: u32) -> f64 {
    let s = s.trim_start_matches(is_space);
    let (neg, s) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let s = if radix == 16 {
        s.strip_prefix("0x")
            .or_else(|| s.strip_prefix("0X"))
            .unwrap_or(s)
    } else {
        s
    };
    let mut exact: Option<u128> = Some(0);
    let mut approx = 0f64;
    let mut any = false;
    for d in s.chars().map_while(|c| c.to_digit(radix)) {
        any = true;
        exact = exact
            .and_then(|v| v.checked_mul(u128::from(radix)))
            .and_then(|v| v.checked_add(u128::from(d)));
        approx = approx * f64::from(radix) + f64::from(d);
    }
    if !any {
        return f64::NAN;
    }
    // Rounded once from the exact value where it fits, like V8.
    let v = exact.map_or(approx, |v| v as f64);
    if neg { -v } else { v }
}

/// JavaScript's `parseFloat`: the longest prefix that is a decimal literal
/// or `Infinity`, NaN when there is none.
pub fn parse_float(s: &str) -> f64 {
    let s = s.trim_start_matches(is_space);
    let b = s.as_bytes();
    let mut i = 0;
    let neg = match b.first() {
        Some(b'-') => {
            i = 1;
            true
        }
        Some(b'+') => {
            i = 1;
            false
        }
        _ => false,
    };
    let signed = |v: f64| if neg { -v } else { v };
    if s[i..].starts_with("Infinity") {
        return signed(f64::INFINITY);
    }
    let digits = |from: usize| b[from..].iter().take_while(|c| c.is_ascii_digit()).count();
    let int = digits(i);
    let mut end = i + int;
    let mut frac = 0;
    if b.get(end) == Some(&b'.') {
        frac = digits(end + 1);
        if int > 0 || frac > 0 {
            end += 1 + frac;
        }
    }
    if int == 0 && frac == 0 {
        return f64::NAN;
    }
    if matches!(b.get(end), Some(b'e' | b'E')) {
        let sign = usize::from(matches!(b.get(end + 1), Some(b'+' | b'-')));
        let exp = digits(end + 1 + sign);
        if exp > 0 {
            end += 1 + sign + exp;
        }
    }
    s[i..end].parse::<f64>().map_or(f64::NAN, signed)
}

/// `ToInt32`: what `|`, `&`, `<<` and `>>` do to their operands.
pub fn to_int32(n: f64) -> i32 {
    to_uint32(n) as i32
}

pub fn to_uint32(n: f64) -> u32 {
    if !n.is_finite() {
        return 0;
    }
    n.trunc().rem_euclid(4_294_967_296.0) as u32
}

/// A JSON array of numbers as `JSON.stringify` writes it (NaN is `null`),
/// cut to `max` characters with `...` the way CyberChef's
/// `Utils.truncate` samples data in errors.
pub fn json_sample(values: &[f64], max: usize) -> String {
    let mut s = String::from("[");
    for (i, v) in values.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        if v.is_finite() {
            s.push_str(&number(*v));
        } else {
            s.push_str("null");
        }
        // Enough to know it will be cut.
        if s.len() > max {
            break;
        }
    }
    s.push(']');
    if s.len() > max {
        s.truncate(max - 3);
        s.push_str("...");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prints_like_js() {
        assert_eq!(number(0.5), "0.5");
        assert_eq!(number(-0.0), "0");
        assert_eq!(number(261007875622861540000.0), "261007875622861540000");
        assert_eq!(number(1e21), "1e+21");
        assert_eq!(number(1.1754943508222875e-38), "1.1754943508222875e-38");
        assert_eq!(number(7.5615378426174155e81), "7.5615378426174155e+81");
        assert_eq!(number(0.10000000149011612), "0.10000000149011612");
        assert_eq!(number(1e-7), "1e-7");
        assert_eq!(number(0.000001), "0.000001");
        assert_eq!(number(5e-324), "5e-324");
        assert_eq!(
            number(f64::from(f32::from_bits(0xc459_c549))),
            "-871.0825805664062"
        );
        assert_eq!(radix_string(300.0, 16), "12c");
        assert_eq!(radix_string(-1.0, 16), "-1");
        assert_eq!(radix_string(1e20, 16), "56bc75e2d63100000");
    }

    #[test]
    fn parses_like_js() {
        assert_eq!(parse_int(" -0x1fz", 16), -31.0);
        assert!(parse_int("z", 10).is_nan());
        assert_eq!(parse_int("0x10", 2), 0.0);
        assert_eq!(parse_float("1.5x"), 1.5);
        assert_eq!(parse_float("-.5e1"), -5.0);
        assert_eq!(parse_float("5.e"), 5.0);
        assert_eq!(parse_float("Infinityx"), f64::INFINITY);
        assert!(parse_float(".").is_nan());
        assert_eq!(to_int32(4294967295.0), -1);
        assert_eq!(json_sample(&[72.0, f64::NAN], 25), "[72,null]");
        let long: Vec<f64> = (1..15).map(f64::from).collect();
        assert_eq!(json_sample(&long, 25), "[1,2,3,4,5,6,7,8,9,10,...");
    }
}
