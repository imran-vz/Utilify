//! Parse ASN.1 hex string: a text dump of the BER element at the starting
//! index, read with `utilify-asn1`'s `RawWalker` per ITU-T X.690.
//!
//! The walker delimits elements per X.690 §8.1: identifiers in the
//! low- and high-tag-number forms (§8.1.2.4, any tag number), definite
//! lengths in the short and long forms (§8.1.3.4, §8.1.3.5, including
//! non-minimal long forms, which BER allows) and the indefinite form with
//! its end-of-contents octets (§8.1.3.6, §8.1.5). Truncated or overrunning
//! elements, the reserved length octet `FF`, an indefinite length on a
//! primitive element and nesting deeper than 300 levels are errors.
//!
//! Primitive contents are checked where the dump interprets them:
//! BOOLEAN is one octet (§8.2.1), INTEGER and ENUMERATED are minimal
//! two's complement (§8.3.2, §8.4; ENUMERATED is shown as its integer
//! value), NULL is empty (§8.8.2), BIT STRING has a valid unused-bits
//! octet (§8.6.2.2), OBJECT IDENTIFIER contents are minimal complete
//! subidentifiers (§8.19.2) whose first one is split into two arcs per
//! §8.19.4 (`2.999` from `88 37`), and character strings decode in their
//! type's character set (§8.23, by `StringType::decode`; BMPString is
//! UCS-2). Types that must be primitive (or constructed) in the other
//! form are errors (§8.1.2.5). Contents that fail a check are an error.
//!
//! OCTET STRING, BIT STRING (with no unused bits) and primitive tagged
//! values are shown as encapsulating when their contents are themselves a
//! structurally valid sequence of elements (`RawElement::encapsulated`).
//! Beyond the depth limit, nested encapsulations are shown as octets.
//!
//! The text layout is CyberChef's (jsrsasign's `ASN1HEX.dump`): two spaces
//! of indent per level, the labels `BITSTRING`, `OCTETSTRING`,
//! `ObjectIdentifier`, `SEQUENCE {}` for an empty SEQUENCE, hex shortened
//! by the "Truncate octet strings longer than" option, and jsrsasign's OID
//! names. Context-specific `[2]` inside a subjectAltName extension
//! (dNSName, RFC 5280 §4.2.1.6) and tagged values starting with `http`
//! are shown as text when they are valid ASCII/UTF-8. Labels jsrsasign
//! lacks use the X.680 names, and tags of other classes the X.680
//! notation (`[APPLICATION 1]`, `[PRIVATE 1]`, `[UNIVERSAL 15]`).
//!
//! The dump covers the one element at the starting index (counted in hex
//! digits, as `String.prototype.substr`); anything after it is not read.

use super::CATEGORY;
use crate::{Arg, Args, Dish, OpError, Operation};
use num_bigint::BigUint;
use utilify_asn1::{
    Arc, Class, Header, Integer, Oid, RawElement, RawWalker, Rules, StringType, universal,
};

pub(super) fn ops() -> Vec<&'static Operation> {
    vec![&PARSE_ASN1_HEX_STRING]
}

pub static PARSE_ASN1_HEX_STRING: Operation = Operation {
    name: "Parse ASN.1 hex string",
    category: CATEGORY,
    description: "Abstract Syntax Notation One (ASN.1) is a standard and notation that describes rules and structures for representing, encoding, transmitting, and decoding data in telecommunications and computer networking.\n\nThis operation parses arbitrary ASN.1 data (encoded as an hex string: use the 'To Hex' operation if necessary) and presents the resulting tree.",
    info_url: Some("https://wikipedia.org/wiki/Abstract_Syntax_Notation_One"),
    args: &[
        Arg::number("Starting index", 0.0),
        Arg::number("Truncate octet strings longer than", 32.0),
    ],
    run: |input, args: &Args| {
        let hex: Vec<u16> = input
            .text()
            .chars()
            .filter(|c| !is_js_space(*c))
            .collect::<String>()
            .encode_utf16()
            .collect();
        let size = hex.len() as i64;
        let from = substr(&hex, js_int(args.num(0)), size);
        let dumper = Dumper { omit: args.num(1) };
        decode_hex(from)
            .and_then(|bytes| dumper.dump(&bytes))
            .map(Dish::Text)
            .map_err(|e| OpError::new(format!("Parse ASN.1 hex string - {e}")))
    },
};

/// JavaScript's `\s`: whitespace and line terminators.
fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200a}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202f}'
                | '\u{205f}'
                | '\u{3000}'
                | '\u{feff}'
    )
}

/// `ToIntegerOrInfinity`, saturated into an `i64` (NaN is 0).
fn js_int(n: f64) -> i64 {
    if n.is_nan() {
        0
    } else {
        n.trunc().clamp(-1e15, 1e15) as i64
    }
}

/// `String.prototype.substr` on UTF-16 units (or ASCII bytes).
fn substr<T>(s: &[T], start: i64, len: i64) -> &[T] {
    let size = s.len() as i64;
    let start = if start < 0 {
        (size + start).max(0)
    } else {
        start.min(size)
    };
    let len = len.clamp(0, size - start);
    &s[start as usize..(start + len) as usize]
}

/// Hex digits (either case) to bytes.
fn decode_hex(h: &[u16]) -> Result<Vec<u8>, String> {
    let digit = |i: usize| {
        char::from_u32(u32::from(h[i]))
            .and_then(|c| c.to_digit(16))
            .ok_or_else(|| {
                let c = String::from_utf16_lossy(&h[i..=i]);
                format!("invalid hex digit '{c}' at index {i}")
            })
    };
    if h.len() % 2 == 1 {
        return Err("odd number of hex digits".into());
    }
    (0..h.len())
        .step_by(2)
        .map(|i| Ok((digit(i)? * 16 + digit(i + 1)?) as u8))
        .collect()
}

fn to_hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect()
}

/// How deep nesting may go.
const MAX_DEPTH: usize = 300;

/// The DER contents of OID 2.5.29.17 (subjectAltName).
const SUBJECT_ALT_NAME: [u8; 3] = [0x55, 0x1d, 0x11];

fn universal_name(n: u64) -> Option<&'static str> {
    use universal::*;
    Some(match n {
        BOOLEAN => "BOOLEAN",
        INTEGER => "INTEGER",
        BIT_STRING => "BITSTRING",
        OCTET_STRING => "OCTETSTRING",
        NULL => "NULL",
        OBJECT_IDENTIFIER => "ObjectIdentifier",
        OBJECT_DESCRIPTOR => "ObjectDescriptor",
        EXTERNAL => "EXTERNAL",
        REAL => "REAL",
        ENUMERATED => "ENUMERATED",
        EMBEDDED_PDV => "EMBEDDED PDV",
        UTF8_STRING => "UTF8String",
        RELATIVE_OID => "RELATIVE-OID",
        TIME => "TIME",
        SEQUENCE => "SEQUENCE",
        SET => "SET",
        NUMERIC_STRING => "NumericString",
        PRINTABLE_STRING => "PrintableString",
        TELETEX_STRING => "TeletexString",
        VIDEOTEX_STRING => "VideotexString",
        IA5_STRING => "IA5String",
        UTC_TIME => "UTCTime",
        GENERALIZED_TIME => "GeneralizedTime",
        GRAPHIC_STRING => "GraphicString",
        VISIBLE_STRING => "VisibleString",
        GENERAL_STRING => "GeneralString",
        UNIVERSAL_STRING => "UniversalString",
        CHARACTER_STRING => "CHARACTER STRING",
        BMP_STRING => "BMPString",
        DATE => "DATE",
        TIME_OF_DAY => "TIME-OF-DAY",
        DATE_TIME => "DATE-TIME",
        DURATION => "DURATION",
        OID_IRI => "OID-IRI",
        RELATIVE_OID_IRI => "RELATIVE-OID-IRI",
        _ => return None,
    })
}

/// The tag number in decimal, re-read from the identifier octets when it
/// does not fit in `u64` (X.690 §8.1.2.4.2: base-128 groups after the
/// first octet).
fn tag_number(e: &RawElement<'_>) -> String {
    match e.number {
        Some(n) => n.to_string(),
        None => e.identifier[1..]
            .iter()
            .fold(BigUint::default(), |n, &b| {
                (n << 7u8) | BigUint::from(b & 0x7f)
            })
            .to_string(),
    }
}

/// The element's label: its universal type name or the X.680 tag notation.
fn label(e: &RawElement<'_>) -> String {
    match e.class {
        Class::Universal => match e.number.and_then(universal_name) {
            Some(name) => name.into(),
            None => format!("[UNIVERSAL {}]", tag_number(e)),
        },
        Class::Application => format!("[APPLICATION {}]", tag_number(e)),
        Class::ContextSpecific => format!("[{}]", tag_number(e)),
        Class::Private => format!("[PRIVATE {}]", tag_number(e)),
    }
}

/// An error about the element at `e`.
fn invalid(e: &RawElement<'_>, reason: &str) -> String {
    format!("invalid {} at offset {}: {reason}", label(e), e.offset)
}

/// The dotted arcs of a checked OID, all in decimal.
fn oid_arcs(oid: &Oid<'_>) -> Vec<String> {
    oid.arcs()
        .into_iter()
        .map(|arc| match arc {
            Arc::Hex(digits) => BigUint::parse_bytes(digits.as_bytes(), 16)
                .map_or_else(|| format!("0x{digits}"), |n| n.to_string()),
            arc => arc.to_string(),
        })
        .collect()
}

/// `true` if the SEQUENCE `e` is an X.509 extension (2 or 3 children, an
/// OBJECT IDENTIFIER first and an OCTET STRING last); then `Some` of
/// whether it is subjectAltName. Only definite-length children are looked
/// at, by their headers.
fn extension(e: &RawElement<'_>, input: &[u8]) -> Option<bool> {
    let mut kids: Vec<(Header, usize)> = vec![];
    let mut pos = e.contents.start;
    while pos < e.contents.end {
        if kids.len() == 3 {
            return None;
        }
        let h = Header::decode(input.get(pos..e.contents.end)?, Rules::Ber).ok()?;
        let len = h.length.definite()?;
        kids.push((h, pos));
        pos = pos.checked_add(h.header_len)?.checked_add(len)?;
    }
    let (first, at) = kids.first()?;
    let (last, _) = kids.last()?;
    let is =
        |h: &Header, n| h.tag.class == Class::Universal && !h.tag.constructed && h.tag.number == n;
    if kids.len() < 2
        || !is(first, universal::OBJECT_IDENTIFIER)
        || !is(last, universal::OCTET_STRING)
    {
        return None;
    }
    let start = at + first.header_len;
    let contents = input.get(start..start.checked_add(first.length.definite()?)?)?;
    Some(contents == SUBJECT_ALT_NAME)
}

/// What to print for one element.
struct Line<'a> {
    text: String,
    /// Contents to dump below it, one level deeper.
    encapsulated: Option<RawWalker<'a>>,
    /// For a SEQUENCE that is an X.509 extension: whether its descendants
    /// are inside subjectAltName.
    san: Option<bool>,
}

impl<'a> Line<'a> {
    fn new(text: String) -> Line<'a> {
        Line {
            text,
            encapsulated: None,
            san: None,
        }
    }
}

struct Dumper {
    /// Hex longer than this many bytes is shortened.
    omit: f64,
}

impl Dumper {
    /// jsrsasign's `head..(total N bytes)..tail` for long hex.
    fn shorten(&self, v: &str) -> String {
        let i = self.omit;
        if (v.len() as f64) <= i * 2.0 {
            return v.into();
        }
        let n = js_int(i);
        let b = v.as_bytes();
        format!(
            "{}..(total {}bytes)..{}",
            String::from_utf8_lossy(substr(b, 0, n)),
            super::js::js_number(v.len() as f64 / 2.0),
            String::from_utf8_lossy(substr(b, js_int(v.len() as f64 - i), n))
        )
    }

    /// The dump of the element at the start of `bytes`.
    fn dump(&self, bytes: &[u8]) -> Result<String, String> {
        let first = RawWalker::new(bytes)
            .max_depth(MAX_DEPTH)
            .next()
            .ok_or("no ASN.1 element")?
            .map_err(|e| e.to_string())?;
        let input = &bytes[..first.end()];
        let mut walkers = vec![RawWalker::new(input).max_depth(MAX_DEPTH)];
        // (depth of an extension SEQUENCE, whether it is subjectAltName).
        let mut san: Vec<(usize, bool)> = vec![];
        let mut out = String::new();
        while let Some(walker) = walkers.last_mut() {
            let Some(e) = walker.next() else {
                walkers.pop();
                continue;
            };
            let e = e.map_err(|e| e.to_string())?;
            while san.last().is_some_and(|&(d, _)| d >= e.depth) {
                san.pop();
            }
            let in_san = san.last().is_some_and(|&(_, s)| s);
            let line = self.line(&e, input, in_san)?;
            for _ in 0..e.depth {
                out.push_str("  ");
            }
            out.push_str(&line.text);
            out.push('\n');
            if let Some(s) = line.san {
                san.push((e.depth, s));
            }
            walkers.extend(line.encapsulated);
        }
        Ok(out)
    }

    fn line<'a>(&self, e: &RawElement<'a>, input: &[u8], san: bool) -> Result<Line<'a>, String> {
        let name = label(e);
        let c = e.contents_bytes();
        if e.class != Class::Universal {
            if e.constructed {
                return Ok(Line::new(name));
            }
            if let Some(inner) = e.encapsulated() {
                return Ok(Line {
                    encapsulated: Some(inner),
                    ..Line::new(name)
                });
            }
            let texty = e.class == Class::ContextSpecific
                && ((san && e.number == Some(2)) || c.starts_with(b"http"));
            let shown = match std::str::from_utf8(c) {
                Ok(s) if texty && (!san || s.is_ascii()) => s.into(),
                _ => to_hex(c),
            };
            return Ok(Line::new(format!("{name} {shown}")));
        }
        let Some(n) = e.number else {
            return Ok(Line::new(if e.constructed {
                name
            } else {
                format!("{name} {}", to_hex(c))
            }));
        };
        use universal::*;
        let primitive_only = matches!(
            n,
            BOOLEAN | INTEGER | NULL | OBJECT_IDENTIFIER | REAL | ENUMERATED | RELATIVE_OID
        );
        if e.constructed {
            if primitive_only {
                return Err(invalid(
                    e,
                    "constructed encoding of a primitive type (8.1.2.5)",
                ));
            }
            return Ok(match n {
                SEQUENCE if e.contents.is_empty() => Line::new("SEQUENCE {}".into()),
                SEQUENCE => Line {
                    san: extension(e, input),
                    ..Line::new(name)
                },
                SET | EXTERNAL | EMBEDDED_PDV | CHARACTER_STRING => Line::new(name),
                _ => Line::new(format!("{name}, constructed")),
            });
        }
        let text = match n {
            SEQUENCE | SET | EXTERNAL | EMBEDDED_PDV | CHARACTER_STRING => {
                return Err(invalid(
                    e,
                    "primitive encoding of a constructed type (8.1.2.5)",
                ));
            }
            BOOLEAN => match c {
                [0] => "BOOLEAN FALSE".into(),
                [_] => "BOOLEAN TRUE".into(),
                _ => return Err(invalid(e, "contents must be a single octet (8.2.1)")),
            },
            INTEGER | ENUMERATED => {
                let v = Integer::new(c).map_err(|err| invalid(e, err.reason()))?;
                if n == INTEGER {
                    format!("INTEGER {}", self.shorten(&to_hex(c)))
                } else {
                    format!("ENUMERATED {v}")
                }
            }
            BIT_STRING => {
                match c {
                    [] => return Err(invalid(e, "contents must not be empty (8.6.2)")),
                    [u, ..] if *u > 7 => {
                        return Err(invalid(e, "unused bits must be 0 to 7 (8.6.2.2)"));
                    }
                    [u] if *u != 0 => {
                        return Err(invalid(
                            e,
                            "unused bits must be 0 without subsequent octets (8.6.2.3)",
                        ));
                    }
                    _ => {}
                }
                if let Some(inner) = e.encapsulated() {
                    return Ok(Line {
                        encapsulated: Some(inner),
                        ..Line::new("BITSTRING, encapsulates".into())
                    });
                }
                format!("BITSTRING {}", self.shorten(&to_hex(c)))
            }
            OCTET_STRING => {
                if let Some(inner) = e.encapsulated() {
                    return Ok(Line {
                        encapsulated: Some(inner),
                        ..Line::new("OCTETSTRING, encapsulates".into())
                    });
                }
                format!("OCTETSTRING {}", self.shorten(&to_hex(c)))
            }
            NULL if c.is_empty() => "NULL".into(),
            NULL => return Err(invalid(e, "contents must be empty (8.8.2)")),
            OBJECT_IDENTIFIER => {
                let oid = Oid::new(c).map_err(|err| invalid(e, err.reason()))?;
                let arcs = oid_arcs(&oid);
                let spaced = arcs.join(" ");
                match oid_name(&arcs.join(".")) {
                    "" => format!("ObjectIdentifier ({spaced})"),
                    name => format!("ObjectIdentifier {name} ({spaced})"),
                }
            }
            UTC_TIME | GENERALIZED_TIME => {
                let text = StringType::Visible
                    .decode(c)
                    .ok_or_else(|| invalid(e, "not a VisibleString (8.25)"))?;
                format!("{name} {text}")
            }
            n => match StringType::from_tag_number(n) {
                Some(kind) => {
                    let text = kind.decode(c).ok_or_else(|| {
                        invalid(e, "contents outside the type's character set (8.23)")
                    })?;
                    format!("{name} '{text}'")
                }
                None => format!("{name} {}", to_hex(c)),
            },
        };
        Ok(Line::new(text))
    }
}

/// jsrsasign's `KJUR.asn1.x509.OID.oid2name`: the first registered name,
/// or "".
fn oid_name(oid: &str) -> &'static str {
    OID_NAMES
        .iter()
        .find(|(o, _)| *o == oid)
        .map_or("", |(_, n)| n)
}

/// jsrsasign's `KJUR.asn1.x509.OID.name2oidList`, first name per OID.
const OID_NAMES: &[(&str, &str)] = &[
    ("2.16.840.1.101.3.4.1.2", "aes128-CBC"),
    ("2.16.840.1.101.3.4.1.42", "aes256-CBC"),
    ("1.3.14.3.2.26", "sha1"),
    ("2.16.840.1.101.3.4.2.1", "sha256"),
    ("2.16.840.1.101.3.4.2.2", "sha384"),
    ("2.16.840.1.101.3.4.2.3", "sha512"),
    ("2.16.840.1.101.3.4.2.4", "sha224"),
    ("1.2.840.113549.2.5", "md5"),
    ("1.3.14.7.2.2.1", "md2"),
    ("1.3.36.3.2.1", "ripemd160"),
    ("1.2.840.113549.2.7", "hmacWithSHA1"),
    ("1.2.840.113549.2.8", "hmacWithSHA224"),
    ("1.2.840.113549.2.9", "hmacWithSHA256"),
    ("1.2.840.113549.2.10", "hmacWithSHA384"),
    ("1.2.840.113549.2.11", "hmacWithSHA512"),
    ("1.2.840.113549.1.1.2", "MD2withRSA"),
    ("1.2.840.113549.1.1.3", "MD4withRSA"),
    ("1.2.840.113549.1.1.4", "MD5withRSA"),
    ("1.2.840.113549.1.1.5", "SHA1withRSA"),
    ("1.2.840.113549.1.1.8", "pkcs1-MGF"),
    ("1.2.840.113549.1.1.10", "rsaPSS"),
    ("1.2.840.113549.1.1.14", "SHA224withRSA"),
    ("1.2.840.113549.1.1.11", "SHA256withRSA"),
    ("1.2.840.113549.1.1.12", "SHA384withRSA"),
    ("1.2.840.113549.1.1.13", "SHA512withRSA"),
    ("1.2.840.10045.4.1", "SHA1withECDSA"),
    ("1.2.840.10045.4.3.1", "SHA224withECDSA"),
    ("1.2.840.10045.4.3.2", "SHA256withECDSA"),
    ("1.2.840.10045.4.3.3", "SHA384withECDSA"),
    ("1.2.840.10045.4.3.4", "SHA512withECDSA"),
    ("1.2.840.10040.4.1", "dsa"),
    ("1.2.840.10040.4.3", "SHA1withDSA"),
    ("2.16.840.1.101.3.4.3.1", "SHA224withDSA"),
    ("2.16.840.1.101.3.4.3.2", "SHA256withDSA"),
    ("1.2.840.113549.1.1.1", "rsaEncryption"),
    ("2.5.4.3", "commonName"),
    ("2.5.4.6", "countryName"),
    ("2.5.4.7", "localityName"),
    ("2.5.4.8", "stateOrProvinceName"),
    ("2.5.4.9", "streetAddress"),
    ("2.5.4.10", "organizationName"),
    ("2.5.4.11", "organizationalUnitName"),
    ("0.9.2342.19200300.100.1.25", "domainComponent"),
    ("0.9.2342.19200300.100.1.1", "userId"),
    ("2.5.4.4", "surname"),
    ("2.5.4.42", "givenName"),
    ("2.5.4.12", "title"),
    ("2.5.4.49", "distinguishedName"),
    ("1.2.840.113549.1.9.1", "emailAddress"),
    ("2.5.4.13", "description"),
    ("2.5.4.15", "businessCategory"),
    ("2.5.4.17", "postalCode"),
    ("2.5.4.45", "uniqueIdentifier"),
    ("2.5.4.97", "organizationIdentifier"),
    ("1.3.6.1.4.1.311.60.2.1.1", "jurisdictionOfIncorporationL"),
    ("1.3.6.1.4.1.311.60.2.1.2", "jurisdictionOfIncorporationSP"),
    ("1.3.6.1.4.1.311.60.2.1.3", "jurisdictionOfIncorporationC"),
    ("2.5.29.9", "subjectDirectoryAttributes"),
    ("2.5.29.14", "subjectKeyIdentifier"),
    ("2.5.29.15", "keyUsage"),
    ("2.5.29.17", "subjectAltName"),
    ("2.5.29.18", "issuerAltName"),
    ("2.5.29.19", "basicConstraints"),
    ("2.5.29.20", "cRLNumber"),
    ("2.5.29.21", "cRLReason"),
    ("2.5.29.30", "nameConstraints"),
    ("2.5.29.31", "cRLDistributionPoints"),
    ("2.5.29.32", "certificatePolicies"),
    ("2.5.29.32.0", "anyPolicy"),
    ("2.5.29.33", "policyMappings"),
    ("2.5.29.35", "authorityKeyIdentifier"),
    ("2.5.29.36", "policyConstraints"),
    ("2.5.29.37", "extKeyUsage"),
    ("2.5.29.54", "inhibitAnyPolicy"),
    ("1.3.6.1.5.5.7.1.1", "authorityInfoAccess"),
    ("1.3.6.1.5.5.7.48.1", "ocsp"),
    ("1.3.6.1.5.5.7.48.1.1", "ocspBasic"),
    ("1.3.6.1.5.5.7.48.1.2", "ocspNonce"),
    ("1.3.6.1.5.5.7.48.1.5", "ocspNoCheck"),
    ("1.3.6.1.5.5.7.48.2", "caIssuers"),
    ("2.5.29.37.0", "anyExtendedKeyUsage"),
    ("1.3.6.1.5.5.7.3.1", "serverAuth"),
    ("1.3.6.1.5.5.7.3.2", "clientAuth"),
    ("1.3.6.1.5.5.7.3.3", "codeSigning"),
    ("1.3.6.1.5.5.7.3.4", "emailProtection"),
    ("1.3.6.1.5.5.7.3.8", "timeStamping"),
    ("1.3.6.1.5.5.7.3.9", "ocspSigning"),
    ("1.3.6.1.5.5.7.8.9", "smtpUTF8Mailbox"),
    ("1.3.6.1.5.5.7.9.1", "dateOfBirth"),
    ("1.3.6.1.5.5.7.9.2", "placeOfBirth"),
    ("1.3.6.1.5.5.7.9.3", "gender"),
    ("1.3.6.1.5.5.7.9.4", "countryOfCitizenship"),
    ("1.3.6.1.5.5.7.9.5", "countryOfResidence"),
    ("1.2.840.10045.2.1", "ecPublicKey"),
    ("1.2.840.10045.3.1.7", "P-256"),
    ("1.3.132.0.10", "secp256k1"),
    ("1.3.132.0.34", "secp384r1"),
    ("1.3.132.0.35", "secp521r1"),
    ("1.2.840.113549.1.5.13", "pkcs5PBES2"),
    ("1.2.840.113549.1.5.12", "pkcs5PBKDF2"),
    ("1.2.840.113549.3.7", "des-EDE3-CBC"),
    ("1.2.840.113549.1.7.1", "data"),
    ("1.2.840.113549.1.7.2", "signed-data"),
    ("1.2.840.113549.1.7.3", "enveloped-data"),
    ("1.2.840.113549.1.7.5", "digested-data"),
    ("1.2.840.113549.1.7.6", "encrypted-data"),
    ("1.2.840.113549.1.9.16.1.2", "authenticated-data"),
    ("1.2.840.113549.1.9.16.1.4", "tstinfo"),
    ("1.2.840.113549.1.9.16.2.12", "signingCertificate"),
    ("1.2.840.113549.1.9.16.2.14", "timeStampToken"),
    ("1.2.840.113549.1.9.16.2.15", "signaturePolicyIdentifier"),
    ("1.2.840.113549.1.9.16.2.27", "etsArchiveTimeStamp"),
    ("1.2.840.113549.1.9.16.2.47", "signingCertificateV2"),
    ("1.2.840.113549.1.9.16.2.48", "etsArchiveTimeStampV2"),
    ("1.2.840.113549.1.9.14", "extensionRequest"),
    ("1.2.840.113549.1.9.3", "contentType"),
    ("1.2.840.113549.1.9.4", "messageDigest"),
    ("1.2.840.113549.1.9.5", "signingTime"),
    ("1.2.840.113549.1.9.6", "counterSignature"),
    ("0.4.0.1733.2.4", "archiveTimeStampV3"),
    ("1.2.840.113583.1.1.8", "pdfRevocationInfoArchival"),
    ("1.2.840.113583.1.1.9.1", "adobeTimeStamp"),
    ("2.23.140.1.5.1.1", "smimeMailboxLegacy"),
    ("2.23.140.1.5.1.2", "smimeMailboxMulti"),
    ("2.23.140.1.5.1.3", "smimeMailboxStrict"),
    ("2.23.140.1.5.2.1", "smimeOrganizationLegacy"),
    ("2.23.140.1.5.2.2", "smimeOrganizationMulti"),
    ("2.23.140.1.5.2.3", "smimeOrganizationStrict"),
    ("2.23.140.1.5.3.1", "smimeSponsorLegacy"),
    ("2.23.140.1.5.3.2", "smimeSponsorMulti"),
    ("2.23.140.1.5.3.3", "smimeSponsorStrict"),
    ("2.23.140.1.5.4.1", "smimeIndividualLegacy"),
    ("2.23.140.1.5.4.2", "smimeIndividualMulti"),
    ("2.23.140.1.5.4.3", "smimeIndividualStrict"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ArgValue;

    fn run(input: &str, index: f64, omit: f64) -> String {
        let args = [ArgValue::Num(index), ArgValue::Num(omit)];
        match PARSE_ASN1_HEX_STRING.apply(Dish::Text(input.into()), &args) {
            Ok(d) => d.into_text(),
            Err(e) => e.0,
        }
    }

    fn fails(input: &str) -> bool {
        run(input, 0.0, 32.0).starts_with("Parse ASN.1 hex string - ")
    }

    #[test]
    fn dumps_elements() {
        let cases = [
            ("30 03 02 01 05", "SEQUENCE\n  INTEGER 05\n"),
            ("0101ff", "BOOLEAN TRUE\n"),
            ("010100", "BOOLEAN FALSE\n"),
            ("0303000500", "BITSTRING, encapsulates\n  NULL\n"),
            ("0303010500", "BITSTRING 010500\n"),
            ("04020500", "OCTETSTRING, encapsulates\n  NULL\n"),
            // Its first length matches, but `ffff` is not an element.
            ("04043002ffff", "OCTETSTRING 3002ffff\n"),
            (
                "06092A864886F70D010101",
                "ObjectIdentifier rsaEncryption (1 2 840 113549 1 1 1)\n",
            ),
            // ENUMERATED is an integer (X.690 8.4).
            ("0a01ff", "ENUMERATED -1\n"),
            ("0a020100", "ENUMERATED 256\n"),
            ("0c03c3a941", "UTF8String 'éA'\n"),
            ("1a024142", "VisibleString 'AB'\n"),
            ("1703313233", "UTCTime 123\n"),
            // BMPString is UCS-2, big-endian.
            ("1e0400e90100", "BMPString 'éĀ'\n"),
            ("3000", "SEQUENCE {}\n"),
            ("3100", "SET\n"),
            ("a003020101", "[0]\n  INTEGER 01\n"),
            ("800468747470", "[0] http\n"),
            ("820461626364", "[2] 61626364\n"),
            (
                "30100603551d110409300782056162636465",
                "SEQUENCE\n  ObjectIdentifier subjectAltName (2 5 29 17)\n  OCTETSTRING, encapsulates\n    SEQUENCE\n      [2] abcde\n",
            ),
            ("40024142", "[APPLICATION 0] 4142\n"),
            ("c1024142", "[PRIVATE 1] 4142\n"),
            ("0f024142", "[UNIVERSAL 15] 4142\n"),
            // Non-minimal long-form lengths are BER (8.1.3.5).
            ("3081050201050500", "SEQUENCE\n  INTEGER 05\n  NULL\n"),
            ("308300000302010a", "SEQUENCE\n  INTEGER 0a\n"),
            // Constructed strings hold segments (8.7.3).
            (
                "2406040141040142",
                "OCTETSTRING, constructed\n  OCTETSTRING 41\n  OCTETSTRING 42\n",
            ),
            // Trailing data after the element is not read.
            ("05000101ff", "NULL\n"),
        ];
        for (input, want) in cases {
            assert_eq!(run(input, 0.0, 32.0), want, "{input}");
        }
    }

    #[test]
    fn high_tag_numbers() {
        // X.690 8.1.2.4: tag number 42 in the high-tag-number form.
        assert_eq!(
            run("30069f2a01010500", 0.0, 32.0),
            "SEQUENCE\n  [42] 01\n  NULL\n"
        );
        assert_eq!(run("bf2a03020101", 0.0, 32.0), "[42]\n  INTEGER 01\n");
        assert_eq!(run("5f8148014f", 0.0, 32.0), "[APPLICATION 200] 4f\n");
        // A tag number beyond u64.
        let id = format!("9f{}01", "ff".repeat(10));
        let n = (BigUint::from(1u8) << 77u8) - 127u8;
        assert_eq!(run(&format!("{id}0100"), 0.0, 32.0), format!("[{n}] 00\n"));
    }

    #[test]
    fn indefinite_lengths() {
        assert_eq!(run("30800201050000", 0.0, 32.0), "SEQUENCE\n  INTEGER 05\n");
        assert_eq!(
            run("3080a08002010100000000", 0.0, 32.0),
            "SEQUENCE\n  [0]\n    INTEGER 01\n"
        );
        assert_eq!(run("30800000", 0.0, 32.0), "SEQUENCE {}\n");
        // No end-of-contents octets, or on a primitive (8.1.3.2 a).
        assert!(fails("3080020105"));
        assert!(fails("0480000000"));
    }

    #[test]
    fn structural_errors() {
        for input in [
            "",
            "3082",
            "3005020105",
            "300302020105",
            "05ff",
            "0x0141",
            "abc",
            "3003020105ab",
        ] {
            if input == "3003020105ab" {
                // Trailing bytes after the element are ignored.
                assert_eq!(run(input, 0.0, 32.0), "SEQUENCE\n  INTEGER 05\n");
            } else {
                assert!(fails(input), "{input}");
            }
        }
    }

    #[test]
    fn invalid_contents_are_errors() {
        for input in [
            "0102ffff",   // BOOLEAN of two octets (8.2.1)
            "0200",       // empty INTEGER (8.3.1)
            "02020001",   // non-minimal INTEGER (8.3.2)
            "0a00",       // empty ENUMERATED
            "050100",     // NULL with contents (8.8.2)
            "0300",       // BIT STRING without its initial octet (8.6.2)
            "030108",     // 8 unused bits (8.6.2.2)
            "0600",       // empty OID (8.19.2)
            "06032a0381", // incomplete last subidentifier
            "06028001",   // padded subidentifier
            "0c02c328",   // invalid UTF-8
            "13012a",     // `*` is not printable
            "1e03004100", // odd-length BMPString
            "2103010101", // constructed BOOLEAN (8.1.2.5)
            "1000",       // primitive SEQUENCE
        ] {
            assert!(fails(input), "{input}");
        }
    }

    #[test]
    fn oids_follow_x690() {
        // X.690 8.19.5's example {2 999 3}.
        assert_eq!(run("0603883703", 0.0, 32.0), "ObjectIdentifier (2 999 3)\n");
        assert_eq!(run("060178", 0.0, 32.0), "ObjectIdentifier (2 40)\n");
        assert_eq!(run("06012a", 0.0, 32.0), "ObjectIdentifier (1 2)\n");
        // One arc of 1,057 bits, printed in decimal.
        let groups = 151;
        let mut contents = "2a".to_string();
        for g in 0..groups {
            contents.push_str(if g + 1 < groups { "ff" } else { "7f" });
        }
        let hex = format!("0681{:02x}{contents}", contents.len() / 2);
        let arc = (BigUint::from(1u8) << (7 * groups)) - 1u8;
        assert_eq!(
            run(&hex, 0.0, 32.0),
            format!("ObjectIdentifier (1 2 {arc})\n")
        );
    }

    #[test]
    fn deep_nesting_fails_cleanly() {
        let mut hex = "0500".to_string();
        for _ in 0..1200 {
            hex = format!("3082{:04x}{hex}", hex.len() / 2);
        }
        assert!(fails(&hex));
        // Encapsulation past the limit is shown as octets.
        let mut hex = "0500".to_string();
        for _ in 0..400 {
            hex = format!("0482{:04x}{hex}", hex.len() / 2);
        }
        let out = run(&hex, 0.0, 4.0);
        assert_eq!(out.lines().count(), MAX_DEPTH + 1);
        // 5,000 levels on a small stack.
        let mut hex = "0500".to_string();
        for _ in 0..5000 {
            hex = format!("3083{:06x}{hex}", hex.len() / 2);
        }
        let out = std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(move || fails(&hex))
            .expect("spawn")
            .join()
            .expect("join");
        assert!(out);
    }

    #[test]
    fn index_and_truncation() {
        assert_eq!(run("3006020105020106", 10.0, 32.0), "INTEGER 06\n");
        assert_eq!(run("3006020105020106", -6.0, 32.0), "INTEGER 06\n");
        assert!(run("3006020105020106", 3.0, 32.0).starts_with("Parse ASN.1 hex string - "));
        let octets = format!("0410{}", "ab".repeat(16));
        assert_eq!(
            run(&octets, 0.0, 7.0),
            "OCTETSTRING abababa..(total 16bytes)..bababab\n"
        );
        assert_eq!(
            run(&octets, 0.0, 7.5),
            "OCTETSTRING abababa..(total 16bytes)..abababa\n"
        );
        assert_eq!(
            run(&octets, 0.0, 16.0),
            format!("OCTETSTRING {}\n", "ab".repeat(16))
        );
        assert_eq!(run(&octets, 0.0, 0.5), "OCTETSTRING ..(total 16bytes)..\n");
    }
}
