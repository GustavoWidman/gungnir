//! Trial identity and the layout of an experiment's results directory. Paths under
//! `results/<experiment>/` are named here and nowhere else.
use crate::{
    catalog::System,
    id::{AttemptNo, LanguageId, Rep, ScenarioId},
    store::Lock,
};
use eyre::Result;
use std::{
    fmt,
    path::{Path, PathBuf},
};

/// A step that commits one directory per trial, in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Phase {
    Run,
    Grade,
    Measure,
}

impl Phase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Run => "run",
            Self::Grade => "grade",
            Self::Measure => "measure",
        }
    }
}

impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One system, scenario, language and repetition. A trial's directory is derived from its
/// key, so every phase finds the same trial without stored state.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TrialKey {
    pub system: System,
    pub scenario: ScenarioId,
    pub language: LanguageId,
    pub rep: Rep,
}

/// An experiment's results directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResultsDir(PathBuf);

impl ResultsDir {
    pub fn new(root: PathBuf) -> Self {
        Self(root)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Takes the lock on `.locks/<phase>.lock`, failing at once if another process holds it.
    pub fn lock(&self, phase: Phase) -> Result<Lock> {
        Lock::acquire(&self.0.join(".locks").join(format!("{phase}.lock")))
    }

    /// `trials/<system>/<scenario>/<language>/r<k>/`.
    pub fn trial(&self, key: &TrialKey) -> TrialDir {
        TrialDir(
            self.0
                .join("trials")
                .join(key.system.id().to_string())
                .join(key.scenario.as_str())
                .join(key.language.as_str())
                .join(key.rep.to_string()),
        )
    }
}

/// One trial's directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrialDir(PathBuf);

impl TrialDir {
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// `attempts/<n>/`, holding one launch's transcript and logs, committed or not.
    pub fn attempt(&self, attempt: AttemptNo) -> PathBuf {
        self.0.join("attempts").join(attempt.to_string())
    }

    /// `run/`, `grade/` or `measure/`, committed through [`crate::store::PartialDir`].
    pub fn phase(&self, phase: Phase) -> PathBuf {
        self.0.join(phase.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{id::Effort, testing};
    use tempfile::TempDir;

    fn key(rep: u32) -> TrialKey {
        let system = testing::catalog()
            .system(
                &testing::id("glm"),
                &testing::id("zai"),
                Effort::High,
                testing::id("2.1.316"),
            )
            .unwrap();
        TrialKey {
            system,
            scenario: testing::id("word-freq"),
            language: testing::id("zig"),
            rep: Rep::new(rep).unwrap(),
        }
    }

    #[test]
    fn trial_paths_follow_the_results_layout() {
        let results = ResultsDir::new(PathBuf::from("/runs/pilot"));
        let trial = results.trial(&key(2));
        let dir = "/runs/pilot/trials/claude-code-2.1.316_zai_glm_high/word-freq/zig/r2";
        assert_eq!(trial.path(), Path::new(dir));
        assert_eq!(
            trial.attempt(AttemptNo::FIRST.next()),
            Path::new(dir).join("attempts/2")
        );
        assert_eq!(trial.phase(Phase::Run), Path::new(dir).join("run"));
        assert_eq!(trial.phase(Phase::Grade), Path::new(dir).join("grade"));
        assert_eq!(trial.phase(Phase::Measure), Path::new(dir).join("measure"));
    }

    #[test]
    fn each_phase_has_its_own_lock() {
        let dir = TempDir::new().unwrap();
        let results = ResultsDir::new(dir.path().to_owned());
        let _run = results.lock(Phase::Run).unwrap();
        let _grade = results.lock(Phase::Grade).unwrap();
        assert!(results.lock(Phase::Run).is_err());
        assert!(dir.path().join(".locks/run.lock").is_file());
    }
}
