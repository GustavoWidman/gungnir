//! Everything vendor-specific about driving an agent CLI headless: the command that launches
//! it, the events in its stdout, and the fold of those events into how the attempt ended.
//! Each driver is pure, so it is tested against captured and hand-written transcripts with no
//! process.
mod claude_code;
mod codex;
mod fold;

pub use fold::{AgentOutcome, Ending, Exit, Fold, ProcessEnd, QuotaSpan};

use crate::{
    catalog::System,
    id::{Effort, HarnessName, HarnessVersion},
    secret::{EnvValue, Secret},
};
use chrono::{DateTime, Utc};
use claude_code::ClaudeCode;
use codex::Codex;
use eyre::{Result, eyre};
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
    fn relays(&self) -> bool {
        false
    }
    fn launch(&self, req: &LaunchRequest<'_>) -> Result<Launch>;
    fn decode(&self, line: &str) -> Vec<Event>;
    fn diagnose_exit(&self, _stderr_tail: &str) -> Option<Event> {
        None
    }
    fn artifacts(&self) -> &'static [&'static str] {
        &[]
    }
    fn decode_artifact(&self, _line: &str) -> Vec<Event> {
        Vec::new()
    }
}

static DRIVERS: &[&dyn Driver] = &[&ClaudeCode, &Codex];

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

    /// Whether the CLI can launch with no credential at a plain HTTP base URL, as it does
    /// behind the relay of a backend that relays.
    pub fn relays(self) -> bool {
        self.0.relays()
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

    /// Glob patterns, relative to the isolated home, of the files the CLI leaves there that
    /// [`Harness::decode_artifact`] reads once the process is gone.
    pub fn artifacts(self) -> &'static [&'static str] {
        self.0.artifacts()
    }

    /// The events in one line of an artifact, given without its newline.
    pub fn decode_artifact(self, line: &str) -> Vec<Event> {
        self.0.decode_artifact(line)
    }

    /// Folds a stored transcript and then each stored artifact line by line, exactly as the
    /// runner folds a live attempt.
    pub fn replay(self, transcript: &str, artifacts: &[&str]) -> Fold {
        let mut fold = Fold::default();
        let events = transcript.lines().flat_map(|line| self.decode(line));
        let artifact_events = artifacts
            .iter()
            .flat_map(|artifact| artifact.lines())
            .flat_map(|line| self.decode_artifact(line));
        events
            .chain(artifact_events)
            .for_each(|event| fold.push(event));
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
    /// Every token once, in every bucket: `reasoning` is part of `output`.
    pub fn count(&self) -> u64 {
        self.input + self.cache_read + self.cache_write + self.cache_write_long + self.output
    }

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
    /// The API base URL the agent sends its model requests to, when it is not the vendor's
    /// default: the backend's endpoint, or the relay that stands in for it.
    pub endpoint: Option<&'a str>,
    /// What the agent proves its access with, `None` behind a relay, which adds the key.
    pub credential: Option<&'a CredentialMaterial>,
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

/// What a decoder names a line that is not a JSON object with a type.
const UNPARSABLE: &str = "unparsable";

fn plain(name: &str, value: impl Into<String>) -> (String, EnvValue) {
    (name.to_owned(), EnvValue::Plain(value.into()))
}

/// A guest path as text, for argv and the environment.
fn utf8(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| eyre!("guest path {} is not UTF-8", path.display()))
}

/// Whether `text` contains any of the lowercase `phrases`, ignoring case.
fn mentions(text: &str, phrases: &[&str]) -> bool {
    let text = text.to_lowercase();
    phrases.iter().any(|phrase| text.contains(phrase))
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

    #[test]
    fn every_driver_is_found_by_its_slug_name() {
        for harness in Harness::all() {
            let name: HarnessName = harness.name().parse().unwrap();
            assert_eq!(
                Harness::named(&name).map(Harness::name),
                Some(harness.name())
            );
        }
        assert!(Harness::named(&"gemini-cli".parse().unwrap()).is_none());
    }

    #[test]
    fn every_artifact_pattern_is_a_glob() {
        for harness in Harness::all() {
            for pattern in harness.artifacts() {
                assert!(globset::Glob::new(pattern).is_ok(), "{pattern}");
            }
        }
    }

    #[test]
    fn phrases_match_regardless_of_case() {
        assert!(mentions("You've hit your Usage Limit.", &["usage limit"]));
        assert!(!mentions("context window exceeded", &["usage limit"]));
    }
}
