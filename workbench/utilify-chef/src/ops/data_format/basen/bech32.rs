//! Bech32 (BIP-0173) and Bech32m (BIP-0350), generic or as Bitcoin SegWit
//! addresses.
//!
//! The checksums, the charset and the 8-to-5-bit conversion are
//! `bech32::primitives` (not the crate's top-level `encode`/`decode` or
//! `segwit` modules, whose length limits and rules differ from the op's
//! generic mode). The adapter implements BIP-0173's rules with CyberChef's
//! messages: the 90-character limit, case and separator rules, HRP
//! characters (33-126), witness version 0-16 and program lengths (2-40
//! bytes, 20 or 32 for version 0), and padding of at most 4 zero bits.
//! BIP-0350 pairs the checksum with the witness version: version 0 uses
//! Bech32, versions 1-16 Bech32m. To Bech32 in SegWit mode rejects the
//! other pairing; From Bech32 does not read such a string as a witness
//! program (it decodes every word instead, as for other non-SegWit data).

use super::{CATEGORY, decode_utf8, from_utf16, js_is_space, utf16};
use crate::util::from_hex;
use crate::{Arg, Args, Dish, OpError, Operation};
use bech32::primitives::decode::{CheckedHrpstring, PaddingError, UncheckedHrpstring};
use bech32::{Bech32, Bech32m, ByteIterExt, Checksum, Fe32, Fe32IterExt, Hrp};

/// HRPs that may carry a SegWit witness version in their first word.
const SEGWIT_HRPS: &[&str] = &["bc", "tb", "ltc", "tltc", "bcrt"];

pub static TO_BECH32: Operation = Operation {
    name: "To Bech32",
    category: CATEGORY,
    description: "Bech32 is an encoding scheme primarily used for Bitcoin SegWit addresses (BIP-0173). It uses a 32-character alphabet that excludes easily confused characters (1, b, i, o) and includes a checksum for error detection.\n\nBech32m (BIP-0350) is an updated version that fixes a weakness in the original Bech32 checksum and is used for Bitcoin Taproot addresses.\n\nThe Human-Readable Part (HRP) identifies the network or purpose (e.g., 'bc' for Bitcoin mainnet, 'tb' for testnet, 'age' for AGE encryption keys).\n\nMaximum output length is 90 characters as per specification.",
    info_url: Some("https://wikipedia.org/wiki/Bech32"),
    args: &[
        Arg::string("Human-Readable Part (HRP)", "bc"),
        Arg::choice("Encoding", &["Bech32", "Bech32m"]),
        Arg::choice("Input Format", &["Raw bytes", "Hex"]),
        Arg::choice("Mode", &["Generic", "Bitcoin SegWit"]),
        Arg::number("Witness Version", 0.0),
    ],
    run: to_bech32,
};

fn to_bech32(input: Dish, args: &Args) -> Result<Dish, OpError> {
    let mut data = if args.str(2) == "Hex" {
        let text: String = decode_utf8(&input.bytes(), true)
            .chars()
            .filter(|&c| !js_is_space(c))
            .collect();
        from_hex(&text, "Auto", 2)?
    } else {
        input.into_bytes()
    };
    let segwit = args.str(3) == "Bitcoin SegWit";
    if segwit {
        // Stored into a Uint8Array: truncated, modulo 256, NaN as 0.
        let v = args.num(4);
        let v = if v.is_finite() {
            v.trunc().rem_euclid(256.0) as u8
        } else {
            0
        };
        data.insert(0, v);
    }
    Ok(Dish::Text(encode(
        args.str(0),
        &data,
        args.str(1) == "Bech32m",
        segwit,
    )?))
}

/// CyberChef's `Bech32.encode`. With `segwit`, the first byte is the
/// witness version, kept as one word, and the program's length is checked.
fn encode(hrp: &str, data: &[u8], bech32m: bool, segwit: bool) -> Result<String, OpError> {
    if hrp.is_empty() {
        return Err(OpError::new("Human-Readable Part (HRP) cannot be empty."));
    }
    if let Some(i) = utf16(hrp).iter().position(|&c| !(33..=126).contains(&c)) {
        return Err(OpError::new(format!(
            "HRP contains invalid character at position {i}. Only printable ASCII characters (33-126) are allowed."
        )));
    }
    let hrp = hrp.to_ascii_lowercase();
    let (version, program) = if let (true, Some((&version, program))) = (segwit, data.split_first())
    {
        if version > 16 {
            return Err(OpError::new(format!(
                "Invalid witness version: {version}. Must be 0-16."
            )));
        }
        // BIP-0350: version 0 uses Bech32, versions 1-16 Bech32m.
        if (version == 0) == bech32m {
            return Err(OpError::new(format!(
                "Witness version {version} requires {}.",
                if version == 0 { "Bech32" } else { "Bech32m" }
            )));
        }
        if program.len() < 2 || program.len() > 40 {
            return Err(OpError::new(format!(
                "Invalid witness program length: {}. Must be 2-40 bytes.",
                program.len()
            )));
        }
        if version == 0 && program.len() != 20 && program.len() != 32 {
            return Err(OpError::new(format!(
                "Invalid witness program length for v0: {}. Must be 20 or 32 bytes.",
                program.len()
            )));
        }
        (Some(version), program)
    } else {
        (None, data)
    };
    // HRP, "1", the version word, the data words and a 6-word checksum.
    let len = hrp.len() + 1 + usize::from(version.is_some()) + (program.len() * 8).div_ceil(5) + 6;
    if len > 90 {
        return Err(OpError::new(format!(
            "Encoded string exceeds maximum length of 90 characters (got {len}). Consider using smaller input data."
        )));
    }
    // Within 90 characters the HRP is at most 83, so the library accepts it.
    let hrp = Hrp::parse(&hrp).map_err(|e| OpError::new(e.to_string()))?;
    let version = version
        .map(Fe32::try_from)
        .transpose()
        .map_err(|e| OpError::new(e.to_string()))?;
    Ok(if bech32m {
        checksummed::<Bech32m>(&hrp, program, version)
    } else {
        checksummed::<Bech32>(&hrp, program, version)
    })
}

/// `hrp`, "1", the optional witness version, `data` in 5-bit words and the
/// `Ck` checksum.
fn checksummed<Ck: Checksum>(hrp: &Hrp, data: &[u8], version: Option<Fe32>) -> String {
    let encoder = data.iter().copied().bytes_to_fes().with_checksum::<Ck>(hrp);
    match version {
        Some(v) => encoder.with_witness_version(v).chars().collect(),
        None => encoder.chars().collect(),
    }
}

pub static FROM_BECH32: Operation = Operation {
    name: "From Bech32",
    category: CATEGORY,
    description: "Bech32 is an encoding scheme primarily used for Bitcoin SegWit addresses (BIP-0173). It uses a 32-character alphabet that excludes easily confused characters (1, b, i, o) and includes a checksum for error detection.\n\nBech32m (BIP-0350) is an updated version used for Bitcoin Taproot addresses.\n\nAuto-detect will attempt Bech32 first, then Bech32m if the checksum fails.\n\nOutput format options allow you to see the Human-Readable Part (HRP) along with the decoded data.",
    info_url: Some("https://wikipedia.org/wiki/Bech32"),
    args: &[
        Arg::choice("Encoding", &["Auto-detect", "Bech32", "Bech32m"]),
        Arg::choice(
            "Output Format",
            &["Raw", "Hex", "Bitcoin scriptPubKey", "HRP: Hex", "JSON"],
        ),
    ],
    run: from_bech32,
};

/// A decoded string: its HRP, bytes, which checksum matched, and the
/// witness version when it decoded as a SegWit address.
struct Decoded {
    hrp: String,
    data: Vec<u8>,
    encoding: &'static str,
    witness_version: Option<u8>,
}

fn from_bech32(input: Dish, args: &Args) -> Result<Dish, OpError> {
    let text = input.text();
    let text = text.trim_matches(js_is_space);
    if text.is_empty() {
        return Ok(Dish::Text(String::new()));
    }
    let d = decode(text, args.str(0))?;
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    Ok(Dish::Text(match args.str(1) {
        "Raw" => d.data.iter().map(|&b| b as char).collect(),
        "Bitcoin scriptPubKey" => match d.witness_version {
            // [OP_n][length][program]; OP_0 is 0x00, OP_1-OP_16 0x51-0x60.
            Some(_) if d.data.len() >= 2 => {
                let v = d.data[0];
                let op = if v == 0 { 0 } else { 0x50 + v };
                let program = &d.data[1..];
                hex(&[&[op, program.len() as u8], program].concat())
            }
            _ => hex(&d.data),
        },
        "HRP: Hex" => format!("{}: {}", d.hrp, hex(&d.data)),
        "JSON" => {
            let v =
                serde_json::json!({ "hrp": d.hrp, "encoding": d.encoding, "data": hex(&d.data) });
            serde_json::to_string_pretty(&v).unwrap_or_default()
        }
        _ => hex(&d.data),
    }))
}

/// CyberChef's `Bech32.decode`.
fn decode(s: &str, encoding: &str) -> Result<Decoded, OpError> {
    let units = utf16(s).len();
    if units > 90 {
        return Err(OpError::new(format!(
            "Invalid Bech32 string: exceeds maximum length of 90 characters (got {units})."
        )));
    }
    if s.bytes().any(|b| b.is_ascii_uppercase()) && s.bytes().any(|b| b.is_ascii_lowercase()) {
        return Err(OpError::new(
            "Invalid Bech32 string: mixed case is not allowed. Use all uppercase or all lowercase.",
        ));
    }
    let lower = utf16(&s.to_lowercase());
    let Some(sep) = lower.iter().rposition(|&c| c == u16::from(b'1')) else {
        return Err(OpError::new(
            "Invalid Bech32 string: no separator '1' found.",
        ));
    };
    if sep == 0 {
        return Err(OpError::new(
            "Invalid Bech32 string: Human-Readable Part (HRP) cannot be empty.",
        ));
    }
    if sep + 7 > lower.len() {
        return Err(OpError::new(
            "Invalid Bech32 string: data part is too short (minimum 6 characters for checksum).",
        ));
    }
    if let Some(i) = lower[..sep].iter().position(|&c| !(33..=126).contains(&c)) {
        return Err(OpError::new(format!(
            "HRP contains invalid character at position {i}."
        )));
    }
    for (i, &c) in lower[sep + 1..].iter().enumerate() {
        if char::from_u32(u32::from(c)).is_none_or(|ch| Fe32::from_char(ch).is_err()) {
            return Err(OpError::new(format!(
                "Invalid character '{}' at position {}.",
                from_utf16(&[c]),
                sep + 1 + i
            )));
        }
    }
    // All ASCII now: the checks above leave only HRP and charset characters.
    let lower = from_utf16(&lower);
    let unchecked = UncheckedHrpstring::new(&lower).map_err(|e| OpError::new(e.to_string()))?;
    let (encoding, bech32m) = match encoding {
        "Bech32" if unchecked.has_valid_checksum::<Bech32>() => ("Bech32", false),
        "Bech32" => return Err(OpError::new("Invalid Bech32 checksum.")),
        "Bech32m" if unchecked.has_valid_checksum::<Bech32m>() => ("Bech32m", true),
        "Bech32m" => return Err(OpError::new("Invalid Bech32m checksum.")),
        _ if unchecked.has_valid_checksum::<Bech32>() => ("Bech32", false),
        _ if unchecked.has_valid_checksum::<Bech32m>() => ("Bech32m", true),
        _ => {
            return Err(OpError::new(
                "Invalid Bech32/Bech32m string: checksum verification failed.",
            ));
        }
    };
    // Both checksums are six words, so either removes the same characters.
    let checked = || {
        if bech32m {
            UncheckedHrpstring::new(&lower).map(|u| u.remove_checksum::<Bech32m>())
        } else {
            UncheckedHrpstring::new(&lower).map(|u| u.remove_checksum::<Bech32>())
        }
        .map_err(|e| OpError::new(e.to_string()))
    };
    let hrp = unchecked.hrp().to_lowercase();
    let could_be_segwit = SEGWIT_HRPS.contains(&hrp.as_str());
    let mut program = checked()?;
    let segwit = match program.remove_witness_version() {
        Some(version) if could_be_segwit => words_to_bytes(&program)
            .ok()
            .filter(|p| {
                // BIP-0350: the checksum must match the version.
                (version.to_u8() == 0) != bech32m
                    && bech32::primitives::segwit::validate_witness_program_length(p.len(), version)
                        .is_ok()
            })
            .map(|p| (version.to_u8(), p)),
        _ => None,
    };
    let (bytes, witness_version) = match segwit {
        Some((version, p)) => ([vec![version], p].concat(), Some(version)),
        // Not a valid witness program: decode every word instead.
        None => (
            words_to_bytes(&checked()?)
                .map_err(|e| OpError::new(format!("Failed to decode data: {}", e.0)))?,
            None,
        ),
    };
    Ok(Decoded {
        hrp,
        data: bytes,
        encoding,
        witness_version,
    })
}

/// The data words as bytes; the padding must be under 5 bits and all zero.
fn words_to_bytes(checked: &CheckedHrpstring) -> Result<Vec<u8>, OpError> {
    checked.validate_segwit_padding().map_err(|e| {
        OpError::new(match e {
            PaddingError::TooMuch => "Invalid padding: too many bits remaining",
            _ => "Invalid padding: non-zero bits in padding",
        })
    })?;
    Ok(checked.byte_iter().collect())
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

    // BIP-0173 / BIP-0350 test addresses.
    #[test]
    fn segwit_addresses() {
        let program = "751e76e8199196d454941c45d1b3a323f1433bd6";
        let args = [
            s("bc"),
            s("Bech32"),
            s("Hex"),
            s("Bitcoin SegWit"),
            ArgValue::Num(0.0),
        ];
        assert_eq!(
            run(&TO_BECH32, program, &args),
            "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"
        );
        assert_eq!(
            run(
                &FROM_BECH32,
                "BC1QW508D6QEJXTDG4Y5R3ZARVARY0C5XW7KV8F3T4",
                &[s("Auto-detect"), s("Bitcoin scriptPubKey")]
            ),
            "0014751e76e8199196d454941c45d1b3a323f1433bd6"
        );
        assert_eq!(
            run(
                &FROM_BECH32,
                "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4",
                &[s("Bech32m"), s("Hex")]
            ),
            "Invalid Bech32m checksum."
        );
        // BIP-0350 test vector: a version-1 program with Bech32m.
        let v1 = [
            s("bc"),
            s("Bech32m"),
            s("Hex"),
            s("Bitcoin SegWit"),
            ArgValue::Num(1.0),
        ];
        let p1 = "751e76e8199196d454941c45d1b3a323f1433bd6751e76e8199196d454941c45d1b3a323f1433bd6";
        assert_eq!(
            run(&TO_BECH32, p1, &v1),
            "bc1pw508d6qejxtdg4y5r3zarvary0c5xw7kw508d6qejxtdg4y5r3zarvary0c5xw7kt5nd6y"
        );
        let mut v1_bech32 = v1.clone();
        v1_bech32[1] = s("Bech32");
        assert_eq!(
            run(&TO_BECH32, p1, &v1_bech32),
            "Witness version 1 requires Bech32m."
        );
        assert_eq!(
            run(&TO_BECH32, "", &args),
            "Invalid witness program length: 0. Must be 2-40 bytes."
        );
    }
}
