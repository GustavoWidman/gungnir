//! The run phase: each pending trial's agent, launched headless in a sandbox, until one attempt
//! is committed as the trial's sample.
mod attempt;
#[cfg(test)]
pub(crate) mod fixture;
mod gate;
mod records;
mod reparse;
mod schedule;
mod workspace;

pub use attempt::{Attempted, Rig};
pub use records::{
    AgentExit, Cut, Outcome, Process, ProcessExit, Quota, Reason, RunInputs, RunRecord, Sample,
    Tokens,
};
pub use schedule::{Ceiling, Guard, Tally};

use crate::{
    catalog::{Backend, System},
    experiment::Experiment,
    grade::{self, GradeInputs, GradeRecord, Verdict},
    harness::{Harness, LaunchRequest},
    id::{BackendId, HarnessName, HarnessVersion},
    language::Language,
    lifecycle::{Current, Lifecycle},
    provenance::{Session, SessionId},
    sandbox::{self, HOME, Input, Io, Kill, Limits, Network, Output, Spec, WORK},
    scenario::{Brief, Scenario},
    stop::Stop,
    store::{Digest, PartialDir, read_record, sweep_partial, write_json},
    toolchains::{Closure, Toolchain, Toolchains},
    trial::{Phase, ResultsDir, TrialKey},
    vault::Vault,
};
use eyre::{Result, WrapErr, ensure, eyre};
use reparse::{Reparsed, reparse};
use schedule::schedule;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    time::Duration,
};
use tracing::info;

/// What a harness CLI may use to print its version.
const PROBE_LIMITS: Limits = Limits {
    wall: Duration::from_secs(60),
    memory: 1 << 30,
    tasks: 256,
    cpu: None,
};
const PROBE_OUTPUT: u64 = 64 << 10;

/// One trial as the run phase sees it: the system, and what its agent is given. It holds the
/// scenario's brief and never the scenario, so nothing in it leads to a hidden case.
pub struct Task<'a> {
    pub trial: &'a TrialKey,
    pub language: &'a Language,
    pub brief: &'a Brief,
    pub backend: &'a Backend,
}

impl<'a> Task<'a> {
    pub fn of(experiment: &'a Experiment, trial: &'a TrialKey) -> Result<Self> {
        let language = experiment
            .languages()
            .iter()
            .find(|language| language.id() == &trial.language)
            .ok_or_else(|| eyre!("experiment has no language {}", trial.language))?;
        let scenario = experiment
            .scenarios()
            .iter()
            .find(|scenario| scenario.id() == &trial.scenario)
            .ok_or_else(|| eyre!("experiment has no scenario {}", trial.scenario))?;
        let backend = trial.system.backend();
        let backend = experiment
            .catalog()
            .backend(backend)
            .ok_or_else(|| eyre!("catalog has no backend {backend}"))?;
        Ok(Self {
            trial,
            language,
            brief: scenario.brief(),
            backend,
        })
    }
}

/// What the run phase uses of the machine it runs on.
pub struct Host<'a> {
    pub toolchains: &'a Toolchains,
    pub backend: &'a dyn sandbox::Backend,
    /// The user's home, which `~` in a credential path names.
    pub home: &'a Path,
    /// `$XDG_RUNTIME_DIR`, under which isolated homes live.
    pub runtime: &'a Path,
}

/// `experiment.json`.
#[derive(Serialize)]
struct Manifest<'a> {
    name: &'a str,
    first_session: &'a SessionId,
}

/// Runs every pending trial of `experiment`, held in the repository `repo`, until each has a
/// committed run, `stop` drains, or an attempt fails. Refuses to start when a harness CLI is
/// not at its pinned version, a reference does not pass, or a committed run is stale.
pub async fn run(
    experiment: &Experiment,
    repo: &Path,
    host: &Host<'_>,
    guard: &Guard,
    stop: &Stop,
) -> Result<Tally> {
    let results = experiment.results();
    let _lock = results.lock(Phase::Run)?;
    results.sweep_scratch(Phase::Run)?;
    let backends: BTreeSet<&BackendId> = experiment.systems().iter().map(System::backend).collect();
    let vault = Vault::open(
        experiment.catalog(),
        backends,
        host.home,
        homes(host.runtime, results),
    )?;
    let versions = probe_versions(experiment, host, &vault, stop).await?;
    let used = toolchains_used(experiment, host.toolchains, versions.keys())?;
    let session = Session::start(repo)?
        .with_harnesses(versions)
        .with_toolchains(used);
    session.write(results)?;
    if !results.manifest().exists() {
        let manifest = Manifest {
            name: experiment.name(),
            first_session: session.id(),
        };
        write_json(&results.manifest(), &manifest)?;
    }
    check_references(experiment, host, session.id(), stop).await?;
    let pending = pending_trials(experiment, host.toolchains, session.id())?;
    let rig = Rig {
        results,
        toolchains: host.toolchains,
        vault: &vault,
        backend: host.backend,
        session: session.id(),
        budget: experiment.budget(),
        stop,
    };
    let limits = experiment.concurrency();
    schedule(pending, limits, guard, stop, async |trial: &TrialKey| {
        rig.attempt(&Task::of(experiment, trial)?).await
    })
    .await
}

/// The experiment's trials without a committed run, in order, after refusing every committed
/// run that is stale and reparsing the rest, which may withdraw a run and leave its trial
/// pending.
fn pending_trials(
    experiment: &Experiment,
    toolchains: &Toolchains,
    session: &SessionId,
) -> Result<Vec<TrialKey>> {
    let results = experiment.results();
    let trials = experiment.trials();
    let mut lifecycles = Vec::with_capacity(trials.len());
    for trial in &trials {
        let dir = results.trial(trial);
        sweep_partial(&dir.phase(Phase::Run))?;
        let current = Current::of(experiment, trial, toolchains)?;
        lifecycles.push(Lifecycle::read(&dir, &current)?);
    }
    let stale: Vec<String> = trials
        .iter()
        .zip(&lifecycles)
        .filter(|(_, lifecycle)| lifecycle.stale == Some(Phase::Run))
        .map(|(trial, _)| format!("\n  {trial}"))
        .collect();
    ensure!(
        stale.is_empty(),
        "{} committed runs were made from other inputs than the current ones, and running them \
         again would spend quota and change the sample; restore the inputs or withdraw these \
         runs:{}",
        stale.len(),
        stale.concat()
    );

    let mut pending = Vec::new();
    for (trial, lifecycle) in trials.into_iter().zip(&lifecycles) {
        let Some(committed) = lifecycle.run() else {
            pending.push(trial);
            continue;
        };
        let harness = Harness::named(trial.system.harness())
            .ok_or_else(|| eyre!("no driver for harness {}", trial.system.harness()))?;
        let attempt = committed.attempt();
        match reparse(results, &results.trial(&trial), committed, harness, session)? {
            Reparsed::Current => {}
            Reparsed::Updated => info!("{trial}: reparsed attempt {attempt}"),
            Reparsed::Demoted => {
                info!("{trial}: reparsing found a rate limit, attempt {attempt} discarded");
                pending.push(trial);
            }
        }
    }
    info!(
        "{}: {} of {} trials pending",
        experiment.name(),
        pending.len(),
        lifecycles.len()
    );
    Ok(pending)
}

/// Where the experiment's isolated homes live: a directory of `$XDG_RUNTIME_DIR/gungnir/`
/// named after the results directory, so that only the holder of its run lock uses it.
fn homes(runtime: &Path, results: &ResultsDir) -> PathBuf {
    let digest = Digest::of_bytes(results.path().as_os_str().as_bytes()).to_string();
    runtime.join("gungnir").join(&digest[..16])
}

/// Asks each harness CLI the experiment uses for its version, in an offline sandbox with only
/// its own environment, and refuses one that is not at its pin.
async fn probe_versions(
    experiment: &Experiment,
    host: &Host<'_>,
    vault: &Vault,
    stop: &Stop,
) -> Result<BTreeMap<HarnessName, HarnessVersion>> {
    let mut found = BTreeMap::new();
    for system in experiment.systems() {
        let name = system.harness();
        if found.contains_key(name) {
            continue;
        }
        let harness = Harness::named(name).ok_or_else(|| eyre!("no driver for harness {name}"))?;
        let key = vault.key(system.backend())?;
        let launch = harness.launch(&LaunchRequest {
            system,
            endpoint: key.endpoint(),
            credential: key.material(),
            home: Path::new(HOME),
            workspace: Path::new(WORK),
            prompt: "",
        })?;
        let program = launch
            .argv
            .first()
            .ok_or_else(|| eyre!("harness {name} launches no program"))?;
        let (base, toolchain) = (
            host.toolchains.base(),
            host.toolchains.harness(name.as_str())?,
        );
        let spec = Spec {
            network: Network::Offline,
            closure: Closure::union([base.closure(), toolchain.closure()]),
            mounts: Vec::new(),
            path: vec![toolchain.bin(), base.bin()],
            env: Vec::new(),
            cwd: HOME.into(),
            limits: PROBE_LIMITS,
        };
        let io = Io {
            stdin: Input::Null,
            stdout: Output::Capture { cap: PROBE_OUTPUT },
            stderr: Output::Capture { cap: PROBE_OUTPUT },
            kill: Kill::default(),
        };
        let argv = [program.as_str(), "--version"].map(OsString::from);
        let finished = host
            .backend
            .exec(&spec, &argv, io, stop)
            .await
            .wrap_err_with(|| format!("running {program} --version"))?;
        ensure!(
            finished.exit.success(),
            "{program} --version ended with {:?}: {}",
            finished.exit,
            String::from_utf8_lossy(&finished.stderr.bytes).trim()
        );
        let version = harness.parse_version(&String::from_utf8_lossy(&finished.stdout.bytes))?;
        ensure!(
            &version == system.harness_version(),
            "harness {name} is at version {version}, but the experiment pins {}; rebuild the \
             toolchains or change [pins]",
            system.harness_version()
        );
        found.insert(name.clone(), version);
    }
    Ok(found)
}

/// The toolchain environments an attempt of the experiment can use, named as under
/// `$GUNGNIR_TOOLCHAINS`.
fn toolchains_used<'a, 'h>(
    experiment: &Experiment,
    toolchains: &'a Toolchains,
    harnesses: impl IntoIterator<Item = &'h HarnessName>,
) -> Result<Vec<(String, &'a Toolchain)>> {
    let mut used = vec![("base".to_owned(), toolchains.base())];
    for language in experiment.languages() {
        let id = language.id();
        used.push((format!("language/{id}"), toolchains.language(id.as_str())?));
    }
    for harness in harnesses {
        used.push((
            format!("harness/{harness}"),
            toolchains.harness(harness.as_str())?,
        ));
    }
    Ok(used)
}

/// Judges each scenario's reference in each of the experiment's languages exactly as trials
/// are judged, keeping each grade under `references/`. Refuses to go on unless every one
/// passes.
async fn check_references(
    experiment: &Experiment,
    host: &Host<'_>,
    session: &SessionId,
    stop: &Stop,
) -> Result<()> {
    let mut failed = Vec::new();
    for scenario in experiment.scenarios() {
        for language in experiment.languages() {
            let cell = format!("{}/{}", scenario.id(), language.id());
            let verdict = reference(
                experiment.results(),
                scenario,
                language,
                host,
                session,
                stop,
            )
            .await
            .wrap_err_with(|| format!("judging the reference {cell}"))?;
            if verdict != Verdict::Pass {
                failed.push(format!("{cell} ({verdict})"));
            }
        }
    }
    ensure!(
        failed.is_empty(),
        "references that do not pass, so no trial runs: {}; their grades are under {}",
        failed.join(", "),
        experiment.results().path().join("references").display()
    );
    Ok(())
}

/// The verdict on `scenario`'s reference in `language`: its committed grade when that is
/// current, or a grade made now and committed in its place. A judgement the sandbox could not
/// make is an error, never committed.
async fn reference(
    results: &ResultsDir,
    scenario: &Scenario,
    language: &Language,
    host: &Host<'_>,
    session: &SessionId,
    stop: &Stop,
) -> Result<Verdict> {
    let source = scenario.reference(language.id()).ok_or_else(|| {
        eyre!(
            "scenario {} has no {} reference",
            scenario.id(),
            language.id()
        )
    })?;
    let dir = results.reference(scenario.id(), language.id());
    let inputs = GradeInputs::new(language, host.toolchains, scenario, source.dir())?;
    match read_record::<GradeRecord>(&dir)? {
        Some(grade) if grade.inputs() == &inputs => return Ok(grade.judgement().verdict()),
        Some(_) => results.retract(Phase::Run, &dir)?,
        None => {}
    }
    let partial = PartialDir::begin(&dir)?;
    let judgement = grade::judge(
        language,
        host.toolchains,
        source.dir(),
        scenario,
        host.backend,
        stop,
        partial.path(),
    )
    .await?;
    if let Some(fault) = judgement.fault() {
        return Err(eyre!("the sandbox could not judge it: {fault}"));
    }
    let verdict = judgement.verdict();
    partial.commit(&GradeRecord::new(session.clone(), inputs, judgement))?;
    Ok(verdict)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        id::AttemptNo,
        testing::{id, write},
        trial::FINAL,
    };
    use fixture::{Agent, Play, Repo};
    use std::{
        fs,
        sync::atomic::{AtomicBool, Ordering},
    };
    use tempfile::TempDir;

    /// The fixture repository, a user home with its credentials, and a runtime directory.
    struct Machine {
        repo: Repo,
        home: TempDir,
        runtime: TempDir,
    }

    impl Machine {
        fn new() -> Self {
            Self {
                repo: Repo::new(),
                home: fixture::credentials(),
                runtime: TempDir::new().unwrap(),
            }
        }

        async fn run(&self, agent: &Agent) -> Result<Tally> {
            let host = Host {
                toolchains: &self.repo.toolchains,
                backend: agent,
                home: self.home.path(),
                runtime: self.runtime.path(),
            };
            let experiment = self.repo.experiment();
            run(
                &experiment,
                self.repo.root.path(),
                &host,
                &Guard::default(),
                &Stop::default(),
            )
            .await
        }

        fn results(&self) -> ResultsDir {
            self.repo.experiment().results().clone()
        }
    }

    /// Agents that complete the task: a captured transcript of their harness, and a solution.
    fn solver() -> Agent {
        Agent::new(solving)
    }

    fn solving(spec: &Spec) -> Play {
        let codex = spec.env.iter().any(|(name, _)| name == "CODEX_HOME");
        let mut play = Play::transcript(if codex {
            "codex/luna-command-success.jsonl"
        } else {
            "claude-code/haiku-bash-success.jsonl"
        });
        play.workspace = vec![("main.py".to_owned(), "print(input())\n".to_owned())];
        play
    }

    fn committed_attempt(results: &ResultsDir, trial: &TrialKey) -> Option<AttemptNo> {
        read_record::<RunRecord>(&results.trial(trial).phase(Phase::Run))
            .unwrap()
            .map(|run| run.attempt())
    }

    #[tokio::test]
    async fn a_run_commits_each_pending_trial_once_and_records_its_provenance() {
        let machine = Machine::new();
        let agent = solver();
        let tally = machine.run(&agent).await.unwrap();
        assert_eq!(
            tally,
            Tally {
                committed: 2,
                ..Tally::default()
            }
        );
        let results = machine.results();
        for trial in machine.repo.experiment().trials() {
            assert_eq!(committed_attempt(&results, &trial), Some(AttemptNo::FIRST));
        }

        let sessions: Vec<_> = fs::read_dir(results.path().join("sessions"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        let [session] = sessions.as_slice() else {
            panic!("expected one session: {sessions:?}");
        };
        let session: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(session).unwrap()).unwrap();
        assert_eq!(session["harnesses"]["claude-code"], "2.1.316");
        assert_eq!(session["harnesses"]["codex"], "0.166.0");
        assert_eq!(
            session["toolchains"]["harness/codex"],
            "/nix/store/cccc-gungnir-harness-codex"
        );
        let reference: GradeRecord = read_record(&results.reference(&id("echo"), &id("python")))
            .unwrap()
            .unwrap();
        assert_eq!(reference.judgement().verdict(), Verdict::Pass);
        assert!(results.manifest().is_file());

        assert_eq!(machine.run(&agent).await.unwrap(), Tally::default());
        assert_eq!(agent.launches().len(), 2);
    }

    #[tokio::test]
    async fn a_crashed_attempt_is_discarded_and_its_trial_run_again_in_the_same_invocation() {
        let machine = Machine::new();
        let crashed = AtomicBool::new(false);
        let agent = Agent::new(move |spec| {
            let mut play = solving(spec);
            let claude = !spec.env.iter().any(|(name, _)| name == "CODEX_HOME");
            if claude && !crashed.swap(true, Ordering::SeqCst) {
                play.stdout.truncate(2);
                play.exit = sandbox::Exit::Code(1);
            }
            play
        });
        let tally = machine.run(&agent).await.unwrap();
        assert_eq!(
            tally,
            Tally {
                committed: 2,
                discarded: 1,
                ..Tally::default()
            }
        );
        let results = machine.results();
        let opus = fixture::trial(&machine.repo.experiment(), "opus");
        assert_eq!(
            committed_attempt(&results, &opus),
            Some(AttemptNo::FIRST.next())
        );
        let first = fs::read_to_string(results.trial(&opus).attempt(AttemptNo::FIRST).discarded());
        let first: serde_json::Value = serde_json::from_str(&first.unwrap()).unwrap();
        assert_eq!(first["reason"]["kind"], "crashed");
    }

    #[tokio::test]
    async fn a_harness_at_another_version_than_its_pin_is_refused() {
        let machine = Machine::new();
        let agent = solver().printing("codex", "codex-cli 0.167.0\n");
        let message = format!("{:#}", machine.run(&agent).await.unwrap_err());
        assert!(message.contains("0.167.0"), "{message}");
        assert!(message.contains("0.166.0"), "{message}");
        assert!(agent.launches().is_empty());
    }

    #[tokio::test]
    async fn a_reference_that_does_not_pass_refuses_the_run_and_keeps_its_grade() {
        let machine = Machine::new();
        write(
            machine.repo.root.path(),
            "scenarios/echo/hidden/edge.out",
            "x\n",
        );
        let agent = solver();
        let message = format!("{:#}", machine.run(&agent).await.unwrap_err());
        assert!(message.contains("echo/python (wrong-answer)"), "{message}");
        assert!(agent.launches().is_empty());
        let reference = machine.results().reference(&id("echo"), &id("python"));
        assert!(reference.join("cases/edge.out").is_file());
    }

    #[tokio::test]
    async fn stale_runs_are_refused_by_name_and_nothing_runs() {
        let machine = Machine::new();
        let agent = solver();
        machine.run(&agent).await.unwrap();
        write(
            machine.repo.root.path(),
            "scenarios/echo/spec.md",
            "Echo every line of stdin.\n",
        );

        let message = format!("{:#}", machine.run(&agent).await.unwrap_err());
        for trial in machine.repo.experiment().trials() {
            assert!(message.contains(&trial.to_string()), "{message}");
        }
        assert_eq!(agent.launches().len(), 2);
    }

    #[tokio::test]
    async fn a_rate_limit_an_older_parser_missed_is_reparsed_and_its_trial_run_again() {
        let machine = Machine::new();
        let experiment = machine.repo.experiment();
        let trial = fixture::trial(&experiment, "opus");
        let dir = experiment.results().trial(&trial);
        let limited = Play::transcript("claude-code/synthetic-rate-limit-event.jsonl");
        write(
            dir.attempt(AttemptNo::FIRST).path(),
            "transcript.jsonl",
            limited.stdout.join("\n"),
        );
        let partial = PartialDir::begin(&dir.phase(Phase::Run)).unwrap();
        write(&partial.path().join(FINAL), "main.py", "");
        let record = RunRecord {
            parser: 0,
            ..fixture::record(&machine.repo, &trial, Digest::of_bytes(b""))
        };
        partial.commit(&record).unwrap();

        let tally = machine.run(&solver()).await.unwrap();
        assert_eq!(tally.committed, 2);
        let results = machine.results();
        assert!(dir.attempt(AttemptNo::FIRST).discarded().is_file());
        assert_eq!(
            committed_attempt(&results, &trial),
            Some(AttemptNo::FIRST.next())
        );
    }
}
