//! Modhex: YubiKey's hexadecimal with the digits `cbdefghijklnrtuv`.

use super::{CATEGORY, byte_array, js_is_space, utf16};
use crate::util::{char_rep, parse_int, split_delim};
use crate::{Arg, Dish, Operation};

const MODHEX: &[u8; 16] = b"cbdefghijklnrtuv";
const HEX: &[u8; 16] = b"0123456789abcdef";

const TO_MODHEX_DELIMS: &[&str] = &[
    "Space",
    "Percent",
    "Comma",
    "Semi-colon",
    "Colon",
    "Line feed",
    "CRLF",
    "None",
];

const FROM_MODHEX_DELIMS: &[&str] = &[
    "Auto",
    "Space",
    "Percent",
    "Comma",
    "Semi-colon",
    "Colon",
    "Line feed",
    "CRLF",
    "None",
];

pub static TO_MODHEX: Operation = Operation {
    name: "To Modhex",
    category: CATEGORY,
    description: "Converts the input string to modhex bytes separated by the specified delimiter.",
    info_url: Some("https://en.wikipedia.org/wiki/YubiKey#ModHex"),
    args: &[
        Arg::choice("Delimiter", TO_MODHEX_DELIMS),
        Arg::number("Bytes per line", 0.0),
    ],
    run: |input, args| {
        let delim = char_rep(if args.str(0).is_empty() {
            "Space"
        } else {
            args.str(0)
        })
        .unwrap_or(" ");
        // JavaScript's `(i + 1) % lineSize === 0`, so 0, NaN and fractions
        // behave as in CyberChef.
        let line = args.num(1);
        let data = input.bytes();
        let mut out = String::with_capacity(data.len() * (2 + delim.len()));
        for (i, &b) in data.iter().enumerate() {
            out.push(MODHEX[usize::from(b >> 4)] as char);
            out.push(MODHEX[usize::from(b & 15)] as char);
            out.push_str(delim);
            if i + 1 != data.len() && ((i + 1) as f64 % line) == 0.0 {
                out.push('\n');
            }
        }
        out.truncate(out.len() - if data.is_empty() { 0 } else { delim.len() });
        Ok(Dish::Text(out))
    },
};

pub static FROM_MODHEX: Operation = Operation {
    name: "From Modhex",
    category: CATEGORY,
    description: "Converts a modhex byte string back into its raw value.",
    info_url: Some("https://en.wikipedia.org/wiki/YubiKey#ModHex"),
    args: &[Arg::choice("Delimiter", FROM_MODHEX_DELIMS)],
    run: |input, args| {
        let data: String = input
            .text()
            .to_lowercase()
            .chars()
            .filter(|&c| !js_is_space(c))
            .collect();
        let pieces: Vec<&str> = match args.str(0) {
            "" | "Auto" => data
                .split(|c: char| !MODHEX.contains(&(c as u8)) || !c.is_ascii())
                .collect(),
            "None" => vec![&data],
            d => split_delim(&data, d),
        };
        // Each code unit to its hex digit; CyberChef writes "undefined" for
        // anything else, which then reads as junk hex.
        let mut hex = String::with_capacity(data.len());
        for piece in pieces {
            for u in utf16(piece) {
                match MODHEX.iter().position(|&m| u16::from(m) == u) {
                    Some(p) => hex.push(HEX[p] as char),
                    None => hex.push_str("undefined"),
                }
            }
        }
        // `fromHex(hex, "None", 2)`: two characters at a time, NaN for junk.
        let chars: Vec<char> = hex.chars().collect();
        let values = chars
            .chunks(2)
            .map(|c| parse_int(&c.iter().collect::<String>(), 16))
            .collect();
        Ok(Dish::Bytes(byte_array(values)?))
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

    // Expected values from CyberChef.
    #[test]
    fn to_modhex() {
        assert_eq!(
            run(&TO_MODHEX, "ab", &[s("Space"), ArgValue::Num(1.5)]),
            "hb hd"
        );
        assert_eq!(
            run(&TO_MODHEX, "abcd", &[s("Colon"), ArgValue::Num(2.0)]),
            "hb:hd:\nhe:hf"
        );
        assert_eq!(run(&TO_MODHEX, "", &[]), "");
    }

    #[test]
    fn from_modhex() {
        assert_eq!(run(&FROM_MODHEX, "CB DE", &[]), "\u{1}#");
        assert_eq!(
            run(&FROM_MODHEX, "cbx", &[s("None")]),
            "Data is not a valid byteArray: [1,null,222,15,null,13]"
        );
    }
}
