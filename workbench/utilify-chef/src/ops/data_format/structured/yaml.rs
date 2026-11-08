//! YAML to and from JSON, following YAML 1.2.2
//! (<https://yaml.org/spec/1.2.2/>).
//!
//! # YAML to JSON
//!
//! `saphyr-parser` 0.2 parses the stream (chapters 5 to 9) and `saphyr`'s
//! `YamlLoader` builds the node graph into our `YNode` (it implements
//! `LoadableYamlNode`). The loader runs with `early_parse` off, so every
//! scalar reaches us as its text, style and tag, and the adapter resolves
//! it under the core schema itself (§10.3):
//!
//! - An untagged plain scalar is resolved by the §10.3.2 table: `null`,
//!   `Null`, `NULL`, `~` and the empty scalar are null; `true`/`True`/
//!   `TRUE`/`false`/`False`/`FALSE` booleans; `[-+]?[0-9]+`, `0o[0-7]+` and
//!   `0x[0-9a-fA-F]+` integers; `[-+]?(\.[0-9]+|[0-9]+(\.[0-9]*)?)([eE][-+]?[0-9]+)?`,
//!   `[-+]?\.(inf|Inf|INF)` and `\.(nan|NaN|NAN)` floats; anything else a
//!   string (so `++7`, `+-1`, `0x-1F`, `+0x1F`, `1_000` are strings).
//! - Quoted and block scalars, and nodes with the non-specific tag `!`, are
//!   strings, sequences or mappings by kind (§6.9.1, §10.3.2).
//! - An explicit `!!str` makes a string; `!!null`, `!!bool`, `!!int` and
//!   `!!float` take their content (in any style: a tag is a node property,
//!   §6.9.1) and fail when it is not one of that tag's core forms
//!   (`!!int "0x1F"` is 31, `!!bool True` true, `!!int 1.5` an error).
//!   `!!seq` and `!!map` must be on a node of that kind. Every other tag,
//!   including the yaml.org types outside the core schema (`!!binary`,
//!   `!!timestamp`, `!!set`), is unrecognized (§10.3.1), so the node has no
//!   JSON value (§3.3.3): an error.
//! - Integers and floats of any size become the nearest IEEE-754 double,
//!   as `JSON.parse` reads the same digits (ECMA-262 `Number`); beyond
//!   ±1.8e308 that is ±Infinity, which `JSON.stringify` prints as `null`,
//!   as it does `.inf` and `.nan`.
//!
//! What JSON can hold decides the rest: one document (an empty stream or
//! several documents are errors), mapping keys turned into JSON strings
//! (`1` is `"1"`), collection keys refused, and keys equal as strings
//! refused as duplicates (`1` and `'1'`; YAML itself requires unique keys,
//! §3.2.1.1). Aliases are shared rather than copied while loading.
//!
//! Resource limits (ours, not the spec's): collections nest at most
//! [`MAX_DEPTH`] (5,000, the nesting JSON input is allowed) deep in the
//! document; the parser then stops reading. Values built through aliases
//! count toward [`MAX_NODES`] and nest at most [`MAX_VALUE_DEPTH`], so alias
//! bombs fail instead of exhausting memory or the stack. saphyr-parser
//! has a limit of its own: at most 255 open flow collections ("recursion
//! limit exceeded").
//!
//! Three adapter fixes make saphyr-parser 0.2.2 read valid input as the
//! spec does:
//!
//! - A block scalar with no content lines at the end of the input
//!   (`a: |\n\n`, `--- >\n`, `- |+\n`) comes back as `"\n"`; by §8.1.1.2
//!   clipping and stripping give `""` and keeping gives one line feed per
//!   empty line after the header. Such scalars are recognised by their
//!   position (saphyr reports them at their `|`/`>` header, scalars with
//!   content at their first content line), and the header's chomping
//!   indicator decides the text.
//! - A block scalar at the document root counts its indentation indicator
//!   from the root's indentation, -1 (§9.1.3, rule 207; §8.1.1.1); saphyr
//!   counts from 0. Indicators 2 to 9 are lowered by one in the input so it
//!   reads the same content.
//! - saphyr-parser never returns when the input ends in a line starting
//!   with `%` (a directive to its scanner) with no line break after it
//!   (`%x`, `a: 1\n%x`); a final line break is added, which changes nothing
//!   else there.
//!
//! Upstream bugs left (saphyr-parser 0.2.2, at the parse level, where the
//! adapter cannot see the source structure):
//!
//! - A tab as separation after `:` or `-` (`a:\tb`) is an error ("':' must
//!   be followed by a valid YAML whitespace"); `s-separate-in-line` allows
//!   tabs (§6.2, rule 66).
//! - `|1`/`>1` at the document root means content at column 0 (§9.1.3);
//!   saphyr expects column 1, so `--- |1\nx` fails and `--- |1\n  x` reads
//!   `" x\n"` instead of `"  x\n"`.
//! - yaml.org tags on collections are dropped by the loader before the node
//!   reaches us, so `!!str [a]` and `!!set {a}` load as plain collections
//!   instead of failing. (Verbatim `!<tag:yaml.org,2002:…>` tags are kept
//!   and checked.)
//! - Error positions count characters, though saphyr's message says
//!   "byte".
//!
//! # JSON to YAML
//!
//! `utilify-yaml-emit` writes the value with its own style choices
//! (`StyleMode::Prefer`, no requests: every scalar gets a style that reads
//! back as the same value, §5 to §9) under its YAML 1.1-safe schema
//! (`Schema::Yaml11Safe`). The output is YAML 1.2.2 and loads back to the
//! same JSON under the core schema; the YAML 1.1-safe profile also quotes
//! strings YAML 1.1 loaders (libyaml, PyYAML, Go yaml.v2, many YAML 1.2
//! loaders that keep 1.1 types) read as other types (`yes`, `on`, `0777`,
//! `1_000`, `2001-12-14`, `<<`), so the output means the same to them too.
//! That costs a few quotes and nothing in the core reading, and a JSON
//! converter's output is usually fed to such tools. Numbers are written as
//! ECMA-262 prints them, with `.0` before an exponent (`1.0e+21`) and
//! `-0.0`, which are core floats and integers (§10.3.2) and YAML 1.1 ones.
//!
//! Presentation (no spec defines it): two-space indentation, block
//! collections, literal and folded block scalars, folding at 80 columns
//! counted in characters. Strings at the document root that would need an
//! indentation indicator are double-quoted, and root keys that look like
//! document markers (`---`, `... a`) quoted (§9.1.2): the emitter does both.
//!
//! Both ops run on a large stack (`with_big_stack`): JSON input may nest
//! 5,000 deep, and the loader, the conversion and the event walk recurse
//! once per level. The emitter itself is iterative.

use super::CATEGORY;
use super::js::{Js, Obj, json_dish, json_input, with_big_stack};
use crate::{Dish, OpError, Operation};
use num_traits::Num;
use saphyr::{
    AnnotatedMapping, LoadableYamlNode, Marker, ScalarStyle, ScanError, Tag, Yaml, YamlLoader,
};
use saphyr_parser::{BufferedInput, Parser, Span};
use std::borrow::Cow;
use std::cell::Cell;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use utilify_yaml_emit::{
    self as emit, Collection, Emitter, Error, Event, Options, ScalarValue, Schema,
};

pub(super) fn ops() -> Vec<&'static Operation> {
    vec![&YAML_TO_JSON, &JSON_TO_YAML]
}

pub static YAML_TO_JSON: Operation = Operation {
    name: "YAML to JSON",
    category: CATEGORY,
    description: "Convert YAML to JSON",
    info_url: Some("https://en.wikipedia.org/wiki/YAML"),
    args: &[],
    run: |input, _| {
        let text = input.text();
        with_big_stack(|| {
            let v = load(&text).map_err(|e| OpError::new(format!("Unable to parse YAML: {e}")))?;
            json_dish(&v)
        })
    },
};

pub static JSON_TO_YAML: Operation = Operation {
    name: "JSON to YAML",
    category: CATEGORY,
    description: "Format a JSON object into YAML",
    info_url: Some("https://en.wikipedia.org/wiki/YAML"),
    args: &[],
    run: |input, _| with_big_stack(|| dump(&json_input(&input)?).map(Dish::Text)),
};

// ---------------------------------------------------------------------------
// Loader

/// How deep collections may nest in the document: the nesting JSON input
/// is allowed. A resource limit (the loader and the conversion recurse per
/// level), not a YAML rule.
const MAX_DEPTH: usize = 5_000;

/// Values built from aliases count toward this, so alias bombs fail
/// instead of exhausting memory.
const MAX_NODES: usize = 5_000_000;

/// Aliases can nest values deeper than the document does; past this the
/// value is refused rather than recursed into.
const MAX_VALUE_DEPTH: usize = 10_000;

thread_local! {
    /// Collections open in the `saphyr` loader while it builds `YNode`s.
    static OPEN: Cell<usize> = const { Cell::new(0) };
    /// Set when a collection is nested past `MAX_DEPTH`; the input then
    /// ends early, so the parser stops there.
    static TOO_DEEP: Cell<bool> = const { Cell::new(false) };
}

/// A scalar as the parser read it: resolved later, in `Convert`.
#[derive(Clone, Debug, PartialEq)]
struct RawScalar<'a> {
    text: Cow<'a, str>,
    style: ScalarStyle,
    tag: Option<Cow<'a, Tag>>,
}

/// The node `saphyr`'s loader builds for us. Collections and scalars are
/// shared (`Rc`), so an alias costs one pointer. Every node carries its
/// position, and nodes compare by position (plus content), so the loader's
/// own duplicate-key check only sees an alias used twice; `Convert` checks
/// keys by their JSON string.
#[derive(Clone, Debug)]
enum YNode<'a> {
    /// The loader's "no key yet" placeholder (`Yaml::BadValue` unplaced).
    Hole,
    /// An alias to a collection still being built (a recursive alias).
    Bad(Marker),
    Scalar(Rc<RawScalar<'a>>, Marker),
    /// A collection with a tag outside the core schema.
    Tagged(Cow<'a, Tag>, Rc<YNode<'a>>),
    Seq(Rc<Vec<YNode<'a>>>, Marker),
    Map(Rc<AnnotatedMapping<'a, YNode<'a>>>, Marker),
}

impl YNode<'_> {
    fn marker(&self) -> Marker {
        match self {
            YNode::Hole => Marker::default(),
            YNode::Tagged(_, n) => n.marker(),
            YNode::Bad(m) | YNode::Scalar(_, m) | YNode::Seq(_, m) | YNode::Map(_, m) => *m,
        }
    }

    fn at(self, m: Marker) -> Self {
        match self {
            YNode::Hole | YNode::Bad(_) => YNode::Bad(m),
            YNode::Scalar(s, _) => YNode::Scalar(s, m),
            YNode::Seq(s, _) => YNode::Seq(s, m),
            YNode::Map(o, _) => YNode::Map(o, m),
            YNode::Tagged(t, n) => YNode::Tagged(t, Rc::new(Rc::unwrap_or_clone(n).at(m))),
        }
    }
}

impl PartialEq for YNode<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.marker() == other.marker()
            && match (self, other) {
                (YNode::Hole, YNode::Hole) | (YNode::Bad(_), YNode::Bad(_)) => true,
                (YNode::Scalar(a, _), YNode::Scalar(b, _)) => a == b,
                (YNode::Tagged(t, a), YNode::Tagged(u, b)) => t == u && Rc::ptr_eq(a, b),
                (YNode::Seq(a, _), YNode::Seq(b, _)) => Rc::ptr_eq(a, b),
                (YNode::Map(a, _), YNode::Map(b, _)) => Rc::ptr_eq(a, b),
                _ => false,
            }
    }
}

impl Eq for YNode<'_> {}

impl Hash for YNode<'_> {
    fn hash<H: Hasher>(&self, h: &mut H) {
        let m = self.marker();
        (m.index(), m.line(), m.col()).hash(h);
    }
}

impl<'a> LoadableYamlNode<'a> for YNode<'a> {
    type HashKey = YNode<'a>;

    fn from_bare_yaml(yaml: Yaml<'a>) -> Self {
        match yaml {
            Yaml::Representation(text, style, tag) => {
                YNode::Scalar(Rc::new(RawScalar { text, style, tag }), Marker::default())
            }
            Yaml::Sequence(_) => YNode::Seq(Rc::default(), Marker::default()),
            Yaml::Mapping(_) => YNode::Map(Rc::default(), Marker::default()),
            Yaml::Tagged(t, n) => YNode::Tagged(t, Rc::new(YNode::from_bare_yaml(*n))),
            // With `early_parse` off scalars arrive as `Representation`;
            // aliases arrive resolved, or as `BadValue`.
            Yaml::Value(_) | Yaml::Alias(_) | Yaml::BadValue => YNode::Hole,
        }
    }

    fn is_sequence(&self) -> bool {
        matches!(self, YNode::Seq(..))
    }

    fn is_mapping(&self) -> bool {
        matches!(self, YNode::Map(..))
    }

    fn is_badvalue(&self) -> bool {
        matches!(self, YNode::Hole)
    }

    fn sequence_mut(&mut self) -> &mut Vec<Self> {
        if !self.is_sequence() {
            // The loader only calls this on sequences.
            *self = YNode::Seq(Rc::default(), self.marker());
        }
        match self {
            YNode::Seq(items, _) => Rc::make_mut(items),
            _ => unreachable!(),
        }
    }

    fn mapping_mut(&mut self) -> &mut AnnotatedMapping<'a, Self> {
        if !self.is_mapping() {
            // The loader only calls this on mappings.
            *self = YNode::Map(Rc::default(), self.marker());
        }
        match self {
            YNode::Map(map, _) => Rc::make_mut(map),
            _ => unreachable!(),
        }
    }

    fn into_tagged(self, tag: Cow<'a, Tag>) -> Self {
        YNode::Tagged(tag, Rc::new(self))
    }

    fn take(&mut self) -> Self {
        std::mem::replace(self, YNode::Hole)
    }

    /// Scalars and aliases. An alias keeps its target's position (a
    /// scalar's is where it was defined); an alias to a collection still
    /// open is a recursive alias.
    fn with_span(self, span: Span) -> Self {
        match self {
            YNode::Scalar(s, m) if m.line() == 0 => YNode::Scalar(s, span.start),
            YNode::Hole => YNode::Bad(span.start),
            other => other,
        }
    }

    fn with_start_marker(self, mark: Marker) -> Self {
        if OPEN.get() >= MAX_DEPTH {
            TOO_DEEP.set(true);
        }
        OPEN.set(OPEN.get() + 1);
        self.at(mark)
    }

    fn with_end_marker(self, _: Marker) -> Self {
        OPEN.set(OPEN.get().saturating_sub(1));
        self
    }
}

/// An error at `m`, positioned like `saphyr`'s own errors when the node's
/// position is known.
fn error_at(m: Marker, reason: &str) -> String {
    if m.line() == 0 {
        reason.into()
    } else {
        ScanError::new_str(m, reason).to_string()
    }
}

const YAML_ORG: &str = "tag:yaml.org,2002:";

// --- core schema resolution (§10.3.2) ---

fn all_digits(s: &str, radix: u32) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_digit(radix))
}

fn core_null(s: &str) -> bool {
    matches!(s, "" | "~" | "null" | "Null" | "NULL")
}

fn core_bool(s: &str) -> Option<bool> {
    match s {
        "true" | "True" | "TRUE" => Some(true),
        "false" | "False" | "FALSE" => Some(false),
        _ => None,
    }
}

/// The nearest double to the unsigned integer `digits` in `radix`.
fn radix_to_f64(digits: &str, radix: u32) -> f64 {
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return 0.0;
    }
    if let Ok(v) = u128::from_str_radix(digits, radix) {
        return v as f64;
    }
    // More than 1,025 bits is beyond every finite double.
    let bits_per_digit = if radix == 16 { 4 } else { 3 };
    if digits.len().saturating_mul(bits_per_digit) > 1029 {
        return f64::INFINITY;
    }
    num_bigint::BigUint::from_str_radix(digits, radix)
        .ok()
        .and_then(|n| n.to_string().parse().ok())
        .unwrap_or(f64::INFINITY)
}

/// The core schema's `!!int`: `[-+]?[0-9]+`, `0o[0-7]+`, `0x[0-9a-fA-F]+`.
fn core_int(s: &str) -> Option<f64> {
    if let Some(d) = s.strip_prefix("0o") {
        return all_digits(d, 8).then(|| radix_to_f64(d, 8));
    }
    if let Some(d) = s.strip_prefix("0x") {
        return all_digits(d, 16).then(|| radix_to_f64(d, 16));
    }
    let body = s.strip_prefix(['-', '+']).unwrap_or(s);
    // Rust's float parser rounds decimal digits of any length correctly.
    all_digits(body, 10).then(|| s.parse().unwrap_or(f64::NAN))
}

/// `[-+]?(\.[0-9]+|[0-9]+(\.[0-9]*)?)([eE][-+]?[0-9]+)?`.
fn core_float_number(s: &str) -> bool {
    let body = s.strip_prefix(['-', '+']).unwrap_or(s);
    let (mant, exp) = match body.find(['e', 'E']) {
        Some(i) => (&body[..i], Some(&body[i + 1..])),
        None => (body, None),
    };
    if let Some(e) = exp
        && !all_digits(e.strip_prefix(['+', '-']).unwrap_or(e), 10)
    {
        return false;
    }
    match mant.split_once('.') {
        None => all_digits(mant, 10),
        Some(("", f)) => all_digits(f, 10),
        Some((i, f)) => all_digits(i, 10) && (f.is_empty() || all_digits(f, 10)),
    }
}

/// The core schema's `!!float`.
fn core_float(s: &str) -> Option<f64> {
    match s {
        ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF" => Some(f64::INFINITY),
        "-.inf" | "-.Inf" | "-.INF" => Some(f64::NEG_INFINITY),
        ".nan" | ".NaN" | ".NAN" => Some(f64::NAN),
        // Rust's parser takes every text the pattern allows (`1.`, `.5`,
        // `1.E5`) and rounds to the nearest double.
        _ if core_float_number(s) => Some(s.parse().unwrap_or(f64::NAN)),
        _ => None,
    }
}

/// An untagged plain scalar.
fn resolve_plain(s: &str) -> Js {
    if core_null(s) {
        Js::Null
    } else if let Some(b) = core_bool(s) {
        Js::Bool(b)
    } else if let Some(n) = core_int(s).or_else(|| core_float(s)) {
        Js::Num(n)
    } else {
        Js::Str(s.into())
    }
}

/// The JSON value of the loaded nodes.
struct Convert<'t> {
    nodes: usize,
    /// The text the parser read, for the empty block scalar fix.
    text: &'t str,
    /// Byte offset of each character of `text` (saphyr's positions count
    /// characters), built on first use.
    offsets: Option<Vec<usize>>,
}

impl Convert<'_> {
    fn value(&mut self, n: &YNode, at: Marker, depth: usize) -> Result<Js, String> {
        let at = match n.marker() {
            m if m.line() == 0 => at,
            m => m,
        };
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(error_at(at, "aliases expand to too large a document"));
        }
        if depth > MAX_VALUE_DEPTH {
            return Err(error_at(at, "aliases nest too deeply"));
        }
        match n {
            YNode::Hole | YNode::Bad(_) => Err(error_at(
                at,
                "a recursive alias (an alias inside the node it refers to) has no JSON value",
            )),
            YNode::Scalar(s, _) => self.scalar(s, at),
            YNode::Tagged(tag, inner) => {
                let name = format!("{}{}", tag.handle, tag.suffix);
                let kind_tag = match &**inner {
                    YNode::Seq(..) => "tag:yaml.org,2002:seq",
                    YNode::Map(..) => "tag:yaml.org,2002:map",
                    _ => "",
                };
                // `saphyr` reads `!` as the handle `""` with suffix `!`, and
                // a verbatim tag `!<uri>` as suffix `uri`.
                if name == "!" || name == kind_tag {
                    self.value(inner, at, depth)
                } else {
                    Err(unknown_tag(at, &name, "a collection"))
                }
            }
            YNode::Seq(items, _) => items
                .iter()
                .map(|item| self.value(item, at, depth + 1))
                .collect::<Result<_, _>>()
                .map(Js::Arr),
            YNode::Map(pairs, _) => {
                let mut obj = Obj::new();
                for (k, v) in pairs.iter() {
                    let key_at = match k.marker() {
                        m if m.line() == 0 => at,
                        m => m,
                    };
                    let key = self.value(k, at, depth + 1)?;
                    if key.is_object_like() {
                        return Err(error_at(
                            key_at,
                            "a sequence or mapping as a key has no JSON form",
                        ));
                    }
                    let name = key.to_js_string();
                    if obj.get(&name).is_some() {
                        return Err(error_at(key_at, "duplicated mapping key"));
                    }
                    let v = self.value(v, at, depth + 1)?;
                    obj.insert(name, v);
                }
                Ok(Js::Obj(obj))
            }
        }
    }

    /// A scalar resolved by its tag, or by its style and text (§10.3.2).
    fn scalar(&mut self, s: &RawScalar, at: Marker) -> Result<Js, String> {
        let text = self.block_text(s, at);
        let Some(tag) = &s.tag else {
            return Ok(match s.style {
                ScalarStyle::Plain => resolve_plain(&text),
                _ => Js::Str(text.into_owned()),
            });
        };
        let name = format!("{}{}", tag.handle, tag.suffix);
        let invalid = |kind: &str| error_at(at, &format!("the scalar is not a valid !!{kind}"));
        match name.strip_prefix(YAML_ORG) {
            _ if name == "!" => Ok(Js::Str(text.into_owned())),
            Some("str") => Ok(Js::Str(text.into_owned())),
            Some("null") if core_null(&text) => Ok(Js::Null),
            Some("bool") => core_bool(&text)
                .map(Js::Bool)
                .ok_or_else(|| invalid("bool")),
            Some("int") => core_int(&text).map(Js::Num).ok_or_else(|| invalid("int")),
            Some("float") => core_float(&text)
                .map(Js::Num)
                .ok_or_else(|| invalid("float")),
            Some("null") => Err(invalid("null")),
            _ => Err(unknown_tag(at, &name, "a scalar")),
        }
    }

    /// The scalar's text, with saphyr's empty block scalar bug fixed: a
    /// block scalar without content lines at the end of the input is
    /// `"\n"` (or more line feeds) there, and saphyr reports it at its
    /// header rather than at a content line.
    fn block_text<'s>(&mut self, s: &'s RawScalar, at: Marker) -> Cow<'s, str> {
        let text = Cow::Borrowed(&*s.text);
        if !matches!(s.style, ScalarStyle::Literal | ScalarStyle::Folded)
            || s.text.is_empty()
            || s.text.bytes().any(|b| b != b'\n')
            || at.line() == 0
        {
            return text;
        }
        let source = self.text;
        let offsets = self
            .offsets
            .get_or_insert_with(|| source.char_indices().map(|(i, _)| i).collect());
        let Some(rest) = offsets.get(at.index()).map(|&i| &source[i..]) else {
            return text;
        };
        let Some(header) = rest.strip_prefix(['|', '>']) else {
            return text;
        };
        // The indicators (`+`, `-`, a digit) come before any space or comment.
        let indicators = header
            .find(|c: char| !matches!(c, '+' | '-' | '0'..='9'))
            .map_or(header, |i| &header[..i]);
        if !indicators.contains('+') {
            // Clip and strip leave nothing of empty lines (§8.1.1.2).
            return Cow::Borrowed("");
        }
        // Keep: one line feed per line after the header, up to the first
        // line with anything but spaces. saphyr gets this right except
        // when there is no such line.
        let after_header = header.find(['\n', '\r']).map_or("", |i| &header[i..]);
        let lines = after_header
            .strip_prefix("\r\n")
            .or_else(|| after_header.strip_prefix(['\n', '\r']))
            .unwrap_or("");
        let empty_lines = lines
            .split_inclusive(['\n', '\r'])
            .take_while(|l| {
                l.trim_end_matches(['\n', '\r']).bytes().all(|b| b == b' ')
                    && l.ends_with(['\n', '\r'])
            })
            .count();
        if empty_lines == 0 {
            Cow::Borrowed("")
        } else {
            text
        }
    }
}

fn unknown_tag(at: Marker, name: &str, on: &str) -> String {
    let shown = match name.strip_prefix(YAML_ORG) {
        Some(short) => format!("!!{short}"),
        None => format!("!<{name}>"),
    };
    error_at(
        at,
        &format!("the tag {shown} is not a core schema tag for {on}"),
    )
}

/// One document, as a JSON value.
fn load(text: &str) -> Result<Js, String> {
    OPEN.set(0);
    TOO_DEEP.set(false);
    // A byte order mark at the start of the stream is not content (§5.2).
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    // saphyr-parser 0.2.2 never returns when the input ends in a line that
    // starts with `%` (a directive to its scanner) with no line break after
    // it (`%x`, `a: 1\n%x`). A final line break changes nothing else there.
    let last_line = text.rfind(['\n', '\r']).map_or(text, |i| &text[i + 1..]);
    let text: Cow<str> = if last_line.starts_with('%') {
        format!("{text}\n").into()
    } else {
        text.into()
    };
    let text = root_block_indent(&text);
    let mut loader = YamlLoader::<YNode>::default();
    loader.early_parse(false);
    let mut parser = Parser::new(BufferedInput::new(
        text.chars().take_while(|_| !TOO_DEEP.get()),
    ));
    let parsed = parser.load(&mut loader, true);
    if TOO_DEEP.get() {
        return Err(format!(
            "collections nest deeper than {MAX_DEPTH} levels (a resource limit)"
        ));
    }
    parsed.map_err(|e| e.to_string())?;
    if let Some(e) = loader.error() {
        return Err(e.to_string());
    }
    let docs = loader.into_documents();
    let mut convert = Convert {
        nodes: 0,
        text: &text,
        offsets: None,
    };
    match docs.as_slice() {
        [] => Err("expected a document, but the input is empty".into()),
        [doc] => convert.value(doc, Marker::default(), 0),
        _ => Err("expected a single document in the stream, but found more".into()),
    }
}

/// A block scalar at the document root counts its indentation indicator
/// from the root's indentation, -1 (YAML 1.2.2 §9.1.3, rule 207, and
/// §8.1.1.1); `saphyr` counts from 0. Lower the indicator by one so it
/// reads the same content. (`|1`, content at column 0, can't be lowered.)
fn root_block_indent(text: &str) -> Cow<'_, str> {
    // Skip comments, directives, blank lines and a `---` marker.
    let mut rest = text;
    while !rest.is_empty() {
        let line_end = rest.find('\n').map_or(rest.len(), |i| i + 1);
        let line = &rest[..line_end];
        let mut t = line.trim_start_matches([' ', '\t']);
        if line.starts_with('%') {
            t = "";
        } else if line.starts_with("---") && line[3..].starts_with([' ', '\t', '\r', '\n']) {
            t = line[3..].trim_start_matches([' ', '\t']);
        }
        if !(t.is_empty() || t.starts_with(['#', '\r', '\n'])) {
            rest = &rest[line.len() - t.len()..];
            break;
        }
        rest = &rest[line_end..];
    }
    let start = text.len() - rest.len();
    let header: Vec<char> = rest.chars().take(3).collect();
    if !matches!(header.first(), Some('|' | '>')) {
        return text.into();
    }
    let digit = match header.get(1..) {
        Some([d, ..]) | Some(['+' | '-', d, ..]) if ('2'..='9').contains(d) => *d,
        _ => return text.into(),
    };
    let at = start + rest[1..].find(digit).map_or(0, |i| i + 1);
    let lowered = char::from(digit as u8 - 1);
    format!("{}{lowered}{}", &text[..at], &text[at + 1..]).into()
}

// ---------------------------------------------------------------------------
// Dumper

/// The emitter's YAML 1.1-safe profile with two-space indentation and
/// 80-column folding, and no limits of its own: JSON input is already in
/// memory and nests up to 5,000 deep, and object keys are unique.
const OPTIONS: Options = Options::YAML_1_2
    .schema(Schema::Yaml11Safe)
    .indent(2)
    .line_width(Some(80))
    .max_depth(usize::MAX)
    .max_event_len(usize::MAX)
    .max_rendered_len(usize::MAX)
    .unique_keys(false);

fn write_node(e: &mut Emitter<String>, v: &Js) -> Result<(), Error> {
    match v {
        Js::Undefined | Js::Null => e.emit(emit::Scalar::null()),
        Js::Bool(b) => e.emit(emit::Scalar::bool(*b)),
        Js::Num(n) => e.emit(emit::Scalar::new(ScalarValue::Number(*n))),
        Js::Str(s) => e.emit(emit::Scalar::str(s)),
        Js::Arr(items) => {
            e.emit(Event::SequenceStart(Collection::new()))?;
            for item in items {
                write_node(e, item)?;
            }
            e.emit(Event::SequenceEnd)
        }
        Js::Obj(o) => {
            e.emit(Event::MappingStart(Collection::new()))?;
            for (k, v) in o.iter() {
                e.emit(emit::Scalar::str(&k))?;
                write_node(e, v)?;
            }
            e.emit(Event::MappingEnd)
        }
    }
}

/// A JSON value as a YAML document.
fn dump(v: &Js) -> Result<String, OpError> {
    let mut e = Emitter::with_options(String::new(), OPTIONS);
    write_node(&mut e, v)
        .and_then(|()| e.finish())
        .map_err(|e| OpError::new(format!("Unable to write YAML: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_json(yaml: &str) -> String {
        YAML_TO_JSON
            .apply(Dish::Text(yaml.into()), &[])
            .map_or_else(|e| e.0, Dish::into_text)
    }

    fn to_yaml(json: &str) -> String {
        JSON_TO_YAML
            .apply(Dish::Text(json.into()), &[])
            .map_or_else(|e| e.0, Dish::into_text)
    }

    #[test]
    fn loads_scalars_and_collections() {
        assert_eq!(
            to_json("number: 3\nplain: string\nblock: |\n  two\n  lines"),
            "{\n    \"number\": 3,\n    \"plain\": \"string\",\n    \"block\": \"two\\nlines\\n\"\n}"
        );
        assert_eq!(
            to_json("[a: 1, b]"),
            "[\n    {\n        \"a\": 1\n    },\n    \"b\"\n]"
        );
        assert_eq!(
            to_json("- >\n  a\n  b\n\n  c\n"),
            "[\n    \"a b\\nc\\n\"\n]"
        );
        assert_eq!(to_json("- a\n -b"), "[\n    \"a -b\"\n]");
        assert_eq!(
            to_json("a: &x {b: 1}\nc: *x"),
            "{\n    \"a\": {\n        \"b\": 1\n    },\n    \"c\": {\n        \"b\": 1\n    }\n}"
        );
        // A byte order mark is not content (§5.2); `...` may be followed by
        // a comment (§9.1.2).
        assert_eq!(to_json("\u{feff}a\n... # end\n"), "\"a\"");
        assert_eq!(to_json("---\n"), "null");
    }

    /// YAML 1.2.2 Example 10.9, "Core Tag Resolution".
    #[test]
    fn core_schema_resolution() {
        let doc = "A null: null\nAlso a null: # Empty\nNot a null: \"\"\n\
                   Booleans: [ true, True, false, FALSE ]\n\
                   Integers: [ 0, 0o7, 0x3A, -19 ]\n\
                   Floats: [ 0., -0.0, .5, +12e03, -2E+05 ]\n\
                   Also floats: [ .inf, -.Inf, +.INF, .NAN ]\n";
        assert_eq!(
            super::super::js::stringify_compact(&load(doc).unwrap())
                .unwrap()
                .unwrap(),
            r#"{"A null":null,"Also a null":null,"Not a null":"","Booleans":[true,true,false,false],"Integers":[0,7,58,-19],"Floats":[0,0,0.5,12000,-200000],"Also floats":[null,null,null,null]}"#
        );
        let flat = |y: &str| {
            load(y)
                .map(|v| super::super::js::stringify_compact(&v).unwrap().unwrap())
                .unwrap_or_else(|e| e)
        };
        // Not in the table: strings.
        assert_eq!(
            flat(
                "[+-1, ++7, 0x-1F, +0x1F, -0o7, 0o8, 0x, 1_000, 1e, .e1, ., 2001-12-14, yes, <<, .NaN0, -.nan]"
            ),
            r#"["+-1","++7","0x-1F","+0x1F","-0o7","0o8","0x","1_000","1e",".e1",".","2001-12-14","yes","<<",".NaN0","-.nan"]"#
        );
        // Any size, to the nearest double.
        assert_eq!(
            flat(
                "[18446744073709551617, 0x10000000000000000, 0o2000000000000000000000, 1e309, -1e309, 1e-400]"
            ),
            r#"[18446744073709552000,18446744073709552000,18446744073709552000,null,null,0]"#
        );
        assert_eq!(flat(&format!("0x{}", "f".repeat(300))), "null");
        assert_eq!(flat(&format!("0x{}1", "0".repeat(300))), "1");
        // Quoted, block and `!` scalars are strings.
        assert_eq!(
            flat("['1', \"true\", ! 0x1F, ! ]"),
            r#"["1","true","0x1F",""]"#
        );
    }

    #[test]
    fn explicit_tags() {
        let flat = |y: &str| {
            load(y)
                .map(|v| super::super::js::stringify_compact(&v).unwrap().unwrap())
                .unwrap_or_else(|e| e)
        };
        assert_eq!(
            flat(
                "[!!int \"0x1F\", !!int '1', !!float 1, !!bool True, !!null '', !!null ~, !!str 1, !!str, !<tag:yaml.org,2002:int> 7]"
            ),
            r#"[31,1,1,true,null,null,"1","",7]"#
        );
        // `!` on a collection: a sequence or mapping by kind (§6.9.1).
        assert_eq!(flat("! [1, ! {a: ! 1}]"), r#"[1,{"a":"1"}]"#);
        for (y, e) in [
            (
                "!!int 1.5",
                "the scalar is not a valid !!int at byte 6 line 1 column 7",
            ),
            (
                "!!float 0x1F",
                "the scalar is not a valid !!float at byte 8 line 1 column 9",
            ),
            (
                "!!bool yes",
                "the scalar is not a valid !!bool at byte 7 line 1 column 8",
            ),
            (
                "!!null 0",
                "the scalar is not a valid !!null at byte 7 line 1 column 8",
            ),
            (
                "!!binary aGVsbG8=",
                "the tag !!binary is not a core schema tag for a scalar at byte 9 line 1 column 10",
            ),
            (
                "!!seq a",
                "the tag !!seq is not a core schema tag for a scalar at byte 6 line 1 column 7",
            ),
            (
                "!foo 1",
                "the tag !<!foo> is not a core schema tag for a scalar at byte 5 line 1 column 6",
            ),
            (
                "!<tag:yaml.org,2002:map> [a]",
                "the tag !!map is not a core schema tag for a collection at byte 25 line 1 column 26",
            ),
            (
                "!foo [a]",
                "the tag !<!foo> is not a core schema tag for a collection at byte 5 line 1 column 6",
            ),
        ] {
            assert_eq!(flat(y), e, "{y}");
        }
    }

    #[test]
    fn load_errors() {
        // `saphyr`'s messages and positions.
        assert_eq!(
            to_json("key: [1, 2"),
            "Unable to parse YAML: while parsing a flow sequence, expected ',' or ']' at byte 10 line 2 column 1"
        );
        assert_eq!(
            to_json("a\n... x"),
            "Unable to parse YAML: invalid content after document end marker at byte 6 line 2 column 5"
        );
        // Ours: keys equal as JSON strings are duplicates.
        assert_eq!(
            to_json("a: 1\na: 2"),
            "Unable to parse YAML: duplicated mapping key at byte 5 line 2 column 1"
        );
        assert_eq!(
            to_json("1: a\n'1': b"),
            "Unable to parse YAML: duplicated mapping key at byte 5 line 2 column 1"
        );
        assert_eq!(
            to_json("0x1: a\n1: b"),
            "Unable to parse YAML: duplicated mapping key at byte 7 line 2 column 1"
        );
        assert_eq!(
            to_json("? [1]\n: 2"),
            "Unable to parse YAML: a sequence or mapping as a key has no JSON form at byte 2 line 1 column 3"
        );
        assert_eq!(
            to_json(""),
            "Unable to parse YAML: expected a document, but the input is empty"
        );
        assert_eq!(
            to_json("--- 1\n--- 2"),
            "Unable to parse YAML: expected a single document in the stream, but found more"
        );
        assert_eq!(
            to_json("&a [*a]"),
            "Unable to parse YAML: a recursive alias (an alias inside the node it refers to) has no JSON value at byte 4 line 1 column 5"
        );
    }

    /// Valid input saphyr-parser 0.2.2 misreads, fixed in the adapter.
    #[test]
    fn saphyr_fixes() {
        // A block scalar with no content lines (§8.1.1.2).
        for (y, v) in [
            ("a: |\n\n", "{\n    \"a\": \"\"\n}"),
            ("a: >  # c\n  \n\n", "{\n    \"a\": \"\"\n}"),
            ("a: |2-\n\n", "{\n    \"a\": \"\"\n}"),
            ("--- >\n", "\"\""),
            ("|\n", "\"\""),
            ("- |+\n", "[\n    \"\"\n]"),
            ("- |+\n\n", "[\n    \"\\n\"\n]"),
            ("- |+\n\n  \n", "[\n    \"\\n\\n\"\n]"),
            ("- |+\n\n- x", "[\n    \"\\n\",\n    \"x\"\n]"),
            ("- |\n\n- x", "[\n    \"\",\n    \"x\"\n]"),
            ("&a |\n\n", "\"\""),
        ] {
            assert_eq!(to_json(y), v, "{y:?}");
        }
        // A root block scalar's indentation indicator counts from -1
        // (§9.1.3).
        assert_eq!(to_json("|3-\n   a\n  b\n"), "\" a\\nb\"");
        assert_eq!(to_json("|2-\n   a\n  b\n"), "\"  a\\n b\"");
        assert_eq!(to_json("- |\n  null\n"), "[\n    \"null\\n\"\n]");
        assert_eq!(to_json("# c\n--- >2\n  a\n"), "\" a\\n\"");
        assert_eq!(to_json("a: |2\n   x\n"), "{\n    \"a\": \" x\\n\"\n}");
        // A final `%` line used to hang saphyr-parser.
        for y in ["%", "%x", "%x # c", "a: 1\n%x", "a: |\n  x\n%"] {
            assert!(to_json(y).starts_with("Unable to parse YAML: "), "{y}");
        }
        assert_eq!(to_json("a\n%b"), "\"a %b\"");
    }

    /// Valid input saphyr-parser 0.2.2 still misreads (see the module
    /// docs). These pin today's behaviour, so an upgrade that fixes them
    /// shows up here.
    #[test]
    fn upstream_saphyr_bugs() {
        // §6.2: tabs separate too; the spec reads `{"a": "b"}`.
        assert_eq!(
            to_json("a:\tb"),
            "Unable to parse YAML: ':' must be followed by a valid YAML whitespace at byte 3 line 1 column 4"
        );
        // §9.1.3: `|1` at the root is content at column 0; the spec reads
        // "x\n" and "  x\n".
        assert!(to_json("--- |1\nx\n").starts_with("Unable to parse YAML: "));
        assert_eq!(to_json("--- |1\n  x\n"), "\" x\\n\"");
        // yaml.org tags on collections are dropped before we see them;
        // the spec makes `!!str [a]` an error.
        assert_eq!(to_json("!!str [a]"), "[\n    \"a\"\n]");
    }

    #[test]
    fn load_limits() {
        // Block collections (`- - 1`, as JSON to YAML writes deep arrays).
        let block = |n: usize| format!("{}1", "- ".repeat(n));
        assert!(to_json(&block(MAX_DEPTH)).starts_with("[\n    [\n"));
        assert_eq!(
            to_json(&block(MAX_DEPTH + 1)),
            "Unable to parse YAML: collections nest deeper than 5000 levels (a resource limit)"
        );
        assert!(to_json(&block(200_000)).starts_with("Unable to parse YAML: "));
        // saphyr-parser's own limit: 255 open flow collections.
        let flow = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n));
        assert!(to_json(&flow(255)).starts_with("[\n    [\n"));
        assert_eq!(
            to_json(&flow(256)),
            "Unable to parse YAML: recursion limit exceeded at byte 255 line 1 column 256"
        );
        let block: String = (0..300)
            .map(|i| format!("{}- \n", "  ".repeat(i)))
            .collect();
        assert!(to_json(&block).starts_with("[\n    [\n"));
        // Ten levels of ten aliases: 10^10 values.
        let mut bomb = "a0: &a0 [1, 1, 1, 1, 1, 1, 1, 1, 1, 1]\n".to_string();
        for i in 1..10 {
            let p = format!("*a{}, ", i - 1).repeat(10);
            bomb.push_str(&format!("a{i}: &a{i} [{}]\n", p.trim_end_matches(", ")));
        }
        assert!(to_json(&bomb).contains("aliases expand to too large a document"));
        // Shared values are copied out in full.
        assert_eq!(
            to_json("[&a [1], *a]"),
            "[\n    [\n        1\n    ],\n    [\n        1\n    ]\n]"
        );
    }

    /// The app runs ops on 512 KiB threads; JSON input may nest 5,000 deep.
    #[test]
    fn deep_input_does_not_overflow_small_stacks() {
        let run = |f: fn() -> String| {
            std::thread::Builder::new()
                .stack_size(512 << 10)
                .spawn(f)
                .and_then(|h| h.join().map_err(|_| std::io::Error::other("panicked")))
        };
        let yaml = run(|| {
            let json = format!("{}1{}", "[".repeat(5000), "]".repeat(5000));
            to_yaml(&json)
        });
        let yaml = yaml.expect("JSON to YAML overflowed");
        assert!(yaml.starts_with("- - - "));
        assert!(yaml.ends_with("- 1\n"));
        let obj = run(|| {
            let json = format!("{}1{}", "{\"k\":".repeat(5000), "}".repeat(5000));
            to_yaml(&json)
        });
        assert!(obj.expect("JSON to YAML overflowed").ends_with("k: 1\n"));
        let deep = run(|| {
            to_json(&to_yaml(&format!(
                "{}1{}",
                "[".repeat(5000),
                "]".repeat(5000)
            )))
        });
        assert!(
            deep.expect("YAML to JSON overflowed")
                .starts_with("[\n    [\n")
        );
        let back = run(|| to_json(&format!("{}1", "- ".repeat(100_000))));
        assert!(
            back.expect("YAML to JSON overflowed")
                .starts_with("Unable to parse YAML: ")
        );
    }

    #[test]
    fn dumps_json() {
        assert_eq!(
            to_yaml("{ \"number\": 3, \"plain\": \"string\" }"),
            "number: 3\nplain: string\n"
        );
        assert_eq!(
            to_yaml(
                r#"{"a":[1,{"b":null}],"c":{},"d":[],"e":"yes","f":"multi\nline","g":"","h":-0,"i":1e21,"j":0.5}"#
            ),
            "a:\n  - 1\n  - b: null\nc: {}\nd: []\ne: 'yes'\nf: |-\n  multi\n  line\ng: ''\nh: -0.0\ni: 1.0e+21\nj: 0.5\n"
        );
        assert_eq!(to_yaml(r#"["keep\n\n"]"#), "- |+\n  keep\n\n...\n");
        assert_eq!(to_yaml(r#"["keep\n\n", []]"#), "- |+\n  keep\n\n- []\n");
        assert_eq!(
            to_yaml("{bad"),
            "Error translating from ArrayBuffer to JSON: SyntaxError: Expected property name or '}' in JSON at position 1 (line 1 column 2)"
        );
    }

    /// The emitter's own safe choices where js-yaml wrote YAML that reads
    /// back differently, or only under its own rules.
    #[test]
    fn dump_style_choices() {
        // Document markers at the start of a line are quoted (§9.1.2): js-yaml
        // wrote `--- a: 1`, which starts a new document.
        assert_eq!(
            to_yaml(r#"{"---":1,"--- a":2,"...":3,"... b":{"--- c":4}}"#),
            "'---': 1\n'--- a': 2\n'...': 3\n'... b':\n  --- c: 4\n"
        );
        // A root string that would need an indentation indicator is quoted
        // (§9.1.3 and common parsers disagree on the indicator there).
        assert_eq!(to_yaml(r#"" a\nb""#), "\" a\\nb\"\n");
        assert_eq!(to_yaml(r#"[" a\nb"]"#), "- |2-\n   a\n  b\n");
        // YAML 1.1 types are quoted for YAML 1.1 loaders too.
        assert_eq!(
            to_yaml(r#"["on", "y", "0777", "1_000", "1:20", "2001-12-14", "<<", "=", "1e3", "x"]"#),
            "- 'on'\n- 'y'\n- '0777'\n- '1_000'\n- '1:20'\n- '2001-12-14'\n- '<<'\n- '='\n- '1e3'\n- x\n"
        );
        // Folding counts characters.
        let emoji = ["😀😀"; 30].join(" ");
        let yaml = to_yaml(&format!("[{emoji:?}]"));
        assert!(yaml.starts_with("- >-\n  😀😀 "), "{yaml}");
        assert!(yaml.lines().all(|l| l.chars().count() <= 80), "{yaml}");
        assert_eq!(to_json(&yaml), format!("[\n    {emoji:?}\n]"));
    }

    /// A small deterministic generator (xorshift) for the round-trip corpus.
    struct Gen(u64);

    impl Gen {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }

        fn string(&mut self) -> String {
            const PARTS: &[&str] = &[
                "",
                " ",
                "  ",
                "a",
                "word",
                "yes",
                "No",
                "on",
                "OFF",
                "y",
                "~",
                "null",
                "Null",
                "true",
                "False",
                "-",
                "--",
                "---",
                "...",
                "?",
                ":",
                ",",
                "[",
                "]",
                "{",
                "}",
                "#",
                "&a",
                "*a",
                "!",
                "!!str",
                "|",
                ">",
                "'",
                "\"",
                "%",
                "@",
                "`",
                "=",
                "<<",
                "0",
                "00",
                "0777",
                "0x1F",
                "0o17",
                "0b101",
                "+1",
                "-1",
                "++7",
                "+-1",
                "0x-1",
                "1e3",
                "1.e+21",
                "1.0",
                ".5",
                "1_000",
                "1:20",
                ".inf",
                "-.Inf",
                ".nan",
                "2001-12-14",
                "2001-12-14 21:59:43.10 -5",
                "\n",
                "\n\n",
                "\r",
                "\r\n",
                "\t",
                "\u{85}",
                "\u{a0}",
                "\u{2028}",
                "\u{feff}",
                "\u{1}",
                "\u{7f}",
                "é",
                "😀",
                "\u{e000}",
                ": ",
                " #",
                "- ",
                "? ",
                "a: b",
                "a #b",
            ];
            let n = self.below(5);
            let mut s = String::new();
            for _ in 0..n {
                s.push_str(PARTS[self.below(PARTS.len())]);
                if self.below(4) == 0 {
                    s.push(' ');
                }
            }
            if self.below(20) == 0 {
                // Long enough to fold.
                let words = ["lorem", "ipsum", "😀", "dolor", " ", "x"];
                for _ in 0..40 {
                    s.push_str(words[self.below(words.len())]);
                    s.push(' ');
                }
            }
            s
        }

        fn number(&mut self) -> f64 {
            const NUMS: &[f64] = &[
                0.0,
                -0.0,
                1.0,
                -1.0,
                0.5,
                1.5e-7,
                1e21,
                1e-7,
                5e-324,
                1.7976931348623157e308,
                123456789012345680000.0,
                9007199254740993.0,
                0.1,
                -2.5e-10,
                1e100,
                777.0,
            ];
            if self.below(2) == 0 {
                NUMS[self.below(NUMS.len())]
            } else {
                (self.next() as i64 as f64) / f64::from(1u32 << self.below(32))
            }
        }

        fn value(&mut self, depth: usize) -> Js {
            match self.below(if depth > 4 { 4 } else { 6 }) {
                0 => Js::Null,
                1 => Js::Bool(self.below(2) == 0),
                2 => Js::Num(self.number()),
                3 => Js::Str(self.string()),
                4 => Js::Arr((0..self.below(4)).map(|_| self.value(depth + 1)).collect()),
                _ => {
                    let mut o = Obj::new();
                    for _ in 0..self.below(4) {
                        let k = self.string();
                        o.insert(k, self.value(depth + 1));
                    }
                    Js::Obj(o)
                }
            }
        }
    }

    /// Every dump loads back (saphyr, core schema) to the same JSON.
    #[test]
    fn round_trips_generated_corpus() {
        let mut g = Gen(0x9e37_79b9_7f4a_7c15);
        let json = |v: &Js| json_dish(v).map(Dish::into_text).unwrap_or_else(|e| e.0);
        for i in 0..5000 {
            let v = g.value(0);
            let yaml = dump(&v).unwrap_or_else(|e| panic!("value {i}: {}", e.0));
            let back = load(&yaml).unwrap_or_else(|e| panic!("value {i}: {e}\n{yaml}"));
            assert_eq!(json(&back), json(&v), "value {i}:\n{yaml}");
        }
    }
}
