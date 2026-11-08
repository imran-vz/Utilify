//! Consistent Overhead Byte Stuffing: frames without zero bytes, through
//! `utilify-cobs`, per S. Cheshire and M. Baker, "Consistent Overhead Byte
//! Stuffing", IEEE/ACM Transactions on Networking 7(2), 1999. The packet
//! is encoded without a frame delimiter (the `0x00` that would follow it).
//!
//! - Encoding (§III.B): a phantom zero is appended to every packet, so the
//!   empty packet encodes as the single code byte `01`.
//! - Decoding (§III.A, Table 1; Listing 2) is the library's strict mode:
//!   empty data (every encoding is at least one byte), a `00` byte, or a
//!   code block that runs past the end of the data is an error.

use super::CATEGORY;
use crate::{Dish, OpError, Operation};
use utilify_cobs::{ErrorKind, Options};

pub(super) fn ops() -> Vec<&'static Operation> {
    vec![&TO_COBS, &FROM_COBS]
}

pub static TO_COBS: Operation = Operation {
    name: "To COBS",
    category: CATEGORY,
    description: "Encodes bytes in COBS format",
    info_url: Some("https://wikipedia.org/wiki/Consistent_Overhead_Byte_Stuffing"),
    args: &[],
    run: |input, _| Ok(Dish::Bytes(to_cobs(&input.bytes()))),
};

pub static FROM_COBS: Operation = Operation {
    name: "From COBS",
    category: CATEGORY,
    description: "Decodes COBS encoded bytes",
    info_url: Some("https://wikipedia.org/wiki/Consistent_Overhead_Byte_Stuffing"),
    args: &[],
    run: |input, _| from_cobs(&input.bytes()).map(Dish::Bytes),
};

/// COBS-encodes one packet, without a delimiter.
pub fn to_cobs(data: &[u8]) -> Vec<u8> {
    utilify_cobs::encode(data)
}

/// Decodes one COBS-encoded packet, strictly.
pub fn from_cobs(data: &[u8]) -> Result<Vec<u8>, OpError> {
    utilify_cobs::decode_with(data, &Options::STANDARD).map_err(|e| match e.kind() {
        ErrorKind::ZeroByte => {
            OpError::new("Could not decode from COBS: payload must not contain a 0x00 byte")
        }
        _ => OpError::new(format!("Could not decode from COBS: {e}")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_per_the_paper() {
        // §III.B: the phantom zero makes the empty packet one code byte.
        assert_eq!(to_cobs(&[]), [1]);
        // Fig. 3.
        assert_eq!(
            to_cobs(&[0x45, 0, 0, 0x2c, 0x4c, 0x79, 0, 0, 0x40, 0x06, 0x4f, 0x37]),
            [
                2, 0x45, 1, 4, 0x2c, 0x4c, 0x79, 1, 5, 0x40, 0x06, 0x4f, 0x37
            ]
        );
        assert_eq!(to_cobs(&[0]), [1, 1]);
        assert_eq!(to_cobs(&[0, 0]), [1, 1, 1]);
        assert_eq!(to_cobs(&[0x11, 0x22, 0, 0x33]), [3, 0x11, 0x22, 2, 0x33]);
        let run: Vec<u8> = (1..=254).collect();
        let mut want = vec![255];
        want.extend(&run);
        assert_eq!(to_cobs(&run), want);
    }

    #[test]
    fn decodes_strictly() {
        assert_eq!(
            from_cobs(&[3, 0x11, 0x22, 2, 0x33]).unwrap(),
            [0x11, 0x22, 0, 0x33]
        );
        assert_eq!(from_cobs(&[1, 1, 1]).unwrap(), [0, 0]);
        assert_eq!(from_cobs(&[1]).unwrap(), Vec::<u8>::new());
        assert!(from_cobs(&[1, 0]).is_err());
        // A block that claims more bytes than follow, and empty data.
        assert!(from_cobs(&[5, 0x11]).is_err());
        assert!(from_cobs(&[]).is_err());
    }
}
