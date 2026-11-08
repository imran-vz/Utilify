//! Action Message Format (AMF0 and AMF3).
//!
//! Encoding converts the JSON with `utilify-amf`'s JSON mapping and writes
//! it per the specifications:
//!
//! - AMF 0: numbers (§2.2), booleans, `null`, anonymous objects (§2.5) and
//!   strict arrays (§2.12). Strings and keys are UTF-8 with byte lengths
//!   (§1.3.1); strings of more than 65535 bytes are long strings (§2.14),
//!   and a key of more than 65535 bytes, which no `UTF-8` field can hold,
//!   is an error.
//! - AMF 3: integral numbers in the 29-bit range are integers, the rest
//!   doubles (§3.6); arrays are dense arrays (§3.11); objects anonymous and
//!   dynamic (§3.12), or sealed when a key is empty (an empty name ends the
//!   dynamic members). Repeated strings and member names go by
//!   string-table reference (§1.3.2) and repeated traits by traits
//!   reference (§3.12), as those tables are by value; lengths beyond the
//!   U29 fields (§1.3.1) are errors.
//! - No object references: AMF 0 §2.9 and AMF 3 §2.2 reference the same
//!   object again, and JSON values have no identity, so a repeated value is
//!   written again in full.
//!
//! Text with lone surrogates (from `\u` escapes) has them as U+FFFD, as
//! everywhere the operations read JSON. The output decodes back to the
//! input JSON.
//!
//! CyberChef's `@astronautlabs/amf` differs and those differences are not
//! kept: it rejects non-ASCII AMF 0 strings, replaces repeated AMF 0
//! arrays and objects with references found by comparing their JSON, and in
//! AMF 3 rejects booleans, `null` and arrays and writes every number as a
//! double, every object sealed and nothing by reference.
//!
//! Decoding reads the value with `utilify-amf`'s `wire::Reader` per the
//! AMF 0 and AMF 3 specifications, and prints it as CyberChef's element
//! tree: one JSON object per value with its `marker` and wire fields
//! (`$value`, `length`, `stringOrReference`, `$lengthOrReference`,
//! `$objectTypeIndicator`, `$traits`...). What the tree holds is the
//! specification's reading:
//!
//! - AMF 0 strings are UTF-8 (§1.3.1; invalid UTF-8 is an error, NULs and
//!   BOMs are kept) and lengths count bytes; ECMA arrays (§2.10) read
//!   properties up to the object end (`_count` is shown as declared,
//!   `endKey`/`endMarker` as the `00 00 09` that ends them); dates (§2.13)
//!   show the milliseconds as sent and the `timeZone` field; references
//!   (§2.9) show their `index`. Reserved markers (MovieClip, RecordSet,
//!   object end outside an object) and unknown markers are errors.
//! - AMF 3 integers (§3.6) are 29-bit signed, and the fourth byte of a
//!   U29 holds 8 bits (§1.3.1). Objects (§3.12) list `_values` (one per
//!   sealed member, in order) before `_dynamicMembers`. `Vector.<Number>`
//!   holds doubles and `Vector.<T>` its `objectTypeName` before its
//!   `values` (§3.15); `Vector.<int>`/`<uint>` keep their elements' raw
//!   `bytes`. A string, object or traits reference (§1.3.2, §2.2, §3.12)
//!   is shown as its header and `$reference` index (string references
//!   also with the `$value` they resolve to); references to entries that
//!   don't exist yet are errors. Byte arrays (§3.14) show their bytes.
//!   Externalizable objects (§3.12) of the Flex classes
//!   (`flex.messaging.io.ArrayCollection`, `ArrayList`, `ObjectProxy`)
//!   show their body as `$externalized`; any other class's body has a
//!   class-defined format, so decoding it is an error.
//! - Truncated input, flag bytes other than 0 or 1, and nesting deeper
//!   than 5000 containers are errors carrying the byte offset.

use super::CATEGORY;
use super::js::{Js, Obj, json_dish, json_input, with_big_stack};
use crate::{Arg, Args, Dish, OpError, Operation};
use utilify_amf::wire::{
    Amf0Event, Amf3Event, Amf3String, EventKind, Reader, StringValue, Text, TraitsRef, VectorKind,
};
use utilify_amf::{DecodeOptions, EncodeOptions, JsonOptions, encode_from_json};
use utilify_js_json as json;

pub(super) fn ops() -> Vec<&'static Operation> {
    vec![&AMF_ENCODE, &AMF_DECODE]
}

const FORMATS: &[&str] = &["AMF0", "AMF3"];

const DESCRIPTION: &str = "Action Message Format (AMF) is a binary format used to serialize object graphs such as ActionScript objects and XML, or send messages between an Adobe Flash client and a remote service, usually a Flash Media Server or third party alternatives.";

pub static AMF_ENCODE: Operation = Operation {
    name: "AMF Encode",
    category: CATEGORY,
    description: DESCRIPTION,
    info_url: Some("https://wikipedia.org/wiki/Action_Message_Format"),
    args: &[Arg::choice_at("Format", FORMATS, 1)],
    run: amf_encode,
};

pub static AMF_DECODE: Operation = Operation {
    name: "AMF Decode",
    category: CATEGORY,
    description: DESCRIPTION,
    info_url: Some("https://wikipedia.org/wiki/Action_Message_Format"),
    args: &[Arg::choice_at("Format", FORMATS, 1)],
    run: amf_decode,
};

fn amf_encode(input: Dish, args: &Args) -> Result<Dish, OpError> {
    let options = if args.str(0) == "AMF0" {
        JsonOptions::AMF0
    } else {
        JsonOptions::AMF3
    };
    // Repeated AMF 3 strings (values, member names, class names) go by
    // string-table reference (§1.3.2) and repeated traits by traits
    // reference (§3.12): both tables are by value. Object references
    // (AMF 0 §2.9, AMF 3 §2.2) stand for the same object again, and JSON
    // has no identity, so every value is written inline.
    let options = options.encode_options(
        EncodeOptions::DEFAULT
            .string_references(true)
            .traits_references(true),
    );
    // JSON nests up to the parser's limit; the conversions are iterative,
    // but dropping the values recurses.
    with_big_stack(|| {
        let value = json_input(&input)?;
        let value = value.to_json_value()?.unwrap_or(json::Value::Null);
        let out = encode_from_json(&value, &options)
            .map_err(|e| OpError::new(format!("AMF Encode - {e}")))?;
        Ok(Dish::Bytes(out))
    })
}

// ---------------------------------------------------------------------------
// Decoding: `utilify-amf`'s wire reader, its events assembled into the
// element tree.

/// How deep containers may nest.
const MAX_DEPTH: usize = 5000;

/// What a container collects.
enum Kind {
    /// AMF 0 object, typed object or ECMA array: key, value pairs.
    Props,
    /// AMF 0 strict array, AMF 3 `Vector.<T>`, `Vector.<Number>`, or an
    /// externalizable body: values in order.
    List,
    /// The AMF 0 AVM+ switch: one AMF 3 value.
    AvmPlus,
    /// AMF 3 array: associative pairs, then dense values.
    Array,
    /// AMF 3 object: sealed values, then dynamic pairs.
    Object,
    /// AMF 3 `Vector.<int>` / `Vector.<uint>`: the elements' bytes.
    Bytes,
    /// AMF 3 dictionary: key, value pairs.
    Dictionary,
}

struct Frame {
    kind: Kind,
    obj: Obj,
    /// Array fields, in output order.
    lists: Vec<(&'static str, Vec<Js>)>,
    /// Fields after the lists.
    tail: Vec<(&'static str, Js)>,
    /// A key waiting for its value.
    key: Option<Js>,
    bytes: Vec<u8>,
}

impl Frame {
    fn new(kind: Kind, obj: Obj, lists: &[&'static str]) -> Frame {
        Frame {
            kind,
            obj,
            lists: lists.iter().map(|&n| (n, vec![])).collect(),
            tail: vec![],
            key: None,
            bytes: vec![],
        }
    }

    fn push(&mut self, list: usize, v: Js) {
        if let Some((_, items)) = self.lists.get_mut(list) {
            items.push(v);
        }
    }

    /// Take a finished value; `true` when that completes the frame.
    fn attach(&mut self, v: Js) -> bool {
        let pair = |first: &str, k: Js, v: Js| {
            let mut o = Obj::new();
            set(&mut o, first, k);
            set(&mut o, "value", v);
            Js::Obj(o)
        };
        match self.kind {
            Kind::Props => {
                let mut o = match self.key.take() {
                    Some(Js::Obj(o)) => o,
                    _ => Obj::new(),
                };
                set(&mut o, "value", v);
                self.push(0, Js::Obj(o));
            }
            Kind::List => self.push(0, v),
            Kind::AvmPlus => {
                set(&mut self.obj, "$value", v);
                return true;
            }
            Kind::Array | Kind::Object => {
                // Arrays: associative pairs first; objects: sealed values
                // first.
                let (keyed, plain) = match self.kind {
                    Kind::Array => (0, 1),
                    _ => (1, 0),
                };
                match self.key.take() {
                    Some(k) => self.push(keyed, pair("$key", k, v)),
                    None => self.push(plain, v),
                }
            }
            Kind::Dictionary => match self.key.take() {
                Some(k) => self.push(0, pair("key", k, v)),
                None => self.key = Some(v),
            },
            Kind::Bytes => {}
        }
        false
    }

    fn finish(self) -> Js {
        let mut obj = self.obj;
        for (name, items) in self.lists {
            set(&mut obj, name, Js::Arr(items));
        }
        if matches!(self.kind, Kind::Bytes) {
            set(&mut obj, "bytes", Js::typed_array(&self.bytes));
        }
        for (name, v) in self.tail {
            set(&mut obj, name, v);
        }
        Js::Obj(obj)
    }
}

fn num(n: impl Into<f64>) -> Js {
    Js::Num(n.into())
}

fn set(o: &mut Obj, key: &str, v: Js) {
    o.insert(key.into(), v);
}

fn text(t: Text<'_>) -> Js {
    Js::str(t.to_string_lossy().into_owned())
}

/// An element with its marker and fields.
fn element(marker: u8, fields: Vec<(&str, Js)>) -> Obj {
    let mut o = Obj::new();
    set(&mut o, "marker", num(marker));
    for (k, v) in fields {
        set(&mut o, k, v);
    }
    o
}

fn element_fields(fields: Vec<(&str, Js)>) -> Obj {
    let mut o = Obj::new();
    for (k, v) in fields {
        set(&mut o, k, v);
    }
    o
}

/// Start a container element.
fn open(
    stack: &mut Vec<Frame>,
    marker: u8,
    kind: Kind,
    fields: Vec<(&str, Js)>,
    lists: &[&'static str],
) -> Option<Js> {
    stack.push(Frame::new(kind, element(marker, fields), lists));
    None
}

/// An AMF 3 `UTF-8-vr` (§1.3.2): `{$lengthOrReference, $value}` inline,
/// `{$lengthOrReference, $reference, $value}` for a string reference
/// (resolved through the string table).
fn sor(reader: &Reader<'_>, s: &Amf3String<'_>) -> Js {
    let mut o = Obj::new();
    set(&mut o, "$lengthOrReference", num(s.header));
    match s.value {
        StringValue::Inline { text: t, .. } => set(&mut o, "$value", text(t)),
        StringValue::Reference(i) => {
            set(&mut o, "$reference", num(i));
            if let Some(t) = reader.resolve(s) {
                set(&mut o, "$value", text(t));
            }
        }
    }
    Js::Obj(o)
}

/// The field holding an AMF 3 complex value's U29 header.
fn header_field(marker: u8) -> &'static str {
    match marker {
        0x08 => "$indicator",
        0x09 => "$denseLengthOrReference",
        0x0a => "$objectTypeIndicator",
        _ => "$lengthOrReference",
    }
}

fn amf_decode(input: Dish, args: &Args) -> Result<Dish, OpError> {
    let bytes = input.bytes();
    let amf0 = args.str(0) == "AMF0";
    // Building, printing and dropping the tree recurses.
    with_big_stack(|| {
        let options = DecodeOptions::FLEX.max_depth(MAX_DEPTH);
        let reader = if amf0 {
            Reader::amf0(&bytes, &options)
        } else {
            Reader::amf3(&bytes, &options)
        };
        let tree =
            decode_tree(reader, &bytes).map_err(|e| OpError::new(format!("AMF Decode - {e}")))?;
        json_dish(&tree)
    })
}

/// The element tree of the one value `reader` reads.
fn decode_tree(mut reader: Reader<'_>, input: &[u8]) -> Result<Js, String> {
    let mut stack: Vec<Frame> = vec![];
    let mut result = None;
    while let Some(event) = reader.next() {
        let event = event.map_err(|e| e.to_string())?;
        let m = event.marker.unwrap_or_default();
        let leaf = |fields: Vec<(&str, Js)>| Some(Js::Obj(element(m, fields)));
        let done = match event.kind {
            EventKind::Amf0(e) => match e {
                Amf0Event::Number(n) => leaf(vec![("$value", num(n))]),
                Amf0Event::Boolean(b) => leaf(vec![("$value", Js::Bool(b != 0))]),
                Amf0Event::String(t) => {
                    leaf(vec![("length", num(t.len() as f64)), ("$value", text(t))])
                }
                Amf0Event::LongString(t) | Amf0Event::XmlDocument(t) => {
                    leaf(vec![("_length", num(t.len() as f64)), ("$value", text(t))])
                }
                Amf0Event::ObjectStart { .. } => {
                    open(&mut stack, m, Kind::Props, vec![], &["properties"])
                }
                Amf0Event::TypedObjectStart { class_name, .. } => open(
                    &mut stack,
                    m,
                    Kind::Props,
                    vec![
                        ("classNameLength", num(class_name.len() as f64)),
                        ("className", text(class_name)),
                    ],
                    &["properties"],
                ),
                Amf0Event::EcmaArrayStart { declared_count, .. } => {
                    open(
                        &mut stack,
                        m,
                        Kind::Props,
                        vec![("_count", num(declared_count))],
                        &["properties"],
                    );
                    if let Some(f) = stack.last_mut() {
                        f.tail = vec![("endKey", num(0)), ("endMarker", num(9))];
                    }
                    None
                }
                Amf0Event::StrictArrayStart { count, .. } => open(
                    &mut stack,
                    m,
                    Kind::List,
                    vec![("count", num(count))],
                    &["values"],
                ),
                Amf0Event::Key(t) => {
                    if let Some(f) = stack.last_mut() {
                        let mut o = Obj::new();
                        set(&mut o, "keyLength", num(t.len() as f64));
                        set(&mut o, "key", text(t));
                        f.key = Some(Js::Obj(o));
                    }
                    None
                }
                Amf0Event::Reference(i) => leaf(vec![("index", num(i))]),
                Amf0Event::Date { millis, time_zone } => {
                    leaf(vec![("$value", num(millis)), ("timeZone", num(time_zone))])
                }
                Amf0Event::AvmPlus => open(&mut stack, m, Kind::AvmPlus, vec![], &[]),
                Amf0Event::End => stack.pop().map(Frame::finish),
                // Null, undefined, unsupported.
                _ => leaf(vec![]),
            },
            EventKind::Amf3(e) => match e {
                Amf3Event::Integer { value, .. } => leaf(vec![("$value", num(value))]),
                Amf3Event::Double(n) => leaf(vec![("$value", num(n))]),
                Amf3Event::String(s) => leaf(vec![("stringOrReference", sor(&reader, &s))]),
                Amf3Event::Reference { header, index } => {
                    let fields = vec![(header_field(m), num(header)), ("$reference", num(index))];
                    if matches!(m, 0x07 | 0x0b) {
                        let r = element_fields(fields);
                        leaf(vec![("stringOrReference", Js::Obj(r))])
                    } else {
                        leaf(fields)
                    }
                }
                Amf3Event::XmlDocument {
                    header, text: t, ..
                }
                | Amf3Event::Xml {
                    header, text: t, ..
                } => {
                    let mut s = Obj::new();
                    set(&mut s, "$lengthOrReference", num(header));
                    set(&mut s, "$value", text(t));
                    leaf(vec![("stringOrReference", Js::Obj(s))])
                }
                Amf3Event::Date { header, millis, .. } => {
                    leaf(vec![("$indicator", num(header)), ("$value", num(millis))])
                }
                Amf3Event::ArrayStart { header, .. } => open(
                    &mut stack,
                    m,
                    Kind::Array,
                    vec![("$denseLengthOrReference", num(header))],
                    &["associativeValues", "$values"],
                ),
                Amf3Event::ObjectStart {
                    header,
                    traits_ref,
                    traits,
                    ..
                } => {
                    let t = match traits_ref {
                        TraitsRef::Inline { .. } => {
                            let mut t = Obj::new();
                            set(&mut t, "className", sor(&reader, &traits.class_name));
                            let names = traits.sealed_names.iter().map(|n| sor(&reader, n));
                            set(&mut t, "sealedMemberNames", Js::Arr(names.collect()));
                            t
                        }
                        TraitsRef::Reference { index } => {
                            let mut t = Obj::new();
                            set(&mut t, "$reference", num(index));
                            t
                        }
                    };
                    let fields = vec![
                        ("$objectTypeIndicator", num(header)),
                        ("$traits", Js::Obj(t)),
                    ];
                    if traits.externalizable {
                        open(&mut stack, m, Kind::List, fields, &["$externalized"])
                    } else {
                        open(
                            &mut stack,
                            m,
                            Kind::Object,
                            fields,
                            &["_values", "_dynamicMembers"],
                        )
                    }
                }
                Amf3Event::ByteArray { header, bytes, .. } => leaf(vec![
                    ("$lengthOrReference", num(header)),
                    ("$value", Js::typed_array(bytes)),
                ]),
                Amf3Event::VectorStart {
                    header,
                    kind,
                    fixed,
                    type_name,
                    ..
                } => {
                    let mut fields = vec![
                        ("$lengthOrReference", num(header)),
                        ("isFixed", Js::Bool(fixed)),
                    ];
                    match (kind, type_name) {
                        (VectorKind::Int | VectorKind::Uint, _) => {
                            open(&mut stack, m, Kind::Bytes, fields, &[])
                        }
                        (VectorKind::Double, _) => {
                            open(&mut stack, m, Kind::List, fields, &["values"])
                        }
                        (_, name) => {
                            if let Some(name) = name {
                                fields.push(("objectTypeName", sor(&reader, &name)));
                            }
                            open(&mut stack, m, Kind::List, fields, &["values"])
                        }
                    }
                }
                Amf3Event::DictionaryStart {
                    header, weak_keys, ..
                } => open(
                    &mut stack,
                    m,
                    Kind::Dictionary,
                    vec![
                        ("$lengthOrReference", num(header)),
                        ("hasWeakKeys", Js::Bool(weak_keys)),
                    ],
                    &["$entries"],
                ),
                Amf3Event::Key(s) => {
                    let k = sor(&reader, &s);
                    if let Some(f) = stack.last_mut() {
                        f.key = Some(k);
                    }
                    None
                }
                Amf3Event::IntElement(_) | Amf3Event::UintElement(_) => {
                    if let Some(f) = stack.last_mut() {
                        f.bytes.extend_from_slice(
                            input.get(event.offset..event.end).unwrap_or_default(),
                        );
                    }
                    None
                }
                Amf3Event::DoubleElement(n) => {
                    if let Some(f) = stack.last_mut() {
                        f.push(0, num(n));
                    }
                    None
                }
                Amf3Event::ExternalBytes(b) => {
                    if let Some(f) = stack.last_mut() {
                        f.push(0, Js::typed_array(b));
                    }
                    None
                }
                Amf3Event::OpaqueExternalizable { class_name, .. } => {
                    return Err(format!(
                        "the body of externalizable class \"{}\" at offset {} has a class-defined format (AMF 3 §3.12) that is not known",
                        class_name.to_string_lossy(),
                        event.offset
                    ));
                }
                Amf3Event::End => stack.pop().map(Frame::finish),
                Amf3Event::KeyListEnd => None,
                // Undefined, null, false, true.
                _ => leaf(vec![]),
            },
        };
        let Some(mut v) = done else { continue };
        // Hand the value up, closing AVM+ switches it completes.
        loop {
            let Some(f) = stack.last_mut() else {
                result = Some(v);
                break;
            };
            if !f.attach(v) {
                break;
            }
            match stack.pop() {
                Some(f) => v = f.finish(),
                None => break,
            }
        }
    }
    result.ok_or_else(|| "unexpected end of input".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ArgValue;
    use crate::util::from_hex;

    fn encode(format: &str, json: &str) -> String {
        let args = [ArgValue::Str(format.into())];
        match AMF_ENCODE.apply(Dish::Text(json.into()), &args) {
            Ok(d) => d.into_bytes().iter().map(|b| format!("{b:02x}")).collect(),
            Err(e) => e.0,
        }
    }

    fn decode(format: &str, hex: &str) -> String {
        let bytes = from_hex(hex, "Auto", 2).unwrap();
        let args = [ArgValue::Str(format.into())];
        match AMF_DECODE.apply(Dish::Bytes(bytes), &args) {
            Ok(d) => d.into_text(),
            Err(e) => e.0,
        }
    }

    fn hex(s: &str) -> String {
        s.replace(' ', "")
    }

    #[test]
    fn amf0_encodes_per_spec() {
        let cases = [
            ("1", "00 3ff0000000000000"),
            ("true", "01 01"),
            ("null", "05"),
            ("\"abc\"", "02 0003 616263"),
            // §1.3.1: UTF-8, lengths in bytes.
            ("\"é\"", "02 0002 c3a9"),
            ("\"😀\"", "02 0004 f09f9880"),
            ("{\"é\":\"€\"}", "03 0002 c3a9 02 0003 e282ac 000009"),
            ("[]", "0a 00000000"),
            ("{}", "03 000009"),
            (
                "{\"a\":1,\"b\":\"x\"}",
                "03 0001 61 00 3ff0000000000000 0001 62 02 0001 78 000009",
            ),
            // No object references: repeats are written in full.
            (
                "[[1],[1]]",
                "0a 00000002 0a 00000001 00 3ff0000000000000 0a 00000001 00 3ff0000000000000",
            ),
            (
                "[{\"x\":1},{\"x\":1}]",
                "0a 00000002 03 0001 78 00 3ff0000000000000 000009 03 0001 78 00 3ff0000000000000 000009",
            ),
            // Lone surrogates read as U+FFFD.
            ("\"\\ud800\"", "02 0003 efbfbd"),
        ];
        for (json, want) in cases {
            assert_eq!(encode("AMF0", json), hex(want), "{json}");
        }
        // §2.4 / §2.14: the short form up to 65535 bytes, then a long string.
        let s = "é".repeat(0x7FFF) + "a";
        assert!(encode("AMF0", &format!("\"{s}\"")).starts_with("02ffff"));
        let s = "é".repeat(0x8000);
        assert!(encode("AMF0", &format!("\"{s}\"")).starts_with("0c00010000c3a9"));
        // A key is a `UTF-8` field: at most 65535 bytes.
        let out = encode("AMF0", &format!("{{\"{}\":1}}", "é".repeat(0x8000)));
        assert!(out.starts_with("AMF Encode - "), "{out}");
        assert_eq!(
            encode("AMF0", ""),
            "Error translating from ArrayBuffer to JSON: SyntaxError: Unexpected end of JSON input"
        );
    }

    #[test]
    fn amf3_encodes_per_spec() {
        let cases = [
            // §3.6: integers in the 29-bit range, doubles otherwise.
            ("1", "04 01"),
            ("-1", "04 ffffffff"),
            ("268435455", "04 bfffffff"),
            ("268435456", "05 41b0000000000000"),
            ("1.5", "05 3ff8000000000000"),
            ("-0", "05 8000000000000000"),
            ("true", "03"),
            ("false", "02"),
            ("null", "01"),
            ("\"é\"", "06 05 c3a9"),
            ("\"\"", "06 01"),
            // §3.11: dense arrays.
            ("[]", "09 01 01"),
            ("[1,true]", "09 05 01 04 01 03"),
            // §1.3.2: a repeated string by string-table reference.
            ("[\"x\",\"x\",\"y\"]", "09 07 01 06 03 78 06 00 06 03 79"),
            // §3.12: anonymous dynamic objects; repeated traits and member
            // names by reference, the inner object written in full.
            ("{}", "0a 0b 01 01"),
            ("{\"a\":{\"a\":1}}", "0a 0b 01 03 61 0a 01 00 04 01 01 01"),
            (
                "[{\"x\":1},{\"x\":1}]",
                "09 05 01 0a 0b 01 03 78 04 01 01 0a 01 00 04 01 01",
            ),
            // An empty key can't be a dynamic member, so the object is sealed.
            ("{\"\":1}", "0a 13 01 01 04 01"),
        ];
        for (json, want) in cases {
            assert_eq!(encode("AMF3", json), hex(want), "{json}");
        }
        // Strings past 2^21 bytes take the 4-byte U29 (§1.3.1).
        let s = "a".repeat(1 << 21);
        assert!(encode("AMF3", &format!("\"{s}\"")).starts_with("068180800161"));
    }

    fn obj(v: &Js) -> &Obj {
        match v {
            Js::Obj(o) => o,
            v => panic!("{v:?}"),
        }
    }

    /// The JSON an AMF 3 Decode element tree stands for (the subset AMF
    /// Encode writes). `traits` is the traits table: sealed member names.
    fn amf3_tree_to_js(t: &Js, traits: &mut Vec<Vec<String>>) -> Js {
        let Js::Obj(o) = t else { panic!("{t:?}") };
        let field = |k: &str| o.get(k).unwrap_or_else(|| panic!("{k} in {o:?}"));
        let text = |v: &Js| match obj(v).get("$value") {
            Some(Js::Str(s)) => s.clone(),
            v => panic!("{v:?}"),
        };
        let list = |v: &Js| match v {
            Js::Arr(a) => a.clone(),
            v => panic!("{v:?}"),
        };
        match field("marker") {
            Js::Num(n) if *n == 1.0 => Js::Null,
            Js::Num(n) if *n == 2.0 => Js::Bool(false),
            Js::Num(n) if *n == 3.0 => Js::Bool(true),
            Js::Num(n) if *n == 4.0 || *n == 5.0 => field("$value").clone(),
            Js::Num(n) if *n == 6.0 => Js::Str(text(field("stringOrReference"))),
            Js::Num(n) if *n == 9.0 => {
                assert_eq!(field("associativeValues"), &Js::Arr(vec![]));
                let values = list(field("$values"));
                Js::Arr(values.iter().map(|v| amf3_tree_to_js(v, traits)).collect())
            }
            Js::Num(n) if *n == 10.0 => {
                let t = obj(field("$traits"));
                let names = match t.get("$reference") {
                    Some(Js::Num(i)) => traits[*i as usize].clone(),
                    _ => {
                        let names: Vec<String> = list(t.get("sealedMemberNames").unwrap())
                            .iter()
                            .map(text)
                            .collect();
                        traits.push(names.clone());
                        names
                    }
                };
                let mut out = Obj::new();
                for (k, v) in names.into_iter().zip(list(field("_values"))) {
                    out.insert(k, amf3_tree_to_js(&v, traits));
                }
                for m in list(field("_dynamicMembers")) {
                    let m = obj(&m);
                    let key = text(m.get("$key").unwrap());
                    out.insert(key, amf3_tree_to_js(m.get("value").unwrap(), traits));
                }
                Js::Obj(out)
            }
            m => panic!("marker {m:?}"),
        }
    }

    fn amf0_tree_to_js(t: &Js) -> Js {
        let Js::Obj(o) = t else { panic!("{t:?}") };
        let field = |k: &str| o.get(k).unwrap_or_else(|| panic!("{k} in {o:?}")).clone();
        match o.get("marker") {
            Some(Js::Num(n)) => match *n as u8 {
                0 | 1 | 2 | 12 => field("$value"),
                5 => Js::Null,
                3 => {
                    let Js::Arr(props) = field("properties") else {
                        panic!()
                    };
                    let mut out = Obj::new();
                    for p in props {
                        let Js::Obj(p) = p else { panic!() };
                        let Some(Js::Str(k)) = p.get("key") else {
                            panic!()
                        };
                        out.insert(k.clone(), amf0_tree_to_js(p.get("value").unwrap()));
                    }
                    Js::Obj(out)
                }
                10 => {
                    let Js::Arr(values) = field("values") else {
                        panic!()
                    };
                    Js::Arr(values.iter().map(amf0_tree_to_js).collect())
                }
                m => panic!("marker {m}"),
            },
            m => panic!("{m:?}"),
        }
    }

    /// Pseudo-random JSON text: every JSON type, non-ASCII and astral
    /// text, empty, repeated and integer-like keys, strings past the AMF 0
    /// short-string limit, and numbers on both sides of the AMF 3 integer
    /// range.
    struct Gen(u64);

    impl Gen {
        fn next(&mut self, n: u64) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 % n
        }

        fn string(&mut self) -> String {
            if self.next(40) == 0 {
                // Past 65535 UTF-8 bytes (but fewer UTF-16 units).
                return "é".repeat(0x8000 + self.next(10) as usize);
            }
            self.key()
        }

        fn key(&mut self) -> String {
            const PIECES: &[&str] = &[
                "a", "key", "é", "€", "😀", "日本", "\\u0000", "\\\"", "\\n", "0", "12",
            ];
            let n = self.next(4);
            (0..n)
                .map(|_| PIECES[self.next(PIECES.len() as u64) as usize])
                .collect()
        }

        fn value(&mut self, depth: u32, out: &mut String) {
            let pick = if depth == 0 {
                self.next(5)
            } else {
                self.next(7)
            };
            match pick {
                0 => out.push_str(["null", "true", "false"][self.next(3) as usize]),
                1 => {
                    const NUMS: &[&str] = &[
                        "0",
                        "-0",
                        "1",
                        "-1",
                        "268435455",
                        "268435456",
                        "-268435456",
                        "-268435457",
                        "1.5",
                        "1e300",
                        "-2.5e-8",
                        "4294967296",
                    ];
                    out.push_str(NUMS[self.next(NUMS.len() as u64) as usize]);
                }
                2..=4 => {
                    out.push('"');
                    out.push_str(&self.string());
                    out.push('"');
                }
                5 => {
                    out.push('[');
                    for i in 0..self.next(5) {
                        if i > 0 {
                            out.push(',');
                        }
                        self.value(depth - 1, out);
                    }
                    out.push(']');
                }
                _ => {
                    out.push('{');
                    for i in 0..self.next(5) {
                        if i > 0 {
                            out.push(',');
                        }
                        out.push('"');
                        if self.next(8) > 0 {
                            let key = self.key();
                            out.push_str(&key);
                        }
                        out.push_str("\":");
                        self.value(depth - 1, out);
                    }
                    out.push('}');
                }
            }
        }
    }

    /// Checks AMF Encode's output decodes to the input JSON, both with AMF
    /// Decode and with `utilify-amf`.
    fn round_trip(json: &str) {
        with_big_stack(|| round_trip_on_this_stack(json));
    }

    fn round_trip_on_this_stack(json: &str) {
        let want = super::super::js::parse(json).unwrap();
        for format in ["AMF0", "AMF3"] {
            let args = [ArgValue::Str(format.into())];
            let bytes = AMF_ENCODE
                .apply(Dish::Text(json.into()), &args)
                .unwrap_or_else(|e| panic!("{format} {json}: {}", e.0))
                .into_bytes();
            let options = if format == "AMF0" {
                JsonOptions::AMF0
            } else {
                JsonOptions::AMF3
            };
            let lib = utilify_amf::decode_to_json(&bytes, &options).unwrap();
            assert_eq!(Js::from(lib), want, "{format} utilify-amf: {json}");
            let tree = AMF_DECODE
                .apply(Dish::Bytes(bytes), &args)
                .unwrap()
                .into_text();
            let tree = super::super::js::parse(&tree).unwrap();
            let ours = if format == "AMF0" {
                amf0_tree_to_js(&tree)
            } else {
                amf3_tree_to_js(&tree, &mut vec![])
            };
            assert_eq!(ours, want, "{format} AMF Decode: {json}");
        }
    }

    #[test]
    fn encoding_round_trips() {
        let mut g = Gen(0x9E37_79B9_7F4A_7C15);
        for _ in 0..300 {
            let mut json = String::new();
            g.value(5, &mut json);
            round_trip(&json);
        }
        round_trip("[\"x\",{\"x\":\"x\"},{\"x\":[\"x\"]},{\"\":\"\",\"x\":1}]");
        let deep = 300;
        round_trip(&format!(
            "{}\"é\"{}",
            "[{\"é\":".repeat(deep),
            "}]".repeat(deep)
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
    fn deep_input_does_not_overflow() {
        let n = 4000;
        let arrays = format!("{}1{}", "[".repeat(n), "]".repeat(n));
        let amf0 = small_stack(|| encode("AMF0", &arrays));
        let want = format!("{}003ff0000000000000", "0a00000001".repeat(n));
        assert_eq!(amf0, want);
        // Inner objects reuse the traits and the member name by reference.
        let objects = format!("{}1{}", "{\"a\":".repeat(n), "}".repeat(n));
        let amf3 = small_stack(|| encode("AMF3", &objects));
        assert_eq!(
            amf3,
            format!("0a0b010361{}0401{}", "0a0100".repeat(n - 1), "01".repeat(n))
        );
        let bytes = from_hex(&amf3, "Auto", 2).unwrap();
        // (The library's values drop recursively.)
        let back = with_big_stack(|| {
            let options = JsonOptions::AMF3.decode_options(DecodeOptions::STRICT.max_depth(n));
            utilify_amf::decode_to_json(&bytes, &options)
                .map(|v| super::super::js::stringify_compact(&Js::from(v)))
        });
        assert_eq!(back.unwrap().unwrap().unwrap(), objects);
        assert!(small_stack(|| decode("AMF0", &amf0)).starts_with("{\n    \"marker\": 10,"));
        let too_deep = "0a00000001".repeat(MAX_DEPTH + 1);
        assert_eq!(
            small_stack(|| decode("AMF0", &too_deep)),
            format!("AMF Decode - nesting deeper than {MAX_DEPTH} at offset 25000")
        );
        let too_deep = "0a13010361".repeat(MAX_DEPTH + 1);
        assert!(
            small_stack(|| decode("AMF3", &too_deep))
                .starts_with(&format!("AMF Decode - nesting deeper than {MAX_DEPTH}"))
        );
    }

    fn compact(s: &str) -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect()
    }

    fn check(format: &str, cases: &[(&str, &str)]) {
        for (hex, want) in cases {
            assert_eq!(compact(&decode(format, hex)), *want, "{hex}");
        }
    }

    #[test]
    fn amf0_decodes_per_spec() {
        check(
            "AMF0",
            &[
                ("003ff0000000000000", r#"{"marker":0,"$value":1}"#),
                ("0101", r#"{"marker":1,"$value":true}"#),
                // §1.3.1: UTF-8, NUL included.
                (
                    "020003610062",
                    r#"{"marker":2,"length":3,"$value":"a\u0000b"}"#,
                ),
                // Key lengths count UTF-8 bytes (§1.3.1).
                (
                    "030002c3a905000009",
                    r#"{"marker":3,"properties":[{"keyLength":2,"key":"é","value":{"marker":5}}]}"#,
                ),
                (
                    "1000014100016106000009",
                    r#"{"marker":16,"classNameLength":1,"className":"A","properties":[{"keyLength":1,"key":"a","value":{"marker":6}}]}"#,
                ),
                // §2.10: properties run to the object end, whatever the count.
                (
                    "08000000050001610101000009",
                    r#"{"marker":8,"_count":5,"properties":[{"keyLength":1,"key":"a","value":{"marker":1,"$value":true}}],"endKey":0,"endMarker":9}"#,
                ),
                (
                    "0a000000020800000001000161003ff0000000000000000009003ff0000000000000",
                    r#"{"marker":10,"count":2,"values":[{"marker":8,"_count":1,"properties":[{"keyLength":1,"key":"a","value":{"marker":0,"$value":1}}],"endKey":0,"endMarker":9},{"marker":0,"$value":1}]}"#,
                ),
                // §2.9: a reference to the object at table index 1.
                (
                    "0a0000000203000009070001",
                    r#"{"marker":10,"count":2,"values":[{"marker":3,"properties":[]},{"marker":7,"index":1}]}"#,
                ),
                // §2.13: milliseconds as sent, and the time-zone field.
                (
                    "0b3ff80000000000000000",
                    r#"{"marker":11,"$value":1.5,"timeZone":0}"#,
                ),
                (
                    "0b7ff80000000000000000",
                    r#"{"marker":11,"$value":null,"timeZone":0}"#,
                ),
                ("0c0000000161", r#"{"marker":12,"_length":1,"$value":"a"}"#),
                // §3.1: an AMF 3 value follows the switch.
                (
                    "110405",
                    r#"{"marker":17,"$value":{"marker":4,"$value":5}}"#,
                ),
            ],
        );
        for (hex, msg) in [
            ("", "unexpected end of input at offset 0"),
            ("0800000000", "unexpected end of input at offset 5"),
            ("12", "unknown type marker 0x12 at offset 0"),
            ("04", "type marker 0x04 is not allowed here at offset 0"),
            (
                "070000",
                "reference to missing AMF 0 object table entry 0 at offset 1",
            ),
            ("020001ff", "invalid UTF-8 at offset 3"),
            ("1104", "unexpected end of input at offset 2"),
        ] {
            let out = decode("AMF0", hex);
            assert!(out.starts_with("AMF Decode - "), "{hex}: {out}");
            assert!(
                out.contains(msg.split(" at ").next().unwrap_or(msg)),
                "{hex}: {out}"
            );
        }
    }

    #[test]
    fn amf3_decodes_per_spec() {
        check(
            "AMF3",
            &[
                // §3.6: a 29-bit signed integer; §1.3.1: the fourth U29 byte
                // holds 8 bits.
                ("04ffffffff", r#"{"marker":4,"$value":-1}"#),
                ("0480c08000", r#"{"marker":4,"$value":2097152}"#),
                // §3.12: sealed member values, then dynamic members.
                (
                    "0a1b01036104010362040201",
                    r#"{"marker":10,"$objectTypeIndicator":27,"$traits":{"className":{"$lengthOrReference":1,"$value":""},"sealedMemberNames":[{"$lengthOrReference":3,"$value":"a"}]},"_values":[{"marker":4,"$value":1}],"_dynamicMembers":[{"$key":{"$lengthOrReference":3,"$value":"b"},"value":{"marker":4,"$value":2}}]}"#,
                ),
                // §3.15: `Vector.<Number>` holds doubles.
                (
                    "0905010f03003ff80000000000000407",
                    r#"{"marker":9,"$denseLengthOrReference":5,"associativeValues":[],"$values":[{"marker":15,"$lengthOrReference":3,"isFixed":false,"values":[1.5]},{"marker":4,"$value":7}]}"#,
                ),
                // §3.15: the object type name comes before the values.
                (
                    "100300032a060378",
                    r#"{"marker":16,"$lengthOrReference":3,"isFixed":false,"objectTypeName":{"$lengthOrReference":3,"$value":"*"},"values":[{"marker":6,"stringOrReference":{"$lengthOrReference":3,"$value":"x"}}]}"#,
                ),
                (
                    "0d050000000001ffffffff",
                    r#"{"marker":13,"$lengthOrReference":5,"isFixed":false,"bytes":{"0":0,"1":0,"2":0,"3":1,"4":255,"5":255,"6":255,"7":255}}"#,
                ),
                (
                    "0c050102",
                    r#"{"marker":12,"$lengthOrReference":5,"$value":{"0":1,"1":2}}"#,
                ),
                (
                    "1103000401060361",
                    r#"{"marker":17,"$lengthOrReference":3,"hasWeakKeys":false,"$entries":[{"key":{"marker":4,"$value":1},"value":{"marker":6,"stringOrReference":{"$lengthOrReference":3,"$value":"a"}}}]}"#,
                ),
            ],
        );
    }

    #[test]
    fn amf3_references_per_spec() {
        check(
            "AMF3",
            &[
                // §2.2: object-table entry 1 is the date.
                (
                    "090501080142774a5e2d4000000802",
                    r#"{"marker":9,"$denseLengthOrReference":5,"associativeValues":[],"$values":[{"marker":8,"$indicator":1,"$value":1600510940160},{"marker":8,"$indicator":2,"$reference":1}]}"#,
                ),
                // §1.3.2: a string reference, resolved.
                (
                    "0905010603780600",
                    r#"{"marker":9,"$denseLengthOrReference":5,"associativeValues":[],"$values":[{"marker":6,"stringOrReference":{"$lengthOrReference":3,"$value":"x"}},{"marker":6,"stringOrReference":{"$lengthOrReference":0,"$reference":0,"$value":"x"}}]}"#,
                ),
                // §3.12: a traits reference.
                (
                    "0905010a1301036104010a010402",
                    r#"{"marker":9,"$denseLengthOrReference":5,"associativeValues":[],"$values":[{"marker":10,"$objectTypeIndicator":19,"$traits":{"className":{"$lengthOrReference":1,"$value":""},"sealedMemberNames":[{"$lengthOrReference":3,"$value":"a"}]},"_values":[{"marker":4,"$value":1}],"_dynamicMembers":[]},{"marker":10,"$objectTypeIndicator":1,"$traits":{"$reference":0},"_values":[{"marker":4,"$value":2}],"_dynamicMembers":[]}]}"#,
                ),
            ],
        );
        // An object reference to the enclosing array.
        assert_eq!(
            compact(&decode("AMF3", "0903010900")),
            r#"{"marker":9,"$denseLengthOrReference":3,"associativeValues":[],"$values":[{"marker":9,"$denseLengthOrReference":0,"$reference":0}]}"#
        );
        // Flex's ArrayCollection body is one AMF 3 value.
        let class = "flex.messaging.io.ArrayCollection";
        let hex = format!(
            "0a07{:02x}{}090101",
            class.len() * 2 + 1,
            class
                .bytes()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        assert_eq!(
            compact(&decode("AMF3", &hex)),
            format!(
                r#"{{"marker":10,"$objectTypeIndicator":7,"$traits":{{"className":{{"$lengthOrReference":67,"$value":"{class}"}},"sealedMemberNames":[]}},"$externalized":[{{"marker":9,"$denseLengthOrReference":1,"associativeValues":[],"$values":[]}}]}}"#
            )
        );
        for (hex, msg) in [
            ("0600", "reference to missing AMF 3 string table entry 0"),
            ("0a00", "reference to missing"),
            ("0a01", "reference to missing"),
            ("0a0703610102", "externalizable class \"a\""),
            ("0902", "reference to missing"),
            ("04", "unexpected end of input"),
            ("0d0300", "unexpected end of input"),
            ("0d0302", "flag byte 0x02"),
        ] {
            let out = decode("AMF3", hex);
            assert!(
                out.starts_with("AMF Decode - ") && out.contains(msg),
                "{hex}: {out}"
            );
        }
    }
}
