//! Measuring. A program that passed grading is timed against the reference solution in its
//! language on every workload of its scenario.
mod stats;
mod timing;

pub use stats::{Ratio, Spread};
pub use timing::{Block, Sample, Side, Summary, Timed, schedule};
