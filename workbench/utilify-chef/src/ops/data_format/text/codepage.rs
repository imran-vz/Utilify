//! The code pages of "Encode text", "Decode text", "Text Encoding Brute
//! Force": every page is `utilify-codepages`' published mapping (see its
//! README for each page's source), strict in both directions. A byte
//! sequence the page leaves undefined, or a character it has no code for, is
//! an error (the crate's message), not a substitute byte or character.
//!
//! - Page numbers are the registry numbers the crate follows. 1132 is IBM's
//!   EBCDIC Lao (CCSID 1132), and 895 is the page IBM registers under that
//!   number, Japanese 7-bit Latin (ISO 646-JP). CyberChef's option labels
//!   ("IBM EBCDIC Lao (1132/1133/1341)", "Kamenický (Czech) MS-DOS (895)")
//!   are kept so recipes load; the npm `codepage` package CyberChef uses
//!   had IBM's ASCII-based Lao page (1133) and the Kamenický set there.
//! - Stateful and multi-byte pages are the crate's full codecs: ISO-2022-JP
//!   (RFC 1468), ISO-2022-KR (RFC 1557), ISO-2022-CN (RFC 1922), HZ
//!   (RFC 1843), ISCII with script switching (IS 13194:1991), GB18030 with
//!   its four-byte codes (WHATWG Encoding §10.2).
//! - UTF-7 (65000) is RFC 2152 in both directions (the crate's encoder and
//!   decoder).
//! - UTF-8, UTF-16 and UTF-32 decode strictly: ill-formed sequences, a
//!   final odd byte of UTF-16 and values past U+10FFFF are errors
//!   (Unicode §3.9). A UTF-8 byte order mark at the start is a signature
//!   and is removed (Unicode §2.6, WHATWG Encoding "UTF-8 decode"); on the
//!   byte-order-specific UTF-16LE/BE and UTF-32LE/BE an initial U+FEFF is
//!   text, ZERO WIDTH NO-BREAK SPACE (Unicode §3.10 D97-D100, RFC 2781
//!   §3.3). US-ASCII (20127) is ASCII: bytes and characters past 0x7F are
//!   errors.
//!
//! Where the spec can't be followed: Mazovia (620) and Europa 3 (29001) have
//! no authoritative mapping and the crate doesn't have them, so they keep
//! the tables in [`OWN_TABLES`] (taken from the `codepage` package, the only
//! source we have; Europa 3's U+F8FF entries are that source's).

use utilify_codepages::{BomHandling, CodePage, Options};

/// Single-byte tables without an authoritative source: 256 characters in
/// byte order, U+FFFD where unmapped.
const OWN_TABLES: &[(u32, &str)] = &[
    (
        620,
        "\u{0}\u{1}\u{2}\u{3}\u{4}\u{5}\u{6}\u{7}\u{8}\u{9}\u{a}\u{b}\u{c}\u{d}\u{e}\u{f}\u{10}\u{11}\u{12}\u{13}\u{14}\u{15}\u{16}\u{17}\u{18}\u{19}\u{1a}\u{1b}\u{1c}\u{1d}\u{1e}\u{1f} !\u{22}#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\u{5c}]^_`abcdefghijklmnopqrstuvwxyz{|}~\u{7f}ÇüéâäàąçêëèïîćÄĄĘęłôöĆûùŚÖÜ¢Ł¥śƒŹŻóÓńŃźż¿⌐¬½¼¡«»░▒▓│┤╡╢╖╕╣║╗╝╜╛┐└┴┬├─┼╞╟╚╔╩╦╠═╬╧╨╤╥╙╘╒╓╫╪┘┌█▄▌▐▀αßΓπΣσµτΦΘΩδ∞φε∩≡±≥≤⌠⌡÷≈°∙·√ⁿ²■\u{a0}",
    ),
    (
        29001,
        "\u{f8ff}\u{f8ff}ΈΉΊΌΎ°◘○◙♂♀♪♬☼▶◀↕‼¶§£Ώ↑↓→←Ë↔▲▼ !\u{22}#$%&'()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[\u{5c}]^_`abcdefghijklmnopqrstuvwxyz{|}~\u{7f}ÇüéâäàåçêëèïîìÄÅÉæÆôöòûùΑÖÜøαØάΒáíóúñÑβΓγΔδΕεέΖζΗηή│ªÁÂÀΘθ║╗╝ΙΪ┐└º¡¿─΄ãÃ╚╔ιίϊ═ΐΚκΛÊλΜμÍΝν┘┌ΞξΟοόÓßÔΠõÕπΡρÚΣςσΤτΥΫυύϋΰΦφΧχΨ·ψΩωώ\u{f8ff}",
    ),
];

/// Decodes `bytes` from page `cp`.
pub(super) fn decode(cp: u32, bytes: &[u8]) -> Result<String, String> {
    if let Some(table) = own_table(cp) {
        let table: Vec<char> = table.chars().collect();
        return bytes
            .iter()
            .enumerate()
            .map(|(i, &b)| match table.get(usize::from(b)) {
                Some(&c) if c != '\u{FFFD}' => Ok(c),
                _ => Err(format!(
                    "byte 0x{b:02X} at {i} is unmapped in code page {cp}"
                )),
            })
            .collect();
    }
    let page = CodePage::new(cp).ok_or_else(|| unknown(cp))?;
    let options = if cp == 65001 {
        Options::STRICT.with_bom(BomHandling::Remove)
    } else {
        Options::STRICT
    };
    page.decode(bytes, &options)
        .map(std::borrow::Cow::into_owned)
        .map_err(|e| e.to_string())
}

/// Encodes `text` in page `cp`.
pub(super) fn encode(cp: u32, text: &str) -> Result<Vec<u8>, String> {
    if let Some(table) = own_table(cp) {
        let table: Vec<char> = table.chars().collect();
        return text
            .char_indices()
            .map(|(i, c)| {
                table
                    .iter()
                    .rposition(|&t| t == c && c != '\u{FFFD}')
                    .map(|b| b as u8)
                    .ok_or_else(|| format!("{c:?} at {i} has no code in code page {cp}"))
            })
            .collect();
    }
    let page = CodePage::new(cp).ok_or_else(|| unknown(cp))?;
    page.encode(text, &Options::STRICT)
        .map(std::borrow::Cow::into_owned)
        .map_err(|e| e.to_string())
}

fn own_table(cp: u32) -> Option<&'static str> {
    OWN_TABLES.iter().find(|t| t.0 == cp).map(|t| t.1)
}

fn unknown(cp: u32) -> String {
    utilify_codepages::Error::UnknownCodePage(cp).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_byte_pages() {
        assert_eq!(
            encode(500, "hello").unwrap(),
            [0x88, 0x85, 0x93, 0x93, 0x96]
        );
        assert_eq!(
            decode(500, &[0x88, 0x85, 0x93, 0x93, 0x96]).unwrap(),
            "hello"
        );
        // A NUL is a character like any other.
        assert_eq!(decode(437, &[0x00, 0x41, 0x42]).unwrap(), "\0AB");
        assert_eq!(decode(20273, &[0x00, 0x01]).unwrap(), "\0\u{1}");
        // Unmappable characters and undefined bytes are errors.
        assert!(encode(1252, "a€☣").is_err());
        assert!(decode(1252, &[0x81]).is_err());
        // 1132 is IBM's EBCDIC Lao; 895 is ISO 646-JP.
        assert_eq!(decode(1132, &[0xC1]).unwrap(), "A");
        assert_eq!(decode(895, &[0x5C, 0x7E]).unwrap(), "¥‾");
        assert!(decode(895, &[0x80]).is_err());
        // The two pages kept as our own tables.
        assert_eq!(decode(620, &[0x86, 0x41]).unwrap(), "ąA");
        assert_eq!(encode(620, "ą").unwrap(), [0x86]);
        assert_eq!(decode(29001, &[0x02]).unwrap(), "Έ");
        assert!(encode(29001, "€").is_err());
    }

    #[test]
    fn multi_byte_and_stateful_pages() {
        assert_eq!(decode(932, &[0x82, 0xA0]).unwrap(), "あ");
        assert_eq!(encode(932, "あ").unwrap(), [0x82, 0xA0]);
        assert!(decode(932, &[0x41, 0x81]).is_err());
        // GB18030 four-byte codes (WHATWG Encoding §10.2).
        assert_eq!(decode(54936, &[0x81, 0x30, 0x81, 0x30]).unwrap(), "\u{80}");
        // ISO-2022-JP (RFC 1468).
        assert_eq!(encode(50220, "あ").unwrap(), b"\x1b$B$\"\x1b(B");
        assert_eq!(decode(50220, b"\x1b$B$\"\x1b(B").unwrap(), "あ");
    }

    #[test]
    fn unicode_forms() {
        // RFC 2152 UTF-7: runs of characters outside the direct sets are
        // shifted together.
        assert_eq!(encode(65000, "Hi Mom -☺-!").unwrap(), b"Hi Mom -+Jjo--!");
        assert_eq!(encode(65000, "日本語").unwrap(), b"+ZeVnLIqe-");
        assert_eq!(decode(65000, b"+ZeVnLIqe-").unwrap(), "日本語");
        assert_eq!(decode(65000, b"a+-b").unwrap(), "a+b");
        assert_eq!(decode(65000, b"+2D0-+3gA-").unwrap(), "😀");
        // A U+FEFF in a run is a character.
        assert_eq!(decode(65000, b"+/v8-").unwrap(), "\u{FEFF}");
        assert_eq!(encode(20127, "abc").unwrap(), b"abc");
        assert!(encode(20127, "é").is_err());
        assert!(decode(20127, &[0xE9]).is_err());
        assert_eq!(encode(12001, "😀").unwrap(), [0, 1, 0xF6, 0]);
        assert_eq!(decode(12000, &[0, 0xF6, 1, 0]).unwrap(), "😀");
        assert!(decode(1200, b"h\0i\0!").is_err());
        assert_eq!(decode(65001, b"\xEF\xBB\xBFhi").unwrap(), "hi");
        assert_eq!(decode(1200, b"\xFF\xFEh\0").unwrap(), "\u{FEFF}h");
        assert!(decode(65001, b"\xC0\x80").is_err());
    }
}
