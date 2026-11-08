//! The operations, one module per CyberChef category.

use crate::Operation;

pub mod data_format;

pub(crate) fn all() -> Vec<&'static Operation> {
    let mut ops = vec![];
    ops.extend(data_format::ops());
    ops
}
