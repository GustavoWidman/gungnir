//! Fixtures the run phase's tests share: a repository with a two-trial experiment, toolchains
//! for its languages and harnesses, and committed runs and grades.
use super::{AgentExit, Outcome, Process, ProcessExit, RunInputs, RunRecord, Task};
use crate::{
    experiment::Experiment,
    grade::{
        self, GradeRecord,
        fake::{self, Scripted, finished},
    },
    id::AttemptNo,
    lifecycle::Current,
    provenance::SessionId,
    sandbox::{Exit, Input},
    stop::Stop,
    store::{Digest, PartialDir},
    testing::{self, write},
    toolchains::Toolchains,
    trial::{FINAL, Phase, TrialKey},
};
use chrono::Utc;
use std::{collections::BTreeMap, fs, os::unix::fs::symlink, path::Path, time::Duration};
use tempfile::TempDir;

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

/// Commits a first attempt of `trial`, made from the current inputs, whose agent completed
/// and left `files`.
pub fn commit_run(repo: &Repo, trial: &TrialKey, files: &[(&str, &str)]) {
    let experiment = repo.experiment();
    let task = Task::of(&experiment, trial).unwrap();
    let inputs = RunInputs::current(&task, &repo.toolchains, experiment.budget()).unwrap();
    let partial = PartialDir::begin(&experiment.results().trial(trial).phase(Phase::Run)).unwrap();
    let workspace = partial.path().join(FINAL);
    for (path, contents) in files {
        write(&workspace, path, contents);
    }
    let record = RunRecord {
        session: session(),
        inputs,
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
        snapshot: Digest::of_tree(&workspace, task.language.ignore()).unwrap(),
    };
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
