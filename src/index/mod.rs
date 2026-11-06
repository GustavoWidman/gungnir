//! The gungnir index: one score per language for how well coding agents do in it, against the
//! average language for the same model and scenario. 100 is an average language and higher is
//! better.
//!
//! Each session counts on five dimensions: whether its program passed, its tokens, its wall
//! time, and, for a passing program, its speed against the fastest reference and its total
//! cyclomatic complexity. In each cell (model, scenario, language) a dimension's value is the
//! logarithm of the smoothed pass rate `(k + 1/2) / (n + 1)`, or the mean logarithm of the others
//! over the sessions or passing programs that have them. A language's advantage is its value
//! minus the mean over the languages of the same model and scenario, negated for every dimension
//! but the pass rate, and averaged over the scenarios where the cell exists. A dimension's score
//! is 100 times the exponential of the mean advantage over the models that have it, a model's
//! index 100 times the exponential of the weighted mean of its advantages over the dimensions,
//! and the index 100 times the exponential of the weighted mean of the dimensions' means over
//! models. A two-stage bootstrap, scenarios and then sessions within each cell, gives every
//! score its interval.
mod read;
mod score;
#[cfg(test)]
mod tests;
mod write;

pub use read::{Generation, Read, read};
pub use score::{Bootstrap, Card, Estimate, Sample, Scores, Timing, score};
pub use write::write;

use eyre::{Report, Result, ensure, eyre};
use serde::Serialize;
use std::{collections::BTreeMap, fmt, str::FromStr};

/// One dimension the index weighs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Dimension {
    /// Whether the program passed every hidden case.
    Pass,
    /// The session's tokens in every bucket: input, cache reads, writes to either cache and
    /// output. A session that reported none lacks it.
    Tokens,
    /// The session's wall time.
    Time,
    /// A passing program's median wall time over the fastest reference's, the geometric mean
    /// over the workloads its measurement timed.
    Speed,
    /// A passing program's total cyclomatic complexity, at least 1.
    Simplicity,
}

impl Dimension {
    /// Every dimension, in the order tables list them.
    pub const ALL: [Self; 5] = [
        Self::Pass,
        Self::Tokens,
        Self::Time,
        Self::Speed,
        Self::Simplicity,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Tokens => "tokens",
            Self::Time => "time",
            Self::Speed => "speed",
            Self::Simplicity => "simplicity",
        }
    }

    /// Its position in [`Dimension::ALL`].
    fn position(self) -> usize {
        self as usize
    }

    /// Whether a larger cell value is better. Only the pass rate's is; the others are costs.
    fn larger_is_better(self) -> bool {
        self == Self::Pass
    }

    /// Whether only passing programs have the dimension.
    fn passing_only(self) -> bool {
        matches!(self, Self::Speed | Self::Simplicity)
    }
}

impl fmt::Display for Dimension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Dimension {
    type Err = Report;

    fn from_str(name: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|dimension| dimension.as_str() == name)
            .ok_or_else(|| {
                let names: Vec<&str> = Self::ALL.iter().map(|d| d.as_str()).collect();
                eyre!(
                    "{name:?} is not a dimension; expected one of {}",
                    names.join(", ")
                )
            })
    }
}

/// How much each dimension counts in an index against the others. Every dimension weighs 1
/// unless set, a weight is finite and not negative, and at least one is positive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Weights([f64; Dimension::ALL.len()]);

impl Default for Weights {
    fn default() -> Self {
        Self([1.0; Dimension::ALL.len()])
    }
}

impl Weights {
    /// Every dimension at 1 but those `set` names. Fails on a dimension named twice, a weight
    /// that is negative or not finite, or weights that are all zero.
    pub fn new(set: impl IntoIterator<Item = (Dimension, f64)>) -> Result<Self> {
        let mut weights = Self::default();
        let mut named = Vec::new();
        for (dimension, weight) in set {
            ensure!(
                !named.contains(&dimension),
                "dimension {dimension} is weighed twice"
            );
            ensure!(
                weight.is_finite() && weight >= 0.0,
                "a weight is a number of at least 0, got {weight} for {dimension}"
            );
            named.push(dimension);
            weights.0[dimension.position()] = weight;
        }
        ensure!(
            weights.0.iter().any(|&weight| weight > 0.0),
            "every dimension weighs 0"
        );
        Ok(weights)
    }

    pub fn of(&self, dimension: Dimension) -> f64 {
        self.0[dimension.position()]
    }

    /// Each dimension's weight, by name.
    pub fn by_name(&self) -> BTreeMap<&'static str, f64> {
        Dimension::ALL
            .into_iter()
            .map(|dimension| (dimension.as_str(), self.of(dimension)))
            .collect()
    }
}
