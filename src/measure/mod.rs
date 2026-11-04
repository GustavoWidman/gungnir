//! Measuring. A program that passed grading is timed against the reference solution in its
//! language. On each workload the candidate must first print exactly what the reference
//! prints; then hyperfine times both in ABBA blocks, in offline sandboxes pinned to one core.
mod hyperfine;
mod machine;
mod phase;
mod profile;
mod record;
mod rusage;
mod stats;
#[cfg(test)]
mod tests;
mod timing;

pub use machine::{Identity, Load, Machine, isolated_cpus};
pub use phase::{Measured, Quiet, Request, Selection, measure_trials};
pub use profile::{Profiled, profile_trial};
pub use record::{MeasureInputs, MeasureRecord};
pub use stats::{Ratio, Spread};
pub use timing::{Block, PeakMemory, Sample, Side, Summary, Timed, schedule};

use crate::{
    grade::{Abnormal, BuildOutcome, Crash, Offline, tree},
    id::WorkloadId,
    language::Language,
    sandbox::{Backend, Finished, Input, Io, Kill, Limits, Mount, Output, Spec},
    scenario::Workload,
    stop::{Stage, Stop},
    store::write_atomic,
    toolchains::{Closure, Toolchain, Toolchains},
};
use eyre::{Result, WrapErr, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, ffi::OsString, fmt, num::NonZeroU32, path::Path, time::Duration};
use tempfile::TempDir;

/// Where the workload appears in every sandbox that runs a program on it.
const WORKLOAD: &str = "/workload";
/// Where GNU time writes its report in the sandbox of an output check.
const RUSAGE: &str = "/rusage";
/// The longest one run of a program may take.
const RUN_TIMEOUT: Duration = Duration::from_secs(60);
/// Memory for hyperfine itself, on top of the program it times.
const TIMER_MEMORY: u64 = 64 << 20;
const TASKS: u32 = 256;
/// How much stdout the output check keeps. A candidate that prints more fails the workload.
const OUTPUT_CAP: u64 = 64 << 20;
/// hyperfine's export takes about a kilobyte per run.
const EXPORT_CAP: u64 = 16 << 20;
const STDERR_CAP: u64 = 16 << 10;
/// How much of a failed build's log the error quotes.
const LOG_TAIL: usize = 4 << 10;

/// How a comparison runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// The CPU that every run of either program is pinned to.
    pub core: u32,
    pub protocol: Protocol,
    /// The memory one run of a program may use, in bytes.
    pub memory: u64,
}

/// How many runs a comparison times. A measurement records its protocol among its inputs, so
/// measuring with another one makes it stale; the CPU it was pinned to is only provenance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Protocol {
    /// ABBA blocks per workload. Each block times both programs once.
    pub blocks: NonZeroU32,
    /// hyperfine's timed runs of one program in one block.
    pub runs: NonZeroU32,
    /// hyperfine's untimed runs before them.
    pub warmup: u32,
}

impl Protocol {
    /// The protocol `gungnir measure` and `gungnir bench` use unless told otherwise, chosen
    /// from A/A comparisons of all six pilot references (8 blocks of 5 runs each). A program's
    /// first run in a block was no slower than the rest (median ratio 0.98), since the output
    /// check has already run it on the workload, so a warmup adds runs and no accuracy. The
    /// median of 3 runs survives one outlier and took the block-to-block spread most of the
    /// way from 1 run to 5 (17.0% to 12.5%, against 11.0%). Six blocks cut the t quantile
    /// from 3.18 at four to 2.57, and a 95% interval to about two thirds as wide.
    pub const DEFAULT: Self = Self {
        blocks: NonZeroU32::new(6).unwrap(),
        runs: NonZeroU32::new(3).unwrap(),
        warmup: 0,
    };
}

/// A workspace built offline the way grading builds it, ready to run on workloads.
pub struct Program<'a> {
    offline: Offline<'a>,
    /// Holds the built copy for as long as the program exists.
    _scratch: TempDir,
    measure: Toolchain,
    run: Vec<String>,
}

impl<'a> Program<'a> {
    /// Copies `workspace` without its build output into fresh scratch and builds the copy
    /// offline. A build that does not succeed is an error quoting the end of its log.
    pub async fn build(
        language: &Language,
        toolchains: &Toolchains,
        workspace: &Path,
        backend: &'a dyn Backend,
        stop: &'a Stop,
    ) -> Result<Self> {
        Self::build_with(
            language,
            toolchains,
            workspace,
            &BTreeMap::new(),
            backend,
            stop,
        )
        .await
    }

    /// The same, building with `env` set on top of the language's variables.
    pub async fn build_with(
        language: &Language,
        toolchains: &Toolchains,
        workspace: &Path,
        env: &BTreeMap<String, String>,
        backend: &'a dyn Backend,
        stop: &'a Stop,
    ) -> Result<Self> {
        let scratch = tempfile::Builder::new()
            .prefix("gungnir-measure-")
            .tempdir()
            .wrap_err("creating measurement scratch")?;
        let work = scratch.path().join("work");
        tree::copy(workspace, &work, language.ignore())?;
        let offline = Offline::new(language, toolchains, work, backend, stop)?.with_env(env);
        let (build, log) = offline.build(language.build()).await?;
        if build.outcome != BuildOutcome::Built {
            let tail = String::from_utf8_lossy(&log[log.len().saturating_sub(LOG_TAIL)..]);
            bail!(
                "{} did not build ({:?}):\n{}",
                workspace.display(),
                build.outcome,
                tail.trim_end()
            );
        }
        Ok(Self {
            offline,
            _scratch: scratch,
            measure: toolchains.measure().clone(),
            run: language.run().args().to_vec(),
        })
    }

    /// A read-only sandbox over the built copy and `workload`, with the measurement tools
    /// beside the language's.
    fn spec(&self, workload: &Path, limits: Limits) -> Spec {
        let mut spec = self.offline.spec(false, limits);
        spec.closure = Closure::union([&spec.closure, self.measure.closure()]);
        spec.path.push(self.measure.bin());
        spec.mounts.push(Mount::read_only(workload, WORKLOAD));
        spec
    }
}

/// What timing a candidate against its reference found on each workload, and the host it ran
/// on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Comparison {
    pub machine: Machine,
    pub workloads: BTreeMap<WorkloadId, Outcome>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// Both programs were timed in every block.
    Timed(Timed),
    /// The candidate was not timed to the end, for this reason.
    CandidateFailed(Failure),
}

/// Why a candidate was not timed on a workload.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Failure {
    /// Its check run exited 0 but printed other bytes than the reference's.
    WrongOutput,
    /// Its check run printed more than the check keeps.
    OutputOverCap,
    /// A timed run failed after the check run passed: what hyperfine said.
    TimedRun(String),
    /// Its check run ended abnormally.
    #[serde(untagged)]
    Abnormal(Abnormal),
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongOutput => f.write_str("wrong output"),
            Self::OutputOverCap => f.write_str("output over the cap"),
            Self::TimedRun(why) => write!(f, "failed a timed run: {why}"),
            Self::Abnormal(Abnormal::TimedOut) => f.write_str("timed out"),
            Self::Abnormal(Abnormal::OutOfMemory) => f.write_str("out of memory"),
            Self::Abnormal(Abnormal::Crashed(Crash::Code(code))) => write!(f, "exited {code}"),
            Self::Abnormal(Abnormal::Crashed(Crash::Signal(signal))) => {
                write!(f, "killed by signal {signal}")
            }
        }
    }
}

/// Times `candidate` against `reference` on every workload and writes hyperfine's export of
/// each timing as `hyperfine/<workload>/<block>-<side>.json` under `evidence`.
///
/// On each workload both programs first run once under GNU time, which reports their peak
/// resident memory, and a candidate that does not print exactly what the reference prints
/// fails the workload untimed. An error means there is no comparison: the reference failed, a
/// sandbox could not run, the host failed, or the second Ctrl-C interrupted it. The first
/// Ctrl-C lets a comparison finish.
pub async fn compare(
    reference: &Program<'_>,
    candidate: &Program<'_>,
    workloads: &BTreeMap<WorkloadId, Workload>,
    settings: &Settings,
    backend: &dyn Backend,
    stop: &Stop,
    evidence: &Path,
) -> Result<Comparison> {
    let bench = Bench {
        reference,
        candidate,
        settings,
        backend,
        stop,
    };
    let load = machine::load_average()?;
    let mut outcomes = BTreeMap::new();
    for (id, workload) in workloads {
        let evidence = evidence.join("hyperfine").join(id.as_str());
        let outcome = bench
            .workload(&workload.path, &evidence)
            .await
            .wrap_err_with(|| format!("workload {id}"))?;
        outcomes.insert(id.clone(), outcome);
    }
    Ok(Comparison {
        machine: Machine::probe(settings.core, load)?,
        workloads: outcomes,
    })
}

struct Bench<'a> {
    reference: &'a Program<'a>,
    candidate: &'a Program<'a>,
    settings: &'a Settings,
    backend: &'a dyn Backend,
    stop: &'a Stop,
}

/// How one hyperfine invocation ended.
enum Timing {
    Timed(Vec<Sample>),
    /// A run of the program failed, so hyperfine stopped: why, in its words.
    Failed(String),
}

impl Bench<'_> {
    fn program(&self, side: Side) -> &Program<'_> {
        match side {
            Side::Reference => self.reference,
            Side::Candidate => self.candidate,
        }
    }

    async fn workload(&self, workload: &Path, evidence: &Path) -> Result<Outcome> {
        let (expected, reference_peak) = self.check(Side::Reference, workload).await?;
        if let Some(failure) = unusable(&expected)? {
            bail!("the reference failed its check run: {failure}");
        }
        let (actual, candidate_peak) = self.check(Side::Candidate, workload).await?;
        let failure = match unusable(&actual)? {
            None if actual.stdout.bytes != expected.stdout.bytes => Some(Failure::WrongOutput),
            unusable => unusable,
        };
        if let Some(failure) = failure {
            return Ok(Outcome::CandidateFailed(failure));
        }

        let mut blocks = Vec::new();
        for (index, order) in schedule(self.settings.protocol.blocks.get()).enumerate() {
            let mut block = Block::default();
            for side in order {
                let export = evidence.join(format!("{index}-{side}.json"));
                match self.time(side, workload, &export).await? {
                    Timing::Timed(samples) => *block.side_mut(side) = samples,
                    Timing::Failed(why) if side == Side::Candidate => {
                        return Ok(Outcome::CandidateFailed(Failure::TimedRun(why)));
                    }
                    Timing::Failed(why) => bail!("the reference failed a timed run: {why}"),
                }
            }
            blocks.push(block);
        }
        let peak_memory = PeakMemory {
            reference: reference_peak,
            candidate: candidate_peak,
        };
        Ok(Outcome::Timed(Timed {
            blocks,
            peak_memory,
        }))
    }

    /// Runs the `side` program once on `workload` under GNU time, pinned like a timed run,
    /// keeping its stdout. Also returns its peak resident memory, when GNU time reported it.
    async fn check(&self, side: Side, workload: &Path) -> Result<(Finished, Option<u64>)> {
        let program = self.program(side);
        let limits = Limits {
            wall: RUN_TIMEOUT,
            memory: self.settings.memory,
            tasks: TASKS,
            cpu: Some(self.settings.core),
        };
        let io = Io {
            stdin: Input::File(workload.to_owned()),
            stdout: Output::Capture { cap: OUTPUT_CAP },
            stderr: Output::Capture { cap: 0 },
            kill: Kill::default(),
        };
        let report = tempfile::Builder::new()
            .prefix("gungnir-rusage-")
            .tempdir()
            .wrap_err("creating a directory for GNU time's report")?;
        let mut spec = program.spec(workload, limits);
        spec.mounts.push(Mount::writable(report.path(), RUSAGE));
        let finished = self.exec(&spec, &rusage::argv(&program.run), io).await?;
        Ok((finished, rusage::peak(report.path())?))
    }

    /// Times the `side` program on `workload` with hyperfine and keeps its export at `export`.
    async fn time(&self, side: Side, workload: &Path, export: &Path) -> Result<Timing> {
        let (program, settings) = (self.program(side), self.settings);
        let protocol = settings.protocol;
        let limits = Limits {
            wall: RUN_TIMEOUT.saturating_mul(protocol.warmup.saturating_add(protocol.runs.get())),
            memory: settings.memory.saturating_add(TIMER_MEMORY),
            tasks: TASKS,
            cpu: Some(settings.core),
        };
        let io = Io {
            stdin: Input::Null,
            stdout: Output::Capture { cap: EXPORT_CAP },
            stderr: Output::Capture { cap: STDERR_CAP },
            kill: Kill::default(),
        };
        let argv = hyperfine::argv(&program.run, &protocol);
        let finished = self
            .exec(&program.spec(workload, limits), &argv, io)
            .await?;
        if let Some(ending) = Abnormal::of(finished.exit)? {
            let stderr = String::from_utf8_lossy(&finished.stderr.bytes);
            let why = format!("hyperfine {}: {}", Failure::Abnormal(ending), stderr.trim());
            return Ok(Timing::Failed(why));
        }
        write_atomic(export, &finished.stdout.bytes)?;
        let samples = hyperfine::samples(&finished.stdout.bytes, protocol.runs)
            .wrap_err_with(|| format!("reading {}", export.display()))?;
        Ok(Timing::Timed(samples))
    }

    async fn exec(&self, spec: &Spec, argv: &[OsString], io: Io) -> Result<Finished> {
        ensure!(
            self.stop.stage() < Stage::Killing,
            "interrupted before it finished"
        );
        Ok(self.backend.exec(spec, argv, io, self.stop).await?)
    }
}

/// Why a check run's output cannot be compared, when it cannot.
fn unusable(finished: &Finished) -> Result<Option<Failure>> {
    if let Some(ending) = Abnormal::of(finished.exit)? {
        return Ok(Some(Failure::Abnormal(ending)));
    }
    Ok(finished.stdout.truncated.then_some(Failure::OutputOverCap))
}
