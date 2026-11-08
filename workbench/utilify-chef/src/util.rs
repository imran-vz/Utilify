//! Helpers shared across operations, ported from CyberChef's `Utils` and
//! `lib/` where they define behaviour recipes depend on.

use crate::OpError;

/// `A-Za-z0-9+/=` to its characters, as UTF-16 code units. A `\-` is a
/// literal dash. CyberChef's `Utils.expandAlphRange`, which walks the string
/// by code unit: a range ends at code units, so one between non-BMP
/// characters spans surrogates (`😀-😂` is the high surrogate of 😀 and
/// then 😂's low one, which join back into 😂).
pub fn expand_alph_range(s: &str) -> Vec<u16> {
    const DASH: u16 = b'-' as u16;
    const BACKSLASH: u16 = b'\\' as u16;
    let c: Vec<u16> = s.encode_utf16().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < c.len() {
        if i + 2 < c.len() && c[i + 1] == DASH && c[i] != BACKSLASH {
            out.extend(c[i]..=c[i + 2]);
            i += 3;
        } else if i + 2 < c.len() && c[i] == BACKSLASH && c[i + 1] == DASH {
            out.push(DASH);
            i += 2;
        } else {
            out.push(c[i]);
            i += 1;
        }
    }
    out
}

/// The text a delimiter option stands for. CyberChef's `Utils.charRep`.
pub fn char_rep(token: &str) -> Option<&'static str> {
    Some(match token {
        "Space" => " ",
        "Percent" => "%",
        "Comma" => ",",
        "Semi-colon" => ";",
        "Colon" => ":",
        "Tab" => "\t",
        "Line feed" => "\n",
        "CRLF" => "\r\n",
        "Forward slash" => "/",
        "Backslash" => "\\",
        "0x" => "0x",
        "\\x" => "\\x",
        "Nothing (separate chars)" | "None" => "",
        _ => return None,
    })
}

/// Split on a delimiter option the way CyberChef's `Utils.regexRep` patterns
/// do: Space and None split on runs of whitespace, `0x with comma` on an
/// optional comma before `0x`, the rest on their literal text.
pub fn split_delim<'a>(data: &'a str, token: &str) -> Vec<&'a str> {
    match token {
        "Space" | "None" => split_whitespace_runs(data),
        "0x with comma" => {
            let mut out = vec![];
            let mut rest = data;
            while let Some(p) = rest.find("0x") {
                let end = if p > 0 && rest.as_bytes()[p - 1] == b',' {
                    p - 1
                } else {
                    p
                };
                out.push(&rest[..end]);
                rest = &rest[p + 2..];
            }
            out.push(rest);
            out
        }
        other => match char_rep(other) {
            Some(d) if !d.is_empty() => data.split(d).collect(),
            _ => vec![data],
        },
    }
}

/// `str.split(/\s+/)`: keeps a leading and trailing empty piece.
pub fn split_whitespace_runs(data: &str) -> Vec<&str> {
    let mut out = vec![];
    let mut start = 0;
    let mut in_ws = false;
    for (i, c) in data.char_indices() {
        if js_is_space(c) {
            if !in_ws {
                out.push(&data[start..i]);
                in_ws = true;
            }
        } else if in_ws {
            start = i;
            in_ws = false;
        }
    }
    out.push(if in_ws { "" } else { &data[start..] });
    out
}

/// `data.replace(regexRep(token), "")`.
pub fn strip_delim(data: &str, token: &str) -> String {
    match token {
        "Space" | "None" => data.chars().filter(|&c| !js_is_space(c)).collect(),
        _ => split_delim(data, token).concat(),
    }
}

/// JavaScript's `\s` and what `String.prototype.trim` removes: its
/// WhiteSpace and LineTerminator characters. Unlike [`char::is_whitespace`],
/// U+FEFF is one and U+0085 is not.
pub fn js_is_space(c: char) -> bool {
    crate::ops::data_format::numeric::js::is_space(c)
}

/// JavaScript's `parseInt(s, radix)`: optional sign, then the longest run of
/// digits; `None` where it gives NaN.
pub fn parse_int(s: &str, radix: u32) -> Option<i64> {
    let s = s.trim_start_matches(js_is_space);
    let (neg, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let digits = if radix == 16 {
        digits
            .strip_prefix("0x")
            .or_else(|| digits.strip_prefix("0X"))
            .unwrap_or(digits)
    } else {
        digits
    };
    let run: String = digits.chars().take_while(|c| c.is_digit(radix)).collect();
    if run.is_empty() {
        return None;
    }
    let v = i64::from_str_radix(&run, radix).ok()?;
    Some(if neg { -v } else { v })
}

/// A parsed value stored in a byte array, as `Uint8Array` does it (mod 256).
/// NaN, which CyberChef can't store either, is an error.
pub fn js_byte(v: Option<i64>, what: &str) -> Result<u8, OpError> {
    v.map(|n| n.rem_euclid(256) as u8)
        .ok_or_else(|| OpError::new(format!("Invalid {what} in input")))
}

/// CyberChef's `fromHex`: split on the delimiter (`Auto`: anything not hex,
/// or `0x`), then read `byte_len` digits at a time.
pub fn from_hex(data: &str, delim: &str, byte_len: usize) -> Result<Vec<u8>, OpError> {
    if byte_len < 1 {
        return Err(OpError::new("Byte length must be a positive integer"));
    }
    let pieces: Vec<String> = match delim {
        "Auto" => data
            .replace("0x", " ")
            .replace("0X", " ")
            .split(|c: char| !c.is_ascii_hexdigit())
            .map(str::to_string)
            .collect(),
        "None" => vec![data.to_string()],
        d => split_delim(data, d)
            .into_iter()
            .map(str::to_string)
            .collect(),
    };
    let mut out = vec![];
    for piece in &pieces {
        let chars: Vec<char> = piece.chars().collect();
        for chunk in chars.chunks(byte_len) {
            let s: String = chunk.iter().collect();
            out.push(js_byte(parse_int(&s, 16), "hex")?);
        }
    }
    Ok(out)
}

/// CyberChef's `fromDecimal`.
pub fn from_decimal(data: &str, delim: &str) -> Result<Vec<u8>, OpError> {
    let pieces: Vec<&str> = if delim == "Auto" {
        data.split(|c: char| !(c.is_ascii_digit() || c == '-'))
            .collect()
    } else {
        split_delim(data, delim)
    };
    pieces
        .into_iter()
        .filter(|p| !p.is_empty())
        .map(|p| js_byte(parse_int(p, 10), "decimal"))
        .collect()
}

/// CyberChef's `fromBinary`.
pub fn from_binary(data: &str, delim: &str, byte_len: usize) -> Result<Vec<u8>, OpError> {
    if byte_len < 1 {
        return Err(OpError::new("Byte length must be a positive integer"));
    }
    let bits: Vec<char> = strip_delim(data, delim).chars().collect();
    bits.chunks(byte_len)
        .map(|c| js_byte(parse_int(&c.iter().collect::<String>(), 2), "binary"))
        .collect()
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard Base64, padded.
pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                out.push(B64[((n >> shift) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Standard or URL-safe Base64, padding optional, other characters ignored.
pub fn base64_decode_loose(s: &str) -> Option<Vec<u8>> {
    let mut bits = 0u32;
    let mut n = 0;
    let mut out = vec![];
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            _ => continue,
        };
        bits = (bits << 6) | u32::from(v);
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push((bits >> n) as u8);
        }
    }
    Some(out)
}

/// Text in a toggle argument's format as bytes. CyberChef's
/// `Utils.convertToByteArray`.
pub fn convert_to_bytes(s: &str, format: &str) -> Result<Vec<u8>, OpError> {
    match format.to_ascii_lowercase().as_str() {
        "binary" => from_binary(s, "Space", 8),
        "hex" => from_hex(s, "Auto", 2),
        "decimal" => from_decimal(s, "Auto"),
        "base64" => Ok(base64_decode_loose(s).unwrap_or_default()),
        "utf8" => Ok(s.as_bytes().to_vec()),
        _ => Ok(crate::dish::text_to_bytes(s)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_ranges() {
        let text = |s: &str| String::from_utf16_lossy(&expand_alph_range(s));
        assert_eq!(text("a-dX\\-Y"), "abcdX-Y");
        assert_eq!(expand_alph_range("A-Za-z0-9+/=").len(), 65);
        assert_eq!(text("é😀"), "é😀");
        // Per code unit, as CyberChef: the range runs from 😀's low
        // surrogate down to 😂's high one, so it is empty.
        assert_eq!(text("😀-😂"), "😂");
        assert_eq!(expand_alph_range("😀-😂"), [0xd83d, 0xde02]);
        // Up to the high surrogate, then the low one on its own.
        let wide = expand_alph_range("~-😀");
        assert_eq!(wide.len(), 0xd83d - 0x7e + 2);
        assert_eq!(wide[wide.len() - 2..], [0xd83d, 0xde00]);
    }

    #[test]
    fn splits_like_js_regexes() {
        assert_eq!(split_whitespace_runs(" a  b "), vec!["", "a", "b", ""]);
        // JavaScript's `\s`, not Rust's whitespace.
        assert_eq!(split_whitespace_runs("a\u{feff}b"), vec!["a", "b"]);
        assert_eq!(split_whitespace_runs("a\u{85}b"), vec!["a\u{85}b"]);
        assert_eq!(
            strip_delim("1\u{feff}0\u{2028}1\u{85}", "Space"),
            "101\u{85}"
        );
        assert_eq!(
            split_delim("0x41,0x42", "0x with comma"),
            vec!["", "41", "42"]
        );
        assert_eq!(split_delim("a;b", "Semi-colon"), vec!["a", "b"]);
    }

    #[test]
    fn parses_ints_like_js() {
        assert_eq!(parse_int("1z", 16), Some(1));
        assert_eq!(parse_int("zz", 16), None);
        assert_eq!(parse_int("-12", 10), Some(-12));
        assert_eq!(parse_int("\u{feff}\u{3000}7", 10), Some(7));
        assert_eq!(parse_int("\u{85}7", 10), None);
        assert_eq!(js_byte(Some(300), "x").unwrap(), 44);
    }

    #[test]
    fn codecs() {
        assert_eq!(
            from_hex("de ad 0xbe,ef", "Auto", 2).unwrap(),
            [0xde, 0xad, 0xbe, 0xef]
        );
        assert_eq!(from_decimal("72 105", "Auto").unwrap(), b"Hi");
        assert_eq!(from_binary("01001000 01101001", "Space", 8).unwrap(), b"Hi");
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
        assert_eq!(base64_decode_loose("aGVsbG8").unwrap(), b"hello");
        assert_eq!(convert_to_bytes("ÿ", "Latin1").unwrap(), [0xff]);
    }
}
