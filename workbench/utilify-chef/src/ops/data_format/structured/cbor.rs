//! CBOR (RFC 8949) to and from JSON, with `utilify-cbor` doing the wire
//! format and checking validity (§5.3).
//!
//! Encoding (`EncodeOptions::DETERMINISTIC`, §4.2.1 core deterministic
//! encoding, NaN as the half `f97e00` of §4.2.2): JSON numbers that are
//! integers within ±(2^53 - 1) become integers; every other number (`-0`,
//! fractions, larger magnitudes) the shortest float that holds it exactly
//! (preferred serialization, §4.1). Map keys are sorted bytewise.
//!
//! Decoding is strict (`DecodeOptions::STRICT`): input must be well-formed
//! and valid (§5.3: UTF-8 text, no duplicate map keys, and valid content for
//! the tags of RFC 8949 §3.4 and RFC 8746). Arrays, maps and tags nest at
//! most [`MAX_DEPTH`] deep, the depth `JSON.parse` accepts on input, so
//! anything CBOR Encode writes reads back. Errors are the library's,
//! reported before those of the tag interpretations below (which follow
//! input order among themselves), after "CBOR Decode - ".
//!
//! Tags are interpreted per their specifications (IANA CBOR tag registry):
//! - 0 (§3.4.1): an RFC 3339 `date-time` (as refined by RFC 4287 §3.3),
//!   checked by the library and output as the string itself;
//! - 1 (§3.4.2): seconds since the epoch, integer or float, as RFC 3339 text
//!   in UTC with the exact fraction (years outside 0000–9999 expanded as
//!   `±YYYYYY`); non-finite or out-of-range content is an error;
//! - 100 and 1004 (RFC 8943): days since 1970-01-01 (an integer), and an
//!   RFC 3339 `full-date` string, both output as `YYYY-MM-DD`;
//! - 2 and 3 (§3.4.3): bignums, as integers (below);
//! - 32 (§3.4.5.3): a URI-reference (RFC 3986, checked by the library),
//!   output as `url`'s WHATWG serialization when it parses as an absolute
//!   URL and as is otherwise (a relative reference);
//! - 33 and 34 (§3.4.5.3): base64url without padding and padded base64
//!   (RFC 4648 §5, §4), checked by the library and decoded;
//! - 35 (§3.4.5.3): a regular expression whose syntax (PCRE or ECMA-262) is
//!   agreed out of band, so any text string;
//! - 21065 (RFC 9485): an I-Regexp, checked against its ABNF;
//! - 21066 (github.com/hildjj/cbor-specs `regexp.md`): `[pattern, flags?]`,
//!   both text strings (flags may be absent or `undefined`), checked as
//!   `new RegExp(pattern, flags)` would (`regress`'s ECMAScript parser);
//! - all three regular expressions output as `{tag, contents}` (`{"tag":
//!   21066, "contents": ["a+", "g"]}`, an `undefined` flags as `null`): JSON
//!   has no regular expression, and the tag number says which syntax the
//!   pattern is in, which the bare string would lose;
//! - 64–87 but 76 (RFC 8746): typed arrays, by element; binary16 elements
//!   included, binary128 ones (83, 87) left as `{tag, contents}`;
//! - 258 (a set): an array of its elements; 262 (embedded JSON): a byte string holding JSON
//!   text, parsed; 273: a byte string of WTF-8 that is not valid UTF-8;
//!   280 (a Lisp symbol): a name string, or `[name]` or
//!   `[package or null, name]`; 55799 (§3.4.6): its content;
//! - any other tag: `{tag, contents}`.
//!
//! Integers (major types 0 and 1, and bignums) within ±2^53 are JSON
//! numbers; beyond, where an ECMAScript number can't hold every integer, a
//! string of their exact decimal digits (`"18446744073709551615"`). The same
//! holds for 64-bit typed array elements and tag numbers.
//!
//! Maps whose keys are all text strings are JSON objects. Any other map is
//! an array of `[key, value]` pairs in input order (`{1: "a"}` →
//! `[[1, "a"]]`), each key and value converted as any other item. RFC 8949
//! §6.1 makes a map a JSON object only "if all keys are UTF-8 strings"; it
//! allows converting other keys to strings but warns that "doing so
//! introduces a danger of key collision" (`1` and `"1"`, `h'01'` and
//! `[1]`), so keys are kept as values instead. (cbor2 makes these a
//! JavaScript `Map`, which `JSON.stringify` prints as `{}`, losing them.)
//!
//! Presentation kept from CyberChef (cbor2's JavaScript values, as
//! `JSON.stringify` prints them): byte strings and typed arrays as objects
//! of their elements by index (`{"0": 1, "1": 2}`), simple values as
//! `{"value": n}`, unknown tags as
//! `{"tag": n, "contents": ...}`, `undefined` and symbols dropped from
//! objects, `null` in arrays (and a top-level `undefined` failing as
//! CyberChef does), and non-finite floats as `null`.
//!
//! Remaining limits: an RFC 9485 I-Regexp is checked against the ABNF only
//! (a range like `[z-a]` passes); `regress` accepts a quantified `\b` or
//! `\B` (`\b*`), which ECMA-262 rejects, and a 21066 pattern past its limits
//! (more than 255 nested groups or classes, more than 65,535 quantifiers)
//! or with more than 100,000 `|` is not checked; lone surrogates of WTF-8
//! (tag 273) and of embedded JSON become U+FFFD, which the output strings
//! can't hold otherwise.

use super::CATEGORY;
use super::js::with_big_stack;
use super::js::{self, Js, Obj, json_dish, json_input};
use crate::{Dish, OpError, Operation};
use base64::Engine;
use num_bigint::BigUint;
use utilify_cbor::{DecodeOptions, EncodeOptions, Value};
use utilify_msgpack::Timestamp;

pub(super) fn ops() -> Vec<&'static Operation> {
    vec![&CBOR_ENCODE, &CBOR_DECODE]
}

const DESCRIPTION: &str = "Concise Binary Object Representation (CBOR) is a binary data serialization format loosely based on JSON. Like JSON it allows the transmission of data objects that contain name–value pairs, but in a more concise manner. This increases processing and transfer speeds at the cost of human readability. It is defined in IETF RFC 8949.";

pub static CBOR_ENCODE: Operation = Operation {
    name: "CBOR Encode",
    category: CATEGORY,
    description: DESCRIPTION,
    info_url: Some("https://wikipedia.org/wiki/CBOR"),
    args: &[],
    run: |input, _| {
        with_big_stack(|| {
            let value = to_value(&json_input(&input)?);
            let opts = EncodeOptions::DETERMINISTIC.canonical_nan(true);
            Ok(Dish::Bytes(opts.encode(&value)))
        })
    },
};

pub static CBOR_DECODE: Operation = Operation {
    name: "CBOR Decode",
    category: CATEGORY,
    description: DESCRIPTION,
    info_url: Some("https://wikipedia.org/wiki/CBOR"),
    args: &[],
    run: |input, _| {
        with_big_stack(|| {
            let value =
                decode(&input.bytes()).map_err(|e| OpError::new(format!("CBOR Decode - {e}")))?;
            match value {
                // CyberChef fails presenting an undefined result with this.
                Js::Undefined => Err(OpError::new("\"undefined\" is not valid JSON")),
                v => json_dish(&v),
            }
        })
    },
};

// ---------------------------------------------------------------- encoding

const MAX_SAFE: u64 = 9_007_199_254_740_991;

/// Safe integers (but not -0) as integers, everything else as a float.
fn number(n: f64) -> Value {
    if n == 0.0 && n.is_sign_negative() {
        Value::Float(n)
    } else if n.fract() == 0.0 && n.abs() <= MAX_SAFE as f64 {
        if n < 0.0 {
            Value::Negative((-n - 1.0) as u64)
        } else {
            Value::Unsigned(n as u64)
        }
    } else {
        Value::Float(n)
    }
}

fn to_value(v: &Js) -> Value {
    match v {
        Js::Undefined => Value::Undefined,
        Js::Null => Value::Null,
        Js::Bool(b) => Value::Bool(*b),
        Js::Num(n) => number(*n),
        Js::Str(s) => Value::Text(s.clone()),
        Js::Arr(items) => Value::from(items.iter().map(to_value).collect::<Vec<_>>()),
        Js::Obj(o) => Value::Map(
            o.iter()
                .map(|(k, v)| (Value::Text(k.into_owned()), to_value(v)))
                .collect::<Vec<_>>()
                .into(),
        ),
    }
}

// ---------------------------------------------------------------- decoding

/// How deeply arrays, maps and tags may nest: the depth `JSON.parse`
/// accepts.
pub const MAX_DEPTH: usize = 5001;

/// Integers within ±2^53 are JSON numbers.
const MAX_EXACT: u64 = 1 << 53;

fn decode(buf: &[u8]) -> Result<Js, String> {
    let opts = DecodeOptions::STRICT
        .max_depth(MAX_DEPTH)
        .max_item_size(usize::MAX);
    let value = opts.decode(buf).map_err(|e| e.to_string())?;
    to_js(value)
}

/// "invalid content for tag N: reason", as the library words its own.
fn invalid(tag: u64, reason: impl std::fmt::Display) -> String {
    format!("invalid content for tag {tag}: {reason}")
}

/// An integer `-1 - n` (when `negative`) or `n`, as a JSON number within
/// ±2^53 and as its decimal digits beyond.
fn integer(negative: bool, n: u128) -> Js {
    let exact = u128::from(MAX_EXACT);
    if !negative && n <= exact {
        Js::Num(n as f64)
    } else if negative && n < exact {
        Js::Num(-1.0 - n as f64)
    } else if negative {
        Js::Str(format!("-{}", n + 1))
    } else {
        Js::Str(n.to_string())
    }
}

fn bignum(negative: bool, bytes: &[u8]) -> Js {
    let n = BigUint::from_bytes_be(bytes);
    match u128::try_from(&n) {
        Ok(n) if n < u128::MAX => integer(negative, n),
        _ if negative => Js::Str(format!("-{}", n + 1u32)),
        _ => Js::Str(n.to_string()),
    }
}

fn text(v: &Value) -> Option<String> {
    match v {
        Value::Text(s) => Some(s.clone()),
        Value::IndefiniteText(chunks) => Some(chunks.concat()),
        _ => None,
    }
}

fn bytes(v: &Value) -> Option<Vec<u8>> {
    match v {
        Value::Bytes(b) => Some(b.clone()),
        Value::IndefiniteBytes(chunks) => Some(chunks.concat()),
        _ => None,
    }
}

fn items(v: &Value) -> Option<&[Value]> {
    match v {
        Value::Array(a) | Value::IndefiniteArray(a) => Some(a),
        _ => None,
    }
}

/// A decoded, valid item as JSON.
fn to_js(v: Value) -> Result<Js, String> {
    Ok(match v {
        Value::Unsigned(n) => integer(false, n.into()),
        Value::Negative(n) => integer(true, n.into()),
        Value::Bytes(b) => Js::typed_array(&b),
        Value::IndefiniteBytes(chunks) => Js::typed_array(&chunks.concat()),
        Value::Text(s) => Js::Str(s),
        Value::IndefiniteText(chunks) => Js::Str(chunks.concat()),
        Value::Array(a) | Value::IndefiniteArray(a) => Js::Arr(
            a.into_vec()
                .into_iter()
                .map(to_js)
                .collect::<Result<_, _>>()?,
        ),
        Value::Map(m) | Value::IndefiniteMap(m) => {
            let pairs = m.into_vec();
            if !pairs
                .iter()
                .all(|(k, _)| matches!(k, Value::Text(_) | Value::IndefiniteText(_)))
            {
                // Not all keys are text (§6.1): `[key, value]` pairs.
                return Ok(Js::Arr(
                    pairs
                        .into_iter()
                        .map(|(k, v)| Ok(Js::Arr(vec![to_js(k)?, to_js(v)?])))
                        .collect::<Result<_, String>>()?,
                ));
            }
            let mut o = Obj::new();
            for (k, v) in pairs {
                if let Some(k) = text(&k) {
                    o.insert(k, to_js(v)?);
                }
            }
            Js::Obj(o)
        }
        Value::Tag(t) => {
            let number = t.number;
            tag(number, t.into_content())?
        }
        Value::Bool(b) => Js::Bool(b),
        Value::Null => Js::Null,
        Value::Undefined => Js::Undefined,
        Value::Simple(s) => Js::Obj(
            [("value".to_string(), Js::Num(f64::from(s.get())))]
                .into_iter()
                .collect(),
        ),
        Value::Float(f) => Js::Num(f),
    })
}

/// `{tag, contents}`, for tags without an interpretation.
fn generic(number: u64, content: Value) -> Result<Js, String> {
    Ok(Js::Obj(
        [
            ("tag".to_string(), integer(false, number.into())),
            ("contents".to_string(), to_js(content)?),
        ]
        .into_iter()
        .collect(),
    ))
}

/// A tag's content per its specification. The library has checked the
/// content of tags 0–5, 24, 32–36, 40, 41, 1040 and the typed arrays.
fn tag(n: u64, content: Value) -> Result<Js, String> {
    let fail = |reason: &str| Err(invalid(n, reason));
    Ok(match n {
        0 => match text(&content) {
            Some(s) => Js::Str(s),
            None => return fail("expected a text string"),
        },
        1 => Js::Str(
            epoch_time(&content)
                .ok_or_else(|| invalid(n, "not a finite number of seconds within ±2^63"))?,
        ),
        100 => {
            let days = match content {
                Value::Unsigned(d) => i128::from(d),
                Value::Negative(d) => -1 - i128::from(d),
                _ => return fail("expected an integer (RFC 8943 §2)"),
            };
            Js::Str(day_date(days).ok_or_else(|| invalid(n, "date out of range"))?)
        }
        1004 => match text(&content) {
            Some(s) if is_full_date(&s) => Js::Str(s),
            _ => return fail("expected an RFC 3339 full-date string (RFC 8943 §3)"),
        },
        2 | 3 => match bytes(&content) {
            Some(b) => bignum(n == 3, &b),
            None => return fail("expected a byte string"),
        },
        32 => match text(&content) {
            Some(s) => Js::Str(url::Url::parse(&s).map_or(s, String::from)),
            None => return fail("expected a text string"),
        },
        33 | 34 => {
            let Some(s) = text(&content) else {
                return fail("expected a text string");
            };
            let engine = if n == 33 {
                &base64::engine::general_purpose::URL_SAFE_NO_PAD
            } else {
                &base64::engine::general_purpose::STANDARD
            };
            Js::typed_array(&engine.decode(s).map_err(|e| invalid(n, e))?)
        }
        21065 => match text(&content) {
            Some(s) if is_i_regexp(&s) => generic(n, content)?,
            Some(_) => return fail("not an I-Regexp (RFC 9485 §3)"),
            None => return fail("expected a text string"),
        },
        21066 => {
            const SHAPE: &str = "expected [pattern, flags?] with text strings";
            let (pattern, flags) = match items(&content) {
                Some([p]) => (text(p), Some(String::new())),
                Some([p, Value::Undefined]) => (text(p), Some(String::new())),
                Some([p, f]) => (text(p), text(f)),
                _ => return fail(SHAPE),
            };
            let (Some(pattern), Some(flags)) = (pattern, flags) else {
                return fail(SHAPE);
            };
            new_regexp(&pattern, &flags).map_err(|e| invalid(n, e))?;
            generic(n, content)?
        }
        64..=87 if n != 76 && n != 83 && n != 87 => match bytes(&content) {
            Some(b) => typed_array(n, &b),
            None => return fail("expected a byte string"),
        },
        258 => {
            if items(&content).is_none() {
                return fail("expected an array");
            }
            to_js(content)?
        }
        262 => {
            let Some(b) = bytes(&content) else {
                return fail("expected a byte string");
            };
            let s = String::from_utf8(b).map_err(|_| invalid(n, "JSON text is not UTF-8"))?;
            js::parse(&s).map_err(|e| invalid(n, e))?
        }
        273 => match bytes(&content).map(|b| wtf8(&b)) {
            Some(Ok(s)) => Js::Str(s),
            Some(Err(reason)) => return fail(reason),
            None => return fail("expected a byte string"),
        },
        280 => {
            let ok = match &content {
                Value::Text(_) | Value::IndefiniteText(_) => true,
                v => match items(v) {
                    Some([name]) => text(name).is_some(),
                    Some([package, name]) => {
                        (text(package).is_some() || matches!(package, Value::Null))
                            && text(name).is_some()
                    }
                    _ => false,
                },
            };
            if !ok {
                return fail("expected a name, [name] or [package, name]");
            }
            // A symbol, which JSON can't show.
            Js::Undefined
        }
        55799 => to_js(content)?,
        _ => generic(n, content)?,
    })
}

/// Tag 1: RFC 3339 text in UTC for a number of seconds since the epoch.
fn epoch_time(content: &Value) -> Option<String> {
    let t = match *content {
        Value::Unsigned(s) => Timestamp::from_seconds(i64::try_from(s).ok()?),
        Value::Negative(s) => Timestamp::from_seconds(-1 - i64::try_from(s).ok()?),
        Value::Float(f) => {
            // Within ±2^63, whole seconds and the nanoseconds of the fraction.
            if !f.is_finite() || f.abs() >= i64::MAX as f64 {
                return None;
            }
            let secs = f.floor();
            let mut s = secs as i64;
            let mut ns = ((f - secs) * 1e9).round() as u32;
            if ns >= 1_000_000_000 {
                s = s.checked_add(1)?;
                ns -= 1_000_000_000;
            }
            Timestamp::new(s, ns)?
        }
        _ => return None,
    };
    Some(t.to_iso8601())
}

/// Tag 100: the `YYYY-MM-DD` date `days` after 1970-01-01.
fn day_date(days: i128) -> Option<String> {
    let secs = i64::try_from(days.checked_mul(86_400)?).ok()?;
    let iso = Timestamp::from_seconds(secs).to_iso8601();
    iso.split_once('T').map(|(date, _)| date.to_string())
}

/// RFC 3339 §5.6 `full-date`, with the §5.7 day-of-month limits.
fn is_full_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let num = |r: std::ops::Range<usize>| -> Option<u32> {
        let d = &b[r];
        d.iter()
            .all(u8::is_ascii_digit)
            .then(|| d.iter().fold(0, |n, c| n * 10 + u32::from(c - b'0')))
    };
    let (Some(y), Some(m), Some(d)) = (num(0..4), num(5..7), num(8..10)) else {
        return false;
    };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&d)
}

/// Tag 273: WTF-8 (UTF-8 that may encode unpaired surrogates) that is not
/// valid UTF-8, as a string; the surrogates become U+FFFD.
fn wtf8(b: &[u8]) -> Result<String, &'static str> {
    if std::str::from_utf8(b).is_ok() {
        return Err("valid UTF-8 belongs in a text string");
    }
    let mut out = String::new();
    let mut i = 0;
    // Whether the previous code point was a lead surrogate.
    let mut lead = false;
    while i < b.len() {
        let c = b[i];
        let (len, min) = match c {
            0x00..=0x7f => (1, 0),
            0xc2..=0xdf => (2, 0x80),
            0xe0..=0xef => (3, 0x800),
            0xf0..=0xf4 => (4, 0x1_0000),
            _ => return Err("not WTF-8"),
        };
        let Some(seq) = b.get(i..i + len) else {
            return Err("not WTF-8");
        };
        let mut cp = u32::from(c) & (0x7f >> len.min(4));
        if len == 1 {
            cp = u32::from(c);
        }
        for &x in &seq[1..] {
            if x & 0xc0 != 0x80 {
                return Err("not WTF-8");
            }
            cp = cp << 6 | u32::from(x & 0x3f);
        }
        if cp < min || cp > 0x10_ffff {
            return Err("not WTF-8");
        }
        let is_lead = (0xd800..0xdc00).contains(&cp);
        // A surrogate pair must be encoded as one 4-byte sequence.
        if lead && (0xdc00..0xe000).contains(&cp) {
            return Err("not WTF-8 (a surrogate pair encoded as two code points)");
        }
        lead = is_lead;
        out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
        i += len;
    }
    Ok(out)
}

/// RFC 9485 §3: whether `s` matches the I-Regexp ABNF (and isn't `[^]`).
/// Iterative: groups only count their depth.
fn is_i_regexp(s: &str) -> bool {
    let c: Vec<char> = s.chars().collect();
    let at = |i: usize| c.get(i).copied();
    let normal = |ch: char| {
        !matches!(
            ch,
            '(' | ')' | '*' | '+' | '.' | '?' | '[' | '\\' | ']' | '{' | '|' | '}'
        )
    };
    let single_esc = |ch: char| {
        matches!(
            ch,
            '('..='+' | '-' | '.' | '?' | '['..='^' | 'n' | 'r' | 't' | '{'..='}'
        )
    };
    // `\p{..}` or `\P{..}` at `i` (the backslash): the index after it.
    let category = |i: usize| -> Option<usize> {
        if at(i) != Some('\\') || !matches!(at(i + 1), Some('p' | 'P')) || at(i + 2) != Some('{') {
            return None;
        }
        let subs = match at(i + 3)? {
            'L' => "lmotu",
            'M' => "cen",
            'N' => "dlo",
            'P' => "cdefios",
            'Z' => "lps",
            'S' => "ckmo",
            'C' => "cfno",
            _ => return None,
        };
        let mut j = i + 4;
        if at(j).is_some_and(|x| subs.contains(x)) {
            j += 1;
        }
        (at(j) == Some('}')).then_some(j + 1)
    };
    // A CCchar at `i`: the index after it.
    let cc_char = |i: usize| -> Option<usize> {
        match at(i)? {
            '\\' => at(i + 1).filter(|&x| single_esc(x)).map(|_| i + 2),
            '-' | '[' | ']' => None,
            _ => Some(i + 1),
        }
    };
    // A charClassExpr at `i` (the `[`): the index after it.
    let class = |mut i: usize| -> Option<usize> {
        i += 1;
        if at(i) == Some('^') {
            i += 1;
        }
        let mut first = true;
        loop {
            match at(i)? {
                ']' if !first => return Some(i + 1),
                '-' if first => i += 1,
                '-' => return (at(i + 1) == Some(']')).then_some(i + 2),
                _ => {
                    if let Some(j) = category(i) {
                        i = j;
                    } else {
                        i = cc_char(i)?;
                        if at(i) == Some('-') && at(i + 1) != Some(']') {
                            i = cc_char(i + 1)?;
                        }
                    }
                }
            }
            first = false;
        }
    };
    let mut i = 0;
    let mut depth = 0usize;
    while let Some(ch) = at(i) {
        // The atom, then an optional quantifier.
        i = match ch {
            '|' => {
                i += 1;
                continue;
            }
            '(' => {
                depth += 1;
                i += 1;
                continue;
            }
            ')' => {
                let Some(d) = depth.checked_sub(1) else {
                    return false;
                };
                depth = d;
                i + 1
            }
            '.' => i + 1,
            '\\' => match category(i).or_else(|| cc_char(i)) {
                Some(j) => j,
                None => return false,
            },
            '[' => match class(i) {
                Some(j) => j,
                None => return false,
            },
            ch if normal(ch) => i + 1,
            _ => return false,
        };
        match at(i) {
            Some('*' | '+' | '?') => i += 1,
            Some('{') => {
                let digits = |mut j: usize| {
                    let start = j;
                    while at(j).is_some_and(|x| x.is_ascii_digit()) {
                        j += 1;
                    }
                    (j > start).then_some(j)
                };
                let Some(mut j) = digits(i + 1) else {
                    return false;
                };
                if at(j) == Some(',') {
                    j = digits(j + 1).unwrap_or(j + 1);
                }
                if at(j) != Some('}') {
                    return false;
                }
                i = j + 1;
            }
            _ => {}
        }
    }
    depth == 0
}

/// `new RegExp(source, flags)`: a SyntaxError for flags other than
/// `dgimsuyv` (each once, not both `u` and `v`) or a pattern `regress`
/// rejects.
fn new_regexp(source: &str, flags: &str) -> Result<(), String> {
    let mut seen = String::new();
    for f in flags.chars() {
        if !"dgimsuyv".contains(f)
            || seen.contains(f)
            || (f == 'u' && seen.contains('v'))
            || (f == 'v' && seen.contains('u'))
        {
            return Err(format!(
                "Invalid flags supplied to RegExp constructor '{flags}'"
            ));
        }
        seen.push(f);
    }
    // regress recurses once per `|` of an alternation (about 1 KiB each in
    // a debug build) and up to 255 levels of nesting; past 100,000 `|` the
    // stack it would need is too much to reserve, so the pattern goes
    // unchecked.
    let bars = source.matches('|').count();
    if bars <= 100_000 {
        let mut opts = regress::Flags::from(flags);
        // Only the syntax matters: skip the optimizer, which is quadratic
        // in long alternations. `v` is checked as strictly as `u`.
        opts.no_opt = true;
        opts.unicode |= opts.unicode_sets;
        let checked = stacker::grow((4 << 20) + bars * 2048, || {
            regress::Regex::with_flags(source, opts).map(drop)
        });
        if let Err(e) = checked
            && !REGRESS_LIMITS.contains(&e.text.as_str())
        {
            return Err(format!(
                "Invalid regular expression: /{source}/{}: {e}",
                canonical_flags(&seen)
            ));
        }
    }
    Ok(())
}

/// Flags in the order `RegExp.prototype.flags` lists them.
fn canonical_flags(flags: &str) -> String {
    "dgimsuvy".chars().filter(|&f| flags.contains(f)).collect()
}

/// regress's own size limits, which V8 doesn't have: such a pattern may be
/// valid.
const REGRESS_LIMITS: &[&str] = &[
    "Regular expression is too deeply nested",
    "Loop count limit exceeded",
];

/// The RFC 8746 typed arrays (but binary128), by element.
fn typed_array(tag: u64, b: &[u8]) -> Js {
    // Tags 68 (clamped) and 72 (signed) are single bytes.
    if tag == 64 || tag == 68 {
        return Js::typed_array(b);
    }
    // Bit 4: float; bit 3: signed; bit 2: little endian; bits 0-1: size.
    let float = tag & 0x10 != 0;
    let signed = tag & 0x08 != 0;
    let little = tag & 0x04 != 0;
    let size = if float {
        2 << (tag & 3)
    } else {
        1 << (tag & 3)
    };
    let items = b.chunks_exact(size).map(|c| {
        let mut v = [0u8; 8];
        if little {
            v[..size].copy_from_slice(c);
        } else {
            for (i, &x) in c.iter().rev().enumerate() {
                v[i] = x;
            }
        }
        let raw = u64::from_le_bytes(v);
        match (float, signed, size) {
            (true, _, 2) => Js::Num(utilify_cbor::float::f16_to_f64(raw as u16)),
            (true, _, 4) => Js::Num(f64::from(f32::from_bits(raw as u32))),
            (true, _, _) => Js::Num(f64::from_bits(raw)),
            (false, false, _) => integer(false, raw.into()),
            (false, true, 1) => Js::Num(f64::from(raw as u8 as i8)),
            (false, true, 2) => Js::Num(f64::from(raw as u16 as i16)),
            (false, true, 4) => Js::Num(f64::from(raw as u32 as i32)),
            (false, true, _) => {
                let n = raw as i64;
                integer(
                    n < 0,
                    if n < 0 {
                        (-1 - i128::from(n)) as u128
                    } else {
                        n as u128
                    },
                )
            }
        }
    });
    Js::Obj(items.enumerate().map(|(i, v)| (i.to_string(), v)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::from_hex;

    fn dec(hex: &str) -> String {
        let bytes = from_hex(hex, "Auto", 2).unwrap();
        match CBOR_DECODE.apply(Dish::Bytes(bytes), &[]) {
            Ok(d) => d.into_text(),
            Err(e) => e.0,
        }
    }

    /// `dec`, minified (errors as they are).
    fn min(hex: &str) -> String {
        let out = dec(hex);
        match js::parse(&out) {
            Ok(v) => js::stringify_compact(&v).unwrap().unwrap(),
            Err(_) => out,
        }
    }

    fn enc(json: &str) -> String {
        match CBOR_ENCODE.apply(Dish::Text(json.into()), &[]) {
            Ok(d) => d.into_bytes().iter().map(|b| format!("{b:02x}")).collect(),
            Err(e) => e.0,
        }
    }

    #[test]
    fn encodes_deterministically() {
        assert_eq!(enc("-0"), "f98000");
        assert_eq!(enc("1.5"), "f93e00");
        assert_eq!(enc("65505"), "19ffe1");
        assert_eq!(enc("100000.5"), "fa47c35040");
        assert_eq!(enc("0.1"), "fb3fb999999999999a");
        assert_eq!(enc("9007199254740992"), "fa5a000000");
        assert_eq!(enc("-9007199254740991"), "3b001ffffffffffffe");
        assert_eq!(enc("5.960464477539063e-8"), "f90001");
        assert_eq!(enc("1e400"), "f97c00");
        // §4.2.1: keys sorted bytewise on their encodings.
        assert_eq!(
            enc("{\"bb\":1,\"a\":2,\"1\":3,\"\":4}"),
            "a4600461310361610262626201"
        );
        assert_eq!(enc("[\"é\",null,true]"), "8362c3a9f6f5");
    }

    #[test]
    fn decodes_values() {
        assert_eq!(dec("f9 3e 00"), "1.5");
        assert_eq!(dec("a1 01 02"), "[\n    [\n        1,\n        2\n    ]\n]");
        assert_eq!(
            dec("4401020304"),
            "{\n    \"0\": 1,\n    \"1\": 2,\n    \"2\": 3,\n    \"3\": 4\n}"
        );
        assert_eq!(dec("f820"), "{\n    \"value\": 32\n}");
        assert_eq!(
            dec("c4 82 21 19 6a b3"),
            "{\n    \"tag\": 4,\n    \"contents\": [\n        -2,\n        27315\n    ]\n}"
        );
        assert_eq!(dec("7f 61 61 61 62 ff"), "\"ab\"");
        assert_eq!(
            dec("a1 69 5f5f70726f746f5f5f 01"),
            "{\n    \"__proto__\": 1\n}"
        );
        assert_eq!(dec("f7"), "\"undefined\" is not valid JSON");
        let deep = format!("{}01", "81".repeat(MAX_DEPTH));
        assert!(dec(&deep).starts_with("[\n    [\n"));
        assert!(
            dec(&format!("{}01", "c6".repeat(MAX_DEPTH - 1))).starts_with("{\n    \"tag\": 6,")
        );
    }

    #[test]
    fn large_integers_are_exact() {
        // RFC 8949 Appendix A examples.
        assert_eq!(dec("1b000000e8d4a51000"), "1000000000000");
        assert_eq!(dec("1bffffffffffffffff"), "\"18446744073709551615\"");
        assert_eq!(dec("3bffffffffffffffff"), "\"-18446744073709551616\"");
        assert_eq!(dec("c249010000000000000000"), "\"18446744073709551616\"");
        assert_eq!(dec("c349010000000000000000"), "\"-18446744073709551617\"");
        assert_eq!(dec("1b0020000000000000"), "9007199254740992");
        assert_eq!(dec("1b0020000000000001"), "\"9007199254740993\"");
        assert_eq!(dec("3b001fffffffffffff"), "-9007199254740992");
        assert_eq!(dec("3b0020000000000000"), "\"-9007199254740993\"");
        assert_eq!(dec("c24101"), "1");
        assert_eq!(dec("c340"), "-1");
    }

    #[test]
    fn dates() {
        // §3.4.1 (Appendix A): kept as the RFC 3339 text.
        assert_eq!(
            dec("c074323031332d30332d32315432303a30343a30305a"),
            "\"2013-03-21T20:04:00Z\""
        );
        // Not RFC 3339: no time, day 30 of February, lowercase `t`.
        assert!(
            dec("c0 6a 323031332d30322d3330")
                .starts_with("CBOR Decode - invalid content for tag 0")
        );
        assert!(
            dec("c074323031332d30332d32317432303a30343a30305a")
                .starts_with("CBOR Decode - invalid content for tag 0")
        );
        // §3.4.2 (Appendix A): integer and float seconds.
        assert_eq!(dec("c11a514b67b0"), "\"2013-03-21T20:04:00Z\"");
        assert_eq!(dec("c1fb41d452d9ec200000"), "\"2013-03-21T20:04:00.5Z\"");
        assert_eq!(dec("c13a00000000"), "\"1969-12-31T23:59:59Z\"");
        assert_eq!(
            dec("c11b7fffffffffffffff"),
            "\"+292277026596-12-04T15:30:07Z\""
        );
        assert!(dec("c1f97e00").starts_with("CBOR Decode - invalid content for tag 1"));
        assert!(dec("c11bffffffffffffffff").starts_with("CBOR Decode - invalid content for tag 1"));
        assert!(dec("c1 63616263").starts_with("CBOR Decode - invalid content for tag 1"));
        // RFC 8943 §4 examples: 1940-10-09 and 1980-12-08.
        assert_eq!(dec("d864 39 29b3"), "\"1940-10-09\"");
        assert_eq!(dec("d864 19 0f9a"), "\"1980-12-08\"");
        assert_eq!(dec("d903ec 6a 313934302d31302d3039"), "\"1940-10-09\"");
        assert!(
            dec("d903ec 6a 313934302d30322d3330")
                .starts_with("CBOR Decode - invalid content for tag 1004")
        );
        assert!(dec("d864 f93c00").starts_with("CBOR Decode - invalid content for tag 100"));
    }

    #[test]
    fn encoded_text() {
        // Tag 32: WHATWG serialization, or a relative reference as is.
        assert_eq!(
            dec("d8 20 78 1e 48545450733a2f2f4578616d706c652e636f6d2f612f2e2e2f623f712366"),
            "\"https://example.com/b?q#f\""
        );
        assert_eq!(dec("d8 20 66 6d61696c746f"), "\"mailto\"");
        assert!(dec("d8 20 63 612062").starts_with("CBOR Decode - invalid content for tag 32"));
        // Tags 33 and 34 (§3.4.5.3): strict RFC 4648.
        assert_eq!(
            dec("d8 21 63 2d5f41"),
            "{\n    \"0\": 251,\n    \"1\": 240\n}"
        );
        assert_eq!(dec("d8 22 64 41413d3d"), "{\n    \"0\": 0\n}");
        for bad in [
            "d8 21 64 41413d3d",   // base64url with padding
            "d8 21 62 4142",       // nonzero padding bits
            "d8 21 61 41",         // one character in the last block
            "d8 22 62 4141",       // base64 without padding
            "d8 22 64 41413d41",   // misplaced padding
            "d8 22 65 414141413d", // wrong number of padding characters
        ] {
            assert!(
                dec(bad).starts_with("CBOR Decode - invalid content for tag 3"),
                "{bad}"
            );
        }
    }

    #[test]
    fn other_tags() {
        // RFC 8746 typed arrays.
        assert_eq!(dec("d8 4d 42 ff01"), "{\n    \"0\": 511\n}");
        assert_eq!(dec("d8 50 42 3c00"), "{\n    \"0\": 1\n}");
        assert_eq!(dec("d8 54 42 003c"), "{\n    \"0\": 1\n}");
        assert_eq!(
            dec("d8 47 48 ffffffffffffffff"),
            "{\n    \"0\": \"18446744073709551615\"\n}"
        );
        assert_eq!(dec("d8 4b 48 ffffffffffffffff"), "{\n    \"0\": -1\n}");
        assert!(dec("d8 41 43 000000").starts_with("CBOR Decode - invalid content for tag 65"));
        // Tag 262: JSON text in a byte string.
        assert_eq!(dec("d9 0106 43 5b315d"), "[\n    1\n]");
        assert!(dec("d9 0106 63 5b315d").starts_with("CBOR Decode - invalid content for tag 262"));
        // Tag 273: WTF-8 that isn't UTF-8 (a lone surrogate).
        assert_eq!(dec("d9 0111 44 61eda080"), "\"a\u{fffd}\"");
        assert!(dec("d9 0111 43 616263").starts_with("CBOR Decode - invalid content for tag 273"));
        assert!(
            dec("d9 0111 46 eda0bdedb880").starts_with("CBOR Decode - invalid content for tag 273")
        );
        // Tag 280: a symbol, which JSON can't show.
        assert_eq!(dec("81 d9 0118 63 616263"), "[\n    null\n]");
        assert_eq!(dec("81 d9 0118 82 f6 61 61"), "[\n    null\n]");
        assert!(dec("d9 0118 01").starts_with("CBOR Decode - invalid content for tag 280"));
        // Tag 258, a set: an array.
        assert_eq!(min("d9 0102 82 01 02"), "[1,2]");
        assert_eq!(min("d9 0102 80"), "[]");
        assert_eq!(min("a1 61 73 d9 0102 81 61 78"), r#"{"s":["x"]}"#);
        assert!(dec("d9 0102 01").starts_with("CBOR Decode - invalid content for tag 258"));
        assert_eq!(dec("d9 d9f7 01"), "1");
    }

    #[test]
    fn maps_with_other_keys_are_pairs() {
        // §6.1: an object only when every key is text; otherwise every
        // entry is a [key, value] pair, in input order.
        assert_eq!(min("a2 61 62 02 61 61 01"), r#"{"b":2,"a":1}"#);
        assert_eq!(min("a2 01 61 61 61 31 61 62"), r#"[[1,"a"],["1","b"]]"#);
        assert_eq!(min("a1 41 01 f5"), r#"[[{"0":1},true]]"#);
        assert_eq!(min("a1 82 01 02 a0"), r#"[[[1,2],{}]]"#);
        assert_eq!(min("a1 a1 01 02 03"), r#"[[[[1,2]],3]]"#);
        assert_eq!(min("a1 f6 f7"), "[[null,null]]");
        assert_eq!(min("bf 20 7f 61 61 ff ff"), r#"[[-1,"a"]]"#);
        assert_eq!(
            min("a1 1b ffffffffffffffff 00"),
            r#"[["18446744073709551615",0]]"#
        );
        // Nested in text-keyed maps, and in tags (259, explicit maps).
        assert_eq!(min("a1 61 6d a1 01 02"), r#"{"m":[[1,2]]}"#);
        assert_eq!(min("d9 0103 a1 01 02"), r#"{"tag":259,"contents":[[1,2]]}"#);
        assert_eq!(
            min("d9 0103 a1 61 61 02"),
            r#"{"tag":259,"contents":{"a":2}}"#
        );
        // Errors in keys and values still count.
        assert!(min("a1 c1 61 61 01").starts_with("CBOR Decode - "));
        assert!(min("a1 01 c1 61 61").starts_with("CBOR Decode - "));
    }

    #[test]
    fn decode_errors() {
        assert_eq!(
            dec(""),
            "CBOR Decode - unexpected end of input in data item at offset 0"
        );
        assert_eq!(
            dec("00 01"),
            "CBOR Decode - unexpected data after the data item at offset 1"
        );
        assert_eq!(
            dec("1c"),
            "CBOR Decode - reserved additional information 28 in initial byte 0x1c at offset 0"
        );
        // §5.6: a map with duplicate keys is not valid.
        assert!(dec("a2 01 02 01 03").starts_with("CBOR Decode - duplicate map key"));
        // §3.4: tags 65535, 2^32 - 1 and 2^64 - 1 are always invalid.
        assert!(dec("d9ffff01").starts_with("CBOR Decode - invalid content for tag 65535"));
        assert!(dec("dbffffffffffffffff01").starts_with("CBOR Decode - invalid content for tag"));
        assert_eq!(
            dec("81 ff"),
            "CBOR Decode - unexpected \"break\" stop code at offset 1"
        );
        assert_eq!(
            dec("43 0102"),
            "CBOR Decode - unexpected end of input in data item at offset 0"
        );
        assert_eq!(
            dec("62c328"),
            "CBOR Decode - text string is not valid UTF-8 at offset 0"
        );
        let deep = dec(&format!("{}01", "81".repeat(MAX_DEPTH + 1)));
        assert!(
            deep.starts_with("CBOR Decode - nesting deeper than 5001"),
            "{deep}"
        );
    }

    #[test]
    fn i_regexps() {
        for p in [
            "",
            "a|b",
            "()",
            "a{2,3}",
            "a{2,}",
            "[^a-z]",
            "[-a]",
            "[a-]",
            "[\\]]",
            "\\p{Lu}+",
            "[\\p{L}\\n-]",
            "x.y",
            "(a(b)c)*",
            "^$",
            "\\{",
            "é",
        ] {
            assert!(is_i_regexp(p), "{p}");
        }
        for p in [
            "[^]",
            "(",
            ")",
            "a**",
            "\\d",
            "\\b",
            "a{",
            "a{,2}",
            "[]",
            "[a-\\p{L}]",
            "\\p{X}",
            "{1}",
            "(?:a)",
            "a]",
            "[\\d]",
        ] {
            assert!(!is_i_regexp(p), "{p}");
        }
        let t = |s: &str| {
            let hex: String = s.bytes().map(|b| format!("{b:02x}")).collect();
            format!("{:02x}{hex}", 0x60 + s.len())
        };
        assert_eq!(
            dec(&format!("d9 5249 {}", t("a[]"))),
            "CBOR Decode - invalid content for tag 21065: not an I-Regexp (RFC 9485 §3)"
        );
        assert_eq!(
            min(&format!("d9 5249 {}", t("[a-z]+"))),
            r#"{"tag":21065,"contents":"[a-z]+"}"#
        );
    }

    #[test]
    fn regexps_like_new_regexp() {
        // A CBOR text string, as hex.
        let t = |s: &str| {
            let hex: String = s.bytes().map(|b| format!("{b:02x}")).collect();
            match s.len() {
                n @ 0..24 => format!("{:02x}{hex}", 0x60 + n),
                n => format!("7a{n:08x}{hex}"),
            }
        };
        // Tag 35's syntax is agreed out of band: any text string.
        assert_eq!(
            dec(&format!("d8 23 {}", t("("))),
            "{\n    \"tag\": 35,\n    \"contents\": \"(\"\n}"
        );
        let tag21066 = |p: &str, f: &str| min(&format!("d9 524a 82 {} {}", t(p), t(f)));
        let one = |p: &str| min(&format!("d9 524a 81 {}", t(p)));
        // Valid ones are `{tag, contents}`, contents as decoded.
        let rx = |items: Vec<Js>| {
            let o: Obj = [
                ("tag".to_string(), Js::Num(21066.0)),
                ("contents".to_string(), Js::Arr(items)),
            ]
            .into_iter()
            .collect();
            js::stringify_compact(&Js::Obj(o)).unwrap().unwrap()
        };
        let rx1 = |p: &str| rx(vec![Js::str(p)]);
        // Annex B's looser syntax holds without `u`.
        for p in ["]", "a{", "\\c", "(?<n>a)\\k<n>", "a|b"] {
            assert_eq!(one(p), rx1(p), "{p}");
        }
        assert_eq!(
            tag21066("\\p{L}", "giu"),
            r#"{"tag":21066,"contents":["\\p{L}","giu"]}"#
        );
        assert_eq!(
            tag21066("[a--b]", "v"),
            rx(vec![Js::str("[a--b]"), Js::str("v")])
        );
        // An `undefined` flags is null in the array.
        assert_eq!(
            min(&format!("d9 524a 82 {} f7", t("a"))),
            r#"{"tag":21066,"contents":["a",null]}"#
        );
        // Not text strings, or not one or two items.
        for bad in [
            format!("d9 524a 82 {} f6", t("a")),
            "d9 524a 81 01".to_string(),
            "d9 524a 80".to_string(),
            format!("d9 524a 83 {0} {0} {0}", t("a")),
            t("a"),
        ] {
            let bad = if bad.starts_with("d9") {
                bad
            } else {
                format!("d9 524a {bad}")
            };
            assert!(
                dec(&bad).starts_with("CBOR Decode - invalid content for tag 21066: expected"),
                "{bad}"
            );
        }
        // Invalid ones fail with new RegExp's SyntaxError (regress's wording
        // after the colon).
        let invalid = |out: String, prefix: &str| {
            assert!(
                out.starts_with(&format!(
                    "CBOR Decode - invalid content for tag 21066: Invalid regular expression: {prefix}: "
                )),
                "{out}"
            );
        };
        invalid(one("("), "/(/");
        invalid(tag21066("\\p{Foo}", "u"), "/\\p{Foo}/u");
        invalid(tag21066("[a--b]", "u"), "/[a--b]/u");
        // Flags in RegExp.prototype.flags order, as V8 prints them.
        invalid(tag21066("(", "ysd"), "/(/dsy");
        for f in ["uv", "gg", "x"] {
            assert_eq!(
                tag21066("a", f),
                format!(
                    "CBOR Decode - invalid content for tag 21066: Invalid flags supplied to RegExp constructor '{f}'"
                )
            );
        }
        // Accepted difference: ECMA-262 rejects a quantified assertion;
        // regress takes it.
        assert_eq!(one("\\b*"), rx1("\\b*"));
        // Past regress's limits a pattern isn't checked.
        let stars = "a*".repeat(70_000);
        assert_eq!(one(&stars), rx1(&stars));
        let deep = format!("{}{}", "(?:".repeat(300), ")".repeat(300));
        assert_eq!(one(&deep), rx1(&deep));
        // Long alternations need a big stack, not a slow optimizer.
        let alts = format!("{}a", "a|".repeat(20_000));
        assert_eq!(one(&alts), rx1(&alts));
        assert!(one(&format!("{alts}(")).starts_with(
            "CBOR Decode - invalid content for tag 21066: Invalid regular expression: /a|"
        ));
    }

    /// Run on a thread as small as the app's.
    fn small_stack<T: Send>(f: impl FnOnce() -> T + Send) -> T {
        std::thread::scope(|s| {
            std::thread::Builder::new()
                .stack_size(512 << 10)
                .spawn_scoped(s, f)
                .unwrap()
                .join()
                .unwrap()
        })
    }

    #[test]
    fn deep_input_on_small_stacks() {
        let n = 5000;
        let json = format!("{}1{}", "[".repeat(n), "]".repeat(n));
        let bytes = small_stack(|| enc(&json));
        assert_eq!(bytes, format!("{}01", "81".repeat(n)));
        assert!(small_stack(|| dec(&bytes)).starts_with("[\n    [\n"));
        let tagged = format!("{}01", "c6".repeat(n));
        assert!(small_stack(|| dec(&tagged)).starts_with("{\n    \"tag\": 6,"));
        // Maps keyed by maps (two JSON arrays each), and sets of sets (a
        // tag and an array each, so half as many).
        let keyed = format!("{}{}", "a1".repeat(n), "01".repeat(n + 1));
        assert!(small_stack(|| dec(&keyed)).starts_with("[\n    [\n        [\n"));
        let sets = format!("{}01", "d9010281".repeat(n / 2));
        assert!(small_stack(|| dec(&sets)).starts_with("[\n    [\n"));
    }
}
