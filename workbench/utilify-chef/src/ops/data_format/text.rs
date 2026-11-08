//! Text encodings and escapes: HTML entities, URL percent-encoding, Unicode
//! escapes and normalisation, smart characters, quoted-printable, Punycode,
//! code pages ("Encode text" and friends), Braille, caret notation and MIME
//! encoded words.
//!
//! CyberChef runs these on JavaScript strings, so where results depend on
//! UTF-16 code units (escapes, surrogate pairs built from two entities) the
//! work is done on `u16` units and lone surrogates become U+FFFD at the end.

use super::CATEGORY;
use crate::args::ArgKind;
use crate::{Arg, Args, Dish, OpError, Operation};
use unicode_normalization::UnicodeNormalization;

mod codepage;
mod mime;
mod punycode;
mod quoted_printable;

pub(super) fn ops() -> Vec<&'static Operation> {
    vec![
        &TO_HTML_ENTITY,
        &FROM_HTML_ENTITY,
        &URL_ENCODE,
        &URL_DECODE,
        &ESCAPE_UNICODE,
        &UNESCAPE_UNICODE,
        &NORMALISE_UNICODE,
        &ESCAPE_SMART,
        &TO_QUOTED_PRINTABLE,
        &FROM_QUOTED_PRINTABLE,
        &TO_PUNYCODE,
        &FROM_PUNYCODE,
        &ENCODE_TEXT,
        &DECODE_TEXT,
        &TEXT_ENCODING_BRUTE_FORCE,
        &TO_BRAILLE,
        &FROM_BRAILLE,
        &CARET_M_DECODE,
        &MIME_DECODING,
    ]
}

/// CyberChef's `Ingredient.validate` for an option argument, which runs
/// before the operation: it can't be empty and must be one of the options,
/// ignoring case. The value is returned as given.
fn option<'a>(op: &Operation, args: &'a Args, i: usize) -> Result<&'a str, OpError> {
    let value = args.str(i);
    let Some(spec) = op.args.get(i) else {
        return Ok(value);
    };
    let ArgKind::Choice { options, .. } = spec.kind else {
        return Ok(value);
    };
    if value.is_empty() && !options.contains(&"") {
        return Err(OpError::new(format!("{} cannot be empty.", spec.name)));
    }
    let lower = value.to_lowercase();
    if !options.iter().any(|o| o.to_lowercase() == lower) {
        return Err(OpError::new(format!(
            "{} must be one of the following: {}.",
            spec.name,
            options.join(", ")
        )));
    }
    Ok(value)
}

/// An operation's byteArray result, checked like CyberChef's `Dish.set`:
/// any value past 0xFF is an error quoting the start of the array.
fn byte_array(values: &[u32]) -> Result<Dish, OpError> {
    if values.iter().all(|&v| v <= 0xFF) {
        return Ok(Dish::Bytes(values.iter().map(|&v| v as u8).collect()));
    }
    let json = format!(
        "[{}]",
        values
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",")
    );
    // Utils.truncate(json, 25)
    let sample = if json.len() > 25 {
        format!("{}...", &json[..22])
    } else {
        json
    };
    Err(OpError::new(format!(
        "Data is not a valid byteArray: {sample}"
    )))
}

/// A JavaScript string's units as text, lone surrogates as U+FFFD.
fn units_to_string(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}

/// CyberChef's `Utils.chr`: a code as UTF-16, with anything past U+FFFF
/// split into a surrogate pair (wrapping past U+10FFFF, as its bit masks do).
fn push_chr(out: &mut Vec<u16>, code: u64) {
    if code > 0xFFFF {
        let o = code - 0x10000;
        out.push((((o >> 10) & 0x3FF) | 0xD800) as u16);
        out.push((0xDC00 | (o & 0x3FF)) as u16);
    } else {
        out.push(code as u16);
    }
}

// ---------------------------------------------------------------------------
// HTML entities

const HTML_ENTITY_FORMATS: &[&str] = &["Named entities", "Numeric entities", "Hex entities"];

pub static TO_HTML_ENTITY: Operation = Operation {
    name: "To HTML Entity",
    category: CATEGORY,
    description: "Converts characters to HTML entities\n\ne.g. & becomes &amp;",
    info_url: Some("https://wikipedia.org/wiki/List_of_XML_and_HTML_character_entity_references"),
    args: &[
        Arg::flag("Convert all characters", false),
        Arg::choice("Convert to", HTML_ENTITY_FORMATS),
    ],
    run: to_html_entity,
};

/// A character's name in the WHATWG named character reference table (HTML
/// §13.5), as `utilify-html-entities` picks it, without the `&` and `;`.
fn entity_name(c: char) -> Option<&'static str> {
    utilify_html_entities::name_of(c).map(|n| n.trim_end_matches(';'))
}

fn to_html_entity(input: Dish, args: &Args) -> Result<Dish, OpError> {
    let convert_all = args.flag(0);
    let to = option(&TO_HTML_ENTITY, args, 1)?;
    let (numeric, hex) = (to == "Numeric entities", to == "Hex entities");
    let mut out = String::new();
    for c in input.text().chars() {
        let code = u32::from(c);
        let named = entity_name(c);
        let wide = code > 255 || named.is_some();
        let decimal = || format!("&#{code};");
        let hexadecimal = || format!("&#x{code:02x};");
        let entity = if convert_all && numeric {
            decimal()
        } else if convert_all && hex {
            hexadecimal()
        } else if convert_all {
            named.map_or_else(decimal, |n| format!("&{n};"))
        } else if numeric && wide {
            decimal()
        } else if hex && wide {
            hexadecimal()
        } else if numeric || hex {
            c.to_string()
        } else if let Some(n) = named {
            format!("&{n};")
        } else if code > 255 {
            decimal()
        } else {
            c.to_string()
        };
        out.push_str(&entity);
    }
    Ok(Dish::Text(out))
}

pub static FROM_HTML_ENTITY: Operation = Operation {
    name: "From HTML Entity",
    category: CATEGORY,
    description: "Converts HTML entities back to characters\n\ne.g. &amp; becomes &",
    info_url: Some("https://wikipedia.org/wiki/List_of_XML_and_HTML_character_entity_references"),
    args: &[],
    run: |input, _| Ok(Dish::Text(from_html_entity(&input.text()))),
};

/// Character references decoded as the HTML tokenizer does in text content
/// (WHATWG HTML §13.2.5.77–84, via `utilify-html-entities`), with its
/// error recovery: what a browser displays. Legacy names work without `;`,
/// numeric references to U+0000, surrogates or past U+10FFFF become U+FFFD,
/// C1 controls map through Windows-1252, and anything that isn't a
/// reference stays as written.
fn from_html_entity(input: &str) -> String {
    let opts = utilify_html_entities::DecodeOptions::RECOVER;
    match utilify_html_entities::decode_with(input, &opts) {
        Ok(text) => text.into_owned(),
        // Recovery mode reports no parse errors.
        Err(_) => input.to_owned(),
    }
}

// ---------------------------------------------------------------------------
// URL percent-encoding

pub static URL_ENCODE: Operation = Operation {
    name: "URL Encode",
    category: CATEGORY,
    description: "Encodes problematic characters into percent-encoding, a format supported by URIs/URLs.\n\ne.g. = becomes %3d",
    info_url: Some("https://wikipedia.org/wiki/Percent-encoding"),
    args: &[Arg::flag("Encode all special chars", false)],
    run: |input, args| {
        let set = if args.flag(0) {
            percent_encoding::NON_ALPHANUMERIC
        } else {
            URL_RESERVED
        };
        let out = percent_encoding::percent_encode(&input.bytes(), set).to_string();
        Ok(Dish::Text(out))
    },
};

/// Bytes "URL Encode" escapes unless "Encode all special chars" is set:
/// everything but ASCII letters, digits and the URI reserved characters
/// (plus `%`).
const URL_RESERVED: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b':')
    .remove(b'/')
    .remove(b'?')
    .remove(b'#')
    .remove(b'[')
    .remove(b']')
    .remove(b'@')
    .remove(b'!')
    .remove(b'$')
    .remove(b'&')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')')
    .remove(b'*')
    .remove(b'+')
    .remove(b',')
    .remove(b';')
    .remove(b'=')
    .remove(b'%');

pub static URL_DECODE: Operation = Operation {
    name: "URL Decode",
    category: CATEGORY,
    description: "Converts URI/URL percent-encoded characters back to their raw values.\n\ne.g. %3d becomes =",
    info_url: Some("https://wikipedia.org/wiki/Percent-encoding"),
    args: &[Arg::flag("Treat \"+\" as space", true)],
    run: |input, args| {
        let text = input.text();
        let text = if args.flag(0) {
            text.replace('+', " ").into()
        } else {
            text
        };
        let out = decode_uri_component(&text).unwrap_or_else(|| {
            let units: Vec<u16> = text.encode_utf16().collect();
            units_to_string(&unescape(&units))
        });
        Ok(Dish::Text(out))
    },
};

fn hex_unit(u: Option<&u16>) -> Option<u32> {
    u.and_then(|&u| char::from_u32(u32::from(u)))
        .and_then(|c| c.to_digit(16))
}

/// The byte of the `%XX` at `k`, if it is one.
fn percent_byte(s: &[u16], k: usize) -> Option<u8> {
    if s.get(k) != Some(&u16::from(b'%')) {
        return None;
    }
    Some((hex_unit(s.get(k + 1))? * 16 + hex_unit(s.get(k + 2))?) as u8)
}

/// JavaScript's `decodeURIComponent`; `None` where it throws a URIError.
/// It throws on a `%` that doesn't start an escape, which `percent-encoding`
/// passes through, so that is checked first; the decoded bytes must then be
/// UTF-8 (escapes only ever add whole sequences or bytes that can't join a
/// literal character's, so this is the same as checking each run of escapes).
fn decode_uri_component(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let hex = |k: usize| b.get(k).is_some_and(u8::is_ascii_hexdigit);
    let well_formed = b
        .iter()
        .enumerate()
        .all(|(k, &c)| c != b'%' || (hex(k + 1) && hex(k + 2)));
    if !well_formed {
        return None;
    }
    percent_encoding::percent_decode_str(s)
        .decode_utf8()
        .ok()
        .map(Into::into)
}

/// JavaScript's `unescape`: `%XX` and `%uXXXX`, anything else as it is.
fn unescape(s: &[u16]) -> Vec<u16> {
    let mut out = Vec::with_capacity(s.len());
    let mut k = 0;
    while k < s.len() {
        if s[k] == u16::from(b'%') {
            if s.get(k + 1) == Some(&u16::from(b'u')) {
                let digits: Option<Vec<u32>> = (2..6).map(|d| hex_unit(s.get(k + d))).collect();
                if let Some(d) = digits {
                    out.push(d.iter().fold(0, |acc, &v| acc * 16 + v) as u16);
                    k += 6;
                    continue;
                }
            }
            if let Some(b) = percent_byte(s, k) {
                out.push(u16::from(b));
                k += 3;
                continue;
            }
        }
        out.push(s[k]);
        k += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// Unicode escapes and normalisation

const UNICODE_PREFIXES: &[&str] = &["\\u", "%u", "U+"];

pub static ESCAPE_UNICODE: Operation = Operation {
    name: "Escape Unicode Characters",
    category: CATEGORY,
    description: "Converts characters to their unicode-escaped notations.\n\nSupports the prefixes:\n\\u\n%u\nU+\ne.g. σου becomes \\u03C3\\u03BF\\u03C5",
    info_url: None,
    args: &[
        Arg::choice("Prefix", UNICODE_PREFIXES),
        Arg::flag("Encode all chars", false),
        Arg::number("Padding", 4.0),
        Arg::flag("Uppercase hex", true),
    ],
    run: escape_unicode,
};

fn escape_unicode(input: Dish, args: &Args) -> Result<Dish, OpError> {
    let prefix = option(&ESCAPE_UNICODE, args, 0)?;
    let encode_all = args.flag(1);
    let padding = args.num(2);
    let uppercase = args.flag(3);
    if padding.is_nan() {
        return Err(OpError::new("Padding must be a number."));
    }
    // padStart's length: whole, at least 0; V8 caps strings near 2^29.
    let width = padding.max(0.0).floor();
    if width >= f64::from(1u32 << 29) {
        return Err(OpError::new("Invalid string length"));
    }
    let width = width as usize;
    let text = input.text();
    let mut out = String::with_capacity(text.len());
    for unit in text.encode_utf16() {
        if !encode_all && (0x20..=0x7E).contains(&unit) {
            out.push(char::from(unit as u8));
            continue;
        }
        let hex = if uppercase {
            format!("{unit:X}")
        } else {
            format!("{unit:x}")
        };
        out.push_str(prefix);
        out.push_str(&format!("{hex:0>width$}"));
    }
    Ok(Dish::Text(out))
}

pub static UNESCAPE_UNICODE: Operation = Operation {
    name: "Unescape Unicode Characters",
    category: CATEGORY,
    description: "Converts unicode-escaped character notation back into raw characters.\n\nSupports the prefixes:\n\\u\n%u\nU+\ne.g. \\u03c3\\u03bf\\u03c5 becomes σου",
    info_url: None,
    args: &[Arg::choice("Prefix", UNICODE_PREFIXES)],
    run: unescape_unicode,
};

fn unescape_unicode(input: Dish, args: &Args) -> Result<Dish, OpError> {
    let arg = option(&UNESCAPE_UNICODE, args, 0)?;
    // CyberChef looks the prefix up exactly; a differently cased one passes
    // validation and builds its regex from "undefined".
    let prefix = if UNICODE_PREFIXES.contains(&arg) {
        arg
    } else {
        "undefined"
    };
    let max_digits = if arg == "U+" { 6 } else { 4 };
    let prefix: Vec<u16> = prefix.encode_utf16().collect();
    let s: Vec<u16> = input.text().encode_utf16().collect();
    let same = |a: u16, b: u16| {
        a == b || (a < 128 && b < 128 && (a as u8).eq_ignore_ascii_case(&(b as u8)))
    };
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let has_prefix = s.len() - i >= prefix.len()
            && s[i..i + prefix.len()]
                .iter()
                .zip(&prefix)
                .all(|(&a, &b)| same(a, b));
        if has_prefix {
            let start = i + prefix.len();
            let digits = s[start..]
                .iter()
                .take(max_digits)
                .take_while(|&&u| hex_unit(Some(&u)).is_some())
                .count();
            if digits >= 4 {
                let code = s[start..start + digits].iter().fold(0u64, |acc, &u| {
                    acc * 16 + u64::from(hex_unit(Some(&u)).unwrap_or(0))
                });
                push_chr(&mut out, code);
                i = start + digits;
                continue;
            }
        }
        out.push(s[i]);
        i += 1;
    }
    Ok(Dish::Text(units_to_string(&out)))
}

pub static NORMALISE_UNICODE: Operation = Operation {
    name: "Normalise Unicode",
    category: CATEGORY,
    description: "Transform Unicode characters to one of the Normalisation Forms",
    info_url: Some("https://wikipedia.org/wiki/Unicode_equivalence#Normal_forms"),
    args: &[Arg::choice("Normal Form", &["NFD", "NFC", "NFKD", "NFKC"])],
    run: |input, args| {
        let text = input.text();
        let out = match option(&NORMALISE_UNICODE, args, 0)? {
            "NFD" => text.nfd().collect(),
            "NFC" => text.nfc().collect(),
            "NFKD" => text.nfkd().collect(),
            "NFKC" => text.nfkc().collect(),
            _ => return Err(OpError::new("Unknown Normalisation Form")),
        };
        Ok(Dish::Text(out))
    },
};

// ---------------------------------------------------------------------------
// Smart characters

pub static ESCAPE_SMART: Operation = Operation {
    name: "Escape Smart Characters",
    category: CATEGORY,
    description: "Converts smart (typographic) Unicode characters — e.g. smart quotes, em/en dashes, ellipses, ©, ®, ™, arrows — into their plain ASCII equivalents.\n\nCharacters with no ASCII mapping (e.g. ☣) are handled according to the 'Unmappable characters' option.\n\ne.g. “Hello” — world… becomes \"Hello\" -- world...",
    info_url: None,
    args: &[Arg::choice(
        "Unmappable characters",
        &["Include", "Remove", "Replace with '.'"],
    )],
    run: escape_smart,
};

fn smart_replacement(c: char) -> Option<&'static str> {
    Some(match c {
        '“' | '”' | '„' | '‟' | '″' => "\"",
        '‘' | '’' | '‚' | '‛' | '′' => "'",
        '‐' | '‑' | '‒' | '–' => "-",
        '—' | '―' => "--",
        '…' => "...",
        '©' => "(c)",
        '®' => "(r)",
        '™' => "(tm)",
        '←' => "<--",
        '→' => "-->",
        '↑' => "^",
        '↓' => "v",
        '↔' => "<->",
        '⇐' => "<==",
        '⇒' => "==>",
        '⇔' => "<=>",
        '«' => "<<",
        '»' => ">>",
        '‹' => "<",
        '›' => ">",
        '×' => "x",
        '÷' => "/",
        '±' => "+/-",
        '•' => "*",
        '·' => ".",
        '\u{A0}' | '\u{2002}' | '\u{2003}' | '\u{2009}' | '\u{200A}' => " ",
        _ => return None,
    })
}

fn escape_smart(input: Dish, args: &Args) -> Result<Dish, OpError> {
    let unmappable = option(&ESCAPE_SMART, args, 0)?;
    let text = input.text();
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii() {
            out.push(c);
        } else if let Some(r) = smart_replacement(c) {
            out.push_str(r);
        } else {
            match unmappable {
                "Remove" => {}
                "Replace with '.'" => out.push('.'),
                _ => out.push(c),
            }
        }
    }
    Ok(Dish::Text(out))
}

// ---------------------------------------------------------------------------
// Quoted-printable

pub static TO_QUOTED_PRINTABLE: Operation = Operation {
    name: "To Quoted Printable",
    category: CATEGORY,
    description: "Quoted-Printable, or QP encoding, is an encoding using printable ASCII characters (alphanumeric and the equals sign '=') to transmit 8-bit data over a 7-bit data path or, generally, over a medium which is not 8-bit clean. It is defined as a MIME content transfer encoding for use in email.\n\nQP works by using the equals sign '=' as an escape character. It also limits line length to 76, as some software has limits on line length.",
    info_url: Some("https://wikipedia.org/wiki/Quoted-printable"),
    args: &[],
    run: |input, _| Ok(Dish::Text(quoted_printable::encode(&input.bytes()))),
};

pub static FROM_QUOTED_PRINTABLE: Operation = Operation {
    name: "From Quoted Printable",
    category: CATEGORY,
    description: "Converts QP-encoded text back to standard text. This format is a content transfer encoding common in email messages.\n\ne.g. The quoted-printable encoded string hello=20world becomes hello world",
    info_url: Some("https://wikipedia.org/wiki/Quoted-printable"),
    args: &[],
    run: |input, _| {
        quoted_printable::decode(&input.text())
            .map(Dish::Bytes)
            .map_err(OpError::new)
    },
};

// ---------------------------------------------------------------------------
// Punycode

pub static TO_PUNYCODE: Operation = Operation {
    name: "To Punycode",
    category: CATEGORY,
    description: "Punycode is a way to represent Unicode with the limited character subset of ASCII supported by the Domain Name System.\n\ne.g. münchen encodes to mnchen-3ya",
    info_url: Some("https://wikipedia.org/wiki/Punycode"),
    args: &[Arg::flag("Internationalised domain name", false)],
    run: |input, args| {
        let text = input.text();
        let out = if args.flag(0) {
            punycode::to_ascii(&text)
        } else {
            punycode::encode(&text)
        };
        out.map(Dish::Text).map_err(OpError::new)
    },
};

pub static FROM_PUNYCODE: Operation = Operation {
    name: "From Punycode",
    category: CATEGORY,
    description: "Punycode is a way to represent Unicode with the limited character subset of ASCII supported by the Domain Name System.\n\ne.g. mnchen-3ya decodes to münchen",
    info_url: Some("https://wikipedia.org/wiki/Punycode"),
    args: &[Arg::flag("Internationalised domain name", false)],
    run: |input, args| {
        let text = input.text();
        let out = if args.flag(0) {
            punycode::to_unicode(&text)
        } else {
            punycode::decode(&text)
        };
        out.map(Dish::Text).map_err(OpError::new)
    },
};

// ---------------------------------------------------------------------------
// Code pages

/// CyberChef's `CHR_ENC_CODE_PAGES` (option name to code page), plus the
/// option list and the descriptions that list it.
macro_rules! code_pages {
    ($($name:literal = $cp:literal,)*) => {
        const CODE_PAGES: &[(&str, u32)] = &[$(($name, $cp)),*];
        const CHARSETS: &[&str] = &[$($name),*];
        const ENCODE_TEXT_DESCRIPTION: &str = concat!(
            "Encodes text into the chosen character encoding.\n\nSupported charsets are:",
            $("\n", $name),*
        );
        const DECODE_TEXT_DESCRIPTION: &str = concat!(
            "Decodes text from the chosen character encoding.\n\nSupported charsets are:",
            $("\n", $name),*
        );
        const BRUTE_FORCE_DESCRIPTION: &str = concat!(
            "Enumerates all supported text encodings for the input, allowing you to quickly spot the correct one.\n\nSupported charsets are:",
            $("\n", $name),*
        );
    };
}

code_pages! {
    "UTF-8 (65001)" = 65001,
    "UTF-7 (65000)" = 65000,
    "UTF-16LE (1200)" = 1200,
    "UTF-16BE (1201)" = 1201,
    "UTF-32LE (12000)" = 12000,
    "UTF-32BE (12001)" = 12001,
    "IBM EBCDIC International (500)" = 500,
    "IBM EBCDIC US-Canada (37)" = 37,
    "IBM EBCDIC Multilingual/ROECE (Latin 2) (870)" = 870,
    "IBM EBCDIC Greek Modern (875)" = 875,
    "IBM EBCDIC French (1010)" = 1010,
    "IBM EBCDIC Turkish (Latin 5) (1026)" = 1026,
    "IBM EBCDIC Latin 1/Open System (1047)" = 1047,
    "IBM EBCDIC Lao (1132/1133/1341)" = 1132,
    "IBM EBCDIC US-Canada (037 + Euro symbol) (1140)" = 1140,
    "IBM EBCDIC Germany (20273 + Euro symbol) (1141)" = 1141,
    "IBM EBCDIC Denmark-Norway (20277 + Euro symbol) (1142)" = 1142,
    "IBM EBCDIC Finland-Sweden (20278 + Euro symbol) (1143)" = 1143,
    "IBM EBCDIC Italy (20280 + Euro symbol) (1144)" = 1144,
    "IBM EBCDIC Latin America-Spain (20284 + Euro symbol) (1145)" = 1145,
    "IBM EBCDIC United Kingdom (20285 + Euro symbol) (1146)" = 1146,
    "IBM EBCDIC France (20297 + Euro symbol) (1147)" = 1147,
    "IBM EBCDIC International (500 + Euro symbol) (1148)" = 1148,
    "IBM EBCDIC Icelandic (20871 + Euro symbol) (1149)" = 1149,
    "IBM EBCDIC Germany (20273)" = 20273,
    "IBM EBCDIC Denmark-Norway (20277)" = 20277,
    "IBM EBCDIC Finland-Sweden (20278)" = 20278,
    "IBM EBCDIC Italy (20280)" = 20280,
    "IBM EBCDIC Latin America-Spain (20284)" = 20284,
    "IBM EBCDIC United Kingdom (20285)" = 20285,
    "IBM EBCDIC Japanese Katakana Extended (20290)" = 20290,
    "IBM EBCDIC France (20297)" = 20297,
    "IBM EBCDIC Arabic (20420)" = 20420,
    "IBM EBCDIC Greek (20423)" = 20423,
    "IBM EBCDIC Hebrew (20424)" = 20424,
    "IBM EBCDIC Korean Extended (20833)" = 20833,
    "IBM EBCDIC Thai (20838)" = 20838,
    "IBM EBCDIC Icelandic (20871)" = 20871,
    "IBM EBCDIC Cyrillic Russian (20880)" = 20880,
    "IBM EBCDIC Turkish (20905)" = 20905,
    "IBM EBCDIC Latin 1/Open System (1047 + Euro symbol) (20924)" = 20924,
    "IBM EBCDIC Cyrillic Serbian-Bulgarian (21025)" = 21025,
    "OEM United States (437)" = 437,
    "OEM Greek (formerly 437G); Greek (DOS) (737)" = 737,
    "OEM Baltic; Baltic (DOS) (775)" = 775,
    "OEM Russian; Cyrillic + Euro symbol (808)" = 808,
    "OEM Multilingual Latin 1; Western European (DOS) (850)" = 850,
    "OEM Latin 2; Central European (DOS) (852)" = 852,
    "OEM Cyrillic (primarily Russian) (855)" = 855,
    "OEM Turkish; Turkish (DOS) (857)" = 857,
    "OEM Multilingual Latin 1 + Euro symbol (858)" = 858,
    "OEM Portuguese; Portuguese (DOS) (860)" = 860,
    "OEM Icelandic; Icelandic (DOS) (861)" = 861,
    "OEM Hebrew; Hebrew (DOS) (862)" = 862,
    "OEM French Canadian; French Canadian (DOS) (863)" = 863,
    "OEM Arabic; Arabic (864) (864)" = 864,
    "OEM Nordic; Nordic (DOS) (865)" = 865,
    "OEM Russian; Cyrillic (DOS) (866)" = 866,
    "OEM Modern Greek; Greek, Modern (DOS) (869)" = 869,
    "OEM Cyrillic (primarily Russian) + Euro Symbol (872)" = 872,
    "Windows-874 Thai (874)" = 874,
    "Windows-1250 Central European (1250)" = 1250,
    "Windows-1251 Cyrillic (1251)" = 1251,
    "Windows-1252 Latin (1252)" = 1252,
    "Windows-1253 Greek (1253)" = 1253,
    "Windows-1254 Turkish (1254)" = 1254,
    "Windows-1255 Hebrew (1255)" = 1255,
    "Windows-1256 Arabic (1256)" = 1256,
    "Windows-1257 Baltic (1257)" = 1257,
    "Windows-1258 Vietnam (1258)" = 1258,
    "ISO-8859-1 Latin 1 Western European (28591)" = 28591,
    "ISO-8859-2 Latin 2 Central European (28592)" = 28592,
    "ISO-8859-3 Latin 3 South European (28593)" = 28593,
    "ISO-8859-4 Latin 4 North European (28594)" = 28594,
    "ISO-8859-5 Latin/Cyrillic (28595)" = 28595,
    "ISO-8859-6 Latin/Arabic (28596)" = 28596,
    "ISO-8859-7 Latin/Greek (28597)" = 28597,
    "ISO-8859-8 Latin/Hebrew (28598)" = 28598,
    "ISO 8859-8 Hebrew (ISO-Logical) (38598)" = 38598,
    "ISO-8859-9 Latin 5 Turkish (28599)" = 28599,
    "ISO-8859-10 Latin 6 Nordic (28600)" = 28600,
    "ISO-8859-11 Latin/Thai (28601)" = 28601,
    "ISO-8859-13 Latin 7 Baltic Rim (28603)" = 28603,
    "ISO-8859-14 Latin 8 Celtic (28604)" = 28604,
    "ISO-8859-15 Latin 9 (28605)" = 28605,
    "ISO-8859-16 Latin 10 (28606)" = 28606,
    "ISO 2022 JIS Japanese with no halfwidth Katakana (50220)" = 50220,
    "ISO 2022 JIS Japanese with halfwidth Katakana (50221)" = 50221,
    "ISO 2022 Japanese JIS X 0201-1989 (1 byte Kana-SO/SI) (50222)" = 50222,
    "ISO 2022 Korean (50225)" = 50225,
    "ISO 2022 Simplified Chinese (50227)" = 50227,
    "ISO 6937 Non-Spacing Accent (20269)" = 20269,
    "EUC Japanese (51932)" = 51932,
    "EUC Simplified Chinese (51936)" = 51936,
    "EUC Korean (51949)" = 51949,
    "ISCII Devanagari (57002)" = 57002,
    "ISCII Bengali (57003)" = 57003,
    "ISCII Tamil (57004)" = 57004,
    "ISCII Telugu (57005)" = 57005,
    "ISCII Assamese (57006)" = 57006,
    "ISCII Oriya (57007)" = 57007,
    "ISCII Kannada (57008)" = 57008,
    "ISCII Malayalam (57009)" = 57009,
    "ISCII Gujarati (57010)" = 57010,
    "ISCII Punjabi (57011)" = 57011,
    "Japanese Shift-JIS (932)" = 932,
    "Simplified Chinese GBK (936)" = 936,
    "Korean (949)" = 949,
    "Traditional Chinese Big5 (950)" = 950,
    "US-ASCII (7-bit) (20127)" = 20127,
    "Simplified Chinese GB2312 (20936)" = 20936,
    "KOI8-R Russian Cyrillic (20866)" = 20866,
    "KOI8-U Ukrainian Cyrillic (21866)" = 21866,
    "Mazovia (Polish) MS-DOS (620)" = 620,
    "Arabic (ASMO 708) (708)" = 708,
    "Arabic (Transparent ASMO); Arabic (DOS) (720)" = 720,
    "Kamenický (Czech) MS-DOS (895)" = 895,
    "Korean (Johab) (1361)" = 1361,
    "MAC Roman (10000)" = 10000,
    "Japanese (Mac) (10001)" = 10001,
    "MAC Traditional Chinese (Big5) (10002)" = 10002,
    "Korean (Mac) (10003)" = 10003,
    "Arabic (Mac) (10004)" = 10004,
    "Hebrew (Mac) (10005)" = 10005,
    "Greek (Mac) (10006)" = 10006,
    "Cyrillic (Mac) (10007)" = 10007,
    "MAC Simplified Chinese (GB 2312) (10008)" = 10008,
    "Romanian (Mac) (10010)" = 10010,
    "Ukrainian (Mac) (10017)" = 10017,
    "Thai (Mac) (10021)" = 10021,
    "MAC Latin 2 (Central European) (10029)" = 10029,
    "Icelandic (Mac) (10079)" = 10079,
    "Turkish (Mac) (10081)" = 10081,
    "Croatian (Mac) (10082)" = 10082,
    "CNS Taiwan (Chinese Traditional) (20000)" = 20000,
    "TCA Taiwan (20001)" = 20001,
    "ETEN Taiwan (Chinese Traditional) (20002)" = 20002,
    "IBM5550 Taiwan (20003)" = 20003,
    "TeleText Taiwan (20004)" = 20004,
    "Wang Taiwan (20005)" = 20005,
    "Western European IA5 (IRV International Alphabet 5) (20105)" = 20105,
    "IA5 German (7-bit) (20106)" = 20106,
    "IA5 Swedish (7-bit) (20107)" = 20107,
    "IA5 Norwegian (7-bit) (20108)" = 20108,
    "T.61 (20261)" = 20261,
    "Japanese (JIS 0208-1990 and 0212-1990) (20932)" = 20932,
    "Korean Wansung (20949)" = 20949,
    "Extended/Ext Alpha Lowercase (21027)" = 21027,
    "Europa 3 (29001)" = 29001,
    "Atari ST/TT (47451)" = 47451,
    "HZ-GB2312 Simplified Chinese (52936)" = 52936,
    "Simplified Chinese GB18030 (54936)" = 54936,
}

fn code_page(name: &str) -> Result<u32, OpError> {
    CODE_PAGES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|&(_, cp)| cp)
        .ok_or_else(|| OpError::new("Invalid encoding"))
}

pub static ENCODE_TEXT: Operation = Operation {
    name: "Encode text",
    category: CATEGORY,
    description: ENCODE_TEXT_DESCRIPTION,
    info_url: Some("https://wikipedia.org/wiki/Character_encoding"),
    args: &[Arg::choice("Encoding", CHARSETS)],
    run: |input, args| {
        let cp = code_page(option(&ENCODE_TEXT, args, 0)?)?;
        codepage::encode(cp, &input.text())
            .map(Dish::Bytes)
            .map_err(OpError::new)
    },
};

pub static DECODE_TEXT: Operation = Operation {
    name: "Decode text",
    category: CATEGORY,
    description: DECODE_TEXT_DESCRIPTION,
    info_url: Some("https://wikipedia.org/wiki/Character_encoding"),
    args: &[Arg::choice("Encoding", CHARSETS)],
    run: |input, args| {
        let cp = code_page(option(&DECODE_TEXT, args, 0)?)?;
        codepage::decode(cp, &input.bytes())
            .map(Dish::Text)
            .map_err(OpError::new)
    },
};

pub static TEXT_ENCODING_BRUTE_FORCE: Operation = Operation {
    name: "Text Encoding Brute Force",
    category: CATEGORY,
    description: BRUTE_FORCE_DESCRIPTION,
    info_url: Some("https://wikipedia.org/wiki/Character_encoding"),
    args: &[Arg::choice("Mode", &["Encode", "Decode"])],
    run: text_encoding_brute_force,
};

/// Every code page's take on the input, as CyberChef's HTML table. Decoding
/// reads the input string's characters as the bytes (text with characters
/// past U+00FF has no such reading, so every page shows "Could not
/// decode."); encoded bytes are shown as UTF-8 when valid, else one
/// character per byte.
fn text_encoding_brute_force(input: Dish, args: &Args) -> Result<Dish, OpError> {
    let decode = option(&TEXT_ENCODING_BRUTE_FORCE, args, 0)? == "Decode";
    let text = input.text();
    let bytes: Option<Vec<u8>> = text.chars().map(|c| u8::try_from(c).ok()).collect();
    let mut table = String::from(
        "<table class='table table-hover table-sm table-bordered table-nonfluid'><tr><th>Encoding</th><th>Value</th></tr>",
    );
    for &(name, cp) in CODE_PAGES {
        let value = if decode {
            bytes
                .as_deref()
                .ok_or_else(String::new)
                .and_then(|b| codepage::decode(cp, b))
        } else {
            codepage::encode(cp, &text).map(|b| bytes_to_utf8(&b))
        };
        let value = value.unwrap_or_else(|_| "Could not decode.".into());
        table.push_str(&format!(
            "<tr><td>{name}</td><td>{}</td></tr>",
            escape_html(&escape_whitespace(&value))
        ));
    }
    table.push_str("<table>");
    Ok(Dish::Html(table))
}

/// CyberChef's `Utils.byteArrayToUtf8`: strict UTF-8 (a leading BOM
/// dropped, as TextDecoder does), else one character per byte.
fn bytes_to_utf8(b: &[u8]) -> String {
    match std::str::from_utf8(b) {
        Ok(s) => s.strip_prefix('\u{FEFF}').unwrap_or(s).to_string(),
        Err(_) => crate::dish::latin1_to_string(b),
    }
}

/// CyberChef's `Utils.escapeWhitespace`: U+0009..U+0010 to the private use
/// characters U+E009..U+E010, which its output pane shows as symbols.
fn escape_whitespace(s: &str) -> String {
    s.chars()
        .map(|c| match u32::from(c) {
            v @ 0x09..=0x10 => char::from_u32(0xE000 + v).unwrap_or(c),
            _ => c,
        })
        .collect()
}

/// CyberChef's `Utils.escapeHtml` (which also turns NUL into U+E000).
fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            '`' => out.push_str("&#x60;"),
            '\0' => out.push('\u{E000}'),
            c => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Braille

const BRAILLE_ASCII: &str = " A1B'K2L@CIF/MSP\"E3H9O6R^DJG>NTQ,*5<-U8V.%[$+X!&;:4\\0Z7(_?W]#Y)=";
const BRAILLE_DOTS: &str = "⠀⠁⠂⠃⠄⠅⠆⠇⠈⠉⠊⠋⠌⠍⠎⠏⠐⠑⠒⠓⠔⠕⠖⠗⠘⠙⠚⠛⠜⠝⠞⠟⠠⠡⠢⠣⠤⠥⠦⠧⠨⠩⠪⠫⠬⠭⠮⠯⠰⠱⠲⠳⠴⠵⠶⠷⠸⠹⠺⠻⠼⠽⠾⠿";

pub static TO_BRAILLE: Operation = Operation {
    name: "To Braille",
    category: CATEGORY,
    description: "Converts text to six-dot braille symbols.",
    info_url: Some("https://wikipedia.org/wiki/Braille"),
    args: &[],
    run: |input, _| {
        let out = input
            .text()
            .chars()
            .map(|c| {
                // indexOf of the upper-cased character, which may be longer
                // than one (ß is SS) or ASCII from non-ASCII (ı is I).
                let upper: String = c.to_uppercase().collect();
                BRAILLE_ASCII
                    .find(&upper)
                    .and_then(|i| BRAILLE_DOTS.chars().nth(i))
                    .unwrap_or(c)
            })
            .collect();
        Ok(Dish::Text(out))
    },
};

pub static FROM_BRAILLE: Operation = Operation {
    name: "From Braille",
    category: CATEGORY,
    description: "Converts six-dot braille symbols to text.",
    info_url: Some("https://wikipedia.org/wiki/Braille"),
    args: &[],
    run: |input, _| {
        let out = input
            .text()
            .chars()
            .map(|c| {
                BRAILLE_DOTS
                    .chars()
                    .position(|d| d == c)
                    .map_or(c, |i| char::from(BRAILLE_ASCII.as_bytes()[i]))
            })
            .collect();
        Ok(Dish::Text(out))
    },
};

// ---------------------------------------------------------------------------
// Caret and M- notation

pub static CARET_M_DECODE: Operation = Operation {
    name: "Caret/M-decode",
    category: CATEGORY,
    description: "Decodes caret or M-encoded strings, i.e. ^M turns into a newline, M-^] turns into 0x9d. Sources such as `cat -v`.\n\nPlease be aware that when using `cat -v` ^_ (caret-underscore) will not be encoded, but represents a valid encoding (namely that of 0x1f).",
    info_url: Some("https://en.wikipedia.org/wiki/Caret_notation"),
    args: &[],
    run: |input, _| byte_array(&caret_m_decode(&input.text())),
};

/// `^X` is X - 64 (`^?` is DEL), `M-X` is X + 128 and `M-^X` is X + 64
/// (`M-^?` is 0xFF); anything else passes through, and an unfinished prefix
/// at the end is dropped, as in CyberChef. Units are stored as bytes, so
/// characters past U+00FF make the result an invalid byte array.
fn caret_m_decode(input: &str) -> Vec<u32> {
    let (m, dash, caret) = (u32::from(b'M'), u32::from(b'-'), u32::from(b'^'));
    let mut bytes: Vec<u32> = vec![];
    let mut prev = "";
    for unit in input.encode_utf16() {
        let c = u32::from(unit);
        prev = match prev {
            "M-^" => {
                match c {
                    64..=95 => bytes.push(c + 64),
                    63 => bytes.push(255),
                    _ => bytes.extend([m, dash, caret, c]),
                }
                ""
            }
            "M-" if c == caret => "M-^",
            "M-" => {
                if (32..=126).contains(&c) {
                    bytes.push(c + 128);
                } else {
                    bytes.extend([m, dash, c]);
                }
                ""
            }
            "M" if c == dash => "M-",
            "M" => {
                bytes.extend([m, c]);
                ""
            }
            "^" => {
                match c {
                    64..=126 => bytes.push(c - 64),
                    63 => bytes.push(127),
                    _ => bytes.extend([caret, c]),
                }
                ""
            }
            _ if c == m => "M",
            _ if c == caret => "^",
            _ => {
                bytes.push(c);
                ""
            }
        };
    }
    bytes
}

// ---------------------------------------------------------------------------
// MIME encoded words

pub static MIME_DECODING: Operation = Operation {
    name: "MIME Decoding",
    category: CATEGORY,
    description: "Enables the decoding of MIME message header extensions for non-ASCII text",
    info_url: Some("https://tools.ietf.org/html/rfc2047"),
    args: &[],
    run: |input, _| mime::decode_headers(&input.bytes()).map(Dish::Text),
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ArgValue;

    fn run(op: &Operation, input: &str, args: &[ArgValue]) -> String {
        match op.apply(Dish::Text(input.into()), args) {
            Ok(d) => d.display().into_owned(),
            Err(e) => e.0,
        }
    }

    fn s(v: &str) -> ArgValue {
        ArgValue::Str(v.into())
    }

    const T: ArgValue = ArgValue::Bool(true);
    const F: ArgValue = ArgValue::Bool(false);

    // Expected values below come from CyberChef itself, except where a
    // comment cites the spec CyberChef deviates from.

    #[test]
    fn html_entities() {
        let named = [F, s("Named entities")];
        assert_eq!(
            run(&TO_HTML_ENTITY, "<é€😀>", &named),
            "&lt;&eacute;&euro;&#128512;&gt;"
        );
        assert_eq!(
            run(&TO_HTML_ENTITY, "a<😀", &[T, s("Hex entities")]),
            "&#x61;&#x3c;&#x1f600;"
        );
        assert_eq!(run(&TO_HTML_ENTITY, "", &named), "");
        assert_eq!(
            run(
                &FROM_HTML_ENTITY,
                "&lt;&#65;&#x42;&#X43;&#x1F600;&#xd83d;&#128;&copy 1&bogus;&toString;",
                &[]
            ),
            "<ABC😀\u{FFFD}€© 1&bogus;&toString;"
        );
    }

    #[test]
    fn url_encoding() {
        assert_eq!(run(&URL_ENCODE, "a b=/é", &[F]), "a%20b=/%E9");
        assert_eq!(run(&URL_ENCODE, "a b=/", &[T]), "a%20b%3D%2F");
        assert_eq!(run(&URL_DECODE, "a+b%3D%E2%82%AC", &[T]), "a b=€");
        // Invalid UTF-8 falls back to unescape().
        assert_eq!(run(&URL_DECODE, "%E2%82%u20AC+", &[F]), "\u{e2}\u{82}€+");
    }

    #[test]
    fn unicode_escapes() {
        assert_eq!(
            run(
                &ESCAPE_UNICODE,
                "σου 😀 a",
                &[s("%u"), F, ArgValue::Num(4.), T]
            ),
            "%u03C3%u03BF%u03C5 %uD83D%uDE00 a"
        );
        assert_eq!(
            run(
                &ESCAPE_UNICODE,
                "σου 😀 a",
                &[s("U+"), T, ArgValue::Num(6.), F]
            ),
            "U+0003c3U+0003bfU+0003c5U+000020U+00d83dU+00de00U+000020U+000061"
        );
        assert_eq!(
            run(&UNESCAPE_UNICODE, "\\ud83d\\uDE00\\U0041\\u12", &[s("\\u")]),
            "😀A\\u12"
        );
        assert_eq!(
            run(&UNESCAPE_UNICODE, "U+1F600 u+00041", &[s("U+")]),
            "😀 A"
        );
        assert_eq!(run(&NORMALISE_UNICODE, "e\u{301}", &[s("NFC")]), "é");
        assert_eq!(
            run(&NORMALISE_UNICODE, "hi", &[s("nfd")]),
            "Unknown Normalisation Form"
        );
        assert_eq!(
            run(&NORMALISE_UNICODE, "hi", &[s("")]),
            "Normal Form cannot be empty."
        );
    }

    #[test]
    fn smart_characters() {
        let text = "“Hello” — world… ☣";
        assert_eq!(
            run(&ESCAPE_SMART, text, &[s("Include")]),
            "\"Hello\" -- world... ☣"
        );
        assert_eq!(
            run(&ESCAPE_SMART, text, &[s("Remove")]),
            "\"Hello\" -- world... "
        );
        assert_eq!(
            run(&ESCAPE_SMART, text, &[s("Replace with '.'")]),
            "\"Hello\" -- world... ."
        );
    }

    #[test]
    fn quoted_printable() {
        assert_eq!(run(&TO_QUOTED_PRINTABLE, "", &[]), "");
        assert_eq!(
            run(&TO_QUOTED_PRINTABLE, "Ünïcödé text = fine\t\nnext", &[]),
            "=DCn=EFc=F6d=E9 text =3D fine=09\r\nnext"
        );
        assert_eq!(run(&FROM_QUOTED_PRINTABLE, "", &[]), "");
        assert_eq!(
            run(&FROM_QUOTED_PRINTABLE, "=C3=9Cn=\r\nx=3d=4", &[]),
            "Ünx==4"
        );
        assert_eq!(
            run(&FROM_QUOTED_PRINTABLE, "Γειά", &[]),
            // CyberChef: "Data is not a valid byteArray: [915,949,953,940]"
            "Data is not a valid byteArray: U+0393 is past 0xFF"
        );
    }

    #[test]
    fn punycode_ops() {
        assert_eq!(run(&TO_PUNYCODE, "", &[F]), "");
        assert_eq!(run(&TO_PUNYCODE, "bücher", &[F]), "bcher-kva");
        assert_eq!(
            run(&TO_PUNYCODE, "user@bücher.example。com", &[T]),
            "user@xn--bcher-kva.example.com"
        );
        assert_eq!(run(&FROM_PUNYCODE, "bcher-kva", &[F]), "bücher");
        assert_eq!(
            run(&FROM_PUNYCODE, "user@xn--bcher-kva.example.com", &[T]),
            "user@bücher.example.com"
        );
        assert_eq!(run(&FROM_PUNYCODE, "ab-ü", &[F]), "Invalid input");
        assert_eq!(run(&FROM_PUNYCODE, "", &[F]), "");
    }

    #[test]
    fn code_pages() {
        let utf16 = [s("UTF-16LE (1200)")];
        assert_eq!(
            ENCODE_TEXT.apply(Dish::Text("hi".into()), &utf16).unwrap(),
            Dish::Bytes(b"h\0i\0".to_vec())
        );
        assert_eq!(run(&DECODE_TEXT, "h\0i\0", &utf16), "hi");
        assert_eq!(
            run(&ENCODE_TEXT, "hi", &[s("")]),
            "Encoding cannot be empty."
        );
        assert_eq!(
            run(&ENCODE_TEXT, "hi", &[s("utf-8 (65001)")]),
            "Invalid encoding"
        );
        // ISO-2022-KR (RFC 1557), unknown to CyberChef's package.
        assert_eq!(run(&DECODE_TEXT, "A", &[s("ISO 2022 Korean (50225)")]), "A");
        // Unmappable characters are errors, not 0x00 bytes.
        assert!(
            ENCODE_TEXT
                .apply(Dish::Text("☣".into()), &[s("Windows-1252 Latin (1252)")])
                .is_err()
        );
        assert!(
            run(&ENCODE_TEXT, "hi", &[s("nope")])
                .starts_with("Encoding must be one of the following: UTF-8 (65001), UTF-7")
        );
        let table = run(&TEXT_ENCODING_BRUTE_FORCE, "hi", &[s("Encode")]);
        assert!(table.starts_with("<table class='table table-hover table-sm table-bordered table-nonfluid'><tr><th>Encoding</th><th>Value</th></tr><tr><td>UTF-8 (65001)</td><td>hi</td></tr>"));
        assert!(table.contains("<tr><td>UTF-16LE (1200)</td><td>h\u{e000}i\u{e000}</td></tr>"));
        assert!(table.contains("<tr><td>ISO 2022 Korean (50225)</td><td>hi</td></tr>"));
        let table = run(&TEXT_ENCODING_BRUTE_FORCE, "\u{e9}", &[s("Decode")]);
        assert!(table.contains("<tr><td>Windows-1252 Latin (1252)</td><td>\u{e9}</td></tr>"));
        assert!(
            table.contains("<tr><td>US-ASCII (7-bit) (20127)</td><td>Could not decode.</td></tr>")
        );
        assert!(table.ends_with("</tr><table>"));
    }

    #[test]
    fn braille() {
        assert_eq!(
            run(&TO_BRAILLE, "Hello, World! ßı", &[]),
            "⠓⠑⠇⠇⠕⠠⠀⠺⠕⠗⠇⠙⠮⠀ß⠊"
        );
        assert_eq!(run(&FROM_BRAILLE, "⠓⠑⠇⠇⠕ x", &[]), "HELLO x");
        assert_eq!(run(&TO_BRAILLE, "", &[]), "");
    }

    #[test]
    fn caret_and_mime() {
        let decoded = CARET_M_DECODE
            .apply(Dish::Text("^M^?M-aM-^]M-^?x^".into()), &[])
            .unwrap();
        assert_eq!(
            decoded,
            Dish::Bytes(vec![0x0D, 0x7F, 0xE1, 0x9D, 0xFF, b'x'])
        );
        assert_eq!(
            run(&CARET_M_DECODE, "Ā", &[]),
            "Data is not a valid byteArray: [256]"
        );
        assert_eq!(
            run(
                &MIME_DECODING,
                "Subject: =?UTF-8?Q?a?= =?UTF-8?B?w6k=?=\r\n x",
                &[]
            ),
            "Subject: aé\n x"
        );
        assert_eq!(run(&MIME_DECODING, "=?koi8-r?B?QQ==?=", &[]), "A");
    }
}
