//! Avro to JSON: Avro object container files (Avro 1.12 specification,
//! "Object Container Files") read strictly with `utilify-avro`
//! (`DecodeOptions::SPEC`, null and deflate codecs as CyberChef offers).
//!
//! The library follows the specification: `int` values outside 32 bits,
//! boolean bytes other than 0 and 1 ("Primitive Types": a boolean is a
//! single byte, 0 or 1), invalid UTF-8 in strings, block byte sizes that
//! don't match the block's data, sync marker mismatches and files cut short
//! are errors, and any schema the specification allows is accepted (a
//! union field default may match any branch, as Avro 1.12 "Complex Types /
//! Records" allows; long defaults may use the whole 64-bit range). Every
//! record is output, `null` ones included. Errors are the library's,
//! after "Error parsing Avro file: ".
//!
//! Longs are exact. A `long` (64-bit signed) within ±2^53, where an
//! ECMAScript number (an IEEE 754 double) holds every integer and prints
//! its exact digits, is a JSON number; one beyond is a JSON string of its
//! exact decimal digits, such as `"9007199254740993"`, rather than a
//! rounded number.
//!
//! The rest of the layout is CyberChef's presentation (avsc's plain-JSON
//! shapes, not the specification's "JSON Encoding", which the op does not
//! claim to produce): unions wrapped in a `{"<branch name>": value}` object
//! only when their branches can't be told apart by JSON type, logical types
//! read as their underlying type (allowed: "Logical Types" lets
//! implementations ignore them), maps and records as JavaScript objects
//! (array-index keys first, a repeated map key keeping its first place and
//! last value), and bytes and fixed as Node `Buffer` objects
//! (`{"type": "Buffer", "data": [...]}`).

use super::CATEGORY;
use super::js::{self, Js, Obj, with_big_stack};
use crate::{Arg, Args, Dish, OpError, Operation};
use utilify_avro::{Datum, DecodeOptions, Schema, SchemaOptions, Type, TypeId};

pub(super) fn ops() -> Vec<&'static Operation> {
    vec![&AVRO_TO_JSON]
}

pub static AVRO_TO_JSON: Operation = Operation {
    name: "Avro to JSON",
    category: CATEGORY,
    description: "Converts Avro encoded data into JSON.",
    info_url: Some("https://wikipedia.org/wiki/Apache_Avro"),
    args: &[Arg::flag("Force Valid JSON", true)],
    run: avro_to_json,
};

/// The prefix of every error.
const PARSE_ERROR: &str = "Error parsing Avro file";

/// Strict decoding, with resource limits raised for deep schemas and
/// files of many empty records.
const OPTIONS: DecodeOptions = DecodeOptions::SPEC
    .schema_options(SchemaOptions::SPEC.max_depth(4096))
    .max_zero_size_items(1 << 26);

fn avro_to_json(input: Dish, args: &Args) -> Result<Dish, OpError> {
    let data = input.bytes();
    if data.is_empty() {
        return Err(OpError::new("Please provide an input."));
    }
    let force = args.flag(0);
    // Values nest as deep as the decoder allows; converting, printing and
    // dropping them recurses.
    with_big_stack(|| {
        let records = decode_container(&data).map_err(OpError::new)?;
        let out = if force {
            let value = match <[Js; 1]>::try_from(records) {
                Ok([one]) => one,
                Err(all) => Js::Arr(all),
            };
            js::stringify_pretty(&value)?.unwrap_or_default()
        } else {
            let mut out = String::new();
            for r in &records {
                out.push_str(&js::stringify_compact(r)?.unwrap_or_else(|| "undefined".into()));
                out.push('\n');
            }
            out
        };
        Ok(Dish::Text(out))
    })
}

/// The records of a container file, or the error message.
fn decode_container(data: &[u8]) -> Result<Vec<Js>, String> {
    let container =
        utilify_avro::read_container(data, &OPTIONS).map_err(|e| format!("{PARSE_ERROR}: {e}"))?;
    let schema = &container.schema;
    let root = schema.root();
    container
        .records
        .iter()
        .map(|d| {
            to_js(schema, root, d)
                .ok_or_else(|| format!("{PARSE_ERROR}: a value does not match the schema"))
        })
        .collect()
}

/// Longs within ±2^53 are JSON numbers.
const MAX_EXACT: u64 = 1 << 53;

/// A long as JSON: a number within ±2^53, where a double holds every
/// integer and JavaScript prints its exact digits; else its exact decimal
/// digits as a string.
fn long(n: i64) -> Js {
    if n.unsigned_abs() <= MAX_EXACT {
        Js::Num(n as f64)
    } else {
        Js::Str(n.to_string())
    }
}

/// What an unwrapped union can tell apart by value.
fn bucket(t: &Type) -> &'static str {
    match t {
        Type::Int | Type::Long | Type::Float | Type::Double => "number",
        Type::Bytes | Type::Fixed(_) => "buffer",
        Type::String | Type::Enum(_) => "string",
        Type::Map(_) | Type::Record(_) => "object",
        t => t.type_name(),
    }
}

/// avsc's name for a union branch: the fullname of a named type.
fn branch_name(t: &Type) -> String {
    t.name()
        .map_or_else(|| t.type_name().to_string(), |n| n.fullname())
}

/// A decoded value as JSON; `None` for a value that doesn't fit its type
/// (which the library never produces).
fn to_js(schema: &Schema, ty: TypeId, d: &Datum) -> Option<Js> {
    let num = |n: f64| Some(Js::Num(n));
    match (schema.get(ty), d) {
        (Type::Null, Datum::Null) => Some(Js::Null),
        (Type::Boolean, Datum::Boolean(b)) => Some(Js::Bool(*b)),
        (Type::Int, Datum::Int(n)) => num(f64::from(*n)),
        (Type::Long, Datum::Long(n)) => Some(long(*n)),
        (Type::Float, Datum::Float(n)) => num(f64::from(*n)),
        (Type::Double, Datum::Double(n)) => num(*n),
        (Type::Bytes, Datum::Bytes(b)) | (Type::Fixed(_), Datum::Fixed(b)) => Some(Js::buffer(b)),
        (Type::String, Datum::String(s)) => Some(Js::str(s.clone())),
        (Type::Enum(e), Datum::Enum(i)) => e.symbols.get(*i as usize).cloned().map(Js::Str),
        (Type::Array(items), Datum::Array(a)) => a
            .iter()
            .map(|x| to_js(schema, *items, x))
            .collect::<Option<_>>()
            .map(Js::Arr),
        (Type::Map(values), Datum::Map(entries)) => {
            let mut o = Obj::new();
            for (k, x) in entries {
                o.insert(k.clone(), to_js(schema, *values, x)?);
            }
            Some(Js::Obj(o))
        }
        (Type::Record(r), Datum::Record(values)) => {
            let mut o = Obj::new();
            for (f, x) in r.fields.iter().zip(values) {
                o.insert(f.name.clone(), to_js(schema, f.ty, x)?);
            }
            Some(Js::Obj(o))
        }
        (Type::Union(branches), Datum::Union(i, inner)) => {
            let &b = branches.get(*i as usize)?;
            let v = to_js(schema, b, inner)?;
            let mut seen = std::collections::HashSet::new();
            let wrapped = !branches.iter().all(|&b| seen.insert(bucket(schema.get(b))));
            Some(match schema.get(b) {
                Type::Null => Js::Null,
                t if wrapped => {
                    let mut o = Obj::new();
                    o.insert(branch_name(t), v);
                    Js::Obj(o)
                }
                _ => v,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ArgValue;

    fn run(input: &[u8], force: bool) -> String {
        match AVRO_TO_JSON.apply(Dish::Bytes(input.to_vec()), &[ArgValue::Bool(force)]) {
            Ok(d) => d.into_text(),
            Err(e) => e.0,
        }
    }

    fn zigzag(out: &mut Vec<u8>, n: i64) {
        let mut v = ((n << 1) ^ (n >> 63)) as u64;
        while v >= 0x80 {
            out.push((v as u8) | 0x80);
            v >>= 7;
        }
        out.push(v as u8);
    }

    /// A container file with sync marker `07 x 16`, built like avsc does.
    fn container(schema: &str, codec: Option<&str>, blocks: &[(i64, &[u8])]) -> Vec<u8> {
        let mut out = b"Obj\x01".to_vec();
        let mut meta = vec![("avro.schema", schema.as_bytes())];
        if let Some(c) = codec {
            meta.push(("avro.codec", c.as_bytes()));
        }
        zigzag(&mut out, meta.len() as i64);
        for (k, v) in meta {
            zigzag(&mut out, k.len() as i64);
            out.extend_from_slice(k.as_bytes());
            zigzag(&mut out, v.len() as i64);
            out.extend_from_slice(v);
        }
        out.push(0);
        out.extend_from_slice(&[7; 16]);
        for (count, data) in blocks {
            zigzag(&mut out, *count);
            zigzag(&mut out, data.len() as i64);
            out.extend_from_slice(data);
            out.extend_from_slice(&[7; 16]);
        }
        out
    }

    fn is_error(out: &str) -> bool {
        out.starts_with("Error parsing Avro file: ")
    }

    #[test]
    fn empty_and_truncated_input() {
        assert_eq!(run(b"", true), "Please provide an input.");
        let file = container("\"int\"", None, &[(3, &[2, 4, 6])]);
        assert_eq!(run(&file, true), "[\n    1,\n    2,\n    3\n]");
        // A file cut short, in the header or in a block, is malformed.
        assert_eq!(
            run(&file[..3], true),
            "Error parsing Avro file: unexpected end of input at offset 3"
        );
        assert!(is_error(&run(&file[..file.len() - 1], true)));
        assert!(is_error(&run(&file[..file.len() - 1], false)));
        let mut bad = file.clone();
        bad[0] = b'P';
        assert_eq!(
            run(&bad, true),
            "Error parsing Avro file: not an Avro object container file (bad magic)"
        );
        let mut sync = file.clone();
        *sync.last_mut().unwrap() ^= 1;
        assert!(run(&sync, true).contains("sync marker mismatch"));
    }

    #[test]
    fn records_across_blocks() {
        let file = container("\"int\"", None, &[(2, &[2, 4]), (1, &[4])]);
        assert_eq!(run(&file, true), "[\n    1,\n    2,\n    2\n]");
        assert_eq!(run(&file, false), "1\n2\n2\n");
        // The block's byte size must match its records ("Object Container
        // Files": the size of the serialized objects in the block).
        let extra = container("\"int\"", None, &[(2, &[2, 4, 6])]);
        assert!(is_error(&run(&extra, true)));
        // Strings are UTF-8 ("Primitive Types").
        let one = container("\"string\"", None, &[(1, &[4, 0xff, 0x41])]);
        assert!(is_error(&run(&one, true)));
        // Null records are records like any other.
        let nulls = container("[\"null\",\"int\"]", None, &[(3, &[2, 6, 0, 2, 4])]);
        assert_eq!(run(&nulls, false), "3\nnull\n2\n");
        assert_eq!(
            run(&container("\"null\"", None, &[(2, &[])]), false),
            "null\nnull\n"
        );
    }

    #[test]
    fn longs_are_exact() {
        let long = |n: i64| {
            let mut data = vec![];
            zigzag(&mut data, n);
            run(&container("\"long\"", None, &[(1, &data)]), true)
        };
        assert_eq!(long(9_007_199_254_740_990), "9007199254740990");
        assert_eq!(long(-9_007_199_254_740_990), "-9007199254740990");
        assert_eq!(long(9_007_199_254_740_992), "9007199254740992");
        // Past 2^53, values a double can't hold are exact decimal strings.
        assert_eq!(long(9_007_199_254_740_993), "\"9007199254740993\"");
        assert_eq!(long(i64::MAX), "\"9223372036854775807\"");
        assert_eq!(long(i64::MIN), "\"-9223372036854775808\"");
        assert_eq!(long(1 << 60), "\"1152921504606846976\"");
    }

    #[test]
    fn numbers_per_spec() {
        // An int is 32 bits; this varint is 2^63 - 1 zig-zagged.
        let big = [0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01];
        assert!(is_error(&run(
            &container("\"int\"", None, &[(1, &big)]),
            true
        )));
        assert_eq!(
            run(&container("\"long\"", None, &[(1, &big)]), true),
            "\"9223372036854775807\""
        );
        // A varint longer than 64 bits is an error for a long too.
        let over = [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f];
        assert!(is_error(&run(
            &container("\"long\"", None, &[(1, &over)]),
            true
        )));
        // A boolean is one byte, 0 or 1.
        assert_eq!(
            run(&container("\"boolean\"", None, &[(2, &[0, 1])]), false),
            "false\ntrue\n"
        );
        assert!(is_error(&run(
            &container("\"boolean\"", None, &[(1, &[2])]),
            true
        )));
        let f = 1.1f32.to_le_bytes();
        assert_eq!(
            run(&container("\"float\"", None, &[(1, &f)]), true),
            "1.100000023841858"
        );
    }

    #[test]
    fn complex_types() {
        let record = r#"{"type":"record","name":"a.b","fields":[{"name":"x","type":{"type":"fixed","name":"F","size":1}},{"name":"y","type":"a.F"},{"name":"m","type":{"type":"map","values":"int"}},{"name":"e","type":{"type":"enum","name":"E","symbols":["P","Q"]}}]}"#;
        let data = [2, 3, 4, 2, 0x31, 4, 2, 0x30, 6, 0, 2];
        assert_eq!(
            run(&container(record, None, &[(1, &data)]), false),
            "{\"x\":{\"type\":\"Buffer\",\"data\":[2]},\"y\":{\"type\":\"Buffer\",\"data\":[3]},\"m\":{\"0\":3,\"1\":2},\"e\":\"Q\"}\n"
        );
        // Ambiguous unions are wrapped in their branch name.
        assert_eq!(
            run(
                &container("[\"int\",\"long\"]", None, &[(1, &[2, 2])]),
                true
            ),
            "{\n    \"long\": 1\n}"
        );
        let named = r#"[{"type":"record","name":"A","fields":[]},{"type":"record","name":"B","fields":[]}]"#;
        assert_eq!(
            run(&container(named, None, &[(1, &[2])]), false),
            "{\"B\":{}}\n"
        );
        let enums = r#"{"type":"enum","name":"E","symbols":["A","B"]}"#;
        assert_eq!(
            run(&container(enums, None, &[(2, &[0, 2])]), false),
            "\"A\"\n\"B\"\n"
        );
        // Enum symbols are names ("Complex Types / Enums").
        let bad = r#"{"type":"enum","name":"E","symbols":["A",true]}"#;
        assert!(is_error(&run(&container(bad, None, &[(1, &[0])]), false)));
        let array = r#"{"type":"array","items":"int"}"#;
        assert_eq!(
            run(&container(array, None, &[(1, &[3, 4, 2, 4, 0])]), false),
            "[1,2]\n"
        );
    }

    #[test]
    fn map_keys_keep_javascript_order() {
        // Keys in encoding order, array indexes first, a repeat keeping its
        // first place and last value.
        let schema = r#"{"type":"map","values":"int"}"#;
        let mut data = vec![10];
        for (k, v) in [("b", 1), ("a", 2), ("2", 3), ("1", 4), ("b", 5)] {
            zigzag(&mut data, k.len() as i64);
            data.extend_from_slice(k.as_bytes());
            zigzag(&mut data, v);
        }
        data.push(0);
        assert_eq!(
            run(&container(schema, None, &[(1, &data)]), false),
            "{\"1\":4,\"2\":3,\"b\":5,\"a\":2}\n"
        );
    }

    #[test]
    fn deep_values_do_not_overflow() {
        let mut schema = String::from("\"int\"");
        for i in 0..1000 {
            schema = format!(
                r#"{{"type":"record","name":"R{i}","fields":[{{"name":"f","type":{schema}}}]}}"#
            );
        }
        let out = run(&container(&schema, None, &[(1, &[2])]), false);
        assert_eq!(
            out,
            format!("{}1{}\n", "{\"f\":".repeat(1000), "}".repeat(1000))
        );
    }

    #[test]
    fn invalid_schemas() {
        for schema in [
            r#"{"type":"record","name":"R","fields":[{"name":"x","type":"int"},{"name":"x","type":"int"}]}"#,
            r#"{"type":"record","name":"R","fields":[{"name":"x","type":"int","default":1.5}]}"#,
            r#"{"type":"record","name":"int","fields":[]}"#,
            r#"["int","int"]"#,
            r#"{"type":"R"}"#,
            r#"{"type":"fixed","size":-1}"#,
            "nope",
        ] {
            let out = run(&container(schema, None, &[(1, &[2, 2])]), true);
            assert!(is_error(&out), "{schema}: {out}");
        }
        let defaults = r#"{"type":"record","name":"R","fields":[{"name":"x","type":{"type":"record","name":"S","fields":[{"name":"q","type":"int","default":3}]},"default":{}}]}"#;
        assert_eq!(
            run(&container(defaults, None, &[(1, &[2])]), false),
            "{\"x\":{\"q\":1}}\n"
        );
        // Avro 1.12 lets a union default match any branch.
        let union = r#"{"type":"record","name":"R","fields":[{"name":"x","type":["int","null"],"default":null},{"name":"y","type":"long","default":9223372036854775807}]}"#;
        assert_eq!(
            run(&container(union, None, &[(1, &[2, 4])]), false),
            "{\"x\":null,\"y\":2}\n"
        );
        assert!(is_error(&run(
            &container("\"int\"", Some("snappy"), &[(1, &[2])]),
            true
        )));
    }

    #[test]
    fn deflate_codec() {
        let packed = miniz_oxide::deflate::compress_to_vec(&[2, 4], 6);
        let file = container("\"int\"", Some("deflate"), &[(2, &packed)]);
        assert_eq!(run(&file, true), "[\n    1,\n    2\n]");
        let cut = container("\"int\"", Some("deflate"), &[(1, &packed[..1])]);
        assert!(is_error(&run(&cut, true)));
        assert!(is_error(&run(
            &container("\"int\"", Some("deflate"), &[(1, &[])]),
            true
        )));
    }
}
