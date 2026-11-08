//! Base58 (Bitcoin and Ripple alphabets) and Base62, both whole-number
//! conversions of the input, on `utilify-base58` and `utilify-base62`.
//!
//! Base58 follows draft-msporny-base58-03 §3: each leading zero byte is one
//! leading zero symbol (the alphabet's first), both ways. "Remove
//! non-alphabet chars" (CyberChef's argument) drops other characters
//! before decoding, so leading zeros are counted on what remains: "x11"
//! and "11" both decode to two zero bytes.
//!
//! Base62 has no formal specification. It is plain positional notation
//! (`LeadingZeros::Integer`, as in CyberChef): leading zero bytes are not
//! kept, zero encodes as a single zero symbol, and decoding yields the
//! minimal big-endian bytes. The alphabet must be 62 distinct characters.
//! From Base62 ignores characters outside the alphabet, as CyberChef's op
//! does; text without any alphabet symbol decodes to nothing.
//!
//! The alphabets are strings of UTF-16 code units (CyberChef's argument),
//! while the crates take Unicode scalar values: each code unit stands for
//! one `char` ([`unit_char`]), surrogates for private-use characters
//! outside the BMP, and the crates' output is mapped back unit by unit.

use super::{
    CATEGORY, all_unique, char_unit, from_utf16, index_of, js_alphabet, thrown, unit_char, utf16,
};
use crate::{Arg, Dish, OpError, Operation};
use utilify_base62::{InvalidSymbols, LeadingZeros, Options};

/// CyberChef's `Base58.ALPHABET_OPTIONS`.
const BASE58_ALPHABETS: &[(&str, &str)] = &[
    (
        "Bitcoin",
        "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz",
    ),
    (
        "Ripple",
        "rpshnaf39wBUDNEGHJKLM4PQRST7VWXYZ2bcdeCg65jkm8oFqi1tuvAxyz",
    ),
];

/// Code units as a string of [`unit_char`]s.
fn unit_str(units: &[u16]) -> String {
    units.iter().map(|&u| unit_char(u)).collect()
}

/// A crate's encoded output back to text, via [`char_unit`].
fn unit_text(encoded: &str) -> String {
    from_utf16(&encoded.chars().map(char_unit).collect::<Vec<_>>())
}

/// The alphabet argument, or Bitcoin's when it's empty.
fn base58_alphabet(
    arg: &str,
    error: &str,
) -> Result<(Vec<u16>, utilify_base58::Alphabet), OpError> {
    let alph = js_alphabet(if arg.is_empty() {
        BASE58_ALPHABETS[0].1
    } else {
        arg
    });
    if alph.len() != 58 || !all_unique(&alph) {
        return Err(OpError::new(error));
    }
    let chars: Vec<char> = alph.iter().map(|&u| unit_char(u)).collect();
    let crate_alph =
        utilify_base58::Alphabet::from_chars(&chars).map_err(|_| OpError::new(error))?;
    Ok((alph, crate_alph))
}

pub static TO_BASE58: Operation = Operation {
    name: "To Base58",
    category: CATEGORY,
    description: "Base58 (similar to Base64) is a notation for encoding arbitrary byte data. It differs from Base64 by removing easily misread characters (i.e. l, I, 0 and O) to improve human readability.\n\nThis operation encodes data in an ASCII string (with an alphabet of your choosing, presets included).\n\ne.g. hello world becomes StV1DL6CwTryKyV\n\nBase58 is commonly used in cryptocurrencies (Bitcoin, Ripple, etc).",
    info_url: Some("https://wikipedia.org/wiki/Base58"),
    args: &[Arg::editable("Alphabet", BASE58_ALPHABETS)],
    run: |input, args| {
        let (_, alph) = base58_alphabet(args.str(0), "Error: alphabet must be of length 58")?;
        Ok(Dish::Text(unit_text(&alph.encode(&input.bytes()))))
    },
};

pub static FROM_BASE58: Operation = Operation {
    name: "From Base58",
    category: CATEGORY,
    description: "Base58 (similar to Base64) is a notation for encoding arbitrary byte data. It differs from Base64 by removing easily misread characters (i.e. l, I, 0 and O) to improve human readability.\n\nThis operation decodes data from an ASCII string (with an alphabet of your choosing, presets included) back into its raw form.\n\ne.g. StV1DL6CwTryKyV becomes hello world\n\nBase58 is commonly used in cryptocurrencies (Bitcoin, Ripple, etc).",
    info_url: Some("https://wikipedia.org/wiki/Base58"),
    args: &[
        Arg::editable("Alphabet", BASE58_ALPHABETS),
        Arg::flag("Remove non-alphabet chars", true),
    ],
    run: |input, args| {
        let (units, alph) = base58_alphabet(args.str(0), "Alphabet must be of length 58")?;
        let input = utf16(&input.text());
        let mut digits = Vec::with_capacity(input.len());
        for (pos, &c) in input.iter().enumerate() {
            if index_of(&units, c) >= 0 {
                digits.push(c);
            } else if !args.flag(1) {
                return Err(OpError::new(format!(
                    "Char '{}' at position {pos} not in alphabet",
                    from_utf16(&[c])
                )));
            }
        }
        // Leading zero symbols are counted after filtering (§3).
        let out = alph
            .decode(unit_str(&digits))
            .map_err(|e| OpError::new(e.to_string()))?;
        Ok(Dish::Bytes(out))
    },
};

const BASE62_DESCRIPTION: &str = "Base62 is a notation for encoding arbitrary byte data using a restricted set of symbols that can be conveniently used by humans and processed by computers. The high number base results in shorter strings than with the decimal or hexadecimal system.";

/// The alphabet argument as Base62 options: 62 distinct code units,
/// plain positional notation, characters outside the alphabet skipped.
fn base62_options(op: &str, arg: &str) -> Result<Options, OpError> {
    let alph = js_alphabet(arg);
    let digits = utilify_base62::Alphabet::new(&unit_str(&alph)).map_err(|e| thrown(op, e))?;
    Ok(Options::new()
        .with_alphabet(digits)
        .with_leading_zeros(LeadingZeros::Integer)
        .with_invalid_symbols(InvalidSymbols::Skip)
        .with_max_input_len(usize::MAX))
}

pub static TO_BASE62: Operation = Operation {
    name: "To Base62",
    category: CATEGORY,
    description: BASE62_DESCRIPTION,
    info_url: Some("https://wikipedia.org/wiki/List_of_numeral_systems"),
    args: &[Arg::string("Alphabet", "0-9A-Za-z")],
    run: |input, args| {
        let data = input.bytes();
        if data.is_empty() {
            return Ok(Dish::Text(String::new()));
        }
        let options = base62_options("To Base62", args.str(0))?;
        Ok(Dish::Text(unit_text(&options.encode(&data))))
    },
};

pub static FROM_BASE62: Operation = Operation {
    name: "From Base62",
    category: CATEGORY,
    description: BASE62_DESCRIPTION,
    info_url: Some("https://wikipedia.org/wiki/List_of_numeral_systems"),
    args: &[Arg::string("Alphabet", "0-9A-Za-z")],
    run: |input, args| {
        let options = base62_options("From Base62", args.str(0))?;
        let text = unit_str(&utf16(&input.text()));
        options
            .decode(text)
            .map(Dish::Bytes)
            .map_err(|e| thrown("From Base62", e))
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

    #[test]
    fn base58() {
        assert_eq!(run(&TO_BASE58, "hello world", &[]), "StV1DL6CwTryKyV");
        assert_eq!(run(&TO_BASE58, "\0\0a", &[]), "112g");
        assert_eq!(
            run(&TO_BASE58, "a", &[s("abc")]),
            "Error: alphabet must be of length 58"
        );
        assert_eq!(run(&FROM_BASE58, "StV1DL6CwTryKyV", &[]), "hello world");
        // Leading zeros counted after removing non-alphabet chars.
        assert_eq!(run(&FROM_BASE58, "0112g", &[]), "\0\0a");
        assert_eq!(run(&FROM_BASE58, "1012g", &[]), "\0\0a");
        assert_eq!(
            run(&FROM_BASE58, "a", &[s("abc")]),
            "Alphabet must be of length 58"
        );
    }

    #[test]
    fn base62() {
        assert_eq!(run(&TO_BASE62, "\0", &[]), "0");
        assert_eq!(run(&TO_BASE62, "\0\0a", &[]), "1Z");
        assert_eq!(run(&TO_BASE62, "", &[]), "");
        assert_eq!(run(&FROM_BASE62, "1Z", &[]), "a");
        assert_eq!(run(&FROM_BASE62, "0", &[]), "\0");
        assert!(run(&TO_BASE62, "a", &[s("0-9")]).starts_with("To Base62 - "));
        assert!(run(&TO_BASE62, "a", &[s("aab")]).starts_with("To Base62 - "));
        // Characters outside the alphabet are ignored, `_` included.
        assert_eq!(run(&FROM_BASE62, "1_Z", &[]), "a");
        assert_eq!(run(&FROM_BASE62, "!!!", &[]), "");
        // Upper-case text is not re-read in lower case.
        let lower = s("0-9a-z!@#$%^&*()=[]{};:,<>?/|~_+");
        assert_eq!(run(&FROM_BASE62, "A", &[lower]), "");
    }
}
