//! Grading. A workspace is copied into fresh scratch, built in an offline sandbox, and run on
//! every hidden case in a sandbox of its own. Each case's stdout is compared on the host, so
//! expected outputs never enter a sandbox. Lizard measures the sources' complexity.
mod compare;
mod complexity;
#[cfg(test)]
pub(crate) mod fake;
mod judgement;
mod phase;
mod record;
#[cfg(test)]
mod tests;
pub(crate) mod tree;

pub use complexity::{Complexity, Counting, FileComplexity, Function};
pub(crate) use judgement::duration;
pub use judgement::{
    Abnormal, Build, BuildOutcome, Case, Crash, Failure, Judgement, Outcome, Verdict,
};
pub use phase::{Graded, grade_runs};
pub use record::{GradeInputs, GradeRecord};

use crate::{
    id::CaseId,
    language::{Argv, Language},
    sandbox::{
        Backend, Fault, Finished, Input, Io, Kill, Limits, Mount, Network, OomKill, Output, Spec,
        WORK,
    },
    scenario::{CaseFiles, Compare, Scenario},
    secret::EnvValue,
    stop::Stop,
    toolchains::{Closure, Toolchains},
};
use eyre::{Result, WrapErr};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// What each build step may use.
const BUILD_LIMITS: Limits = Limits {
    wall: Duration::from_secs(600),
    memory: 4 << 30,
    tasks: 1024,
    cpu: None,
    oom: OomKill::Sandbox,
};
/// How much of each build step's output the log keeps.
const BUILD_LOG_CAP: u64 = 1 << 20;
/// How much of a case's stdout is kept. A program that writes more fails the case.
const STDOUT_CAP: u64 = 8 << 20;
const CASE_TASKS: u32 = 256;

/// Judges `workspace` as a `language` solution to `scenario`: measures the complexity of a
/// copy of it, builds the copy offline and, if the build succeeds, runs every hidden case. Writes `build.log` into `evidence`, and
/// `cases/<case>.out` with the stdout kept from each case that ran and did not pass.
///
/// A sandbox that cannot run a command is judged an environment fault. An error means there
/// is no judgement: the host failed, or the second Ctrl-C killed a sandbox.
pub async fn judge(
    language: &Language,
    toolchains: &Toolchains,
    workspace: &Path,
    scenario: &Scenario,
    backend: &dyn Backend,
    stop: &Stop,
    evidence: &Path,
) -> Result<Judgement> {
    let scratch = tempfile::Builder::new()
        .prefix("gungnir-judge-")
        .tempdir()
        .wrap_err("creating grading scratch")?;
    let work = scratch.path().join("work");
    tree::copy(workspace, &work, language.ignore())?;
    let complexity = complexity::measure(language, &work).await?;

    let offline = Offline::new(language, toolchains, work, backend, stop)?;
    let (build, log) = offline.build(language.build()).await?;
    write(&evidence.join("build.log"), &log)?;
    let cases = match build.outcome {
        BuildOutcome::Built => offline.run(language.run(), scenario, evidence).await?,
        BuildOutcome::Failed { .. } | BuildOutcome::Fault { .. } => BTreeMap::new(),
    };
    Ok(Judgement::new(build, cases, complexity))
}

/// Runs commands in offline sandboxes over one scratch copy of a workspace, with the
/// language's toolchain and environment.
pub(crate) struct Offline<'a> {
    backend: &'a dyn Backend,
    stop: &'a Stop,
    closure: Closure,
    path: Vec<PathBuf>,
    env: Vec<(String, EnvValue)>,
    work: PathBuf,
}

impl<'a> Offline<'a> {
    /// Sandboxes over `work`, a scratch copy of a `language` workspace.
    pub(crate) fn new(
        language: &Language,
        toolchains: &Toolchains,
        work: PathBuf,
        backend: &'a dyn Backend,
        stop: &'a Stop,
    ) -> Result<Self> {
        let toolchain = toolchains.language(language.id().as_str())?;
        Ok(Self {
            backend,
            stop,
            closure: toolchain.closure().clone(),
            path: vec![toolchain.bin(), toolchains.base().bin()],
            env: language
                .env()
                .iter()
                .map(|(name, value)| (name.clone(), EnvValue::Plain(value.clone())))
                .collect(),
            work,
        })
    }

    /// The same, with `env` set on top of the language's variables.
    pub(crate) fn with_env<'e>(
        mut self,
        env: impl IntoIterator<Item = (&'e String, &'e String)>,
    ) -> Self {
        for (name, value) in env {
            self.env.retain(|(set, _)| set != name);
            self.env
                .push((name.clone(), EnvValue::Plain(value.clone())));
        }
        self
    }

    /// An offline sandbox whose working directory is the workspace, mounted at [`WORK`].
    pub(crate) fn spec(&self, writable: bool, limits: Limits) -> Spec {
        let workspace = if writable {
            Mount::writable(&self.work, WORK)
        } else {
            Mount::read_only(&self.work, WORK)
        };
        Spec {
            network: Network::Offline,
            closure: self.closure.clone(),
            mounts: vec![workspace],
            path: self.path.clone(),
            env: self.env.clone(),
            cwd: WORK.into(),
            limits,
        }
    }

    /// Runs `command`, keeping up to `cap` bytes of stdout and none of stderr. Also returns
    /// the wall time, which for a fault is how long the sandbox took to fail.
    async fn exec(
        &self,
        spec: &Spec,
        command: &[OsString],
        stdin: Input,
        cap: u64,
    ) -> (Duration, Result<Finished, Fault>) {
        let io = Io {
            stdin,
            stdout: Output::Capture { cap },
            stderr: Output::Capture { cap: 0 },
            kill: Kill::default(),
        };
        let started = Instant::now();
        let result = self.backend.exec(spec, command, io, self.stop).await;
        let wall = result
            .as_ref()
            .map_or_else(|_| started.elapsed(), |finished| finished.wall);
        (wall, result)
    }

    /// Runs the build steps in order over a writable workspace, stopping at the first that
    /// does not exit 0. Returns the build and its log.
    pub(crate) async fn build(&self, steps: &[Argv]) -> Result<(Build, Vec<u8>)> {
        let spec = self.spec(true, BUILD_LIMITS);
        let mut log = Vec::new();
        let mut wall = Duration::ZERO;
        for (step, argv) in steps.iter().enumerate() {
            log.extend_from_slice(format!("$ {argv}\n").as_bytes());
            // The shell merges stderr into stdout, so the log keeps the order the build printed in.
            let command: Vec<OsString> = ["sh", "-c", "exec \"$@\" 2>&1", "sh"]
                .into_iter()
                .chain(argv.args().iter().map(String::as_str))
                .map(OsString::from)
                .collect();
            let (took, result) = self.exec(&spec, &command, Input::Null, BUILD_LOG_CAP).await;
            wall += took;
            let finished = match result {
                Ok(finished) => finished,
                Err(fault) => {
                    let message = fault.to_string();
                    log.extend_from_slice(format!("{message}\n").as_bytes());
                    let outcome = BuildOutcome::Fault { step, message };
                    return Ok((Build { wall, outcome }, log));
                }
            };
            log.extend_from_slice(&finished.stdout.bytes);
            if finished.stdout.truncated {
                let total = finished.stdout.total;
                let note = format!("\n[log cut at {BUILD_LOG_CAP} of {total} bytes]\n");
                log.extend_from_slice(note.as_bytes());
            }
            if let Some(ending) = Abnormal::of(finished.exit)? {
                let outcome = BuildOutcome::Failed { step, ending };
                return Ok((Build { wall, outcome }, log));
            }
        }
        let outcome = BuildOutcome::Built;
        Ok((Build { wall, outcome }, log))
    }

    /// Runs every hidden case in a sandbox of its own over a read-only workspace.
    async fn run(
        &self,
        run: &Argv,
        scenario: &Scenario,
        evidence: &Path,
    ) -> Result<BTreeMap<CaseId, Case>> {
        let limits = Limits {
            wall: scenario.limits().time(),
            memory: scenario.limits().memory_bytes(),
            tasks: CASE_TASKS,
            cpu: None,
            oom: OomKill::Sandbox,
        };
        let spec = self.spec(false, limits);
        let command: Vec<OsString> = run.args().iter().map(OsString::from).collect();
        let mut cases = BTreeMap::new();
        for (id, files) in scenario.hidden() {
            let stdin = Input::File(files.input.clone());
            let (wall, result) = self.exec(&spec, &command, stdin, STDOUT_CAP).await;
            let outcome = match result {
                Ok(finished) => {
                    let outcome = outcome(&finished, files, scenario.compare())?;
                    if outcome != Outcome::Pass {
                        let path = evidence.join("cases").join(format!("{id}.out"));
                        write(&path, &finished.stdout.bytes)?;
                    }
                    outcome
                }
                Err(fault) => Outcome::Fault(fault.to_string()),
            };
            cases.insert(id.clone(), Case { wall, outcome });
        }
        Ok(cases)
    }
}

/// Judges a case that ran: by its ending first, then by how much it wrote, then by what.
fn outcome(finished: &Finished, files: &CaseFiles, compare: Compare) -> Result<Outcome> {
    if let Some(abnormal) = Abnormal::of(finished.exit)? {
        return Ok(Outcome::Abnormal(abnormal));
    }
    if finished.stdout.truncated {
        return Ok(Outcome::OutputOverCap);
    }
    let expected =
        fs::read(&files.output).wrap_err_with(|| format!("reading {}", files.output.display()))?;
    Ok(
        if compare::matches(compare, &finished.stdout.bytes, &expected) {
            Outcome::Pass
        } else {
            Outcome::WrongAnswer
        },
    )
}

fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    path.parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| fs::write(path, bytes))
        .wrap_err_with(|| format!("writing {}", path.display()))
}
