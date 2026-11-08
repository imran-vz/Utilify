//! Rison, JSON's URI-friendly cousin: `(a:!t,b:!(1,'x y'))`.
//!
//! `utilify-rison` encodes and decodes per the Rison specification
//! (<https://github.com/Nanonid/rison>), with nothing rewritten around it:
//!
//! - ids (grammar: `id = idstart idchar*`) are ASCII letters and digits,
//!   `-_./~` and non-ASCII characters, not starting with `-` or a digit.
//!   Encode writes other strings quoted (`'a=b'`), and Decode rejects bare
//!   text outside that grammar (`a=b`), where CyberChef's library allowed
//!   any character but `` '!:(),*@$`` in both directions;
//! - numbers follow the grammar (`int frac? exp?`, no leading zeros, a
//!   lowercase `e`, no `+`). Rison has no literal for infinity, and it
//!   expresses only JSON's data, so a number beyond a double's range
//!   (`1e999`) fails to decode ("number out of range") and an infinite
//!   input number (JSON `1e999` parses to Infinity, ECMA-262) fails to
//!   encode, where CyberChef wrote and read `!n`, turning it into `null`;
//! - object keys are encoded sorted, as the specification asks ("object
//!   keys should be lexically sorted when encoding"); decoded objects keep
//!   the input's key order (read through the serde deserializer, as its
//!   `Value` sorts), and every key, `__proto__` too, is data;
//! - "Encode URI" is `EncodeOptions::uri`: the specification's "Interaction
//!   with URI %-encoding" rule (keep `,:@$/` and RFC 3986 unreserved
//!   characters plus `!*'()`, a space as `+`, percent-encode every other
//!   UTF-8 byte), applied to the whole text, where `rison.quote` undid only
//!   the first of each escape.
//!
//! What stays CyberChef's: the option names and the messages for "Encode
//! Object"/"Encode Array" given the wrong shape. Decoding gives up past
//! 2500 nesting levels (a resource limit; the specification sets none).

use super::CATEGORY;
use super::js::{Js, Obj, json_dish, json_input, with_big_stack};
use crate::{Arg, Dish, OpError, Operation};
use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use utilify_rison::{DecodeOptions, EncodeOptions, Form, Map, Number, Value};

pub(super) fn ops() -> Vec<&'static Operation> {
    vec![&RISON_ENCODE, &RISON_DECODE]
}

const DESCRIPTION: &str = "Rison, a data serialization format optimized for compactness in URIs. Rison is a slight variation of JSON that looks vastly superior after URI encoding. Rison still expresses exactly the same set of data structures as JSON, so data can be translated back and forth without loss or guesswork.";

pub static RISON_ENCODE: Operation = Operation {
    name: "Rison Encode",
    category: CATEGORY,
    description: DESCRIPTION,
    info_url: Some("https://github.com/Nanonid/rison"),
    args: &[Arg::choice(
        "Encode Option",
        &["Encode", "Encode Object", "Encode Array", "Encode URI"],
    )],
    run: |input, args| {
        // Parsing, converting and dropping the value recurse per level.
        with_big_stack(|| {
            let v = json_input(&input)?;
            // The rison library's own errors, which CyberChef shows prefixed.
            let thrown = |m: &str| OpError::new(format!("Rison Encode - {m}"));
            let (form, uri) = match args.str(0) {
                "Encode" => (Form::Rison, false),
                "Encode Object" if !matches!(v, Js::Obj(_)) => {
                    return Err(thrown("rison.encode_object expects an object argument"));
                }
                "Encode Object" => (Form::ORison, false),
                "Encode Array" if !matches!(v, Js::Arr(_)) => {
                    return Err(thrown("rison.encode_array expects an array argument"));
                }
                "Encode Array" => (Form::ARison, false),
                "Encode URI" => (Form::Rison, true),
                _ => return Err(OpError::new("Invalid encode option")),
            };
            let value = to_value(&v).map_err(|m| thrown(&m))?;
            // Encoding is iterative, so no depth limit is needed.
            EncodeOptions::SPEC
                .form(form)
                .uri(uri)
                .max_depth(usize::MAX)
                .encode(&value)
                .map(Dish::Text)
                .map_err(|e| thrown(&e.to_string()))
        })
    },
};

/// The value to encode; Rison holds neither `undefined` nor non-finite
/// numbers.
fn to_value(v: &Js) -> Result<Value, String> {
    Ok(match v {
        Js::Undefined => return Err("rison can't encode the undefined value".into()),
        Js::Null => Value::Null,
        Js::Bool(b) => Value::Bool(*b),
        Js::Num(n) => Value::Number(
            Number::from_f64(*n)
                .ok_or_else(|| utilify_rison::ErrorKind::NonFiniteNumber.to_string())?,
        ),
        Js::Str(s) => Value::String(s.clone()),
        Js::Arr(items) => Value::Array(items.iter().map(to_value).collect::<Result<_, _>>()?),
        Js::Obj(o) => {
            let mut m = Map::new();
            for (k, v) in o.iter() {
                m.insert(k.into_owned(), to_value(v)?);
            }
            Value::Object(m)
        }
    })
}

pub static RISON_DECODE: Operation = Operation {
    name: "Rison Decode",
    category: CATEGORY,
    description: DESCRIPTION,
    info_url: Some("https://github.com/Nanonid/rison"),
    args: &[Arg::editable(
        "Decode Option",
        &[
            ("Decode", "Decode"),
            ("Decode Object", "Decode Object"),
            ("Decode Array", "Decode Array"),
        ],
    )],
    run: |input, args| {
        let form = match args.str(0) {
            "Decode" => Form::Rison,
            "Decode Object" => Form::ORison,
            "Decode Array" => Form::ARison,
            _ => return Err(OpError::new("Invalid Decode option")),
        };
        let src = input.text();
        let opts = DecodeOptions::SPEC
            .form(form)
            .max_depth(MAX_DEPTH)
            .recursion_limit(MAX_DEPTH);
        // The deserializer, building the value and printing it recurse.
        with_big_stack(|| {
            let v = utilify_rison::from_str_with::<RisonJs>(&src, &opts)
                .map_err(|e| OpError::new(format!("Rison Decode - {e}")))?;
            json_dish(&v.0)
        })
    },
};

/// Nesting deeper than this gives up (the specification sets no limit;
/// this bounds the work and the stack).
const MAX_DEPTH: usize = 2500;

/// A decoded value, built from the deserializer's events so objects keep
/// their keys in input order, every key, `__proto__` too, as data.
struct RisonJs(Js);

impl<'de> Deserialize<'de> for RisonJs {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(JsVisitor).map(RisonJs)
    }
}

struct JsVisitor;

impl<'de> Visitor<'de> for JsVisitor {
    type Value = Js;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("a Rison value")
    }

    fn visit_bool<E>(self, b: bool) -> Result<Js, E> {
        Ok(Js::Bool(b))
    }

    fn visit_unit<E>(self) -> Result<Js, E> {
        Ok(Js::Null)
    }

    fn visit_u64<E>(self, n: u64) -> Result<Js, E> {
        Ok(Js::Num(n as f64))
    }

    fn visit_i64<E>(self, n: i64) -> Result<Js, E> {
        Ok(Js::Num(n as f64))
    }

    fn visit_u128<E>(self, n: u128) -> Result<Js, E> {
        Ok(Js::Num(n as f64))
    }

    fn visit_i128<E>(self, n: i128) -> Result<Js, E> {
        Ok(Js::Num(n as f64))
    }

    fn visit_f64<E>(self, n: f64) -> Result<Js, E> {
        Ok(Js::Num(n))
    }

    fn visit_str<E>(self, s: &str) -> Result<Js, E> {
        Ok(Js::str(s))
    }

    fn visit_string<E>(self, s: String) -> Result<Js, E> {
        Ok(Js::Str(s))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Js, A::Error> {
        let mut items = vec![];
        while let Some(RisonJs(v)) = seq.next_element()? {
            items.push(v);
        }
        Ok(Js::Arr(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Js, A::Error> {
        let mut o = Obj::new();
        while let Some((k, RisonJs(v))) = map.next_entry::<String, RisonJs>()? {
            o.insert(k, v);
        }
        Ok(Js::Obj(o))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ArgValue;

    fn run(op: &Operation, input: &str, option: &str) -> String {
        match op.apply(Dish::Text(input.into()), &[ArgValue::Str(option.into())]) {
            Ok(d) => d.into_text(),
            Err(e) => e.0,
        }
    }

    #[test]
    fn encodes_per_spec() {
        // The specification's examples.
        assert_eq!(
            run(
                &RISON_ENCODE,
                r#"{"a":0,"b":"foo","c":"23skidoo"}"#,
                "Encode"
            ),
            "(a:0,b:foo,c:'23skidoo')"
        );
        assert_eq!(
            run(
                &RISON_ENCODE,
                r#"["A","B",{"supportsObjects":true}]"#,
                "Encode Array"
            ),
            "A,B,(supportsObjects:!t)"
        );
        assert_eq!(
            run(
                &RISON_ENCODE,
                r#"{"b":[1e21,-0.5,null,false],"a":"x y!'","":""}"#,
                "Encode"
            ),
            "('':'',a:'x y!!!'',b:!(1e21,-0.5,!n,!f))"
        );
        // Only the spec's idchars go bare.
        assert_eq!(
            run(
                &RISON_ENCODE,
                r#"{"q":"a=b&c?","r":"a b","s":"-x","t":"é/~","u":"/common/document"}"#,
                "Encode"
            ),
            "(q:'a=b&c?',r:'a b',s:'-x',t:é/~,u:/common/document)"
        );
        assert_eq!(
            run(&RISON_ENCODE, r#"{"a":"x!y","b":"it's"}"#, "Encode Object"),
            "a:'x!!y',b:'it!'s'"
        );
        // URI encoding applies to every occurrence.
        assert_eq!(
            run(&RISON_ENCODE, r#"{"a":"1 2, 3"}"#, "Encode URI"),
            "(a:'1+2,+3')"
        );
        assert_eq!(
            run(&RISON_ENCODE, r#""a/b,c:d@e$f g h,i?é+%""#, "Encode URI"),
            "'a/b,c:d@e$f+g+h,i%3F%C3%A9%2B%25'"
        );
        // No literal for infinity.
        assert_eq!(
            run(&RISON_ENCODE, "[1e999]", "Encode Array"),
            "Rison Encode - Rison cannot represent NaN or Infinity"
        );
        assert_eq!(run(&RISON_ENCODE, "[-0,1e21]", "Encode Array"), "0,1e21");
        assert_eq!(
            run(&RISON_ENCODE, "[1]", "Encode Object"),
            "Rison Encode - rison.encode_object expects an object argument"
        );
        assert_eq!(
            run(&RISON_ENCODE, r#"{"__prototype__":null}"#, "Encode"),
            "(__prototype__:!n)"
        );
        let deep = format!("{}{}", "[".repeat(4900), "]".repeat(4900));
        let out = run(&RISON_ENCODE, &deep, "Encode");
        assert_eq!(out.len(), 3 * 4900);
    }

    #[test]
    fn decodes_per_spec() {
        assert_eq!(
            run(&RISON_DECODE, "!(1,-2.5e3,'a!!b',x,!t,!n)", "Decode"),
            "[\n    1,\n    -2500,\n    \"a!b\",\n    \"x\",\n    true,\n    null\n]"
        );
        assert_eq!(
            run(&RISON_DECODE, "a:1,b:!()", "Decode Object"),
            "{\n    \"a\": 1,\n    \"b\": []\n}"
        );
        // Keys keep their input order; `__proto__` is a key like any other.
        assert_eq!(
            run(&RISON_DECODE, "(b:1,a:2,'1':3,__proto__:6,b:4)", "Decode"),
            "{\n    \"1\": 3,\n    \"b\": 4,\n    \"a\": 2,\n    \"__proto__\": 6\n}"
        );
        assert_eq!(
            run(&RISON_DECODE, "(q:'a=b&c',p:/x/y.z~)", "Decode"),
            "{\n    \"q\": \"a=b&c\",\n    \"p\": \"/x/y.z~\"\n}"
        );
        assert_eq!(run(&RISON_DECODE, "", "Decode Array"), "[]");
        // Outside the grammar: ids with other characters, leading zeros,
        // `E`, `e+`, non-key values, numbers out of a double's range.
        for bad in [
            "", "!(1,,2)", "'a!", "(", "(!t:3)", "01", "1E3", "1e+3", "1-2", "(((((", "(q:a=b)",
            "a?", "1e999",
        ] {
            let out = run(&RISON_DECODE, bad, "Decode");
            assert!(out.starts_with("Rison Decode - "), "{bad}: {out}");
        }
        assert_eq!(
            run(&RISON_DECODE, "!(1e999)", "Decode"),
            "Rison Decode - number out of range at line 1 column 3 (byte 2)"
        );
        let deep = format!("{}{}", "!(".repeat(2400), ")".repeat(2400));
        assert!(run(&RISON_DECODE, &deep, "Decode").starts_with("[\n    [\n"));
        assert_eq!(run(&RISON_DECODE, "x", "Other"), "Invalid Decode option");
    }

    #[test]
    fn deep_input_fits_small_stacks() {
        let run = || {
            let deep = format!("{}{}", "[".repeat(5000), "]".repeat(5000));
            assert_eq!(run(&RISON_ENCODE, &deep, "Encode").len(), 3 * 5000);
            let deep = format!("{}{}", "!(".repeat(5000), ")".repeat(5000));
            assert!(run(&RISON_DECODE, &deep, "Decode").starts_with("Rison Decode - nesting"));
        };
        let thread = std::thread::Builder::new().stack_size(512 << 10).spawn(run);
        assert!(thread.unwrap().join().is_ok());
    }
}
