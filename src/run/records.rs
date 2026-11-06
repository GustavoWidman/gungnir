//! What the run phase commits: the inputs a run was made from, the run record with the
//! agent's outcome, and the record of an attempt that is no sample, with the policy that tells
//! the two apart.
use super::{Task, workspace};
use crate::{
    experiment::Budget,
    grade::duration,
    harness::{self, AgentOutcome, Ending, ProcessEnd, QuotaSpan, TokenUsage, UsageSample},
    id::AttemptNo,
    provenance::SessionId,
    sandbox,
    store::Digest,
    toolchains::Toolchains,
};
use chrono::{DateTime, Utc};
use eyre::{Result, WrapErr};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt, path::PathBuf, time::Duration};

/// Everything a run depends on that gungnir controls, and the time budgets it ran under.
/// Equal inputs make the same kind of sample, so a committed run is stale exactly when
/// [`RunInputs::is_stale`] says its inputs differ from the current ones.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunInputs {
    system: String,
    api_model: String,
    endpoint: Option<String>,
    prompt: Digest,
    /// The workspace the agent starts in.
    workspace: Digest,
    toolchains: StoredToolchains,
    /// Recorded, never compared, like `idle`.
    #[serde(with = "duration")]
    wall: Duration,
    #[serde(with = "duration")]
    idle: Duration,
}

/// The store paths of the environments an agent runs in, each naming its whole closure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct StoredToolchains {
    base: PathBuf,
    language: PathBuf,
    harness: PathBuf,
}

impl RunInputs {
    /// The inputs of running `task` now, for an agent that starts in a workspace with digest
    /// `workspace`.
    pub fn new(
        task: &Task<'_>,
        toolchains: &Toolchains,
        budget: Budget,
        workspace: Digest,
    ) -> Result<Self> {
        let system = &task.trial.system;
        Ok(Self {
            system: system.id().to_string(),
            api_model: system.api_model().to_owned(),
            endpoint: task.backend.endpoint().map(|url| url.as_str().to_owned()),
            prompt: Digest::of_bytes(task.prompt(toolchains)?.as_bytes()),
            workspace,
            toolchains: StoredToolchains {
                base: toolchains.base().root().to_owned(),
                language: toolchains
                    .language(task.language.id().as_str())?
                    .root()
                    .to_owned(),
                harness: toolchains
                    .harness(system.harness().as_str())?
                    .root()
                    .to_owned(),
            },
            wall: budget.wall(),
            idle: budget.idle(),
        })
    }

    /// Whether a run made from these inputs is stale against `current`, the inputs of running
    /// its trial now. The budgets are left out: they only bound how long gungnir waits for a
    /// session to end on its own, and only such a session is a sample, so raising them changes
    /// no sample already taken.
    pub fn is_stale(&self, current: &Self) -> bool {
        let Self {
            system,
            api_model,
            endpoint,
            prompt,
            workspace,
            toolchains,
            wall: _,
            idle: _,
        } = self;
        (system, api_model, endpoint, prompt, workspace, toolchains)
            != (
                &current.system,
                &current.api_model,
                &current.endpoint,
                &current.prompt,
                &current.workspace,
                &current.toolchains,
            )
    }

    /// The inputs of running `task` now, scaffolding its workspace in scratch to digest it.
    pub fn current(task: &Task<'_>, toolchains: &Toolchains, budget: Budget) -> Result<Self> {
        let scratch = tempfile::Builder::new()
            .prefix("gungnir-scaffold-")
            .tempdir()
            .wrap_err("creating scaffolding scratch")?;
        let digest = workspace::scaffold(task.language, task.brief, scratch.path())?;
        Self::new(task, toolchains, budget, digest)
    }
}

/// `run/record.json`: the attempt a trial committed as its sample. The run directory also holds
/// the agent's `prompt.md` and its workspace as it left it in `final/`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    pub(super) session: SessionId,
    pub(super) inputs: RunInputs,
    pub(super) attempt: AttemptNo,
    pub(super) started: DateTime<Utc>,
    pub(super) process: Process,
    /// The version of the harness's parser that folded `outcome` from the transcript.
    pub(super) parser: u32,
    pub(super) outcome: Outcome,
    /// The digest of `final/`, which is what grading judges.
    pub(super) snapshot: Digest,
}

impl RunRecord {
    /// The invocation that ran the attempt.
    pub fn session(&self) -> &SessionId {
        &self.session
    }

    pub fn inputs(&self) -> &RunInputs {
        &self.inputs
    }

    /// The attempt this run committed, whose transcript is under `attempts/<n>/`.
    pub fn attempt(&self) -> AttemptNo {
        self.attempt
    }

    pub fn started(&self) -> DateTime<Utc> {
        self.started
    }

    pub fn process(&self) -> &Process {
        &self.process
    }

    pub fn parser(&self) -> u32 {
        self.parser
    }

    pub fn outcome(&self) -> &Outcome {
        &self.outcome
    }

    pub fn snapshot(&self) -> Digest {
        self.snapshot
    }
}

/// `attempts/<n>/discarded.json`: an attempt that is no sample. It is the run phase's own
/// bookkeeping, which tells a discarded attempt from one a crash interrupted and keeps attempt
/// numbers unique across retries; nothing reports it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "StoredDiscard")]
pub struct DiscardedAttempt {
    /// The invocation that discarded the attempt: the one that ran it, or a later one that
    /// found its committed run was no sample.
    pub(super) session: SessionId,
    pub(super) started: DateTime<Utc>,
    /// How the agent's process ended, unless the sandbox could not tell.
    pub(super) process: Option<Process>,
    pub(super) parser: u32,
    pub(super) reason: Reason,
    pub(super) usage: Vec<Sample>,
}

/// `discarded.json` as files of any age hold it. Before discards had reasons, only rate limits
/// were discarded, with the vendor's reset time beside the usage.
#[derive(Deserialize)]
struct StoredDiscard {
    session: SessionId,
    started: DateTime<Utc>,
    process: Option<Process>,
    parser: u32,
    reason: Option<Reason>,
    resets_at: Option<DateTime<Utc>>,
    usage: Vec<Sample>,
}

impl From<StoredDiscard> for DiscardedAttempt {
    fn from(stored: StoredDiscard) -> Self {
        let StoredDiscard {
            session,
            started,
            process,
            parser,
            reason,
            resets_at,
            usage,
        } = stored;
        Self {
            session,
            started,
            process,
            parser,
            reason: reason.unwrap_or(Reason::RateLimited { resets_at }),
            usage,
        }
    }
}

/// Why an attempt is no sample. A sample is an agent session that ended on its own: the CLI
/// reported it complete, or the model declined the task. Anything else says nothing about the
/// model, so the attempt is discarded and its trial runs again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Reason {
    /// The account's quota ended the session, and the vendor may have said when it resets.
    RateLimited { resets_at: Option<DateTime<Utc>> },
    /// The CLI reported an error that ended the session.
    Errored { message: String },
    /// The CLI ended without its final report.
    Crashed,
    /// gungnir ended the agent when one of its time budgets ran out.
    TimedOut { budget: Timeout },
    /// The second Ctrl-C ended the agent.
    Stopped,
    /// The sandbox could not run the agent to its end.
    Fault { message: String },
}

/// Which of an attempt's time budgets ran out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Timeout {
    Wall,
    Idle,
}

impl Reason {
    /// Why an attempt whose agent ended with `exit`, its process ending as `process` says, is
    /// no sample, or `None` when it is one.
    pub fn of(exit: &AgentExit, process: &Process) -> Option<Self> {
        match exit {
            AgentExit::Completed | AgentExit::Refused => None,
            AgentExit::Errored { message } => Some(Self::Errored {
                message: message.clone(),
            }),
            AgentExit::Crashed => Some(Self::Crashed),
            AgentExit::Killed => Some(match (process.cut, process.exit) {
                (Some(Cut::RateLimited), _) => Self::RateLimited { resets_at: None },
                (Some(Cut::Idle), _) => Self::TimedOut {
                    budget: Timeout::Idle,
                },
                (None, ProcessExit::TimedOut) => Self::TimedOut {
                    budget: Timeout::Wall,
                },
                (None, _) => Self::Stopped,
            }),
        }
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RateLimited {
                resets_at: Some(at),
            } => write!(f, "rate limited until {at}"),
            Self::RateLimited { resets_at: None } => f.write_str("rate limited"),
            Self::Errored { message } => write!(f, "the CLI reported an error: {message}"),
            Self::Crashed => f.write_str("the CLI ended without its final report"),
            Self::TimedOut {
                budget: Timeout::Wall,
            } => f.write_str("the wall budget ran out"),
            Self::TimedOut {
                budget: Timeout::Idle,
            } => f.write_str("the idle budget ran out"),
            Self::Stopped => f.write_str("stopped by the second Ctrl-C"),
            Self::Fault { message } => f.write_str(message),
        }
    }
}

/// How an attempt counts: as a sample, or not, for a reason.
#[derive(Debug)]
pub(super) enum Counted {
    Sample(Outcome),
    Discarded { reason: Reason, usage: Vec<Sample> },
}

impl Counted {
    /// How an attempt that ended as `ending`, its process ending as `process` says, counts.
    pub(super) fn of(ending: Ending, process: &Process) -> Self {
        match ending {
            Ending::RateLimited { resets_at, usage } => Self::Discarded {
                reason: Reason::RateLimited { resets_at },
                usage: samples(usage),
            },
            Ending::Finished(outcome) => {
                let outcome = Outcome::from(outcome);
                match Reason::of(&outcome.exit, process) {
                    None => Self::Sample(outcome),
                    Some(reason) => Self::Discarded {
                        reason,
                        usage: outcome.usage,
                    },
                }
            }
        }
    }
}

/// How the agent's process ended, as the runner saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Process {
    pub exit: ProcessExit,
    /// Why gungnir ended the process before it exited, when it did.
    pub cut: Option<Cut>,
    /// From launch to exit, sandbox setup included.
    #[serde(with = "duration")]
    pub wall: Duration,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
}

impl Process {
    /// Whether the process ended on its own or the runner ended it, as the fold needs to know.
    pub fn end(&self) -> ProcessEnd {
        match self.exit {
            ProcessExit::TimedOut | ProcessExit::Killed => ProcessEnd::Killed,
            ProcessExit::Code(_) | ProcessExit::Signal(_) | ProcessExit::OutOfMemory => {
                ProcessEnd::Exited
            }
        }
    }
}

/// [`sandbox::Exit`] as a record stores it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProcessExit {
    Code(i32),
    Signal(i32),
    TimedOut,
    OutOfMemory,
    Killed,
}

impl From<sandbox::Exit> for ProcessExit {
    fn from(exit: sandbox::Exit) -> Self {
        match exit {
            sandbox::Exit::Code(code) => Self::Code(code),
            sandbox::Exit::Signal(signal) => Self::Signal(signal),
            sandbox::Exit::TimedOut => Self::TimedOut,
            sandbox::Exit::OutOfMemory => Self::OutOfMemory,
            sandbox::Exit::Killed => Self::Killed,
        }
    }
}

/// Why gungnir ended an agent early.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Cut {
    /// No stdout line arrived within the idle budget.
    Idle,
    /// The agent's stdout reported that the account's quota ended the session.
    RateLimited,
}

/// [`AgentOutcome`] as a record stores it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub exit: AgentExit,
    pub turns: Option<u32>,
    pub usage: Vec<Sample>,
    #[serde(with = "optional_duration")]
    pub api_time: Option<Duration>,
    pub model: Option<String>,
    pub quota: BTreeMap<String, Quota>,
    pub unrecognised: BTreeMap<String, u32>,
}

impl Outcome {
    pub fn total(&self) -> TokenUsage {
        self.usage.iter().map(|sample| sample.tokens.into()).sum()
    }
}

impl From<AgentOutcome> for Outcome {
    fn from(outcome: AgentOutcome) -> Self {
        let AgentOutcome {
            exit,
            turns,
            usage,
            api_time,
            model,
            quota,
            unrecognised,
        } = outcome;
        Self {
            exit: exit.into(),
            turns,
            usage: samples(usage),
            api_time,
            model,
            quota: quota
                .into_iter()
                .map(|(window, QuotaSpan { first, last })| (window, Quota { first, last }))
                .collect(),
            unrecognised,
        }
    }
}

/// [`harness::Exit`] as a record stores it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentExit {
    Completed,
    Refused,
    Errored { message: String },
    Crashed,
    Killed,
}

impl From<harness::Exit> for AgentExit {
    fn from(exit: harness::Exit) -> Self {
        match exit {
            harness::Exit::Completed => Self::Completed,
            harness::Exit::Refused => Self::Refused,
            harness::Exit::Errored { message } => Self::Errored { message },
            harness::Exit::Crashed => Self::Crashed,
            harness::Exit::Killed => Self::Killed,
        }
    }
}

/// [`UsageSample`] as a record stores it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sample {
    pub request: Option<String>,
    pub model: Option<String>,
    pub tokens: Tokens,
}

/// [`TokenUsage`] as a record stores it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    pub input: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cache_write_long: u64,
    pub output: u64,
    pub reasoning: Option<u64>,
}

impl From<Tokens> for TokenUsage {
    fn from(tokens: Tokens) -> Self {
        let Tokens {
            input,
            cache_read,
            cache_write,
            cache_write_long,
            output,
            reasoning,
        } = tokens;
        Self {
            input,
            cache_read,
            cache_write,
            cache_write_long,
            output,
            reasoning,
        }
    }
}

/// [`QuotaSpan`] as a record stores it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Quota {
    pub first: f64,
    pub last: f64,
}

pub(super) fn samples(usage: Vec<UsageSample>) -> Vec<Sample> {
    usage
        .into_iter()
        .map(
            |UsageSample {
                 request,
                 model,
                 tokens,
             }| {
                let TokenUsage {
                    input,
                    cache_read,
                    cache_write,
                    cache_write_long,
                    output,
                    reasoning,
                } = tokens;
                Sample {
                    request,
                    model,
                    tokens: Tokens {
                        input,
                        cache_read,
                        cache_write,
                        cache_write_long,
                        output,
                        reasoning,
                    },
                }
            },
        )
        .collect()
}

/// Optional durations as humantime text, like every other duration in the results.
mod optional_duration {
    use serde::{Deserialize, Deserializer, Serializer, de};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(
        duration: &Option<Duration>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match duration {
            Some(duration) => serializer.collect_str(&humantime::format_duration(*duration)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Duration>, D::Error> {
        Option::<String>::deserialize(deserializer)?
            .map(|text| humantime::parse_duration(&text).map_err(de::Error::custom))
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        harness::{Ending, Harness},
        run::fixture::{self, Repo},
        testing::write,
    };
    use std::{fs, path::Path};

    fn current(repo: &Repo) -> RunInputs {
        let experiment = repo.experiment();
        let trial = fixture::trial(&experiment, "opus");
        let task = Task::of(&experiment, &trial).unwrap();
        RunInputs::current(&task, &repo.toolchains, experiment.budget()).unwrap()
    }

    /// Replaces `from`, which must occur, with `to` in the file at `root/path`.
    fn replace(root: &Path, path: &str, from: &str, to: &str) {
        let before = fs::read_to_string(root.join(path)).unwrap();
        assert!(before.contains(from), "{path} lacks {from}");
        write(root, path, before.replace(from, to));
    }

    #[test]
    fn stored_inputs_go_stale_exactly_when_the_agent_would_see_something_else() {
        type Edit = fn(&Path);
        let edits: [(Edit, bool); 10] = [
            (
                |root| write(root, "scenarios/echo/hidden/edge.out", "x\n"),
                false,
            ),
            (
                |root| write(root, "scenarios/echo/reference/python/main.py", "pass\n"),
                false,
            ),
            (
                |root| write(root, "scenarios/echo/spec.md", "Echo.\n"),
                true,
            ),
            (
                |root| write(root, "scenarios/echo/examples/basic.out", "b\n"),
                true,
            ),
            (
                |root| write(root, "languages/python/template/main.py", "#\n"),
                true,
            ),
            (
                |root| replace(root, "languages/python/preamble.md", "Build", "Compile"),
                true,
            ),
            (
                |root| {
                    replace(
                        root,
                        "languages/python/language.toml",
                        "[\"true\"]",
                        "[\"false\"]",
                    )
                },
                true,
            ),
            (
                |root| replace(root, "experiments/fixture.toml", "\"10m\"", "\"30m\""),
                false,
            ),
            (
                |root| replace(root, "experiments/fixture.toml", "\"40m\"", "\"4h\""),
                false,
            ),
            (
                |root| replace(root, "experiments/fixture.toml", "\"40m\"", "\"20m\""),
                false,
            ),
        ];
        for (index, (edit, stale)) in edits.into_iter().enumerate() {
            let repo = Repo::new();
            let json = serde_json::to_string(&current(&repo)).unwrap();
            let stored: RunInputs = serde_json::from_str(&json).unwrap();
            assert_eq!(stored, current(&repo));
            assert!(!stored.is_stale(&current(&repo)));

            edit(repo.root.path());
            assert_eq!(stored.is_stale(&current(&repo)), stale, "edit {index}");
        }
    }

    #[test]
    fn the_inputs_of_a_backend_at_its_vendors_endpoint_keep_their_stored_form() {
        let stored = r#"{"system":"codex-0.166.0_openai_gpt-6.1-terra_high","api_model":"gpt-6.1-terra","endpoint":null,"prompt":"e16a921f1ebc8a9900692d18fee1b71c4a1ebe2d5b72d5f662ae8d6323ef6dbe","workspace":"8f0afac15ce1e597476b66d3cbaa3560528b0c91bf05312f7b23d3442e061c26","toolchains":{"base":"/nix/store/9n09h64ihkgb3bn80hpyrdviz5qxrl7c-gungnir-base","language":"/nix/store/3vjfd762ak1sk996ssdyxxrkkjngk8y5-gungnir-language-cpp","harness":"/nix/store/46s3jghp9ji7vbq842zgnfdg8q6d98fy-gungnir-harness-codex"},"wall":"4h","idle":"30m"}"#;
        let inputs: RunInputs = serde_json::from_str(stored).unwrap();
        assert_eq!(serde_json::to_string(&inputs).unwrap(), stored);

        let repo = Repo::new();
        let experiment = repo.experiment();
        let trial = fixture::trial(&experiment, "luna");
        let task = Task::of(&experiment, &trial).unwrap();
        let current = RunInputs::current(&task, &repo.toolchains, experiment.budget()).unwrap();
        let json = serde_json::to_value(current).unwrap();
        assert_eq!(json["endpoint"], serde_json::Value::Null);
    }

    #[test]
    fn the_budgets_a_run_ran_under_stay_on_its_record() {
        let repo = Repo::new();
        let json = serde_json::to_value(current(&repo)).unwrap();
        assert_eq!(
            (&json["wall"], &json["idle"]),
            (&"40m".into(), &"10m".into())
        );
    }

    #[test]
    fn a_run_record_reads_back_with_its_outcome() {
        let repo = Repo::new();
        let transcript = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/harness/claude-code/haiku-bash-success.jsonl"),
        )
        .unwrap();
        let claude = Harness::named(&"claude-code".parse().unwrap()).unwrap();
        let Ending::Finished(outcome) = claude.replay(&transcript, &[]).finish(ProcessEnd::Exited)
        else {
            panic!("the fixture finishes");
        };
        let total = outcome.total();
        let record = RunRecord {
            session: serde_json::from_str("\"20261103T120000Z-42\"").unwrap(),
            inputs: current(&repo),
            attempt: AttemptNo::FIRST,
            started: Utc::now(),
            process: Process {
                exit: sandbox::Exit::Code(0).into(),
                cut: None,
                wall: Duration::from_millis(91_250),
                stdout_bytes: 12_000,
                stderr_bytes: 0,
            },
            parser: claude.parser_version(),
            outcome: outcome.into(),
            snapshot: Digest::of_bytes(b"final"),
        };

        let json = serde_json::to_string_pretty(&record).unwrap();
        let read: RunRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(read, record);
        assert_eq!(read.outcome().total(), total);
        assert_eq!(read.outcome().exit, AgentExit::Completed);
        assert_eq!(read.outcome().api_time, Some(Duration::from_millis(1634)));
        assert_eq!(read.process().end(), ProcessEnd::Exited);
        assert!(json.contains("\"wall\": \"1m 31s 250ms\""), "{json}");
    }
}
