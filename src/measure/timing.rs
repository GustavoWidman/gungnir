//! What timing two programs on one workload yields: runs grouped into ABBA blocks, and the
//! statistics derived from them.
use super::stats::{Ratio, Spread, median};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Which of the two compared programs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Side {
    Reference,
    Candidate,
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Reference => "reference",
            Self::Candidate => "candidate",
        })
    }
}

/// The order each of `blocks` blocks times the two programs in. Consecutive blocks mirror
/// each other, reference first and then candidate first (ABBA), so a drift in the machine's
/// speed favours neither program. An even count puts each program first equally often.
pub fn schedule(blocks: u32) -> impl Iterator<Item = [Side; 2]> {
    (0..blocks).map(|block| {
        if block % 2 == 0 {
            [Side::Reference, Side::Candidate]
        } else {
            [Side::Candidate, Side::Reference]
        }
    })
}

/// One timed run, as hyperfine measured it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    /// Seconds from start to exit.
    pub wall: f64,
    /// CPU seconds in user mode.
    pub user: f64,
    /// CPU seconds in the kernel.
    pub system: f64,
    /// Peak resident memory in bytes.
    pub peak_memory: u64,
    /// Instructions retired, when the kernel let hyperfine count them.
    pub instructions: Option<u64>,
}

/// The timed runs of each program in one block, which ran back to back.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub reference: Vec<Sample>,
    pub candidate: Vec<Sample>,
}

impl Block {
    pub fn side(&self, side: Side) -> &[Sample] {
        match side {
            Side::Reference => &self.reference,
            Side::Candidate => &self.candidate,
        }
    }

    pub(super) fn side_mut(&mut self, side: Side) -> &mut Vec<Sample> {
        match side {
            Side::Reference => &mut self.reference,
            Side::Candidate => &mut self.candidate,
        }
    }

    /// The natural log of the candidate's median wall time over the reference's.
    pub fn log_ratio(&self) -> f64 {
        let wall = |side| median(self.side(side).iter().map(|sample| sample.wall));
        (wall(Side::Candidate) / wall(Side::Reference)).ln()
    }
}

/// Both programs timed on one workload, block by block in the order of [`schedule`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timed {
    pub blocks: Vec<Block>,
}

impl Timed {
    pub fn summary(&self) -> Summary {
        let walls = |side| {
            let samples = self.blocks.iter().flat_map(move |block| block.side(side));
            samples.map(|sample| sample.wall)
        };
        let logs: Vec<f64> = self.blocks.iter().map(Block::log_ratio).collect();
        Summary {
            reference: Spread::of(walls(Side::Reference)),
            candidate: Spread::of(walls(Side::Candidate)),
            ratio: Ratio::of(&logs),
        }
    }
}

/// Each program's wall times over every block, and how many times longer the candidate takes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Summary {
    pub reference: Spread,
    pub candidate: Spread,
    pub ratio: Ratio,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs(walls: &[f64]) -> Vec<Sample> {
        walls
            .iter()
            .map(|&wall| Sample {
                wall,
                user: wall,
                system: 0.0,
                peak_memory: 1 << 20,
                instructions: None,
            })
            .collect()
    }

    fn block(reference: &[f64], candidate: &[f64]) -> Block {
        Block {
            reference: runs(reference),
            candidate: runs(candidate),
        }
    }

    #[test]
    fn blocks_alternate_which_program_goes_first() {
        let sequence: Vec<Side> = schedule(4).flatten().collect();
        let (a, b) = (Side::Reference, Side::Candidate);
        assert_eq!(sequence, [a, b, b, a, a, b, b, a]);
        assert_eq!(schedule(1).collect::<Vec<_>>(), [[a, b]]);
        assert_eq!(schedule(0).count(), 0);
    }

    #[test]
    fn a_block_ratio_compares_medians_not_means() {
        let outlier = block(&[1.0, 1.0, 9.0], &[2.0, 2.0, 2.0]);
        assert!((outlier.log_ratio() - 2f64.ln()).abs() < 1e-12);
    }

    #[test]
    fn the_summary_pools_runs_per_side_and_ratios_per_block() {
        let timed = Timed {
            blocks: vec![
                block(&[1.0, 1.5], &[2.0, 3.0]),
                block(&[1.25, 0.75], &[2.5, 3.5]),
            ],
        };
        let summary = timed.summary();
        assert_eq!(summary.reference, Spread::of([0.75, 1.0, 1.25, 1.5]));
        assert_eq!(summary.candidate, Spread::of([2.0, 2.5, 3.0, 3.5]));
        assert_eq!(summary.ratio, Ratio::of(&[2f64.ln(), 3f64.ln()]));
    }

    #[test]
    fn timed_runs_read_back_from_json() {
        let timed = Timed {
            blocks: vec![block(&[0.5], &[0.25])],
        };
        let json = serde_json::to_string(&timed).unwrap();
        assert!(
            json.starts_with("[{\"reference\":[{\"wall\":0.5,"),
            "{json}"
        );
        assert_eq!(serde_json::from_str::<Timed>(&json).unwrap(), timed);
    }
}
