//! Utilify's data workbench: a Rust port of CyberChef's operations, chained
//! into recipes. No UI and no gpui dependency.
//!
//! - [`Dish`]: the value passed between operations, with CyberChef's
//!   text/bytes conversions.
//! - [`Operation`] and [`args`]: what an operation is and what it takes.
//! - [`recipe`]: steps, baking, and CyberChef-compatible recipe JSON and
//!   chef format (`From_Base64('A-Za-z0-9+/=',true)`).
//! - [`ops`]: the operations, one module per CyberChef category.
//! - [`catalog`]: CyberChef's full category list, ported or not.
//! - [`session`]: the workbench state kept across launches.
//!
//! Operations are ported domain by domain; names, arguments and defaults
//! match CyberChef so its recipes load unchanged, and its test vectors run in
//! `tests/cyberchef_vectors.rs`.

pub mod args;
pub mod catalog;
pub mod dish;
pub mod ops;
pub mod recipe;
pub mod session;
pub mod util;

pub use args::{Arg, ArgKind, ArgValue, Args};
pub use dish::Dish;
pub use recipe::{Baked, Recipe, Step};

use std::fmt;

/// One operation: a pure function from a dish and its arguments to a dish.
pub struct Operation {
    /// CyberChef's name, which recipes refer to.
    pub name: &'static str,
    /// CyberChef's category, as in [`catalog::CATEGORIES`].
    pub category: &'static str,
    /// Plain-text description, shown in the workbench.
    pub description: &'static str,
    pub info_url: Option<&'static str>,
    pub args: &'static [Arg],
    pub run: fn(Dish, &Args) -> Result<Dish, OpError>,
}

impl Operation {
    pub fn default_args(&self) -> Vec<ArgValue> {
        self.args.iter().map(Arg::default_value).collect()
    }

    pub fn apply(&self, dish: Dish, values: &[ArgValue]) -> Result<Dish, OpError> {
        (self.run)(dish, &Args::prepare(self.args, values))
    }
}

impl fmt::Debug for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Operation")
            .field("name", &self.name)
            .finish()
    }
}

/// An operation's failure, shown in place of the output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpError(pub String);

impl OpError {
    pub fn new(msg: impl Into<String>) -> Self {
        OpError(msg.into())
    }
}

impl fmt::Display for OpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for OpError {}

/// Every ported operation, in catalog order.
pub fn operations() -> &'static [&'static Operation] {
    use std::sync::OnceLock;
    static ALL: OnceLock<Vec<&'static Operation>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut ops = ops::all();
        let rank = |name: &str| {
            catalog::CATEGORIES
                .iter()
                .flat_map(|(_, names)| names.iter())
                .position(|n| *n == name)
                .unwrap_or(usize::MAX)
        };
        ops.sort_by_key(|op| rank(op.name));
        ops
    })
}

pub fn find(name: &str) -> Option<&'static Operation> {
    operations().iter().copied().find(|op| op.name == name)
}

/// `Number.prototype.toString()`: shortest round-trip digits, plain
/// notation for exponents -7 to 20, exponential beyond (`1e+21`, `5e-324`),
/// as `ryu-js` writes it.
pub fn format_number(n: f64) -> String {
    ryu_js::Buffer::new().format(n).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_print_like_js() {
        assert_eq!(format_number(3.0), "3");
        assert_eq!(
            format_number(261007875622861540000.0),
            "261007875622861540000"
        );
        assert_eq!(format_number(-0.5), "-0.5");
        assert_eq!(format_number(1e21), "1e+21");
        assert_eq!(format_number(1.5e-7), "1.5e-7");
        assert_eq!(format_number(f64::NAN), "NaN");
        assert_eq!(format_number(-0.0), "0");
        assert_eq!(
            format_number(123456789012345677877.0),
            "123456789012345680000"
        );
    }

    #[test]
    fn ported_operations_are_in_the_catalog_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for op in operations() {
            assert!(seen.insert(op.name), "{} registered twice", op.name);
            let listed = catalog::CATEGORIES
                .iter()
                .find(|(cat, _)| *cat == op.category)
                .is_some_and(|(_, names)| names.contains(&op.name));
            assert!(listed, "{} is not in {}", op.name, op.category);
        }
    }
}
