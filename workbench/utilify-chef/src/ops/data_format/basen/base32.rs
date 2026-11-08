//! Base32 (RFC 4648 §6 and the extended-hex alphabet of §7) and Base45
//! (RFC 9285).
//!
//! The codecs are libraries: `data-encoding` for Base32, `utilify-base45`
//! for Base45. Both work in their standard alphabet; the adapters map
//! symbols to and from the alphabet argument by index, so any alphabet of
//! the right size works (CyberChef's argument, including emoji: symbols are
//! code points). An alphabet of the wrong size, or with a repeated symbol,
//! is an error rather than output that loses digits or decodes ambiguously.
//!
//! - Base32 (`data-encoding`, RFC 4648): 32 symbols plus an optional 33rd,
//!   the padding character. Padding ends the data (§3.3, §6), so several
//!   padded encodings joined together are an error; characters outside the
//!   alphabet (when not removed, §3.3), bad lengths, misplaced padding and
//!   non-zero pad bits (§3.5) are errors. Unpadded input decodes (§3.2).
//! - Base45 (`utilify-base45`, RFC 9285): every digit of a group is the
//!   alphabet's symbol (§4.3), including high zero digits. A dangling last
//!   symbol, a final pair above 255 or a triplet above 65535 is an error
//!   (§6); out-of-alphabet characters and large triplets keep CyberChef's
//!   wording.

use super::{
    CATEGORY, find_units, from_utf16, js_alphabet, keep_only, standard_symbols, symbol_value,
    thrown, utf16,
};
use crate::{Arg, Dish, OpError, Operation};
use data_encoding::{BASE32, BASE32_NOPAD};
use std::collections::HashSet;
use utilify_base45::{Alphabet, ErrorKind};

/// CyberChef's `Base32.ALPHABET_OPTIONS`.
const BASE32_ALPHABETS: &[(&str, &str)] = &[("Standard", "A-Z2-7="), ("Hex Extended", "0-9A-V=")];

/// Used when the alphabet argument is empty.
const BASE32_DEFAULT: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567=";

const BASE32_DESCRIPTION: &str = "Base32 is a notation for encoding arbitrary byte data using a restricted set of symbols that can be conveniently used by humans and processed by computers. It uses a smaller set of characters than Base64, usually the uppercase alphabet and the numbers 2 to 7.";

pub static TO_BASE32: Operation = Operation {
    name: "To Base32",
    category: CATEGORY,
    description: BASE32_DESCRIPTION,
    info_url: Some("https://wikipedia.org/wiki/Base32"),
    args: &[Arg::editable("Alphabet", BASE32_ALPHABETS)],
    run: |input, args| {
        // Indexed by code point (`Array.from` of the joined alphabet), so
        // emoji alphabets work; index 32 is the padding character, or
        // nothing when the alphabet has none.
        let units = js_alphabet(match args.str(0) {
            "" => BASE32_DEFAULT,
            a => a,
        });
        let alph = code_points(&units);
        check_alphabet(
            &alph,
            &[32, 33],
            "Base32 alphabet must be 32 distinct symbols, or 33 with a padding character",
        )?;
        let out: Vec<u16> = BASE32
            .encode(&input.bytes())
            .bytes()
            .filter_map(|s| alph.get(symbol_value(BASE32_DEFAULT, s)))
            .flat_map(|c| c.iter().copied())
            .collect();
        Ok(Dish::Text(from_utf16(&out)))
    },
};

pub static FROM_BASE32: Operation = Operation {
    name: "From Base32",
    category: CATEGORY,
    description: BASE32_DESCRIPTION,
    info_url: Some("https://wikipedia.org/wiki/Base32"),
    args: &[
        Arg::editable("Alphabet", BASE32_ALPHABETS),
        Arg::flag("Remove non-alphabet chars", true),
    ],
    run: |input, args| {
        let input = utf16(&input.text());
        if input.is_empty() {
            return Ok(Dish::Bytes(vec![]));
        }
        let alph = js_alphabet(match args.str(0) {
            "" => BASE32_DEFAULT,
            a => a,
        });
        check_alphabet(
            &code_points(&alph),
            &[32, 33],
            "Base32 alphabet must be 32 distinct symbols, or 33 with a padding character",
        )?;
        let data = if args.flag(1) {
            keep_only(&input, &alph)
        } else {
            input
        };
        let std = standard_symbols(&data, &alph, BASE32_DEFAULT);
        // RFC 4648 §3.3: padding ends the data; only padding may follow it.
        if let Some(p) = std.iter().position(|&c| c == b'=')
            && let Some(q) = std[p..].iter().position(|&c| c != b'=')
        {
            return Err(thrown(
                "From Base32",
                format!("data after padding at {}", p + q),
            ));
        }
        let encoding = if std.contains(&b'=') {
            &BASE32
        } else {
            &BASE32_NOPAD
        };
        encoding
            .decode(&std)
            .map(Dish::Bytes)
            .map_err(|e| thrown("From Base32", e))
    },
};

const BASE45_ALPHABET: &str = "0-9A-Z $%*+\\-./:";

const BASE45_DESCRIPTION: &str = "Base45 is a notation for encoding arbitrary byte data using a restricted set of symbols that can be conveniently used by humans and processed by computers. The high number base results in shorter strings than with the decimal or hexadecimal system. Base45 is optimized for usage with QR codes.";

pub static TO_BASE45: Operation = Operation {
    name: "To Base45",
    category: CATEGORY,
    description: BASE45_DESCRIPTION,
    info_url: Some("https://wikipedia.org/wiki/List_of_numeral_systems"),
    args: &[Arg::string("Alphabet", BASE45_ALPHABET)],
    run: |input, args| {
        let units = js_alphabet(args.str(0));
        let alph = code_points(&units);
        check_alphabet(&alph, &[45], BASE45_ALPHABET_ERROR)?;
        // The crate's RFC 9285 symbols, each mapped to the alphabet's.
        let out: Vec<u16> = utilify_base45::encode(&input.bytes())
            .chars()
            .filter_map(|c| Alphabet::RFC9285.value_of(c))
            .filter_map(|d| alph.get(usize::from(d)))
            .flat_map(|c| c.iter().copied())
            .collect();
        Ok(Dish::Text(from_utf16(&out)))
    },
};

const BASE45_ALPHABET_ERROR: &str = "Base45 alphabet must be 45 distinct symbols";

/// The alphabet's code points must be one of `sizes` long, with no repeats.
fn check_alphabet(points: &[&[u16]], sizes: &[usize], msg: &str) -> Result<(), OpError> {
    let distinct = points.iter().collect::<HashSet<_>>().len() == points.len();
    if distinct && sizes.contains(&points.len()) {
        Ok(())
    } else {
        Err(OpError::new(msg))
    }
}

pub static FROM_BASE45: Operation = Operation {
    name: "From Base45",
    category: CATEGORY,
    description: BASE45_DESCRIPTION,
    info_url: Some("https://wikipedia.org/wiki/List_of_numeral_systems"),
    args: &[
        Arg::string("Alphabet", BASE45_ALPHABET),
        Arg::flag("Remove non-alphabet chars", true),
    ],
    run: |input, args| {
        let input = utf16(&input.text());
        if input.is_empty() {
            return Ok(Dish::Bytes(vec![]));
        }
        let alph = js_alphabet(args.str(0));
        check_alphabet(&code_points(&alph), &[45], BASE45_ALPHABET_ERROR)?;
        let data = if args.flag(1) {
            keep_only(&input, &alph)
        } else {
            input
        };
        // Each code point to the RFC 9285 symbol for its `indexOf` in the
        // alphabet, so the library sees one ASCII byte per code point.
        let points = code_points(&data);
        let mut std = String::with_capacity(points.len());
        for c in &points {
            let idx = find_units(&alph, c);
            if idx < 0 {
                return Err(OpError::new(format!(
                    "Character not in alphabet: '{}'",
                    from_utf16(c)
                )));
            }
            match Alphabet::RFC9285.symbols().get(idx as usize) {
                Some(&s) => std.push(s),
                None => {
                    return Err(thrown(
                        "From Base45",
                        format!("'{}' is past the alphabet's 45th symbol", from_utf16(c)),
                    ));
                }
            }
        }
        utilify_base45::decode(&std).map(Dish::Bytes).map_err(|e| {
            let at = e.position();
            match e.kind() {
                ErrorKind::TripletOutOfRange { .. } => {
                    let rev: Vec<u16> = points[at..(at + 3).min(points.len())]
                        .iter()
                        .rev()
                        .flat_map(|c| c.iter().copied())
                        .collect();
                    OpError::new(format!("Triplet too large: '{}'", from_utf16(&rev)))
                }
                _ => thrown("From Base45", e),
            }
        })
    },
};

/// Code units grouped into code points, as a JavaScript string iterates;
/// a lone surrogate is its own point.
fn code_points(units: &[u16]) -> Vec<&[u16]> {
    let mut out = Vec::with_capacity(units.len());
    let mut i = 0;
    while i < units.len() {
        let pair = (0xd800..0xdc00).contains(&units[i])
            && units
                .get(i + 1)
                .is_some_and(|u| (0xdc00..0xe000).contains(u));
        let n = if pair { 2 } else { 1 };
        out.push(&units[i..i + n]);
        i += n;
    }
    out
}

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
    fn base32() {
        assert_eq!(run(&TO_BASE32, "a", &[s("")]), "ME======");
        assert_eq!(run(&TO_BASE32, "abc", &[s("A-Z2-7")]), "MFRGG");
        // Emoji by code point, but ranges by code unit: "😀-😂" is 😂 alone.
        assert_eq!(run(&TO_BASE32, "\u{ff}", &[s("A-Z2-6🀄=")]), "🀄4======");
        assert_eq!(run(&TO_BASE32, "\u{ff}", &[s("A-Z2-6😀-😂=")]), "😂4======");
        assert_eq!(run(&FROM_BASE32, "ME", &[]), "a");
        assert_eq!(
            run(&FROM_BASE32, "!!!!", &[s("A-Z2-7="), b(false)]),
            // CyberChef: "Data is not a valid byteArray: [-1,-1,240]".
            "From Base32 - invalid symbol at 0"
        );
        assert_eq!(run(&FROM_BASE32, "", &[]), "");
        // RFC 4648 §3.3: padding ends the data.
        assert_eq!(
            run(&FROM_BASE32, "ME======ME======", &[]),
            "From Base32 - data after padding at 8"
        );
        assert!(run(&TO_BASE32, "a", &[s("A-Z")]).contains("32 distinct"));
        // RFC 4648 §10.
        for (plain, enc) in [
            ("f", "MY======"),
            ("foob", "MZXW6YQ="),
            ("foobar", "MZXW6YTBOI======"),
        ] {
            assert_eq!(run(&TO_BASE32, plain, &[s("A-Z2-7=")]), enc);
            assert_eq!(run(&FROM_BASE32, enc, &[]), plain);
        }
        assert_eq!(
            run(&TO_BASE32, "foobar", &[s("0-9A-V=")]),
            "CPNMUOJ1E8======"
        );
    }

    #[test]
    fn base45() {
        // RFC 9285 §4.3.
        assert_eq!(run(&TO_BASE45, "AB", &[]), "BB8");
        assert_eq!(run(&TO_BASE45, "Hello!!", &[]), "%69 VD92EX0");
        assert_eq!(run(&FROM_BASE45, "QED8WEX0", &[]), "ietf!");
        // High zero digits come from the alphabet too.
        let shifted = s("1-9A-Z $%*+\\-./:0");
        assert_eq!(run(&TO_BASE45, "\0", std::slice::from_ref(&shifted)), "11");
        assert_eq!(
            run(&FROM_BASE45, "11", &[shifted, ArgValue::Bool(true)]),
            "\0"
        );
        assert_eq!(run(&TO_BASE45, "a", &[s("A-Z")]), BASE45_ALPHABET_ERROR);
        assert_eq!(run(&FROM_BASE45, "BB8", &[]), "AB");
        assert_eq!(run(&FROM_BASE45, "GGW", &[]), "Triplet too large: 'WGG'");
        assert_eq!(
            run(&FROM_BASE45, "BB8a", &[s(BASE45_ALPHABET), b(false)]),
            "Character not in alphabet: 'a'"
        );
    }
}
