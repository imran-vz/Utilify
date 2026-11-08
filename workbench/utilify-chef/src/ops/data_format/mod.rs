//! CyberChef's "Data format" category: encodings and representations of
//! data (hex, Base64 and friends, character codes, text encodings, and
//! structured formats like CBOR and YAML).

use crate::Operation;

pub const CATEGORY: &str = "Data format";

mod basen;
mod hex;
pub(crate) mod numeric;
mod structured;
mod text;

pub(crate) fn ops() -> Vec<&'static Operation> {
    [
        hex::ops(),
        numeric::ops(),
        basen::ops(),
        text::ops(),
        structured::ops(),
    ]
    .concat()
}
