//! "To Punycode" and "From Punycode".
//!
//! Without "Internationalised domain name" the input is one Punycode string,
//! encoded and decoded per RFC 3492 by `idna`'s punycode module. Decoding
//! keeps the case of basic code points (mixed-case annotation, §5), and a
//! non-basic code point before the last delimiter is an error (§6.2).
//!
//! With it, the input is a domain name, or an email address whose domain
//! (after the last `@`; the local part is left alone, RFC 6531 §3.3) is
//! converted: UTS #46 ToASCII / ToUnicode (`idna::domain_to_ascii` and
//! `idna::domain_to_unicode`, nontransitional processing, no STD3 or DNS
//! length checks, as the WHATWG URL Standard's host parsing uses it). That
//! maps the label separators U+3002, U+FF0E and U+FF61 to `.`, case-folds
//! and NFC-normalises labels, recognises the ACE prefix `xn--` in any case
//! (RFC 5890 §2.3.2.1), and rejects labels that aren't valid IDNA (UTS #46
//! §4.1). CyberChef's punycode.js instead encodes labels as they are,
//! lowercases `xn--` labels before decoding them and drops anything after a
//! second `@`.
//!
//! Errors are "Invalid input" (`idna` doesn't say more), the RFC 3492 §6.2
//! non-basic code point error, or, for encoding past `idna`'s 32-bit limit,
//! "Overflow".

const OVERFLOW: &str = "Overflow: input needs wider integers to process";
const NOT_BASIC: &str = "Illegal input >= 0x80 (not a basic code point)";
const INVALID: &str = "Invalid input";

/// Punycode to text.
pub(super) fn decode(input: &str) -> Result<String, String> {
    let basic = input.rfind('-').unwrap_or(0);
    if !input[..basic].is_ascii() {
        return Err(NOT_BASIC.into());
    }
    idna::punycode::decode_to_string(input).ok_or_else(|| INVALID.into())
}

/// Text to Punycode; `idna` only fails on overflow.
pub(super) fn encode(input: &str) -> Result<String, String> {
    idna::punycode::encode_str(input).ok_or_else(|| OVERFLOW.into())
}

/// Applies `domain` to the domain of an email address (after its last
/// `@`), or to the whole input.
fn map_domain(
    input: &str,
    domain: impl Fn(&str) -> Result<String, String>,
) -> Result<String, String> {
    match input.rsplit_once('@') {
        Some((local, host)) => Ok(format!("{local}@{}", domain(host)?)),
        None => domain(input),
    }
}

pub(super) fn to_ascii(input: &str) -> Result<String, String> {
    map_domain(input, |d| {
        idna::domain_to_ascii(d).map_err(|_| INVALID.to_string())
    })
}

pub(super) fn to_unicode(input: &str) -> Result<String, String> {
    map_domain(input, |d| match idna::domain_to_unicode(d) {
        (text, Ok(())) => Ok(text),
        (_, Err(_)) => Err(INVALID.into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc_3492_examples() {
        assert_eq!(encode("münchen").unwrap(), "mnchen-3ya");
        assert_eq!(decode("mnchen-3ya").unwrap(), "münchen");
        // RFC 3492 §7.1 (B) and (L): Chinese, and mixed case kept.
        assert_eq!(
            encode("他们为什么不说中文").unwrap(),
            "ihqwcrb4cv8a8dqg056pqjye"
        );
        assert_eq!(
            decode("3B-ww4c5e180e575a65lsy2b").unwrap(),
            "3年B組金八先生"
        );
        assert_eq!(decode("Bcher-kva").unwrap(), "Bücher");
    }

    #[test]
    fn idna_domains() {
        assert_eq!(to_ascii("mañana.com").unwrap(), "xn--maana-pta.com");
        // UTS #46 mapping: case folding and the ideographic full stop.
        assert_eq!(to_ascii("Bücher。DE").unwrap(), "xn--bcher-kva.de");
        assert_eq!(to_unicode("me@xn--maana-pta.com").unwrap(), "me@mañana.com");
        // The ACE prefix in any case; the domain is after the last `@`.
        assert_eq!(to_unicode("XN--maana-pta.com").unwrap(), "mañana.com");
        assert_eq!(
            to_ascii("\"a@b\"@bücher.de").unwrap(),
            "\"a@b\"@xn--bcher-kva.de"
        );
        assert_eq!(to_unicode("xn--a.com"), Err(INVALID.into()));
    }

    #[test]
    fn errors() {
        assert_eq!(decode("ü-a"), Err(NOT_BASIC.into()));
        assert_eq!(decode("a-!"), Err(INVALID.into()));
        assert_eq!(decode("99999999"), Err(INVALID.into()));
    }
}
