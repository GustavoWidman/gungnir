//! One attempt: the agent launched headless in a sandbox whose only way out is the attempt's
//! own egress proxy, its stdout streamed into the transcript and decoded as it arrives, and
//! the trial's run directory or the attempt's discard record committed from how it ended.
use super::{
    Cut, Process, Reason, RunInputs, RunRecord, Task,
    records::{Counted, DiscardedAttempt, samples},
    workspace,
};
use crate::{
    egress::{Allowlist, Proxy},
    experiment::Budget,
    harness::{self, Ending, Fold, Harness, LaunchRequest, ProcessEnd},
    id::AttemptNo,
    provenance::SessionId,
    sandbox::{
        Backend, Fault, Finished, HOME, Input, Io, Kill, Limits, Mount, Network, OomKill, Output,
        Spec, WORK,
    },
    secret::{EnvValue, Redactor},
    stop::Stop,
    store::{PartialDir, write_json},
    toolchains::{Closure, Toolchains},
    trial::{AttemptDir, FINAL, PROMPT, Phase, ResultsDir},
    vault::{Home, Vault},
};
use chrono::Utc;
use eyre::{Result, WrapErr, eyre};
use globset::{Glob, GlobSetBuilder};
use std::{
    ffi::OsString,
    fs::{self, File},
    future::Future,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::sync::mpsc;

/// What an agent may use besides time: its CLI, the builds and tests it runs, and theirs.
const MEMORY: u64 = 4 << 30;
const TASKS: u32 = 2048;
/// How much stdout the transcript keeps. Lines past it are lost to the transcript and the fold.
const TRANSCRIPT_CAP: u64 = 1 << 30;
const STDERR_CAP: u64 = 64 << 20;
/// How much of the end of stderr may explain an exit that stdout did not.
const STDERR_TAIL: u64 = 16 << 10;
/// Lines in flight between the sandbox and the files they go to.
const LINE_BUFFER: usize = 256;

/// What every attempt of one run phase shares.
pub struct Rig<'a> {
    pub results: &'a ResultsDir,
    pub toolchains: &'a Toolchains,
    pub vault: &'a Vault,
    pub backend: &'a dyn Backend,
    pub session: &'a SessionId,
    pub budget: Budget,
    pub stop: &'a Stop,
}

/// How an attempt whose agent ran left its trial.
#[derive(Debug)]
pub enum Attempted {
    /// The attempt is a sample: the trial's run directory is committed with this record.
    Committed(Box<RunRecord>),
    /// The attempt is no sample, for `reason`. It is discarded and the trial is still pending.
    Discarded { attempt: AttemptNo, reason: Reason },
}

/// What the sandbox reported, and why gungnir cut the agent short when it did.
struct Ran {
    finished: Result<Finished, Fault>,
    cut: Option<Cut>,
}

impl Rig<'_> {
    /// Makes the next attempt of `task`'s trial, committing its run when it is a sample and
    /// discarding it otherwise. An error leaves the attempt, if it started, as interrupted.
    pub async fn attempt(&self, task: &Task<'_>) -> Result<Attempted> {
        let system = &task.trial.system;
        let harness = Harness::named(system.harness())
            .ok_or_else(|| eyre!("no driver for harness {}", system.harness()))?;
        let scratch = self.results.scratch(Phase::Run);
        fs::create_dir_all(&scratch).wrap_err_with(|| format!("creating {}", scratch.display()))?;
        let work = tempfile::Builder::new()
            .prefix("work-")
            .tempdir_in(&scratch)
            .wrap_err_with(|| format!("creating a workspace in {}", scratch.display()))?;
        let digest = workspace::scaffold(task.language, task.brief, work.path())?;
        let inputs = RunInputs::new(task, self.toolchains, self.budget, digest)?;
        let prompt = task.prompt(self.toolchains)?;

        let key = self.vault.key(system.backend())?;
        let launch = harness.launch(&LaunchRequest {
            system,
            endpoint: key.launch_endpoint(),
            credential: key.launch_credential(),
            home: Path::new(HOME),
            workspace: Path::new(WORK),
            prompt: &prompt,
        })?;
        let home = self.vault.home(&key, &launch.files)?;
        let trial = self.results.trial(task.trial);
        let number = trial
            .attempts()
            .last()
            .map_or(AttemptNo::FIRST, |last| last.number().next());
        let attempt = trial.attempt(number);
        fs::create_dir_all(attempt.path())
            .wrap_err_with(|| format!("creating {}", attempt.path().display()))?;
        let proxy = Proxy::start(
            &home.socket(),
            Allowlist::new(task.backend.egress()),
            key.relay()?,
            &attempt.egress(),
        )
        .await?;
        let spec = self.spec(task, &home, work.path(), launch.env, &proxy)?;
        let redactor = Redactor::new(key.secrets());
        let started = Utc::now();
        let ran = self
            .supervise(
                harness,
                &spec,
                &launch.argv,
                launch.stdin,
                &attempt,
                &redactor,
            )
            .await;
        let proxied = proxy.close().await;
        let collected = collect_artifacts(harness, &home.path(), &attempt.artifacts(), &redactor);
        let fresh = home.close();
        let (Ran { finished, cut }, (), artifacts, fresh) = (ran?, proxied?, collected?, fresh?);

        if !fresh.is_empty() {
            let refreshed = Redactor::new(&fresh);
            refreshed.redact_file(&attempt.transcript())?;
            refreshed.redact_file(&attempt.stderr())?;
            for artifact in &artifacts {
                refreshed.redact_file(artifact)?;
            }
        }
        let discard = |process, reason: Reason, usage| -> Result<Attempted> {
            let discarded = DiscardedAttempt {
                session: self.session.clone(),
                started,
                process,
                parser: harness.parser_version(),
                reason: reason.clone(),
                usage,
            };
            write_json(&attempt.discarded(), &discarded)?;
            Ok(Attempted::Discarded {
                attempt: number,
                reason,
            })
        };
        let finished = match finished {
            Ok(finished) => finished,
            Err(fault) => {
                let usage = match conclude(harness, &attempt, ProcessEnd::Killed)? {
                    Ending::Finished(outcome) => outcome.usage,
                    Ending::RateLimited { usage, .. } => usage,
                };
                let reason = Reason::Fault {
                    message: fault.to_string(),
                };
                return discard(None, reason, samples(usage));
            }
        };
        let process = Process {
            exit: finished.exit.into(),
            cut,
            wall: finished.wall,
            stdout_bytes: finished.stdout.total,
            stderr_bytes: finished.stderr.total,
        };
        match Counted::of(conclude(harness, &attempt, process.end())?, &process) {
            Counted::Discarded { reason, usage } => discard(Some(process), reason, usage),
            Counted::Sample(outcome) => {
                let redactor = Redactor::new(key.secrets().iter().chain(&fresh));
                let partial = PartialDir::begin(&trial.phase(Phase::Run))?;
                let prompt_file = partial.path().join(PROMPT);
                fs::write(&prompt_file, &prompt)
                    .wrap_err_with(|| format!("writing {}", prompt_file.display()))?;
                let snapshot = workspace::keep(
                    work.path(),
                    task.language,
                    &redactor,
                    &partial.path().join(FINAL),
                )?;
                let record = RunRecord {
                    session: self.session.clone(),
                    inputs,
                    attempt: number,
                    started,
                    process,
                    parser: harness.parser_version(),
                    outcome,
                    snapshot,
                };
                partial.commit(&record)?;
                Ok(Attempted::Committed(Box::new(record)))
            }
        }
    }

    /// The agent's sandbox: the workspace and the home writable, the base, language and
    /// harness environments, the language's variables then the launch's, and no network but
    /// the proxy.
    fn spec(
        &self,
        task: &Task<'_>,
        home: &Home,
        work: &Path,
        launch_env: Vec<(String, EnvValue)>,
        proxy: &Proxy,
    ) -> Result<Spec> {
        let base = self.toolchains.base();
        let language = self.toolchains.language(task.language.id().as_str())?;
        let harness = self
            .toolchains
            .harness(task.trial.system.harness().as_str())?;
        let mut env: Vec<(String, EnvValue)> = task
            .language
            .env()
            .iter()
            .map(|(name, value)| (name.clone(), EnvValue::Plain(value.clone())))
            .collect();
        env.extend(launch_env);
        Ok(Spec {
            network: Network::Egress(proxy.grant()),
            closure: Closure::union([base.closure(), language.closure(), harness.closure()]),
            mounts: vec![
                Mount::writable(work, WORK),
                Mount::writable(home.path(), HOME),
            ],
            path: vec![language.bin(), base.bin(), harness.bin()],
            env,
            cwd: WORK.into(),
            limits: Limits {
                wall: self.budget.wall(),
                memory: MEMORY,
                tasks: TASKS,
                cpu: None,
                oom: OomKill::Process,
            },
        })
    }

    /// Runs the agent with the prompt on stdin, streaming its stdout into the transcript and
    /// its stderr into the log, both redacted line by line.
    async fn supervise(
        &self,
        harness: Harness,
        spec: &Spec,
        argv: &[String],
        stdin: String,
        attempt: &AttemptDir,
        redactor: &Redactor,
    ) -> Result<Ran> {
        let create = |path: &Path| {
            File::create_new(path).wrap_err_with(|| format!("creating {}", path.display()))
        };
        let (transcript, stderr_log) = (create(&attempt.transcript())?, create(&attempt.stderr())?);
        let (stdout_sink, stdout) = mpsc::channel(LINE_BUFFER);
        let (stderr_sink, stderr) = mpsc::channel(LINE_BUFFER);
        let kill = Kill::default();
        let io = Io {
            stdin: Input::Bytes(stdin.into_bytes()),
            stdout: Output::Lines {
                sink: stdout_sink,
                cap: TRANSCRIPT_CAP,
            },
            stderr: Output::Lines {
                sink: stderr_sink,
                cap: STDERR_CAP,
            },
            kill: kill.clone(),
        };
        let argv: Vec<OsString> = argv.iter().map(OsString::from).collect();
        let idle = self.budget.idle();
        let (finished, cut, logged) = tokio::join!(
            self.backend.exec(spec, &argv, io, self.stop),
            or_kill(
                &kill,
                watch(stdout, transcript, redactor, harness, idle, &kill)
            ),
            or_kill(&kill, log(stderr, stderr_log, redactor)),
        );
        let (cut, ()) = (cut?, logged?);
        Ok(Ran { finished, cut })
    }
}

/// Copies the agent's stdout into the transcript line by line, redacted, and folds each line
/// as it arrives. Kills the agent when its stdout reports a rate limit or stays silent for
/// `idle`, and says which.
async fn watch(
    mut lines: mpsc::Receiver<Vec<u8>>,
    mut transcript: File,
    redactor: &Redactor,
    harness: Harness,
    idle: Duration,
    kill: &Kill,
) -> Result<Option<Cut>> {
    let mut fold = Fold::default();
    let mut cut = None;
    loop {
        let next = match cut {
            Some(_) => lines.recv().await,
            None => match tokio::time::timeout(idle, lines.recv()).await {
                Ok(next) => next,
                Err(_) => {
                    kill.kill();
                    cut = Some(Cut::Idle);
                    continue;
                }
            },
        };
        let Some(line) = next else {
            return Ok(cut);
        };
        let mut line = redactor.redact(&line);
        let text = String::from_utf8_lossy(&line).into_owned();
        line.push(b'\n');
        transcript
            .write_all(&line)
            .wrap_err("writing the transcript")?;
        if cut.is_none() {
            harness
                .decode(&text)
                .into_iter()
                .for_each(|event| fold.push(event));
            if fold.rate_limited() {
                kill.kill();
                cut = Some(Cut::RateLimited);
            }
        }
    }
}

/// Appends the agent's stderr to its log line by line, redacted.
async fn log(
    mut lines: mpsc::Receiver<Vec<u8>>,
    mut file: File,
    redactor: &Redactor,
) -> Result<()> {
    while let Some(line) = lines.recv().await {
        let mut line = redactor.redact(&line);
        line.push(b'\n');
        file.write_all(&line)
            .wrap_err("writing the agent's stderr")?;
    }
    Ok(())
}

/// Kills the agent when `work` fails, so that an agent is never left running unrecorded.
async fn or_kill<T>(kill: &Kill, work: impl Future<Output = Result<T>>) -> Result<T> {
    let result = work.await;
    if result.is_err() {
        kill.kill();
    }
    result
}

/// How the attempt in `attempt` ended: its transcript folded by the harness's parser as it is
/// now, with the end of its stderr to explain an exit that stdout did not. Live and replayed,
/// the same files give the same ending.
pub(super) fn conclude(harness: Harness, attempt: &AttemptDir, end: ProcessEnd) -> Result<Ending> {
    let bytes = fs::read(attempt.transcript())
        .wrap_err_with(|| format!("reading {}", attempt.transcript().display()))?;
    let transcript = String::from_utf8_lossy(&bytes);
    let artifacts = stored_artifacts(&attempt.artifacts())?;
    let artifacts: Vec<&str> = artifacts.iter().map(String::as_str).collect();
    let ending = harness.replay(&transcript, &artifacts).finish(end);
    let unexplained = matches!(
        &ending,
        Ending::Finished(outcome)
            if matches!(outcome.exit, harness::Exit::Crashed | harness::Exit::Killed)
    );
    if !unexplained {
        return Ok(ending);
    }
    let Some(diagnosis) = harness.diagnose_exit(&tail(&attempt.stderr(), STDERR_TAIL)?) else {
        return Ok(ending);
    };
    let mut fold = harness.replay(&transcript, &artifacts);
    fold.push(diagnosis);
    Ok(fold.finish(end))
}

/// Copies every file under `home` that matches one of the harness's artifact patterns into
/// `into`, keeping its path relative to `home`, and redacts each copy. Returns the copies.
fn collect_artifacts(
    harness: Harness,
    home: &Path,
    into: &Path,
    redactor: &Redactor,
) -> Result<Vec<PathBuf>> {
    let patterns = harness.artifacts();
    if patterns.is_empty() {
        return Ok(Vec::new());
    }
    let mut set = GlobSetBuilder::new();
    for pattern in patterns {
        set.add(Glob::new(pattern).wrap_err_with(|| format!("artifact pattern {pattern:?}"))?);
    }
    let set = set.build().wrap_err("building the artifact patterns")?;
    let mut copies = Vec::new();
    for file in regular_files(home)? {
        let relative = file.strip_prefix(home).unwrap_or(&file);
        if !set.is_match(relative) {
            continue;
        }
        let copy = into.join(relative);
        fs::create_dir_all(copy.parent().unwrap_or(into))
            .wrap_err_with(|| format!("creating the parent of {}", copy.display()))?;
        fs::copy(&file, &copy)
            .wrap_err_with(|| format!("copying {} to {}", file.display(), copy.display()))?;
        redactor.redact_file(&copy)?;
        copies.push(copy);
    }
    copies.sort();
    Ok(copies)
}

/// The contents of every file stored under an attempt's artifact directory, in path order.
fn stored_artifacts(dir: &Path) -> Result<Vec<String>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut files = regular_files(dir)?;
    files.sort();
    files
        .iter()
        .map(|file| {
            fs::read(file)
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .wrap_err_with(|| format!("reading {}", file.display()))
        })
        .collect()
}

/// Every regular file under `dir`, never following symlinks.
fn regular_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_owned()];
    while let Some(current) = pending.pop() {
        let listing =
            fs::read_dir(&current).wrap_err_with(|| format!("listing {}", current.display()))?;
        for entry in listing {
            let entry = entry.wrap_err_with(|| format!("listing {}", current.display()))?;
            let kind = entry
                .file_type()
                .wrap_err_with(|| format!("inspecting {}", entry.path().display()))?;
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                files.push(entry.path());
            }
        }
    }
    Ok(files)
}

/// The last `bytes` of the file at `path`, or nothing when there is no such file.
fn tail(path: &Path, bytes: u64) -> Result<String> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(err) => return Err(err).wrap_err_with(|| format!("opening {}", path.display())),
    };
    let mut kept = Vec::new();
    file.seek(SeekFrom::End(0))
        .and_then(|len| file.seek(SeekFrom::Start(len.saturating_sub(bytes))))
        .and_then(|_| file.read_to_end(&mut kept))
        .wrap_err_with(|| format!("reading {}", path.display()))?;
    Ok(String::from_utf8_lossy(&kept).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        experiment::Experiment,
        grade::tree,
        run::{
            AgentExit, ProcessExit,
            fixture::{self, Agent, HOSTED_KEY, LOGIN, Play, Repo, THROTTLED_KEY, TOKEN},
            records::Timeout,
        },
        sandbox,
        secret::Secret,
        store::read_record,
        testing::write,
        trial::TrialDir,
    };
    use chrono::DateTime;
    use globset::GlobSet;
    use std::{collections::BTreeMap, path::PathBuf};
    use tempfile::TempDir;

    /// The fixture repository with its credentials, an open vault and a stop.
    struct Bench {
        repo: Repo,
        home: TempDir,
        runtime: TempDir,
        experiment: Experiment,
        vault: Vault,
        stop: Stop,
    }

    /// Codex on `model` at `backend`'s own endpoint, once, on `echo` in Python.
    fn endpoint_experiment(model: &str, backend: &str) -> String {
        format!(
            r#"
name = "{backend}"
results = "../runs/{backend}"
repetitions = 1
scenarios = ["echo"]
languages = ["python"]

[pins]
codex = "0.166.0"

[budget]
wall = "40m"
idle = "10m"

[concurrency]
{backend} = 1

[[systems]]
model = "{model}"
efforts = ["high"]
"#
        )
    }

    impl Bench {
        fn new() -> Self {
            let repo = Repo::new();
            let experiment = repo.experiment();
            Self::of(repo, experiment)
        }

        /// The fixture repository running [`endpoint_experiment`] instead.
        fn at(model: &str, backend: &str) -> Self {
            let repo = Repo::new();
            let rel = format!("experiments/{backend}.toml");
            write(repo.root.path(), &rel, endpoint_experiment(model, backend));
            let experiment = Experiment::load(&repo.root.path().join(rel)).unwrap();
            Self::of(repo, experiment)
        }

        fn of(repo: Repo, experiment: Experiment) -> Self {
            let home = fixture::credentials();
            let runtime = TempDir::new().unwrap();
            let vault = Vault::open(
                experiment.catalog(),
                experiment.concurrency().keys(),
                home.path(),
                runtime.path().join("gungnir"),
            )
            .unwrap();
            Self {
                repo,
                home,
                runtime,
                experiment,
                vault,
                stop: Stop::default(),
            }
        }

        async fn attempt(&self, model: &str, agent: &Agent) -> Attempted {
            let trial = fixture::trial(&self.experiment, model);
            let task = Task::of(&self.experiment, &trial).unwrap();
            let session = fixture::session();
            let rig = Rig {
                results: self.experiment.results(),
                toolchains: &self.repo.toolchains,
                vault: &self.vault,
                backend: agent,
                session: &session,
                budget: self.experiment.budget(),
                stop: &self.stop,
            };
            rig.attempt(&task).await.unwrap()
        }

        fn trial(&self, model: &str) -> TrialDir {
            let trial = fixture::trial(&self.experiment, model);
            self.experiment.results().trial(&trial)
        }

        /// Nothing an attempt left outside the results directory.
        fn assert_clean(&self) {
            let homes = self.runtime.path().join("gungnir");
            assert_eq!(fs::read_dir(homes).unwrap().count(), 0);
            let scratch = self.experiment.results().scratch(Phase::Run);
            assert_eq!(fs::read_dir(scratch).unwrap().count(), 0);
        }
    }

    fn files(root: &Path) -> Vec<PathBuf> {
        tree::files(root, &GlobSet::empty())
            .unwrap()
            .into_iter()
            .map(|(rel, _)| rel)
            .collect()
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    #[tokio::test]
    async fn a_finished_attempt_commits_its_run_without_build_output_or_secrets() {
        let bench = Bench::new();
        let agent = Agent::new(|_| {
            let mut play = Play::transcript("claude-code/haiku-bash-success.jsonl");
            play.stdout
                .push(format!(r#"{{"type":"user","echo":"{TOKEN}"}}"#));
            play.stderr.push(format!("using {TOKEN}"));
            play.workspace = [
                ("main.py", "print(input())\n".to_owned()),
                ("notes.txt", format!("token {TOKEN}")),
                ("__pycache__/main.cpython.pyc", String::new()),
            ]
            .map(|(path, contents)| (path.to_owned(), contents))
            .into();
            play
        });

        let Attempted::Committed(record) = bench.attempt("opus", &agent).await else {
            panic!("expected a committed run");
        };
        let trial = bench.trial("opus");
        let run = trial.phase(Phase::Run);
        assert_eq!(
            read_record::<RunRecord>(&run).unwrap().as_ref(),
            Some(&*record)
        );
        assert_eq!(record.attempt(), AttemptNo::FIRST);
        assert_eq!(record.outcome().exit, AgentExit::Completed);
        assert_eq!(record.outcome().turns, Some(2));
        assert_eq!(record.process().exit, ProcessExit::Code(0));
        assert_eq!(record.process().cut, None);

        let task_trial = fixture::trial(&bench.experiment, "opus");
        let task = Task::of(&bench.experiment, &task_trial).unwrap();
        let prompt = task.prompt(&bench.repo.toolchains).unwrap();
        assert_eq!(read(&run.join(PROMPT)), prompt);
        assert_eq!(
            files(&trial.final_workspace()),
            [
                "examples/basic.in",
                "examples/basic.out",
                "main.py",
                "notes.txt"
            ]
            .map(PathBuf::from)
        );
        assert_eq!(
            read(&trial.final_workspace().join("notes.txt")),
            "token [REDACTED]"
        );

        let attempt = trial.attempt(AttemptNo::FIRST);
        let transcript = read(&attempt.transcript());
        assert_eq!(transcript.lines().count(), 7);
        assert!(!transcript.contains(TOKEN) && transcript.contains("[REDACTED]"));
        assert_eq!(read(&attempt.stderr()), "using [REDACTED]\n");
        assert!(attempt.egress().is_file());
        assert!(!attempt.discarded().exists());
        bench.assert_clean();

        let [launched] = agent.launches().try_into().unwrap();
        assert_eq!(launched.argv[..2], ["claude", "-p"]);
        assert_eq!(launched.stdin, prompt.as_bytes());
        let spec = launched.spec;
        assert!(matches!(spec.network, Network::Egress(_)));
        let guests: Vec<(&Path, bool)> = spec
            .mounts
            .iter()
            .map(|mount| (mount.guest.as_path(), mount.writable))
            .collect();
        assert_eq!(guests, [(Path::new(WORK), true), (Path::new(HOME), true)]);
        let toolchains = &bench.repo.toolchains;
        assert_eq!(
            spec.path,
            [
                toolchains.language("python").unwrap().bin(),
                toolchains.base().bin(),
                toolchains.harness("claude-code").unwrap().bin(),
            ]
        );
        let env: BTreeMap<&str, &EnvValue> = spec
            .env
            .iter()
            .map(|(name, value)| (name.as_str(), value))
            .collect();
        assert_eq!(
            env["PYTHONDONTWRITEBYTECODE"],
            &EnvValue::Plain("1".to_owned())
        );
        assert_eq!(
            env["TOKEN"],
            &EnvValue::Secret(Secret::new(TOKEN.to_owned()))
        );
        assert_eq!(spec.cwd, Path::new(WORK));
        assert_eq!(spec.limits.wall, Duration::from_secs(40 * 60));
    }

    #[tokio::test]
    async fn a_rate_limited_attempt_is_discarded_and_the_next_one_numbered_after_it() {
        let bench = Bench::new();
        let limited = Agent::new(|_| Play {
            exit: sandbox::Exit::Killed,
            ..Play::transcript("claude-code/synthetic-rate-limit-event.jsonl")
        });
        let resets_at = DateTime::from_timestamp(1_793_762_400, 0);
        let Attempted::Discarded { attempt, reason } = bench.attempt("opus", &limited).await else {
            panic!("expected a discarded attempt");
        };
        assert_eq!(attempt, AttemptNo::FIRST);
        assert_eq!(reason, Reason::RateLimited { resets_at });
        let trial = bench.trial("opus");
        let discarded: DiscardedAttempt =
            serde_json::from_str(&read(&trial.attempt(attempt).discarded())).unwrap();
        assert_eq!(discarded.reason, reason);
        assert_eq!(
            discarded.process.and_then(|process| process.cut),
            Some(Cut::RateLimited)
        );
        let output: u64 = discarded.usage.iter().map(|s| s.tokens.output).sum();
        assert_eq!(output, 40);
        assert!(!trial.phase(Phase::Run).exists());
        bench.assert_clean();

        let finished = Agent::new(|_| Play::transcript("claude-code/haiku-bash-success.jsonl"));
        let Attempted::Committed(record) = bench.attempt("opus", &finished).await else {
            panic!("expected a committed run");
        };
        assert_eq!(record.attempt(), AttemptNo::FIRST.next());
    }

    #[tokio::test]
    async fn an_exit_stdout_does_not_explain_is_diagnosed_from_stderr() {
        let bench = Bench::new();
        let agent = Agent::new(|_| {
            let mut play = Play::transcript("claude-code/haiku-bash-success.jsonl");
            play.stdout.truncate(1);
            play.stderr = vec!["Error: Claude AI usage limit reached".to_owned()];
            play.exit = sandbox::Exit::Code(1);
            play
        });
        let attempted = bench.attempt("opus", &agent).await;
        assert!(
            matches!(
                attempted,
                Attempted::Discarded {
                    reason: Reason::RateLimited { resets_at: None },
                    ..
                }
            ),
            "{attempted:?}"
        );
    }

    /// How an attempt left its trial, as a test expects it.
    #[derive(Debug, PartialEq)]
    enum Left {
        Committed(AgentExit),
        Discarded(Reason),
    }

    #[tokio::test(start_paused = true)]
    async fn only_a_session_that_ended_on_its_own_is_committed_and_the_rest_are_discarded() {
        let played = |name: &str| Play::transcript(&format!("claude-code/{name}"));
        let cut_short = |exit| {
            let mut play = played("haiku-bash-success.jsonl");
            play.stdout.truncate(2);
            Play { exit, ..play }
        };
        let cases = [
            (
                "completed",
                played("haiku-bash-success.jsonl"),
                Left::Committed(AgentExit::Completed),
            ),
            (
                "refused",
                played("synthetic-refusal.jsonl"),
                Left::Committed(AgentExit::Refused),
            ),
            (
                "errored",
                played("unknown-model-error.jsonl"),
                Left::Discarded(Reason::Errored {
                    message: "There's an issue with the selected model (claude-nonexistent-9). \
                              It may not exist or you may not have access to it. Run --model \
                              to pick a different model."
                        .to_owned(),
                }),
            ),
            (
                "crashed",
                cut_short(sandbox::Exit::Code(1)),
                Left::Discarded(Reason::Crashed),
            ),
            (
                "out of memory",
                cut_short(sandbox::Exit::OutOfMemory),
                Left::Discarded(Reason::Crashed),
            ),
            (
                "wall budget",
                cut_short(sandbox::Exit::TimedOut),
                Left::Discarded(Reason::TimedOut {
                    budget: Timeout::Wall,
                }),
            ),
            (
                "idle budget",
                Play {
                    silence: Duration::from_secs(3600),
                    ..cut_short(sandbox::Exit::Killed)
                },
                Left::Discarded(Reason::TimedOut {
                    budget: Timeout::Idle,
                }),
            ),
            (
                "second ctrl-c",
                cut_short(sandbox::Exit::Killed),
                Left::Discarded(Reason::Stopped),
            ),
            (
                "sandbox fault",
                Play {
                    fault: Some("the scope vanished".to_owned()),
                    ..cut_short(sandbox::Exit::Code(0))
                },
                Left::Discarded(Reason::Fault {
                    message: "sandbox fault: the scope vanished".to_owned(),
                }),
            ),
        ];
        for (label, play, expected) in cases {
            let bench = Bench::new();
            let agent = Agent::new(move |_| play.clone());
            let trial = bench.trial("opus");
            let attempt = trial.attempt(AttemptNo::FIRST);
            let left = match bench.attempt("opus", &agent).await {
                Attempted::Committed(record) => {
                    assert!(!attempt.discarded().exists(), "{label}");
                    Left::Committed(record.outcome().exit.clone())
                }
                Attempted::Discarded { reason, .. } => {
                    assert!(!trial.phase(Phase::Run).exists(), "{label}");
                    let stored: DiscardedAttempt =
                        serde_json::from_str(&read(&attempt.discarded())).unwrap();
                    assert_eq!(stored.reason, reason, "{label}");
                    assert!(attempt.transcript().is_file(), "{label}");
                    Left::Discarded(reason)
                }
            };
            assert_eq!(left, expected, "{label}");
            bench.assert_clean();
        }
    }

    #[tokio::test]
    async fn an_endpoint_backend_commits_a_session_and_discards_what_the_endpoint_cut_short() {
        let failed = |name: &str| Play {
            exit: sandbox::Exit::Code(1),
            ..Play::transcript(&format!("codex/{name}"))
        };
        let mut completed = Play::transcript("codex/nemotron-command-success.jsonl");
        completed.stdout.insert(
            3,
            format!(r#"{{"type":"item.updated","text":"{HOSTED_KEY}"}}"#),
        );
        let cases = [
            (
                "completed",
                completed,
                Left::Committed(AgentExit::Completed),
            ),
            (
                "429",
                failed("endpoint-429.jsonl"),
                Left::Discarded(Reason::RateLimited { resets_at: None }),
            ),
            (
                "server error",
                failed("endpoint-500.jsonl"),
                Left::Discarded(Reason::Errored {
                    message: "We’re currently experiencing high demand, which may cause \
                              temporary errors."
                        .to_owned(),
                }),
            ),
            (
                "dropped stream",
                failed("endpoint-stream-drop.jsonl"),
                Left::Discarded(Reason::Errored {
                    message: "stream disconnected before completion: stream closed before \
                              response.completed"
                        .to_owned(),
                }),
            ),
        ];
        for (label, play, expected) in cases {
            let bench = Bench::at("nemo", "hosted");
            let agent = Agent::new(move |spec| {
                let home = spec
                    .mounts
                    .iter()
                    .find(|mount| mount.guest == Path::new(HOME))
                    .unwrap();
                let config = read(&home.host.join(".codex/config.toml"));
                assert!(
                    config.contains(r#"base_url = "https://api.hosted.example/v1""#),
                    "{config}"
                );
                assert!(!config.contains(HOSTED_KEY), "{config}");
                play.clone()
            });
            let attempt = bench.trial("nemo").attempt(AttemptNo::FIRST);
            let left = match bench.attempt("nemo", &agent).await {
                Attempted::Committed(record) => {
                    let inputs = serde_json::to_value(record.inputs()).unwrap();
                    assert_eq!(inputs["api_model"], "vendor/nemo", "{label}");
                    assert_eq!(
                        inputs["endpoint"], "https://api.hosted.example/v1",
                        "{label}"
                    );
                    Left::Committed(record.outcome().exit.clone())
                }
                Attempted::Discarded { reason, .. } => {
                    assert!(attempt.discarded().is_file(), "{label}");
                    Left::Discarded(reason)
                }
            };
            assert_eq!(left, expected, "{label}");
            let transcript = read(&attempt.transcript());
            assert!(!transcript.contains(HOSTED_KEY), "{label}");

            let [launched] = agent.launches().try_into().unwrap();
            assert_eq!(
                launched.argv[..7],
                [
                    "codex",
                    "exec",
                    "--json",
                    "--skip-git-repo-check",
                    "--dangerously-bypass-approvals-and-sandbox",
                    "-m",
                    "vendor/nemo"
                ]
            );
            assert!(
                launched.spec.env.contains(&(
                    "HOSTED_API_KEY".to_owned(),
                    EnvValue::Secret(Secret::new(HOSTED_KEY.to_owned()))
                )),
                "{label}"
            );
            bench.assert_clean();
        }
    }

    #[tokio::test]
    async fn a_relayed_backend_keeps_its_key_out_of_the_sandbox_and_records_its_endpoint() {
        let bench = Bench::at("tron", "throttled");
        let agent = Agent::new(|spec| {
            for (name, value) in &spec.env {
                let EnvValue::Plain(value) = value else {
                    panic!("{name} is a secret");
                };
                assert!(!value.contains(THROTTLED_KEY), "{name}");
            }
            assert!(matches!(spec.network, Network::Egress(_)));
            for mount in &spec.mounts {
                for file in files(&mount.host) {
                    let text = read(&mount.host.join(&file));
                    assert!(!text.contains(THROTTLED_KEY), "{}", file.display());
                }
            }
            let home = spec
                .mounts
                .iter()
                .find(|mount| mount.guest == Path::new(HOME))
                .unwrap();
            let config = read(&home.host.join(".codex/config.toml"));
            assert!(
                config.contains(r#"base_url = "http://relay.invalid""#),
                "{config}"
            );
            assert!(!config.contains("env_key"), "{config}");
            Play::transcript("codex/nemotron-command-success.jsonl")
        });
        let Attempted::Committed(record) = bench.attempt("tron", &agent).await else {
            panic!("expected a committed run");
        };
        let inputs = serde_json::to_value(record.inputs()).unwrap();
        assert_eq!(inputs["api_model"], "vendor/tron");
        assert_eq!(inputs["endpoint"], "https://api.throttled.example/v1");
        assert_eq!(record.outcome().exit, AgentExit::Completed);

        let [launched] = agent.launches().try_into().unwrap();
        assert_eq!(launched.argv[5..7], ["-m", "vendor/tron"]);
        assert!(
            !launched
                .spec
                .env
                .iter()
                .any(|(name, _)| name == "THROTTLED_API_KEY")
        );
        bench.assert_clean();
    }

    #[test]
    fn a_discard_record_without_a_reason_reads_as_a_rate_limit() {
        let old = r#"{
            "session": "20261103T110000Z-7",
            "started": "2026-11-03T11:00:00Z",
            "process": {"exit": "killed", "cut": "rate-limited", "wall": "30s",
                        "stdout_bytes": 0, "stderr_bytes": 0},
            "parser": 1,
            "resets_at": "2026-11-03T16:00:00Z",
            "usage": []
        }"#;
        let read: DiscardedAttempt = serde_json::from_str(old).unwrap();
        assert_eq!(
            read.reason,
            Reason::RateLimited {
                resets_at: DateTime::from_timestamp(1_793_721_600, 0)
            }
        );
        let again: DiscardedAttempt =
            serde_json::from_str(&serde_json::to_string(&read).unwrap()).unwrap();
        assert_eq!(again, read);
    }

    #[tokio::test]
    async fn a_login_codex_refreshed_goes_back_to_the_master_and_is_redacted() {
        let bench = Bench::new();
        let refreshed = LOGIN
            .replace("2026-11-03T20:04:12Z", "2026-11-03T21:00:00Z")
            .replace("eyJ.access", "eyJ.renewed");
        let agent = Agent::new({
            let refreshed = refreshed.clone();
            move |spec| {
                let codex_home = spec.env.iter().find(|(name, _)| name == "CODEX_HOME");
                assert_eq!(
                    codex_home.map(|(_, value)| value),
                    Some(&EnvValue::Plain("/home/agent/.codex".to_owned()))
                );
                let mut play = Play::transcript("codex/luna-command-success.jsonl");
                play.home = vec![(".codex/auth.json".to_owned(), refreshed.clone())];
                play.stdout.insert(
                    2,
                    r#"{"type":"item.updated","text":"eyJ.renewed"}"#.to_owned(),
                );
                play
            }
        });
        let Attempted::Committed(record) = bench.attempt("luna", &agent).await else {
            panic!("expected a committed run");
        };
        assert_eq!(record.outcome().exit, AgentExit::Completed);
        let master = bench.home.path().join("codex/auth.json");
        assert_eq!(read(&master), refreshed);
        let transcript = read(&bench.trial("luna").attempt(AttemptNo::FIRST).transcript());
        assert!(!transcript.contains("eyJ.renewed"), "{transcript}");
        bench.assert_clean();
    }
}
