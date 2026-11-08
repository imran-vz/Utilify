//! Hexadecimal: bytes as hex digits, hexdumps, hex content and PEM.
//!
//! - **To Hex / From Hex** are `utilify-hexdump`'s [`hex`] strings: To Hex
//!   keeps CyberChef's output (delimiter between bytes, or before each for
//!   `0x`, `\x` and `%`; a newline every "Bytes per line"); From Hex is the
//!   crate's strict decoder, which reads exactly that grammar (whole bytes,
//!   the chosen delimiter, either case; "Auto" detects the delimiter from
//!   the first byte) and errors on anything else.
//! - **To Hexdump** is a [`Layout`]: `hexdump -C`'s line without the middle
//!   gap, squeezing or trailing newline (CyberChef's presentation), with
//!   [`LayoutLimits`] raised to the 65,536-byte maximum width.
//! - **From Hexdump** detects the dump's format and parses it strictly with
//!   the crate (there is no formal standard for hex dumps; each format is
//!   its tool's manual page). Supported:
//!   - `hexdump -C` / `hexdump -C -v` (hexdump(1), "Canonical hex+ASCII
//!     display"), including `*` lines, which stand for repeats of the line
//!     before up to the next line's offset, and the final length line;
//!   - `xxd` with any `-c`, `-g`, `-u`, and `-d` offsets (xxd(1));
//!   - BSD `od -A x|o|d -t x1` (od(1)), with `*` lines;
//!   - Wireshark's "Copy Bytes as Hex + ASCII Dump" and "…as Hex Dump"
//!     (Wireshark User's Guide §6.2, tables 6.2 to 6.4, and "The Packet
//!     Bytes Pane"): a 3- to 8-digit hex offset, sixteen single-spaced
//!     bytes, with or without an extra space after the eighth, and an
//!     optional column of printable ASCII with `.` for other bytes. The
//!     guide gives no exact spacing, so it is read from the text; an ASCII
//!     column with an extra space after the eighth character is not read;
//!   - `text2pcap(1)` input (DESCRIPTION): a hex offset, single-spaced
//!     bytes, text after the bytes ignored, an optional final length line;
//!   - 010 Editor's hex text with addresses (010 Editor manual, Hex Editor
//!     Options: `0000h: 48 65 6C ; Hel`), with or without the `;` column,
//!     whose characters are printable ASCII with `.` for other bytes;
//!   - To Hexdump's layout, any width, either case, either character
//!     column, with or without the final length line.
//!
//!   The detection is `utilify-hexdump`'s ([`parse_auto_with`]); the
//!   offset-prefixed dumps (Wireshark, `text2pcap`, 010 Editor) are tried
//!   after the tools' formats, so a text those read keeps their reading.
//!   The character column must match the bytes, offsets must follow on
//!   from the first one (which may be non-zero, as with `hexdump -s`), and
//!   only the last line may be short. Text in no supported format, or that
//!   reads differently in two of them, is an error.
//! - **To Hex Content / From Hex Content** are Snort's `content` syntax
//!   (Snort Users Manual 2.9.x, §3.5.1): `|..|` runs of hex bytes in text,
//!   `\` escapes. To Hex Content's choice of which bytes to write as hex is
//!   CyberChef's.
//! - **PEM to Hex / Hex to PEM** are RFC 7468 textual encodings: PEM to Hex
//!   reads with the lax grammar (§3), Hex to PEM writes the strict one.
//! - **Swap endianness** reverses whole words of bytes.

use utilify_hexdump::{
    CharColumn, Charset, Layout, LayoutLimits, OffsetFormat, ParseOptions, Squeeze, hex,
    parse_auto_with,
};

use super::CATEGORY;
use crate::dish::{bytes_to_text, text_to_bytes};
use crate::util::{base64_encode, char_rep};
use crate::{Arg, Args, Dish, OpError, Operation};

pub(super) fn ops() -> Vec<&'static Operation> {
    vec![
        &TO_HEX,
        &FROM_HEX,
        &TO_HEXDUMP,
        &FROM_HEXDUMP,
        &TO_HEX_CONTENT,
        &FROM_HEX_CONTENT,
        &PEM_TO_HEX,
        &HEX_TO_PEM,
        &SWAP_ENDIANNESS,
    ]
}

const TO_HEX_DELIMS: &[&str] = &[
    "Space",
    "Percent",
    "Comma",
    "Semi-colon",
    "Colon",
    "Line feed",
    "CRLF",
    "0x",
    "0x with comma",
    "\\x",
    "None",
];

const FROM_HEX_DELIMS: &[&str] = &[
    "Auto",
    "Space",
    "Percent",
    "Comma",
    "Semi-colon",
    "Colon",
    "Line feed",
    "CRLF",
    "0x",
    "0x with comma",
    "\\x",
    "None",
];

pub static TO_HEX: Operation = Operation {
    name: "To Hex",
    category: CATEGORY,
    description: "Converts the input string to hexadecimal bytes separated by the specified delimiter.\n\ne.g. The UTF-8 encoded string Γειά σου becomes ce 93 ce b5 ce b9 ce ac 20 cf 83 ce bf cf 85 0a",
    info_url: Some("https://wikipedia.org/wiki/Hexadecimal"),
    args: &[
        Arg::choice("Delimiter", TO_HEX_DELIMS),
        Arg::number("Bytes per line", 0.0),
    ],
    run: to_hex_op,
};

fn to_hex_op(input: Dish, args: &Args) -> Result<Dish, OpError> {
    let (delim, extra) = match args.str(0) {
        "0x with comma" => ("0x", ","),
        d => (
            char_rep(if d.is_empty() { "Space" } else { d }).unwrap_or(" "),
            "",
        ),
    };
    let line = args.num(1);
    let line = if line.is_finite() && line >= 1.0 {
        line as usize
    } else {
        0
    };
    Ok(Dish::Text(to_hex(&input.bytes(), delim, 2, extra, line)))
}

/// To Hex's output: `0x`, `\x` and `%` go before each byte, other
/// delimiters between bytes; a newline every `line_size` bytes.
pub fn to_hex(data: &[u8], delim: &str, padding: usize, extra: &str, line_size: usize) -> String {
    let prepend = matches!(delim, "0x" | "\\x" | "%");
    let mut out = String::with_capacity(data.len() * (padding + delim.len() + extra.len()));
    for (i, b) in data.iter().enumerate() {
        let hex = format!("{b:0padding$x}");
        if prepend {
            out.push_str(delim);
            out.push_str(&hex);
        } else {
            out.push_str(&hex);
            out.push_str(delim);
        }
        out.push_str(extra);
        if i + 1 != data.len() && line_size > 0 && (i + 1) % line_size == 0 {
            out.push('\n');
        }
    }
    let trunc = extra.len() + if prepend { 0 } else { delim.len() };
    out.truncate(out.len().saturating_sub(trunc));
    out
}

pub static FROM_HEX: Operation = Operation {
    name: "From Hex",
    category: CATEGORY,
    description: "Converts a hexadecimal byte string back into its raw value.\n\ne.g. ce 93 ce b5 ce b9 ce ac 20 cf 83 ce bf cf 85 0a becomes the UTF-8 encoded string Γειά σου",
    info_url: Some("https://wikipedia.org/wiki/Hexadecimal"),
    args: &[Arg::choice("Delimiter", FROM_HEX_DELIMS)],
    run: |input, args| {
        let delim = match args.str(0) {
            "" | "Auto" => None,
            d => Some(hex_delimiter(d)),
        };
        decode_hex(&input.text(), delim).map(Dish::Bytes)
    },
};

/// The `utilify-hexdump` delimiter for a Delimiter option.
fn hex_delimiter(name: &str) -> hex::Delimiter {
    match name {
        "Percent" => hex::Delimiter::Percent,
        "Comma" => hex::Delimiter::Comma,
        "Semi-colon" => hex::Delimiter::SemiColon,
        "Colon" => hex::Delimiter::Colon,
        "Line feed" => hex::Delimiter::LineFeed,
        "CRLF" => hex::Delimiter::CrLf,
        "0x" => hex::Delimiter::ZeroX,
        "0x with comma" => hex::Delimiter::ZeroXComma,
        "\\x" => hex::Delimiter::BackslashX,
        "None" => hex::Delimiter::Bare,
        _ => hex::Delimiter::Space,
    }
}

/// A plain hex string, strictly: whole bytes with the delimiter between
/// (or, for `0x`, `\x` and `%`, before) each, as To Hex writes them; `None`
/// detects the delimiter from the first byte.
fn decode_hex(text: &str, delim: Option<hex::Delimiter>) -> Result<Vec<u8>, OpError> {
    hex::decode(text, delim).map_err(|e| OpError::new(e.to_string()))
}

const HEXDUMP_MAX_WIDTH: f64 = 65536.0;

/// `utilify-hexdump`'s limits raised to CyberChef's widest line: 65,536
/// bytes take about 320 KiB (three hex characters and up to two Latin-1
/// bytes each).
const HEXDUMP_LIMITS: LayoutLimits = LayoutLimits::new()
    .with_max_bytes_per_line(65_536)
    .with_max_line_len(512 << 10);

pub static TO_HEXDUMP: Operation = Operation {
    name: "To Hexdump",
    category: CATEGORY,
    description: "Creates a hexdump of the input data, displaying both the hexadecimal values of each byte and an ASCII representation alongside.\n\nThe 'UNIX format' argument defines which subset of printable characters are displayed in the preview column.",
    info_url: Some("https://wikipedia.org/wiki/Hex_dump"),
    args: &[
        Arg::number_in("Width", 16.0, 1.0, HEXDUMP_MAX_WIDTH),
        Arg::flag("Upper case hex", false),
        Arg::flag("Include final length", false),
        Arg::flag("UNIX format", false),
    ],
    run: to_hexdump_op,
};

fn to_hexdump_op(input: Dish, args: &Args) -> Result<Dish, OpError> {
    let width = args.num(0);
    // The argument checks on the number, then the operation's own.
    if width.is_nan() {
        return Err(OpError::new("Width must be a number."));
    }
    if width < 1.0 {
        return Err(OpError::new("Width must be greater than or equal to 1."));
    }
    if width > HEXDUMP_MAX_WIDTH {
        return Err(OpError::new("Width must be less than or equal to 65536."));
    }
    if width.fract() != 0.0 {
        return Err(OpError::new("Width must be a positive integer"));
    }
    let (width, upper, final_len, unix) =
        (width as usize, args.flag(1), args.flag(2), args.flag(3));
    let data = input.bytes();
    let charset = if unix {
        Charset::Ascii
    } else {
        Charset::Latin1
    };
    let layout =
        to_hexdump_layout(width, upper, charset).with_final_offset(final_len.then_some(false));
    layout
        .format(&data)
        .map(Dish::Text)
        .map_err(|e| OpError::new(e.to_string()))
}

/// To Hexdump's line: offset, two spaces, each byte followed by a space,
/// padded to the full width, then ` |chars|`; no middle gap, no squeezing
/// and no newline after the last line.
fn to_hexdump_layout(width: usize, upper: bool, charset: Charset) -> Layout {
    Layout::hexdump_canonical()
        .with_limits(HEXDUMP_LIMITS)
        .with_offset(Some(OffsetFormat::hex(8).with_uppercase(upper)))
        .with_bytes_per_line(width)
        .with_gap(0, "")
        .with_uppercase(upper)
        .with_chars(Some(CharColumn::hexdump().with_charset(charset)))
        .with_squeeze(Squeeze::Off)
        .with_final_offset(None)
        .with_trailing_newline(false)
}

pub static FROM_HEXDUMP: Operation = Operation {
    name: "From Hexdump",
    category: CATEGORY,
    description: "Attempts to convert a hexdump back into raw data. This operation supports many different hexdump variations, but probably not all. Make sure you verify that the data it gives you is correct before continuing analysis.",
    info_url: Some("https://wikipedia.org/wiki/Hex_dump"),
    args: &[],
    run: |input, _| from_hexdump(&input.text()).map(Dish::Bytes),
};

/// Largest decoded length: `*` lines can otherwise ask for exabytes from a
/// few characters. 256 MiB, the same bound as Swap endianness' padding.
const HEXDUMP_MAX_LEN: usize = 1 << 28;

/// Reads a dump in one of the formats listed in the module docs.
///
/// Every format that parses the whole text is tried; when they disagree
/// the text is ambiguous and is an error rather than a guess.
fn from_hexdump(text: &str) -> Result<Vec<u8>, OpError> {
    // To Hexdump writes nothing for empty input.
    if text.trim().is_empty() {
        return Ok(vec![]);
    }
    let opts = ParseOptions::new()
        .any_start_offset(true)
        .max_len(HEXDUMP_MAX_LEN);
    let err = |e: utilify_hexdump::Error| OpError::new(e.to_string());
    let tools = parse_auto_with(text, &opts);
    let mut ours: Option<Result<Vec<u8>, utilify_hexdump::Error>> = None;
    if let Some(width) = to_hexdump_width(text) {
        // ASCII is the "UNIX format" column, Latin-1 the default; they
        // read the same bytes wherever both parse.
        for charset in [Charset::Latin1, Charset::Ascii] {
            let r = to_hexdump_layout(width, false, charset)
                .with_final_offset(Some(false))
                .parse_with(text, &opts)
                .map(|p| p.data);
            if ours.as_ref().is_none_or(|o| o.is_err()) {
                ours = Some(r);
            }
        }
    }
    match (tools, ours) {
        (Ok(a), Some(Ok(b))) if a.data != b => Err(OpError::new(
            utilify_hexdump::ErrorKind::AmbiguousFormat.to_string(),
        )),
        (Ok(p), _) => Ok(p.data),
        (Err(_), Some(Ok(b))) => Ok(b),
        // The text looks like To Hexdump's layout: its error points at the
        // offending line and column.
        (Err(e), Some(Err(o))) if e.kind() == utilify_hexdump::ErrorKind::UnrecognizedFormat => {
            Err(err(o))
        }
        (Err(e), _) if e.kind() == utilify_hexdump::ErrorKind::UnrecognizedFormat => {
            Err(OpError::new(format!(
                "{e}; supported: hexdump -C, xxd, od -t x1, Wireshark, text2pcap, 010 Editor and To Hexdump's layout"
            )))
        }
        (Err(e), _) => Err(err(e)),
    }
}

/// The width of a To Hexdump line: `offset  ` then `width` bytes of three
/// characters each, then ` |`. The hex area holds no `|`, so the first one
/// on the first line ends it. `hexdump -C`'s middle gap makes its hex area
/// one character longer than a multiple of three, so it never matches.
fn to_hexdump_width(text: &str) -> Option<usize> {
    let line = text.lines().find(|l| !l.trim().is_empty())?;
    let hex = line.find("  ").map(|i| &line[i + 2..])?;
    let len = hex.find(" |")?;
    let width = len / 3;
    (len % 3 == 0 && (1..=HEXDUMP_MAX_WIDTH as usize).contains(&width)).then_some(width)
}

const HEX_CONTENT_CONVERT: &[&str] = &[
    "Only special chars",
    "Only special chars including spaces",
    "All chars",
];

pub static TO_HEX_CONTENT: Operation = Operation {
    name: "To Hex Content",
    category: CATEGORY,
    description: "Converts special characters in a string to hexadecimal. This format is used by SNORT for representing hex within ASCII text.\n\ne.g. foo=bar becomes foo|3d|bar.",
    info_url: Some(
        "http://manual-snort-org.s3-website-us-east-1.amazonaws.com/node32.html#SECTION00451000000000000000",
    ),
    args: &[
        Arg::choice("Convert", HEX_CONTENT_CONVERT),
        Arg::flag("Print spaces between bytes", false),
    ],
    run: to_hex_content_op,
};

fn to_hex_content_op(input: Dish, args: &Args) -> Result<Dish, OpError> {
    let data = input.bytes();
    let spaces = args.flag(1);
    let convert = args.str(0);
    if convert == "All chars" {
        let delim = if spaces { " " } else { "" };
        return Ok(Dish::Text(format!("|{}|", to_hex(&data, delim, 2, "", 0))));
    }
    let convert_spaces = convert == "Only special chars including spaces";
    let mut out = String::with_capacity(data.len() * 2);
    let mut in_hex = false;
    for &b in data.iter() {
        let special = if b == b' ' {
            convert_spaces
        } else {
            !b.is_ascii_alphanumeric()
        };
        if special {
            if !in_hex {
                out.push('|');
                in_hex = true;
            } else if spaces {
                out.push(' ');
            }
            out.push_str(&format!("{b:02x}"));
        } else {
            if in_hex {
                out.push('|');
                in_hex = false;
            }
            out.push(char::from(b));
        }
    }
    if in_hex {
        out.push('|');
    }
    Ok(Dish::Text(out))
}

pub static FROM_HEX_CONTENT: Operation = Operation {
    name: "From Hex Content",
    category: CATEGORY,
    description: "Translates hexadecimal bytes in text back to raw bytes. This format is used by SNORT for representing hex within ASCII text.\n\ne.g. foo|3d|bar becomes foo=bar.",
    info_url: Some(
        "http://manual-snort-org.s3-website-us-east-1.amazonaws.com/node32.html#SECTION00451000000000000000",
    ),
    args: &[],
    run: |input, _| from_hex_content(&input.bytes()).map(Dish::Bytes),
};

/// Snort's content syntax (Snort Users Manual 2.9.x, §3.5.1 `content`):
/// text is its own bytes, `|` opens and closes a run of hex bytes
/// (whitespace allowed between bytes), and `\` takes the next character
/// literally (the manual requires `;`, `\` and `"` to be escaped; quoting
/// belongs to the rule, not the content, so bare `;` and `"` are read as
/// themselves). An unclosed `|`, a non-hex character or half a byte inside
/// one, and a trailing `\` are errors.
fn from_hex_content(data: &[u8]) -> Result<Vec<u8>, OpError> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        match data[i] {
            b'\\' => {
                let &c = data.get(i + 1).ok_or_else(|| {
                    OpError::new(format!("Escape at offset {i} has no character"))
                })?;
                out.push(c);
                i += 2;
            }
            b'|' => {
                let open = i;
                i += 1;
                let mut high: Option<u8> = None;
                loop {
                    let Some(&c) = data.get(i) else {
                        return Err(OpError::new(format!(
                            "Hex run opened at offset {open} is not closed with '|'"
                        )));
                    };
                    i += 1;
                    match (c, high) {
                        (b'|', None) => break,
                        (b' ' | b'\t' | b'\r' | b'\n', None) => {}
                        (_, _) if c.is_ascii_hexdigit() => {
                            let v = (c as char).to_digit(16).unwrap_or(0) as u8;
                            match high.take() {
                                Some(h) => out.push(h << 4 | v),
                                None => high = Some(v),
                            }
                        }
                        (b'|' | b' ' | b'\t' | b'\r' | b'\n', Some(_)) => {
                            return Err(OpError::new(format!(
                                "Half a byte at offset {} in hex run",
                                i - 2
                            )));
                        }
                        _ => {
                            return Err(OpError::new(format!(
                                "Invalid character {:?} at offset {} in hex run",
                                char::from(c),
                                i - 1
                            )));
                        }
                    }
                }
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    Ok(out)
}

pub static PEM_TO_HEX: Operation = Operation {
    name: "PEM to Hex",
    category: CATEGORY,
    description: "Converts PEM (Privacy Enhanced Mail) format to a hexadecimal DER (Distinguished Encoding Rules) string.",
    info_url: Some("https://wikipedia.org/wiki/Privacy-Enhanced_Mail#Format"),
    args: &[],
    run: |input, _| pem_to_hex(&input.text()).map(Dish::Text),
};

/// RFC 7468 §3 `label`: printable ASCII but `-`, with single spaces or
/// hyphens between them; possibly empty.
fn is_pem_label(label: &str) -> bool {
    let labelchar = |b: u8| matches!(b, 0x21..=0x2c | 0x2e..=0x7e);
    let b = label.as_bytes();
    b.is_empty()
        || (labelchar(b[0])
            && labelchar(b[b.len() - 1])
            && b.windows(2).all(|w| labelchar(w[0]) || labelchar(w[1]))
            && b.iter().all(|&c| labelchar(c) || c == b' ' || c == b'-'))
}

/// RFC 7468 §3 `W`: whitespace the lax parser skips in the Base64 text.
fn is_pem_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n' | '\x0b' | '\x0c')
}

/// Every textual encoding in `text` (RFC 7468 §2) as hex, one per line.
/// Text outside the encapsulation boundaries is ignored (§2: parsers "MUST
/// NOT malfunction" on it); inside, the lax grammar (§3 `laxbase64text`)
/// applies: whitespace anywhere, padding optional, anything else an error.
fn pem_to_hex(text: &str) -> Result<String, OpError> {
    use base64::Engine;
    use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
    const BEGIN: &str = "-----BEGIN ";
    const END: &str = "-----END ";
    const LAX: GeneralPurpose = GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
    );
    let mut out = vec![];
    let mut from = 0;
    while let Some(p) = text[from..].find(BEGIN).map(|p| from + p) {
        let start = p + BEGIN.len();
        let Some(len) = text[start..].find("-----") else {
            return Err(OpError::new("PEM header has no closing '-----'"));
        };
        let label = &text[start..start + len];
        if !is_pem_label(label) {
            return Err(OpError::new(format!("Invalid PEM label '{label}'")));
        }
        let body = start + len + 5;
        let footer = format!("{END}{label}-----");
        let Some(end) = text[body..].find(END).map(|e| body + e) else {
            return Err(OpError::new(format!("PEM footer '{footer}' not found")));
        };
        if !text[end..].starts_with(&footer) {
            return Err(OpError::new(format!(
                "PEM footer does not match: expected '{footer}'"
            )));
        }
        let b64: String = text[body..end]
            .chars()
            .filter(|&c| !is_pem_space(c))
            .collect();
        let der = LAX
            .decode(&b64)
            .map_err(|e| OpError::new(format!("Invalid Base64 in PEM '{label}': {e}")))?;
        out.push(hex::encode_bare(&der));
        from = end + footer.len();
    }
    Ok(out.join("\n"))
}

pub static HEX_TO_PEM: Operation = Operation {
    name: "Hex to PEM",
    category: CATEGORY,
    description: "Converts a hexadecimal DER (Distinguished Encoding Rules) string into PEM (Privacy Enhanced Mail) format.",
    info_url: Some("https://wikipedia.org/wiki/Privacy-Enhanced_Mail"),
    args: &[Arg::string("Header string", "CERTIFICATE")],
    run: |input, args| hex_to_pem(&input.text(), args.str(0)).map(Dish::Text),
};

/// RFC 7468's strict form (§3 `stricttextualmsg`): the DER as Base64 in
/// lines of exactly 64 characters but the last (§2, "generators MUST wrap
/// the base64-encoded lines so that each line consists of exactly 64
/// characters except for the final line"), padded, CRLF line ends. The
/// input is hex digits; JavaScript whitespace between them is ignored.
fn hex_to_pem(text: &str, label: &str) -> Result<String, OpError> {
    if !is_pem_label(label) {
        return Err(OpError::new(format!("Invalid PEM label '{label}'")));
    }
    let digits: String = text.chars().filter(|&c| !is_js_space(c)).collect();
    let der = decode_hex(&digits, Some(hex::Delimiter::Bare))?;
    let mut out = format!("-----BEGIN {label}-----\r\n");
    let b64 = base64_encode(&der);
    for line in b64.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(line).unwrap_or_default());
        out.push_str("\r\n");
    }
    out.push_str(&format!("-----END {label}-----\r\n"));
    Ok(out)
}

/// JavaScript's `\s`.
fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\x0b' | '\x0c' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

pub static SWAP_ENDIANNESS: Operation = Operation {
    name: "Swap endianness",
    category: CATEGORY,
    description: "Switches the data from big-endian to little-endian or vice-versa. Data can be read in as hexadecimal or raw bytes. It will be returned in the same format as it is entered.",
    info_url: Some("https://wikipedia.org/wiki/Endianness"),
    args: &[
        Arg::choice("Data format", &["Hex", "Raw"]),
        Arg::number("Word length (bytes)", 4.0),
        Arg::flag("Pad incomplete words", true),
    ],
    run: swap_endianness_op,
};

/// Padding beyond this many bytes per word would only exhaust memory.
const MAX_PADDED_WORD: f64 = (1 << 28) as f64;

fn swap_endianness_op(input: Dish, args: &Args) -> Result<Dish, OpError> {
    let (raw, word_len, pad) = (args.str(0) == "Raw", args.num(1), args.flag(2));
    if word_len.is_nan() {
        return Err(OpError::new("Word length (bytes) must be a number."));
    }
    if word_len <= 0.0 {
        return Err(OpError::new("Word length must be greater than 0"));
    }
    // A word is a whole number of bytes.
    if word_len.fract() != 0.0 {
        return Err(OpError::new("Word length must be a whole number of bytes"));
    }
    if pad && word_len > MAX_PADDED_WORD || word_len.is_infinite() {
        return Err(OpError::new("Word length is too large"));
    }
    let text = input.text();
    let data = if raw {
        text_to_bytes(&text)
    } else {
        decode_hex(&text, None)?
    };
    let word_len = word_len as usize;
    let mut out = Vec::with_capacity(data.len());
    for chunk in data.chunks(word_len) {
        if pad {
            out.resize(out.len() + word_len - chunk.len(), 0);
        }
        out.extend(chunk.iter().rev());
    }
    Ok(Dish::Text(if raw {
        bytes_to_text(&out).into_owned()
    } else {
        to_hex(&out, " ", 2, "", 0)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ArgValue;

    fn run_bytes(op: &Operation, input: &str) -> Vec<u8> {
        op.apply(Dish::Text(input.into()), &[])
            .unwrap()
            .into_bytes()
    }

    fn run(op: &Operation, input: &str, args: &[ArgValue]) -> String {
        op.apply(Dish::Text(input.into()), args)
            .unwrap()
            .into_text()
    }

    #[test]
    fn to_hex_delimiters() {
        let s = |v: &str| ArgValue::Str(v.into());
        assert_eq!(run(&TO_HEX, "abc", &[s("0x with comma")]), "0x61,0x62,0x63");
        assert_eq!(run(&TO_HEX, "abc", &[s("\\x")]), "\\x61\\x62\\x63");
        assert_eq!(
            run(&TO_HEX, "abcd", &[s("Space"), ArgValue::Num(2.)]),
            "61 62 \n63 64"
        );
        assert_eq!(run(&TO_HEX, "", &[]), "");
    }

    #[test]
    fn from_hex_round_trips() {
        let hex = run(&TO_HEX, "Γειά σου", &[]);
        let back = FROM_HEX.apply(Dish::Text(hex), &[]).unwrap();
        assert_eq!(back.text(), "Γειά σου");
        let back = |input: &str, d: &str| {
            FROM_HEX
                .apply(Dish::Text(input.into()), &[ArgValue::Str(d.into())])
                .map_or_else(|e| e.0, |d| d.into_text())
        };
        for d in TO_HEX_DELIMS {
            let hex = run(
                &TO_HEX,
                "abcd",
                &[ArgValue::Str((*d).into()), ArgValue::Num(3.)],
            );
            assert_eq!(back(&hex, d), "abcd", "{d}");
            assert_eq!(back(&hex, "Auto"), "abcd", "{d}");
        }
    }

    #[test]
    fn from_hex_is_strict() {
        let e = |input: &str, d: &str| err(&FROM_HEX, input, &[s(d)]);
        assert_eq!(
            e("zz", "Auto"),
            "line 1, column 1: expected a hex digit, found 'z'"
        );
        assert_eq!(
            e("48 6", "Space"),
            "line 1, column 4: odd number of hex digits"
        );
        assert!(!e("48,65", "Space").is_empty());
        assert!(!e("48 65x", "Auto").is_empty());
    }

    fn s(v: &str) -> ArgValue {
        ArgValue::Str(v.into())
    }

    fn n(v: f64) -> ArgValue {
        ArgValue::Num(v)
    }

    fn b(v: bool) -> ArgValue {
        ArgValue::Bool(v)
    }

    fn err(op: &Operation, input: &str, args: &[ArgValue]) -> String {
        op.apply(Dish::Text(input.into()), args).unwrap_err().0
    }

    #[test]
    fn to_hexdump_options_and_errors() {
        let args = [n(2.), b(true), b(true), b(false)];
        assert_eq!(
            run(&TO_HEXDUMP, "abc", &args),
            "00000000  61 62  |ab|\n00000002  63     |c|\n00000003"
        );
        assert_eq!(run(&TO_HEXDUMP, "", &args), "");
        let ctl = "\0\u{1f} ~\u{7f}\u{a0}\u{ad}\u{ff}";
        assert_eq!(
            run(&TO_HEXDUMP, ctl, &[]),
            format!(
                "00000000  00 1f 20 7e 7f a0 ad ff{} |.. ~.\u{a0}.ÿ|",
                " ".repeat(25)
            )
        );
        let unix = [n(8.), b(false), b(false), b(true)];
        assert_eq!(
            run(&TO_HEXDUMP, ctl, &unix),
            "00000000  00 1f 20 7e 7f a0 ad ff  |.. ~....|"
        );
        // Past utilify-hexdump's default 4,096 bytes per line.
        assert_eq!(
            run(
                &TO_HEXDUMP,
                "\u{ab}b",
                &[n(5000.), b(true), b(true), b(false)]
            ),
            format!(
                "00000000  AB 62{}|\u{ab}b|\n00000002",
                " ".repeat(3 * 4998 + 2)
            )
        );
        // CyberChef's widest line, then a second line.
        let wide = run(
            &TO_HEXDUMP,
            &"\u{ff}".repeat(65537),
            &[n(65536.), b(false), b(true), b(false)],
        );
        assert_eq!(
            wide,
            format!(
                "00000000  {}  |{}|\n00010000  ff{}|\u{ff}|\n00010001",
                "ff ".repeat(65535) + "ff",
                "\u{ff}".repeat(65536),
                " ".repeat(3 * 65535 + 2)
            )
        );
        assert_eq!(
            run(&TO_HEXDUMP, "\u{ab}", &[n(16.), b(true), b(true), b(false)]),
            format!("00000000  AB{}|\u{ab}|\n00000001", " ".repeat(3 * 15 + 2))
        );
        assert_eq!(
            err(&TO_HEXDUMP, "abc", &[n(0.)]),
            "Width must be greater than or equal to 1."
        );
        assert_eq!(
            err(&TO_HEXDUMP, "abc", &[n(65537.)]),
            "Width must be less than or equal to 65536."
        );
        assert_eq!(
            err(&TO_HEXDUMP, "abc", &[n(1.5)]),
            "Width must be a positive integer"
        );
    }

    #[test]
    fn from_hexdump_reads_the_tools_formats() {
        let data = b"FreeBSD: The power to serve\n";
        // hexdump(1) -C, with a `*` line standing for repeats of the line
        // before it, up to the next offset.
        let canonical = "\
00000000  46 72 65 65 42 53 44 3a  20 54 68 65 20 70 6f 77  |FreeBSD: The pow|
00000010  65 72 20 74 6f 20 73 65  72 76 65 0a              |er to serve.|
0000001c
";
        assert_eq!(run_bytes(&FROM_HEXDUMP, canonical), data);
        let zeros = "\
00000000  00 00 00 00 00 00 00 00  00 00 00 00 00 00 00 00  |................|
*
00000030  41                                                |A|
00000031
";
        let mut want = vec![0; 48];
        want.push(b'A');
        assert_eq!(run_bytes(&FROM_HEXDUMP, zeros), want);
        // xxd, with -c/-g/-u inferred, and BSD od -t x1.
        let xxd = "00000000: 4865 6c6c 6f2c 2077 6f72 6c64 0a         Hello, world.\n";
        assert_eq!(run_bytes(&FROM_HEXDUMP, xxd), b"Hello, world\n");
        let xxd = "00000000: 48656C6C 6F2C2077  Hello, w\n00000008: 6F726C64           orld\n";
        assert_eq!(run_bytes(&FROM_HEXDUMP, xxd), b"Hello, world");
        let od = "0000000    48  69  0a\n0000003\n";
        assert_eq!(run_bytes(&FROM_HEXDUMP, od), b"Hi\n");
        // A dump made with `hexdump -s`.
        let skip = "00000010  41 42                                             |AB|\n00000012\n";
        assert_eq!(run_bytes(&FROM_HEXDUMP, skip), b"AB");
        assert_eq!(run_bytes(&FROM_HEXDUMP, ""), b"");
    }

    /// The start of an Ethernet/IPv4 frame, as in the samples below.
    const PACKET: [u8; 20] = [
        0x00, 0x50, 0x56, 0xc0, 0x00, 0x08, 0x00, 0x0c, 0x29, 0x1a, 0x2b, 0x3c, 0x08, 0x00, 0x45,
        0x00, 0x00, 0x34, 0x12, 0xab,
    ];

    #[test]
    fn from_hexdump_reads_wireshark_copies() {
        // Copy Bytes as Hex + ASCII Dump (Wireshark User's Guide §6.2).
        let ascii = "\
0000   00 50 56 c0 00 08 00 0c 29 1a 2b 3c 08 00 45 00   .PV.....).+<..E.
0010   00 34 12 ab                                       .4..
";
        assert_eq!(run_bytes(&FROM_HEXDUMP, ascii), PACKET);
        // The packet bytes pane's extra space after the eighth byte, upper
        // case and CRLF line ends.
        let gap = "\
0000  00 50 56 C0 00 08 00 0C  29 1A 2B 3C 08 00 45 00   .PV.....).+<..E.\r
0010  00 34 12 AB                                        .4..\r
";
        assert_eq!(run_bytes(&FROM_HEXDUMP, gap), PACKET);
        // Copy Bytes as Hex Dump: no ASCII column.
        let hex = "\
0000   00 50 56 c0 00 08 00 0c 29 1a 2b 3c 08 00 45 00
0010   00 34 12 ab
";
        assert_eq!(run_bytes(&FROM_HEXDUMP, hex), PACKET);
    }

    #[test]
    fn from_hexdump_reads_text2pcap_input() {
        // text2pcap(1), DESCRIPTION: the example, without its preamble
        // line, with its final length line.
        let sample = "\
000000 00 0e b6 00 00 02 00 0e b6 00 00 01 08 00 45 00
000010 00 28 00 00 00 00 ff 01 37 d1 c0 00 02 01 c0 00
000020 02 02 08 00 a6 2f 00 01 00 01 48 65 6c 6c 6f 20
000030 57 6f 72 6c 64 21
000036
";
        let bytes = run_bytes(&FROM_HEXDUMP, sample);
        assert_eq!(bytes.len(), 0x36);
        assert!(bytes.ends_with(b"Hello World!"));
        assert_eq!(
            run_bytes(&FROM_HEXDUMP, sample.trim_end_matches("000036\n")),
            bytes
        );
        // "Any text on a line after the bytes is ignored."
        let notes = "000000 48 69 0a  greeting\n000003\n";
        assert_eq!(run_bytes(&FROM_HEXDUMP, notes), b"Hi\n");
    }

    #[test]
    fn from_hexdump_reads_010_editor_hex_text() {
        let text = "\
0000h: 48 65 6C 6C 6F 2C 20 30 31 30 20 45 64 69 74 6F ; Hello, 010 Edito
0010h: 72 21                                           ; r!
";
        assert_eq!(run_bytes(&FROM_HEXDUMP, text), b"Hello, 010 Editor!");
        // Without the `;` column, and with lower-case hex and a colon-less
        // address ("Use Lowercase Hex", "Show Colon After Addresses" off).
        let bare = "0000h 48 65 6c 6c 6f 2c 20 30 31 30 20 45 64 69 74 6f\n0010h 72 21\n";
        assert_eq!(run_bytes(&FROM_HEXDUMP, bare), b"Hello, 010 Editor!");
    }

    #[test]
    fn from_hexdump_reads_to_hexdump() {
        let text = "Γειά σου\0\u{ff}";
        for args in [
            vec![n(5.)],
            vec![n(16.), b(true), b(true), b(false)],
            vec![n(3.), b(false), b(false), b(true)],
            vec![n(1.), b(true), b(false), b(true)],
        ] {
            let dump = run(&TO_HEXDUMP, text, &args);
            let back = FROM_HEXDUMP.apply(Dish::Text(dump), &[]).unwrap();
            assert_eq!(back.text(), text, "{args:?}");
        }
    }

    #[test]
    fn from_hexdump_rejects_other_text() {
        let e = |t: &str| err(&FROM_HEXDUMP, t, &[]);
        assert_eq!(
            e("no hex here"),
            "not a recognised hex dump format; supported: hexdump -C, xxd, od -t x1, Wireshark, text2pcap, 010 Editor and To Hexdump's layout"
        );
        // A Wireshark or 010 Editor character column must match the bytes.
        assert!(!e("0000  41 42  AC\n").is_empty());
        assert!(!e("0000h: 41 42 ; AC\n").is_empty());
        // The character column must match the bytes.
        assert_eq!(
            e("00000000  41 42  |AC|"),
            "line 1, column 20: expected 'B', found 'C'"
        );
        // Offsets must follow the data.
        assert!(!e("00000000  41 42  |AB|\n00000003  43     |C|").is_empty());
        // A `*` must be followed by the offset it repeats up to.
        assert!(
            !e(&format!(
                "00000000  {}  |{}|\n*\n",
                "41 ".repeat(16).trim_end(),
                "A".repeat(16)
            ))
            .is_empty()
        );
    }

    #[test]
    fn to_hex_content_modes() {
        assert_eq!(
            run(&TO_HEX_CONTENT, "foo=bar baz/", &[]),
            "foo|3d|bar baz|2f|"
        );
        let spaces = [s("Only special chars including spaces"), b(true)];
        assert_eq!(
            run(&TO_HEX_CONTENT, "a  =\u{1}b c", &spaces),
            "a|20 20 3d 01|b|20|c"
        );
        assert_eq!(
            run(&TO_HEX_CONTENT, "a b", &[s("All chars"), b(true)]),
            "|61 20 62|"
        );
        assert_eq!(
            run(&TO_HEX_CONTENT, "a b", &[s("All chars"), b(false)]),
            "|612062|"
        );
        assert_eq!(run(&TO_HEX_CONTENT, "", &[]), "");
    }

    #[test]
    fn from_hex_content_is_snort_content() {
        // Snort manual §3.5.1's example.
        assert_eq!(
            run_bytes(&FROM_HEX_CONTENT, "|5c 00|P|00|I|00|P|00|E|00 5c|"),
            b"\\\0P\0I\0P\0E\0\\"
        );
        assert_eq!(
            run(&FROM_HEX_CONTENT, "foo|3d|bar|41 42|43", &[]),
            "foo=barAB43"
        );
        assert_eq!(run(&FROM_HEX_CONTENT, "a\\|b\\\\c\\;", &[]), "a|b\\c;");
        assert_eq!(run(&FROM_HEX_CONTENT, "||é€", &[]), "é€");
        let e = |t: &str| err(&FROM_HEX_CONTENT, t, &[]);
        assert_eq!(
            e("a|41"),
            "Hex run opened at offset 1 is not closed with '|'"
        );
        assert_eq!(e("|zz|"), "Invalid character 'z' at offset 1 in hex run");
        assert_eq!(e("|4 1|"), "Half a byte at offset 1 in hex run");
        assert_eq!(e("|414|"), "Half a byte at offset 3 in hex run");
        assert_eq!(e("ab\\"), "Escape at offset 2 has no character");
        // To Hex Content's output reads back.
        for mode in HEX_CONTENT_CONVERT {
            for spaces in [false, true] {
                let c = run(&TO_HEX_CONTENT, "a =\\|é", &[s(mode), b(spaces)]);
                assert_eq!(
                    run(&FROM_HEX_CONTENT, &c, &[]),
                    "a =\\|é",
                    "{mode} {spaces}"
                );
            }
        }
    }

    #[test]
    fn pem_to_hex_is_rfc7468() {
        let pem = "intro\n-----BEGIN ABC-----\nQQ==\n-----END ABC-----\ntext\n-----BEGIN X509 CRL-----\r\nQU\n JD\n-----END X509 CRL-----\n-----BEGIN -----\n-----END -----";
        assert_eq!(run(&PEM_TO_HEX, pem, &[]), "41\n414243\n");
        // Padding is optional in the lax grammar; one-character labels count.
        assert_eq!(
            run(&PEM_TO_HEX, "-----BEGIN X-----QQ-----END X-----", &[]),
            "41"
        );
        assert_eq!(run(&PEM_TO_HEX, "no PEM", &[]), "");
        let e = |t: &str| err(&PEM_TO_HEX, t, &[]);
        assert_eq!(
            e("-----BEGIN ABC-----QUJD"),
            "PEM footer '-----END ABC-----' not found"
        );
        assert_eq!(
            e("-----BEGIN ABC-----QUJD-----END ABD-----"),
            "PEM footer does not match: expected '-----END ABC-----'"
        );
        assert_eq!(
            e("-----BEGIN A  B-----QUJD-----END A  B-----"),
            "Invalid PEM label 'A  B'"
        );
        assert!(
            e("-----BEGIN A-----QQ=A=-----END A-----").starts_with("Invalid Base64 in PEM 'A'")
        );
        assert!(
            e("-----BEGIN A-----\nProc-Type: 4,ENCRYPTED\nQUJD\n-----END A-----")
                .starts_with("Invalid Base64")
        );
    }

    #[test]
    fn hex_to_pem_is_rfc7468_strict() {
        let hex = "abcdef".repeat(9);
        let want = "-----BEGIN PUBLIC KEY-----\r\nq83vq83vq83vq83vq83vq83vq83vq83vq83v\r\n-----END PUBLIC KEY-----\r\n";
        assert_eq!(run(&HEX_TO_PEM, &hex, &[s("PUBLIC KEY")]), want);
        let a = "A".repeat(64);
        assert_eq!(
            run(&HEX_TO_PEM, &"00".repeat(50), &[s("X")]),
            format!("-----BEGIN X-----\r\n{a}\r\nAAA=\r\n-----END X-----\r\n")
        );
        assert_eq!(
            run(&HEX_TO_PEM, &"00".repeat(48), &[s("X")]),
            format!("-----BEGIN X-----\r\n{a}\r\n-----END X-----\r\n")
        );
        assert_eq!(
            run(&HEX_TO_PEM, "30 82\n01 0a", &[]),
            "-----BEGIN CERTIFICATE-----\r\nMIIBCg==\r\n-----END CERTIFICATE-----\r\n"
        );
        assert_eq!(
            run(&HEX_TO_PEM, "", &[s("X")]),
            "-----BEGIN X-----\r\n-----END X-----\r\n"
        );
        // Round trip.
        let pem = run(&HEX_TO_PEM, &"0123456789abcdef".repeat(10), &[]);
        assert_eq!(run(&PEM_TO_HEX, &pem, &[]), "0123456789abcdef".repeat(10));
        assert!(!err(&HEX_TO_PEM, "30 82\n01g", &[]).is_empty());
        assert!(!err(&HEX_TO_PEM, "308", &[]).is_empty());
        assert_eq!(
            err(&HEX_TO_PEM, "30", &[s("A--B")]),
            "Invalid PEM label 'A--B'"
        );
    }

    #[test]
    fn swap_endianness_words() {
        let hex = "0102030405";
        assert_eq!(run(&SWAP_ENDIANNESS, hex, &[]), "04 03 02 01 00 00 00 05");
        assert_eq!(
            run(&SWAP_ENDIANNESS, hex, &[s("Hex"), n(2.), b(false)]),
            "02 01 04 03 05"
        );
        assert_eq!(
            run(&SWAP_ENDIANNESS, hex, &[s("Hex"), n(3.), b(false)]),
            "03 02 01 05 04"
        );
        assert_eq!(
            run(&SWAP_ENDIANNESS, "01 02 03 04", &[s("Hex"), n(2.), b(true)]),
            "02 01 04 03"
        );
        assert_eq!(
            run(&SWAP_ENDIANNESS, "abcdé", &[s("Raw"), n(2.), b(true)]),
            "badc\0é"
        );
        assert_eq!(
            run(&SWAP_ENDIANNESS, "abcdé€", &[s("Raw"), n(3.), b(false)]),
            "cba©Ãd¬\u{82}â"
        );
        assert_eq!(run(&SWAP_ENDIANNESS, "", &[]), "");
        assert_eq!(
            err(&SWAP_ENDIANNESS, hex, &[s("Hex"), n(0.)]),
            "Word length must be greater than 0"
        );
        assert_eq!(
            err(&SWAP_ENDIANNESS, hex, &[s("Hex"), n(1.5), b(true)]),
            "Word length must be a whole number of bytes"
        );
        assert!(!err(&SWAP_ENDIANNESS, "01 0g", &[]).is_empty());
    }
}
