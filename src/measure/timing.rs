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

/// Both programs timed on one workload, block by block in the order of [`schedule`], and the
/// peak memory of each in its output check.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Timed {
    pub blocks: Vec<Block>,
    pub peak_memory: PeakMemory,
}

/// Each program's peak resident memory in bytes during its output check, as GNU time read it
/// from the kernel. `None` when GNU time reported none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeakMemory {
    pub reference: Option<u64>,
    pub candidate: Option<u64>,
}

/// `12.5MiB (reference 48.0MiB)`, the candidate's first, with `-` for a peak not reported.
impl fmt::Display for PeakMemory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mebibytes = |bytes: Option<u64>| {
            bytes.map_or_else(
                || "-".to_owned(),
                |bytes| format!("{:.1}MiB", bytes as f64 / f64::from(1 << 20)),
            )
        };
        write!(
            f,
            "{} (reference {})",
            mebibytes(self.candidate),
            mebibytes(self.reference)
        )
    }
}

impl Timed {
    pub fn summary(&self) -> Summary {
        let samples = |side| self.blocks.iter().flat_map(move |block| block.side(side));
        let walls = |side| samples(side).map(|sample| sample.wall);
        let instructions = |side| {
            let counts: Option<Vec<f64>> = samples(side)
                .map(|sample| sample.instructions.map(|count| count as f64))
                .collect();
            counts.filter(|counts| !counts.is_empty()).map(median)
        };
        let logs: Vec<f64> = self.blocks.iter().map(Block::log_ratio).collect();
        Summary {
            reference: Spread::of(walls(Side::Reference)),
            candidate: Spread::of(walls(Side::Candidate)),
            ratio: Ratio::of(&logs),
            instructions: instructions(Side::Candidate)
                .zip(instructions(Side::Reference))
                .map(|(candidate, reference)| candidate / reference),
            peak_memory: self.peak_memory,
        }
    }
}

/// Each program's wall times over every block, how many times longer the candidate takes, and
/// how many times more instructions it retires.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Summary {
    pub reference: Spread,
    pub candidate: Spread,
    pub ratio: Ratio,
    /// The candidate's median user-space instruction count over the reference's, pooling every
    /// timed run. `None` unless the kernel let hyperfine count every run of both.
    pub instructions: Option<f64>,
    pub peak_memory: PeakMemory,
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

    fn timed(blocks: Vec<Block>) -> Timed {
        Timed {
            blocks,
            peak_memory: PeakMemory {
                reference: Some(4 << 20),
                candidate: None,
            },
        }
    }

    #[test]
    fn the_summary_pools_runs_per_side_and_ratios_per_block() {
        let timed = timed(vec![
            block(&[1.0, 1.5], &[2.0, 3.0]),
            block(&[1.25, 0.75], &[2.5, 3.5]),
        ]);
        let summary = timed.summary();
        assert_eq!(summary.reference, Spread::of([0.75, 1.0, 1.25, 1.5]));
        assert_eq!(summary.candidate, Spread::of([2.0, 2.5, 3.0, 3.5]));
        assert_eq!(summary.ratio, Ratio::of(&[2f64.ln(), 3f64.ln()]));
        assert_eq!(summary.instructions, None);
        assert_eq!(summary.peak_memory, timed.peak_memory);
    }

    #[test]
    fn the_instruction_ratio_compares_pooled_medians_and_needs_every_count() {
        let counted = |counts: &[u64]| -> Vec<Sample> {
            counts
                .iter()
                .map(|&count| Sample {
                    instructions: Some(count),
                    ..runs(&[1.0])[0]
                })
                .collect()
        };
        let mut timed = timed(vec![
            Block {
                reference: counted(&[100, 900]),
                candidate: counted(&[150, 160]),
            },
            Block {
                reference: counted(&[100, 100]),
                candidate: counted(&[150, 140]),
            },
        ]);
        assert_eq!(timed.summary().instructions, Some(1.5));

        timed.blocks[1].candidate[0].instructions = None;
        assert_eq!(timed.summary().instructions, None);
    }

    #[test]
    fn timed_runs_read_back_from_json() {
        let timed = timed(vec![block(&[0.5], &[0.25])]);
        let json = serde_json::to_string(&timed).unwrap();
        assert!(
            json.starts_with("{\"blocks\":[{\"reference\":[{\"wall\":0.5,"),
            "{json}"
        );
        assert_eq!(serde_json::from_str::<Timed>(&json).unwrap(), timed);
    }
}
