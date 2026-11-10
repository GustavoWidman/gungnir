//! Reviewing a committed run before anything is scheduled: its outcome re-derived from the
//! stored transcript once its harness's parser has changed, without calling a model, and the
//! run withdrawn when it is no sample, whether the newer parser or the sample policy says so.
//! A withdrawn run's trial is pending again. Reviewing only reads; applying a review writes.
use super::{
    Reason, RunRecord, Sample,
    attempt::conclude,
    records::{Counted, DiscardedAttempt},
};
use crate::{
    harness::Harness,
    provenance::SessionId,
    store::{rewrite_record, write_json},
    trial::{Phase, ResultsDir, TrialDir},
};
use eyre::Result;

/// What a committed run needs before its trial counts as done.
#[derive(Clone, Debug, PartialEq)]
pub enum Review {
    /// Nothing: it is a sample, folded by its harness's current parser.
    Current,
    /// Its record should hold this one, whose outcome the current parser folds and which is
    /// still a sample.
    Reparsed(Box<RunRecord>),
    /// It is no sample, for `reason`, as the outcome `parser` folded says: its attempt should
    /// be discarded with what it spent, and the run withdrawn with every phase built on it.
    Withdrawn {
        reason: Reason,
        parser: u32,
        usage: Vec<Sample>,
    },
}

/// Reviews `run`, committed in `dir`, with the current parser of `harness` and the sample
/// policy.
pub(super) fn review(dir: &TrialDir, run: &RunRecord, harness: Harness) -> Result<Review> {
    let parser = harness.parser_version();
    if run.parser >= parser {
        return Ok(match Reason::of(&run.outcome.exit, &run.process) {
            None => Review::Current,
            Some(reason) => Review::Withdrawn {
                reason,
                parser: run.parser,
                usage: run.outcome.usage.clone(),
            },
        });
    }
    let ending = conclude(harness, &dir.attempt(run.attempt), run.process.end())?;
    Ok(match Counted::of(ending, &run.process) {
        Counted::Sample(outcome) => Review::Reparsed(Box::new(RunRecord {
            parser,
            outcome,
            ..run.clone()
        })),
        Counted::Discarded { reason, usage } => Review::Withdrawn {
            reason,
            parser,
            usage,
        },
    })
}

/// Carries out `review` of `run`, committed in `dir`, discarding in the name of `session`. A
/// withdrawal removes the phases built on the run before the run itself, so a crash at any
/// point leaves the run committed until nothing depends on it, and the next invocation reviews
/// it again.
pub(super) fn apply(
    results: &ResultsDir,
    dir: &TrialDir,
    run: &RunRecord,
    review: &Review,
    session: &SessionId,
) -> Result<()> {
    match review {
        Review::Current => Ok(()),
        Review::Reparsed(updated) => rewrite_record(&dir.phase(Phase::Run), updated),
        Review::Withdrawn {
            reason,
            parser,
            usage,
        } => {
            for phase in [Phase::Measure, Phase::Grade] {
                results.retract(Phase::Run, &dir.phase(phase))?;
            }
            let discarded = DiscardedAttempt {
                session: session.clone(),
                started: run.started,
                process: Some(run.process),
                parser: *parser,
                reason: reason.clone(),
                usage: usage.clone(),
            };
            write_json(&dir.attempt(run.attempt).discarded(), &discarded)?;
            results.retract(Phase::Run, &dir.phase(Phase::Run))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        id::AttemptNo,
        lifecycle::{Current, Lifecycle, Stage},
        run::{
            AgentExit, Cut, ProcessExit,
            fixture::{self, Repo},
            records::Timeout,
        },
        store::{Digest, PartialDir, read_json, read_record},
        testing::write,
        trial::FINAL,
    };
    use std::fs;

    /// Commits a first attempt of the `opus` trial with the Claude Code transcript fixture
    /// `name` and the fixture's record as `edit` leaves it.
    fn committed(
        repo: &Repo,
        name: &str,
        edit: impl FnOnce(&mut RunRecord),
    ) -> (TrialDir, RunRecord) {
        let experiment = repo.experiment();
        let trial = fixture::trial(&experiment, "opus");
        let dir = experiment.results().trial(&trial);
        let transcript = fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/harness/claude-code")
                .join(name),
        )
        .unwrap();
        write(
            dir.attempt(AttemptNo::FIRST).path(),
            "transcript.jsonl",
            transcript,
        );
        let partial = PartialDir::begin(&dir.phase(Phase::Run)).unwrap();
        write(&partial.path().join(FINAL), "main.py", "print(input())\n");
        let mut record = fixture::record(repo, &trial, Digest::of_bytes(b"final"));
        edit(&mut record);
        partial.commit(&record).unwrap();
        (dir, record)
    }

    fn older_parser(record: &mut RunRecord) {
        record.parser = 0;
    }

    fn claude() -> Harness {
        Harness::named(&"claude-code".parse().unwrap()).unwrap()
    }

    fn reviewed(dir: &TrialDir, run: &RunRecord) -> Review {
        review(dir, run, claude()).unwrap()
    }

    fn applied(repo: &Repo, dir: &TrialDir, run: &RunRecord, review: &Review) {
        let results = repo.experiment().results().clone();
        apply(&results, dir, run, review, &fixture::session()).unwrap();
    }

    fn stage(repo: &Repo) -> Stage {
        let experiment = repo.experiment();
        let trial = fixture::trial(&experiment, "opus");
        let current = Current::of(&experiment, &trial, &repo.toolchains).unwrap();
        Lifecycle::read(&experiment.results().trial(&trial), &current)
            .unwrap()
            .stage
    }

    #[test]
    fn an_older_parser_s_outcome_is_re_derived_and_the_rest_kept() {
        let repo = Repo::new();
        let (dir, before) = committed(&repo, "haiku-bash-success.jsonl", older_parser);

        let review = reviewed(&dir, &before);
        let stored: RunRecord = read_record(&dir.phase(Phase::Run)).unwrap().unwrap();
        assert_eq!(stored, before, "reviewing writes nothing");
        applied(&repo, &dir, &before, &review);
        let after: RunRecord = read_record(&dir.phase(Phase::Run)).unwrap().unwrap();
        assert_eq!(review, Review::Reparsed(Box::new(after.clone())));
        assert_eq!(after.parser(), claude().parser_version());
        assert_eq!(after.outcome().exit, AgentExit::Completed);
        assert_eq!(after.outcome().turns, Some(2));
        assert_eq!(after.outcome().total().output, 78);
        assert_eq!(
            (after.inputs(), after.snapshot(), after.started()),
            (before.inputs(), before.snapshot(), before.started())
        );
        assert!(dir.final_workspace().join("main.py").is_file());

        assert_eq!(reviewed(&dir, &after), Review::Current);
    }

    #[test]
    fn a_rate_limit_an_older_parser_missed_withdraws_the_run() {
        let repo = Repo::new();
        let (dir, before) = committed(&repo, "synthetic-rate-limit-event.jsonl", older_parser);

        let review = reviewed(&dir, &before);
        applied(&repo, &dir, &before, &review);
        let discarded: DiscardedAttempt = read_json(&dir.attempt(AttemptNo::FIRST).discarded())
            .unwrap()
            .unwrap();
        assert_eq!(
            review,
            Review::Withdrawn {
                reason: discarded.reason.clone(),
                parser: claude().parser_version(),
                usage: discarded.usage.clone(),
            }
        );
        assert!(matches!(discarded.reason, Reason::RateLimited { .. }));
        assert_eq!(discarded.session, fixture::session());
        let output: u64 = discarded.usage.iter().map(|s| s.tokens.output).sum();
        assert_eq!(output, 40);
        assert!(!dir.phase(Phase::Run).exists());
        assert!(dir.attempt(AttemptNo::FIRST).transcript().is_file());
        assert_eq!(stage(&repo), Stage::Pending);
    }

    #[tokio::test]
    async fn a_committed_run_that_is_no_sample_is_withdrawn_with_the_phases_built_on_it() {
        let cases = [
            (
                AgentExit::Crashed,
                ProcessExit::OutOfMemory,
                None,
                Reason::Crashed,
            ),
            (
                AgentExit::Killed,
                ProcessExit::TimedOut,
                None,
                Reason::TimedOut {
                    budget: Timeout::Wall,
                },
            ),
            (
                AgentExit::Killed,
                ProcessExit::Killed,
                Some(Cut::Idle),
                Reason::TimedOut {
                    budget: Timeout::Idle,
                },
            ),
            (
                AgentExit::Errored {
                    message: "overloaded".to_owned(),
                },
                ProcessExit::Code(1),
                None,
                Reason::Errored {
                    message: "overloaded".to_owned(),
                },
            ),
        ];
        for (exit, process_exit, cut, reason) in cases {
            let repo = Repo::new();
            let (dir, run) = committed(&repo, "haiku-bash-success.jsonl", |record| {
                record.outcome.exit = exit;
                record.process.exit = process_exit;
                record.process.cut = cut;
            });
            let trial = fixture::trial(&repo.experiment(), "opus");
            fixture::commit_grade(&repo, &trial).await;
            write(&dir.phase(Phase::Measure), "record.json", "{}");

            let review = reviewed(&dir, &run);
            let Review::Withdrawn { reason: found, .. } = &review else {
                panic!("{reason}: expected a withdrawal, got {review:?}");
            };
            assert_eq!(found, &reason);
            assert!(dir.phase(Phase::Grade).is_dir(), "reviewing writes nothing");

            applied(&repo, &dir, &run, &review);
            applied(&repo, &dir, &run, &review);
            for phase in [Phase::Run, Phase::Grade, Phase::Measure] {
                assert!(!dir.phase(phase).exists(), "{reason}: {phase}");
            }
            let stored: DiscardedAttempt = read_json(&dir.attempt(AttemptNo::FIRST).discarded())
                .unwrap()
                .unwrap();
            assert_eq!(stored.reason, reason);
            assert!(dir.attempt(AttemptNo::FIRST).transcript().is_file());
            assert_eq!(stage(&repo), Stage::Pending, "{reason}");
        }
    }

    #[test]
    fn a_committed_sample_is_left_as_it_is() {
        let repo = Repo::new();
        for (exit, name) in [
            (AgentExit::Completed, "haiku-bash-success.jsonl"),
            (AgentExit::Refused, "synthetic-refusal.jsonl"),
        ] {
            let (dir, run) = committed(&repo, name, |record| record.outcome.exit = exit);
            assert_eq!(reviewed(&dir, &run), Review::Current);
            fs::remove_dir_all(dir.path()).unwrap();
        }
    }
}
