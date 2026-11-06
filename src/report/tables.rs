//! The report's tables and the pure fold of one trial's records into their rows. Every row
//! names its trial by the same key columns, so the tables join on them.
use super::{
    csv::{Columns, encode, row},
    price::{Cost, PriceList, Usd},
};
use crate::{
    catalog::System,
    grade::{Abnormal, Complexity, Crash, GradeRecord, Outcome as CaseOutcome, Verdict},
    harness::TokenUsage,
    id::WorkloadId,
    measure::{self, MeasureRecord, Side},
    provenance::SessionId,
    run::{AgentExit, Cut, ProcessExit, RunRecord},
    trial::{Phase, TrialKey},
};
use chrono::{DateTime, Utc};
use eyre::Result;
use std::collections::BTreeMap;

/// One trial with a committed run, as the report reads it: its records in phase order. A
/// measurement is read only beside a passing grade.
pub struct Trial {
    pub key: TrialKey,
    pub run: RunRecord,
    pub grade: Option<GradeRecord>,
    pub measure: Option<MeasureRecord>,
    /// The first phase whose record was made from inputs other than the current ones.
    pub stale: Option<Phase>,
}

impl Trial {
    /// Every session that wrote one of the trial's records.
    pub fn sessions(&self) -> impl Iterator<Item = &SessionId> {
        [
            Some(self.run.session()),
            self.grade.as_ref().map(GradeRecord::session),
            self.measure.as_ref().map(MeasureRecord::session),
        ]
        .into_iter()
        .flatten()
    }

    /// The grade, when it is current. The lifecycle compares a grade's inputs only once the run
    /// is current, so beside a stale run it is not known to be.
    fn current_grade(&self) -> Option<&GradeRecord> {
        self.grade
            .as_ref()
            .filter(|_| !matches!(self.stale, Some(Phase::Run | Phase::Grade)))
    }

    /// The measurement, when it is current. The lifecycle compares a measurement's inputs only
    /// once the run and grade are current, so beside a stale one it is not known to be.
    fn current_measure(&self) -> Option<&MeasureRecord> {
        self.measure.as_ref().filter(|_| self.stale.is_none())
    }
}

row! {
    /// The trial a row belongs to.
    pub struct Key {
        pub experiment: String,
        pub system: String,
        pub scenario: String,
        pub language: String,
        pub rep: u32,
    }
}

row! {
    /// What the system id is made of.
    pub struct SystemSpec {
        pub harness: String,
        pub harness_version: String,
        pub backend: String,
        pub model: String,
        pub effort: &'static str,
        /// From 0 for `none` to 6 for `max`.
        pub effort_rank: u8,
    }
}

row! {
    /// Tokens in disjoint buckets. `reasoning_tokens` is the part of `output_tokens` spent
    /// reasoning, empty when the CLI does not report it.
    pub struct TokenColumns {
        pub input_tokens: u64,
        pub cache_read_tokens: u64,
        pub cache_write_tokens: u64,
        pub cache_write_1h_tokens: u64,
        pub output_tokens: u64,
        pub reasoning_tokens: Option<u64>,
    }
}

row! {
    /// The list price of some tokens.
    pub struct CostColumns {
        /// Empty when `unpriced_tokens` is not zero.
        pub cost_usd: Option<Usd>,
        /// Tokens with no listed rate.
        pub unpriced_tokens: u64,
        /// Whether some tokens were priced at the base tier because the prompt size of their
        /// requests is unknown.
        pub cost_base_tier_assumed: bool,
    }
}

row! {
    /// A trial whose committed run is a sample.
    pub struct TrialRow {
        pub key: Key,
        pub spec: SystemSpec,
        /// `ran`, `graded` or `measured`.
        pub stage: &'static str,
        pub stale_phase: Option<&'static str>,
        pub run_session: String,
        pub started: DateTime<Utc>,
        /// `completed`, or `refused` when the model declined the task.
        pub exit: &'static str,
        /// `code`, `signal`, `timed-out`, `out-of-memory` or `killed`.
        pub process_exit: &'static str,
        /// The exit code or signal number.
        pub process_status: Option<i32>,
        /// `idle` or `rate-limited` when gungnir ended the agent early.
        pub cut: Option<&'static str>,
        pub turns: Option<u32>,
        pub wall_s: f64,
        pub api_s: Option<f64>,
        pub tokens: TokenColumns,
        pub cost: CostColumns,
        pub grade_session: Option<String>,
        /// `pass` or `fail`.
        pub verdict: Option<&'static str>,
        pub failure: Option<&'static str>,
        /// Of the cases that ran, which is none when the build failed.
        pub cases_passed: Option<usize>,
        pub cases_total: Option<usize>,
        pub build_s: Option<f64>,
        /// The functions lizard found, with each file's top-level code as one more from
        /// complexity version 2.
        pub functions: Option<usize>,
        /// The sum of their cyclomatic complexities.
        pub ccn_total: Option<u64>,
        pub ccn_max: Option<u32>,
        pub nloc: Option<u64>,
        /// The version of the counting that measured the complexity. Version 1 counted only
        /// the code inside functions.
        pub complexity_version: Option<u32>,
        /// The version of the lizard that measured it, empty before version 2.
        pub lizard_version: Option<String>,
        pub measure_session: Option<String>,
        /// The one-minute load average when timing started and when it ended. Measuring
        /// refuses to start at 1.0 or more unless told the machine may be busy.
        pub measure_load_start: Option<f64>,
        pub measure_load_end: Option<f64>,
    }
}

row! {
    /// The first and last utilization the committed run saw of one quota window.
    pub struct QuotaRow {
        pub key: Key,
        pub window: String,
        pub first_used_fraction: f64,
        pub last_used_fraction: f64,
    }
}

row! {
    /// One usage sample of a trial's run.
    pub struct UsageRow {
        pub key: Key,
        /// From 1, in the order the CLI first reported each request.
        pub sample: usize,
        pub request: Option<String>,
        /// The model that served the tokens, when the CLI says.
        pub model: Option<String>,
        pub tokens: TokenColumns,
        pub cost: CostColumns,
    }
}

row! {
    /// One hidden case of a graded trial.
    pub struct CaseRow {
        pub key: Key,
        pub case: String,
        /// `pass`, or the failure class.
        pub outcome: &'static str,
        pub detail: Option<String>,
        pub wall_s: f64,
    }
}

row! {
    /// One function lizard found in a graded trial's sources.
    pub struct FunctionRow {
        pub key: Key,
        pub file: String,
        pub function: String,
        pub line: u32,
        pub ccn: u32,
        pub nloc: u32,
        pub lexical_tokens: u32,
        pub params: u32,
    }
}

row! {
    /// One workload of a measured trial.
    pub struct WorkloadRow {
        pub key: Key,
        pub workload: String,
        /// `timed`, or why the candidate was not timed.
        pub outcome: &'static str,
        pub detail: Option<String>,
        pub blocks: usize,
        /// The geometric mean over blocks of the candidate's median wall time over the
        /// reference's, with its 95% interval.
        pub wall_ratio: Option<f64>,
        pub wall_ratio_low: Option<f64>,
        pub wall_ratio_high: Option<f64>,
        pub reference_median_s: Option<f64>,
        pub reference_min_s: Option<f64>,
        pub reference_max_s: Option<f64>,
        pub candidate_median_s: Option<f64>,
        pub candidate_min_s: Option<f64>,
        pub candidate_max_s: Option<f64>,
        /// The candidate's median user-space instructions over the reference's, when every run
        /// of both counted them.
        pub instructions_ratio: Option<f64>,
        /// Each program's peak resident memory in its output check, as GNU time read it, when
        /// it did.
        pub reference_peak_memory_bytes: Option<u64>,
        pub candidate_peak_memory_bytes: Option<u64>,
    }
}

row! {
    /// One timed run of a measured trial.
    pub struct TimingRow {
        pub key: Key,
        pub workload: String,
        /// From 1, in the order the blocks ran.
        pub block: usize,
        pub side: &'static str,
        /// From 1 within the block and side.
        pub run: usize,
        pub wall_s: f64,
        pub user_s: f64,
        pub system_s: f64,
        /// hyperfine's figure, which never falls below its own footprint of about 3 MiB.
        pub peak_memory_bytes: u64,
        pub instructions: Option<u64>,
    }
}

/// Every table of the report.
#[derive(Debug, Default)]
pub struct Tables {
    trials: Vec<TrialRow>,
    quota: Vec<QuotaRow>,
    usage: Vec<UsageRow>,
    cases: Vec<CaseRow>,
    functions: Vec<FunctionRow>,
    workloads: Vec<WorkloadRow>,
    timings: Vec<TimingRow>,
}

impl Tables {
    /// Adds the rows of `trial` of `experiment`, pricing its tokens with `prices`.
    pub fn add(&mut self, experiment: &str, prices: &PriceList<'_>, trial: &Trial) -> Result<()> {
        let key = Key::of(experiment, &trial.key);
        let system = &trial.key.system;
        let outcome = trial.run.outcome();
        let costs = outcome
            .usage
            .iter()
            .map(|sample| prices.price(system, sample))
            .collect::<Result<Vec<Cost>>>()?;
        for (index, (sample, cost)) in outcome.usage.iter().zip(&costs).enumerate() {
            self.usage.push(UsageRow {
                key: key.clone(),
                sample: index + 1,
                request: sample.request.clone(),
                model: sample.model.clone(),
                tokens: TokenUsage::from(sample.tokens).into(),
                cost: (*cost).into(),
            });
        }
        let cost = costs.into_iter().sum::<Cost>().into();
        self.trials.push(TrialRow::of(
            &key,
            system,
            trial,
            outcome.total().into(),
            cost,
        ));
        self.quota
            .extend(outcome.quota.iter().map(|(window, quota)| QuotaRow {
                key: key.clone(),
                window: window.clone(),
                first_used_fraction: quota.first,
                last_used_fraction: quota.last,
            }));
        if let Some(grade) = trial.current_grade() {
            self.add_grade(&key, grade);
        }
        if let Some(measure) = trial.current_measure() {
            self.add_workloads(&key, measure.workloads());
        }
        Ok(())
    }

    fn add_grade(&mut self, key: &Key, grade: &GradeRecord) {
        let judgement = grade.judgement();
        self.cases
            .extend(judgement.cases().iter().map(|(case, ran)| {
                CaseRow {
                    key: key.clone(),
                    case: case.to_string(),
                    outcome: ran
                        .outcome
                        .failure()
                        .map_or("pass", |failure| failure.as_str()),
                    detail: match &ran.outcome {
                        CaseOutcome::Fault(message) => Some(message.clone()),
                        CaseOutcome::Abnormal(Abnormal::Crashed(crash)) => Some(crashed(*crash)),
                        _ => None,
                    },
                    wall_s: ran.wall.as_secs_f64(),
                }
            }));
        let files = judgement.complexity().into_iter().flat_map(|c| &c.files);
        for (file, measured) in files {
            self.functions
                .extend(measured.functions.iter().map(|function| FunctionRow {
                    key: key.clone(),
                    file: file.clone(),
                    function: function.name.clone(),
                    line: function.line,
                    ccn: function.ccn,
                    nloc: function.nloc,
                    lexical_tokens: function.tokens,
                    params: function.params,
                }));
        }
    }

    fn add_workloads(&mut self, key: &Key, workloads: &BTreeMap<WorkloadId, measure::Outcome>) {
        for (workload, outcome) in workloads {
            let timed = match outcome {
                measure::Outcome::Timed(timed) => timed,
                measure::Outcome::CandidateFailed(failure) => {
                    self.workloads
                        .push(WorkloadRow::failed(key, workload, failure));
                    continue;
                }
            };
            self.workloads
                .push(WorkloadRow::timed(key, workload, timed));
            for (block_index, block) in timed.blocks.iter().enumerate() {
                for side in [Side::Reference, Side::Candidate] {
                    let runs = block.side(side).iter().enumerate();
                    self.timings.extend(runs.map(|(run_index, run)| TimingRow {
                        key: key.clone(),
                        workload: workload.to_string(),
                        block: block_index + 1,
                        side: side_name(side),
                        run: run_index + 1,
                        wall_s: run.wall,
                        user_s: run.user,
                        system_s: run.system,
                        peak_memory_bytes: run.peak_memory,
                        instructions: run.instructions,
                    }));
                }
            }
        }
    }

    /// Each table's file name, row count and CSV text, in a fixed order.
    pub fn files(&self) -> [(&'static str, usize, String); 7] {
        fn file<R: Columns>(name: &'static str, rows: &[R]) -> (&'static str, usize, String) {
            (name, rows.len(), encode(rows))
        }
        [
            file("trials.csv", &self.trials),
            file("quota.csv", &self.quota),
            file("usage.csv", &self.usage),
            file("cases.csv", &self.cases),
            file("functions.csv", &self.functions),
            file("workloads.csv", &self.workloads),
            file("timings.csv", &self.timings),
        ]
    }
}

impl Key {
    fn of(experiment: &str, trial: &TrialKey) -> Self {
        Self {
            experiment: experiment.to_owned(),
            system: trial.system.id().to_string(),
            scenario: trial.scenario.to_string(),
            language: trial.language.to_string(),
            rep: trial.rep.get(),
        }
    }
}

impl SystemSpec {
    fn of(system: &System) -> Self {
        Self {
            harness: system.harness().to_string(),
            harness_version: system.harness_version().to_string(),
            backend: system.backend().to_string(),
            model: system.model().to_string(),
            effort: system.effort().as_str(),
            effort_rank: system.effort().rank(),
        }
    }
}

impl From<TokenUsage> for TokenColumns {
    fn from(tokens: TokenUsage) -> Self {
        Self {
            input_tokens: tokens.input,
            cache_read_tokens: tokens.cache_read,
            cache_write_tokens: tokens.cache_write,
            cache_write_1h_tokens: tokens.cache_write_long,
            output_tokens: tokens.output,
            reasoning_tokens: tokens.reasoning,
        }
    }
}

impl From<Cost> for CostColumns {
    fn from(cost: Cost) -> Self {
        Self {
            cost_usd: cost.usd(),
            unpriced_tokens: cost.unpriced(),
            cost_base_tier_assumed: cost.base_tier_assumed(),
        }
    }
}

impl TrialRow {
    fn of(
        key: &Key,
        system: &System,
        trial: &Trial,
        tokens: TokenColumns,
        cost: CostColumns,
    ) -> Self {
        let Trial {
            run,
            grade,
            measure,
            stale,
            ..
        } = trial;
        let outcome = run.outcome();
        let process = run.process();
        let exit = match &outcome.exit {
            AgentExit::Completed => "completed",
            AgentExit::Refused => "refused",
            AgentExit::Errored { .. } => "errored",
            AgentExit::Crashed => "crashed",
            AgentExit::Killed => "killed",
        };
        let (process_exit, process_status) = match process.exit {
            ProcessExit::Code(code) => ("code", Some(code)),
            ProcessExit::Signal(signal) => ("signal", Some(signal)),
            ProcessExit::TimedOut => ("timed-out", None),
            ProcessExit::OutOfMemory => ("out-of-memory", None),
            ProcessExit::Killed => ("killed", None),
        };
        let judgement = grade.as_ref().map(GradeRecord::judgement);
        let load = measure.as_ref().map(|measure| measure.machine().load);
        let verdict = judgement.map(|judgement| judgement.verdict());
        let complexity = judgement.and_then(|judgement| judgement.complexity());
        let functions = || {
            complexity
                .into_iter()
                .flat_map(|c| c.files.values())
                .flat_map(|file| &file.functions)
        };
        Self {
            key: key.clone(),
            spec: SystemSpec::of(system),
            stage: match (grade, measure) {
                (None, _) => "ran",
                (Some(_), None) => "graded",
                (Some(_), Some(_)) => "measured",
            },
            stale_phase: stale.map(Phase::as_str),
            run_session: run.session().to_string(),
            started: run.started(),
            exit,
            process_exit,
            process_status,
            cut: process.cut.map(|cut| match cut {
                Cut::Idle => "idle",
                Cut::RateLimited => "rate-limited",
            }),
            turns: outcome.turns,
            wall_s: process.wall.as_secs_f64(),
            api_s: outcome.api_time.map(|api| api.as_secs_f64()),
            tokens,
            cost,
            grade_session: grade.as_ref().map(|grade| grade.session().to_string()),
            verdict: verdict.map(|verdict| match verdict {
                Verdict::Pass => "pass",
                Verdict::Fail(_) => "fail",
            }),
            failure: verdict.and_then(|verdict| match verdict {
                Verdict::Pass => None,
                Verdict::Fail(failure) => Some(failure.as_str()),
            }),
            cases_passed: judgement.map(|judgement| {
                let cases = judgement.cases().values();
                cases
                    .filter(|case| case.outcome == CaseOutcome::Pass)
                    .count()
            }),
            cases_total: judgement.map(|judgement| judgement.cases().len()),
            build_s: judgement.map(|judgement| judgement.build().wall.as_secs_f64()),
            functions: complexity.map(|_| functions().count()),
            ccn_total: complexity.map(|_| functions().map(|f| u64::from(f.ccn)).sum()),
            ccn_max: functions().map(|function| function.ccn).max(),
            nloc: complexity.map(|c| c.files.values().map(|file| u64::from(file.nloc)).sum()),
            complexity_version: complexity.map(Complexity::version),
            lizard_version: complexity.and_then(Complexity::lizard).map(str::to_owned),
            measure_session: measure
                .as_ref()
                .map(|measure| measure.session().to_string()),
            measure_load_start: load.map(|load| load.start[0]),
            measure_load_end: load.map(|load| load.end[0]),
        }
    }
}

impl WorkloadRow {
    fn timed(key: &Key, workload: &WorkloadId, timed: &measure::Timed) -> Self {
        let summary = timed.summary();
        let interval = summary.ratio.interval;
        Self {
            key: key.clone(),
            workload: workload.to_string(),
            outcome: "timed",
            detail: None,
            blocks: timed.blocks.len(),
            wall_ratio: Some(summary.ratio.geomean),
            wall_ratio_low: interval.map(|(low, _)| low),
            wall_ratio_high: interval.map(|(_, high)| high),
            reference_median_s: Some(summary.reference.median),
            reference_min_s: Some(summary.reference.min),
            reference_max_s: Some(summary.reference.max),
            candidate_median_s: Some(summary.candidate.median),
            candidate_min_s: Some(summary.candidate.min),
            candidate_max_s: Some(summary.candidate.max),
            instructions_ratio: summary.instructions,
            reference_peak_memory_bytes: summary.peak_memory.reference,
            candidate_peak_memory_bytes: summary.peak_memory.candidate,
        }
    }

    fn failed(key: &Key, workload: &WorkloadId, failure: &measure::Failure) -> Self {
        Self {
            key: key.clone(),
            workload: workload.to_string(),
            outcome: match failure {
                measure::Failure::WrongOutput => "wrong-output",
                measure::Failure::OutputOverCap => "output-over-cap",
                measure::Failure::TimedRun(_) => "timed-run-failed",
                measure::Failure::Abnormal(Abnormal::TimedOut) => "timeout",
                measure::Failure::Abnormal(Abnormal::OutOfMemory) => "out-of-memory",
                measure::Failure::Abnormal(Abnormal::Crashed(_)) => "crash",
            },
            detail: Some(failure.to_string()),
            blocks: 0,
            wall_ratio: None,
            wall_ratio_low: None,
            wall_ratio_high: None,
            reference_median_s: None,
            reference_min_s: None,
            reference_max_s: None,
            candidate_median_s: None,
            candidate_min_s: None,
            candidate_max_s: None,
            instructions_ratio: None,
            reference_peak_memory_bytes: None,
            candidate_peak_memory_bytes: None,
        }
    }
}

fn crashed(crash: Crash) -> String {
    match crash {
        Crash::Code(code) => format!("exit code {code}"),
        Crash::Signal(signal) => format!("signal {signal}"),
    }
}

fn side_name(side: Side) -> &'static str {
    match side {
        Side::Reference => "reference",
        Side::Candidate => "candidate",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        measure::{Block, PeakMemory, Timed},
        testing,
    };

    fn runs(side: &[(f64, u64, Option<u64>)]) -> Vec<measure::Sample> {
        side.iter()
            .map(|&(wall, peak_memory, instructions)| measure::Sample {
                wall,
                user: wall / 2.0,
                system: 0.0,
                peak_memory,
                instructions,
            })
            .collect()
    }

    fn key() -> Key {
        Key {
            experiment: "pilot".into(),
            system: "codex-0.166.0_openai_gpt-6-luna_low".into(),
            scenario: "word-freq".into(),
            language: "rust".into(),
            rep: 1,
        }
    }

    #[test]
    fn a_timed_workload_is_summarised_and_each_run_kept() {
        let blocks = vec![
            Block {
                reference: runs(&[(1.0, 10, Some(1000)), (3.0, 20, Some(1000))]),
                candidate: runs(&[(2.0, 30, Some(3000)), (6.0, 30, Some(3000))]),
            },
            Block {
                candidate: runs(&[(2.0, 25, Some(3000)), (6.0, 25, Some(3000))]),
                reference: runs(&[(1.0, 15, Some(1000)), (3.0, 12, Some(1000))]),
            },
        ];
        let workloads = BTreeMap::from([
            (
                testing::id("large"),
                measure::Outcome::Timed(Timed {
                    blocks,
                    peak_memory: PeakMemory {
                        reference: Some(48 << 20),
                        candidate: None,
                    },
                }),
            ),
            (
                testing::id("small"),
                measure::Outcome::CandidateFailed(measure::Failure::WrongOutput),
            ),
        ]);
        let mut tables = Tables::default();
        tables.add_workloads(&key(), &workloads);

        let [large, small] = &tables.workloads[..] else {
            panic!("expected two workloads, got {:?}", tables.workloads);
        };
        // Each block's medians are 2s and 4s, so the candidate takes twice as long in both.
        for ratio in [
            large.wall_ratio,
            large.wall_ratio_low,
            large.wall_ratio_high,
        ] {
            assert!(
                ratio.is_some_and(|ratio| (ratio - 2.0).abs() < 1e-12),
                "{ratio:?}"
            );
        }
        assert_eq!(
            (large.outcome, large.blocks, large.detail.as_deref()),
            ("timed", 2, None)
        );
        assert_eq!(
            [
                large.reference_median_s,
                large.reference_min_s,
                large.reference_max_s
            ],
            [Some(2.0), Some(1.0), Some(3.0)]
        );
        assert_eq!(
            [
                large.candidate_median_s,
                large.candidate_min_s,
                large.candidate_max_s
            ],
            [Some(4.0), Some(2.0), Some(6.0)]
        );
        assert_eq!(large.instructions_ratio, Some(3.0));
        // The output check's peaks, not the largest of hyperfine's, which the timings keep.
        assert_eq!(
            (
                large.reference_peak_memory_bytes,
                large.candidate_peak_memory_bytes
            ),
            (Some(48 << 20), None)
        );
        assert_eq!(
            (
                small.outcome,
                small.blocks,
                small.wall_ratio,
                small.detail.as_deref()
            ),
            ("wrong-output", 0, None, Some("wrong output"))
        );

        let order: Vec<(usize, &str, usize, f64, u64)> = tables
            .timings
            .iter()
            .map(|row| {
                let peak = row.peak_memory_bytes;
                (row.block, row.side, row.run, row.wall_s, peak)
            })
            .collect();
        assert_eq!(
            order,
            [
                (1, "reference", 1, 1.0, 10),
                (1, "reference", 2, 3.0, 20),
                (1, "candidate", 1, 2.0, 30),
                (1, "candidate", 2, 6.0, 30),
                (2, "reference", 1, 1.0, 15),
                (2, "reference", 2, 3.0, 12),
                (2, "candidate", 1, 2.0, 25),
                (2, "candidate", 2, 6.0, 25),
            ]
        );
        assert!(tables.timings.iter().all(|row| row.workload == "large"));
    }

    #[test]
    fn instructions_compare_only_when_every_run_counted_them() {
        let blocks = vec![Block {
            reference: runs(&[(1.0, 10, Some(1000)), (1.0, 10, None)]),
            candidate: runs(&[(1.0, 10, Some(1000)), (1.0, 10, Some(1000))]),
        }];
        let workloads = BTreeMap::from([(
            testing::id("large"),
            measure::Outcome::Timed(Timed {
                blocks,
                peak_memory: PeakMemory::default(),
            }),
        )]);
        let mut tables = Tables::default();
        tables.add_workloads(&key(), &workloads);
        assert_eq!(tables.workloads[0].instructions_ratio, None);
        assert_eq!(tables.workloads[0].wall_ratio_low, None);
    }
}
