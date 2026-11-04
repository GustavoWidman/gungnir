//! A trial's lifecycle, derived from its committed records and never stored. Every record
//! embeds the inputs it was made from, so comparing them with what its phase would use now
//! tells a stale record.
use crate::{
    experiment::Experiment,
    grade::{GradeInputs, GradeRecord, Verdict},
    language::Language,
    measure::{Identity, MeasureInputs, MeasureRecord, Protocol},
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
    /// Graded. A failing grade is final, and a passing one waits to be measured.
    Graded {
        run: Box<RunRecord>,
        grade: Box<GradeRecord>,
    },
    /// A passing grade, timed against the reference.
    Measured {
        run: Box<RunRecord>,
        grade: Box<GradeRecord>,
        measure: Box<MeasureRecord>,
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
        let mut stale = run.inputs().is_stale(current.run()).then_some(Phase::Run);
        let Some(grade) = read_record::<GradeRecord>(&dir.phase(Phase::Grade))? else {
            return Ok(Self {
                stage: Stage::Ran { run: run.into() },
                stale,
            });
        };
        let workspace = dir.final_workspace();
        if stale.is_none() && grade.inputs() != &current.grade(&workspace)? {
            stale = Some(Phase::Grade);
        }
        let measure = match grade.judgement().verdict() {
            Verdict::Pass => read_record::<MeasureRecord>(&dir.phase(Phase::Measure))?,
            Verdict::Fail(_) => None,
        };
        let Some(measure) = measure else {
            return Ok(Self {
                stage: Stage::Graded {
                    run: run.into(),
                    grade: grade.into(),
                },
                stale,
            });
        };
        if stale.is_none()
            && (measure.inputs() != &current.measure(&workspace)?
                || !measure.machine().is(current.machine))
        {
            stale = Some(Phase::Measure);
        }
        Ok(Self {
            stage: Stage::Measured {
                run: run.into(),
                grade: grade.into(),
                measure: measure.into(),
            },
            stale,
        })
    }

    /// The committed run, once there is one.
    pub fn run(&self) -> Option<&RunRecord> {
        match &self.stage {
            Stage::Pending => None,
            Stage::Ran { run } | Stage::Graded { run, .. } | Stage::Measured { run, .. } => {
                Some(run)
            }
        }
    }
}

/// What each phase would use for one trial now. A measurement is current only when made on the
/// machine the experiment pins, or on this machine when it pins none, and under
/// [`Protocol::DEFAULT`], unless [`Current::with_protocol`] names another protocol.
pub struct Current<'a> {
    run: RunInputs,
    language: &'a Language,
    scenario: &'a Scenario,
    toolchains: &'a Toolchains,
    protocol: Protocol,
    machine: &'a Identity,
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
            protocol: Protocol::DEFAULT,
            machine: match experiment.machine() {
                Some(pinned) => pinned,
                None => Identity::here()?,
            },
        })
    }

    /// The same, with measurements current only when made under `protocol`.
    pub fn with_protocol(self, protocol: Protocol) -> Self {
        Self { protocol, ..self }
    }

    /// The inputs of running the trial now.
    pub fn run(&self) -> &RunInputs {
        &self.run
    }

    pub fn language(&self) -> &'a Language {
        self.language
    }

    pub fn scenario(&self) -> &'a Scenario {
        self.scenario
    }

    /// The inputs of grading the committed workspace `workspace` now.
    pub fn grade(&self, workspace: &Path) -> Result<GradeInputs> {
        GradeInputs::new(self.language, self.toolchains, self.scenario, workspace)
    }

    /// The inputs of measuring the committed workspace `workspace` now.
    pub fn measure(&self, workspace: &Path) -> Result<MeasureInputs> {
        MeasureInputs::new(
            self.language,
            self.toolchains,
            self.scenario,
            workspace,
            &self.protocol,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        run::fixture::{self, Repo},
        store::PartialDir,
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
            Stage::Measured { .. } => "measured",
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

    /// Commits a measurement of `trial` that timed nothing, made from the inputs of measuring
    /// it now under `protocol`, on this machine.
    fn commit_measure(repo: &Repo, trial: &TrialKey, protocol: Protocol) {
        let here = Identity::here().unwrap();
        commit_measure_on(
            repo,
            trial,
            protocol,
            here.cpu.as_deref(),
            here.host.as_deref(),
        );
    }

    /// The same, on the machine with the CPU model `cpu` and the host id `host`.
    fn commit_measure_on(
        repo: &Repo,
        trial: &TrialKey,
        protocol: Protocol,
        cpu: Option<&str>,
        host: Option<&str>,
    ) {
        let experiment = repo.experiment();
        let current = Current::of(&experiment, trial, &repo.toolchains)
            .unwrap()
            .with_protocol(protocol);
        let dir = experiment.results().trial(trial);
        let record = serde_json::json!({
            "session": fixture::session(),
            "inputs": current.measure(&dir.final_workspace()).unwrap(),
            "machine": {
                "cpu": cpu, "host": host, "cpus": 1, "core": 0, "governor": null, "kernel": "7.3.0",
                "perf_event_paranoid": 1, "load": {"start": [0.0, 0.0, 0.0], "end": [0.0, 0.0, 0.0]},
            },
            "workloads": {},
        });
        let partial = PartialDir::begin(&dir.phase(Phase::Measure)).unwrap();
        partial.commit(&record).unwrap();
    }

    #[tokio::test]
    async fn a_measurement_counts_under_a_passing_grade_made_with_the_current_protocol() {
        let repo = Repo::new();
        let experiment = repo.experiment();
        let trial = fixture::trial(&experiment, "opus");
        fixture::commit_run(&repo, &trial, &[("main.py", "print(input())\n")]);
        fixture::commit_grade(&repo, &trial).await;
        commit_measure(&repo, &trial, Protocol::DEFAULT);
        let measured = read(&repo, "opus");
        assert_eq!((stage(&measured), measured.stale), ("measured", None));
        assert!(measured.run().is_some());

        let longer = Protocol {
            warmup: Protocol::DEFAULT.warmup + 1,
            ..Protocol::DEFAULT
        };
        let current = Current::of(&experiment, &trial, &repo.toolchains)
            .unwrap()
            .with_protocol(longer);
        let dir = experiment.results().trial(&trial);
        let remeasure = Lifecycle::read(&dir, &current).unwrap();
        assert_eq!(remeasure.stale, Some(Phase::Measure));

        write(repo.root.path(), "scenarios/echo/hidden/edge.out", "x\n");
        let regrade = read(&repo, "opus");
        assert_eq!(
            (stage(&regrade), regrade.stale),
            ("measured", Some(Phase::Grade))
        );
        fs::remove_dir_all(dir.phase(Phase::Grade)).unwrap();
        fixture::commit_grade(&repo, &trial).await;
        let failed = read(&repo, "opus");
        assert_eq!((stage(&failed), failed.stale), ("graded", None));
    }

    #[tokio::test]
    async fn a_measurement_made_on_another_machine_is_stale() {
        let repo = Repo::new();
        let experiment = repo.experiment();
        let trial = fixture::trial(&experiment, "opus");
        fixture::commit_run(&repo, &trial, &[("main.py", "print(input())\n")]);
        fixture::commit_grade(&repo, &trial).await;
        let dir = experiment.results().trial(&trial);
        let here = Identity::here().unwrap();
        let (cpu, host) = (here.cpu.as_deref(), here.host.as_deref());
        let measured_on = |cpu: Option<&str>, host: Option<&str>| {
            fs::remove_dir_all(dir.phase(Phase::Measure)).ok();
            commit_measure_on(&repo, &trial, Protocol::DEFAULT, cpu, host);
            let lifecycle = read(&repo, "opus");
            (stage(&lifecycle), lifecycle.stale)
        };

        assert_eq!(measured_on(cpu, host), ("measured", None));
        assert_eq!(
            measured_on(Some("Another CPU"), host),
            ("measured", Some(Phase::Measure))
        );
        assert_eq!(
            measured_on(cpu, Some("another-host")),
            ("measured", Some(Phase::Measure))
        );
        assert_eq!(measured_on(cpu, None), ("measured", None));
    }

    #[tokio::test]
    async fn a_pinned_machine_decides_wherever_the_lifecycle_is_read() {
        let repo = Repo::new();
        let file = repo.root.path().join("experiments/fixture.toml");
        let text = fs::read_to_string(&file).unwrap();
        let pin = "[machine]\ncpu = \"13th Gen Intel(R) Core(TM) i9-13900K\"\nhost = \"d35k70p\"\n\n[pins]";
        fs::write(&file, text.replacen("[pins]", pin, 1)).unwrap();
        let experiment = repo.experiment();
        let trial = fixture::trial(&experiment, "opus");
        fixture::commit_run(&repo, &trial, &[("main.py", "print(input())\n")]);
        fixture::commit_grade(&repo, &trial).await;
        let dir = experiment.results().trial(&trial);
        let measured_on = |cpu: Option<&str>, host: Option<&str>| {
            fs::remove_dir_all(dir.phase(Phase::Measure)).ok();
            commit_measure_on(&repo, &trial, Protocol::DEFAULT, cpu, host);
            let lifecycle = read(&repo, "opus");
            (stage(&lifecycle), lifecycle.stale)
        };

        let desktop = (
            Some("13th Gen Intel(R) Core(TM) i9-13900K"),
            Some("d35k70p"),
        );
        assert_eq!(measured_on(desktop.0, desktop.1), ("measured", None));
        assert_eq!(
            measured_on(Some("Another CPU"), Some("another-host")),
            ("measured", Some(Phase::Measure))
        );
        assert_eq!(
            measured_on(desktop.0, Some("another-host")),
            ("measured", Some(Phase::Measure))
        );
    }
}
