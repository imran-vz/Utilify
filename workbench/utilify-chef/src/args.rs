//! Operation arguments: what each takes ([`Arg`]), the values a recipe step
//! holds ([`ArgValue`]), and typed access for the operation ([`Args`]).
//! The kinds follow CyberChef's so its recipes load unchanged.

use crate::OpError;
use crate::ops::data_format::numeric::js::parse_float;

#[derive(Clone, Copy, Debug)]
pub struct Arg {
    pub name: &'static str,
    pub kind: ArgKind,
}

/// An `argSelector` choice: picking it shows the `on` arguments and hides the
/// `off` ones (indices into the operation's arguments).
#[derive(Clone, Copy, Debug)]
pub struct Selector {
    pub name: &'static str,
    pub on: &'static [usize],
    pub off: &'static [usize],
}

#[derive(Clone, Copy, Debug)]
pub enum ArgKind {
    /// One line of text, used as typed.
    String {
        default: &'static str,
    },
    /// One line where `\n`, `\x41`, `é` and friends are unescaped.
    BinaryString {
        default: &'static str,
    },
    /// Multi-line text.
    Text {
        default: &'static str,
    },
    Number {
        default: f64,
        min: Option<f64>,
        max: Option<f64>,
        step: f64,
    },
    Bool {
        default: bool,
    },
    /// Pick one of a fixed list; the value is the option's name.
    Choice {
        options: &'static [&'static str],
        default: usize,
    },
    /// Presets (name, value) for a free-text value, which is unescaped
    /// like [`ArgKind::BinaryString`].
    Editable {
        options: &'static [(&'static str, &'static str)],
        default: usize,
    },
    /// Text plus how to read it (Hex, UTF8, Base64…).
    Toggle {
        default: &'static str,
        formats: &'static [&'static str],
    },
    /// A choice that shows or hides other arguments.
    Selector {
        options: &'static [Selector],
        default: usize,
    },
}

impl Arg {
    pub const fn string(name: &'static str, default: &'static str) -> Arg {
        Arg {
            name,
            kind: ArgKind::String { default },
        }
    }
    pub const fn binary(name: &'static str, default: &'static str) -> Arg {
        Arg {
            name,
            kind: ArgKind::BinaryString { default },
        }
    }
    pub const fn text(name: &'static str, default: &'static str) -> Arg {
        Arg {
            name,
            kind: ArgKind::Text { default },
        }
    }
    pub const fn number(name: &'static str, default: f64) -> Arg {
        Arg {
            name,
            kind: ArgKind::Number {
                default,
                min: None,
                max: None,
                step: 1.0,
            },
        }
    }
    pub const fn number_in(name: &'static str, default: f64, min: f64, max: f64) -> Arg {
        Arg {
            name,
            kind: ArgKind::Number {
                default,
                min: Some(min),
                max: Some(max),
                step: 1.0,
            },
        }
    }
    pub const fn flag(name: &'static str, default: bool) -> Arg {
        Arg {
            name,
            kind: ArgKind::Bool { default },
        }
    }
    pub const fn choice(name: &'static str, options: &'static [&'static str]) -> Arg {
        Arg::choice_at(name, options, 0)
    }
    pub const fn choice_at(
        name: &'static str,
        options: &'static [&'static str],
        default: usize,
    ) -> Arg {
        Arg {
            name,
            kind: ArgKind::Choice { options, default },
        }
    }
    pub const fn editable(
        name: &'static str,
        options: &'static [(&'static str, &'static str)],
    ) -> Arg {
        Arg {
            name,
            kind: ArgKind::Editable {
                options,
                default: 0,
            },
        }
    }
    pub const fn toggle(
        name: &'static str,
        default: &'static str,
        formats: &'static [&'static str],
    ) -> Arg {
        Arg {
            name,
            kind: ArgKind::Toggle { default, formats },
        }
    }
    pub const fn selector(name: &'static str, options: &'static [Selector]) -> Arg {
        Arg {
            name,
            kind: ArgKind::Selector {
                options,
                default: 0,
            },
        }
    }

    pub fn default_value(&self) -> ArgValue {
        match self.kind {
            ArgKind::String { default }
            | ArgKind::BinaryString { default }
            | ArgKind::Text { default } => ArgValue::Str(default.into()),
            ArgKind::Number { default, .. } => ArgValue::Num(default),
            ArgKind::Bool { default } => ArgValue::Bool(default),
            ArgKind::Choice { options, default } => {
                ArgValue::Str(options.get(default).copied().unwrap_or_default().into())
            }
            ArgKind::Editable { options, default } => {
                ArgValue::Str(options.get(default).map_or("", |o| o.1).into())
            }
            ArgKind::Toggle { default, formats } => ArgValue::Toggle {
                string: default.into(),
                format: formats.first().copied().unwrap_or("UTF8").into(),
            },
            ArgKind::Selector { options, default } => {
                ArgValue::Str(options.get(default).map_or("", |o| o.name).into())
            }
        }
    }

    /// Coerce a loaded value to this argument's kind, falling back to the
    /// default when it can't be read (a number given as text is read with
    /// `parseFloat`). [`crate::Recipe::parse`] rejects what CyberChef would
    /// not load before this.
    pub fn coerce(&self, v: ArgValue) -> ArgValue {
        match (self.kind, v) {
            (ArgKind::Number { .. }, ArgValue::Num(n)) => ArgValue::Num(n),
            (ArgKind::Number { .. }, ArgValue::Str(s)) => match parse_float(&s) {
                n if n.is_nan() => self.default_value(),
                n => ArgValue::Num(n),
            },
            (ArgKind::Bool { .. }, ArgValue::Bool(b)) => ArgValue::Bool(b),
            (ArgKind::Toggle { .. }, t @ ArgValue::Toggle { .. }) => t,
            (ArgKind::Toggle { formats, .. }, ArgValue::Str(s)) => ArgValue::Toggle {
                string: s,
                format: formats.first().copied().unwrap_or("UTF8").into(),
            },
            (
                ArgKind::String { .. }
                | ArgKind::BinaryString { .. }
                | ArgKind::Text { .. }
                | ArgKind::Choice { .. }
                | ArgKind::Editable { .. }
                | ArgKind::Selector { .. },
                ArgValue::Str(s),
            ) => ArgValue::Str(s),
            (
                ArgKind::String { .. } | ArgKind::BinaryString { .. } | ArgKind::Text { .. },
                ArgValue::Num(n),
            ) => ArgValue::Str(crate::format_number(n)),
            _ => self.default_value(),
        }
    }

    /// Which other arguments this one's current value hides.
    pub fn hidden_by(&self, v: &ArgValue) -> &'static [usize] {
        match (self.kind, v) {
            (ArgKind::Selector { options, .. }, ArgValue::Str(s)) => {
                options.iter().find(|o| o.name == s).map_or(&[], |o| o.off)
            }
            _ => &[],
        }
    }
}

/// A step's value for one argument, as stored in recipes.
#[derive(Clone, Debug, PartialEq)]
pub enum ArgValue {
    Str(String),
    Num(f64),
    Bool(bool),
    Toggle { string: String, format: String },
}

/// The prepared arguments an operation runs with: escapes already parsed
/// where the kind calls for it, like CyberChef's `Ingredient.prepare`.
pub struct Args {
    values: Vec<ArgValue>,
}

impl Args {
    pub fn prepare(specs: &[Arg], values: &[ArgValue]) -> Args {
        let values = specs
            .iter()
            .enumerate()
            .map(|(i, spec)| {
                let v = values
                    .get(i)
                    .cloned()
                    .map_or_else(|| spec.default_value(), |v| spec.coerce(v));
                match (spec.kind, v) {
                    (ArgKind::BinaryString { .. } | ArgKind::Editable { .. }, ArgValue::Str(s)) => {
                        ArgValue::Str(parse_escaped(&s))
                    }
                    (_, v) => v,
                }
            })
            .collect();
        Args { values }
    }

    /// For operations called with literal values, mostly in tests.
    pub fn from_values(values: Vec<ArgValue>) -> Args {
        Args { values }
    }

    pub fn str(&self, i: usize) -> &str {
        match self.values.get(i) {
            Some(ArgValue::Str(s)) | Some(ArgValue::Toggle { string: s, .. }) => s,
            _ => "",
        }
    }

    pub fn num(&self, i: usize) -> f64 {
        match self.values.get(i) {
            Some(ArgValue::Num(n)) => *n,
            Some(ArgValue::Str(s)) => parse_float(s),
            Some(ArgValue::Bool(b)) => f64::from(u8::from(*b)),
            _ => f64::NAN,
        }
    }

    /// A number argument that must be a whole number in `min..=max`.
    pub fn int(&self, i: usize, name: &str, min: i64, max: i64) -> Result<i64, OpError> {
        let n = self.num(i);
        if !n.is_finite() || n.fract() != 0.0 || (n as i64) < min || (n as i64) > max {
            return Err(OpError::new(format!(
                "{name} must be a whole number from {min} to {max}"
            )));
        }
        Ok(n as i64)
    }

    pub fn flag(&self, i: usize) -> bool {
        matches!(self.values.get(i), Some(ArgValue::Bool(true)))
    }

    /// Text and its format of a [`ArgKind::Toggle`] argument.
    pub fn toggle(&self, i: usize) -> (&str, &str) {
        match self.values.get(i) {
            Some(ArgValue::Toggle { string, format }) => (string, format),
            Some(ArgValue::Str(s)) => (s, "UTF8"),
            _ => ("", "UTF8"),
        }
    }
}

/// Unescape `\n`, `\t`, `\\`, `\x41`, `\101`, `é`, `\u{1F600}` and the
/// other C-style escapes. CyberChef's `Utils.parseEscapedChars`.
pub fn parse_escaped(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let hex_run = |from: usize, n: usize| -> Option<u32> {
        let run: String = chars.get(from..from + n)?.iter().collect();
        run.chars()
            .all(|c| c.is_ascii_hexdigit())
            .then(|| u32::from_str_radix(&run, 16).ok())
            .flatten()
    };
    while i < chars.len() {
        let c = chars[i];
        if c != '\\' || i + 1 >= chars.len() {
            out.push(c);
            i += 1;
            continue;
        }
        let n = chars[i + 1];
        let simple = match n {
            'a' => Some('\x07'),
            'b' => Some('\x08'),
            'f' => Some('\x0c'),
            'n' => Some('\n'),
            'r' => Some('\r'),
            't' => Some('\t'),
            'v' => Some('\x0b'),
            '\'' => Some('\''),
            '"' => Some('"'),
            '\\' => Some('\\'),
            _ => None,
        };
        if let Some(r) = simple {
            out.push(r);
            i += 2;
            continue;
        }
        if n.is_digit(8) {
            // Three digits when the first is 0-3, else up to two.
            let max = if n <= '3' { 3 } else { 2 };
            let mut len = 1;
            while len < max && chars.get(i + 1 + len).is_some_and(|c| c.is_digit(8)) {
                len += 1;
            }
            let digits: String = chars[i + 1..i + 1 + len].iter().collect();
            if let Some(ch) = u32::from_str_radix(&digits, 8)
                .ok()
                .and_then(char::from_u32)
            {
                out.push(ch);
            }
            i += 1 + len;
            continue;
        }
        if n == 'x'
            && let Some(v) = hex_run(i + 2, 2)
        {
            out.push(char::from_u32(v).unwrap_or('\u{fffd}'));
            i += 4;
            continue;
        }
        if n == 'u' {
            if chars.get(i + 2) == Some(&'{') {
                if let Some(close) = chars[i + 3..].iter().position(|&c| c == '}')
                    && (1..=6).contains(&close)
                    && let Some(ch) = hex_run(i + 3, close).and_then(char::from_u32)
                {
                    out.push(ch);
                    i += 4 + close;
                    continue;
                }
            } else if let Some(v) = hex_run(i + 2, 4) {
                out.push(char::from_u32(v).unwrap_or('\u{fffd}'));
                i += 6;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescapes_like_cyberchef() {
        assert_eq!(parse_escaped(r"a\nb\tc\\d"), "a\nb\tc\\d");
        assert_eq!(parse_escaped(r"\x41\101é\u{1F600}"), "AAé😀");
        assert_eq!(parse_escaped(r"\0"), "\0");
        assert_eq!(parse_escaped(r"\q\x4"), r"\q\x4");
        assert_eq!(parse_escaped("end\\"), "end\\");
    }

    #[test]
    fn prepares_defaults_and_coerces() {
        const SPECS: &[Arg] = &[
            Arg::number("Width", 16.0),
            Arg::binary("Delim", r"\n"),
            Arg::choice("Case", &["Upper", "Lower"]),
            Arg::flag("On", true),
        ];
        let args = Args::prepare(SPECS, &[ArgValue::Str("8".into())]);
        assert_eq!(args.num(0), 8.0);
        // parseFloat, as CyberChef reads a number given as text.
        let args = Args::prepare(SPECS, &[ArgValue::Str("\u{feff} 1.5e1px".into())]);
        assert_eq!(args.num(0), 15.0);
        let args = Args::prepare(SPECS, &[ArgValue::Str("px".into())]);
        assert_eq!(args.num(0), 16.0);
        assert_eq!(args.str(1), "\n");
        assert_eq!(args.str(2), "Upper");
        assert!(args.flag(3));
        assert!(args.int(0, "Width", 1, 4).is_err());
    }
}
