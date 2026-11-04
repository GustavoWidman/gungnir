//! What judging a workspace found, and the verdict that follows from it.
use super::complexity::Complexity;
use crate::{id::CaseId, sandbox::Exit};
use eyre::{Result, bail};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt, time::Duration};

/// The build, the result of every hidden case when the build succeeded, and the complexity
/// of the sources.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Judgement {
    build: Build,
    cases: BTreeMap<CaseId, Case>,
    complexity: Option<Complexity>,
}

impl Judgement {
    pub(super) fn new(
        build: Build,
        cases: BTreeMap<CaseId, Case>,
        complexity: Option<Complexity>,
    ) -> Self {
        Self {
            build,
            cases,
            complexity,
        }
    }

    pub fn build(&self) -> &Build {
        &self.build
    }

    /// Every hidden case, or none when the build did not succeed.
    pub fn cases(&self) -> &BTreeMap<CaseId, Case> {
        &self.cases
    }

    /// `None` when the language names no lizard language.
    pub fn complexity(&self) -> Option<&Complexity> {
        self.complexity.as_ref()
    }

    /// Pass when the build succeeded and every case passed. Otherwise the failure with the
    /// most blame among the build and the cases.
    pub fn verdict(&self) -> Verdict {
        self.build
            .outcome
            .failure()
            .into_iter()
            .chain(
                self.cases
                    .values()
                    .filter_map(|case| case.outcome.failure()),
            )
            .min()
            .map_or(Verdict::Pass, Verdict::Fail)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Build {
    /// Every step that ran, sandbox setup included.
    #[serde(with = "duration")]
    pub wall: Duration,
    pub outcome: BuildOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BuildOutcome {
    /// Every step exited 0.
    Built,
    /// The step at index `step` of the language's build ended abnormally, and the later
    /// steps did not run.
    Failed { step: usize, ending: Abnormal },
    /// The sandbox could not run the step at index `step`.
    Fault { step: usize, message: String },
}

impl BuildOutcome {
    fn failure(&self) -> Option<Failure> {
        match self {
            Self::Built => None,
            Self::Failed { .. } => Some(Failure::BuildError),
            Self::Fault { .. } => Some(Failure::Environment),
        }
    }
}

/// One hidden case: how it ended and how long it took, sandbox setup included.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Case {
    #[serde(with = "duration")]
    pub wall: Duration,
    pub outcome: Outcome,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Pass,
    WrongAnswer,
    /// Exited 0 after writing more stdout than the grader keeps.
    OutputOverCap,
    /// The sandbox could not run the case. An environment error, never the program's fault.
    Fault(String),
    #[serde(untagged)]
    Abnormal(Abnormal),
}

impl Outcome {
    pub fn failure(&self) -> Option<Failure> {
        match self {
            Self::Pass => None,
            Self::WrongAnswer => Some(Failure::WrongAnswer),
            Self::OutputOverCap => Some(Failure::OutputOverCap),
            Self::Fault(_) => Some(Failure::Environment),
            Self::Abnormal(abnormal) => Some(match abnormal {
                Abnormal::TimedOut => Failure::Timeout,
                Abnormal::OutOfMemory => Failure::OutOfMemory,
                Abnormal::Crashed(_) => Failure::Crash,
            }),
        }
    }
}

/// How a program ended other than by exiting 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Abnormal {
    TimedOut,
    OutOfMemory,
    Crashed(Crash),
}

impl Abnormal {
    /// How a program that gungnir did not interrupt ended, or `None` when it exited 0. A
    /// program gungnir killed has no ending to judge, so that is an error.
    pub(crate) fn of(exit: Exit) -> Result<Option<Self>> {
        Ok(Some(match exit {
            Exit::Code(0) => return Ok(None),
            Exit::Code(code) => Self::Crashed(Crash::Code(code)),
            Exit::Signal(signal) => Self::Crashed(Crash::Signal(signal)),
            Exit::TimedOut => Self::TimedOut,
            Exit::OutOfMemory => Self::OutOfMemory,
            Exit::Killed => bail!("interrupted"),
        }))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Crash {
    Code(i32),
    Signal(i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Fail(Failure),
}

impl fmt::Display for Verdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pass => f.write_str("pass"),
            Self::Fail(failure) => f.write_str(failure.as_str()),
        }
    }
}

/// Why a workspace failed, in order of blame: when several apply, the verdict is the first.
/// The sandbox outranks the program, its build outranks its cases, and a case that did not
/// run to completion outranks one that printed the wrong answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Failure {
    Environment,
    BuildError,
    Crash,
    OutOfMemory,
    Timeout,
    OutputOverCap,
    WrongAnswer,
}

impl Failure {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Environment => "environment",
            Self::BuildError => "build-error",
            Self::Crash => "crash",
            Self::OutOfMemory => "out-of-memory",
            Self::Timeout => "timeout",
            Self::OutputOverCap => "output-over-cap",
            Self::WrongAnswer => "wrong-answer",
        }
    }
}

/// Durations as humantime text such as `"1s 250ms"`, which reads back exactly.
pub(crate) mod duration {
    use serde::{Deserialize, Deserializer, Serializer, de};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&humantime::format_duration(*duration))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
        humantime::parse_duration(&String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}
