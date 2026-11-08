//! Base-N encodings: Base32, Base45, Base58, Bech32, Base62, Base64 (with
//! its offsets view), Base92, Base85 and Modhex.
//!
//! CyberChef works on JavaScript strings, so alphabets, `indexOf` and
//! `slice` count UTF-16 code units. The helpers here reproduce that where it
//! changes results, and the byte-array check CyberChef runs on every
//! `byteArray` output (some decoders can produce values outside 0-255).

use super::CATEGORY;
use crate::util::expand_alph_range;
use crate::{OpError, Operation};

mod base32;
mod base58;
mod base64;
mod base85;
mod bech32;
mod modhex;

pub(super) fn ops() -> Vec<&'static Operation> {
    vec![
        &base32::TO_BASE32,
        &base32::FROM_BASE32,
        &base32::TO_BASE45,
        &base32::FROM_BASE45,
        &base58::TO_BASE58,
        &base58::FROM_BASE58,
        &bech32::TO_BECH32,
        &bech32::FROM_BECH32,
        &base58::TO_BASE62,
        &base58::FROM_BASE62,
        &base64::TO_BASE64,
        &base64::FROM_BASE64,
        &base64::SHOW_BASE64_OFFSETS,
        &base85::TO_BASE92,
        &base85::FROM_BASE92,
        &base85::TO_BASE85,
        &base85::FROM_BASE85,
        &modhex::TO_MODHEX,
        &modhex::FROM_MODHEX,
    ]
}

/// A string as JavaScript sees it: UTF-16 code units.
fn utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// Code units back to text; a lone surrogate becomes U+FFFD.
fn from_utf16(units: &[u16]) -> String {
    String::from_utf16_lossy(units)
}

/// `Utils.expandAlphRange(alphabet).join("")` as code units.
fn js_alphabet(alphabet: &str) -> Vec<u16> {
    expand_alph_range(alphabet)
}

/// `str.indexOf(c)` for a single code unit, `-1` when absent.
fn index_of(haystack: &[u16], c: u16) -> i32 {
    haystack
        .iter()
        .position(|&h| h == c)
        .map_or(-1, |p| p as i32)
}

/// Where `symbol` sits in a library's ASCII alphabet `table`; adapters map
/// that index into CyberChef's alphabet.
fn symbol_value(table: &str, symbol: u8) -> usize {
    table
        .bytes()
        .position(|t| t == symbol)
        .unwrap_or(usize::MAX)
}

/// Code units in CyberChef's alphabet to the symbols at the same index of a
/// library's ASCII alphabet `table`, by `indexOf` (a repeated symbol counts
/// at its first place). Units the alphabet lacks, or whose index is past
/// `table`, become `!`, which no library alphabet here holds, so the
/// library reports them.
fn standard_symbols(data: &[u16], alph: &[u16], table: &str) -> Vec<u8> {
    data.iter()
        .map(|&c| {
            usize::try_from(index_of(alph, c))
                .ok()
                .and_then(|i| table.as_bytes().get(i).copied())
                .unwrap_or(b'!')
        })
        .collect()
}

/// `str.indexOf(needle)` for a substring, `-1` when absent.
fn find_units(haystack: &[u16], needle: &[u16]) -> i32 {
    if needle.is_empty() {
        return 0;
    }
    haystack
        .windows(needle.len())
        .position(|w| w == needle)
        .map_or(-1, |p| p as i32)
}

/// No code unit repeats: CyberChef's `[].unique.call(alphabet)` check.
fn all_unique(units: &[u16]) -> bool {
    let mut seen = std::collections::HashSet::with_capacity(units.len());
    units.iter().all(|u| seen.insert(*u))
}

/// `data.replace(new RegExp("[^" + escaped(alphabet) + "]", "g"), "")`:
/// keep only the code units that appear in `keep`.
fn keep_only(data: &[u16], keep: &[u16]) -> Vec<u16> {
    let set: std::collections::HashSet<u16> = keep.iter().copied().collect();
    data.iter().copied().filter(|c| set.contains(c)).collect()
}

/// JavaScript's `\s` (and what `String.prototype.trim` removes).
fn js_is_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0b}' | '\u{0c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// `new TextDecoder().decode(bytes)` (fatal or not): a leading BOM is
/// dropped. Lossy UTF-8 when `lossy`, else Latin-1 when not valid UTF-8,
/// as CyberChef's `Utils.byteArrayToUtf8`.
fn decode_utf8(bytes: &[u8], lossy: bool) -> String {
    let body = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    match std::str::from_utf8(body) {
        Ok(s) => s.to_string(),
        Err(_) if lossy => String::from_utf8_lossy(body).into_owned(),
        // Latin-1 keeps every byte, BOM included.
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    }
}

/// The failure CyberChef reports when an operation throws something other
/// than an `OperationError` (a plain string or a library error): the
/// message prefixed with the operation's name.
fn thrown(op: &str, msg: impl std::fmt::Display) -> OpError {
    OpError::new(format!("{op} - {msg}"))
}

/// CyberChef's check on a `byteArray` result: every value a number in
/// 0-255. `None` stands for `NaN`, which JSON prints as `null`.
fn byte_array(values: Vec<Option<i64>>) -> Result<Vec<u8>, OpError> {
    if values
        .iter()
        .all(|v| v.is_some_and(|n| (0..=255).contains(&n)))
    {
        return Ok(values.into_iter().map(|v| v.unwrap_or(0) as u8).collect());
    }
    let json = format!(
        "[{}]",
        values
            .iter()
            .map(|v| v.map_or_else(|| "null".to_string(), |n| n.to_string()))
            .collect::<Vec<_>>()
            .join(",")
    );
    // `Utils.truncate(json, 25)`.
    let sample = if json.len() > 25 {
        format!("{}...", &json[..22])
    } else {
        json
    };
    Err(OpError::new(format!(
        "Data is not a valid byteArray: {sample}"
    )))
}

/// First of the private-use characters that stand for surrogate code units.
const SURROGATE_BASE: u32 = 0xF_0000;

/// One UTF-16 code unit as the `char` the codec crates see: itself, or for a
/// surrogate (not a scalar value) a supplementary private-use character,
/// which no code unit can be, so distinct units stay distinct.
pub(super) fn unit_char(u: u16) -> char {
    char::from_u32(u32::from(u))
        .or_else(|| char::from_u32(SURROGATE_BASE + u32::from(u) - 0xD800))
        .unwrap_or(char::REPLACEMENT_CHARACTER)
}

/// [`unit_char`] backwards, for the crates' encoded output.
pub(super) fn char_unit(c: char) -> u16 {
    let c = u32::from(c);
    match c.checked_sub(SURROGATE_BASE) {
        Some(s) if s < 0x800 => (0xD800 + s) as u16,
        _ => c as u16,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_arrays_are_checked_like_cyberchef() {
        assert_eq!(byte_array(vec![Some(1), Some(255)]).unwrap(), [1, 255]);
        assert_eq!(
            byte_array(vec![Some(-1), Some(-1), Some(240)])
                .unwrap_err()
                .0,
            "Data is not a valid byteArray: [-1,-1,240]"
        );
        let many = vec![Some(1), None, Some(222), Some(15), None, Some(21), Some(30)];
        assert_eq!(
            byte_array(many).unwrap_err().0,
            "Data is not a valid byteArray: [1,null,222,15,null,21..."
        );
    }

    #[test]
    fn js_string_helpers() {
        assert_eq!(find_units(&utf16("abcnull"), &utf16("null")), 3);
        assert_eq!(index_of(&utf16("abc"), b'z' as u16), -1);
        assert!(!all_unique(&utf16("aba")));
        assert_eq!(decode_utf8(&[0xef, 0xbb, 0xbf, b'a'], false), "a");
        assert_eq!(decode_utf8(&[0xe9], false), "é");
    }
}
