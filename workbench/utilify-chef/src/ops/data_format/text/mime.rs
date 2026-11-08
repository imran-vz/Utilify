//! "MIME Decoding": the encoded-words of RFC 2047 (with RFC 2231 §5
//! language tags) in header fields, decoded by `utilify-rfc2047` in strict
//! mode, with the charsets of the IANA registry that `utilify-codepages`
//! has (US-ASCII is ASCII, not Latin-1).
//!
//! The input is read as header fields (RFC 5322 §2.2): a line that starts
//! with a field name and `:` is a field, whose body is decoded in the
//! context its name gives (`Context::for_field`: words in unstructured
//! text, in comments of structured fields, in the phrases of address lists;
//! none in Received, RFC 2047 §5). Lines without a field name are taken as
//! an unstructured field body (RFC 5322 §3.6.8), where a word must stand
//! between linear white space (§6.1), so `(=?…?=)` stays as it is there.
//! Linear white space between adjacent words is dropped (§6.2); a line
//! break not followed by SPACE or TAB is not folding white space, so it
//! ends the field. Bare LF line breaks are read as CRLF.
//!
//! Malformed words (RFC 2047 §2 syntax and 75-character limit, B or Q text,
//! §3 code-switching charsets ending outside ASCII), unknown charsets and
//! octets invalid in their charset are errors (the library's message).
//!
//! Presentation kept from CyberChef: folds and field breaks are not
//! removed, and every line break in the output is LF.

use crate::OpError;
use utilify_rfc2047::{Context, DecodeOptions};

/// The header fields of `input` (CRLF line breaks), with the line each
/// starts on.
fn fields(input: &[u8]) -> Vec<(usize, &[u8])> {
    let mut out = vec![];
    let (mut start, mut line, mut first_line) = (0, 1, 1);
    let mut i = 0;
    while i + 1 < input.len() {
        if input[i] == b'\r' && input[i + 1] == b'\n' {
            line += 1;
            if !matches!(input.get(i + 2), Some(b' ' | b'\t')) {
                out.push((first_line, &input[start..i]));
                start = i + 2;
                first_line = line;
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    out.push((first_line, &input[start..]));
    out
}

/// A field's name (RFC 5322 §3.6.8 `field-name`, printable ASCII but `:`)
/// and the index of its body, if the field has one.
fn field_name(field: &[u8]) -> Option<(&str, usize)> {
    let colon = field.iter().position(|&b| b == b':')?;
    let name = &field[..colon];
    if name.is_empty() || !name.iter().all(|&b| matches!(b, 0x21..=0x39 | 0x3B..=0x7E)) {
        return None;
    }
    Some((std::str::from_utf8(name).ok()?, colon + 1))
}

/// `input` with every bare LF made CRLF.
fn crlf(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len() + input.len() / 32);
    for (i, &b) in input.iter().enumerate() {
        if b == b'\n' && (i == 0 || input[i - 1] != b'\r') {
            out.push(b'\r');
        }
        out.push(b);
    }
    out
}

pub(super) fn decode_headers(input: &[u8]) -> Result<String, OpError> {
    let input = crlf(input);
    let fields = fields(&input);
    let several = fields.len() > 1;
    let mut out = String::with_capacity(input.len());
    for (k, &(line, field)) in fields.iter().enumerate() {
        if k > 0 {
            out.push('\n');
        }
        let (context, body) = match field_name(field) {
            Some((name, at)) => {
                out.push_str(name);
                out.push(':');
                (Context::for_field(name), &field[at..])
            }
            None => (Context::Unstructured, field),
        };
        let options = DecodeOptions::STRICT
            .with_context(context)
            .with_unfold(false);
        let decoded = utilify_rfc2047::decode_with(body, &options).map_err(|e| {
            OpError::new(if several {
                format!("{e}, in the field on line {line}")
            } else {
                e.to_string()
            })
        })?;
        out.push_str(&decoded.replace("\r\n", "\n"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(s: &str) -> Result<String, String> {
        decode_headers(s.as_bytes()).map_err(|e| e.0)
    }

    #[test]
    fn rfc_2047_examples() {
        // §8, with its folds.
        assert_eq!(
            run("From: =?US-ASCII?Q?Keith_Moore?= <moore@cs.utk.edu>\r\n\
                 CC: =?ISO-8859-1?Q?Andr=E9?= Pirard <PIRARD@vm1.ulg.ac.be>\r\n\
                 Subject: =?ISO-8859-1?B?SWYgeW91IGNhbiByZWFkIHRoaXMgeW8=?=\r\n \
                 =?ISO-8859-2?B?dSB1bmRlcnN0YW5kIHRoZSBleGFtcGxlLg==?=")
            .unwrap(),
            "From: Keith Moore <moore@cs.utk.edu>\n\
             CC: André Pirard <PIRARD@vm1.ulg.ac.be>\n\
             Subject: If you can read this you understand the example."
        );
        // Comments in a structured field; not in unstructured text.
        assert_eq!(
            run("Content-Type: text/plain (=?ISO-8859-1?Q?a?= =?ISO-8859-1?Q?b?=)").unwrap(),
            "Content-Type: text/plain (ab)"
        );
        assert_eq!(run("(=?ISO-8859-1?Q?a?=)").unwrap(), "(=?ISO-8859-1?Q?a?=)");
        // Addresses are never decoded.
        assert_eq!(
            run("To: =?UTF-8?Q?x?=@example.org").unwrap(),
            "To: =?UTF-8?Q?x?=@example.org"
        );
        assert_eq!(run("plain\nlines").unwrap(), "plain\nlines");
        assert_eq!(
            run("Subject: =?UTF-8?B?w6k=?=\n x").unwrap(),
            "Subject: é\n x"
        );
    }

    #[test]
    fn charsets_and_errors() {
        // RFC 2231 §5 language tag; KOI8-R from the registry.
        assert_eq!(run("=?US-ASCII*EN?Q?a?=").unwrap(), "a");
        assert_eq!(run("=?koi8-r?B?8dE=?=").unwrap(), "Яя");
        // US-ASCII is ASCII.
        assert!(run("=?US-ASCII?Q?=E9?=").is_err());
        assert!(run("=?utf-8?Q?a b?=").is_ok_and(|s| s == "=?utf-8?Q?a b?="));
        assert!(run("=?utf-8?b?w6k?=").is_err());
        assert!(run("=?x-unknown?Q?a?=").is_err());
        assert!(
            run("a\nb =?utf-8?Q?=FF?=")
                .unwrap_err()
                .ends_with("on line 2")
        );
    }
}
