//! Trial identity and the layout of an experiment's results directory. Paths under
//! `results/<experiment>/` are named here and nowhere else.
use crate::{
    catalog::System,
    id::{AttemptNo, LanguageId, Rep, ScenarioId},
    provenance::SessionId,
    store::Lock,
};
use eyre::{Result, WrapErr};
use std::{
    fmt, fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

/// The agent's prompt, inside a run directory.
pub const PROMPT: &str = "prompt.md";
/// The agent's workspace as it left it, inside a run directory.
pub const FINAL: &str = "final";

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

/// `<system>/<scenario>/<language>/r<k>`, the trial's directory under `trials/`.
impl fmt::Display for TrialKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self {
            system,
            scenario,
            language,
            rep,
        } = self;
        write!(f, "{}/{scenario}/{language}/{rep}", system.id())
    }
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

    /// `experiment.json`: the experiment's name and the session that first ran it.
    pub fn manifest(&self) -> PathBuf {
        self.0.join("experiment.json")
    }

    /// `sessions/<id>.json`.
    pub fn session(&self, id: &SessionId) -> PathBuf {
        self.0.join("sessions").join(format!("{id}.json"))
    }

    /// `references/<scenario>/<language>/`, the reference solution's own grade, committed
    /// through [`crate::store::PartialDir`].
    pub fn reference(&self, scenario: &ScenarioId, language: &LanguageId) -> PathBuf {
        self.0
            .join("references")
            .join(scenario.as_str())
            .join(language.as_str())
    }

    /// `.scratch/<phase>/`, where one invocation of `phase` keeps what it does not commit.
    pub fn scratch(&self, phase: Phase) -> PathBuf {
        self.0.join(".scratch").join(phase.as_str())
    }

    /// Empties `phase`'s scratch of whatever an interrupted invocation left there. Call it
    /// holding the phase's lock.
    pub fn sweep_scratch(&self, phase: Phase) -> Result<()> {
        let scratch = self.scratch(phase);
        match fs::remove_dir_all(&scratch) {
            Err(err) if err.kind() != ErrorKind::NotFound => {
                Err(err).wrap_err_with(|| format!("removing {}", scratch.display()))
            }
            _ => fs::create_dir_all(&scratch)
                .wrap_err_with(|| format!("creating {}", scratch.display())),
        }
    }

    /// Withdraws the committed directory `dir` by renaming it into `phase`'s scratch and
    /// deleting it there. A crash leaves it either committed or in scratch, never half
    /// deleted at its committed path. A missing `dir` is already withdrawn.
    pub fn retract(&self, phase: Phase, dir: &Path) -> Result<()> {
        let scratch = self.scratch(phase);
        fs::create_dir_all(&scratch).wrap_err_with(|| format!("creating {}", scratch.display()))?;
        let holder = tempfile::Builder::new()
            .prefix("retracted-")
            .tempdir_in(&scratch)
            .wrap_err_with(|| format!("creating a directory in {}", scratch.display()))?;
        match fs::rename(dir, holder.path().join("dir")) {
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            renamed => renamed.wrap_err_with(|| format!("retracting {}", dir.display()))?,
        }
        let removed = holder.path().to_owned();
        holder
            .close()
            .wrap_err_with(|| format!("removing {}", removed.display()))
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
    pub fn attempt(&self, number: AttemptNo) -> AttemptDir {
        AttemptDir {
            number,
            path: self.0.join("attempts").join(number.to_string()),
        }
    }

    /// Every attempt made so far, in order. Attempts are numbered from one without gaps, so
    /// the first missing number is the next attempt's.
    pub fn attempts(&self) -> Vec<AttemptDir> {
        std::iter::successors(Some(self.attempt(AttemptNo::FIRST)), |attempt| {
            Some(self.attempt(attempt.number.next()))
        })
        .take_while(|attempt| attempt.path.is_dir())
        .collect()
    }

    /// `run/`, `grade/` or `measure/`, committed through [`crate::store::PartialDir`].
    pub fn phase(&self, phase: Phase) -> PathBuf {
        self.0.join(phase.as_str())
    }

    /// `run/final/`, the workspace of the committed run.
    pub fn final_workspace(&self) -> PathBuf {
        self.phase(Phase::Run).join(FINAL)
    }
}

/// One attempt's directory, `attempts/<n>/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttemptDir {
    number: AttemptNo,
    path: PathBuf,
}

impl AttemptDir {
    pub fn number(&self) -> AttemptNo {
        self.number
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The agent's stdout, line by line as it arrived, secrets redacted.
    pub fn transcript(&self) -> PathBuf {
        self.path.join("transcript.jsonl")
    }

    /// The agent's stderr, secrets redacted.
    pub fn stderr(&self) -> PathBuf {
        self.path.join("stderr.log")
    }

    /// Every connection the agent attempted.
    pub fn egress(&self) -> PathBuf {
        self.path.join("egress.jsonl")
    }

    /// Files the CLI left in its isolated home that its driver reads, such as Codex's session
    /// rollout, under their paths relative to that home, secrets redacted.
    pub fn artifacts(&self) -> PathBuf {
        self.path.join("artifacts")
    }

    /// Present only for an attempt a rate limit ended.
    pub fn discarded(&self) -> PathBuf {
        self.path.join("discarded.json")
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
            key(2).to_string(),
            dir.strip_prefix("/runs/pilot/trials/").unwrap()
        );
        let attempt = trial.attempt(AttemptNo::FIRST.next());
        assert_eq!(attempt.path(), Path::new(dir).join("attempts/2"));
        assert_eq!(
            attempt.transcript(),
            Path::new(dir).join("attempts/2/transcript.jsonl")
        );
        assert_eq!(trial.phase(Phase::Run), Path::new(dir).join("run"));
        assert_eq!(trial.phase(Phase::Grade), Path::new(dir).join("grade"));
        assert_eq!(trial.phase(Phase::Measure), Path::new(dir).join("measure"));
        assert_eq!(trial.final_workspace(), Path::new(dir).join("run/final"));
        assert_eq!(
            results.reference(&testing::id("word-freq"), &testing::id("zig")),
            Path::new("/runs/pilot/references/word-freq/zig")
        );
    }

    #[test]
    fn attempts_are_listed_up_to_the_first_missing_number() {
        let dir = TempDir::new().unwrap();
        let trial = ResultsDir::new(dir.path().to_owned()).trial(&key(1));
        assert!(trial.attempts().is_empty());

        for attempt in ["1", "2", "4"] {
            fs::create_dir_all(trial.path().join("attempts").join(attempt)).unwrap();
        }
        let numbers: Vec<String> = trial
            .attempts()
            .iter()
            .map(|attempt| attempt.number().to_string())
            .collect();
        assert_eq!(numbers, ["1", "2"]);
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

    #[test]
    fn sweeping_scratch_leaves_it_empty_and_spares_other_phases() {
        let dir = TempDir::new().unwrap();
        let results = ResultsDir::new(dir.path().to_owned());
        testing::write(&results.scratch(Phase::Run), "work/main.py", "");
        testing::write(&results.scratch(Phase::Grade), "kept", "");

        results.sweep_scratch(Phase::Run).unwrap();
        results.sweep_scratch(Phase::Run).unwrap();
        assert_eq!(
            fs::read_dir(results.scratch(Phase::Run)).unwrap().count(),
            0
        );
        assert!(results.scratch(Phase::Grade).join("kept").is_file());
    }

    #[test]
    fn a_retracted_directory_is_gone_and_retracting_again_is_harmless() {
        let dir = TempDir::new().unwrap();
        let results = ResultsDir::new(dir.path().to_owned());
        let run = results.trial(&key(1)).phase(Phase::Run);
        testing::write(&run, "final/main.py", "print()");

        results.retract(Phase::Run, &run).unwrap();
        assert!(!run.exists());
        results.retract(Phase::Run, &run).unwrap();
        assert_eq!(
            fs::read_dir(results.scratch(Phase::Run)).unwrap().count(),
            0
        );
    }
}
