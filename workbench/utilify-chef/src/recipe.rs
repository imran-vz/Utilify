//! A recipe is a list of steps run in order over the input ("baking").
//! Recipes save and load as CyberChef's JSON
//! (`[{"op":"To Base64","args":["A-Za-z0-9+/="]}]`) and read its chef format
//! (`To_Base64('A-Za-z0-9+/=')`) and share URLs too, so recipes copied from
//! CyberChef work here.

use crate::{ArgKind, ArgValue, Dish, Operation};
use serde_json::{Map, Value, json};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct Step {
    pub op: &'static Operation,
    pub args: Vec<ArgValue>,
    /// Skipped when baking.
    pub disabled: bool,
    /// Baking pauses before this step.
    pub breakpoint: bool,
}

impl Step {
    pub fn new(op: &'static Operation) -> Self {
        Step {
            op,
            args: op.default_args(),
            disabled: false,
            breakpoint: false,
        }
    }

    pub fn with_args(op: &'static Operation, args: Vec<ArgValue>) -> Self {
        let args = op
            .args
            .iter()
            .enumerate()
            .map(|(i, spec)| {
                args.get(i)
                    .cloned()
                    .map_or_else(|| spec.default_value(), |v| spec.coerce(v))
            })
            .collect();
        Step {
            args,
            ..Step::new(op)
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Recipe {
    pub steps: Vec<Step>,
}

/// Where baking stopped short of the end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Halt {
    /// Step `step` failed; the output holds its message.
    Error { step: usize, message: String },
    /// Paused before step `step`, which has a breakpoint.
    Breakpoint { step: usize },
}

#[derive(Clone, Debug)]
pub struct Baked {
    pub dish: Dish,
    pub halt: Option<Halt>,
    /// Steps that ran.
    pub ran: usize,
    pub duration: Duration,
}

impl Recipe {
    /// Run every enabled step over `input`. A failing step stops the recipe
    /// and its message becomes the output, as in CyberChef. Breakpoints pause
    /// before their step unless `from` is past them (resuming).
    pub fn bake(&self, input: Dish) -> Baked {
        self.bake_from(input, 0)
    }

    /// Bake starting at step `from`, as when stepping past a breakpoint with
    /// the dish it paused on.
    pub fn bake_from(&self, input: Dish, from: usize) -> Baked {
        let start = Instant::now();
        let mut dish = input;
        let mut ran = 0;
        let mut halt = None;
        for (i, step) in self.steps.iter().enumerate().skip(from) {
            if step.disabled {
                continue;
            }
            if step.breakpoint && i > from {
                halt = Some(Halt::Breakpoint { step: i });
                break;
            }
            match step.op.apply(dish, &step.args) {
                Ok(next) => {
                    dish = next;
                    ran += 1;
                }
                Err(e) => {
                    dish = Dish::Text(e.0.clone());
                    halt = Some(Halt::Error {
                        step: i,
                        message: e.0,
                    });
                    break;
                }
            }
        }
        Baked {
            dish,
            halt,
            ran,
            duration: start.elapsed(),
        }
    }

    /// CyberChef's recipe JSON.
    pub fn to_json(&self) -> String {
        let steps: Vec<Value> = self.steps.iter().map(step_json).collect();
        serde_json::to_string(&steps).unwrap_or_else(|_| "[]".into())
    }

    /// One step per line, in CyberChef's chef format.
    pub fn to_chef(&self) -> String {
        self.steps
            .iter()
            .map(|s| {
                let v = step_json(s);
                let args = serde_json::to_string(&v["args"]).unwrap_or_default();
                let mut flags = String::new();
                if s.disabled {
                    flags.push_str("/disabled");
                }
                if s.breakpoint {
                    flags.push_str("/breakpoint");
                }
                format!(
                    "{}({}{flags})",
                    s.op.name.replace(' ', "_"),
                    json_to_chef_args(&args[1..args.len() - 1])
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Read a recipe as CyberChef JSON, chef format, or a CyberChef URL.
    /// Unknown operations are an error naming them, and so is a number
    /// argument that isn't one, as CyberChef rejects it when it loads the
    /// recipe ([`number_arg`]).
    pub fn parse(text: &str) -> Result<Recipe, String> {
        let text = text.trim_matches(crate::util::js_is_space);
        if text.is_empty() {
            return Ok(Recipe::default());
        }
        let config = if text.starts_with('[') {
            serde_json::from_str::<Value>(text).map_err(|e| format!("Invalid recipe JSON: {e}"))?
        } else if let Some(recipe) = url_param(text, "recipe") {
            parse_chef(&recipe)?
        } else {
            parse_chef(text)?
        };
        let Value::Array(items) = config else {
            return Err("A recipe is a list of operations".into());
        };
        let mut steps = vec![];
        let mut unknown = vec![];
        let mut bad_number = None;
        for item in items {
            let name = item["op"].as_str().unwrap_or_default();
            let Some(op) = crate::find(name) else {
                unknown.push(name.to_string());
                continue;
            };
            let empty = vec![];
            let json_args = item["args"].as_array().unwrap_or(&empty);
            let mut args: Vec<ArgValue> = json_args.iter().map(arg_from_json).collect();
            for ((spec, json), arg) in op.args.iter().zip(json_args).zip(&mut args) {
                if !matches!(spec.kind, ArgKind::Number { .. }) {
                    continue;
                }
                match number_arg(json) {
                    Ok(Some(n)) => *arg = ArgValue::Num(n),
                    Ok(None) => {}
                    Err(sample) => {
                        bad_number.get_or_insert_with(|| {
                            format!(
                                "Failed to hydrate operation '{}': Failed to set value of \
                                 ingredient '{}': Invalid ingredient value. Not a number: {sample}",
                                op.name, spec.name
                            )
                        });
                    }
                }
            }
            let mut step = Step::with_args(op, args);
            step.disabled = item["disabled"].as_bool().unwrap_or(false);
            step.breakpoint = item["breakpoint"].as_bool().unwrap_or(false);
            steps.push(step);
        }
        // CyberChef looks every operation up before it sets any argument.
        if !unknown.is_empty() {
            return Err(format!("Not available yet: {}", unknown.join(", ")));
        }
        match bad_number {
            Some(e) => Err(e),
            None => Ok(Recipe { steps }),
        }
    }
}

/// The `input=` of a CyberChef URL, which is Base64 of the input. CyberChef
/// reads it with `fromBase64`'s defaults: characters outside `A-Za-z0-9+/=`
/// are dropped and padding is optional.
pub fn url_input(url: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    let b64: String = url_param(url, "input")?
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
        .collect();
    let engine = base64::engine::GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        base64::engine::GeneralPurposeConfig::new()
            .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent)
            .with_decode_allow_trailing_bits(true),
    );
    engine.decode(b64).ok()
}

fn step_json(s: &Step) -> Value {
    let args: Vec<Value> =
        s.op.args
            .iter()
            .zip(&s.args)
            .map(|(spec, v)| match (spec.kind, v) {
                (_, ArgValue::Str(s)) => json!(s),
                (_, ArgValue::Num(n)) => {
                    if n.fract() == 0.0 && n.abs() < 9e15 {
                        json!(*n as i64)
                    } else {
                        json!(n)
                    }
                }
                (_, ArgValue::Bool(b)) => json!(b),
                (ArgKind::Toggle { .. }, ArgValue::Toggle { string, format })
                | (_, ArgValue::Toggle { string, format }) => {
                    json!({ "option": format, "string": string })
                }
            })
            .collect();
    let mut m = Map::new();
    m.insert("op".into(), json!(s.op.name));
    m.insert("args".into(), Value::Array(args));
    if s.disabled {
        m.insert("disabled".into(), json!(true));
    }
    if s.breakpoint {
        m.insert("breakpoint".into(), json!(true));
    }
    Value::Object(m)
}

fn arg_from_json(v: &Value) -> ArgValue {
    match v {
        Value::Bool(b) => ArgValue::Bool(*b),
        Value::Number(n) => ArgValue::Num(n.as_f64().unwrap_or_default()),
        Value::String(s) => ArgValue::Str(s.clone()),
        Value::Object(o) => ArgValue::Toggle {
            string: o
                .get("string")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .into(),
            format: o
                .get("option")
                .and_then(Value::as_str)
                .unwrap_or("UTF8")
                .into(),
        },
        Value::Null => ArgValue::Str(String::new()),
        Value::Array(_) => ArgValue::Str(v.to_string()),
    }
}

/// A number argument's value as CyberChef's `Ingredient.prepare` reads it:
/// `parseFloat(String(value))`, so `"8"` and `"8px"` are 8, and `null` stays
/// unset (`None`, the default here; CyberChef hands the operation `null`).
/// Where that is NaN (`"abc"`, `true`, an object) the recipe doesn't load;
/// the error is the value's first characters, as CyberChef's
/// `Utils.truncate(value, 10)` gives them.
fn number_arg(v: &Value) -> Result<Option<f64>, String> {
    if let Value::Number(n) = v {
        return Ok(n.as_f64());
    }
    if v.is_null() {
        return Ok(None);
    }
    let s = js_string(v);
    let n = crate::ops::data_format::numeric::js::parse_float(&s);
    if !n.is_nan() {
        return Ok(Some(n));
    }
    let units: Vec<u16> = s.encode_utf16().collect();
    Err(if units.len() > 10 {
        format!("{}...", String::from_utf16_lossy(&units[..7]))
    } else {
        s
    })
}

/// `String(value)` for a value from recipe JSON.
fn js_string(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => crate::format_number(n.as_f64().unwrap_or_default()),
        Value::String(s) => s.clone(),
        // Array.prototype.join: null items are empty.
        Value::Array(a) => a
            .iter()
            .map(|v| {
                if v.is_null() {
                    String::new()
                } else {
                    js_string(v)
                }
            })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".into(),
    }
}

/// JSON arguments as chef format: strings in single quotes, `'` escaped and
/// `"` bare.
fn json_to_chef_args(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    let mut chars = json.chars();
    let mut in_str = false;
    while let Some(c) = chars.next() {
        match (in_str, c) {
            (false, '"') => {
                in_str = true;
                out.push('\'');
            }
            (false, c) => out.push(c),
            (true, '"') => {
                in_str = false;
                out.push('\'');
            }
            (true, '\'') => out.push_str("\\'"),
            (true, '\\') => match chars.next() {
                Some('"') => out.push('"'),
                Some(n) => {
                    out.push('\\');
                    out.push(n);
                }
                None => out.push('\\'),
            },
            (true, c) => out.push(c),
        }
    }
    out
}

/// Chef format to the JSON recipe it stands for.
fn parse_chef(text: &str) -> Result<Value, String> {
    let chars: Vec<char> = text.chars().filter(|&c| c != '\n' && c != '\r').collect();
    let mut steps = vec![];
    let mut i = 0;
    let invalid =
        || "Invalid recipe: expected JSON or CyberChef's Op_Name(args) format".to_string();
    while i < chars.len() {
        while i < chars.len() && crate::util::js_is_space(chars[i]) {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }
        let open = chars[i..]
            .iter()
            .position(|&c| c == '(')
            .ok_or_else(invalid)?
            + i;
        let name: String = chars[i..open].iter().collect();
        let name = name.trim_matches(crate::util::js_is_space);
        if name.is_empty() {
            return Err(invalid());
        }
        // Arguments as JSON, up to the closing paren outside any string.
        let mut json = String::from("[");
        let mut j = open + 1;
        let mut in_str = false;
        let mut flags = String::new();
        loop {
            let Some(&c) = chars.get(j) else {
                return Err(invalid());
            };
            j += 1;
            if in_str {
                match c {
                    '\\' => {
                        let n = chars.get(j).copied().ok_or_else(invalid)?;
                        j += 1;
                        if n == '\'' {
                            json.push('\'');
                        } else {
                            json.push('\\');
                            json.push(n);
                        }
                    }
                    '\'' => {
                        in_str = false;
                        json.push('"');
                    }
                    '"' => json.push_str("\\\""),
                    c => json.push(c),
                }
                continue;
            }
            match c {
                '\'' => {
                    in_str = true;
                    json.push('"');
                }
                ')' => break,
                '/' => {
                    while let Some(&f) = chars.get(j) {
                        if f == ')' {
                            break;
                        }
                        flags.push(f);
                        j += 1;
                    }
                }
                c => json.push(c),
            }
        }
        json.push(']');
        let args: Value = serde_json::from_str(&json)
            .map_err(|e| format!("Invalid arguments for {name}: {e}"))?;
        let mut step = Map::new();
        step.insert(
            "op".into(),
            json!(
                name.replace('_', " ")
                    .trim_matches(crate::util::js_is_space)
            ),
        );
        step.insert("args".into(), args);
        if flags.contains("disabled") {
            step.insert("disabled".into(), json!(true));
        }
        if flags.contains("breakpoint") {
            step.insert("breakpoint".into(), json!(true));
        }
        steps.push(Value::Object(step));
        i = j;
    }
    Ok(Value::Array(steps))
}

/// A `name=value` from a CyberChef URL, read like CyberChef's
/// `parseURIParams` on the query string or else the fragment: parameters
/// split on `&`, one with no `=` or more than one is not a value, a later
/// one wins, and the value has `+` as a space and is then percent-decoded
/// as by `decodeURIComponent` (`None` where that would throw).
fn url_param(url: &str, name: &str) -> Option<String> {
    let params = if let Some(fragment) = url.strip_prefix('#') {
        fragment
    } else if url.contains("://") {
        let (before, fragment) = url.split_once('#').unwrap_or((url, ""));
        match before.split_once('?') {
            Some((_, query)) if !query.is_empty() => query,
            _ => fragment,
        }
    } else {
        return None;
    };
    let value = params.split('&').rev().find_map(|kv| {
        let mut parts = kv.split('=');
        let (k, v) = (parts.next()?, parts.next()?);
        (parts.next().is_none() && k == name).then_some(v)
    })?;
    let value = value.replace('+', " ");
    let b = value.as_bytes();
    let hex = |k: usize| b.get(k).is_some_and(u8::is_ascii_hexdigit);
    if (0..b.len()).any(|k| b[k] == b'%' && !(hex(k + 1) && hex(k + 2))) {
        return None;
    }
    percent_encoding::percent_decode_str(&value)
        .decode_utf8()
        .ok()
        .map(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trips() {
        let json = r#"[{"op":"To Hex","args":["Colon",4]},{"op":"From Hex","args":["Space"],"disabled":true}]"#;
        let r = Recipe::parse(json).unwrap();
        assert_eq!(r.steps.len(), 2);
        assert!(r.steps[1].disabled);
        assert_eq!(r.to_json(), json);
    }

    #[test]
    fn reads_chef_format_and_urls() {
        let chef = "To_Hex('Space',0)\nFrom_Hex('Auto'/breakpoint)";
        let r = Recipe::parse(chef).unwrap();
        assert_eq!(r.steps[0].op.name, "To Hex");
        assert!(r.steps[1].breakpoint);
        assert_eq!(r.to_chef(), chef);
        let url =
            "https://gchq.github.io/CyberChef/#recipe=To_Hex('0x%20with%20comma',0)&input=aGVsbG8";
        let r = Recipe::parse(url).unwrap();
        assert_eq!(r.steps[0].args[0], ArgValue::Str("0x with comma".into()));
        assert_eq!(url_input(url).unwrap(), b"hello");
        let quoted = Recipe::parse(r#"To_Hex('it\'s "x"',0)"#).unwrap();
        assert_eq!(quoted.steps[0].args[0], ArgValue::Str(r#"it's "x""#.into()));
        assert_eq!(quoted.to_chef(), r#"To_Hex('it\'s "x"',0)"#);
    }

    #[test]
    fn url_params_read_like_parse_uri_params() {
        let p = |url: &str| url_param(url, "recipe");
        assert_eq!(p("https://x/#recipe=a+b%2Bc").as_deref(), Some("a b+c"));
        // More than one `=` makes the parameter a flag, not a value.
        assert_eq!(p("https://x/#recipe=a=b"), None);
        assert_eq!(p("https://x/#recipe=a&recipe=b").as_deref(), Some("b"));
        assert_eq!(p("https://x/?recipe=q#recipe=f").as_deref(), Some("q"));
        assert_eq!(p("https://x/?#recipe=f").as_deref(), Some("f"));
        assert_eq!(p("#recipe=%E2%82%AC").as_deref(), Some("€"));
        // decodeURIComponent throws on these.
        assert_eq!(p("#recipe=100%"), None);
        assert_eq!(p("#recipe=%C3"), None);
        assert_eq!(p("recipe=a"), None);
        // `+` is a space, which fromBase64 drops; `%2B` is a `+`.
        assert_eq!(url_input("#input=aGVs+bG8").unwrap(), b"hello");
        assert_eq!(url_input("#input=%2B%2F8%3D").unwrap(), [0xFB, 0xFF]);
    }

    #[test]
    fn number_args_load_like_ingredient_prepare() {
        let width = |arg: &str| {
            Recipe::parse(&format!(r#"[{{"op":"To Hex","args":["Space",{arg}]}}]"#))
                .map(|r| r.steps[0].args[1].clone())
        };
        // parseFloat(String(value)).
        assert_eq!(width("8"), Ok(ArgValue::Num(8.0)));
        assert_eq!(width(r#""8""#), Ok(ArgValue::Num(8.0)));
        assert_eq!(width(r#"" 1.5e1px""#), Ok(ArgValue::Num(15.0)));
        assert_eq!(width(r#""\ufeff4""#), Ok(ArgValue::Num(4.0)));
        assert_eq!(width("[4]"), Ok(ArgValue::Num(4.0)));
        assert_eq!(
            width(r#""-Infinity""#),
            Ok(ArgValue::Num(f64::NEG_INFINITY))
        );
        // null leaves the default here.
        assert_eq!(width("null"), Ok(ArgValue::Num(0.0)));
        // NaN is an error when the recipe loads.
        let err = |sample: &str| {
            format!(
                "Failed to hydrate operation 'To Hex': Failed to set value of ingredient \
                 'Bytes per line': Invalid ingredient value. Not a number: {sample}"
            )
        };
        assert_eq!(width(r#""abc""#), Err(err("abc")));
        assert_eq!(width(r#""""#), Err(err("")));
        assert_eq!(width("true"), Err(err("true")));
        assert_eq!(width("[]"), Err(err("")));
        assert_eq!(width(r#"{"a":1}"#), Err(err("[object...")));
        assert_eq!(width(r#""abcdefghijk""#), Err(err("abcdefg...")));
        assert_eq!(width(r#""\u0085 4""#), Err(err("\u{85} 4")));
        assert_eq!(Recipe::parse("To_Hex('Space','x')").unwrap_err(), err("x"));
        // Unknown operations are reported first, as CyberChef looks them up
        // before setting arguments.
        let e = Recipe::parse(r#"[{"op":"To Hex","args":["Space","x"]},{"op":"Nope"}]"#);
        assert_eq!(e.unwrap_err(), "Not available yet: Nope");
    }

    #[test]
    fn whitespace_is_javascripts() {
        let r = Recipe::parse("\u{feff}\u{3000}To_Hex('Space',0)\u{2028}").unwrap();
        assert_eq!(r.steps[0].op.name, "To Hex");
        let r = Recipe::parse("\u{feff}[]").unwrap();
        assert!(r.steps.is_empty());
    }

    #[test]
    fn unknown_operations_are_named() {
        let e = Recipe::parse(r#"[{"op":"Nope","args":[]}]"#).unwrap_err();
        assert!(e.contains("Nope"));
    }

    #[test]
    fn errors_become_output_and_breakpoints_pause() {
        let mut r = Recipe::parse("From_Hex('None')To_Hex('Space',0)").unwrap();
        let baked = r.bake(Dish::Text("6869".into()));
        assert_eq!(baked.dish.text(), "68 69");
        assert_eq!(baked.halt, None);
        let baked = r.bake(Dish::Text("zz".into()));
        assert!(matches!(baked.halt, Some(Halt::Error { step: 0, .. })));
        assert_eq!(
            baked.dish.text(),
            "line 1, column 1: expected a hex digit, found 'z'"
        );
        r.steps[1].breakpoint = true;
        let paused = r.bake(Dish::Text("6869".into()));
        assert_eq!(paused.halt, Some(Halt::Breakpoint { step: 1 }));
        assert_eq!(paused.dish.text(), "hi");
        let resumed = r.bake_from(paused.dish, 1);
        assert_eq!(resumed.dish.text(), "68 69");
    }
}
