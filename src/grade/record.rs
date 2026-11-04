//! A committed grade and the inputs it was made from, so a later phase can tell a stale grade
//! by comparing the stored inputs with the current ones.
use super::judgement::{Judgement, duration};
use crate::{
    language::Language,
    provenance::SessionId,
    scenario::{Compare, Scenario},
    store::Digest,
    toolchains::Toolchains,
};
use eyre::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

/// Raised whenever grading would judge the same inputs differently, which makes every stored
/// grade stale.
const GRADER_VERSION: u32 = 1;

/// `grade/record.json`. The grade directory also holds the `build.log` and `cases/<case>.out`
/// that [`super::judge`] writes as evidence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradeRecord {
    session: SessionId,
    inputs: GradeInputs,
    judgement: Judgement,
}

impl GradeRecord {
    pub fn new(session: SessionId, inputs: GradeInputs, judgement: Judgement) -> Self {
        Self {
            session,
            inputs,
            judgement,
        }
    }

    /// The invocation that graded.
    pub fn session(&self) -> &SessionId {
        &self.session
    }

    pub fn inputs(&self) -> &GradeInputs {
        &self.inputs
    }

    pub fn judgement(&self) -> &Judgement {
        &self.judgement
    }
}

/// Everything a grade depends on. Equal inputs grade the same, so a stored grade is stale
/// exactly when its inputs differ from the current ones.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradeInputs {
    grader: u32,
    /// The language environment's store path, which names its whole closure.
    toolchain: PathBuf,
    build: Vec<Vec<String>>,
    run: Vec<String>,
    env: BTreeMap<String, String>,
    hidden: Digest,
    compare: StoredCompare,
    limits: StoredLimits,
    /// The workspace without the language's build output.
    workspace: Digest,
}

impl GradeInputs {
    /// The inputs of grading `workspace` as a `language` solution to `scenario` now.
    pub fn new(
        language: &Language,
        toolchains: &Toolchains,
        scenario: &Scenario,
        workspace: &Path,
    ) -> Result<Self> {
        let limits = scenario.limits();
        Ok(Self {
            grader: GRADER_VERSION,
            toolchain: toolchains
                .language(language.id().as_str())?
                .root()
                .to_owned(),
            build: language
                .build()
                .iter()
                .map(|step| step.args().to_vec())
                .collect(),
            run: language.run().args().to_vec(),
            env: language.env().clone(),
            hidden: scenario.hidden_digest(),
            compare: scenario.compare().into(),
            limits: StoredLimits {
                time: limits.time(),
                memory_bytes: limits.memory_bytes(),
            },
            workspace: Digest::of_tree(workspace, language.ignore())?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum StoredCompare {
    Exact,
    Whitespace,
    Numeric { abs: f64, rel: f64 },
}

impl From<Compare> for StoredCompare {
    fn from(compare: Compare) -> Self {
        match compare {
            Compare::Exact => Self::Exact,
            Compare::Whitespace => Self::Whitespace,
            Compare::Numeric(tolerance) => Self::Numeric {
                abs: tolerance.abs,
                rel: tolerance.rel,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct StoredLimits {
    #[serde(with = "duration")]
    time: Duration,
    memory_bytes: u64,
}
