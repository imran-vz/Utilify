//! Type-Length-Value records as JSON.
//!
//! `utilify-tlv`'s fixed-size parser reads each record: a key of up to 8
//! bytes (or none, for Length-Value), then either a little-endian length
//! of 1 to 32 bytes or, with "Use BER", a BER definite length in any of
//! its forms, up to 126 subsequent octets (`LengthEncoding::Ber`, ITU-T
//! X.690 §8.1.3 as used by ISO/IEC 7816-4
//! §5.2). Fractional sizes are whole bytes (`ceil(size)`), as in CyberChef.
//! The JSON shape (`key`, `length`, `value` byte arrays) is CyberChef's.
//!
//! - With "Use BER", the indefinite form `80` is an error: X.690 §8.1.3.6
//!   allows it only for constructed encodings ended by end-of-contents
//!   octets, which a flat TLV stream has no notion of (ISO/IEC 7816-4
//!   §5.2 uses definite lengths only). The reserved `FF` (X.690
//!   §8.1.3.5 c) is an error too.
//! - Without "Use BER", a length size of 0 or less is an invalid argument:
//!   a record with no length field has no defined value.
//! - Keys wider than 8 bytes are sliced here (the library's tag is a
//!   `u64`), and little-endian length fields wider than the library's 32
//!   bytes are read here; a valid one has only zero bytes past the eighth.
//! - A record cut short is an error, as is a BER length of 2^31 or more
//!   that runs past the input.

use super::CATEGORY;
use super::js::{Js, Obj, json_dish};
use crate::{Arg, Args, Dish, OpError, Operation};
use utilify_tlv::ber::MAX_LENGTH_BYTES;
use utilify_tlv::fixed::{self, Endian, FixedOptions, LengthEncoding, MAX_FIELD_SIZE};
use utilify_tlv::{ErrorKind, Field};

pub(super) fn ops() -> Vec<&'static Operation> {
    vec![&PARSE_TLV]
}

pub static PARSE_TLV: Operation = Operation {
    name: "Parse TLV",
    category: CATEGORY,
    description: "Converts a Type-Length-Value (TLV) encoded string into a JSON object.  Can optionally include a Key / Type entry. \n\nTags: Key-Length-Value, KLV, Length-Value, LV",
    info_url: Some("https://wikipedia.org/wiki/Type-length-value"),
    args: &[
        Arg::number("Type/Key size", 1.0),
        Arg::number("Length size", 1.0),
        Arg::flag("Use BER", false),
    ],
    run: parse_tlv_op,
};

/// How many times `for (i = 0; i < n; i++)` runs.
fn iterations(n: f64) -> usize {
    if n > 0.0 {
        n.ceil().min(usize::MAX as f64) as usize
    } else {
        0
    }
}

/// How a record's length is stored.
#[derive(Clone, Copy)]
enum LengthField {
    /// A little-endian integer of this many bytes.
    Fixed(usize),
    /// A BER length, whatever the length size.
    Ber,
}

struct Record<'a> {
    key: &'a [u8],
    length: u64,
    value: &'a [u8],
    /// Offset just past the record.
    end: usize,
}

fn fail(kind: ErrorKind, offset: usize) -> OpError {
    OpError::new(format!("Could not parse TLV: {kind} at offset {offset}"))
}

fn parse_tlv_op(input: Dish, args: &Args) -> Result<Dish, OpError> {
    let (key_size, bytes_in_length, ber) = (args.num(0), args.num(1), args.flag(2));
    if key_size <= 0.0 && bytes_in_length <= 0.0 {
        return Err(OpError::new("Type or Length size must be greater than 0"));
    }
    // A key size of 0 or NaN means no key (`undefined` is left out).
    let has_key = key_size != 0.0 && !key_size.is_nan();
    let key_len = iterations(key_size);
    let field = match (ber, iterations(bytes_in_length)) {
        (true, _) => LengthField::Ber,
        (false, 0) => {
            return Err(OpError::new(
                "Length size must be greater than 0 unless Use BER is set",
            ));
        }
        (false, n) => LengthField::Fixed(n),
    };
    let input = input.bytes();
    let bytes = |b: &[u8]| Js::Arr(b.iter().map(|&b| Js::Num(f64::from(b))).collect());
    let mut data = vec![];
    let mut pos = 0;
    // Every record reads at least one byte, so this ends.
    while pos < input.len() {
        let record = record(&input, pos, key_len, field)?;
        let mut entry = Obj::new();
        if has_key {
            entry.insert("key".into(), bytes(record.key));
        }
        entry.insert("length".into(), Js::Num(record.length as f64));
        entry.insert("value".into(), bytes(record.value));
        data.push(Js::Obj(entry));
        pos = record.end;
    }
    json_dish(&Js::Arr(data))
}

/// The record at `start`, with a key of `key_len` bytes.
fn record(
    input: &[u8],
    start: usize,
    key_len: usize,
    field: LengthField,
) -> Result<Record<'_>, OpError> {
    let wide = matches!(field, LengthField::Fixed(n) if n > MAX_FIELD_SIZE);
    let lib_key = if key_len <= 8 && !wide { key_len } else { 0 };
    let at = start
        .checked_add(key_len - lib_key)
        .filter(|&at| at <= input.len())
        .ok_or_else(|| fail(ErrorKind::UnexpectedEnd { field: Field::Tag }, input.len()))?;
    let own_key = &input[start..at];
    let opts = match field {
        LengthField::Fixed(n) if wide => return wide_length(input, start, own_key, n),
        LengthField::Fixed(n) => FixedOptions::new(lib_key, n).endian(Endian::Little),
        LengthField::Ber => {
            FixedOptions::new(lib_key, MAX_LENGTH_BYTES).length_encoding(LengthEncoding::Ber)
        }
    }
    .max_value_len(usize::MAX);
    let rest = &input[at..];
    let err = match fixed::iter(rest, &opts).next() {
        Some(Ok(r)) => {
            return Ok(Record {
                key: if lib_key > 0 { r.tag_bytes() } else { own_key },
                length: r.length() as u64,
                value: r.value(),
                end: at + r.raw().len(),
            });
        }
        Some(Err(e)) => e,
        None => {
            return Err(fail(
                ErrorKind::UnexpectedEnd {
                    field: Field::Length,
                },
                input.len(),
            ));
        }
    };
    let offset = err.offset().map_or(at, |o| at.saturating_add(o as usize));
    Err(fail(err.kind(), offset))
}

/// A record whose little-endian length field is wider than the library's
/// [`MAX_FIELD_SIZE`]; its key, already read, ends where the field starts.
fn wide_length<'a>(
    input: &'a [u8],
    start: usize,
    key: &'a [u8],
    n: usize,
) -> Result<Record<'a>, OpError> {
    let at = start + key.len();
    let field = input
        .get(at..)
        .and_then(|rest| rest.get(..n))
        .ok_or_else(|| {
            fail(
                ErrorKind::UnexpectedEnd {
                    field: Field::Length,
                },
                input.len(),
            )
        })?;
    let (low, high) = field.split_at(8);
    if high.iter().any(|&b| b != 0) {
        return Err(fail(ErrorKind::LengthOverflow, at));
    }
    let length = low
        .iter()
        .rev()
        .fold(0u64, |acc, &b| (acc << 8) | u64::from(b));
    let value_at = at + n;
    let available = input.len() - value_at;
    let value = usize::try_from(length)
        .ok()
        .and_then(|len| input[value_at..].get(..len))
        .ok_or_else(|| {
            let available = available as u64;
            fail(ErrorKind::TruncatedValue { length, available }, start)
        })?;
    Ok(Record {
        key,
        length,
        value,
        end: value_at + value.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ArgValue;

    fn run(input: &[u8], key: f64, len: f64, ber: bool) -> String {
        let args = [ArgValue::Num(key), ArgValue::Num(len), ArgValue::Bool(ber)];
        match PARSE_TLV.apply(Dish::Bytes(input.to_vec()), &args) {
            Ok(d) => d.into_text(),
            Err(e) => e.0,
        }
    }

    fn compact(s: &str) -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect()
    }

    #[test]
    fn parses_records() {
        assert_eq!(
            compact(&run(b"\x01\x02ab", 1.0, 1.0, false)),
            r#"[{"key":[1],"length":2,"value":[97,98]}]"#
        );
        // Little-endian multi-byte lengths.
        assert_eq!(
            compact(&run(b"\x02\x00ab", 0.0, 2.0, false)),
            r#"[{"length":2,"value":[97,98]}]"#
        );
        assert_eq!(
            compact(&run(b"\x07\x02\x00ab\x08\x00\x00", 1.0, 2.0, false)),
            r#"[{"key":[7],"length":2,"value":[97,98]},{"key":[8],"length":0,"value":[]}]"#
        );
        // Fractional sizes read whole bytes.
        assert_eq!(
            compact(&run(b"\x01\x02\x01\x00a", 1.5, 1.2, false)),
            r#"[{"key":[1,2],"length":1,"value":[97]}]"#
        );
        // A negative key size gives an empty key.
        assert_eq!(
            compact(&run(b"\x01a", -1.0, 1.0, false)),
            r#"[{"key":[],"length":1,"value":[97]}]"#
        );
        // Without BER a record needs a length field.
        for len in [0.0, -1.0] {
            assert_eq!(
                run(b"\x01\x02", 1.0, len, false),
                "Length size must be greater than 0 unless Use BER is set"
            );
        }
        // With BER the length size is not used.
        assert_eq!(
            compact(&run(b"\x01\x01a", 1.0, 0.0, true)),
            r#"[{"key":[1],"length":1,"value":[97]}]"#
        );
        // Keys over 8 bytes and length fields over 32 bytes are read here.
        let mut wide = vec![0xff; 9];
        wide.push(1);
        wide.extend([0; 39]);
        wide.push(b'a');
        assert_eq!(
            compact(&run(&wide, 9.0, 40.0, false)),
            format!(
                r#"[{{"key":[{}],"length":1,"value":[97]}}]"#,
                ["255"; 9].join(",")
            )
        );
        let mut overflow = vec![1; 40];
        overflow.push(1);
        assert!(run(&overflow, 0.0, 40.0, false).starts_with("Could not parse TLV: "));
        assert_eq!(
            compact(&run(b"\x01\x82\x00\x01a", 1.0, 1.0, true)),
            r#"[{"key":[1],"length":1,"value":[97]}]"#
        );
        // X.690 8.1.3.5: long forms may use more octets than needed.
        assert_eq!(
            compact(&run(b"\x01\x83\x00\x00\x01a", 1.0, 1.0, true)),
            r#"[{"key":[1],"length":1,"value":[97]}]"#
        );
        // Up to 126 subsequent octets (X.690 8.1.3.5 b), zero-padded.
        let mut longest = vec![1, 0xfe];
        longest.extend([0; 125]);
        longest.extend([1, b'a']);
        assert_eq!(
            compact(&run(&longest, 1.0, 1.0, true)),
            r#"[{"key":[1],"length":1,"value":[97]}]"#
        );
        // The indefinite form (X.690 8.1.3.6) has no place in flat TLV.
        for (input, key) in [(&b"\x01\x80\x02\x01a"[..], 1.0), (b"\x80\x01a", 0.0)] {
            let out = run(input, key, 4.0, true);
            assert!(out.starts_with("Could not parse TLV: "), "{out}");
        }
        assert_eq!(run(b"", 1.0, 1.0, false), "[]");
        // A truncated record is an error with the library's layouts.
        assert!(run(b"\x01\x05ab", 1.0, 1.0, false).starts_with("Could not parse TLV: "));
        assert!(run(b"\x01", 1.0, 2.0, false).starts_with("Could not parse TLV: "));
        assert!(run(b"\x05ab", 0.0, 1.0, false).starts_with("Could not parse TLV: "));
        assert!(run(b"\x01\x82\x01", 1.0, 1.0, true).starts_with("Could not parse TLV: "));
        assert!(run(b"\x01\xff", 1.0, 1.0, true).starts_with("Could not parse TLV: "));
        assert_eq!(
            run(b"ab", 0.0, 0.0, false),
            "Type or Length size must be greater than 0"
        );
    }
}
