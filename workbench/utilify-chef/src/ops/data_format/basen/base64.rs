//! Base64 (RFC 4648) with CyberChef's alphabet presets, and the offsets
//! view that shows how a string encodes at each of the three positions in a
//! block.
//!
//! The codec is `data-encoding`, in the standard alphabet; the adapter maps
//! symbols to and from the alphabet argument by index (index 64 is the
//! padding character). The alphabet argument and its range syntax are
//! CyberChef's.
//!
//! From Base64 follows RFC 4648:
//! - Padding ends the data (§3.3, §4): a padding character may only be
//!   followed by more padding, so several padded encodings joined together
//!   are an error, not decoded one after another.
//! - Padded input must be a whole number of 4-character blocks with at most
//!   two padding characters (§4); unpadded input may end in a 2- or
//!   3-character block (§3.2 lets a referring specification omit padding)
//!   but not a single character, which carries no byte.
//! - Pad bits (§3.5, "decoders MAY chose to reject an encoding if the pad
//!   bits have not been set to zero"): rejected in "Strict mode", ignored
//!   otherwise.
//! - "Strict mode" also requires padding when the alphabet has a padding
//!   character (§3.2: implementations "MUST include appropriate pad
//!   characters"), with CyberChef's messages; a 64-character alphabet has
//!   no padding, so unpadded input is accepted.
//! - With "Remove non-alphabet chars" off, any other character is an error
//!   (§3.3).
//!
//! Show Base64 offsets keeps CyberChef's markup, colours and wording (a
//! presentation no specification defines), but the content follows RFC
//! 4648 §4, where character `i` encodes bits `6i..6i+6`:
//! - A character is static only if all its bits come from the input; the
//!   one character straddling each end of the input is variable (green),
//!   and padding is the alphabet's 65th symbol (red), whatever it is.
//!   CyberChef found the tail by searching for "=", so alphabets with
//!   another or no padding symbol, or with "=" as a data symbol, showed
//!   variable and padding characters as static.
//! - A tooltip shows exactly the input bytes lying wholly inside the static
//!   run (UTF-8 text, `\xNN` for bytes outside a valid sequence). CyberChef
//!   decoded the run with its variable neighbours and sliced the result, so
//!   tooltips included partly-determined bytes (offset 1 of "Hello" showed
//!   "\x08ello") or dropped whole ones (offset 0 showed "Hel", not "Hell").

use super::{
    CATEGORY, decode_utf8, from_utf16, index_of, js_alphabet, keep_only, standard_symbols,
    symbol_value, thrown, utf16,
};
use crate::{Arg, Args, Dish, OpError, Operation};
use data_encoding::{BASE64, BASE64_NOPAD, Encoding};
use std::sync::LazyLock;

/// CyberChef's `Base64.ALPHABET_OPTIONS`.
pub(super) const BASE64_ALPHABETS: &[(&str, &str)] = &[
    ("Standard (RFC 4648): A-Za-z0-9+/=", "A-Za-z0-9+/="),
    ("URL safe (RFC 4648 §5): A-Za-z0-9-_", "A-Za-z0-9-_"),
    ("Filename safe: A-Za-z0-9+-=", "A-Za-z0-9+\\-="),
    ("itoa64: ./0-9A-Za-z=", "./0-9A-Za-z="),
    ("XML: A-Za-z0-9_.", "A-Za-z0-9_."),
    ("y64: A-Za-z0-9._-", "A-Za-z0-9._-"),
    ("z64: 0-9a-zA-Z+/=", "0-9a-zA-Z+/="),
    ("Radix-64 (RFC 4880): 0-9A-Za-z+/=", "0-9A-Za-z+/="),
    ("Uuencoding: [space]-_", " -_"),
    ("Xxencoding: +-0-9A-Za-z", "+\\-0-9A-Za-z"),
    (
        "BinHex: !-,-0-689@A-NP-VX-Z[`a-fh-mp-r",
        "!-,-0-689@A-NP-VX-Z[`a-fh-mp-r",
    ),
    ("ROT13: N-ZA-Mn-za-m0-9+/=", "N-ZA-Mn-za-m0-9+/="),
    ("UNIX crypt: ./0-9A-Za-z", "./0-9A-Za-z"),
    (
        "Atom128: /128GhIoPQROSTeUbADfgHijKLM+n0pFWXY456xyzB7=39VaqrstJklmNuZvwcdEC",
        "/128GhIoPQROSTeUbADfgHijKLM+n0pFWXY456xyzB7=39VaqrstJklmNuZvwcdEC",
    ),
    (
        "Megan35: 3GHIJKLMNOPQRSTUb=cdefghijklmnopWXYZ/12+406789VaqrstuvwxyzABCDEF5",
        "3GHIJKLMNOPQRSTUb=cdefghijklmnopWXYZ/12+406789VaqrstuvwxyzABCDEF5",
    ),
    (
        "Zong22: ZKj9n+yf0wDVX1s/5YbdxSo=ILaUpPBCHg8uvNO4klm6iJGhQ7eFrWczAMEq3RTt2",
        "ZKj9n+yf0wDVX1s/5YbdxSo=ILaUpPBCHg8uvNO4klm6iJGhQ7eFrWczAMEq3RTt2",
    ),
    (
        "Hazz15: HNO4klm6ij9n+J2hyf0gzA8uvwDEq3X1Q7ZKeFrWcVTts/MRGYbdxSo=ILaUpPBC5",
        "HNO4klm6ij9n+J2hyf0gzA8uvwDEq3X1Q7ZKeFrWcVTts/MRGYbdxSo=ILaUpPBC5",
    ),
];

const DEFAULT_ALPHABET: &str = "A-Za-z0-9+/=";

pub static TO_BASE64: Operation = Operation {
    name: "To Base64",
    category: CATEGORY,
    description: "Base64 is a notation for encoding arbitrary byte data using a restricted set of symbols that can be conveniently used by humans and processed by computers.\n\nThis operation encodes raw data into an ASCII Base64 string.\n\ne.g. hello becomes aGVsbG8=",
    info_url: Some("https://wikipedia.org/wiki/Base64"),
    args: &[Arg::editable("Alphabet", BASE64_ALPHABETS)],
    run: |input, args| {
        Ok(Dish::Text(from_utf16(&to_base64(
            &input.bytes(),
            args.str(0),
        )?)))
    },
};

/// CyberChef's `toBase64`. The alphabet may hold 64 characters (no padding)
/// or 65 (the last is the padding character).
fn to_base64(data: &[u8], alphabet: &str) -> Result<Vec<u16>, OpError> {
    let alph = js_alphabet(alphabet);
    if alph.len() != 64 && alph.len() != 65 {
        return Err(OpError::new(format!(
            "Invalid Base64 alphabet length ({}): {}",
            alph.len(),
            from_utf16(&alph)
        )));
    }
    // Index 64 is the padding character, or nothing without one (`charAt`).
    Ok(BASE64
        .encode(data)
        .bytes()
        .filter_map(|s| alph.get(symbol_value(STANDARD, s)).copied())
        .collect())
}

pub static FROM_BASE64: Operation = Operation {
    name: "From Base64",
    category: CATEGORY,
    description: "Base64 is a notation for encoding arbitrary byte data using a restricted set of symbols that can be conveniently used by humans and processed by computers.\n\nThis operation decodes data from an ASCII Base64 string back into its raw format.\n\ne.g. aGVsbG8= becomes hello",
    info_url: Some("https://wikipedia.org/wiki/Base64"),
    args: &[
        Arg::editable("Alphabet", BASE64_ALPHABETS),
        Arg::flag("Remove non-alphabet chars", true),
        Arg::flag("Strict mode", false),
    ],
    run: |input, args| {
        let data = utf16(&input.text());
        Ok(Dish::Bytes(from_base64(
            &data,
            args.str(0),
            args.flag(1),
            args.flag(2),
            "From Base64",
        )?))
    },
};

/// RFC 4648 Base64 to bytes. Characters outside the alphabet are dropped
/// first when `remove`; otherwise they are an error. Strict mode requires
/// padding (when the alphabet has it) and zero pad bits, with CyberChef's
/// messages for its own checks.
fn from_base64(
    data: &[u16],
    alphabet: &str,
    remove: bool,
    strict: bool,
    op: &str,
) -> Result<Vec<u8>, OpError> {
    if data.is_empty() {
        return Ok(vec![]);
    }
    let alph = js_alphabet(if alphabet.is_empty() {
        DEFAULT_ALPHABET
    } else {
        alphabet
    });
    if alph.len() != 64 && alph.len() != 65 {
        return Err(OpError::new(format!(
            "Error: Base64 alphabet should be 64 characters long, or 65 with a padding character. Found {}: {}",
            alph.len(),
            from_utf16(&alph)
        )));
    }
    let data = if remove {
        keep_only(data, &alph)
    } else {
        data.to_vec()
    };

    let pad = alph.get(64).copied();
    if strict {
        if data.len() % 4 == 1 {
            return Err(OpError::new(format!(
                "Error: Invalid Base64 input length ({}). Cannot be 4n+1, even without padding chars.",
                data.len()
            )));
        }
        if let Some(pad) = pad {
            if let Some(pad_pos) = data.iter().position(|&c| c == pad)
                && (pad_pos + 2 < data.len() || data.last() != Some(&pad))
            {
                return Err(OpError::new(format!(
                    "Error: Base64 padding character ({}) not used in the correct place.",
                    from_utf16(&[pad])
                )));
            }
            // RFC 4648 §3.2: padding is required.
            if data.len() % 4 != 0 {
                return Err(OpError::new("Error: Base64 not padded to a multiple of 4."));
            }
        }
        if data.iter().any(|&c| index_of(&alph, c) < 0) {
            return Err(OpError::new(
                "Error: Base64 input contains non-alphabet char(s)",
            ));
        }
    }

    // RFC 4648 §3.3: padding ends the data; only padding may follow it.
    if let Some(p) = pad.and_then(|pad| data.iter().position(|&c| c == pad))
        && let Some(q) = data[p..].iter().position(|&c| Some(c) != pad)
    {
        return Err(thrown(op, format!("data after padding at {}", p + q)));
    }
    let std = standard_symbols(&data, &alph, STANDARD);
    let encoding = match (std.contains(&b'='), strict) {
        (true, true) => &BASE64,
        (true, false) => &*PADDED,
        (false, true) => &BASE64_NOPAD,
        (false, false) => &*UNPADDED,
    };
    encoding.decode(&std).map_err(|e| thrown(op, e))
}

/// The standard alphabet and padding, in which the library works.
const STANDARD: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=";

/// Standard Base64 decoders that ignore non-zero pad bits (RFC 4648 §3.5
/// leaves rejecting them to the decoder; strict mode does).
static PADDED: LazyLock<Encoding> = LazyLock::new(|| lenient(&BASE64));
static UNPADDED: LazyLock<Encoding> = LazyLock::new(|| lenient(&BASE64_NOPAD));

fn lenient(base: &Encoding) -> Encoding {
    let mut spec = base.specification();
    spec.check_trailing_bits = false;
    // A specification taken from a valid encoding stays valid.
    spec.encoding().unwrap_or_else(|_| base.clone())
}

pub static SHOW_BASE64_OFFSETS: Operation = Operation {
    name: "Show Base64 offsets",
    category: CATEGORY,
    description: "When a string is within a block of data and the whole block is Base64'd, the string itself could be represented in Base64 in three distinct ways depending on its offset within the block.\n\nThis operation shows all possible offsets for a given string so that each possible encoding can be considered.",
    info_url: Some("https://wikipedia.org/wiki/Base64#Output_padding"),
    args: &[
        Arg::binary("Alphabet", DEFAULT_ALPHABET),
        Arg::flag("Show variable chars and padding", true),
        Arg::choice("Input format", &["Raw", "Base64"]),
    ],
    run: show_offsets,
};

const TOOLTIP_SCRIPT: &str =
    "<script type='application/javascript'>$('[data-toggle=\"tooltip\"]').tooltip()</script>";

fn show_offsets(input: Dish, args: &Args) -> Result<Dish, OpError> {
    let (alphabet, show_variable) = (args.str(0), args.flag(1));
    let mut data = input.into_bytes();
    if args.str(2) == "Base64" {
        data = from_base64(
            &utf16(&decode_utf8(&data, false)),
            "",
            true,
            false,
            "Show Base64 offsets",
        )?;
    }
    let encode = |prefix: &[u8]| to_base64(&[prefix, &data].concat(), alphabet);
    let offsets = [encode(&[])?, encode(&[0])?, encode(&[0, 0])?];
    if data.is_empty() {
        return Err(OpError::new("Please enter a string."));
    }

    let [o0, o1, o2] = [0, 1, 2].map(|k| render_offset(&offsets[k], k, &data, show_variable));
    Ok(Dish::Html(if show_variable {
        format!(
            "Characters highlighted in <span class='hl5'>green</span> could change if the input is surrounded by more data.\
             \nCharacters highlighted in <span class='hl3'>red</span> are for padding purposes only.\
             \nUnhighlighted characters are <span data-toggle='tooltip' data-placement='top' title='Tooltip on left'>static</span>.\
             \nHover over the static sections to see what they decode to on their own.\n\
             \nOffset 0: {o0}\nOffset 1: {o1}\nOffset 2: {o2}{TOOLTIP_SCRIPT}"
        )
    } else {
        format!("{o0}\n{o1}\n{o2}")
    }))
}

/// One offset's line. At offset `k` the input follows `k` unknown bytes and
/// is followed by unknown bytes, so each output character falls in one of
/// four classes by the bits it encodes (RFC 4648 §4: character `i` encodes
/// bits `6i..6i+6` of the stream, the input occupies bits `8k..8(k+n)`):
/// - bits wholly before the input: red (they come from the stand-in zero
///   bytes, so they only pad the position);
/// - bits partly from the input and partly from a neighbour: green
///   (variable), the one character straddling each end of the input;
/// - bits wholly from the input: static, with a tooltip;
/// - padding characters (the alphabet's 65th symbol), red.
///
/// The tooltip shows exactly the input bytes whose eight bits all lie in
/// the static characters, so a byte shared with a green character is left
/// out.
fn render_offset(full: &[u16], k: usize, data: &[u8], show_variable: bool) -> String {
    let start_bit = 8 * k;
    let end_bit = 8 * (k + data.len());
    let red_lead = start_bit / 6;
    let static_start = start_bit.div_ceil(6);
    let static_end = (end_bit / 6).max(static_start);
    let data_chars = end_bit.div_ceil(6).min(full.len());
    let part =
        |a: usize, b: usize| &full[a.min(full.len())..b.clamp(a.min(full.len()), full.len())];
    let stat = part(static_start, static_end);
    if !show_variable {
        return esc(stat);
    }
    // Input bytes j with 8(k+j) >= 6*static_start and 8(k+j+1) <= 6*static_end.
    let first = (6 * static_start).saturating_sub(start_bit).div_ceil(8);
    let last = ((6 * static_end) / 8).saturating_sub(k).min(data.len());
    let tip = data.get(first..last).unwrap_or_default();
    let mut out = String::new();
    if k > 0 {
        out.push_str(&format!(
            "<span class='hl3'>{}</span><span class='hl5'>{}</span>",
            esc(part(0, red_lead)),
            esc(part(red_lead, static_start))
        ));
    }
    out.push_str(&format!(
        "<span data-toggle='tooltip' data-placement='top' title='{}'>{}</span>",
        esc(&utf16(&bytes_text(tip))),
        esc(stat)
    ));
    let (var, pad) = (part(static_end, data_chars), part(data_chars, full.len()));
    if !var.is_empty() || !pad.is_empty() {
        out.push_str(&format!(
            "<span class='hl5'>{}</span><span class='hl3'>{}</span>",
            esc(var),
            esc(pad)
        ));
    }
    out
}

/// Tooltip text for `bytes`: UTF-8 where the bytes form valid sequences,
/// and `\xNN` for each byte that does not (a sequence cut by the edge of
/// the static run, or non-UTF-8 input), so every byte is shown.
fn bytes_text(bytes: &[u8]) -> String {
    let mut out = String::new();
    for chunk in bytes.utf8_chunks() {
        out.push_str(chunk.valid());
        for b in chunk.invalid() {
            out.push_str(&format!("\\x{b:02x}"));
        }
    }
    out
}

/// CyberChef's `Utils.escapeHtml`, which (unlike [`crate::dish::escape_html`])
/// leaves `/` alone and maps NUL to U+E000.
fn esc(units: &[u16]) -> String {
    let s = from_utf16(units);
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            '`' => out.push_str("&#x60;"),
            '\0' => out.push('\u{e000}'),
            c => out.push(c),
        }
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
    fn to_base64_alphabets() {
        assert_eq!(run(&TO_BASE64, "hello", &[]), "aGVsbG8=");
        assert_eq!(run(&TO_BASE64, "hello", &[s("A-Za-z0-9-_")]), "aGVsbG8");
        assert_eq!(
            run(&TO_BASE64, "", &[s("abc")]),
            "Invalid Base64 alphabet length (3): abc"
        );
        assert_eq!(
            run(&TO_BASE64, "\u{ff}\u{fe}", &[s("./0-9A-Za-z=")]),
            "zzs="
        );
    }

    #[test]
    fn from_base64_modes() {
        let std = s(DEFAULT_ALPHABET);
        assert_eq!(run(&FROM_BASE64, "aGVs bG8=", &[]), "hello");
        assert_eq!(
            run(&FROM_BASE64, "aGVs!bG8", &[std.clone(), b(false)]),
            // CyberChef: "hel\u{b1}\u{bc}"; `data-encoding` rejects the "!".
            "From Base64 - invalid symbol at 4"
        );
        assert_eq!(
            run(&FROM_BASE64, "abc!", &[std.clone(), b(false), b(true)]),
            "Error: Base64 input contains non-alphabet char(s)"
        );
        assert_eq!(
            run(&FROM_BASE64, "aGVsb", &[std.clone(), b(true), b(true)]),
            "Error: Invalid Base64 input length (5). Cannot be 4n+1, even without padding chars."
        );
        assert_eq!(
            run(&FROM_BASE64, "aG=sbG8=", &[std.clone(), b(true), b(true)]),
            "Error: Base64 padding character (=) not used in the correct place."
        );
        assert_eq!(
            run(&FROM_BASE64, "aGVsbG=", &[std.clone(), b(true), b(true)]),
            "Error: Base64 not padded to a multiple of 4."
        );
        assert_eq!(
            run(&FROM_BASE64, "aGVsbG8", &[std.clone(), b(true), b(true)]),
            "Error: Base64 not padded to a multiple of 4."
        );
        // No padding character, so unpadded input is the only form.
        assert_eq!(
            run(
                &FROM_BASE64,
                "aGVsbG8",
                &[s("A-Za-z0-9+/"), b(true), b(true)]
            ),
            "hello"
        );
        // RFC 4648 §3.3: padding ends the data.
        assert_eq!(
            run(&FROM_BASE64, "aGk=aGk=", &[std.clone(), b(true), b(false)]),
            "From Base64 - data after padding at 4"
        );
        // RFC 4648 §3.5: non-zero pad bits are rejected in strict mode only.
        assert_eq!(
            run(&FROM_BASE64, "aGVsbG9=", std::slice::from_ref(&std)),
            "hello"
        );
        assert!(
            run(&FROM_BASE64, "aGVsbG9=", &[std.clone(), b(true), b(true)])
                .starts_with("From Base64 - ")
        );
        assert_eq!(
            run(&FROM_BASE64, "x", &[s("ab")]),
            "Error: Base64 alphabet should be 64 characters long, or 65 with a padding character. Found 2: ab"
        );
        assert_eq!(run(&FROM_BASE64, "", &[s("ab")]), "");
    }

    // RFC 4648 §10.
    #[test]
    fn rfc4648_vectors() {
        let std = [s(DEFAULT_ALPHABET), b(true), b(true)];
        for (plain, enc) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(run(&TO_BASE64, plain, &[]), enc);
            assert_eq!(run(&FROM_BASE64, enc, &std), plain);
        }
    }

    const LEGEND: &str = "Characters highlighted in <span class='hl5'>green</span> could change if the input is surrounded by more data.\nCharacters highlighted in <span class='hl3'>red</span> are for padding purposes only.\nUnhighlighted characters are <span data-toggle='tooltip' data-placement='top' title='Tooltip on left'>static</span>.\nHover over the static sections to see what they decode to on their own.\n\n";

    fn offsets(input: &[u8], args: &[ArgValue]) -> String {
        match SHOW_BASE64_OFFSETS.apply(Dish::Bytes(input.to_vec()), args) {
            Ok(d) => d.display().into_owned(),
            Err(e) => e.0,
        }
    }

    fn tip(title: &str, body: &str) -> String {
        format!("<span data-toggle='tooltip' data-placement='top' title='{title}'>{body}</span>")
    }

    #[test]
    fn offsets_markup() {
        // "Hello" is 48 65 6c 6c 6f. Offset 0: "SGVsbG" is bits 0..36, so
        // bytes 0..4 ("Hell"); "8" mixes the "o" with what follows.
        // Offset 1: "E" mixes the unknown byte with "H", so "hlbGxv"
        // (bits 12..48) holds "ello" whole. Offset 2: "B" mixes, "IZWxsb"
        // (bits 18..54) holds "ell" whole, "w" mixes "o" with what follows.
        assert_eq!(
            run(&SHOW_BASE64_OFFSETS, "Hello", &[]),
            format!(
                "{LEGEND}Offset 0: {}<span class='hl5'>8</span><span class='hl3'>=</span>\
                 \nOffset 1: <span class='hl3'>A</span><span class='hl5'>E</span>{}\
                 \nOffset 2: <span class='hl3'>AA</span><span class='hl5'>B</span>{}<span class='hl5'>w</span><span class='hl3'>==</span>{TOOLTIP_SCRIPT}",
                tip("Hell", "SGVsbG"),
                tip("ello", "hlbGxv"),
                tip("ell", "IZWxsb"),
            )
        );
        assert_eq!(run(&SHOW_BASE64_OFFSETS, "", &[]), "Please enter a string.");
        assert_eq!(
            run(
                &SHOW_BASE64_OFFSETS,
                "aGVsbG8=",
                &[s(DEFAULT_ALPHABET), b(false), s("Base64")]
            ),
            "aGVsbG\nhlbGxv\noZWxsb"
        );
    }

    /// The characters of `data` at offset `k` that no choice of the
    /// neighbouring bytes changes, found by encoding with different
    /// neighbours: `(first index, the characters)`.
    fn invariant_run(data: &[u8], k: usize) -> (usize, String) {
        let encodings: Vec<Vec<u8>> = [0x00u8, 0xff, 0x5a, 0xa5]
            .iter()
            .flat_map(|&p| {
                [
                    vec![],
                    vec![0x00],
                    vec![0xff],
                    vec![0xff, 0xff],
                    vec![0x00, 0x00, 0x00],
                ]
                .into_iter()
                .map(move |suffix| {
                    BASE64
                        .encode(&[vec![p; k], data.to_vec(), suffix].concat())
                        .into_bytes()
                })
            })
            .collect();
        let shortest = encodings.iter().map(Vec::len).min().unwrap_or(0);
        let same: Vec<usize> = (0..shortest)
            .filter(|&i| encodings.iter().all(|e| e[i] == encodings[0][i]))
            .collect();
        let first = same.first().copied().unwrap_or(0);
        // The invariant characters form one run.
        assert!(same.windows(2).all(|w| w[1] == w[0] + 1), "{data:?} at {k}");
        let run = same.iter().map(|&i| encodings[0][i] as char).collect();
        (first, run)
    }

    // RFC 4648 §4 by brute force, lengths 1-7 with non-ASCII bytes: the
    // static run is exactly the characters the neighbours cannot change,
    // the green characters are the ones they can, and the tooltip is the
    // input bytes lying wholly inside the static run.
    #[test]
    fn offsets_follow_rfc4648() {
        let sample = [0xc3u8, 0xa9, 0x00, 0xff, b'<', 0x80, b'z'];
        for n in 0..=7 {
            let data = &sample[..n];
            let html = offsets(data, &[]);
            if n == 0 {
                assert_eq!(html, "Please enter a string.");
                continue;
            }
            let plain = offsets(data, &[s(DEFAULT_ALPHABET), b(false)]);
            let lines: Vec<&str> = plain.split('\n').collect();
            for k in 0..3 {
                let full = BASE64.encode(&[vec![0; k], data.to_vec()].concat());
                let (first, stat) = invariant_run(data, k);
                assert_eq!(lines[k], esc(&utf16(&stat)), "{data:?} at {k}");
                // An empty run sits right after the leading characters.
                let start = if stat.is_empty() {
                    k + usize::from(k > 0)
                } else {
                    first
                };
                let end = start + stat.len();
                let data_chars = (8 * (k + n)).div_ceil(6);
                // Bytes wholly inside the run: drop the first byte when an
                // unknown byte precedes it, and the last when the input does
                // not end on a 3-byte group.
                let lo = usize::from(k > 0);
                let hi = n - usize::from((k + n) % 3 != 0);
                let tip_bytes = data.get(lo..hi).unwrap_or_default();
                let mut line = String::new();
                if k > 0 {
                    line += &format!(
                        "<span class='hl3'>{}</span><span class='hl5'>{}</span>",
                        &full[..k],
                        &full[k..start]
                    );
                }
                line += &tip(&esc(&utf16(&bytes_text(tip_bytes))), &esc(&utf16(&stat)));
                if end < full.len() {
                    line += &format!(
                        "<span class='hl5'>{}</span><span class='hl3'>{}</span>",
                        &full[end..data_chars],
                        &full[data_chars..]
                    );
                }
                assert!(
                    html.contains(&format!("Offset {k}: {line}")),
                    "{data:?} at {k}: {html}"
                );
            }
        }
    }

    #[test]
    fn offsets_tooltips() {
        let tips = |data: &[u8]| {
            let html = offsets(data, &[]);
            html.split("title='")
                .skip(2)
                .map(|t| t[..t.find('\'').unwrap_or(0)].to_string())
                .collect::<Vec<_>>()
        };
        // One byte is never whole in a static run.
        assert_eq!(tips(b"a"), ["", "", ""]);
        assert_eq!(tips(b"ab"), ["a", "b", ""]);
        assert_eq!(tips(b"abc"), ["abc", "b", "b"]);
        // UTF-8 cut at the edge of the run shows the stray bytes.
        assert_eq!(tips("é".as_bytes()), ["\\xc3", "\\xa9", ""]);
        assert_eq!(tips("aé".as_bytes()), ["aé", "\\xc3", "\\xc3"]);
        assert_eq!(
            tips(&[0xff, 0xfe, 0xfd, 0xfc]),
            ["\\xff\\xfe\\xfd", "\\xfe\\xfd", "\\xfe\\xfd\\xfc"]
        );
    }

    // Classification by bits, not by searching for "=": a 64-symbol
    // alphabet has no padding, and Atom128 uses "=" as a data symbol.
    #[test]
    fn offsets_any_alphabet() {
        assert_eq!(
            offsets(b"Hello", &[s("A-Za-z0-9-_")]),
            format!(
                "{LEGEND}Offset 0: {}<span class='hl5'>8</span><span class='hl3'></span>\
                 \nOffset 1: <span class='hl3'>A</span><span class='hl5'>E</span>{}\
                 \nOffset 2: <span class='hl3'>AA</span><span class='hl5'>B</span>{}<span class='hl5'>w</span><span class='hl3'></span>{TOOLTIP_SCRIPT}",
                tip("Hell", "SGVsbG"),
                tip("ello", "hlbGxv"),
                tip("ell", "IZWxsb"),
            )
        );
        let atom = "/128GhIoPQROSTeUbADfgHijKLM+n0pFWXY456xyzB7=39VaqrstJklmNuZvwcdEC";
        let plain = offsets(b"Hello", &[s(atom), b(false)]);
        let lens: Vec<usize> = plain.split('\n').map(str::len).collect();
        assert_eq!(lens, [6, 6, 6]);
        // The vector alphabet whose padding symbol is "6".
        assert_eq!(
            offsets(
                &[0x00, 0x10, 0x83, 0x10, 0x51, 0x87],
                &[
                    s("<script>ale(1)/.ABCDEFGHIJKLMNOPQRSTUVWXYZbdfghjkmnoquvwxyz023456"),
                    b(false)
                ]
            ),
            "&lt;script&gt;\n&lt;AQmsBR\n&lt;ia1AEI"
        );
    }
}
