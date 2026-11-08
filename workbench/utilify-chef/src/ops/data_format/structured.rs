//! Structured data formats: JSON-shaped values to and from CSV, YAML,
//! Rison, CBOR, MessagePack, AMF and Avro, plus TLV, COBS and ASN.1 dumps.
//! `js` models the JavaScript values CyberChef passes between them.

use super::CATEGORY;
use crate::Operation;

mod amf;
mod asn1;
mod avro;
mod cbor;
mod cobs;
mod csv;
mod js;
mod msgpack;
mod rison;
mod tlv;
mod yaml;

pub(super) fn ops() -> Vec<&'static Operation> {
    [
        amf::ops(),
        asn1::ops(),
        avro::ops(),
        cbor::ops(),
        cobs::ops(),
        csv::ops(),
        msgpack::ops(),
        rison::ops(),
        tlv::ops(),
        yaml::ops(),
    ]
    .concat()
}
