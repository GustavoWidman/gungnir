//! One attempt: the agent launched headless in a sandbox whose only way out is the attempt's
//! own egress proxy, its stdout streamed into the transcript and decoded as it arrives, and
//! the trial's run directory or the attempt's discard record committed from how it ended.
use super::{
    Cut, DiscardedAttempt, Process, RunInputs, RunRecord, Sample, Task, records::samples, workspace,
};
use crate::{
    egress::{Allowlist, Proxy},
    experiment::Budget,
    harness::{self, Ending, Fold, Harness, LaunchRequest, ProcessEnd},
    id::AttemptNo,
    provenance::SessionId,
    sandbox::{
        self, Backend, Finished, HOME, Input, Io, Kill, Limits, Mount, Network, Output, Spec, WORK,
    },
    secret::{EnvValue, Redactor},
    stop::Stop,
    store::{PartialDir, write_json},
    toolchains::{Closure, Toolchains},
    trial::{AttemptDir, FINAL, PROMPT, Phase, ResultsDir},
    vault::{Home, Vault},
};
use chrono::{DateTime, Utc};
use eyre::{Result, WrapErr, eyre};
use globset::{Glob, GlobSetBuilder};
use std::{
    ffi::OsString,
    fs::{self, File},
    future::Future,
    io::{ErrorKind, Read, Seek, SeekFrom, Write},
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
    /// The trial's run directory is committed with this record.
    Committed(Box<RunRecord>),
    /// The account's quota ended the session. The attempt is discarded and the trial is still
    /// pending.
    RateLimited {
        attempt: AttemptNo,
        resets_at: Option<DateTime<Utc>>,
    },
    /// The second Ctrl-C killed the agent. The attempt stays as interrupted.
    Interrupted { attempt: AttemptNo },
}

/// What the sandbox reported, and why gungnir cut the agent short when it did.
struct Ran {
    finished: Finished,
    cut: Option<Cut>,
}

impl Rig<'_> {
    /// Makes the next attempt of `task`'s trial. An error leaves the attempt, if it started,
    /// as interrupted.
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
        let prompt = task.language.prompt(task.brief);

        let key = self.vault.key(system.backend())?;
        let launch = harness.launch(&LaunchRequest {
            system,
            endpoint: key.endpoint(),
            credential: key.material(),
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
        let process = Process {
            exit: finished.exit.into(),
            cut,
            wall: finished.wall,
            stdout_bytes: finished.stdout.total,
            stderr_bytes: finished.stderr.total,
        };
        // Only the second Ctrl-C kills an agent that gungnir did not cut short.
        if finished.exit == sandbox::Exit::Killed && cut.is_none() {
            return Ok(Attempted::Interrupted { attempt: number });
        }
        match conclude(harness, &attempt, process.end())? {
            Ending::RateLimited { resets_at, usage } => {
                let discarded = DiscardedAttempt {
                    session: self.session.clone(),
                    started,
                    process,
                    parser: harness.parser_version(),
                    resets_at,
                    usage: samples(usage),
                };
                write_json(&attempt.discarded(), &discarded)?;
                Ok(Attempted::RateLimited {
                    attempt: number,
                    resets_at,
                })
            }
            Ending::Finished(outcome) => {
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
                    outcome: outcome.into(),
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
        Ok(Ran {
            finished: finished?,
            cut,
        })
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

/// What the attempt in `attempt`, which committed nothing, spent as far as its stored transcript
/// and artifacts tell. An attempt that has not written a transcript yet spent nothing visible.
pub fn replay_usage(harness: Harness, attempt: &AttemptDir) -> Result<Vec<Sample>> {
    let path = attempt.transcript();
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err).wrap_err_with(|| format!("reading {}", path.display())),
    };
    let artifacts = stored_artifacts(&attempt.artifacts())?;
    let artifacts: Vec<&str> = artifacts.iter().map(String::as_str).collect();
    let fold = harness.replay(&String::from_utf8_lossy(&bytes), &artifacts);
    let usage = match fold.finish(ProcessEnd::Killed) {
        Ending::Finished(outcome) => outcome.usage,
        Ending::RateLimited { usage, .. } => usage,
    };
    Ok(samples(usage))
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
            fixture::{self, Agent, LOGIN, Play, Repo, TOKEN},
        },
        secret::Secret,
        store::read_record,
        trial::TrialDir,
    };
    use globset::GlobSet;
    use std::{collections::BTreeMap, path::PathBuf};
    use tempfile::TempDir;

    /// The fixture repository with its credentials, an open vault and a session.
    struct Bench {
        repo: Repo,
        home: TempDir,
        runtime: TempDir,
        experiment: Experiment,
        vault: Vault,
        stop: Stop,
    }

    impl Bench {
        fn new() -> Self {
            let repo = Repo::new();
            let experiment = repo.experiment();
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
        let prompt = task.language.prompt(task.brief);
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
        let Attempted::RateLimited { attempt, resets_at } = bench.attempt("opus", &limited).await
        else {
            panic!("expected a rate limit");
        };
        assert_eq!(attempt, AttemptNo::FIRST);
        assert_eq!(resets_at, DateTime::from_timestamp(1_793_762_400, 0));
        let trial = bench.trial("opus");
        let discarded: DiscardedAttempt =
            serde_json::from_str(&read(&trial.attempt(attempt).discarded())).unwrap();
        assert_eq!(discarded.resets_at(), resets_at);
        assert_eq!(discarded.process.cut, Some(Cut::RateLimited));
        let output: u64 = discarded.usage().iter().map(|s| s.tokens.output).sum();
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
                Attempted::RateLimited {
                    resets_at: None,
                    ..
                }
            ),
            "{attempted:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_agent_silent_past_the_idle_budget_is_cut_and_its_run_kept() {
        let bench = Bench::new();
        let agent = Agent::new(|_| {
            let mut play = Play::transcript("claude-code/haiku-bash-success.jsonl");
            play.stdout.truncate(2);
            play.silence = Duration::from_secs(3600);
            play.exit = sandbox::Exit::Killed;
            play
        });
        let Attempted::Committed(record) = bench.attempt("opus", &agent).await else {
            panic!("expected a committed run");
        };
        assert_eq!(record.process().cut, Some(Cut::Idle));
        assert_eq!(record.outcome().exit, AgentExit::Killed);
    }

    #[tokio::test]
    async fn an_agent_the_second_ctrl_c_killed_leaves_an_interrupted_attempt() {
        let bench = Bench::new();
        let agent = Agent::new(|_| {
            let mut play = Play::transcript("claude-code/haiku-bash-success.jsonl");
            play.stdout.truncate(2);
            play.exit = sandbox::Exit::Killed;
            play
        });
        let attempted = bench.attempt("opus", &agent).await;
        assert!(
            matches!(attempted, Attempted::Interrupted { attempt } if attempt == AttemptNo::FIRST),
            "{attempted:?}"
        );
        let trial = bench.trial("opus");
        let attempt = trial.attempt(AttemptNo::FIRST);
        assert_eq!(read(&attempt.transcript()).lines().count(), 2);
        assert!(!attempt.discarded().exists());
        assert!(!trial.phase(Phase::Run).exists());
        bench.assert_clean();

        let claude = Harness::named(&"claude-code".parse().unwrap()).unwrap();
        let spent: Vec<(u64, u64, u64)> = replay_usage(claude, &attempt)
            .unwrap()
            .iter()
            .map(|sample| {
                let tokens = sample.tokens;
                (tokens.cache_read, tokens.cache_write_long, tokens.output)
            })
            .collect();
        assert_eq!(spent, [(11_160, 6042, 16)]);
        let unstarted = trial.attempt(AttemptNo::FIRST.next());
        assert!(replay_usage(claude, &unstarted).unwrap().is_empty());
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
