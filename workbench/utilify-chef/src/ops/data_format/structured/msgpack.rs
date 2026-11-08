//! MessagePack (the msgpack specification, github.com/msgpack/msgpack
//! `spec.md`) to and from JSON, with `utilify-msgpack`'s token reader and
//! writer doing the wire format.
//!
//! Encoding follows "Serialization: type to format conversion": every
//! number that is a whole number in the MessagePack integer range
//! (-2^63 to 2^64 - 1) is an integer in its smallest format (`-0` is 0);
//! every other number (fractions, and whole numbers outside that range,
//! such as `1e20` or `1e300`) is a float 64, so no value is wrapped or
//! truncated. Strings are str, arrays and objects array and map (members
//! whose value is `undefined` dropped, as `JSON.parse` never makes one).
//!
//! Decoding is strict, as the library's defaults are: strings must be valid
//! UTF-8 (the spec lets a deserializer reject invalid sequences), and
//! errors are the library's, in input order (truncated input, trailing
//! bytes, reserved byte 0xc1, bad timestamps). Arrays and maps nest at most
//! [`MAX_DEPTH`] deep, the depth `JSON.parse` accepts on input, so anything
//! To MessagePack writes reads back.
//!
//! Extension types are application-defined ("Extension types"), so they
//! are decoded generically by length, whatever the ext format: type -1, the
//! only predefined one, is a timestamp ("Timestamp extension type": 4, 8 or
//! 12 bytes; any other length, or nanoseconds over 999,999,999, is an
//! error), shown as ISO 8601 text in UTC with full nanosecond precision
//! (`1970-01-01T00:00:01.5Z`; years outside 0000–9999 expanded, as
//! `±YYYYYY`). Every other type is `[type, bytes]`.
//!
//! A map whose keys are all str, and distinct, is a JSON object. Any other
//! map is an array of `[key, value]` pairs in input order (`{1: "a"}` →
//! `[[1, "a"]]`), keys shown as any other value. The spec lets a map's keys
//! be any objects ("Map represents key-value pairs of objects") and doesn't
//! forbid repeating one, and JSON member names are only strings, so this
//! keeps every entry: notepack's `String(key)` names (`1` and `"1"` both
//! `"1"`, bin keys read as lossy UTF-8, the last repeated key winning) lost
//! entries and the keys' types.
//!
//! Presentation kept from CyberChef (notepack's JavaScript values, which no
//! spec defines in JSON): integers as JSON numbers (as JavaScript reads
//! them; past 2^53 the nearest double), and bin and extension bytes as Node
//! `Buffer` objects (`{"type": "Buffer", "data": [...]}`). Empty input
//! fails with notepack's message, "Could not parse".

use super::CATEGORY;
use super::js::{Js, Obj, json_dish, json_input, with_big_stack};
use crate::{Dish, OpError, Operation};
use std::collections::HashSet;
use utilify_msgpack::{
    DecodeError, DecodeErrorKind, DecodeOptions, EncodeError, EncodeErrorKind, ExtensionType,
    Integer, Timestamp, Token, TokenReader, TokenWriter,
};

pub(super) fn ops() -> Vec<&'static Operation> {
    vec![&TO_MESSAGEPACK, &FROM_MESSAGEPACK]
}

pub static TO_MESSAGEPACK: Operation = Operation {
    name: "To MessagePack",
    category: CATEGORY,
    description: "Converts JSON to MessagePack encoded byte buffer. MessagePack is a computer data interchange format. It is a binary form for representing simple data structures like arrays and associative arrays.",
    info_url: Some("https://wikipedia.org/wiki/MessagePack"),
    args: &[],
    run: |input, _| {
        with_big_stack(|| {
            let mut w = TokenWriter::new(vec![]).max_depth(usize::MAX);
            encode(&json_input(&input)?, &mut w)
                .and_then(|()| w.finish())
                .map(Dish::Bytes)
                .map_err(|e| OpError::new(format!("Could not encode JSON to MessagePack: {e}")))
        })
    },
};

pub static FROM_MESSAGEPACK: Operation = Operation {
    name: "From MessagePack",
    category: CATEGORY,
    description: "Converts MessagePack encoded data to JSON. MessagePack is a computer data interchange format. It is a binary form for representing simple data structures like arrays and associative arrays.",
    info_url: Some("https://wikipedia.org/wiki/MessagePack"),
    args: &[],
    run: |input, _| {
        with_big_stack(|| {
            let value = decode(&input.bytes())
                .map_err(|e| OpError::new(format!("Could not decode MessagePack to JSON: {e}")))?;
            json_dish(&value)
        })
    },
};

fn encode(v: &Js, w: &mut TokenWriter<Vec<u8>>) -> Result<(), EncodeError> {
    match v {
        // JSON input never holds undefined at the top.
        Js::Undefined | Js::Null => w.write(Token::Nil),
        Js::Bool(b) => w.write(Token::Bool(*b)),
        Js::Num(n) => w.write(number(*n)),
        Js::Str(s) => w.write(Token::Str(s.as_bytes())),
        Js::Arr(items) => {
            w.write(Token::Array(length("array", items.len())?))?;
            items.iter().try_for_each(|item| encode(item, w))
        }
        Js::Obj(o) => {
            let entries: Vec<_> = o
                .iter()
                .filter(|(_, v)| !matches!(v, Js::Undefined))
                .collect();
            w.write(Token::Map(length("map", entries.len())?))?;
            for (k, v) in entries {
                w.write(Token::Str(k.as_bytes()))?;
                encode(v, w)?;
            }
            Ok(())
        }
    }
}

fn length(what: &'static str, length: usize) -> Result<u32, EncodeError> {
    u32::try_from(length).map_err(|_| EncodeError::new(EncodeErrorKind::TooLong { what, length }))
}

/// A whole number in the MessagePack integer range as an integer (which
/// `TokenWriter::write` puts in its smallest format), anything else as a
/// float 64.
fn number(n: f64) -> Token<'static> {
    if n.is_finite() && n.trunc() == n {
        // `as` saturates, so a number past the i128 range fails the check.
        let i = n as i128;
        if i as f64 == n
            && let Ok(i) = Integer::try_from(i)
        {
            return Token::Integer(i);
        }
    }
    Token::Float64(n)
}

/// How deeply arrays and maps may nest: the depth `JSON.parse` accepts.
pub const MAX_DEPTH: usize = 5001;

/// Strict decoding (UTF-8 strings, timestamps checked).
const DECODE: DecodeOptions = DecodeOptions::new().max_depth(MAX_DEPTH);

fn decode(buf: &[u8]) -> Result<Js, String> {
    if buf.is_empty() {
        return Err("Error: Could not parse".into());
    }
    let mut tokens = TokenReader::with_options(buf, &DECODE);
    let value = value(&mut tokens).map_err(|e| e.to_string())?;
    if !tokens.remaining().is_empty() {
        let at = tokens.position();
        return Err(DecodeError::new(DecodeErrorKind::TrailingBytes, at).to_string());
    }
    Ok(value)
}

fn value(tokens: &mut TokenReader<'_>) -> Result<Js, DecodeError> {
    Ok(item(tokens)?.0)
}

/// The next value, and whether it was a str (which alone may be a JSON
/// object's member name).
fn item(tokens: &mut TokenReader<'_>) -> Result<(Js, bool), DecodeError> {
    // `TokenReader`'s own `position`, not `Iterator::position` on `&mut`.
    let at = TokenReader::position(tokens);
    let Some((_, token)) = tokens.next_token()? else {
        return Err(DecodeError::new(DecodeErrorKind::UnexpectedEnd, at));
    };
    let value = match token {
        Token::Nil => Js::Null,
        Token::Bool(b) => Js::Bool(b),
        Token::Integer(i) => Js::Num(i.as_f64()),
        Token::Float32(f) => Js::Num(f64::from(f)),
        Token::Float64(f) => Js::Num(f),
        // The reader has checked the UTF-8.
        Token::Str(s) => return Ok((Js::Str(String::from_utf8_lossy(s).into_owned()), true)),
        Token::Bin(b) => Js::buffer(b),
        Token::Array(n) => {
            let mut out = vec![];
            for _ in 0..n {
                out.push(value(tokens)?);
            }
            Js::Arr(out)
        }
        Token::Map(n) => map(tokens, n)?,
        // The reader has checked the timestamp's length and nanoseconds.
        Token::Ext { type_id: -1, data } => match Timestamp::from_payload(data) {
            Ok(t) => Js::Str(t.to_iso8601()),
            Err(e) => return Err(DecodeError::custom_message(e)),
        },
        Token::Ext { type_id, data } => {
            Js::Arr(vec![Js::Num(f64::from(type_id)), Js::buffer(data)])
        }
    };
    Ok((value, false))
}

/// A map of `n` entries: an object when its keys are distinct strs, and
/// `[key, value]` pairs otherwise.
fn map(tokens: &mut TokenReader<'_>, n: u32) -> Result<Js, DecodeError> {
    let mut pairs = vec![];
    let mut names = true;
    for _ in 0..n {
        let (key, is_str) = item(tokens)?;
        names &= is_str;
        pairs.push((key, value(tokens)?));
    }
    if names {
        let mut seen = HashSet::new();
        let distinct = pairs
            .iter()
            .all(|(k, _)| matches!(k, Js::Str(k) if seen.insert(k.as_str())));
        if distinct {
            let mut o = Obj::new();
            for (k, v) in pairs {
                if let Js::Str(k) = k {
                    o.insert(k, v);
                }
            }
            return Ok(Js::Obj(o));
        }
    }
    Ok(Js::Arr(
        pairs
            .into_iter()
            .map(|(k, v)| Js::Arr(vec![k, v]))
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::super::js;
    use super::*;
    use crate::util::from_hex;

    fn from(hex: &str) -> String {
        let bytes = from_hex(hex, "Auto", 2).unwrap();
        match FROM_MESSAGEPACK.apply(Dish::Bytes(bytes), &[]) {
            Ok(d) => d.into_text(),
            Err(e) => e.0,
        }
    }

    fn to(json: &str) -> String {
        match TO_MESSAGEPACK.apply(Dish::Text(json.into()), &[]) {
            Ok(d) => d.into_bytes().iter().map(|b| format!("{b:02x}")).collect(),
            Err(e) => e.0,
        }
    }

    #[test]
    fn encodes_smallest_forms() {
        assert_eq!(to("-0"), "00");
        assert_eq!(to("-33"), "d0df");
        assert_eq!(to("4294967296"), "cf0000000100000000");
        assert_eq!(to("-2147483649"), "d3ffffffff7fffffff");
        assert_eq!(to("1.5"), "cb3ff8000000000000");
        assert_eq!(
            to("{\"b\":1,\"a\":null,\"1\":true}"),
            "83a131c3a16201a161c0"
        );
        assert_eq!(to("\"é😀\""), "a6c3a9f09f9880");
        assert_eq!(
            to(""),
            "Error translating from ArrayBuffer to JSON: SyntaxError: Unexpected end of JSON input"
        );
    }

    #[test]
    fn numbers_past_the_integer_range_are_floats() {
        // 2^64 - 2048 and -2^63 are integers; 2^64 and beyond are not.
        assert_eq!(to("18446744073709549568"), "cffffffffffffff800");
        assert_eq!(to("-9223372036854775808"), "d38000000000000000");
        assert_eq!(to("18446744073709551616"), "cb43f0000000000000");
        assert_eq!(to("1e20"), "cb4415af1d78b58c40");
        assert_eq!(to("-1e30"), "cbc6293e5939a08cea");
        assert_eq!(to("1e300"), "cb7e37e43c8800759c");
        // Round trip.
        assert_eq!(from("cb4415af1d78b58c40"), "100000000000000000000");
    }

    #[test]
    fn decodes_values() {
        assert_eq!(
            from("c403010203"),
            "{\n    \"type\": \"Buffer\",\n    \"data\": [\n        1,\n        2,\n        3\n    ]\n}"
        );
        assert_eq!(from("cf0020000000000001"), "9007199254740992");
        assert_eq!(
            from("82 a161 01 a162 02"),
            "{\n    \"a\": 1,\n    \"b\": 2\n}"
        );
        let deep = format!("{}01", "91".repeat(MAX_DEPTH));
        assert!(from(&deep).starts_with("[\n    [\n"));
    }

    /// `from`, minified (errors as they are).
    fn min(hex: &str) -> String {
        let out = from(hex);
        match js::parse(&out) {
            Ok(v) => js::stringify_compact(&v).unwrap().unwrap(),
            Err(_) => out,
        }
    }

    #[test]
    fn maps_with_other_keys_are_pairs() {
        // Distinct str keys: an object.
        assert_eq!(min("82 a162 02 a161 01"), r#"{"b":2,"a":1}"#);
        // Any other key, in input order, alongside str ones.
        assert_eq!(min("82 01 a161 a131 a162"), r#"[[1,"a"],["1","b"]]"#);
        assert_eq!(min("81 92 01 02 03"), "[[[1,2],3]]");
        assert_eq!(min("81 c0 c3"), "[[null,true]]");
        assert_eq!(min("81 cb3ff8000000000000 00"), "[[1.5,0]]");
        assert_eq!(
            min("81 c403010203 01"),
            r#"[[{"type":"Buffer","data":[1,2,3]},1]]"#
        );
        assert_eq!(min("81 d6ff00000001 01"), r#"[["1970-01-01T00:00:01Z",1]]"#);
        assert_eq!(
            min("81 d40005 01"),
            r#"[[[0,{"type":"Buffer","data":[5]}],1]]"#
        );
        assert_eq!(min("81 81 01 02 03"), "[[[[1,2]],3]]");
        // A repeated str key keeps both entries.
        assert_eq!(min("82 a161 01 a161 02"), r#"[["a",1],["a",2]]"#);
        // Nested in objects and arrays.
        assert_eq!(min("81 a16d 81 01 02"), r#"{"m":[[1,2]]}"#);
        assert_eq!(min("80"), "{}");
        // Errors in keys and values still count, in input order.
        assert!(min("81 c1 01").starts_with("Could not decode MessagePack to JSON: "));
        assert!(min("81 01 c1").starts_with("Could not decode MessagePack to JSON: "));
    }

    #[test]
    fn extensions_are_generic() {
        let ext = |t: &str, bytes: &str| {
            format!(
                "[\n    {t},\n    {{\n        \"type\": \"Buffer\",\n        \"data\": [\n{bytes}\n        ]\n    }}\n]"
            )
        };
        // Type 0 has no meaning of its own, in any ext format.
        assert_eq!(from("d40005"), ext("0", "            5"));
        assert_eq!(from("c7010005"), ext("0", "            5"));
        assert_eq!(from("c701 05 07"), ext("5", "            7"));
        assert_eq!(from("c8 0001 7f 01"), ext("127", "            1"));
        assert_eq!(from("d4 80 01"), ext("-128", "            1"));
        assert_eq!(
            from("c700 02"),
            "[\n    2,\n    {\n        \"type\": \"Buffer\",\n        \"data\": []\n    }\n]"
        );
    }

    #[test]
    fn timestamps() {
        // Timestamp 32, 64 and 96, by length whatever the ext format.
        assert_eq!(from("d6 ff 00000001"), "\"1970-01-01T00:00:01Z\"");
        assert_eq!(from("c7 04 ff 00000001"), "\"1970-01-01T00:00:01Z\"");
        // 64: 30-bit nanoseconds over 34-bit seconds; full precision.
        assert_eq!(from("d7ff 77359400 00000001"), "\"1970-01-01T00:00:01.5Z\"");
        assert_eq!(
            from("d7ff ee6b27ff ffffffff"),
            "\"2514-05-30T01:53:03.999999999Z\""
        );
        // Nanoseconds past 999,999,999 (here 2^30 - 1) are invalid.
        assert_eq!(
            from("d7ff fffffffc ffffffff"),
            "Could not decode MessagePack to JSON: timestamp nanoseconds 1073741823 exceed 999999999 at byte 0"
        );
        assert_eq!(
            from("c70cff 00000000 ffffffff ffffffff"),
            "\"1969-12-31T23:59:59Z\""
        );
        assert_eq!(
            from("c70cff 00000001 00000000 00000001"),
            "\"1970-01-01T00:00:01.000000001Z\""
        );
        assert_eq!(
            from("c70cff 00000000 7fffffff ffffffff"),
            "\"+292277026596-12-04T15:30:07Z\""
        );
        // Any other length, or nanoseconds past 999,999,999, is an error.
        assert!(from("d5 ff 0001").starts_with("Could not decode MessagePack to JSON: "));
        assert_eq!(
            from("c8000cff 000000000000000000000001"),
            "\"1970-01-01T00:00:01Z\""
        );
        assert!(
            from("c70cff 3b9aca00 00000000 00000000")
                .starts_with("Could not decode MessagePack to JSON: ")
        );
    }

    #[test]
    fn decode_errors() {
        assert_eq!(
            from(""),
            "Could not decode MessagePack to JSON: Error: Could not parse"
        );
        assert_eq!(
            from("00 01"),
            "Could not decode MessagePack to JSON: trailing bytes after the object at byte 1"
        );
        assert_eq!(
            from("a5616263"),
            "Could not decode MessagePack to JSON: unexpected end of input in the object at byte 0"
        );
        assert_eq!(
            from("c1"),
            "Could not decode MessagePack to JSON: reserved byte 0xc1 at byte 0"
        );
        // Strings are UTF-8.
        assert!(from("d9 02 c3 28").starts_with("Could not decode MessagePack to JSON: "));
        // Errors come in input order.
        let bad_time = from("92 c703ff000001 c1");
        assert!(bad_time.contains("at byte 1"), "{bad_time}");
        let deep = format!("{}01", "91".repeat(MAX_DEPTH + 1));
        assert!(from(&deep).starts_with("Could not decode MessagePack to JSON: "));
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
        let bytes = small_stack(|| to(&json));
        assert_eq!(bytes, format!("{}01", "91".repeat(n)));
        assert!(small_stack(|| from(&bytes)).starts_with("[\n    [\n"));
        let tagged = format!("{}01", "81a0".repeat(n));
        assert!(small_stack(|| from(&tagged)).starts_with("{\n    \"\": {"));
        // Maps keyed by maps: two JSON arrays each.
        let keyed = format!("{}{}", "81".repeat(n), "01".repeat(n + 1));
        assert!(small_stack(|| from(&keyed)).starts_with("[\n    [\n        [\n"));
    }
}
