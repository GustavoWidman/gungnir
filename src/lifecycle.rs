//! A trial's lifecycle, derived from its committed records and never stored. Every record
//! embeds the inputs it was made from, so comparing them with what its phase would use now
//! tells a stale record.
use crate::{
    experiment::Experiment,
    grade::{GradeInputs, GradeRecord},
    language::Language,
    run::{RunInputs, RunRecord, Task},
    scenario::Scenario,
    store::read_record,
    toolchains::Toolchains,
    trial::{Phase, TrialDir, TrialKey},
};
use eyre::{Result, eyre};
use std::path::Path;

/// How far a trial has got: its committed records, in phase order.
#[derive(Clone, Debug, PartialEq)]
pub enum Stage {
    /// No run is committed. Whatever a later phase left behind does not count.
    Pending,
    Ran {
        run: Box<RunRecord>,
    },
    Graded {
        run: Box<RunRecord>,
        grade: Box<GradeRecord>,
    },
}

/// A trial's stage, and the first phase whose committed record was made from inputs other than
/// the ones that phase would use now.
#[derive(Clone, Debug, PartialEq)]
pub struct Lifecycle {
    pub stage: Stage,
    pub stale: Option<Phase>,
}

impl Lifecycle {
    /// Reads the committed records of the trial in `dir` in phase order, comparing each one's
    /// inputs with `current`.
    pub fn read(dir: &TrialDir, current: &Current<'_>) -> Result<Self> {
        let Some(run) = read_record::<RunRecord>(&dir.phase(Phase::Run))? else {
            return Ok(Self {
                stage: Stage::Pending,
                stale: None,
            });
        };
        let mut stale = (run.inputs() != current.run()).then_some(Phase::Run);
        let Some(grade) = read_record::<GradeRecord>(&dir.phase(Phase::Grade))? else {
            return Ok(Self {
                stage: Stage::Ran { run: run.into() },
                stale,
            });
        };
        if stale.is_none() && grade.inputs() != &current.grade(&dir.final_workspace())? {
            stale = Some(Phase::Grade);
        }
        Ok(Self {
            stage: Stage::Graded {
                run: run.into(),
                grade: grade.into(),
            },
            stale,
        })
    }

    /// The committed run, once there is one.
    pub fn run(&self) -> Option<&RunRecord> {
        match &self.stage {
            Stage::Pending => None,
            Stage::Ran { run } | Stage::Graded { run, .. } => Some(run),
        }
    }
}

/// What each phase would use for one trial now.
pub struct Current<'a> {
    run: RunInputs,
    language: &'a Language,
    scenario: &'a Scenario,
    toolchains: &'a Toolchains,
}

impl<'a> Current<'a> {
    pub fn of(
        experiment: &'a Experiment,
        trial: &'a TrialKey,
        toolchains: &'a Toolchains,
    ) -> Result<Self> {
        let task = Task::of(experiment, trial)?;
        let scenario = experiment
            .scenarios()
            .iter()
            .find(|scenario| scenario.id() == &trial.scenario)
            .ok_or_else(|| eyre!("experiment has no scenario {}", trial.scenario))?;
        Ok(Self {
            run: RunInputs::current(&task, toolchains, experiment.budget())?,
            language: task.language,
            scenario,
            toolchains,
        })
    }

    /// The inputs of running the trial now.
    pub fn run(&self) -> &RunInputs {
        &self.run
    }

    /// The inputs of grading the committed workspace `workspace` now.
    pub fn grade(&self, workspace: &Path) -> Result<GradeInputs> {
        GradeInputs::new(self.language, self.toolchains, self.scenario, workspace)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        run::fixture::{self, Repo},
        testing::write,
    };
    use std::fs;

    fn read(repo: &Repo, model: &str) -> Lifecycle {
        let experiment = repo.experiment();
        let trial = fixture::trial(&experiment, model);
        let current = Current::of(&experiment, &trial, &repo.toolchains).unwrap();
        Lifecycle::read(&experiment.results().trial(&trial), &current).unwrap()
    }

    fn stage(lifecycle: &Lifecycle) -> &'static str {
        match lifecycle.stage {
            Stage::Pending => "pending",
            Stage::Ran { .. } => "ran",
            Stage::Graded { .. } => "graded",
        }
    }

    #[tokio::test]
    async fn a_trial_advances_as_its_phases_commit_and_never_past_a_missing_one() {
        let repo = Repo::new();
        let experiment = repo.experiment();
        let trial = fixture::trial(&experiment, "opus");
        assert_eq!(stage(&read(&repo, "opus")), "pending");

        fixture::commit_run(&repo, &trial, &[("main.py", "print(input())\n")]);
        let ran = read(&repo, "opus");
        assert_eq!((stage(&ran), ran.stale), ("ran", None));
        assert!(ran.run().is_some());

        fixture::commit_grade(&repo, &trial).await;
        let graded = read(&repo, "opus");
        assert_eq!((stage(&graded), graded.stale), ("graded", None));
        assert_eq!(stage(&read(&repo, "luna")), "pending");

        let dir = experiment.results().trial(&trial);
        fs::remove_dir_all(dir.phase(Phase::Run)).unwrap();
        assert_eq!(
            read(&repo, "opus"),
            Lifecycle {
                stage: Stage::Pending,
                stale: None
            }
        );
    }

    #[tokio::test]
    async fn stale_names_the_first_phase_whose_inputs_changed() {
        let repo = Repo::new();
        let experiment = repo.experiment();
        let trial = fixture::trial(&experiment, "opus");
        fixture::commit_run(&repo, &trial, &[("main.py", "print(input())\n")]);
        fixture::commit_grade(&repo, &trial).await;
        let root = repo.root.path();

        write(root, "scenarios/echo/hidden/edge.out", "x\n");
        assert_eq!(read(&repo, "opus").stale, Some(Phase::Grade));

        write(root, "scenarios/echo/spec.md", "Echo every line.\n");
        assert_eq!(read(&repo, "opus").stale, Some(Phase::Run));

        fs::remove_dir_all(experiment.results().trial(&trial).phase(Phase::Grade)).unwrap();
        let ran = read(&repo, "opus");
        assert_eq!((stage(&ran), ran.stale), ("ran", Some(Phase::Run)));
    }
}
