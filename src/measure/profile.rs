//! Profiles that explain an outlier: a trial's candidate and the reference in its language, each
//! run once under `perf record` on one workload in an offline sandbox, with the samples folded
//! into stacks and drawn as a flame graph. Profiling is not measuring, and no stage reads it.
use super::{Outcome, Program, RUN_TIMEOUT, Side, TASKS, machine};
use crate::{
    experiment::Experiment,
    grade::Verdict,
    id::WorkloadId,
    language::{CallGraph, Language},
    lifecycle::{Current, Lifecycle, Stage},
    provenance::{Session, SessionId},
    sandbox::{Backend, Input, Io, Kill, Limits, Mount, Output},
    secret::EnvValue,
    stop::Stop,
    store::PartialDir,
    toolchains::{Closure, Toolchains},
    trial::{Phase, TrialKey},
    workload::{self, Ensured},
};
use eyre::{Result, WrapErr, bail, ensure, eyre};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};
use tracing::info;

/// Where the sandbox writes one program's profile.
const OUT: &str = "/profile";
/// perf's samples per second, a quarter of its default, so that a DWARF profile copies about
/// 8 MiB of stack per second of run.
const FREQUENCY: u32 = 999;
/// Memory for perf beside the program it samples.
const PERF_MEMORY: u64 = 1 << 30;
/// How many times its usual limit a profiled run may take, since unwinding slows it.
const SLOWDOWN: u32 = 5;
/// How much of perf's log an error quotes.
const LOG_TAIL: usize = 2 << 10;
/// The highest `kernel.perf_event_paranoid` that lets an unprivileged perf sample its own
/// processes in user space.
const PERF_PARANOID: i32 = 2;

/// `profile/<workload>/record.json`, beside `candidate/` and `reference/`, which each hold
/// `perf.data`, `perf.log`, `folded.txt` and `flame.svg`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProfileRecord {
    session: SessionId,
    workload: WorkloadId,
    call_graph: CallGraph,
    /// Samples per second.
    frequency: u32,
    perf_event_paranoid: Option<i32>,
    /// What a profile of a program in this language shows, and what it cannot.
    shows: String,
}

/// A committed profile.
pub struct Profiled {
    pub workload: WorkloadId,
    pub dir: PathBuf,
    /// Each program's functions that most samples landed in, with their share of the samples.
    pub hottest: Vec<(Side, Vec<(String, f64)>)>,
    pub shows: String,
}

/// Profiles the candidate of `trial` and the reference on `workload`, by default the workload
/// whose wall-time ratio was worst, or the first one before the trial is measured. The trial's
/// grade must have passed. Holds the measure lock, so it never runs beside a measurement.
pub async fn profile_trial(
    experiment: &Experiment,
    repo: &Path,
    toolchains: &Toolchains,
    backend: &dyn Backend,
    trial: &TrialKey,
    workload: Option<&WorkloadId>,
    stop: &Stop,
) -> Result<Profiled> {
    let results = experiment.results();
    let _lock = results.lock(Phase::Measure)?;
    let paranoid = machine::perf_event_paranoid();
    if let Some(level) = paranoid.filter(|&level| level > PERF_PARANOID) {
        bail!(
            "perf may not sample: kernel.perf_event_paranoid is {level}, and profiling needs \
             {PERF_PARANOID} or lower, as `sysctl kernel.perf_event_paranoid=1` sets"
        );
    }
    let dir = results.trial(trial);
    let current = Current::of(experiment, trial, toolchains)?;
    let lifecycle = Lifecycle::read(&dir, &current)?;
    let worst = match &lifecycle.stage {
        Stage::Graded { grade, .. } if grade.judgement().verdict() == Verdict::Pass => None,
        Stage::Measured { measure, .. } => worst_workload(measure.workloads()),
        _ => bail!("{trial} has no passing grade to profile"),
    };
    let (language, scenario) = (current.language(), current.scenario());
    let workload = match workload.or(worst.as_ref()) {
        Some(id) => id.clone(),
        None => scenario
            .workloads()
            .keys()
            .next()
            .cloned()
            .ok_or_else(|| eyre!("{} has no workloads", scenario.id()))?,
    };
    let input = &scenario
        .workloads()
        .get(&workload)
        .ok_or_else(|| eyre!("{} has no workload {workload}", scenario.id()))?
        .path;
    if workload::ensure(scenario, toolchains, backend, stop).await? == Ensured::Regenerated {
        info!("{}: regenerated the workloads", scenario.id());
    }

    let session = Session::start(repo)?.with_toolchains(vec![
        ("base".to_owned(), toolchains.base()),
        ("profile".to_owned(), toolchains.profile()?),
        (
            format!("language/{}", language.id()),
            toolchains.language(language.id().as_str())?,
        ),
    ]);
    session.write(results)?;
    let reference = scenario
        .reference(language.id())
        .ok_or_else(|| eyre!("{} has no {} reference", scenario.id(), language.id()))?;
    let reference = Program::build(language, toolchains, reference.dir(), backend, stop)
        .await
        .wrap_err("building the reference")?;
    let candidate = Program::build(language, toolchains, &dir.final_workspace(), backend, stop)
        .await
        .wrap_err("building the candidate")?;

    let target = dir.profile(&workload);
    results.retract(Phase::Measure, &target)?;
    let partial = PartialDir::begin(&target)?;
    let sampler = Sampler {
        language,
        toolchains,
        input,
        memory: scenario.limits().memory_bytes().saturating_add(PERF_MEMORY),
        backend,
        stop,
    };
    let mut hottest = Vec::new();
    for (side, program) in [(Side::Candidate, &candidate), (Side::Reference, &reference)] {
        let out = partial.path().join(side.to_string());
        let title = format!("{trial} {side} on {workload}");
        sampler
            .sample(program, &title, &out)
            .await
            .wrap_err_with(|| format!("profiling the {side}"))?;
        let folded = fs::read_to_string(out.join("folded.txt"))
            .wrap_err_with(|| format!("reading {}", out.join("folded.txt").display()))?;
        hottest.push((side, self_time(&folded, 5)));
    }
    let profile = language.profile();
    partial.commit(&ProfileRecord {
        session: session.id().clone(),
        workload: workload.clone(),
        call_graph: profile.call_graph(),
        frequency: FREQUENCY,
        perf_event_paranoid: paranoid,
        shows: profile.shows().to_owned(),
    })?;
    Ok(Profiled {
        workload,
        dir: target,
        hottest,
        shows: profile.shows().to_owned(),
    })
}

/// The timed workload on which the candidate's wall time was the most times the reference's.
fn worst_workload(workloads: &BTreeMap<WorkloadId, Outcome>) -> Option<WorkloadId> {
    workloads
        .iter()
        .filter_map(|(id, outcome)| match outcome {
            Outcome::Timed(timed) => Some((id, timed.summary().ratio.geomean)),
            Outcome::CandidateFailed(_) => None,
        })
        .max_by(|(_, a), (_, b)| a.total_cmp(b))
        .map(|(id, _)| id.clone())
}

/// Runs programs under perf in offline sandboxes.
struct Sampler<'a> {
    language: &'a Language,
    toolchains: &'a Toolchains,
    /// The workload, fed on stdin.
    input: &'a Path,
    /// The memory of one profiled run, perf's included.
    memory: u64,
    backend: &'a dyn Backend,
    stop: &'a Stop,
}

impl Sampler<'_> {
    /// Runs `program` once on the workload under `perf record`, and writes into `out` the
    /// samples, perf's log, the folded stacks and a flame graph titled `title`.
    async fn sample(&self, program: &Program<'_>, title: &str, out: &Path) -> Result<()> {
        fs::create_dir_all(out).wrap_err_with(|| format!("creating {}", out.display()))?;
        let limits = Limits {
            wall: RUN_TIMEOUT.saturating_mul(SLOWDOWN),
            memory: self.memory,
            tasks: TASKS,
            cpu: None,
        };
        let mut spec = program.spec(self.input, limits);
        let tools = self.toolchains.profile()?;
        spec.closure = Closure::union([&spec.closure, tools.closure()]);
        spec.path.push(tools.bin());
        spec.mounts.push(Mount::writable(out, OUT));
        let profile = self.language.profile();
        for (name, value) in profile.env() {
            spec.env
                .push((name.clone(), EnvValue::Plain(value.clone())));
        }
        let io = Io {
            stdin: Input::File(self.input.to_owned()),
            stdout: Output::Capture { cap: 0 },
            stderr: Output::Capture { cap: 0 },
            kill: Kill::default(),
        };
        let argv: Vec<OsString> = ["sh", "-c", &script(profile.call_graph()), "sh", title]
            .into_iter()
            .chain(program.run.iter().map(String::as_str))
            .map(OsString::from)
            .collect();
        let finished = self.backend.exec(&spec, &argv, io, self.stop).await?;
        if !finished.exit.success() {
            let log = fs::read(out.join("perf.log")).unwrap_or_default();
            let tail = String::from_utf8_lossy(&log[log.len().saturating_sub(LOG_TAIL)..]);
            bail!("perf ended {:?}:\n{}", finished.exit, tail.trim_end());
        }
        ensure!(
            out.join("flame.svg").is_file(),
            "perf wrote no flame graph into {}",
            out.display()
        );
        Ok(())
    }
}

/// The shell script that profiles `$@` with the workload on stdin, then folds the samples and
/// draws them. `perf script` runs in the same sandbox, so it finds the symbol maps a runtime
/// such as CPython writes to `/tmp`.
fn script(call_graph: CallGraph) -> String {
    let call_graph = call_graph.as_str();
    format!(
        "set -eu -o pipefail
title=$1
shift
perf record -F {FREQUENCY} --call-graph {call_graph} -o {OUT}/perf.data -- \"$@\" >/dev/null 2>{OUT}/perf.log
perf script -i {OUT}/perf.data 2>>{OUT}/perf.log | inferno-collapse-perf >{OUT}/folded.txt
inferno-flamegraph --title \"$title\" {OUT}/folded.txt >{OUT}/flame.svg 2>>{OUT}/perf.log
"
    )
}

/// The `count` functions that most samples landed in, by the innermost frame of each folded
/// stack, with their share of all samples.
fn self_time(folded: &str, count: usize) -> Vec<(String, f64)> {
    let mut by_leaf: BTreeMap<&str, u64> = BTreeMap::new();
    let mut total = 0u64;
    for line in folded.lines() {
        let Some((stack, weight)) = line.rsplit_once(' ') else {
            continue;
        };
        let Ok(weight) = weight.parse::<u64>() else {
            continue;
        };
        let leaf = stack.rsplit(';').next().unwrap_or(stack);
        *by_leaf.entry(leaf).or_default() += weight;
        total += weight;
    }
    let mut leaves: Vec<(&str, u64)> = by_leaf.into_iter().collect();
    leaves.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    leaves
        .into_iter()
        .take(count)
        .map(|(leaf, weight)| (leaf.to_owned(), weight as f64 / total as f64))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        grade::fake::{Scripted, finished},
        run::fixture::{self, Repo},
        sandbox::{Exit, Fault, Finished, Spec},
        store::read_record,
        testing::id,
    };

    /// Builds succeed, and a profiled run leaves what perf and inferno would.
    fn sampled(spec: &Spec, argv: &[String], _io: Io) -> Result<Finished, Fault> {
        if argv[2].contains("perf record") {
            let out = spec
                .mounts
                .iter()
                .find(|mount| mount.writable && mount.guest == Path::new(OUT))
                .expect("somewhere to write the profile");
            let folded = "python;main;py::count 3\npython;main 1\n";
            for (file, contents) in [("perf.data", "samples"), ("folded.txt", folded)] {
                fs::write(out.host.join(file), contents).unwrap();
            }
            fs::write(out.host.join("flame.svg"), "<svg/>").unwrap();
        }
        Ok(finished(Exit::Code(0), b""))
    }

    #[tokio::test]
    async fn a_passing_trial_is_profiled_beside_the_reference_and_others_are_refused() {
        let repo = Repo::new();
        let experiment = repo.experiment();
        let (opus, luna) = (
            fixture::trial(&experiment, "opus"),
            fixture::trial(&experiment, "luna"),
        );
        fixture::commit_run(&repo, &opus, &[("main.py", "print(input())\n")]);
        fixture::commit_grade(&repo, &opus).await;
        let backend = Scripted::new(sampled);
        let stop = Stop::default();
        let profile = |trial| {
            let (root, toolchains) = (repo.root.path(), &repo.toolchains);
            profile_trial(&experiment, root, toolchains, &backend, trial, None, &stop)
        };
        if machine::perf_event_paranoid().is_some_and(|level| level > PERF_PARANOID) {
            let message = format!("{:#}", profile(&opus).await.err().unwrap());
            assert!(message.contains("kernel.perf_event_paranoid"), "{message}");
            return;
        }

        let message = format!("{:#}", profile(&luna).await.err().unwrap());
        assert!(message.contains("has no passing grade"), "{message}");
        let profiled = profile(&opus).await.unwrap();
        assert_eq!(profiled.workload, id("large"));
        assert_eq!(
            profiled.hottest,
            [Side::Candidate, Side::Reference].map(|side| (
                side,
                vec![("py::count".to_owned(), 0.75), ("main".to_owned(), 0.25)]
            ))
        );
        let target = experiment.results().trial(&opus).profile(&id("large"));
        assert_eq!(profiled.dir, target);
        let record: ProfileRecord = read_record(&target).unwrap().unwrap();
        assert_eq!(record.call_graph, CallGraph::Fp);
        for side in ["candidate", "reference"] {
            assert!(target.join(side).join("flame.svg").is_file(), "{side}");
        }
        let profiled_runs: Vec<Spec> = backend
            .calls()
            .into_iter()
            .filter(|(_, argv)| argv[2].contains("perf record"))
            .map(|(spec, _)| spec)
            .collect();
        assert_eq!(profiled_runs.len(), 2);
        for spec in profiled_runs {
            assert!(matches!(spec.network, crate::sandbox::Network::Offline));
            assert!(
                spec.closure
                    .contains(repo.toolchains.profile().unwrap().root())
            );
            let perf = spec
                .env
                .iter()
                .find(|(name, _)| name == "PYTHONPERFSUPPORT");
            assert!(matches!(perf, Some((_, EnvValue::Plain(value))) if value == "1"));
        }
    }

    #[test]
    fn self_time_ranks_innermost_frames_by_their_share() {
        let folded = "main;parse;memchr 30\nmain;count;hash 50\nmain;parse 10\nmain;count;hash 10\nbad line\n";
        assert_eq!(
            self_time(folded, 2),
            [("hash".to_owned(), 0.6), ("memchr".to_owned(), 0.3)]
        );
        assert!(self_time("", 3).is_empty());
    }

    #[test]
    fn the_script_records_with_the_language_call_graph_then_folds_and_draws() {
        let script = script(CallGraph::Dwarf);
        assert!(
            script
                .contains("perf record -F 999 --call-graph dwarf -o /profile/perf.data -- \"$@\""),
            "{script}"
        );
        assert!(script.contains("| inferno-collapse-perf >/profile/folded.txt"));
        assert!(script.contains("inferno-flamegraph --title \"$title\""));
        assert!(script.starts_with("set -eu -o pipefail"));
    }
}
