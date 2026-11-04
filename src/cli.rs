//! The command line.
use crate::{
    catalog::SystemId,
    experiment::Experiment,
    grade::{self, Graded, Judgement, Outcome, Verdict},
    id::{BackendId, LanguageId, Rep, ScenarioId, WorkloadId},
    language::Language,
    lifecycle::{self, Current, Lifecycle},
    measure::{
        self, Machine, MeasureInputs, MeasureRecord, Measured, Program, Protocol, Quiet, Selection,
        Settings, Spread, Summary,
    },
    provenance::Session,
    report,
    run::{self, Ceiling, Guard, Host, Tally},
    sandbox::{Backend, Bubblewrap},
    scenario::Scenario,
    stop::{Stage, Stop},
    store::PartialDir,
    toolchains::Toolchains,
    workload::{self, Ensured},
};
use chrono::{DateTime, NaiveDate, Utc};
use clap::{Args, Parser, Subcommand};
use eyre::{Result, WrapErr, bail, ensure, eyre};
use futures_util::{StreamExt, stream};
use nix::{
    sched::{CpuSet, sched_getaffinity},
    unistd::Pid,
};
use std::{
    collections::BTreeMap,
    fmt,
    io::IsTerminal,
    num::{NonZeroU32, NonZeroUsize},
    ops::Add,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Parser)]
#[command(
    version,
    about = "Benchmark coding agents across programming languages and reasoning effort"
)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate an experiment and print the trial matrix it expands to.
    Plan {
        /// The experiment file, such as experiments/pilot.toml.
        experiment: PathBuf,
    },
    /// Build and grade every reference solution the way trials are graded, after making sure
    /// every workload has its pinned digest. Fails if any reference does not pass.
    Check {
        /// Check only this scenario. Repeat for several.
        #[arg(long = "scenario", value_name = "ID")]
        scenarios: Vec<ScenarioId>,
        /// Check only this language. Repeat for several.
        #[arg(long = "language", value_name = "ID")]
        languages: Vec<LanguageId>,
        /// How many references to build and grade at once.
        #[arg(long, value_name = "N", default_value_t = default_jobs())]
        jobs: NonZeroUsize,
    },
    /// Time a candidate against a scenario's reference solution on every workload. Without a
    /// candidate the reference is timed against itself, an A/A test whose ratio interval
    /// shows the noise floor.
    Bench(Bench),
    /// Run each pending trial of an experiment until it commits one attempt that is a sample,
    /// a session that completed or was refused, after checking the harness pins and the
    /// references. The first Ctrl-C lets running agents finish, the second kills them.
    Run {
        /// The experiment file, such as experiments/smoke.toml.
        experiment: PathBuf,
        /// Stop starting attempts on a backend once a quota window reports this fraction used,
        /// as BACKEND[:WINDOW]=FRACTION[@TIME]: WINDOW names one window (any window when
        /// omitted), and TIME, in RFC 3339, counts only attempts that started at or after it.
        /// Such as anthropic:seven_day=0.75 or openai=0.35@2026-11-04T04:05:00-03:00.
        #[arg(long = "quota-ceiling", value_name = "BACKEND[:WINDOW]=FRACTION[@TIME]", value_parser = ceiling)]
        ceilings: Vec<(BackendId, Ceiling)>,
        /// Stop starting attempts on a backend at a time, as BACKEND=RFC3339, such as
        /// anthropic=2026-11-04T04:00:00-03:00. Repeat for several.
        #[arg(long = "stop-at", value_name = "BACKEND=TIME", value_parser = deadline)]
        deadlines: Vec<(BackendId, DateTime<Utc>)>,
        /// Stop a backend after this many attempts in a row errored, crashed or failed.
        #[arg(long, value_name = "N", default_value_t = 3)]
        max_errors: u32,
        /// Leave a trial pending once this many of its attempts were no sample or failed, rate
        /// limits aside. Until then such a trial runs again after the rest of its lane.
        #[arg(long, value_name = "N", default_value = "5")]
        max_attempts: NonZeroU32,
        /// Print the committed runs that would be withdrawn as no sample and the trials each
        /// backend would schedule, after checking the pins, and change nothing. It takes no
        /// lock and does not check the references.
        #[arg(long)]
        dry_run: bool,
    },
    /// Grade each committed run that has no current grade, building and running it offline
    /// on the hidden cases. Safe to run while `run` is still going.
    Grade {
        /// The experiment file, such as experiments/smoke.toml.
        experiment: PathBuf,
        /// How many runs to grade at once.
        #[arg(long, value_name = "N", default_value_t = default_jobs())]
        jobs: NonZeroUsize,
    },
    /// Time each passing trial without a current measurement against the reference in its
    /// language, one comparison at a time on one pinned core. Refuses a busy machine, and holds
    /// the run and grade locks, unless --allow-busy. The first Ctrl-C lets the comparison in
    /// progress finish, the second kills it.
    Measure(Measure),
    /// Print this machine's identity as the [machine] table that pins an experiment's
    /// measurements to it.
    Machine,
    /// Run one trial's candidate and the reference in its language once each under perf on one
    /// workload, and keep the samples, the folded stacks and a flame graph of each under the
    /// trial's profile/<workload>/. Explains an outlier; never part of a measurement.
    Profile {
        /// The experiment file, such as experiments/pilot.toml.
        experiment: PathBuf,
        /// The trial, as <system>/<scenario>/<language>/r<k>.
        #[arg(long, value_name = "TRIAL")]
        trial: String,
        /// The workload to run. Defaults to the one whose wall-time ratio was worst, or the
        /// first before the trial is measured.
        #[arg(long, value_name = "ID")]
        workload: Option<WorkloadId>,
    },
    /// Count each system's trials by stage, and the stale ones.
    Status {
        /// The experiment file, such as experiments/smoke.toml.
        experiment: PathBuf,
    },
    /// Fold the committed records of experiments into CSV tables for analysis, replacing the
    /// output directory as a whole and writing its manifest.json last.
    Report {
        /// The experiment files, such as experiments/pilot.toml.
        #[arg(required = true)]
        experiments: Vec<PathBuf>,
        /// The directory to replace with the report.
        #[arg(long, value_name = "DIR", default_value = "runs/report")]
        out: PathBuf,
        /// Price tokens at the list prices in effect on this date, as YYYY-MM-DD, rather than
        /// today's in UTC.
        #[arg(long, value_name = "DATE")]
        prices: Option<NaiveDate>,
    },
}

impl Cli {
    pub fn run(self) -> Result<()> {
        match self.command {
            Command::Plan { experiment } => {
                print!("{}", Plan(&Experiment::load(&experiment)?));
                Ok(())
            }
            Command::Check {
                scenarios,
                languages,
                jobs,
            } => run_check(&scenarios, &languages, jobs),
            Command::Bench(bench) => bench.run(),
            Command::Run {
                experiment,
                ceilings,
                deadlines,
                max_errors,
                max_attempts,
                dry_run,
            } => {
                let guard = Guard {
                    ceilings: ceilings.into_iter().collect(),
                    deadlines: deadlines.into_iter().collect(),
                    max_errors: Some(max_errors),
                    max_attempts: Some(max_attempts),
                };
                run_trials(&experiment, &guard, dry_run)
            }
            Command::Grade { experiment, jobs } => grade_trials(&experiment, jobs),
            Command::Measure(measure) => measure.run(),
            Command::Machine => {
                print!("{}", machine_pin(measure::Identity::here()?)?);
                Ok(())
            }
            Command::Profile {
                experiment,
                trial,
                workload,
            } => profile(&experiment, &trial, workload.as_ref()),
            Command::Status { experiment } => {
                let experiment = Experiment::load(&experiment)?;
                let runtime = runtime()?;
                let (toolchains, _) = probed(experiment.languages(), &runtime)?;
                print!("{}", StatusTable(&status(&experiment, &toolchains)?));
                Ok(())
            }
            Command::Report {
                experiments,
                out,
                prices,
            } => write_report(&experiments, &out, prices),
        }
    }
}

/// Writes the report of `paths` into `out` and prints each table's row count.
fn write_report(paths: &[PathBuf], out: &Path, prices: Option<NaiveDate>) -> Result<()> {
    let experiments = paths
        .iter()
        .map(|path| Experiment::load(path))
        .collect::<Result<Vec<_>>>()?;
    let runtime = runtime()?;
    let (toolchains, _) = probed(experiments.iter().flat_map(Experiment::languages), &runtime)?;
    let first = paths
        .first()
        .ok_or_else(|| eyre!("no experiment to report"))?;
    let session = Session::start(&repository_of(first)?)?;
    let prices = prices.unwrap_or_else(|| Utc::now().date_naive());
    let manifest = report::report(&experiments, &toolchains, prices, session, out)?;
    print!("{}", ReportTable(manifest.tables()));
    println!("wrote {} at the list prices of {prices}", out.display());
    Ok(())
}

/// What a run would do: each committed run it would withdraw and why, then the trials it would
/// schedule on each backend.
struct PreviewTable<'a>(&'a run::Preview);

impl fmt::Display for PreviewTable<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let run::Preview {
            withdrawals,
            reparsed,
            pending,
        } = self.0;
        writeln!(
            f,
            "would withdraw {} committed runs that are no sample",
            withdrawals.len()
        )?;
        let withdrawn: Vec<[String; 3]> = withdrawals
            .iter()
            .map(|(trial, attempt, reason)| {
                [
                    format!("  {trial}"),
                    format!("attempt {attempt}"),
                    reason.to_string(),
                ]
            })
            .collect();
        write_rows(f, &withdrawn)?;
        writeln!(
            f,
            "would re-derive {reparsed} committed runs with a newer parser"
        )?;
        writeln!(
            f,
            "would schedule {} trials",
            pending.values().sum::<usize>()
        )?;
        let lanes: Vec<[String; 2]> = pending
            .iter()
            .map(|(backend, trials)| [format!("  {backend}"), trials.to_string()])
            .collect();
        write_rows(f, &lanes)?;
        writeln!(f, "references not checked; nothing changed")
    }
}

/// Each table of a report with its row count.
struct ReportTable<'a>(&'a BTreeMap<&'static str, usize>);

impl fmt::Display for ReportTable<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut rows = vec![["table".to_owned(), "rows".to_owned()]];
        rows.extend(
            self.0
                .iter()
                .map(|(table, count)| [(*table).to_owned(), count.to_string()]),
        );
        write_rows(f, &rows)
    }
}

/// Runs an experiment's pending trials, logging progress to stderr. Fails while any trial is
/// still pending, so that a script can tell to run it again. A dry run only prints what a run
/// would do.
fn run_trials(path: &Path, guard: &Guard, dry_run: bool) -> Result<()> {
    log_progress();
    let experiment = Experiment::load(path)?;
    let backends: Vec<&BackendId> = experiment.concurrency().keys().collect();
    for backend in guard.ceilings.keys().chain(guard.deadlines.keys()) {
        ensure!(
            backends.contains(&backend),
            "backend {backend} is not in {}; it uses {}",
            path.display(),
            backends
                .iter()
                .map(|b| b.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let repo = repository_of(path)?;
    let runtime = runtime()?;
    let (toolchains, backend) = probed(experiment.languages(), &runtime)?;
    let home = env_path("HOME")?;
    let runtime_dir = env_path("XDG_RUNTIME_DIR")?;
    let host = Host {
        toolchains: &toolchains,
        backend: &backend,
        home: &home,
        runtime: &runtime_dir,
    };
    if dry_run {
        let preview = runtime.block_on(async {
            let stop = Stop::on_ctrl_c()?;
            run::preview(&experiment, &host, &stop).await
        })?;
        print!("{}", PreviewTable(&preview));
        return Ok(());
    }
    let tally = runtime.block_on(async {
        let stop = Stop::on_ctrl_c()?;
        run::run(&experiment, &repo, &host, guard, &stop).await
    })?;
    let Tally {
        committed,
        discarded,
        failed,
        pending,
    } = tally;
    println!(
        "attempts committed {committed}, discarded {discarded}, failed {failed}; trials pending \
         {pending}"
    );
    ensure!(
        pending == 0,
        "{pending} trials are still pending; run again to continue"
    );
    Ok(())
}

/// Parses `BACKEND[:WINDOW]=FRACTION[@TIME]` with the fraction in (0, 1] and an RFC 3339 time.
fn ceiling(arg: &str) -> Result<(BackendId, Ceiling)> {
    let (target, rest) = arg
        .split_once('=')
        .ok_or_else(|| eyre!("expected BACKEND[:WINDOW]=FRACTION[@TIME], got {arg:?}"))?;
    let (backend, window) = match target.split_once(':') {
        Some((backend, window)) => (backend, Some(window.to_owned())),
        None => (target, None),
    };
    let (fraction, from) = match rest.split_once('@') {
        Some((fraction, time)) => {
            let time = DateTime::parse_from_rfc3339(time)
                .wrap_err_with(|| format!("{time:?} is not an RFC 3339 time"))?;
            (fraction, Some(time.with_timezone(&Utc)))
        }
        None => (rest, None),
    };
    let fraction: f64 = fraction
        .parse()
        .wrap_err_with(|| format!("{fraction:?} is not a number"))?;
    ensure!(
        fraction > 0.0 && fraction <= 1.0,
        "a quota ceiling is a fraction in (0, 1], got {fraction}"
    );
    Ok((
        backend.parse()?,
        Ceiling {
            window,
            fraction,
            from,
        },
    ))
}

/// Parses `BACKEND=TIME` with an RFC 3339 time.
fn deadline(arg: &str) -> Result<(BackendId, DateTime<Utc>)> {
    let (backend, time) = arg
        .split_once('=')
        .ok_or_else(|| eyre!("expected BACKEND=TIME, got {arg:?}"))?;
    let time = DateTime::parse_from_rfc3339(time)
        .wrap_err_with(|| format!("{time:?} is not an RFC 3339 time"))?;
    Ok((backend.parse()?, time.with_timezone(&Utc)))
}

/// Grades an experiment's committed runs, logging each verdict to stderr. Fails while any
/// committed run is left ungraded.
fn grade_trials(path: &Path, jobs: NonZeroUsize) -> Result<()> {
    log_progress();
    let experiment = Experiment::load(path)?;
    let repo = repository_of(path)?;
    let runtime = runtime()?;
    let (toolchains, backend) = probed(experiment.languages(), &runtime)?;
    let graded = runtime.block_on(async {
        let stop = Stop::on_ctrl_c()?;
        grade::grade_runs(&experiment, &repo, &toolchains, &backend, jobs, &stop).await
    })?;
    let Graded {
        passed,
        failed,
        faulted,
        errors,
        unstarted,
    } = graded;
    println!(
        "passed {passed}, failed {failed}, environment faults {faulted}, errors {errors}, not \
         started {unstarted}"
    );
    let left = faulted + errors + unstarted;
    ensure!(left == 0, "{left} runs are still ungraded; grade again");
    Ok(())
}

#[derive(Args)]
struct Measure {
    /// The experiment file, such as experiments/pilot.toml.
    experiment: PathBuf,
    /// Measure only this repetition, such as 1 or r1. Repeat for several.
    #[arg(long = "rep", value_name = "N", value_parser = rep)]
    reps: Vec<Rep>,
    /// Measure only the systems whose id matches this glob, such as '*luna*'. Repeat for
    /// several.
    #[arg(long = "system", value_name = "GLOB")]
    systems: Vec<String>,
    /// ABBA blocks per workload. An even count puts each program first equally often.
    #[arg(long, value_name = "B", default_value_t = Protocol::DEFAULT.blocks)]
    blocks: NonZeroU32,
    /// Timed runs of each program in each block.
    #[arg(long, value_name = "R", default_value_t = Protocol::DEFAULT.runs)]
    runs: NonZeroU32,
    /// Untimed runs of each program before its timed runs in each block.
    #[arg(long, value_name = "W", default_value_t = Protocol::DEFAULT.warmup)]
    warmup: u32,
    /// The CPU every run is pinned to. Defaults to the last one gungnir may run on.
    #[arg(long, value_name = "N")]
    core: Option<u32>,
    /// Measure while other work runs: skip the load check, and do not hold the run and grade
    /// locks. Each record still keeps the load averages it ran under.
    #[arg(long)]
    allow_busy: bool,
}

impl Measure {
    /// Measures the selected trials, logging each committed measurement to stderr. Fails while
    /// any trial it set out to measure is left unmeasured.
    fn run(self) -> Result<()> {
        log_progress();
        let experiment = Experiment::load(&self.experiment)?;
        let repo = repository_of(&self.experiment)?;
        let request = measure::Request {
            selection: Selection::new(&experiment, &self.reps, &self.systems)?,
            core: pinned_core(self.core)?,
            protocol: Protocol {
                blocks: self.blocks,
                runs: self.runs,
                warmup: self.warmup,
            },
            quiet: if self.allow_busy {
                Quiet::Waived
            } else {
                Quiet::Required
            },
        };
        let runtime = runtime()?;
        let (toolchains, backend) = probed(experiment.languages(), &runtime)?;
        let measured = runtime.block_on(async {
            let stop = Stop::on_ctrl_c()?;
            measure::measure_trials(&experiment, &repo, &toolchains, &backend, &request, &stop)
                .await
        })?;
        let Measured {
            committed,
            faulted,
            errors,
            unstarted,
        } = measured;
        println!(
            "committed {committed}, environment faults {faulted}, errors {errors}, not started \
             {unstarted}"
        );
        let left = faulted + errors + unstarted;
        ensure!(
            left == 0,
            "{left} trials are still unmeasured; measure again"
        );
        Ok(())
    }
}

/// Profiles the trial named `name` and prints where the profile went, each program's hottest
/// functions, and what a profile of the language can show.
fn profile(path: &Path, name: &str, workload: Option<&WorkloadId>) -> Result<()> {
    log_progress();
    let experiment = Experiment::load(path)?;
    let trial = experiment
        .trials()
        .into_iter()
        .find(|trial| trial.to_string() == name)
        .ok_or_else(|| {
            eyre!(
                "{} has no trial {name:?}; name one as <system>/<scenario>/<language>/r<k>",
                experiment.name()
            )
        })?;
    let repo = repository_of(path)?;
    let runtime = runtime()?;
    let (toolchains, backend) = probed(experiment.languages(), &runtime)?;
    let profiled = runtime.block_on(async {
        let stop = Stop::on_ctrl_c()?;
        measure::profile_trial(
            &experiment,
            &repo,
            &toolchains,
            &backend,
            &trial,
            workload,
            &stop,
        )
        .await
    })?;
    println!(
        "profiled {trial} on {} into {}",
        profiled.workload,
        profiled.dir.display()
    );
    for (side, hottest) in &profiled.hottest {
        let functions: Vec<String> = hottest
            .iter()
            .map(|(function, share)| format!("{:.1}% {function}", share * 100.0))
            .collect();
        println!(
            "{side}: {side}/flame.svg; most samples in {}",
            functions.join(", ")
        );
    }
    println!("{}: {}", trial.language, profiled.shows);
    Ok(())
}

/// The `[machine]` table that pins measurements to `identity`.
fn machine_pin(identity: &measure::Identity) -> Result<String> {
    let (Some(cpu), Some(host)) = (&identity.cpu, &identity.host) else {
        bail!("this machine is {identity}, which lacks a CPU model or a host id to pin");
    };
    Ok(format!("[machine]\ncpu = {cpu:?}\nhost = {host:?}\n"))
}

/// Parses a repetition as `3` or `r3`.
fn rep(arg: &str) -> Result<Rep> {
    let number: u32 = arg
        .strip_prefix('r')
        .unwrap_or(arg)
        .parse()
        .wrap_err_with(|| format!("{arg:?} is not a repetition such as 1 or r1"))?;
    Rep::new(number).ok_or_else(|| eyre!("repetitions count from 1, got {arg:?}"))
}

/// Logs the phases' progress to stderr, in color only on a terminal.
fn log_progress() {
    // Only fails when a subscriber is already set, which then logs instead.
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .with_target(false)
        .try_init();
}

/// The repository holding the experiment at `path`: the nearest directory above it with a
/// `gungnir.toml`, as `Experiment::load` finds it.
fn repository_of(path: &Path) -> Result<PathBuf> {
    let path = path
        .canonicalize()
        .wrap_err_with(|| format!("reading {}", path.display()))?;
    path.ancestors()
        .skip(1)
        .find(|dir| dir.join("gungnir.toml").is_file())
        .map(Path::to_path_buf)
        .ok_or_else(|| eyre!("no directory above {} has a gungnir.toml", path.display()))
}

fn runtime() -> Result<tokio::runtime::Runtime> {
    tokio::runtime::Runtime::new().wrap_err("starting the async runtime")
}

/// `$GUNGNIR_TOOLCHAINS` with the version of each of `languages` probed, since every prompt
/// states it, and the sandbox backend over its base.
fn probed<'a>(
    languages: impl IntoIterator<Item = &'a Language>,
    runtime: &tokio::runtime::Runtime,
) -> Result<(Toolchains, Bubblewrap)> {
    let mut toolchains = Toolchains::from_env()?;
    let backend = Bubblewrap::new(toolchains.base().clone())?;
    runtime.block_on(toolchains.probe_versions(languages, &backend, &Stop::default()))?;
    Ok((toolchains, backend))
}

fn env_path(name: &str) -> Result<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .ok_or_else(|| eyre!("{name} is not set"))
}

/// A quarter of the CPUs, since one reference's build may use them all.
fn default_jobs() -> NonZeroUsize {
    std::thread::available_parallelism()
        .ok()
        .and_then(|cpus| NonZeroUsize::new(cpus.get() / 4))
        .unwrap_or(NonZeroUsize::MIN)
}

/// Checks the references of the repository holding the current directory, prints the table,
/// and names every reference that did not pass.
fn run_check(scenarios: &[ScenarioId], languages: &[LanguageId], jobs: NonZeroUsize) -> Result<()> {
    let root = repository()?;
    let all_languages = Language::load_all(&root.join("languages"))?;
    let all_scenarios = Scenario::load_all(&root.join("scenarios"), &all_languages)?;
    let selected = references(&all_scenarios, &all_languages, scenarios, languages)?;
    let runtime = runtime()?;
    let (toolchains, backend) = probed(selected.iter().map(|(_, language)| *language), &runtime)?;
    let checked = runtime.block_on(async {
        let stop = Stop::on_ctrl_c()?;
        check(&selected, &toolchains, &backend, &stop, jobs).await
    })?;
    print!(
        "{}",
        CheckTable {
            checked: &checked,
            toolchains: &toolchains,
        }
    );

    let mut failed = 0;
    for checked in &checked {
        let name = format!("{}/{}", checked.scenario.id(), checked.language.id());
        match (&checked.result, &checked.evidence) {
            (Ok(_), None) => continue,
            (Ok(judgement), Some(evidence)) => eprintln!(
                "{name}: {}; build log and outputs in {}",
                judgement.verdict(),
                evidence.display()
            ),
            (Err(error), _) => eprintln!("{name}: {error:#}"),
        }
        failed += 1;
    }
    ensure!(
        failed == 0,
        "{failed} of {} references failed",
        checked.len()
    );
    Ok(())
}

/// The repository that holds the current directory: the nearest directory at or above it with
/// a `gungnir.toml`.
fn repository() -> Result<PathBuf> {
    let cwd = std::env::current_dir().wrap_err("reading the current directory")?;
    cwd.ancestors()
        .find(|dir| dir.join("gungnir.toml").is_file())
        .map(Path::to_owned)
        .ok_or_else(|| {
            eyre!(
                "no directory at or above {} has a gungnir.toml",
                cwd.display()
            )
        })
}

/// Every scenario's reference solution in every language it supports, narrowed to the
/// scenarios and languages named, when any are.
fn references<'a>(
    scenarios: &'a BTreeMap<ScenarioId, Scenario>,
    languages: &'a BTreeMap<LanguageId, Language>,
    only_scenarios: &[ScenarioId],
    only_languages: &[LanguageId],
) -> Result<Vec<(&'a Scenario, &'a Language)>> {
    for id in only_scenarios {
        ensure!(
            scenarios.contains_key(id),
            "no scenario {id} under scenarios/"
        );
    }
    for id in only_languages {
        ensure!(
            languages.contains_key(id),
            "no language {id} under languages/"
        );
    }
    let selected: Vec<_> = scenarios
        .values()
        .filter(|scenario| only_scenarios.is_empty() || only_scenarios.contains(scenario.id()))
        .flat_map(|scenario| {
            scenario
                .languages()
                .filter(|id| only_languages.is_empty() || only_languages.contains(id))
                .map(move |id| (scenario, &languages[id]))
        })
        .collect();
    if selected.is_empty() {
        bail!("no scenario named has a reference in a language named");
    }
    Ok(selected)
}

/// One reference solution, judged.
struct Checked<'a> {
    scenario: &'a Scenario,
    language: &'a Language,
    result: Result<Judgement>,
    /// Where the build log and failing outputs were kept, when the reference did not pass.
    evidence: Option<PathBuf>,
}

/// Ensures the workloads of every scenario in `selected`, then judges each reference, `jobs`
/// at a time. After the first Ctrl-C, references not yet started are not judged.
async fn check<'a>(
    selected: &[(&'a Scenario, &'a Language)],
    toolchains: &Toolchains,
    backend: &dyn Backend,
    stop: &Stop,
    jobs: NonZeroUsize,
) -> Result<Vec<Checked<'a>>> {
    let scenarios: BTreeMap<_, _> = selected
        .iter()
        .map(|(scenario, _)| (scenario.id(), *scenario))
        .collect();
    for scenario in scenarios.values() {
        if workload::ensure(scenario, toolchains, backend, stop).await? == Ensured::Regenerated {
            eprintln!("{}: regenerated the workloads", scenario.id());
        }
    }
    let judge = |scenario: &'a Scenario, language: &'a Language| async move {
        let judged = async {
            ensure!(
                stop.stage() == Stage::Running,
                "interrupted before it started"
            );
            let reference = scenario
                .reference(language.id())
                .ok_or_else(|| eyre!("no {} reference", language.id()))?;
            let evidence = tempfile::Builder::new()
                .prefix("gungnir-check-")
                .tempdir()
                .wrap_err("creating a directory for evidence")?;
            let workspace = reference.dir();
            let judgement = grade::judge(
                language,
                toolchains,
                workspace,
                scenario,
                backend,
                stop,
                evidence.path(),
            )
            .await?;
            Ok((judgement, evidence))
        };
        let (result, evidence) = match judged.await {
            Ok((judgement, _)) if judgement.verdict() == Verdict::Pass => (Ok(judgement), None),
            Ok((judgement, evidence)) => (Ok(judgement), Some(evidence.keep())),
            Err(error) => (Err(error), None),
        };
        Checked {
            scenario,
            language,
            result,
            evidence,
        }
    };
    let mut checked: Vec<Checked> = stream::iter(selected)
        .map(|&(scenario, language)| judge(scenario, language))
        .buffer_unordered(jobs.get())
        .collect()
        .await;
    checked.sort_by_key(|checked| (checked.scenario.id(), checked.language.id()));
    Ok(checked)
}

/// One row per reference: its language's version, its build time, how many hidden cases
/// passed, its verdict, and its slowest case.
struct CheckTable<'a> {
    checked: &'a [Checked<'a>],
    toolchains: &'a Toolchains,
}

impl fmt::Display for CheckTable<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let seconds = |wall: Duration| format!("{:.2}s", wall.as_secs_f64());
        let header = [
            "scenario", "language", "version", "build", "cases", "verdict", "slowest",
        ];
        let mut rows = vec![header.map(String::from)];
        for checked in self.checked {
            let scenario = checked.scenario.id().to_string();
            let language = checked.language.id().to_string();
            let version = self
                .toolchains
                .version(checked.language.id())
                .map_or_else(|_| "-".to_owned(), ToString::to_string);
            let Ok(judgement) = &checked.result else {
                let none = || "-".to_owned();
                rows.push([
                    scenario,
                    language,
                    version,
                    none(),
                    none(),
                    "error".into(),
                    none(),
                ]);
                continue;
            };
            let cases = judgement.cases();
            let passed = cases
                .values()
                .filter(|case| case.outcome == Outcome::Pass)
                .count();
            let slowest = cases.iter().max_by_key(|(_, case)| case.wall).map_or_else(
                || "-".to_owned(),
                |(id, case)| format!("{id} {}", seconds(case.wall)),
            );
            rows.push([
                scenario,
                language,
                version,
                seconds(judgement.build().wall),
                format!("{passed}/{}", checked.scenario.hidden().len()),
                judgement.verdict().to_string(),
                slowest,
            ]);
        }
        write_rows(f, &rows)
    }
}

/// Writes `rows` as left-aligned columns two spaces apart.
fn write_rows<const N: usize>(f: &mut fmt::Formatter<'_>, rows: &[[String; N]]) -> fmt::Result {
    let widths: [usize; N] =
        std::array::from_fn(|column| rows.iter().map(|row| row[column].len()).max().unwrap_or(0));
    for row in rows {
        let cells: Vec<String> = row
            .iter()
            .zip(widths)
            .map(|(cell, width)| format!("{cell:width$}"))
            .collect();
        writeln!(f, "{}", cells.join("  ").trim_end())?;
    }
    Ok(())
}

#[derive(Args)]
struct Bench {
    #[arg(long, value_name = "ID")]
    scenario: ScenarioId,
    #[arg(long, value_name = "ID")]
    language: LanguageId,
    /// The candidate's workspace. Defaults to the reference.
    #[arg(long, value_name = "DIR")]
    candidate: Option<PathBuf>,
    /// The CPU every run is pinned to. Defaults to the last one gungnir may run on.
    #[arg(long, value_name = "N")]
    core: Option<u32>,
    /// ABBA blocks per workload. An even count puts each program first equally often.
    #[arg(long, value_name = "B", default_value_t = Protocol::DEFAULT.blocks)]
    blocks: NonZeroU32,
    /// Timed runs of each program in each block.
    #[arg(long, value_name = "R", default_value_t = Protocol::DEFAULT.runs)]
    runs: NonZeroU32,
    /// Untimed runs of each program before its timed runs in each block.
    #[arg(long, value_name = "W", default_value_t = Protocol::DEFAULT.warmup)]
    warmup: u32,
}

impl Bench {
    /// Ensures the workloads, builds both programs, compares them, commits the record beside
    /// hyperfine's exports in a directory it keeps, and prints the table.
    fn run(self) -> Result<()> {
        let root = repository()?;
        let languages = Language::load_all(&root.join("languages"))?;
        let scenarios = Scenario::load_all(&root.join("scenarios"), &languages)?;
        let scenario = scenarios
            .get(&self.scenario)
            .ok_or_else(|| eyre!("no scenario {} under scenarios/", self.scenario))?;
        let language = languages
            .get(&self.language)
            .ok_or_else(|| eyre!("no language {} under languages/", self.language))?;
        let reference = scenario
            .reference(language.id())
            .ok_or_else(|| eyre!("{} has no {} reference", scenario.id(), language.id()))?
            .dir();
        let candidate = self.candidate.as_deref().unwrap_or(reference);
        let protocol = Protocol {
            blocks: self.blocks,
            runs: self.runs,
            warmup: self.warmup,
        };
        let settings = Settings {
            core: pinned_core(self.core)?,
            protocol,
            memory: scenario.limits().memory_bytes(),
        };
        let session = Session::start(&root)?;
        let toolchains = Toolchains::from_env()?;
        let backend = Bubblewrap::new(toolchains.base().clone())?;
        let kept = tempfile::Builder::new()
            .prefix("gungnir-bench-")
            .tempdir()
            .wrap_err("creating a directory for the record")?;
        let target = kept.path().join("measure");

        let runtime = tokio::runtime::Runtime::new().wrap_err("starting the async runtime")?;
        let record = runtime.block_on(async {
            let stop = Stop::on_ctrl_c()?;
            if workload::ensure(scenario, &toolchains, &backend, &stop).await?
                == Ensured::Regenerated
            {
                eprintln!("{}: regenerated the workloads", scenario.id());
            }
            let build =
                |workspace| Program::build(language, &toolchains, workspace, &backend, &stop);
            let inputs = MeasureInputs::new(language, &toolchains, scenario, candidate, &protocol)?;
            let reference = build(reference).await.wrap_err("building the reference")?;
            let candidate = build(candidate).await.wrap_err("building the candidate")?;
            let partial = PartialDir::begin(&target)?;
            let comparison = measure::compare(
                &reference,
                &candidate,
                scenario.workloads(),
                &settings,
                &backend,
                &stop,
                partial.path(),
            )
            .await?;
            let record = MeasureRecord::new(session.id().clone(), inputs, comparison);
            partial.commit(&record)?;
            Ok::<_, eyre::Report>(record)
        })?;
        print!(
            "{}",
            BenchTable {
                machine: record.machine(),
                workloads: record.workloads(),
            }
        );
        let kept = kept.keep();
        eprintln!(
            "record and hyperfine exports in {}",
            kept.join("measure").display()
        );
        Ok(())
    }
}

/// One system's trials by stage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Counts {
    pending: usize,
    ran: usize,
    /// Trials whose grade passed, measured or not.
    passed: usize,
    failed: usize,
    /// The passing trials with a committed measurement.
    measured: usize,
    /// Trials with a committed record made from other inputs than the current ones.
    stale: usize,
}

impl Add for Counts {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            pending: self.pending + other.pending,
            ran: self.ran + other.ran,
            passed: self.passed + other.passed,
            failed: self.failed + other.failed,
            measured: self.measured + other.measured,
            stale: self.stale + other.stale,
        }
    }
}

/// Counts each system's trials, in the order the experiment lists the systems.
fn status(experiment: &Experiment, toolchains: &Toolchains) -> Result<Vec<(SystemId, Counts)>> {
    let mut systems: Vec<(SystemId, Counts)> = experiment
        .systems()
        .iter()
        .map(|system| (system.id(), Counts::default()))
        .collect();
    for trial in experiment.trials() {
        let dir = experiment.results().trial(&trial);
        let current = Current::of(experiment, &trial, toolchains)?;
        let lifecycle = Lifecycle::read(&dir, &current)?;
        let id = trial.system.id();
        let (_, counts) = systems
            .iter_mut()
            .find(|(system, _)| system == &id)
            .ok_or_else(|| eyre!("experiment lists no system {id}"))?;
        match &lifecycle.stage {
            lifecycle::Stage::Pending => counts.pending += 1,
            lifecycle::Stage::Ran { .. } => counts.ran += 1,
            lifecycle::Stage::Graded { grade, .. }
                if grade.judgement().verdict() == Verdict::Pass =>
            {
                counts.passed += 1;
            }
            lifecycle::Stage::Graded { .. } => counts.failed += 1,
            lifecycle::Stage::Measured { .. } => {
                counts.passed += 1;
                counts.measured += 1;
            }
        }
        counts.stale += usize::from(lifecycle.stale.is_some());
    }
    Ok(systems)
}

/// One row per system with its counts, then their total.
struct StatusTable<'a>(&'a [(SystemId, Counts)]);

impl fmt::Display for StatusTable<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let header = [
            "system", "pending", "ran", "pass", "fail", "measured", "stale",
        ];
        let total = self
            .0
            .iter()
            .fold(Counts::default(), |sum, (_, counts)| sum + *counts);
        let systems = self.0.iter().map(|(id, counts)| (id.to_string(), *counts));
        let mut rows = vec![header.map(String::from)];
        for (system, counts) in systems.chain([("total".to_owned(), total)]) {
            let Counts {
                pending,
                ran,
                passed,
                failed,
                measured,
                stale,
            } = counts;
            let n = |count: usize| count.to_string();
            rows.push([
                system,
                n(pending),
                n(ran),
                n(passed),
                n(failed),
                n(measured),
                n(stale),
            ]);
        }
        write_rows(f, &rows)
    }
}

/// `core` when gungnir may run on it, or else the last CPU gungnir may run on.
fn pinned_core(core: Option<u32>) -> Result<u32> {
    let affinity = sched_getaffinity(Pid::from_raw(0)).wrap_err("reading the CPU affinity")?;
    let allowed: Vec<u32> = (0..CpuSet::count())
        .filter(|&cpu| affinity.is_set(cpu).unwrap_or(false))
        .filter_map(|cpu| u32::try_from(cpu).ok())
        .collect();
    match core {
        Some(core) => {
            ensure!(
                allowed.contains(&core),
                "gungnir may not run on CPU {core}; it may run on {allowed:?}"
            );
            Ok(core)
        }
        None => allowed
            .last()
            .copied()
            .ok_or_else(|| eyre!("gungnir may run on no CPU")),
    }
}

/// One row per workload: each program's median wall time and range, the ratio of the
/// candidate's time to the reference's with its 95% interval, the ratio of their instruction
/// counts, and their peak memory. Then the machine it ran on.
struct BenchTable<'a> {
    machine: &'a Machine,
    workloads: &'a BTreeMap<WorkloadId, measure::Outcome>,
}

impl fmt::Display for BenchTable<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let header = [
            "workload",
            "reference",
            "range",
            "candidate",
            "range",
            "ratio",
            "95% interval",
            "instructions",
            "peak memory",
        ];
        let mut rows = vec![header.map(String::from)];
        for (id, outcome) in self.workloads {
            let none = || "-".to_owned();
            rows.push(match outcome {
                measure::Outcome::Timed(timed) => {
                    let Summary {
                        reference,
                        candidate,
                        ratio,
                        instructions,
                        peak_memory,
                    } = timed.summary();
                    let interval = ratio
                        .interval
                        .map_or_else(none, |(low, high)| format!("[{low:.3}, {high:.3}]"));
                    [
                        id.to_string(),
                        seconds(reference.median),
                        range(reference),
                        seconds(candidate.median),
                        range(candidate),
                        format!("{:.3}", ratio.geomean),
                        interval,
                        instructions.map_or_else(none, |ratio| format!("{ratio:.3}")),
                        peak_memory.to_string(),
                    ]
                }
                measure::Outcome::CandidateFailed(failure) => [
                    id.to_string(),
                    none(),
                    none(),
                    format!("failed: {failure}"),
                    none(),
                    none(),
                    none(),
                    none(),
                    none(),
                ],
            });
        }
        write_rows(f, &rows)?;
        let machine = self.machine;
        let load = |load: [f64; 3]| format!("{:.2} {:.2} {:.2}", load[0], load[1], load[2]);
        writeln!(
            f,
            "pinned to CPU {} of {} ({}), governor {}",
            machine.core,
            machine.cpus,
            machine.cpu.as_deref().unwrap_or("unknown model"),
            machine.governor.as_deref().unwrap_or("unknown")
        )?;
        writeln!(
            f,
            "load average {} at the start, {} at the end",
            load(machine.load.start),
            load(machine.load.end)
        )
    }
}

fn seconds(seconds: f64) -> String {
    if seconds >= 1.0 {
        format!("{seconds:.3}s")
    } else {
        format!("{:.1}ms", seconds * 1000.0)
    }
}

fn range(spread: Spread) -> String {
    format!("[{}, {}]", seconds(spread.min), seconds(spread.max))
}

/// An experiment's matrix as aligned text, one row per model with its efforts.
struct Plan<'a>(&'a Experiment);

impl fmt::Display for Plan<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let experiment = self.0;
        let budget = experiment.budget();
        let concurrency: Vec<String> = experiment
            .concurrency()
            .iter()
            .map(|(backend, limit)| format!("{backend} {limit}"))
            .collect();
        writeln!(f, "experiment   {}", experiment.name())?;
        writeln!(f, "results      {}", experiment.results().path().display())?;
        writeln!(
            f,
            "budget       {} wall, {} idle",
            humantime::format_duration(budget.wall()),
            humantime::format_duration(budget.idle())
        )?;
        writeln!(f, "concurrency  {}", concurrency.join(", "))?;

        let systems = experiment.systems();
        let rows: Vec<[String; 4]> = systems
            .chunk_by(|a, b| {
                (a.harness(), a.harness_version(), a.backend(), a.model())
                    == (b.harness(), b.harness_version(), b.backend(), b.model())
            })
            .map(|group| {
                let first = &group[0];
                let efforts: Vec<&str> = group
                    .iter()
                    .map(|system| system.effort().as_str())
                    .collect();
                [
                    format!("{} {}", first.harness(), first.harness_version()),
                    first.backend().to_string(),
                    first.model().to_string(),
                    efforts.join(" "),
                ]
            })
            .collect();
        let width = |column: usize| rows.iter().map(|row| row[column].len()).max().unwrap_or(0);
        let (harness_width, backend_width, model_width) = (width(0), width(1), width(2));
        writeln!(f, "systems      {}", systems.len())?;
        for [harness, backend, model, efforts] in &rows {
            writeln!(
                f,
                "  {harness:harness_width$}  {backend:backend_width$}  {model:model_width$}  {efforts}"
            )?;
        }

        let scenarios: Vec<&str> = experiment
            .scenarios()
            .iter()
            .map(|s| s.id().as_str())
            .collect();
        let languages: Vec<&str> = experiment
            .languages()
            .iter()
            .map(|l| l.id().as_str())
            .collect();
        writeln!(f, "scenarios    {}", scenarios.join(" "))?;
        writeln!(f, "languages    {}", languages.join(" "))?;
        writeln!(f, "repetitions  {}", experiment.repetitions())?;
        writeln!(f, "trials       {}", experiment.trials().len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        grade::fake::{self, Scripted, finished},
        measure::{Block, Failure, Load, PeakMemory, Sample, Timed},
        sandbox::{Exit, Input},
        testing::{self, id, write},
    };
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn bench_prints_each_workload_then_the_machine() {
        let runs = |wall: f64| {
            let sample = Sample {
                wall,
                user: wall,
                system: 0.0,
                peak_memory: 1 << 20,
                instructions: Some((wall * 1e9) as u64),
            };
            vec![sample; 2]
        };
        let block = |reference, candidate| Block {
            reference: runs(reference),
            candidate: runs(candidate),
        };
        let timed = Timed {
            blocks: vec![block(1.0, 2.0), block(0.5, 1.0)],
            peak_memory: PeakMemory {
                reference: Some(48 << 20),
                candidate: None,
            },
        };
        let workloads = BTreeMap::from([
            (id("full-range"), measure::Outcome::Timed(timed)),
            (
                id("uniform"),
                measure::Outcome::CandidateFailed(Failure::WrongOutput),
            ),
        ]);
        let machine = Machine {
            cpu: Some("AMD Ryzen 7 5800U".into()),
            host: None,
            cpus: 16,
            core: 15,
            governor: None,
            kernel: "7.3.0".into(),
            perf_event_paranoid: Some(1),
            load: Load {
                start: [6.65, 4.26, 3.28],
                end: [7.0, 5.0, 3.5],
            },
        };
        let table = BenchTable {
            machine: &machine,
            workloads: &workloads,
        };

        assert_eq!(
            table.to_string(),
            "workload    reference  range              candidate             range             ratio  95% interval    instructions  peak memory\n\
             full-range  750.0ms    [500.0ms, 1.000s]  1.500s                [1.000s, 2.000s]  2.000  [2.000, 2.000]  2.000         - (reference 48.0MiB)\n\
             uniform     -          -                  failed: wrong output  -                 -      -               -             -\n\
             pinned to CPU 15 of 16 (AMD Ryzen 7 5800U), governor unknown\n\
             load average 6.65 4.26 3.28 at the start, 7.00 5.00 3.50 at the end\n"
        );
    }

    #[test]
    fn bench_pins_only_to_a_cpu_gungnir_may_run_on() {
        let last = pinned_core(None).unwrap();
        assert_eq!(pinned_core(Some(last)).unwrap(), last);
        let message = pinned_core(Some(4096)).unwrap_err().to_string();
        assert!(message.contains("may not run on CPU 4096"), "{message}");
    }

    /// The fixture repository with languages `python` and `rust` and no lizard, so judging
    /// needs no host tool, and fake toolchains for both.
    struct CheckFixture {
        _root: TempDir,
        languages: BTreeMap<LanguageId, Language>,
        scenarios: BTreeMap<ScenarioId, Scenario>,
        toolchains: Toolchains,
    }

    fn check_fixture(extra_languages: &[&str]) -> CheckFixture {
        let root = TempDir::new().unwrap();
        testing::repo(root.path());
        for language in ["python", "rust"].iter().chain(extra_languages) {
            testing::language(root.path(), language);
            let toml = testing::LANGUAGE_TOML.replace("lizard = \"python\"\n", "");
            write(
                root.path(),
                &format!("languages/{language}/language.toml"),
                toml,
            );
        }
        let languages = Language::load_all(&root.path().join("languages")).unwrap();
        let scenarios = Scenario::load_all(&root.path().join("scenarios"), &languages).unwrap();
        let toolchains = fake::toolchains(&root.path().join("toolchains"), &["python", "rust"]);
        CheckFixture {
            _root: root,
            languages,
            scenarios,
            toolchains,
        }
    }

    #[tokio::test]
    async fn check_judges_each_reference_and_tables_the_results() {
        let fixture = check_fixture(&[]);
        let backend = Scripted::new(|spec, _, io| {
            let Input::File(input) = io.stdin else {
                return Ok(finished(Exit::Code(0), b"built\n"));
            };
            let rust = fixture_toolchain(&spec.closure, "rust");
            let stdout = if rust {
                b"wrong\n".to_vec()
            } else {
                fs::read(input).unwrap()
            };
            Ok(finished(Exit::Code(0), &stdout))
        });
        let selected = references(&fixture.scenarios, &fixture.languages, &[], &[]).unwrap();
        let jobs = NonZeroUsize::new(2).unwrap();
        let stop = Stop::default();
        let checked = check(&selected, &fixture.toolchains, &backend, &stop, jobs)
            .await
            .unwrap();

        let table = CheckTable {
            checked: &checked,
            toolchains: &fixture.toolchains,
        };
        assert_eq!(
            table.to_string(),
            "scenario  language  version  build  cases  verdict       slowest\n\
             echo      python    1.0      0.01s  2/2    pass          many 0.01s\n\
             echo      rust      1.0      0.01s  0/2    wrong-answer  many 0.01s\n"
        );
        assert!(checked[0].evidence.is_none());
        let evidence = checked[1].evidence.clone().unwrap();
        assert_eq!(
            fs::read(evidence.join("cases/edge.out")).unwrap(),
            b"wrong\n"
        );
        fs::remove_dir_all(evidence).unwrap();
    }

    fn fixture_toolchain(closure: &crate::toolchains::Closure, language: &str) -> bool {
        let env = format!("bbbb-gungnir-language-{language}");
        closure.paths().any(|path| path.ends_with(&env))
    }

    #[test]
    fn check_selects_references_by_scenario_and_language() {
        let fixture = check_fixture(&["zig"]);
        let select = |scenarios: &[&str], languages: &[&str]| {
            let scenarios: Vec<ScenarioId> = scenarios.iter().map(|s| id(s)).collect();
            let languages: Vec<LanguageId> = languages.iter().map(|l| id(l)).collect();
            references(
                &fixture.scenarios,
                &fixture.languages,
                &scenarios,
                &languages,
            )
            .map(|selected| {
                selected
                    .iter()
                    .map(|(scenario, language)| format!("{}/{}", scenario.id(), language.id()))
                    .collect::<Vec<_>>()
            })
        };
        assert_eq!(select(&[], &[]).unwrap(), ["echo/python", "echo/rust"]);
        assert_eq!(select(&["echo"], &["rust"]).unwrap(), ["echo/rust"]);
        for (scenarios, languages, needle) in [
            (&["word-freq"][..], &[][..], "word-freq"),
            (&[][..], &["cobol"][..], "cobol"),
            (&[][..], &["zig"][..], "no scenario named has a reference"),
        ] {
            let message = select(scenarios, languages).unwrap_err().to_string();
            assert!(message.contains(needle), "{message}");
        }
    }

    #[test]
    fn plan_lists_each_model_with_its_efforts_and_counts_the_trials() {
        let root = TempDir::new().unwrap();
        testing::repo(root.path());
        write(
            root.path(),
            "experiments/trial.toml",
            testing::EXPERIMENT_TOML
                .replace(
                    "[[systems]]\nmodel = \"luna\"",
                    "[[systems]]\nmodel = \"glm\"\nbackend = \"zai\"\nefforts = [\"low\"]\n\n\
                 [[systems]]\nmodel = \"luna\"",
                )
                .replace("openai = 1", "openai = 1\nzai = 3"),
        );
        let experiment = Experiment::load(&root.path().join("experiments/trial.toml")).unwrap();
        let results = experiment.results().path().display().to_string();

        assert_eq!(
            Plan(&experiment).to_string(),
            format!(
                "experiment   trial-run\n\
                 results      {results}\n\
                 budget       40m wall, 10m idle\n\
                 concurrency  anthropic 2, openai 1, zai 3\n\
                 systems      5\n  \
                 claude-code 2.1.316  anthropic  opus  low high\n  \
                 claude-code 2.1.316  zai        glm   low\n  \
                 codex 0.166.0        openai     luna  none low\n\
                 scenarios    echo\n\
                 languages    python rust\n\
                 repetitions  2\n\
                 trials       20\n"
            )
        );
    }

    #[tokio::test]
    async fn status_counts_trials_by_stage_and_never_attempts() {
        use crate::{id::AttemptNo, run::fixture};
        let repo = fixture::Repo::new();
        let experiment = repo.experiment();
        let (opus, luna) = (
            fixture::trial(&experiment, "opus"),
            fixture::trial(&experiment, "luna"),
        );
        let opus_dir = experiment.results().trial(&opus);
        write(
            opus_dir.attempt(AttemptNo::FIRST).path(),
            "transcript.jsonl",
            "",
        );
        fixture::commit_run(&repo, &opus, &[("main.py", "print(input())\n")]);
        fixture::commit_grade(&repo, &opus).await;
        let luna_dir = experiment.results().trial(&luna);
        write(
            luna_dir.attempt(AttemptNo::FIRST).path(),
            "discarded.json",
            "{}",
        );
        write(
            luna_dir.attempt(AttemptNo::FIRST.next()).path(),
            "transcript.jsonl",
            "",
        );
        write(repo.root.path(), "scenarios/echo/hidden/edge.out", "x\n");

        let rows = status(&repo.experiment(), &repo.toolchains).unwrap();
        let counts: Vec<(String, Counts)> = rows
            .iter()
            .map(|(system, counts)| (system.to_string(), *counts))
            .collect();
        assert_eq!(
            counts,
            [
                (
                    opus.system.id().to_string(),
                    Counts {
                        passed: 1,
                        stale: 1,
                        ..Counts::default()
                    }
                ),
                (
                    luna.system.id().to_string(),
                    Counts {
                        pending: 1,
                        ..Counts::default()
                    }
                ),
            ]
        );
        let table = StatusTable(&rows).to_string();
        let header: Vec<&str> = table.lines().next().unwrap().split_whitespace().collect();
        assert_eq!(
            header,
            [
                "system", "pending", "ran", "pass", "fail", "measured", "stale"
            ]
        );
        let total: Vec<&str> = table.lines().last().unwrap().split_whitespace().collect();
        assert_eq!(total, ["total", "1", "0", "1", "0", "0", "1"]);
    }
}
