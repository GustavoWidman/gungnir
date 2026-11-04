//! Everything vendor-specific about driving an agent CLI headless: the command that launches
//! it, the events in its stdout, and the fold of those events into how the attempt ended.
//! Each driver is pure, so it is tested against captured transcripts with no process.
mod fold;

pub use fold::{AgentOutcome, Ending, Exit, Fold, ProcessEnd, QuotaSpan};

use crate::{
    catalog::{Endpoint, System},
    id::{Effort, HarnessName, HarnessVersion},
    secret::{EnvValue, Secret},
};
use chrono::{DateTime, Utc};
use eyre::Result;
use std::{
    fmt,
    iter::Sum,
    ops::Add,
    path::{Path, PathBuf},
    time::Duration,
};

trait Driver: Sync {
    /// The key of the driver's `[harnesses.<name>]` table in `gungnir.toml`.
    fn name(&self) -> &'static str;
    fn parser_version(&self) -> u32;
    fn granularity(&self) -> Granularity;
    fn accepts(&self, effort: Effort) -> bool;
    fn parse_version(&self, stdout: &str) -> Result<HarnessVersion>;
    fn launch(&self, req: &LaunchRequest<'_>) -> Result<Launch>;
    fn decode(&self, line: &str) -> Vec<Event>;
    fn diagnose_exit(&self, _stderr_tail: &str) -> Option<Event> {
        None
    }
}

static DRIVERS: &[&dyn Driver] = &[];

/// A registered harness driver. Only [`Harness::all`] and [`Harness::named`] hand one out.
#[derive(Clone, Copy)]
pub struct Harness(&'static dyn Driver);

impl Harness {
    pub fn all() -> impl Iterator<Item = Self> {
        DRIVERS.iter().map(|&driver| Self(driver))
    }

    pub fn named(name: &HarnessName) -> Option<Self> {
        Self::all().find(|harness| harness.name() == name.as_str())
    }

    pub fn name(self) -> &'static str {
        self.0.name()
    }

    /// Increases whenever decoding changes, so outcomes folded by an older parser are
    /// re-derived from their transcripts.
    pub fn parser_version(self) -> u32 {
        self.0.parser_version()
    }

    pub fn granularity(self) -> Granularity {
        self.0.granularity()
    }

    /// Whether the CLI's effort flag can carry `effort`.
    pub fn accepts(self, effort: Effort) -> bool {
        self.0.accepts(effort)
    }

    /// The version in what the CLI printed for `--version`.
    pub fn parse_version(self, stdout: &str) -> Result<HarnessVersion> {
        self.0.parse_version(stdout)
    }

    pub fn launch(self, req: &LaunchRequest<'_>) -> Result<Launch> {
        self.0.launch(req)
    }

    /// The events in one line of the CLI's stdout, given without its newline.
    pub fn decode(self, line: &str) -> Vec<Event> {
        self.0.decode(line)
    }

    /// An event that explains an exit stdout did not, read from the end of stderr.
    pub fn diagnose_exit(self, stderr_tail: &str) -> Option<Event> {
        self.0.diagnose_exit(stderr_tail)
    }

    /// Folds a stored transcript line by line, exactly as the runner folds a live one.
    pub fn replay(self, transcript: &str) -> Fold {
        let mut fold = Fold::default();
        for line in transcript.lines() {
            for event in self.decode(line) {
                fold.push(event);
            }
        }
        fold
    }
}

impl fmt::Debug for Harness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Harness").field(&self.name()).finish()
    }
}

/// What one usage sample covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Granularity {
    /// One API request.
    Request,
    /// One turn of the session, however many requests it made.
    Turn,
}

/// What a CLI reported, in the vocabulary every harness shares.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The session started on `model` with these tools.
    Init {
        model: String,
        tools: Vec<String>,
    },
    Usage(UsageSample),
    /// The CLI's own total for `model` over the whole session. It exceeds the samples when the
    /// CLI reports some tokens only in aggregate.
    SessionUsage {
        model: String,
        tokens: TokenUsage,
    },
    /// How much of one subscription window the account has used, as a fraction.
    Quota {
        window: String,
        utilization: f64,
        resets_at: Option<DateTime<Utc>>,
        allowed: bool,
    },
    /// The account's quota, not the agent, ended the session.
    RateLimited {
        resets_at: Option<DateTime<Utc>>,
    },
    /// The model declined the task.
    Refused,
    /// The CLI's final report on the session.
    Finished {
        turns: Option<u32>,
        api_time: Option<Duration>,
        error: Option<String>,
    },
    /// A known line with no bearing on the outcome.
    Noise,
    /// A line of a kind the driver does not know, named by its type.
    Unrecognised(String),
}

/// The tokens one request or one turn spent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageSample {
    /// The vendor's id for the request. A later sample with the same id replaces this one.
    /// `None` when the CLI does not attribute the tokens to one request.
    pub request: Option<String>,
    /// The model that served the tokens, when the CLI says.
    pub model: Option<String>,
    pub tokens: TokenUsage,
}

/// Tokens in disjoint buckets, so a sum over buckets counts each token once. `reasoning` is
/// the exception: the part of `output` spent reasoning, when the vendor reports it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TokenUsage {
    /// Prompt tokens neither read from nor written to a cache.
    pub input: u64,
    pub cache_read: u64,
    /// Prompt tokens written to the default cache, five minutes at Anthropic.
    pub cache_write: u64,
    /// Prompt tokens written to the one-hour cache.
    pub cache_write_long: u64,
    pub output: u64,
    pub reasoning: Option<u64>,
}

impl TokenUsage {
    /// Each bucket of `self` less the same bucket of `other`, floored at zero.
    fn saturating_sub(self, other: Self) -> Self {
        Self {
            input: self.input.saturating_sub(other.input),
            cache_read: self.cache_read.saturating_sub(other.cache_read),
            cache_write: self.cache_write.saturating_sub(other.cache_write),
            cache_write_long: self.cache_write_long.saturating_sub(other.cache_write_long),
            output: self.output.saturating_sub(other.output),
            reasoning: self
                .reasoning
                .map(|total| total.saturating_sub(other.reasoning.unwrap_or(0))),
        }
    }
}

impl Add for TokenUsage {
    type Output = Self;

    /// Unreported reasoning counts as nothing, so the sum reports whatever its parts did.
    fn add(self, other: Self) -> Self {
        Self {
            input: self.input + other.input,
            cache_read: self.cache_read + other.cache_read,
            cache_write: self.cache_write + other.cache_write,
            cache_write_long: self.cache_write_long + other.cache_write_long,
            output: self.output + other.output,
            reasoning: match (self.reasoning, other.reasoning) {
                (Some(a), Some(b)) => Some(a + b),
                (one, None) | (None, one) => one,
            },
        }
    }
}

impl Sum for TokenUsage {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::default(), Add::add)
    }
}

/// What a driver needs to launch one attempt of a system.
#[derive(Debug)]
pub struct LaunchRequest<'a> {
    pub system: &'a System,
    /// The backend's API base URL, when it is not the vendor's default.
    pub endpoint: Option<&'a Endpoint>,
    pub credential: &'a CredentialMaterial,
    /// The isolated home, as the agent sees it.
    pub home: &'a Path,
    /// The workspace, as the agent sees it.
    pub workspace: &'a Path,
    pub prompt: &'a str,
}

/// A backend's credential as read from its files.
#[derive(Clone, Debug)]
pub enum CredentialMaterial {
    /// The value of the environment variable `var`.
    Env { var: String, value: Secret<String> },
    /// The contents of a Codex `auth.json`.
    CodexAuth(Secret<String>),
}

/// How to start the agent: run `argv` with `env`, after placing `files` in the isolated home.
#[derive(Debug)]
pub struct Launch {
    pub argv: Vec<String>,
    pub env: Vec<(String, EnvValue)>,
    pub files: Vec<HomeFile>,
    /// The prompt, written to stdin, which is then closed.
    pub stdin: String,
}

/// A file to write into the isolated home before launch.
#[derive(Debug)]
pub struct HomeFile {
    /// Relative to the home.
    pub path: PathBuf,
    pub contents: Secret<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(input: u64, output: u64, reasoning: Option<u64>) -> TokenUsage {
        TokenUsage {
            input,
            output,
            reasoning,
            ..TokenUsage::default()
        }
    }

    #[test]
    fn sums_keep_reasoning_that_any_part_reported() {
        let total: TokenUsage = [
            tokens(1, 10, None),
            tokens(2, 20, Some(5)),
            tokens(3, 30, Some(7)),
        ]
        .into_iter()
        .sum();
        assert_eq!(total, tokens(6, 60, Some(12)));
        assert_eq!(tokens(1, 1, None) + tokens(1, 1, None), tokens(2, 2, None));
    }
}
