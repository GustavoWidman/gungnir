//! The report: every current record of one or more experiments folded into tidy CSV tables
//! for analysis in Python or R.
mod price;

pub use price::{Cost, PriceList, Usd};
