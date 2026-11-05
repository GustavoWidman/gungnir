//! The measure phase: each trial whose passing grade has no current measurement, timed against
//! the reference in its language in one lane per measurement core, and committed with
//! hyperfine's exports. Timing needs a quiet machine, so unless told otherwise the phase
//! refuses a busy one and holds the run and grade locks, which keeps both phases from starting.
use super::{
    Identity, MeasureRecord, Outcome, Program, Protocol, Settings, compare,
    lanes::{self, Cores},
    machine,
};
use crate::{
    experiment::Experiment,
    grade::Verdict,
    id::{LanguageId, Rep, ScenarioId},
    lifecycle::{Current, Lifecycle, Stage},
    provenance::{Session, SessionId},
    sandbox::{Backend, Fault},
    stop::Stop,
    store::{Lock, PartialDir, sweep_partial},
    toolchains::Toolchains,
    trial::{Phase, ResultsDir, TrialDir, TrialKey},
    workload::{self, Ensured},
};
use eyre::{Result, WrapErr, bail, ensure, eyre};
use globset::{Glob, GlobMatcher};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write,
    path::Path,
};
use tokio::sync::OnceCell;
use tracing::{error, info, warn};

/// The one-minute load average from which a machine counts as busy before measuring starts. A
/// task's worth of runnable work beside the timed programs shares their caches, the memory bus
/// and the package's power budget, which sets the clock of every core.
pub const BUSY_LOAD: f64 = 1.0;

/// Whether measuring insists on a quiet machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quiet {
    /// Refuse a busy machine, and hold the run and grade locks while measuring.
    Required,
    /// Measure whatever else runs. Each record keeps the load averages it ran under.
    Waived,
}

/// How one invocation measures.
pub struct Request {
    pub selection: Selection,
    /// The CPUs the lanes pin their comparisons to, one lane per CPU.
    pub cores: Cores,
    pub quiet: Quiet,
}

/// Which trials one invocation may measure: every one, or only those of the repetitions and
/// systems named.
#[derive(Debug, Default)]
pub struct Selection {
    reps: BTreeSet<Rep>,
    systems: Vec<GlobMatcher>,
}

impl Selection {
    /// Trials of the repetitions `reps` (all when empty) whose system id matches one of the
    /// globs `systems` (any when empty). A repetition or glob that selects nothing in
    /// `experiment` is an error, since it is most likely a typo.
    pub fn new(experiment: &Experiment, reps: &[Rep], systems: &[String]) -> Result<Self> {
        let all: Vec<String> = experiment
            .systems()
            .iter()
            .map(|system| system.id().to_string())
            .collect();
        let mut matchers = Vec::new();
        for pattern in systems {
            let matcher = Glob::new(pattern)
                .wrap_err_with(|| format!("--system {pattern:?} is not a glob"))?
                .compile_matcher();
            ensure!(
                all.iter().any(|id| matcher.is_match(id)),
                "--system {pattern:?} matches none of the systems of {}: {}",
                experiment.name(),
                all.join(", ")
            );
            matchers.push(matcher);
        }
        let last = experiment.most_repetitions();
        for rep in reps {
            ensure!(
                Rep::up_to(last).any(|known| known == *rep),
                "{} has repetitions r1 to r{last}, not {rep}",
                experiment.name()
            );
        }
        Ok(Self {
            reps: reps.iter().copied().collect(),
            systems: matchers,
        })
    }

    fn contains(&self, trial: &TrialKey) -> bool {
        let id = trial.system.id().to_string();
        (self.reps.is_empty() || self.reps.contains(&trial.rep))
            && (self.systems.is_empty() || self.systems.iter().any(|glob| glob.is_match(&id)))
    }
}

/// What became of the trials one invocation set out to measure.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Measured {
    pub committed: usize,
    /// Not measured, because a sandbox could not run.
    pub faulted: usize,
    /// Not measured, because building, timing or the host failed, or the second Ctrl-C.
    pub errors: usize,
    /// Not started, because the invocation drained first.
    pub unstarted: usize,
}

/// Measures every selected trial of `experiment`, held in the repository `repo`, whose grade
/// passed and whose measurement is missing or stale under the experiment's protocol, committing
/// each measurement under its trial. The trials start in order, one comparison per core of the
/// request at a time, each pinned to its core for all its runs. Each reference is built once
/// per invocation and shared by the lanes.
pub async fn measure_trials(
    experiment: &Experiment,
    repo: &Path,
    toolchains: &Toolchains,
    backend: &dyn Backend,
    request: &Request,
    stop: &Stop,
) -> Result<Measured> {
    let results = experiment.results();
    pinned(experiment.machine(), Identity::here()?)?;
    machine::check_clock(experiment.protocol().clock_mhz, request.cores.iter())?;
    let _locks = quiet(results, request.quiet)?;
    results.sweep_scratch(Phase::Measure)?;
    let mut used = vec![
        ("base".to_owned(), toolchains.base()),
        ("measure".to_owned(), toolchains.measure()),
    ];
    for language in experiment.languages() {
        let id = language.id();
        used.push((format!("language/{id}"), toolchains.language(id.as_str())?));
    }
    let session = Session::start(repo)?.with_toolchains(used);
    session.write(results)?;

    let trials: Vec<TrialKey> = experiment
        .trials()
        .into_iter()
        .filter(|trial| request.selection.contains(trial))
        .collect();
    let mut due = Vec::new();
    for trial in &trials {
        let dir = results.trial(trial);
        sweep_partial(&dir.phase(Phase::Measure))?;
        let current = Current::of(experiment, trial, toolchains)?;
        let lifecycle = Lifecycle::read(&dir, &current)?;
        if is_due(&lifecycle) {
            due.push(Due {
                trial,
                dir,
                current,
            });
        }
    }
    info!(
        "{}: {} of {} selected trials to measure on CPUs {}",
        experiment.name(),
        due.len(),
        trials.len(),
        request.cores
    );
    let scenarios: BTreeMap<&ScenarioId, _> = due
        .iter()
        .map(|due| (due.current.scenario().id(), due.current.scenario()))
        .collect();
    for scenario in scenarios.values() {
        if workload::ensure(scenario, toolchains, backend, stop).await? == Ensured::Regenerated {
            info!("{}: regenerated the workloads", scenario.id());
        }
    }

    let measuring = Measuring {
        results,
        protocol: experiment.protocol(),
        toolchains,
        backend,
        session: session.id(),
        stop,
        references: References::of(&due),
    };
    let mut measured = Measured::default();
    let unstarted = lanes::on_cores(
        &due,
        &request.cores,
        stop,
        async |due: &&Due, core: u32| measuring.measure(due, core).await,
        |due, done| settle(&mut measured, due.trial, done),
    )
    .await;
    Ok(Measured {
        unstarted,
        ..measured
    })
}

/// Whether a trial in this state needs measuring: graded, passed, and not measured under the
/// current inputs, with its run and grade current.
fn is_due(lifecycle: &Lifecycle) -> bool {
    match (&lifecycle.stage, lifecycle.stale) {
        (Stage::Graded { grade, .. }, None) => grade.judgement().verdict() == Verdict::Pass,
        (Stage::Measured { .. }, Some(Phase::Measure)) => true,
        _ => false,
    }
}

/// Refuses to measure on any machine but the one the experiment pins, when it pins one.
fn pinned(pin: Option<&Identity>, here: &Identity) -> Result<()> {
    if let Some(pin) = pin {
        ensure!(
            pin == here,
            "this machine is {here}, and the experiment's [machine] pins measuring to {pin}; \
             measure on that machine, or change the pin"
        );
    }
    Ok(())
}

/// Holds the locks a measurement needs: its own always, and the run and grade locks while the
/// machine must stay quiet, which also requires a load average below [`BUSY_LOAD`].
fn quiet(results: &ResultsDir, quiet: Quiet) -> Result<Vec<Lock>> {
    let mut locks = vec![results.lock(Phase::Measure)?];
    if quiet == Quiet::Waived {
        return Ok(locks);
    }
    for phase in [Phase::Run, Phase::Grade] {
        let lock = results.lock(phase).wrap_err_with(|| {
            format!(
                "measuring needs a quiet machine, and {phase} is running; --allow-busy measures \
                 anyway"
            )
        })?;
        locks.push(lock);
    }
    let load = machine::load_average()?;
    if let Some(why) = busy(load) {
        bail!("{why}; --allow-busy measures anyway");
    }
    Ok(locks)
}

/// Why a machine with these 1, 5 and 15 minute load averages is too busy to measure on.
fn busy(load: [f64; 3]) -> Option<String> {
    (load[0] >= BUSY_LOAD).then(|| {
        format!(
            "the machine is busy: its one-minute load average is {:.2}, and measuring needs it \
             below {BUSY_LOAD:.1}, which leaves no other task running beside the timed one",
            load[0]
        )
    })
}

/// A selected trial that needs measuring.
struct Due<'a> {
    trial: &'a TrialKey,
    dir: TrialDir,
    current: Current<'a>,
}

/// Each (scenario, language) reference a due trial needs, built the first time a lane needs it
/// and shared by every lane. A lane that needs a reference while another lane builds it waits
/// for that build. A reference that does not build fails every trial that needs it in this
/// invocation.
struct References<'a> {
    built: BTreeMap<(ScenarioId, LanguageId), OnceCell<Result<Program<'a>, String>>>,
}

impl<'a> References<'a> {
    /// None built yet, with a place for the reference of each of `due`.
    fn of(due: &[Due<'_>]) -> Self {
        let keys = due.iter().map(|due| {
            let (scenario, language) = (due.current.scenario(), due.current.language());
            (
                (scenario.id().clone(), language.id().clone()),
                OnceCell::new(),
            )
        });
        Self {
            built: keys.collect(),
        }
    }

    async fn get(
        &self,
        current: &Current<'_>,
        toolchains: &Toolchains,
        backend: &'a dyn Backend,
        stop: &'a Stop,
    ) -> Result<&Program<'a>> {
        let (scenario, language) = (current.scenario(), current.language());
        let key = (scenario.id().clone(), language.id().clone());
        let cell = self.built.get(&key).ok_or_else(|| {
            eyre!(
                "the {} reference of {} is not among the ones due",
                key.1,
                key.0
            )
        })?;
        let built = cell
            .get_or_init(async || {
                let built = match scenario.reference(language.id()) {
                    Some(reference) => {
                        Program::build(language, toolchains, reference.dir(), backend, stop).await
                    }
                    None => Err(eyre!("there is no such reference")),
                };
                built.map_err(|err| format!("{err:#}"))
            })
            .await;
        match built {
            Ok(program) => Ok(program),
            Err(why) => bail!("the {} reference of {} did not build: {why}", key.1, key.0),
        }
    }
}

/// What every lane measures with.
struct Measuring<'a> {
    results: &'a ResultsDir,
    protocol: Protocol,
    toolchains: &'a Toolchains,
    backend: &'a dyn Backend,
    session: &'a SessionId,
    stop: &'a Stop,
    references: References<'a>,
}

impl Measuring<'_> {
    /// Builds `due`'s committed workspace, times it against the reference in its language with
    /// every run pinned to `core`, and commits the measurement in place of a stale one.
    async fn measure(&self, due: &Due<'_>, core: u32) -> Result<MeasureRecord> {
        let current = &due.current;
        let reference = self
            .references
            .get(current, self.toolchains, self.backend, self.stop)
            .await?;
        let (scenario, language) = (current.scenario(), current.language());
        let workspace = due.dir.final_workspace();
        let inputs = current.measure(&workspace)?;
        let candidate = Program::build(
            language,
            self.toolchains,
            &workspace,
            self.backend,
            self.stop,
        )
        .await
        .wrap_err("building the candidate")?;
        let target = due.dir.phase(Phase::Measure);
        self.results.retract(Phase::Measure, &target)?;
        let partial = PartialDir::begin(&target)?;
        let settings = Settings {
            core,
            protocol: self.protocol,
            memory: scenario.limits().memory_bytes(),
        };
        let comparison = compare(
            reference,
            &candidate,
            scenario.workloads(),
            &settings,
            self.backend,
            self.stop,
            partial.path(),
        )
        .await?;
        let record = MeasureRecord::new(self.session.clone(), inputs, comparison);
        partial.commit(&record)?;
        Ok(record)
    }
}

/// Counts and logs how measuring `trial` ended.
fn settle(measured: &mut Measured, trial: &TrialKey, done: Result<MeasureRecord>) {
    match done {
        Ok(record) => {
            measured.committed += 1;
            info!(
                "{trial} on CPU {}: {}",
                record.machine().core,
                describe(&record)
            );
        }
        Err(err) if err.chain().any(|cause| cause.is::<Fault>()) => {
            measured.faulted += 1;
            warn!("{trial}: not measured, a sandbox could not run: {err:#}");
        }
        Err(err) => {
            measured.errors += 1;
            error!("{trial}: not measured: {err:#}");
        }
    }
}

/// Each workload's wall-time ratio with its 95% interval, instruction ratio and peak memory,
/// or why the candidate was not timed on it.
pub fn describe(record: &MeasureRecord) -> String {
    let mut line = String::new();
    for (index, (id, outcome)) in record.workloads().iter().enumerate() {
        let separator = if index == 0 { "" } else { "; " };
        let _ = write!(line, "{separator}{id} ");
        let timed = match outcome {
            Outcome::Timed(timed) => timed,
            Outcome::CandidateFailed(failure) => {
                let _ = write!(line, "failed: {failure}");
                continue;
            }
        };
        let summary = timed.summary();
        let _ = write!(line, "wall {:.3}", summary.ratio.geomean);
        if let Some((low, high)) = summary.ratio.interval {
            let _ = write!(line, " [{low:.3}, {high:.3}]");
        }
        if let Some(instructions) = summary.instructions {
            let _ = write!(line, ", instructions {instructions:.3}");
        }
        let _ = write!(line, ", peak {}", summary.peak_memory);
    }
    line
}

#[cfg(test)]
mod pin_tests {
    use super::*;

    #[test]
    fn measuring_refuses_any_machine_but_the_pinned_one() {
        let desktop = Identity {
            cpu: Some("13th Gen Intel(R) Core(TM) i9-13900K".into()),
            host: Some("d35k70p".into()),
        };
        let vm = Identity {
            cpu: Some("AMD Ryzen 7 5800U with Radeon Graphics".into()),
            host: Some("4a7c1e90".into()),
        };
        assert!(pinned(None, &vm).is_ok());
        assert!(pinned(Some(&desktop), &desktop).is_ok());
        let message = pinned(Some(&desktop), &vm).unwrap_err().to_string();
        assert!(message.contains("AMD Ryzen 7 5800U"), "{message}");
        assert!(message.contains("i9-13900K"), "{message}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::{BoxFuture, Spec};
    use crate::{
        grade::{
            self,
            fake::{Scripted, finished},
        },
        run::fixture::{self, Repo},
        sandbox::{Exit, Finished, Input, Io},
        store::read_record,
        testing::{id, write},
    };
    use serde_json::json;
    use std::{
        ffi::OsString,
        fs,
        num::{NonZeroU32, NonZeroUsize},
        sync::{
            Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::Duration,
    };

    const QUICK: Protocol = Protocol {
        blocks: NonZeroU32::new(2).unwrap(),
        runs: NonZeroU32::new(1).unwrap(),
        warmup: 0,
        clock_mhz: None,
    };

    /// Builds succeed and programs echo their input, peaking at 2 MiB in their output check
    /// and taking 10 ms and a million instructions in every timed run.
    fn timed(spec: &crate::sandbox::Spec, argv: &[String], io: Io) -> Result<Finished, Fault> {
        match argv[0].as_str() {
            "hyperfine" => {
                let runs = &argv[argv.iter().position(|arg| arg == "--runs").unwrap() + 1];
                let run = json!({
                    "time_wall_clock": {"value": 0.01, "unit": "second"},
                    "time_user": {"value": 0.01, "unit": "second"},
                    "time_system": {"value": 0.0, "unit": "second"},
                    "memory_peak_resident": {"value": 3.0e6, "unit": "byte"},
                    "instructions": {"value": 1.0e6},
                    "exit_code": 0,
                });
                let measurements = vec![run; runs.parse().unwrap()];
                let export =
                    json!({"schema_version": 2, "results": [{"measurements": measurements}]});
                Ok(finished(
                    Exit::Code(0),
                    &serde_json::to_vec(&export).unwrap(),
                ))
            }
            "time" => {
                let Input::File(input) = io.stdin else {
                    panic!("a check run reads its workload");
                };
                let report = spec.mounts.iter().find(|mount| mount.writable).unwrap();
                fs::write(report.host.join("peak"), "2048\n").unwrap();
                Ok(finished(Exit::Code(0), &fs::read(input).unwrap()))
            }
            _ => Ok(finished(Exit::Code(0), b"built\n")),
        }
    }

    /// A backend that answers as [`timed`] does, a millisecond later, so that lanes overlap.
    /// It counts the builds it runs and the most programs pinned to CPUs at once, and fails a
    /// program pinned to a CPU that another running program holds.
    #[derive(Default)]
    struct Overlapping {
        builds: AtomicUsize,
        pinned: Mutex<BTreeSet<u32>>,
        most: AtomicUsize,
        /// Escalate the stop the first time a program is pinned.
        drain: AtomicBool,
    }

    impl Backend for Overlapping {
        fn exec<'a>(
            &'a self,
            spec: &'a Spec,
            argv: &'a [OsString],
            io: Io,
            stop: &'a Stop,
        ) -> BoxFuture<'a, Result<Finished, Fault>> {
            Box::pin(async move {
                let argv: Vec<String> = argv
                    .iter()
                    .map(|arg| arg.to_str().unwrap().to_owned())
                    .collect();
                if argv.iter().any(|arg| arg == "compileall") {
                    self.builds.fetch_add(1, Ordering::SeqCst);
                }
                let cpu = spec.limits.cpu;
                if let Some(cpu) = cpu {
                    let mut pinned = self.pinned.lock().unwrap();
                    assert!(
                        pinned.insert(cpu),
                        "two programs pinned to CPU {cpu} at once"
                    );
                    self.most.fetch_max(pinned.len(), Ordering::SeqCst);
                    if self.drain.swap(false, Ordering::SeqCst) {
                        stop.escalate();
                    }
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
                let finished = timed(spec, &argv, io);
                if let Some(cpu) = cpu {
                    self.pinned.lock().unwrap().remove(&cpu);
                }
                finished
            })
        }
    }

    fn request() -> Request {
        Request {
            selection: Selection::default(),
            cores: Cores::one(0),
            quiet: Quiet::Waived,
        }
    }

    fn two_lanes() -> Request {
        Request {
            cores: Cores::new([0, 1]).unwrap(),
            ..request()
        }
    }

    async fn measure(repo: &Repo, backend: &dyn Backend, request: &Request) -> Result<Measured> {
        let experiment = repo.experiment();
        let (root, toolchains, stop) = (repo.root.path(), &repo.toolchains, Stop::default());
        measure_trials(&experiment, root, toolchains, backend, request, &stop).await
    }

    fn lifecycle(repo: &Repo, model: &str) -> Lifecycle {
        let experiment = repo.experiment();
        let trial = fixture::trial(&experiment, model);
        let current = Current::of(&experiment, &trial, &repo.toolchains).unwrap();
        Lifecycle::read(&experiment.results().trial(&trial), &current).unwrap()
    }

    /// Makes the fixture experiment measure under [`QUICK`], commits an echoing run for each of
    /// `models`, and grades every run without a current grade.
    async fn graded(repo: &Repo, models: &[&str]) {
        fixture::declare_protocol(repo, QUICK);
        let experiment = repo.experiment();
        let trials: Vec<TrialKey> = models
            .iter()
            .map(|model| fixture::trial(&experiment, model))
            .collect();
        commit_and_grade(repo, &trials).await;
    }

    /// Makes the fixture experiment measure under [`QUICK`] and run each system `reps` times,
    /// and commits and grades an echoing run for every trial.
    async fn every_trial_graded(repo: &Repo, reps: u32) {
        fixture::declare_protocol(repo, QUICK);
        let file = repo.root.path().join("experiments/fixture.toml");
        let text = fs::read_to_string(&file).unwrap();
        let repeated = text.replacen("repetitions = 1", &format!("repetitions = {reps}"), 1);
        fs::write(&file, repeated).unwrap();
        commit_and_grade(repo, &repo.experiment().trials()).await;
    }

    /// Commits an echoing run for each of `trials`, and grades every run without a current
    /// grade.
    async fn commit_and_grade(repo: &Repo, trials: &[TrialKey]) {
        for trial in trials {
            fixture::commit_run(repo, trial, &[("main.py", "print(input())\n")]);
        }
        let experiment = repo.experiment();
        let (root, toolchains, stop) = (repo.root.path(), &repo.toolchains, Stop::default());
        let echo = fixture::echo();
        grade::grade_runs(
            &experiment,
            root,
            toolchains,
            &echo,
            NonZeroUsize::MIN,
            &stop,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn two_lanes_measure_each_trial_once_on_their_own_cores_and_share_the_reference() {
        let repo = Repo::new();
        every_trial_graded(&repo, 2).await;
        let lanes = Overlapping::default();
        let measured = measure(&repo, &lanes, &two_lanes()).await.unwrap();
        assert_eq!(
            measured,
            Measured {
                committed: 4,
                ..Measured::default()
            }
        );
        assert_eq!(lanes.builds.load(Ordering::SeqCst), 1 + 4);
        assert_eq!(lanes.most.load(Ordering::SeqCst), 2);

        let experiment = repo.experiment();
        let mut cores = BTreeSet::new();
        for trial in experiment.trials() {
            let dir = experiment.results().trial(&trial);
            let record = read_record::<MeasureRecord>(&dir.phase(Phase::Measure)).unwrap();
            cores.insert(record.unwrap().machine().core);
        }
        assert_eq!(cores, BTreeSet::from([0, 1]));
        let again = measure(&repo, &lanes, &two_lanes()).await.unwrap();
        assert_eq!(again, Measured::default());
    }

    #[tokio::test]
    async fn a_drain_lets_every_comparison_in_progress_finish_and_starts_no_more() {
        let repo = Repo::new();
        every_trial_graded(&repo, 2).await;
        let lanes = Overlapping {
            drain: AtomicBool::new(true),
            ..Overlapping::default()
        };
        let measured = measure(&repo, &lanes, &two_lanes()).await.unwrap();
        assert_eq!(
            measured,
            Measured {
                committed: 2,
                unstarted: 2,
                ..Measured::default()
            }
        );
        assert!(!lanes.drain.load(Ordering::SeqCst));
        let rest = measure(&repo, &lanes, &two_lanes()).await.unwrap();
        assert_eq!(rest.committed, 2);
    }

    #[tokio::test]
    async fn passing_trials_are_measured_once_and_again_when_their_measurement_goes_stale() {
        let repo = Repo::new();
        graded(&repo, &["opus"]).await;
        let timer = Scripted::new(timed);
        let measured = measure(&repo, &timer, &request()).await.unwrap();
        assert_eq!(
            measured,
            Measured {
                committed: 1,
                ..Measured::default()
            }
        );
        let stage = lifecycle(&repo, "opus").stage;
        let Stage::Measured {
            measure: record, ..
        } = stage
        else {
            panic!("not measured: {stage:?}");
        };
        let Outcome::Timed(timed) = &record.workloads()[&id("large")] else {
            panic!("not timed: {record:?}");
        };
        let summary = timed.summary();
        assert_eq!(summary.instructions, Some(1.0));
        assert_eq!(summary.peak_memory.candidate, Some(2 << 20));
        assert_eq!(
            describe(&record),
            "large wall 1.000 [1.000, 1.000], instructions 1.000, peak 2.0MiB (reference 2.0MiB)"
        );
        assert_eq!(lifecycle(&repo, "luna").stage, Stage::Pending);

        let again = measure(&repo, &timer, &request()).await.unwrap();
        assert_eq!(again, Measured::default());

        let longer = Protocol {
            runs: NonZeroU32::new(2).unwrap(),
            ..QUICK
        };
        fixture::declare_protocol(&repo, longer);
        assert_eq!(lifecycle(&repo, "opus").stale, Some(Phase::Measure));
        let remeasured = measure(&repo, &timer, &request()).await.unwrap();
        assert_eq!(remeasured.committed, 1);
        assert_eq!(lifecycle(&repo, "opus").stale, None);
        let hyperfine = timer
            .calls()
            .into_iter()
            .filter(|(_, argv)| argv[0] == "hyperfine");
        let runs: Vec<String> = hyperfine
            .map(|(_, argv)| argv[argv.iter().position(|arg| arg == "--runs").unwrap() + 1].clone())
            .collect();
        assert_eq!(
            runs.iter().filter(|runs| *runs == "2").count(),
            4,
            "{runs:?}"
        );
    }

    #[tokio::test]
    async fn a_measurement_under_a_failing_grade_does_not_count() {
        let repo = Repo::new();
        graded(&repo, &["opus"]).await;
        let timer = Scripted::new(timed);
        measure(&repo, &timer, &request()).await.unwrap();

        write(repo.root.path(), "scenarios/echo/hidden/edge.out", "x\n");
        graded(&repo, &[]).await;
        let failed = lifecycle(&repo, "opus");
        assert!(matches!(failed.stage, Stage::Graded { .. }), "{failed:?}");
        assert_eq!(failed.stale, None);
        let experiment = repo.experiment();
        let dir = experiment
            .results()
            .trial(&fixture::trial(&experiment, "opus"));
        let kept = read_record::<MeasureRecord>(&dir.phase(Phase::Measure)).unwrap();
        assert!(kept.is_some());
        let measured = measure(&repo, &timer, &request()).await.unwrap();
        assert_eq!(measured, Measured::default());
    }

    #[tokio::test]
    async fn a_sandbox_fault_commits_nothing_and_the_trial_waits() {
        let repo = Repo::new();
        graded(&repo, &["opus", "luna"]).await;
        let broken = Scripted::new(|spec, argv, io| match argv[0].as_str() {
            "hyperfine" => Err(eyre!("taskset: invalid CPU").into()),
            _ => timed(spec, argv, io),
        });
        let measured = measure(&repo, &broken, &request()).await.unwrap();
        assert_eq!(
            measured,
            Measured {
                faulted: 2,
                ..Measured::default()
            }
        );
        let waiting = lifecycle(&repo, "opus").stage;
        assert!(matches!(waiting, Stage::Graded { .. }), "{waiting:?}");
        let timer = Scripted::new(timed);
        let measured = measure(&repo, &timer, &request()).await.unwrap();
        assert_eq!(measured.committed, 2);
    }

    #[tokio::test]
    async fn a_quiet_machine_means_no_run_or_grade_is_running() {
        let repo = Repo::new();
        graded(&repo, &["opus"]).await;
        let experiment = repo.experiment();
        let _running = experiment.results().lock(Phase::Run).unwrap();
        let quiet = Request {
            quiet: Quiet::Required,
            ..request()
        };
        let timer = Scripted::new(timed);
        let message = format!("{:#}", measure(&repo, &timer, &quiet).await.unwrap_err());
        assert!(message.contains("run is running"), "{message}");
        assert!(message.contains("--allow-busy"), "{message}");
        let measured = measure(&repo, &timer, &request()).await.unwrap();
        assert_eq!(measured.committed, 1);
    }

    #[tokio::test]
    async fn a_clock_the_core_does_not_hold_refuses_before_anything_runs() {
        let repo = Repo::new();
        graded(&repo, &["opus"]).await;
        let held = Protocol {
            clock_mhz: NonZeroU32::new(4_000_000),
            ..QUICK
        };
        fixture::declare_protocol(&repo, held);
        let timer = Scripted::new(timed);
        let message = format!(
            "{:#}",
            measure(&repo, &timer, &request()).await.unwrap_err()
        );
        assert!(message.contains("CPU 0"), "{message}");
        assert!(message.contains("4000000 MHz"), "{message}");
        assert!(timer.calls().is_empty());
        assert!(matches!(
            lifecycle(&repo, "opus").stage,
            Stage::Graded { .. }
        ));
    }

    #[test]
    fn a_load_of_one_task_beside_the_timed_one_is_busy() {
        assert_eq!(busy([0.99, 4.0, 4.0]), None);
        let why = busy([1.0, 0.2, 0.1]).unwrap();
        assert!(why.contains("is 1.00"), "{why}");
    }

    #[test]
    fn a_selection_names_repetitions_and_system_globs_that_exist() {
        let repo = Repo::new();
        let experiment = repo.experiment();
        let rep = |n| Rep::new(n).unwrap();
        let selected = |reps: &[Rep], systems: &[&str]| {
            let systems: Vec<String> = systems.iter().map(|s| s.to_string()).collect();
            Selection::new(&experiment, reps, &systems).map(|selection| {
                let trials = experiment.trials();
                let chosen = trials.iter().filter(|trial| selection.contains(trial));
                chosen
                    .map(|trial| trial.system.model().to_string())
                    .collect::<Vec<_>>()
            })
        };
        assert_eq!(selected(&[], &[]).unwrap(), ["opus", "luna"]);
        assert_eq!(selected(&[rep(1)], &["*luna*"]).unwrap(), ["luna"]);
        let message = selected(&[], &["*sonnet*"]).unwrap_err().to_string();
        assert!(message.contains("matches none"), "{message}");
        let message = selected(&[rep(2)], &[]).unwrap_err().to_string();
        assert!(message.contains("r1 to r1, not r2"), "{message}");
    }
}
