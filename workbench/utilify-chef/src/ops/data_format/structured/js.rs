//! JavaScript values as CyberChef's operations see them: `JSON.parse` with
//! V8's error messages, `JSON.stringify` with JS number printing and
//! property order, so JSON-typed inputs and outputs match exactly.
//!
//! Parsing, printing and number formatting are `utilify-js-json`'s
//! (`JSON_PARSE` mode, which reproduces V8's messages); `Js` adds what a
//! JSON value can't hold (`undefined`) and converts both ways.
//!
//! Accepted differences, from `utilify-js-json`: nesting deeper than 5,001
//! arrays or objects fails with "Maximum call stack size exceeded" (counting
//! only containers; scalars at the limit used to fail too), and error text
//! for malformed JSON is the library's V8 wording (which also fixes ours for
//! a `\` before a non-Latin-1 character: "Unexpected token", as in Node).

use crate::{Dish, OpError};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use utilify_js_json as json;

/// A JSON-like JavaScript value. Exotic values decoders produce (typed
/// arrays, Buffers, Dates, Maps) are stored as what `JSON.stringify` makes
/// of them.
#[derive(Clone, Debug, PartialEq)]
pub enum Js {
    Undefined,
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Js>),
    Obj(Obj),
}

impl Js {
    pub fn str(s: impl Into<String>) -> Js {
        Js::Str(s.into())
    }

    /// A `Uint8Array`, which stringifies as `{"0": b0, "1": b1, ...}`.
    pub fn typed_array(bytes: &[u8]) -> Js {
        let mut o = Obj::new();
        for (i, b) in bytes.iter().enumerate() {
            o.insert(i.to_string(), Js::Num(f64::from(*b)));
        }
        Js::Obj(o)
    }

    /// A Node `Buffer`, whose `toJSON` gives `{"type": "Buffer", "data": [...]}`.
    pub fn buffer(bytes: &[u8]) -> Js {
        let mut o = Obj::new();
        o.insert("type".into(), Js::str("Buffer"));
        o.insert(
            "data".into(),
            Js::Arr(bytes.iter().map(|b| Js::Num(f64::from(*b))).collect()),
        );
        Js::Obj(o)
    }

    pub fn is_object_like(&self) -> bool {
        matches!(self, Js::Arr(_) | Js::Obj(_))
    }

    /// JavaScript's `String(value)` for primitives; arrays join their items
    /// with commas and plain objects are `[object Object]`.
    pub fn to_js_string(&self) -> String {
        match self {
            Js::Undefined => "undefined".into(),
            Js::Null => "null".into(),
            Js::Bool(b) => b.to_string(),
            Js::Num(n) => js_number(*n),
            Js::Str(s) => s.clone(),
            Js::Arr(a) => a
                .iter()
                .map(|v| match v {
                    Js::Undefined | Js::Null => String::new(),
                    // Nested arrays recurse; grow the stack when it runs low.
                    v => stacker::maybe_grow(64 << 10, 1 << 20, || v.to_js_string()),
                })
                .collect::<Vec<_>>()
                .join(","),
            Js::Obj(_) => "[object Object]".into(),
        }
    }
}

/// A plain object's own properties in JavaScript order: array-index keys
/// ascending first, then the others in insertion order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Obj {
    ints: BTreeMap<u32, Js>,
    strs: Vec<(String, Js)>,
    index: HashMap<String, usize>,
}

impl Obj {
    pub fn new() -> Obj {
        Obj::default()
    }

    /// Set a property, keeping its place when it already exists.
    pub fn insert(&mut self, key: String, value: Js) {
        if let Some(i) = array_index(&key) {
            self.ints.insert(i, value);
        } else if let Some(&i) = self.index.get(&key) {
            self.strs[i].1 = value;
        } else {
            self.index.insert(key.clone(), self.strs.len());
            self.strs.push((key, value));
        }
    }

    pub fn get(&self, key: &str) -> Option<&Js> {
        match array_index(key) {
            Some(i) => self.ints.get(&i),
            None => self.index.get(key).map(|&i| &self.strs[i].1),
        }
    }

    pub fn len(&self) -> usize {
        self.ints.len() + self.strs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// `Object.keys`.
    pub fn keys(&self) -> Vec<String> {
        self.iter().map(|(k, _)| k.into_owned()).collect()
    }

    pub fn iter(&self) -> impl Iterator<Item = (std::borrow::Cow<'_, str>, &Js)> {
        let ints = self.ints.iter().map(|(k, v)| (k.to_string().into(), v));
        let strs = self.strs.iter().map(|(k, v)| (k.as_str().into(), v));
        ints.chain(strs)
    }
}

impl FromIterator<(String, Js)> for Obj {
    fn from_iter<T: IntoIterator<Item = (String, Js)>>(iter: T) -> Self {
        let mut o = Obj::new();
        for (k, v) in iter {
            o.insert(k, v);
        }
        o
    }
}

/// A canonical array index (`0` to `2^32 - 2`, no leading zeros).
fn array_index(key: &str) -> Option<u32> {
    if key.is_empty() || key.len() > 10 || (key.len() > 1 && key.starts_with('0')) {
        return None;
    }
    if !key.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    key.parse::<u64>()
        .ok()
        .filter(|&n| n < u64::from(u32::MAX))
        .map(|n| n as u32)
}

/// A number as JavaScript prints it.
pub fn js_number(n: f64) -> String {
    json::number_to_string(n)
}

/// `JSON.stringify(value, null, 4)`: `None` for `undefined`.
pub fn stringify_pretty(v: &Js) -> Result<Option<String>, OpError> {
    stringify(v, "    ")
}

/// `JSON.stringify(value)`.
pub fn stringify_compact(v: &Js) -> Result<Option<String>, OpError> {
    stringify(v, "")
}

pub fn stringify(v: &Js, gap: &str) -> Result<Option<String>, OpError> {
    let Some(value) = v.to_json_value()? else {
        return Ok(None);
    };
    Ok(Some(json::stringify_pretty(
        &value,
        json::Indent::string(gap),
    )))
}

/// The text of a JSON-typed output: pretty JSON, empty for `undefined`.
pub fn json_dish(v: &Js) -> Result<Dish, OpError> {
    Ok(Dish::Text(stringify_pretty(v)?.unwrap_or_default()))
}

impl Js {
    /// The JSON value `JSON.stringify` sees: `None` for `undefined`,
    /// `undefined` members dropped and `undefined` items `null`. Iterative, so any depth
    /// is safe on a small stack.
    pub fn to_json_value(&self) -> Result<Option<json::Value>, OpError> {
        type Entries<'a> = std::vec::IntoIter<(Option<Cow<'a, str>>, &'a Js)>;
        enum Step<'a> {
            Leaf(Option<json::Value>),
            Open(Entries<'a>, json::Value),
        }
        fn step(v: &Js) -> Result<Step<'_>, OpError> {
            Ok(Step::Leaf(Some(match v {
                Js::Undefined => return Ok(Step::Leaf(None)),
                Js::Null => json::Value::Null,
                Js::Bool(b) => json::Value::Bool(*b),
                Js::Num(n) => json::Value::Number(*n),
                Js::Str(s) => json::Value::from(s.as_str()),
                Js::Arr(a) => {
                    let items: Vec<_> = a.iter().map(|v| (None, v)).collect();
                    let out = json::Value::Array(json::Array::with_capacity(a.len()));
                    return Ok(Step::Open(items.into_iter(), out));
                }
                Js::Obj(o) => {
                    let items: Vec<_> = o.iter().map(|(k, v)| (Some(k), v)).collect();
                    let out = json::Value::Object(json::Object::with_capacity(o.len()));
                    return Ok(Step::Open(items.into_iter(), out));
                }
            })))
        }
        fn add(out: &mut json::Value, key: Option<Cow<'_, str>>, v: Option<json::Value>) {
            match (out, key, v) {
                (json::Value::Array(a), _, v) => a.push(v.unwrap_or(json::Value::Null)),
                (json::Value::Object(o), Some(k), Some(v)) => {
                    o.insert(k.as_ref(), v);
                }
                _ => {}
            }
        }

        let (items, out) = match step(self)? {
            Step::Leaf(v) => return Ok(v),
            Step::Open(items, out) => (items, out),
        };
        // (remaining children, value being built, its key in the parent)
        let mut stack = vec![(items, out, None)];
        loop {
            let Some((items, out, _)) = stack.last_mut() else {
                return Ok(None);
            };
            match items.next() {
                Some((key, child)) => match step(child)? {
                    Step::Open(items, child) => stack.push((items, child, key)),
                    Step::Leaf(v) => add(out, key, v),
                },
                None => {
                    let Some((_, done, key)) = stack.pop() else {
                        return Ok(None);
                    };
                    match stack.last_mut() {
                        Some((_, parent, _)) => add(parent, key, Some(done)),
                        None => return Ok(Some(done)),
                    }
                }
            }
        }
    }
}

/// A parsed or decoded JSON value as a `Js` value. Strings are converted
/// lossily (a lone surrogate becomes U+FFFD, as `Js::Str` holds Rust
/// strings); object members keep their insertion order, so keys that become
/// equal that way merge as repeated keys in `JSON.parse` do. Iterative, so
/// any depth is safe on a small stack.
impl From<json::Value> for Js {
    fn from(v: json::Value) -> Js {
        type Entries = std::vec::IntoIter<(Option<String>, json::Value)>;
        fn open(v: json::Value) -> Result<(Entries, Js), Js> {
            match v {
                json::Value::Null => Err(Js::Null),
                json::Value::Bool(b) => Err(Js::Bool(b)),
                json::Value::Number(n) => Err(Js::Num(n)),
                json::Value::String(s) => Err(Js::Str(s.to_string_lossy())),
                json::Value::Array(a) => {
                    let out = Js::Arr(Vec::with_capacity(a.len()));
                    let items: Vec<_> = a.into_vec().into_iter().map(|v| (None, v)).collect();
                    Ok((items.into_iter(), out))
                }
                json::Value::Object(o) => {
                    let items: Vec<_> = o
                        .into_insertion_order()
                        .into_iter()
                        .map(|(k, v)| (Some(k.to_string_lossy()), v))
                        .collect();
                    Ok((items.into_iter(), Js::Obj(Obj::new())))
                }
            }
        }
        fn add(out: &mut Js, key: Option<String>, v: Js) {
            match (out, key) {
                (Js::Arr(a), _) => a.push(v),
                (Js::Obj(o), Some(k)) => o.insert(k, v),
                _ => {}
            }
        }

        let (items, out) = match open(v) {
            Ok(container) => container,
            Err(scalar) => return scalar,
        };
        let mut stack = vec![(items, out, None)];
        loop {
            let Some((items, out, _)) = stack.last_mut() else {
                return Js::Undefined;
            };
            match items.next() {
                Some((key, child)) => match open(child) {
                    Ok((items, child)) => stack.push((items, child, key)),
                    Err(scalar) => add(out, key, scalar),
                },
                None => {
                    let Some((_, done, key)) = stack.pop() else {
                        return Js::Undefined;
                    };
                    match stack.last_mut() {
                        Some((_, parent, _)) => add(parent, key, done),
                        None => return done,
                    }
                }
            }
        }
    }
}

impl From<&json::Value> for Js {
    fn from(v: &json::Value) -> Js {
        Js::from(v.clone())
    }
}

/// Run `f` with a large stack: decoders and dropping a `Js` value recurse
/// once per nesting level, and inputs may nest as deeply as the JavaScript
/// libraries allow (thousands), deeper than the 512 KiB worker threads
/// allow. `stacker` switches to a fresh stack on this thread (reserved, so
/// only the part used is committed).
pub fn with_big_stack<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    stacker::grow(256 << 20, f)
}

/// A JSON-typed input as CyberChef translates it from the dish: the text
/// (a leading BOM dropped, as `TextDecoder` does) through `JSON.parse`.
pub fn json_input(input: &Dish) -> Result<Js, OpError> {
    let text = input.text();
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    parse(text).map_err(|e| {
        OpError::new(format!(
            "Error translating from ArrayBuffer to JSON: SyntaxError: {e}"
        ))
    })
}

/// How deep arrays and objects may nest before parsing gives up, standing
/// in for V8's stack limit.
const MAX_DEPTH: usize = 5001;

/// `JSON.parse`, failing with V8's message (positions count UTF-16 units).
/// Lone surrogates from `\u` escapes become U+FFFD.
pub fn parse(text: &str) -> Result<Js, String> {
    parse_value(text).map(Js::from)
}

/// `JSON.parse` to a JSON value, keeping lone surrogates, with the same
/// errors as [`parse`].
pub fn parse_value(text: &str) -> Result<json::Value, String> {
    json::ParseOptions::JSON_PARSE
        .max_depth(MAX_DEPTH)
        .parse(text)
        .map_err(|e| match e {
            json::Error::DepthLimitExceeded { .. } => "Maximum call stack size exceeded".into(),
            e => e.js_message().unwrap_or_else(|| e.to_string()),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pretty(text: &str) -> String {
        stringify_pretty(&parse(text).unwrap()).unwrap().unwrap()
    }

    #[test]
    fn numbers_print_like_js() {
        let cases = [
            (0.0, "0"),
            (-0.0, "0"),
            (1.5, "1.5"),
            (123456789012345680000.0, "123456789012345680000"),
            (1.2345678901234569e23, "1.2345678901234569e+23"),
            (1.5e-7, "1.5e-7"),
            (0.000001, "0.000001"),
            (5e-324, "5e-324"),
            (1e21, "1e+21"),
            (-2.5e-10, "-2.5e-10"),
            (-(1.0 + 2f64.powi(-17)), "-1.0000076293945312"),
        ];
        for (n, s) in cases {
            assert_eq!(js_number(n), s);
        }
    }

    #[test]
    fn stringify_matches_js() {
        assert_eq!(
            pretty(r#"{"b":1,"2":[],"1":{},"a":[1.0,-0,1e400],"01":"\u0001\u2028é"}"#),
            "{\n    \"1\": {},\n    \"2\": [],\n    \"b\": 1,\n    \"a\": [\n        1,\n        0,\n        null\n    ],\n    \"01\": \"\\u0001\u{2028}é\"\n}"
        );
        assert_eq!(
            stringify_compact(&parse("[1, {\"a\": null}]").unwrap())
                .unwrap()
                .unwrap(),
            "[1,{\"a\":null}]"
        );
        assert_eq!(stringify_pretty(&Js::Undefined).unwrap(), None);
    }

    #[test]
    fn converts_to_and_from_json_values() {
        let mut o = Obj::new();
        o.insert("u".into(), Js::Undefined);
        o.insert("a".into(), Js::Arr(vec![Js::Undefined, Js::Num(f64::NAN)]));
        assert_eq!(
            stringify_compact(&Js::Obj(o)).unwrap().unwrap(),
            "{\"a\":[null,null]}"
        );
        // Lone surrogates become U+FFFD; keys merged that way keep the first
        // position and the last value, as repeated keys do.
        let v = parse(r#"{"\ud800":1,"b":2,"\udc00":3,"\ud83d\ude00":"\udfff"}"#).unwrap();
        assert_eq!(
            stringify_compact(&v).unwrap().unwrap(),
            "{\"\u{fffd}\":3,\"b\":2,\"😀\":\"\u{fffd}\"}"
        );
    }

    #[test]
    fn deep_nesting_fits_small_stacks() {
        let deep = |open: &str, close: &str, n: usize| open.repeat(n) + &close.repeat(n);
        let run = move || {
            for (open, close) in [("[", "]"), ("{\"a\":", "}")] {
                let text = deep(open, close, 5001).replace("{\"a\":}", "{}");
                let out = with_big_stack(|| stringify_compact(&parse(&text).unwrap()));
                assert_eq!(out.unwrap().unwrap(), text);
                assert_eq!(
                    parse(&deep(open, close, 5002)).unwrap_err(),
                    "Maximum call stack size exceeded"
                );
            }
        };
        let thread = std::thread::Builder::new().stack_size(512 << 10).spawn(run);
        assert!(thread.unwrap().join().is_ok());
    }

    #[test]
    fn parse_errors_match_v8() {
        let cases = [
            ("", "Unexpected end of JSON input"),
            (
                "{bad",
                "Expected property name or '}' in JSON at position 1 (line 1 column 2)",
            ),
            (
                "{\"a\":1,}",
                "Expected double-quoted property name in JSON at position 7 (line 1 column 8)",
            ),
            ("[1,]", "Unexpected token ']', \"[1,]\" is not valid JSON"),
            (
                "01",
                "Unexpected number in JSON at position 1 (line 1 column 2)",
            ),
            (
                "[\r\n1,\r\n2\r 3]",
                "Expected ',' or ']' after array element in JSON at position 10 (line 4 column 2)",
            ),
            ("NaN", "\"NaN\" is not valid JSON"),
            (
                "\"\\u12\"",
                "Bad Unicode escape in JSON at position 5 (line 1 column 6)",
            ),
            (
                "[1,2,3,4,5,6,7,8,9,10,x]",
                "Unexpected token 'x', ...\",7,8,9,10,x]\" is not valid JSON",
            ),
            (
                "[x, 1, 2, 3, 4, 5, 6, 7, 8 ,9 ,10]",
                "Unexpected token 'x', \"[x, 1, 2, 3\"... is not valid JSON",
            ),
            (
                "[1,2,3,4,5,6,7,8,9,1,2,x,3,4,5,6,7,8,9,1]",
                "Unexpected token 'x', ...\"7,8,9,1,2,x,3,4,5,6,\"... is not valid JSON",
            ),
            (
                "\"\\一\"",
                "Unexpected token '一', \"\"\\一\"\" is not valid JSON",
            ),
            (
                "\"é😀\" x",
                "Unexpected non-whitespace character after JSON at position 6 (line 1 column 7)",
            ),
        ];
        for (src, msg) in cases {
            assert_eq!(parse(src).unwrap_err(), msg, "{src:?}");
        }
    }
}
