//! Fixtures the run phase's tests share: a repository with a two-trial experiment, toolchains
//! for its languages and harnesses, its credentials, committed runs and grades, and a sandbox
//! backend that plays an agent.
use super::{AgentExit, Outcome, Process, ProcessExit, RunInputs, RunRecord, Task};
use crate::{
    experiment::Experiment,
    grade::{
        self, GradeRecord,
        fake::{self, Scripted, finished},
    },
    id::AttemptNo,
    lifecycle::Current,
    measure::{
        self, Block, Comparison, Identity, Load, Machine, MeasureInputs, MeasureRecord, PeakMemory,
        Protocol, Timed,
    },
    provenance::SessionId,
    sandbox::{
        Backend, BoxFuture, Collected, Exit, Fault, Finished, HOME, Input, Io, Output, Spec, WORK,
    },
    stop::Stop,
    store::{Digest, PartialDir},
    testing::{self, write},
    toolchains::Toolchains,
    trial::{FINAL, Phase, TrialKey},
};
use chrono::Utc;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    os::unix::fs::symlink,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};
use tempfile::TempDir;
use tokio::sync::mpsc;

/// The fixture's Claude token.
pub const TOKEN: &str = "sk-ant-oat01-fixture";
/// The fixture's API key for the relay endpoint.
pub const RELAY_KEY: &str = "nvapi-fixture";
/// The fixture's Codex login.
pub const LOGIN: &str = r#"{"OPENAI_API_KEY":null,"tokens":{"id_token":"eyJ.id","access_token":"eyJ.access","refresh_token":"rt.refresh","account_id":"acct"},"last_refresh":"2026-11-03T20:04:12Z"}"#;

/// A user home holding the fixture catalog's four credentials.
pub fn credentials() -> TempDir {
    let home = TempDir::new().unwrap();
    write(home.path(), "token", format!("{TOKEN}\n"));
    write(home.path(), "key", "glm-key");
    write(home.path(), "relay-key", format!("{RELAY_KEY}\n"));
    write(home.path(), "codex/auth.json", LOGIN);
    home
}

/// What a fake agent does once launched.
#[derive(Clone, Debug)]
pub struct Play {
    /// Files the agent writes, by path relative to its workspace.
    pub workspace: Vec<(String, String)>,
    /// Files the agent writes, by path relative to its home.
    pub home: Vec<(String, String)>,
    pub stdout: Vec<String>,
    pub stderr: Vec<String>,
    /// How long the agent stays silent, with its stdout open, after its last line.
    pub silence: Duration,
    /// How the sandbox reports the agent's end.
    pub exit: Exit,
    /// The sandbox's fault, when it cannot run the agent to its end.
    pub fault: Option<String>,
}

impl Play {
    /// An agent that prints the captured transcript `name` from `tests/fixtures/harness/` and
    /// exits 0.
    pub fn transcript(name: &str) -> Self {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/harness")
            .join(name);
        let text = fs::read_to_string(&path).unwrap();
        Self {
            workspace: Vec::new(),
            home: Vec::new(),
            stdout: text.lines().map(str::to_owned).collect(),
            stderr: Vec::new(),
            silence: Duration::ZERO,
            exit: Exit::Code(0),
            fault: None,
        }
    }
}

/// One launch an [`Agent`] saw.
#[derive(Clone, Debug)]
pub struct Launched {
    pub spec: Spec,
    pub argv: Vec<String>,
    pub stdin: Vec<u8>,
}

type Script = dyn Fn(&Spec) -> Play + Send + Sync;

/// A sandbox backend for the run phase. `--version` prints the pinned version, a grading
/// sandbox builds and echoes every case, and an agent plays what the script decides.
pub struct Agent {
    script: Box<Script>,
    versions: BTreeMap<String, String>,
    launches: Mutex<Vec<Launched>>,
}

impl Agent {
    pub fn new(script: impl Fn(&Spec) -> Play + Send + Sync + 'static) -> Self {
        let versions = [
            ("claude", "2.1.316 (Claude Code)\n"),
            ("codex", "codex-cli 0.166.0\n"),
        ];
        Self {
            script: Box::new(script),
            versions: versions
                .into_iter()
                .map(|(program, printed)| (program.to_owned(), printed.to_owned()))
                .collect(),
            launches: Mutex::default(),
        }
    }

    /// Makes `program --version` print `printed`.
    pub fn printing(mut self, program: &str, printed: &str) -> Self {
        self.versions.insert(program.to_owned(), printed.to_owned());
        self
    }

    /// Every agent launched so far.
    pub fn launches(&self) -> Vec<Launched> {
        self.launches.lock().unwrap().clone()
    }

    /// Answers a sandbox that captures its stdout: a version probe or a grading step.
    fn tool(&self, argv: &[String], stdin: Input) -> Finished {
        if argv.get(1).map(String::as_str) == Some("--version") {
            return finished(Exit::Code(0), self.versions[&argv[0]].as_bytes());
        }
        let echoed = match stdin {
            Input::File(path) => fs::read(path).unwrap(),
            _ => Vec::new(),
        };
        finished(Exit::Code(0), &echoed)
    }
}

impl Backend for Agent {
    fn exec<'a>(
        &'a self,
        spec: &'a Spec,
        argv: &'a [OsString],
        io: Io,
        _stop: &'a Stop,
    ) -> BoxFuture<'a, Result<Finished, Fault>> {
        let argv: Vec<String> = argv
            .iter()
            .map(|arg| arg.to_str().unwrap().to_owned())
            .collect();
        let (Output::Lines { sink: stdout, .. }, Output::Lines { sink: stderr, .. }) =
            (io.stdout, io.stderr)
        else {
            let finished = self.tool(&argv, io.stdin);
            return Box::pin(async move { Ok(finished) });
        };
        let Input::Bytes(stdin) = io.stdin else {
            panic!("an agent reads its prompt from bytes");
        };
        self.launches.lock().unwrap().push(Launched {
            spec: spec.clone(),
            argv,
            stdin,
        });
        let play = (self.script)(spec);
        Box::pin(async move {
            let host = |guest: &str| -> PathBuf {
                let mount = spec
                    .mounts
                    .iter()
                    .find(|mount| mount.guest == Path::new(guest));
                mount.unwrap().host.clone()
            };
            for (path, contents) in &play.workspace {
                write(&host(WORK), path, contents);
            }
            for (path, contents) in &play.home {
                write(&host(HOME), path, contents);
            }
            let stdout_bytes = print(&stdout, &play.stdout).await;
            let stderr_bytes = print(&stderr, &play.stderr).await;
            tokio::time::sleep(play.silence).await;
            drop((stdout, stderr));
            if let Some(fault) = play.fault {
                return Err(eyre::eyre!(fault).into());
            }
            let collected = |total| Collected {
                bytes: Vec::new(),
                total,
                truncated: false,
            };
            Ok(Finished {
                exit: play.exit,
                wall: Duration::from_secs(90),
                stdout: collected(stdout_bytes),
                stderr: collected(stderr_bytes),
            })
        })
    }
}

/// Sends each of `lines` as one line of output. Returns how many bytes a program printing them
/// would have written.
async fn print(sink: &mpsc::Sender<Vec<u8>>, lines: &[String]) -> u64 {
    let mut total = 0;
    for line in lines {
        total += line.len() as u64 + 1;
        let _ = sink.send(line.clone().into_bytes()).await;
    }
    total
}

/// Claude Code's Opus and Codex's Luna, once each, on `echo` in Python.
pub const EXPERIMENT_TOML: &str = r#"
name = "fixture"
results = "../runs/fixture"
repetitions = 1
scenarios = ["echo"]
languages = ["python"]

[pins]
claude-code = "2.1.316"
codex = "0.166.0"

[budget]
wall = "40m"
idle = "10m"

[concurrency]
anthropic = 1
openai = 1

[[systems]]
model = "opus"
efforts = ["low"]

[[systems]]
model = "luna"
efforts = ["low"]
"#;

/// The fixture repository and toolchains for its languages and harnesses. Its Python measures
/// no complexity, so grading needs no host tool.
pub struct Repo {
    pub root: TempDir,
    pub toolchains: Toolchains,
}

impl Repo {
    pub fn new() -> Self {
        let root = TempDir::new().unwrap();
        testing::repo(root.path());
        write(
            root.path(),
            "languages/python/language.toml",
            testing::LANGUAGE_TOML.replace("lizard = \"python\"\n", ""),
        );
        write(root.path(), "experiments/fixture.toml", EXPERIMENT_TOML);
        let toolchains = toolchains(&root.path().join("toolchains"));
        Self { root, toolchains }
    }

    /// The experiment as the repository holds it now.
    pub fn experiment(&self) -> Experiment {
        Experiment::load(&self.root.path().join("experiments/fixture.toml")).unwrap()
    }
}

/// Makes the fixture experiment declare `protocol` in its `[measure]` table.
pub fn declare_protocol(repo: &Repo, protocol: Protocol) {
    let Protocol {
        blocks,
        runs,
        warmup,
        clock_mhz,
    } = protocol;
    let clock = clock_mhz.map_or_else(String::new, |mhz| format!("clock_mhz = {mhz}\n"));
    let table =
        format!("[measure]\nblocks = {blocks}\nruns = {runs}\nwarmup = {warmup}\n{clock}\n[pins]");
    write(
        repo.root.path(),
        "experiments/fixture.toml",
        EXPERIMENT_TOML.replacen("[pins]", &table, 1),
    );
}

/// The trial of `experiment` whose system runs `model`.
pub fn trial(experiment: &Experiment, model: &str) -> TrialKey {
    experiment
        .trials()
        .into_iter()
        .find(|trial| trial.system.model().as_str() == model)
        .unwrap()
}

pub fn session() -> SessionId {
    serde_json::from_str("\"20261103T120000Z-42\"").unwrap()
}

/// Fake toolchains for the fixture's languages and both harnesses.
pub fn toolchains(dir: &Path) -> Toolchains {
    for harness in ["claude-code", "codex"] {
        let env = format!("cccc-gungnir-harness-{harness}");
        let layout = dir.join("harness").join(harness);
        write(&layout, "closure", format!("/nix/store/{env}\n"));
        symlink(format!("/nix/store/{env}/bin"), layout.join("bin")).unwrap();
    }
    fake::toolchains(dir, &["python", "rust"])
}

/// The record of a first attempt of `trial` in the fixture experiment, made from the current
/// inputs, whose agent completed one turn and left a workspace with digest `snapshot`.
pub fn record(repo: &Repo, trial: &TrialKey, snapshot: Digest) -> RunRecord {
    let experiment = repo.experiment();
    let task = Task::of(&experiment, trial).unwrap();
    RunRecord {
        session: session(),
        inputs: RunInputs::current(&task, &repo.toolchains, experiment.budget()).unwrap(),
        attempt: AttemptNo::FIRST,
        started: Utc::now(),
        process: Process {
            exit: ProcessExit::Code(0),
            cut: None,
            wall: Duration::from_secs(60),
            stdout_bytes: 0,
            stderr_bytes: 0,
        },
        parser: 1,
        outcome: Outcome {
            exit: AgentExit::Completed,
            turns: Some(1),
            usage: Vec::new(),
            api_time: None,
            model: None,
            quota: BTreeMap::new(),
            unrecognised: BTreeMap::new(),
        },
        snapshot,
    }
}

/// Commits a first attempt of `trial`, made from the current inputs, whose agent completed
/// and left `files`.
pub fn commit_run(repo: &Repo, trial: &TrialKey, files: &[(&str, &str)]) {
    let experiment = repo.experiment();
    let partial = PartialDir::begin(&experiment.results().trial(trial).phase(Phase::Run)).unwrap();
    let workspace = partial.path().join(FINAL);
    for (path, contents) in files {
        write(&workspace, path, contents);
    }
    let ignore = experiment.languages()[0].ignore();
    let snapshot = Digest::of_tree(&workspace, ignore).unwrap();
    partial.commit(&record(repo, trial, snapshot)).unwrap();
}

/// Commits a measurement of `trial` that timed nothing, made from the inputs of measuring
/// it now under `protocol`, on this machine.
pub fn commit_measure(repo: &Repo, trial: &TrialKey, protocol: Protocol) {
    let here = Identity::here().unwrap();
    commit_measure_on(
        repo,
        trial,
        protocol,
        here.cpu.as_deref(),
        here.host.as_deref(),
    );
}

/// The same, on the machine with the CPU model `cpu` and the host id `host`.
pub fn commit_measure_on(
    repo: &Repo,
    trial: &TrialKey,
    protocol: Protocol,
    cpu: Option<&str>,
    host: Option<&str>,
) {
    let experiment = repo.experiment();
    let current = Current::of(&experiment, trial, &repo.toolchains).unwrap();
    let dir = experiment.results().trial(trial);
    let inputs = MeasureInputs::new(
        current.language(),
        &repo.toolchains,
        current.scenario(),
        &dir.final_workspace(),
        &protocol,
    )
    .unwrap();
    let record = serde_json::json!({
        "session": session(),
        "inputs": inputs,
        "machine": {
            "cpu": cpu, "host": host, "cpus": 1, "core": 0, "governor": null, "kernel": "7.3.0",
            "perf_event_paranoid": 1, "load": {"start": [0.0, 0.0, 0.0], "end": [0.0, 0.0, 0.0]},
        },
        "workloads": {},
    });
    let partial = PartialDir::begin(&dir.phase(Phase::Measure)).unwrap();
    partial.commit(&record).unwrap();
}

/// Commits a measurement of `trial` made under `protocol` on this machine: on `large` the
/// candidate took twice as long as the reference, 1s against 0.5s, and on `small` it printed the
/// wrong output.
pub fn commit_timed(repo: &Repo, trial: &TrialKey, protocol: Protocol) {
    let experiment = repo.experiment();
    let dir = experiment.results().trial(trial);
    let current = Current::of(&experiment, trial, &repo.toolchains).unwrap();
    let inputs = MeasureInputs::new(
        current.language(),
        &repo.toolchains,
        current.scenario(),
        &dir.final_workspace(),
        &protocol,
    )
    .unwrap();
    let runs = |walls: &[f64]| -> Vec<measure::Sample> {
        walls
            .iter()
            .map(|&wall| measure::Sample {
                wall,
                user: wall,
                system: 0.0,
                peak_memory: 3 << 20,
                instructions: None,
            })
            .collect()
    };
    let large = Timed {
        blocks: vec![
            Block {
                reference: runs(&[0.5, 0.5]),
                candidate: runs(&[1.0, 1.0]),
            },
            Block {
                candidate: runs(&[1.0, 1.0]),
                reference: runs(&[0.5, 0.5]),
            },
        ],
        peak_memory: PeakMemory {
            reference: Some(1 << 20),
            candidate: Some(2 << 20),
        },
    };
    let here = Identity::here().unwrap();
    let comparison = Comparison {
        machine: Machine {
            cpu: here.cpu.clone(),
            host: here.host.clone(),
            cpus: 1,
            core: 0,
            isolated: false,
            governor: None,
            clock: None,
            turbo: None,
            kernel: "7.3.0".into(),
            perf_event_paranoid: Some(3),
            load: Load {
                start: [0.5, 0.25, 0.25],
                end: [0.75, 0.25, 0.25],
            },
        },
        workloads: BTreeMap::from([
            (testing::id("large"), measure::Outcome::Timed(large)),
            (
                testing::id("small"),
                measure::Outcome::CandidateFailed(measure::Failure::WrongOutput),
            ),
        ]),
    };
    let record = MeasureRecord::new(session(), inputs, comparison);
    let partial = PartialDir::begin(&dir.phase(Phase::Measure)).unwrap();
    partial.commit(&record).unwrap();
}

/// A sandbox whose every build step succeeds and whose program echoes each case.
pub fn echo() -> Scripted {
    Scripted::new(|_, _, io| {
        let stdout = match io.stdin {
            Input::File(path) => fs::read(path).unwrap(),
            _ => Vec::new(),
        };
        Ok(finished(Exit::Code(0), &stdout))
    })
}

/// Grades `trial`'s committed run with [`echo`] and commits the grade, as the grade phase
/// would.
pub async fn commit_grade(repo: &Repo, trial: &TrialKey) {
    let experiment = repo.experiment();
    let current = Current::of(&experiment, trial, &repo.toolchains).unwrap();
    let dir = experiment.results().trial(trial);
    let workspace = dir.final_workspace();
    let inputs = current.grade(&workspace).unwrap();
    let partial = PartialDir::begin(&dir.phase(Phase::Grade)).unwrap();
    let language = &experiment.languages()[0];
    let scenario = &experiment.scenarios()[0];
    let judgement = grade::judge(
        language,
        &repo.toolchains,
        &workspace,
        scenario,
        &echo(),
        &Stop::default(),
        partial.path(),
    )
    .await
    .unwrap();
    partial
        .commit(&GradeRecord::new(session(), inputs, judgement))
        .unwrap();
}
