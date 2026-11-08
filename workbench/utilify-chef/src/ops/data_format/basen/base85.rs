//! Base85 and Base92, on `utilify-base85` and `utilify-base92`.
//!
//! Base85 is the Ascii85 group arithmetic of the PLRM (3rd ed. §3.13.3):
//! 4 bytes ↔ 5 base-85 digits, big-endian, a final group of n bytes ↔
//! n + 1 digits, `z` for an all-zero *full* group only, groups above
//! 2^32 − 1 rejected, and a final group of a single character an error.
//! The alphabet argument picks the 85 symbols (Ascii85, the Z85 alphabet
//! of ZMQ RFC 32, the RFC 1924 alphabet, or any other): this is CyberChef's
//! argument and the crate's `Z85_PADDED` / `RFC1924` presets read it the
//! same way, as an alphabet over Ascii85 grouping (so Z85's
//! multiple-of-four framing is not enforced, and the IPv6 alphabet is not
//! RFC 1924's single 128-bit number). `z` is written only with the
//! Standard alphabet, as in CyberChef.
//!
//! Delimiters and white space follow the PLRM: To Base85's "Include
//! delimiter" writes the `<~` … `~>` string form (§3.2.2); From Base85
//! accepts an optional leading `<~`, ends the data at `~>` (required after
//! `<~`), rejects a `~` not followed by `>` and anything but white space
//! after `~>`, and ignores PostScript white space (Table 3.1: NUL, TAB,
//! LF, FF, CR, SP). Delimiters are recognised only when `~` is not an
//! alphabet symbol or the all-zero group char, and white space is ignored
//! only when none of those is white space; otherwise they are ordinary
//! characters. "Remove non-alphabet chars" (CyberChef's argument) drops
//! every other character before decoding, keeping the delimiters.
//!
//! CyberChef's alphabet and all-zero group char are strings of UTF-16 code
//! units, `utilify-base85`'s `UnicodeAlphabet` takes `char`s, so each code
//! unit stands for one `char` ([`unit_char`], surrogates as supplementary
//! private-use characters); the crate's character indices are then
//! code-unit indices. A symbol that is half of a surrogate pair is written
//! as U+FFFD when output alone (our text is a Rust `String`).
//!
//! Base92 is thenoviceoof's Base92 (README, DESIGN DECISIONS), straight
//! from `utilify-base92` with its spec options: To Base92 encodes the
//! input's bytes and writes `~` for empty input (¶3); From Base92 is
//! strict, so lengths no byte string encodes to, non-zero padding bits and
//! out-of-range groups are errors.

use super::{CATEGORY, all_unique, char_unit, from_utf16, js_alphabet, thrown, unit_char, utf16};
use crate::{Arg, Dish, OpError, Operation};
use utilify_base85::{ErrorKind, UnicodeAlphabet, UnicodeOptions};

/// CyberChef's `Base85.ALPHABET_OPTIONS`.
const BASE85_ALPHABETS: &[(&str, &str)] = &[
    ("Standard", "!-u"),
    ("Z85 (ZeroMQ)", "0-9a-zA-Z.\\-:+=^!/*?&<>()[]{}@%$#"),
    ("IPv6", "0-9A-Za-z!#$%&()*+\\-;<=>?@^_`{|}~"),
];

/// The alphabet argument's code units as a crate alphabet, if they are 85
/// distinct units.
fn base85_alphabet(alph: &[u16]) -> Option<UnicodeAlphabet> {
    if alph.len() != 85 || !all_unique(alph) {
        return None;
    }
    let symbols: Vec<char> = alph.iter().map(|&u| unit_char(u)).collect();
    UnicodeAlphabet::new(&symbols).ok()
}

/// PostScript white space (PLRM 3rd ed. Table 3.1).
fn is_ps_space(u: u16) -> bool {
    matches!(u, 0x00 | 0x09 | 0x0a | 0x0c | 0x0d | 0x20)
}

const TILDE: u16 = b'~' as u16;

pub static TO_BASE85: Operation = Operation {
    name: "To Base85",
    category: CATEGORY,
    description: "Base85 (also called Ascii85) is a notation for encoding arbitrary byte data. It is usually more efficient that Base64.\n\nThis operation encodes data in an ASCII string (with an alphabet of your choosing, presets included).\n\ne.g. hello world becomes BOu!rD]j7BEbo7\n\nBase85 is commonly used in Adobe's PostScript and PDF file formats.\n\nOptions\nAlphabet\n- Standard - The standard alphabet, referred to as Ascii85\n- Z85 (ZeroMQ) - A string-safe variant of Base85, which avoids quote marks and backslash characters\n- IPv6 - A variant of Base85 suitable for encoding IPv6 addresses (RFC 1924)\nInclude delimiter\nAdds a '<~' and '~>' delimiter to the start and end of the data. This is standard for Adobe's implementation of Base85.",
    info_url: Some("https://wikipedia.org/wiki/Ascii85"),
    args: &[
        Arg::editable("Alphabet", BASE85_ALPHABETS),
        Arg::flag("Include delimiter", false),
    ],
    run: |input, args| {
        let alph = js_alphabet(args.str(0));
        // Only the exact Standard alphabet shortens all-zero groups to `z`.
        let standard = alph == js_alphabet(BASE85_ALPHABETS[0].1);
        let Some(alphabet) = base85_alphabet(&alph) else {
            return Err(OpError::new("Error: Alphabet must be of length 85"));
        };
        let delimit = args.flag(1);
        // `~>` could not end the data if `~` were a symbol.
        if delimit && alph.contains(&TILDE) {
            return Err(thrown(
                "To Base85",
                "delimiters cannot be used with an alphabet containing '~'",
            ));
        }
        // PLRM §3.13.3: `z` stands for full all-zero groups only.
        let options = UnicodeOptions::new(alphabet).zero_group(standard.then_some('z'));
        let text = options
            .encode(input.bytes())
            .map_err(|e| thrown("To Base85", e))?;
        let text = from_utf16(&text.chars().map(char_unit).collect::<Vec<_>>());
        // PLRM §3.2.2: an ASCII base-85 string is `<~` … `~>`, empty or not.
        Ok(Dish::Text(if delimit {
            format!("<~{text}~>")
        } else {
            text
        }))
    },
};

pub static FROM_BASE85: Operation = Operation {
    name: "From Base85",
    category: CATEGORY,
    description: "Base85 (also called Ascii85) is a notation for encoding arbitrary byte data. It is usually more efficient that Base64.\n\nThis operation decodes data from an ASCII string (with an alphabet of your choosing, presets included).\n\ne.g. BOu!rD]j7BEbo7 becomes hello world\n\nBase85 is commonly used in Adobe's PostScript and PDF file formats.",
    info_url: Some("https://wikipedia.org/wiki/Ascii85"),
    args: &[
        Arg::editable("Alphabet", BASE85_ALPHABETS),
        Arg::flag("Remove non-alphabet chars", true),
        Arg::binary("All-zero group char", "z"),
    ],
    run: |input, args| {
        let alph = js_alphabet(args.str(0));
        let zero = utf16(args.str(2)).first().copied();
        let Some(alphabet) = base85_alphabet(&alph) else {
            return Err(OpError::new("Alphabet must be of length 85"));
        };
        if zero.is_some_and(|z| alph.contains(&z)) {
            return Err(OpError::new(
                "The all-zero group char cannot appear in the alphabet",
            ));
        }
        let fail = |msg: String| thrown("From Base85", msg);
        // `~` and white space have their PLRM meaning unless they are symbols.
        let delims = !alph.contains(&TILDE) && zero != Some(TILDE);
        let ws = !alph.iter().any(|&u| is_ps_space(u)) && !zero.is_some_and(is_ps_space);
        let mut data = utf16(&input.text());
        if args.flag(1) {
            data = remove_non_alphabet(&data, &alph, zero, delims);
        }
        let (start, end) = if delims {
            frame(&data, ws).map_err(fail)?
        } else {
            (0, data.len())
        };
        let body = &data[start..end];
        // One char per code unit, so character indices are unit indices.
        let text: String = body.iter().map(|&u| unit_char(u)).collect();
        let options = UnicodeOptions::new(alphabet)
            .zero_group(zero.map(unit_char))
            .ignore_whitespace(ws);
        options.decode(&text).map(Dish::Bytes).map_err(|e| {
            let Some(i) = e.char_index() else {
                return thrown("From Base85", e);
            };
            let unit = from_utf16(
                body.get(i as usize..)
                    .unwrap_or(&[])
                    .get(..1)
                    .unwrap_or(&[]),
            );
            let i = i + start as u64;
            fail(match e.kind() {
                ErrorKind::InvalidChar { .. } => format!("Invalid character '{unit}' at index {i}"),
                ErrorKind::MisplacedShorthandChar { .. } => {
                    format!("group shorthand '{unit}' inside a group at index {i}")
                }
                kind => format!("{kind} at index {i}"),
            })
        })
    },
};

/// "Remove non-alphabet chars": keeps the symbols, the all-zero group char
/// and, when `delims`, the `<~` and `~>` delimiters.
fn remove_non_alphabet(data: &[u16], alph: &[u16], zero: Option<u16>, delims: bool) -> Vec<u16> {
    let (lt, gt) = (u16::from(b'<'), u16::from(b'>'));
    data.iter()
        .enumerate()
        .filter(|&(i, &c)| {
            alph.contains(&c)
                || Some(c) == zero
                || (delims
                    && (c == TILDE
                        || (c == lt && data.get(i + 1) == Some(&TILDE))
                        || (c == gt && i > 0 && data[i - 1] == TILDE)))
        })
        .map(|(_, &c)| c)
        .collect()
}

/// The data between the PLRM delimiters (3rd ed. §3.13.3, §3.2.2), as a
/// range of `data`: an optional leading `<~` (after white space when `ws`),
/// then everything up to the first `~`, which must be followed by `>` and
/// then only white space. `~>` is required after `<~`.
fn frame(data: &[u16], ws: bool) -> Result<(usize, usize), String> {
    let space = |u: u16| ws && is_ps_space(u);
    let first = data.iter().position(|&u| !space(u)).unwrap_or(data.len());
    let opened = data[first..].starts_with(&[u16::from(b'<'), TILDE]);
    let start = if opened { first + 2 } else { 0 };
    let Some(t) = data[start..]
        .iter()
        .position(|&u| u == TILDE)
        .map(|p| start + p)
    else {
        if opened {
            return Err(format!(
                "{} at index {}",
                ErrorKind::MissingEndDelimiter,
                data.len()
            ));
        }
        return Ok((start, data.len()));
    };
    if data.get(t + 1) != Some(&u16::from(b'>')) {
        return Err(format!("{} at index {t}", ErrorKind::InvalidEndDelimiter));
    }
    if let Some(p) = data[t + 2..].iter().position(|&u| !space(u)) {
        return Err(format!(
            "{} at index {}",
            ErrorKind::TrailingData,
            t + 2 + p
        ));
    }
    Ok((start, t))
}

const BASE92_DESCRIPTION: &str = "Base92 is a notation for encoding arbitrary byte data using a restricted set of symbols that can be conveniently used by humans and processed by computers.";

pub static TO_BASE92: Operation = Operation {
    name: "To Base92",
    category: CATEGORY,
    description: BASE92_DESCRIPTION,
    info_url: Some("https://wikipedia.org/wiki/List_of_numeral_systems"),
    args: &[],
    run: |input, _| {
        Ok(Dish::Bytes(
            utilify_base92::encode_with(&input.bytes(), BASE92).into_bytes(),
        ))
    },
};

/// The Base92 README's rules (DESIGN DECISIONS): `~` for empty input,
/// strict decoding.
const BASE92: utilify_base92::Options = utilify_base92::Options::SPEC;

pub static FROM_BASE92: Operation = Operation {
    name: "From Base92",
    category: CATEGORY,
    description: BASE92_DESCRIPTION,
    info_url: Some("https://wikipedia.org/wiki/List_of_numeral_systems"),
    args: &[],
    run: |input, _| {
        let text = input.text();
        utilify_base92::decode_with(text.as_bytes(), BASE92)
            .map(Dish::Bytes)
            .map_err(|e| match e.kind() {
                utilify_base92::ErrorKind::InvalidCharacter { .. } => {
                    let c = usize::try_from(e.position())
                        .ok()
                        .and_then(|p| text.get(p..))
                        .and_then(|t| t.chars().next())
                        .unwrap_or(char::REPLACEMENT_CHARACTER);
                    OpError::new(format!("{c} is not a base92 character"))
                }
                _ => thrown("From Base92", e),
            })
    },
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
    fn b(v: bool) -> ArgValue {
        ArgValue::Bool(v)
    }

    #[test]
    fn base85() {
        assert_eq!(run(&TO_BASE85, "hello world", &[]), "BOu!rD]j7BEbo7");
        // PLRM §3.13.3: `z` only for a full all-zero group.
        assert_eq!(run(&TO_BASE85, "\0\0\0\0", &[]), "z");
        assert_eq!(run(&TO_BASE85, "\0", &[]), "!!");
        assert_eq!(run(&TO_BASE85, "\0\0\0\0\0", &[]), "z!!");
        assert_eq!(run(&TO_BASE85, "", &[s("!-u"), b(true)]), "<~~>");
        assert!(
            run(&TO_BASE85, "a", &[s(BASE85_ALPHABETS[2].1), b(true)])
                .contains("alphabet containing '~'")
        );
        assert_eq!(run(&FROM_BASE85, "<~BOu!rD]j7BEbo7~>", &[]), "hello world");
        assert_eq!(run(&FROM_BASE85, "!!", &[]), "\0");
        assert_eq!(
            run(&FROM_BASE85, "BOu!r9", &[s("!-u"), b(false), s("z")]),
            "From Base85 - final group has a single character at index 5"
        );
        // PLRM: white space is ignored, `~>` ends the data.
        assert_eq!(
            run(
                &FROM_BASE85,
                " BOu!r\nD]j7B Ebo7~>\n",
                &[s("!-u"), b(false), s("z")]
            ),
            "hello world"
        );
        assert_eq!(
            run(
                &FROM_BASE85,
                "<~BOu!rD]j7BEbo7",
                &[s("!-u"), b(false), s("z")]
            ),
            "From Base85 - missing '~>' end delimiter at index 16"
        );
        assert_eq!(
            run(&FROM_BASE85, "z~", &[s("!-u"), b(true), s("z")]),
            "From Base85 - '~' not followed by '>' at index 1"
        );
        assert_eq!(
            run(&FROM_BASE85, "z~>!!", &[s("!-u"), b(false), s("z")]),
            "From Base85 - data after '~>' at index 3"
        );
        assert_eq!(
            run(&FROM_BASE85, "s8W-\"", &[s("!-u"), b(false), s("z")]),
            "From Base85 - group value out of range at index 0"
        );
        assert_eq!(
            run(&FROM_BASE85, "abc", &[s("!-u"), b(true), s("-")]),
            "The all-zero group char cannot appear in the alphabet"
        );
        // With `~` in the alphabet there are no delimiters.
        let ipv6 = s(BASE85_ALPHABETS[2].1);
        assert_eq!(
            run(&FROM_BASE85, "ab]c", &[ipv6, b(false), s("\\")]),
            "From Base85 - Invalid character ']' at index 2"
        );
        assert_eq!(
            run(
                &FROM_BASE85,
                "!\u{2205}",
                &[s("!-u"), b(false), s("\u{2205}")]
            ),
            "From Base85 - group shorthand '\u{2205}' inside a group at index 1"
        );
        // A non-BMP character is two code units, so digits 0 and 1.
        let emoji: String = std::iter::once('\u{1F600}')
            .chain(('!'..='~').filter(|c| !matches!(c, '-' | '\\')).take(83))
            .collect();
        assert_eq!(
            run(&FROM_BASE85, "\u{1F600}##", &[s(&emoji), b(false), s("z")]),
            run(&FROM_BASE85, "!\"%%", &[])
        );
    }

    // Base92 README, USAGE.
    #[test]
    fn base92() {
        assert_eq!(run(&TO_BASE92, "hello world", &[]), "Fc_$aOTdKnsM*k");
        assert_eq!(run(&TO_BASE92, "", &[]), "~");
        // The input's bytes: U+0101 is UTF-8 c4 81.
        assert_eq!(
            run(&TO_BASE92, "\u{101}", &[]),
            utilify_base92::encode("\u{101}".as_bytes())
        );
        assert_eq!(run(&FROM_BASE92, "Fc_$aOTdKnsM*k", &[]), "hello world");
        assert_eq!(run(&FROM_BASE92, "~", &[]), "");
        assert!(run(&FROM_BASE92, "}}}", &[]).contains("out-of-range"));
        assert!(run(&FROM_BASE92, "D-", &[]).starts_with("From Base92 - "));
        assert_eq!(run(&FROM_BASE92, " ", &[]), "  is not a base92 character");
    }
}
