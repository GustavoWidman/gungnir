//! The grade phase: each committed run without a current grade, judged and committed with the
//! judge's evidence, and each current grade whose complexity was counted otherwise than it
//! would be now, recounted in place. It writes only grade directories, so it may run while
//! `run` still runs.
use super::{Complexity, Counting, GradeRecord, Verdict, judge};
use crate::{
    experiment::Experiment,
    lifecycle::{Current, Lifecycle, Stage},
    provenance::{Session, SessionId},
    sandbox::Backend,
    stop::{Stage as StopStage, Stop},
    store::{PartialDir, rewrite_record, sweep_partial},
    toolchains::Toolchains,
    trial::{Phase, ResultsDir, TrialDir, TrialKey},
};
use eyre::{Result, WrapErr};
use futures_util::{StreamExt, stream};
use std::{num::NonZeroUsize, path::Path};
use tracing::{error, info, warn};

/// What became of the trials one invocation set out to grade or recount.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Graded {
    pub passed: usize,
    pub failed: usize,
    /// Current grades whose complexity was measured again and rewritten in place.
    pub recounted: usize,
    /// Not graded, because the sandbox could not judge the workspace.
    pub faulted: usize,
    /// Not graded or recounted, because the work failed on the host or was interrupted.
    pub errors: usize,
    /// Not started, because the invocation drained first.
    pub unstarted: usize,
}

/// What a trial needs from the grade phase.
enum Due {
    /// A grade, for a committed run without one or with a stale one.
    Grade,
    /// Its current grade's complexity measured again.
    Recount(Box<GradeRecord>),
}

/// How grading or recounting one trial ended.
enum Judged {
    Committed(Verdict),
    /// The complexity was rewritten, its cyclomatic complexity summing to `before` and then to
    /// `after`.
    Recounted {
        before: Option<u64>,
        after: Option<u64>,
    },
    /// The sandbox could not judge the workspace, so nothing was committed.
    Fault(String),
    Unstarted,
}

/// Judges every trial of `experiment`, held in the repository `repo`, that has a committed run
/// and no grade or a stale one, `jobs` at a time, committing each grade under the trial. A
/// current grade whose complexity was counted otherwise than measuring would count it now has
/// its complexity measured again from the same workspace and rewritten in place, without
/// building or running anything.
pub async fn grade_runs(
    experiment: &Experiment,
    repo: &Path,
    toolchains: &Toolchains,
    backend: &dyn Backend,
    jobs: NonZeroUsize,
    stop: &Stop,
) -> Result<Graded> {
    let counting = Counting::probe(experiment.languages()).await?;
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
        let needs = match (&lifecycle.stage, lifecycle.stale) {
            (Stage::Ran { .. }, _)
            | (Stage::Graded { .. } | Stage::Measured { .. }, Some(Phase::Grade)) => Due::Grade,
            _ => match lifecycle.current_grade() {
                Some(grade)
                    if !counting.is_current(current.language(), grade.judgement().complexity()) =>
                {
                    Due::Recount(Box::new(grade.clone()))
                }
                _ => continue,
            },
        };
        due.push((trial, dir, current, needs));
    }
    let recounts = due
        .iter()
        .filter(|(.., needs)| matches!(needs, Due::Recount(_)))
        .count();
    info!(
        "{}: {} of {} trials to grade, {recounts} to recount the complexity of",
        experiment.name(),
        due.len() - recounts,
        trials.len()
    );
    let session = session.id();
    let mut judged = stream::iter(due)
        .map(|(trial, dir, current, needs)| async move {
            let judged = match needs {
                Due::Grade => grade(results, &dir, &current, toolchains, backend, session, stop)
                    .await
                    .wrap_err("not graded"),
                Due::Recount(record) => recount(&dir, &current, *record, stop)
                    .await
                    .wrap_err("complexity not recounted"),
            };
            (trial, judged)
        })
        .buffer_unordered(jobs.get());
    let mut graded = Graded::default();
    while let Some((trial, judged)) = judged.next().await {
        settle(&mut graded, trial, judged);
    }
    Ok(graded)
}

/// Counts and logs how grading or recounting `trial` ended.
fn settle(graded: &mut Graded, trial: &TrialKey, judged: Result<Judged>) {
    match judged {
        Ok(Judged::Committed(Verdict::Pass)) => {
            graded.passed += 1;
            info!("{trial}: pass");
        }
        Ok(Judged::Committed(verdict)) => {
            graded.failed += 1;
            info!("{trial}: {verdict}");
        }
        Ok(Judged::Recounted { before, after }) => {
            graded.recounted += 1;
            let sum = |ccn: Option<u64>| ccn.map_or_else(|| "none".to_owned(), |n| n.to_string());
            info!(
                "{trial}: complexity recounted, total CCN {} to {}",
                sum(before),
                sum(after)
            );
        }
        Ok(Judged::Fault(fault)) => {
            graded.faulted += 1;
            warn!("{trial}: not graded, the sandbox could not judge it: {fault}");
        }
        Ok(Judged::Unstarted) => graded.unstarted += 1,
        Err(err) => {
            graded.errors += 1;
            error!("{trial}: {err:#}");
        }
    }
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

/// Measures again the complexity of the workspace that `grade`, the current grade committed in
/// `dir`, judged, and rewrites the grade in place with it. The verdict, the cases and the
/// grading session stay as they are.
async fn recount(
    dir: &TrialDir,
    current: &Current<'_>,
    grade: GradeRecord,
    stop: &Stop,
) -> Result<Judged> {
    if stop.stage() != StopStage::Running {
        return Ok(Judged::Unstarted);
    }
    let complexity = super::recount(current.language(), &dir.final_workspace()).await?;
    let before = ccn(grade.judgement().complexity());
    let after = ccn(complexity.as_ref());
    rewrite_record(&dir.phase(Phase::Grade), &grade.recounted(complexity))?;
    Ok(Judged::Recounted { before, after })
}

/// The sum of every function's cyclomatic complexity.
fn ccn(complexity: Option<&Complexity>) -> Option<u64> {
    complexity.map(|complexity| {
        complexity
            .files
            .values()
            .flat_map(|file| &file.functions)
            .map(|function| u64::from(function.ccn))
            .sum()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        grade::fake::Scripted,
        measure::Protocol,
        run::fixture::{self, Repo},
        store::read_record,
        testing::{self, write},
    };
    use eyre::eyre;
    use serde_json::{Value, json};

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

    #[tokio::test]
    async fn a_current_grade_counted_otherwise_is_recounted_in_place_without_running_anything() {
        if std::process::Command::new("lizard")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("skipped: lizard is not on PATH");
            return;
        }
        let repo = Repo::new();
        let experiment = repo.experiment();
        let opus = fixture::trial(&experiment, "opus");
        let program = "for line in open(0):\n    if line:\n        print(line, end='')\n";
        fixture::commit_run(&repo, &opus, &[("main.py", program)]);
        fixture::commit_grade(&repo, &opus).await;
        fixture::commit_measure(&repo, &opus, Protocol::DEFAULT);
        write(
            repo.root.path(),
            "languages/python/language.toml",
            testing::LANGUAGE_TOML,
        );
        let dir = experiment.results().trial(&opus).phase(Phase::Grade);
        let mut stored: Value = read_record(&dir).unwrap().unwrap();
        stored["judgement"]["complexity"] =
            json!({ "files": { "main.py": { "nloc": 3, "functions": [] } } });
        write(&dir, "record.json", stored.to_string());
        let before = grade_of(&repo, "opus").unwrap();

        let untouched = Scripted::new(|_, argv, _| panic!("ran {argv:?}"));
        let graded = grade_all(&repo, &untouched).await;
        assert_eq!(
            graded,
            Graded {
                recounted: 1,
                ..Graded::default()
            }
        );
        assert!(untouched.calls().is_empty());
        let after = grade_of(&repo, "opus").unwrap();
        assert_eq!(after.session(), before.session());
        assert_eq!(after.inputs(), before.inputs());
        assert_eq!(after.judgement().build(), before.judgement().build());
        assert_eq!(after.judgement().cases(), before.judgement().cases());
        assert!(dir.join("build.log").is_file());
        let complexity = after.judgement().complexity().unwrap();
        assert_eq!(complexity.version(), 2);
        let ccn: Vec<(&str, u32)> = complexity.files["main.py"]
            .functions
            .iter()
            .map(|function| (function.name.as_str(), function.ccn))
            .collect();
        assert_eq!(ccn, [("*global*", 3)]);

        let current = Current::of(&experiment, &opus, &repo.toolchains).unwrap();
        let lifecycle = Lifecycle::read(&experiment.results().trial(&opus), &current).unwrap();
        assert!(matches!(lifecycle.stage, Stage::Measured { .. }));
        assert_eq!(lifecycle.stale, None);
        assert_eq!(grade_all(&repo, &untouched).await, Graded::default());
    }
}
