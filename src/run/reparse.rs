//! Reparsing: a committed outcome re-derived from its stored transcript once its harness's
//! parser has changed, without calling a model. A rate limit the older parser missed demotes
//! the attempt to discarded, and the trial is pending again.
use super::{
    Reason, RunRecord,
    attempt::conclude,
    records::{DiscardedAttempt, samples},
};
use crate::{
    harness::{Ending, Harness},
    provenance::SessionId,
    store::write_json,
    trial::{Phase, ResultsDir, TrialDir},
};
use eyre::Result;

/// What reparsing did to a committed run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reparsed {
    /// The run's parser is the harness's current one.
    Current,
    /// The run's record now holds the outcome the current parser folds.
    Updated,
    /// The current parser found a rate limit: the attempt is discarded and the run withdrawn.
    Demoted,
}

/// Re-derives the outcome of `run`, committed in `dir`, when `harness` now parses with a
/// newer version than the one that folded it. Safe to repeat after a crash at any point.
pub(super) fn reparse(
    results: &ResultsDir,
    dir: &TrialDir,
    run: &RunRecord,
    harness: Harness,
    session: &SessionId,
) -> Result<Reparsed> {
    let parser = harness.parser_version();
    if run.parser >= parser {
        return Ok(Reparsed::Current);
    }
    let attempt = dir.attempt(run.attempt);
    match conclude(harness, &attempt, run.process.end())? {
        Ending::Finished(outcome) => {
            let updated = RunRecord {
                parser,
                outcome: outcome.into(),
                ..run.clone()
            };
            write_json(&dir.record(Phase::Run), &updated)?;
            Ok(Reparsed::Updated)
        }
        Ending::RateLimited { resets_at, usage } => {
            let discarded = DiscardedAttempt {
                session: session.clone(),
                started: run.started,
                process: Some(run.process),
                parser,
                reason: Reason::RateLimited { resets_at },
                usage: samples(usage),
            };
            write_json(&attempt.discarded(), &discarded)?;
            results.retract(Phase::Run, &dir.phase(Phase::Run))?;
            Ok(Reparsed::Demoted)
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
            AgentExit,
            fixture::{self, Repo},
        },
        store::{Digest, PartialDir, read_json, read_record},
        testing::write,
        trial::FINAL,
    };
    use std::fs;

    /// Commits a first attempt of the `opus` trial that an older parser folded, with the
    /// captured Claude Code transcript `name`.
    fn committed(repo: &Repo, name: &str) -> (TrialDir, RunRecord) {
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
        let record = RunRecord {
            parser: 0,
            ..fixture::record(repo, &trial, Digest::of_bytes(b"final"))
        };
        partial.commit(&record).unwrap();
        (dir, record)
    }

    fn claude() -> Harness {
        Harness::named(&"claude-code".parse().unwrap()).unwrap()
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
        let (dir, before) = committed(&repo, "haiku-bash-success.jsonl");
        let results = repo.experiment().results().clone();
        let session = fixture::session();

        let reparsed = reparse(&results, &dir, &before, claude(), &session).unwrap();
        assert_eq!(reparsed, Reparsed::Updated);
        let after: RunRecord = read_record(&dir.phase(Phase::Run)).unwrap().unwrap();
        assert_eq!(after.parser(), claude().parser_version());
        assert_eq!(after.outcome().exit, AgentExit::Completed);
        assert_eq!(after.outcome().turns, Some(2));
        assert_eq!(after.outcome().total().output, 78);
        assert_eq!(
            (after.inputs(), after.snapshot(), after.started()),
            (before.inputs(), before.snapshot(), before.started())
        );
        assert!(dir.final_workspace().join("main.py").is_file());

        assert_eq!(
            reparse(&results, &dir, &after, claude(), &session).unwrap(),
            Reparsed::Current
        );
    }

    #[test]
    fn a_rate_limit_an_older_parser_missed_demotes_the_attempt() {
        let repo = Repo::new();
        let (dir, before) = committed(&repo, "synthetic-rate-limit-event.jsonl");
        let results = repo.experiment().results().clone();
        let session = fixture::session();

        let reparsed = reparse(&results, &dir, &before, claude(), &session).unwrap();
        assert_eq!(reparsed, Reparsed::Demoted);
        let discarded: DiscardedAttempt = read_json(&dir.attempt(AttemptNo::FIRST).discarded())
            .unwrap()
            .unwrap();
        let output: u64 = discarded.usage.iter().map(|s| s.tokens.output).sum();
        assert_eq!(output, 40);
        assert!(!dir.phase(Phase::Run).exists());
        assert!(dir.attempt(AttemptNo::FIRST).transcript().is_file());
        assert_eq!(stage(&repo), Stage::Pending);
    }
}
