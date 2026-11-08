//! Quoted-Printable (RFC 2045 §6.7) for "To Quoted Printable" and "From
//! Quoted Printable", both by `utilify-quoted-printable`.
//!
//! Encoding is the library's RFC 2045 encoder ([`EncodeOptions::RFC2045`]):
//! text mode, so CRLF or a bare LF is a hard line break (rule 4) and a bare
//! CR is `=0D`; only octets rule 2 doesn't allow literally are escaped
//! (`=`, controls, octets past 126), SPACE/TAB before a line break are
//! escaped (rule 3), and lines are filled to 76 characters with soft line
//! breaks (rule 5), never inside an escape.
//!
//! Decoding is the library's lenient decoder, the robust behaviour RFC 2045
//! §6.7 recommends in its NOTE on illegal substrings: lowercase hex digits
//! are accepted, an `=` that doesn't start a valid escape is kept as it is,
//! trailing SPACE/TAB on a line is transport padding and deleted (rule 3),
//! and overlong lines are decoded. Hard line breaks come out as CRLF, the
//! canonical form, or as LF when the input has no CRLF (a local newline
//! convention, which rule 4 allows), so text keeps its line endings.
//!
//! CyberChef reads each character of the input as a byte, so characters
//! past U+00FF are an error.
//!
//! [`EncodeOptions::RFC2045`]: utilify_quoted_printable::EncodeOptions::RFC2045

use utilify_quoted_printable::{DecodeOptions, LineEnding};

/// "To Quoted Printable".
pub(super) fn encode(data: &[u8]) -> String {
    utilify_quoted_printable::encode(data)
}

/// "From Quoted Printable".
pub(super) fn decode(input: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::with_capacity(input.len());
    for c in input.chars() {
        let Ok(b) = u8::try_from(u32::from(c)) else {
            return Err(format!(
                "Data is not a valid byteArray: U+{:04X} is past 0xFF",
                u32::from(c)
            ));
        };
        bytes.push(b);
    }
    let ending = if bytes.windows(2).any(|w| w == b"\r\n") {
        LineEnding::CrLf
    } else {
        LineEnding::Lf
    };
    utilify_quoted_printable::decode_with(&bytes, &DecodeOptions::LENIENT.line_ending(ending))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_per_rfc_2045() {
        assert_eq!(encode(b"hello world"), "hello world");
        // Only `=`, controls and 8-bit octets are escaped; `"`, `?` and `_`
        // are literal (rule 2).
        assert_eq!(encode(b"a\"?_=b"), "a\"?_=3Db");
        // Rule 3: only the white space right before a line break is escaped.
        assert_eq!(encode("é=\n x \t\n".as_bytes()), "=C3=A9=3D\r\n x =09\r\n");
        // A bare CR is not a line break in text.
        assert_eq!(encode(b"a\rb"), "a=0Db");
        // Rule 5: lines filled to 76 characters, the soft break's `=` included.
        let long = "a".repeat(80);
        assert_eq!(
            encode(long.as_bytes()),
            format!("{}=\r\n{}", "a".repeat(75), "a".repeat(5))
        );
    }

    #[test]
    fn decodes_robustly() {
        let bytes = |s: &str| decode(s).unwrap();
        assert_eq!(bytes("hello=20world=\r\n!="), b"hello world!");
        assert_eq!(bytes("=4=41=G1=3d"), b"=4A=G1=");
        assert_eq!(bytes("a=20\nb  \n"), b"a \nb\n");
        assert_eq!(bytes("a=20\r\nb\r\n"), b"a \r\nb\r\n");
        assert_eq!(bytes("caf\u{e9}=3d"), b"caf\xE9=");
        assert!(decode("Ā").is_err());
    }
}
