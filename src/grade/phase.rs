//! The grade phase: each committed run without a current grade, judged and committed with the
//! judge's evidence. It writes only grade directories, so it may run while `run` still runs.
use super::{GradeRecord, Verdict, judge};
use crate::{
    experiment::Experiment,
    lifecycle::{Current, Lifecycle, Stage},
    provenance::{Session, SessionId},
    sandbox::Backend,
    stop::{Stage as StopStage, Stop},
    store::{PartialDir, sweep_partial},
    toolchains::Toolchains,
    trial::{Phase, ResultsDir, TrialDir, TrialKey},
};
use eyre::Result;
use futures_util::{StreamExt, stream};
use std::{num::NonZeroUsize, path::Path};
use tracing::{error, info, warn};

/// What became of the trials one invocation set out to grade.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Graded {
    pub passed: usize,
    pub failed: usize,
    /// Not graded, because the sandbox could not judge the workspace.
    pub faulted: usize,
    /// Not graded, because judging failed on the host or was interrupted.
    pub errors: usize,
    /// Not started, because the invocation drained first.
    pub unstarted: usize,
}

/// How grading one trial ended.
enum Judged {
    Committed(Verdict),
    /// The sandbox could not judge the workspace, so nothing was committed.
    Fault(String),
    Unstarted,
}

/// Judges every trial of `experiment`, held in the repository `repo`, that has a committed run
/// and no grade or a stale one, `jobs` at a time, committing each grade under the trial.
pub async fn grade_runs(
    experiment: &Experiment,
    repo: &Path,
    toolchains: &Toolchains,
    backend: &dyn Backend,
    jobs: NonZeroUsize,
    stop: &Stop,
) -> Result<Graded> {
    let results = experiment.results();
    let _lock = results.lock(Phase::Grade)?;
    results.sweep_scratch(Phase::Grade)?;
    let mut used = vec![("base".to_owned(), toolchains.base())];
    for language in experiment.languages() {
        let id = language.id();
        used.push((format!("language/{id}"), toolchains.language(id.as_str())?));
    }
    let session = Session::start(repo)?.with_toolchains(used);
    session.write(results)?;

    let trials = experiment.trials();
    let mut due = Vec::new();
    for trial in &trials {
        let dir = results.trial(trial);
        sweep_partial(&dir.phase(Phase::Grade))?;
        let current = Current::of(experiment, trial, toolchains)?;
        let lifecycle = Lifecycle::read(&dir, &current)?;
        match (lifecycle.stage, lifecycle.stale) {
            (Stage::Ran { .. }, _) | (Stage::Graded { .. }, Some(Phase::Grade)) => {
                due.push((trial, dir, current));
            }
            _ => {}
        }
    }
    info!(
        "{}: {} of {} trials to grade",
        experiment.name(),
        due.len(),
        trials.len()
    );
    let session = session.id();
    let judged: Vec<(&TrialKey, Result<Judged>)> = stream::iter(due)
        .map(|(trial, dir, current)| async move {
            let judged = grade(results, &dir, &current, toolchains, backend, session, stop).await;
            (trial, judged)
        })
        .buffer_unordered(jobs.get())
        .collect()
        .await;

    let mut graded = Graded::default();
    for (trial, judged) in judged {
        match judged {
            Ok(Judged::Committed(Verdict::Pass)) => {
                graded.passed += 1;
                info!("{trial}: pass");
            }
            Ok(Judged::Committed(verdict)) => {
                graded.failed += 1;
                info!("{trial}: {verdict}");
            }
            Ok(Judged::Fault(fault)) => {
                graded.faulted += 1;
                warn!("{trial}: not graded, the sandbox could not judge it: {fault}");
            }
            Ok(Judged::Unstarted) => graded.unstarted += 1,
            Err(err) => {
                graded.errors += 1;
                error!("{trial}: not graded: {err:#}");
            }
        }
    }
    Ok(graded)
}

/// Judges the run committed in `dir` and commits its grade in place of a stale one.
async fn grade(
    results: &ResultsDir,
    dir: &TrialDir,
    current: &Current<'_>,
    toolchains: &Toolchains,
    backend: &dyn Backend,
    session: &SessionId,
    stop: &Stop,
) -> Result<Judged> {
    if stop.stage() != StopStage::Running {
        return Ok(Judged::Unstarted);
    }
    let workspace = dir.final_workspace();
    let inputs = current.grade(&workspace)?;
    let target = dir.phase(Phase::Grade);
    results.retract(Phase::Grade, &target)?;
    let partial = PartialDir::begin(&target)?;
    let (language, scenario) = (current.language(), current.scenario());
    let judgement = judge(
        language,
        toolchains,
        &workspace,
        scenario,
        backend,
        stop,
        partial.path(),
    )
    .await?;
    if let Some(fault) = judgement.fault() {
        return Ok(Judged::Fault(fault.to_owned()));
    }
    let verdict = judgement.verdict();
    partial.commit(&GradeRecord::new(session.clone(), inputs, judgement))?;
    Ok(Judged::Committed(verdict))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        grade::fake::Scripted,
        run::fixture::{self, Repo},
        store::read_record,
        testing::write,
    };
    use eyre::eyre;

    async fn grade_all(repo: &Repo, backend: &dyn Backend) -> Graded {
        let experiment = repo.experiment();
        let jobs = NonZeroUsize::new(2).unwrap();
        let stop = Stop::default();
        grade_runs(
            &experiment,
            repo.root.path(),
            &repo.toolchains,
            backend,
            jobs,
            &stop,
        )
        .await
        .unwrap()
    }

    fn grade_of(repo: &Repo, model: &str) -> Option<GradeRecord> {
        let experiment = repo.experiment();
        let trial = fixture::trial(&experiment, model);
        read_record(&experiment.results().trial(&trial).phase(Phase::Grade)).unwrap()
    }

    #[tokio::test]
    async fn committed_runs_are_graded_once_and_again_when_their_grade_goes_stale() {
        let repo = Repo::new();
        let experiment = repo.experiment();
        fixture::commit_run(
            &repo,
            &fixture::trial(&experiment, "opus"),
            &[("main.py", "print(input())\n")],
        );

        let graded = grade_all(&repo, &fixture::echo()).await;
        assert_eq!(
            graded,
            Graded {
                passed: 1,
                ..Graded::default()
            }
        );
        let grade = grade_of(&repo, "opus").unwrap();
        assert_eq!(grade.judgement().verdict(), Verdict::Pass);
        assert!(grade_of(&repo, "luna").is_none());
        assert_eq!(grade_all(&repo, &fixture::echo()).await, Graded::default());

        write(repo.root.path(), "scenarios/echo/hidden/edge.out", "x\n");
        let regraded = grade_all(&repo, &fixture::echo()).await;
        assert_eq!(
            regraded,
            Graded {
                failed: 1,
                ..Graded::default()
            }
        );
        let grade = grade_of(&repo, "opus").unwrap();
        assert_eq!(grade.judgement().verdict().to_string(), "wrong-answer");
    }

    #[tokio::test]
    async fn a_run_the_sandbox_cannot_judge_stays_ungraded_for_the_next_invocation() {
        let repo = Repo::new();
        let experiment = repo.experiment();
        for model in ["opus", "luna"] {
            let trial = fixture::trial(&experiment, model);
            fixture::commit_run(&repo, &trial, &[("main.py", "print(input())\n")]);
        }
        let broken = Scripted::new(|_, _, _| Err(eyre!("bwrap: no user namespaces").into()));

        let graded = grade_all(&repo, &broken).await;
        assert_eq!(
            graded,
            Graded {
                faulted: 2,
                ..Graded::default()
            }
        );
        assert!(grade_of(&repo, "opus").is_none());

        let graded = grade_all(&repo, &fixture::echo()).await;
        assert_eq!(graded.passed, 2);
    }
}
