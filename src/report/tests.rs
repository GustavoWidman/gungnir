use super::*;
use crate::{
    id::AttemptNo,
    measure::{self, Block, Comparison, Load, Machine, MeasureRecord, PeakMemory, Protocol, Timed},
    run::fixture::{self, Repo},
    store::{Digest, PartialDir},
    testing::{self, write},
    trial::{FINAL, Phase, TrialKey},
};
use serde_json::{Value, json};
use std::path::Path;
use tempfile::TempDir;

const DISCARDING_SESSION: &str = "20261103T110000Z-7";

/// Commits `trial`'s second attempt with one request's usage and a quota reading, after a
/// first attempt a rate limit discarded.
fn commit_second_attempt(repo: &Repo, trial: &TrialKey) {
    let dir = repo.experiment().results().trial(trial);
    write(
        dir.attempt(AttemptNo::FIRST).path(),
        "discarded.json",
        json!({
            "session": DISCARDING_SESSION,
            "started": "2026-11-03T11:00:00Z",
            "process": {
                "exit": "killed",
                "cut": "rate-limited",
                "wall": "30s",
                "stdout_bytes": 0,
                "stderr_bytes": 0,
            },
            "parser": 1,
            "resets_at": null,
            "usage": [sample("msg_0", 100, 0, 50)],
        })
        .to_string(),
    );
    write(
        dir.attempt(AttemptNo::FIRST.next()).path(),
        "stderr.log",
        "",
    );

    let partial = PartialDir::begin(&dir.phase(Phase::Run)).unwrap();
    write(&partial.path().join(FINAL), "main.py", "print(input())\n");
    let mut record = serde_json::to_value(fixture::record(repo, trial, Digest::of_bytes(b"x")))
        .expect("a run record serializes");
    record["attempt"] = json!(2);
    record["outcome"]["usage"] = json!([sample("msg_1", 1000, 2000, 500)]);
    record["outcome"]["quota"] = json!({ "five_hour": { "first": 0.1, "last": 0.25 } });
    partial.commit(&record).unwrap();
}

/// Commits a measurement of `trial` made under `protocol`: on `large` the candidate took twice
/// as long as the reference, and on `small` it printed the wrong output.
fn commit_measure(repo: &Repo, trial: &TrialKey, protocol: Protocol) {
    let experiment = repo.experiment();
    let dir = experiment.results().trial(trial);
    let inputs = Current::of(&experiment, trial, &repo.toolchains)
        .unwrap()
        .with_protocol(protocol)
        .measure(&dir.final_workspace())
        .unwrap();
    let runs = |walls: &[f64]| -> Vec<measure::Sample> {
        walls
            .iter()
            .map(|&wall| measure::Sample {
                wall,
                user: wall,
                system: 0.0,
                peak_memory: 3 << 20,
                instructions: None,
            })
            .collect()
    };
    let large = Timed {
        blocks: vec![
            Block {
                reference: runs(&[0.5, 0.5]),
                candidate: runs(&[1.0, 1.0]),
            },
            Block {
                candidate: runs(&[1.0, 1.0]),
                reference: runs(&[0.5, 0.5]),
            },
        ],
        peak_memory: PeakMemory {
            reference: Some(1 << 20),
            candidate: Some(2 << 20),
        },
    };
    let comparison = Comparison {
        machine: Machine {
            cpu: None,
            cpus: 1,
            core: 0,
            governor: None,
            kernel: "7.3.0".into(),
            perf_event_paranoid: Some(3),
            load: Load {
                start: [0.5, 0.25, 0.25],
                end: [0.75, 0.25, 0.25],
            },
        },
        workloads: BTreeMap::from([
            (testing::id("large"), measure::Outcome::Timed(large)),
            (
                testing::id("small"),
                measure::Outcome::CandidateFailed(measure::Failure::WrongOutput),
            ),
        ]),
    };
    let record = MeasureRecord::new(fixture::session(), inputs, comparison);
    let partial = PartialDir::begin(&dir.phase(Phase::Measure)).unwrap();
    partial.commit(&record).unwrap();
}

fn sample(request: &str, input: u64, cache_read: u64, output: u64) -> Value {
    json!({
        "request": request,
        "model": "claude-opus",
        "tokens": {
            "input": input,
            "cache_read": cache_read,
            "cache_write": 0,
            "cache_write_long": 0,
            "output": output,
            "reasoning": null,
        },
    })
}

/// Leaves `trial` with one attempt that is neither committed nor discarded, whose transcript
/// is the captured Codex session `name`.
fn interrupt(repo: &Repo, trial: &TrialKey, name: &str) {
    let transcript = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/harness/codex")
            .join(name),
    )
    .unwrap();
    let dir = repo.experiment().results().trial(trial);
    write(
        dir.attempt(AttemptNo::FIRST).path(),
        "transcript.jsonl",
        transcript,
    );
}

/// Each row of the CSV file `name` in `dir`, by column name. The fixture's cells hold no
/// separators, so splitting on commas reads them.
fn table(dir: &Path, name: &str) -> Vec<BTreeMap<String, String>> {
    let text = fs::read_to_string(dir.join(name)).unwrap();
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().unwrap().split(',').collect();
    lines
        .map(|line| {
            let cells = line.split(',').map(str::to_owned);
            header
                .iter()
                .map(|name| (*name).to_owned())
                .zip(cells)
                .collect()
        })
        .collect()
}

fn cells<'a>(rows: &'a [BTreeMap<String, String>], columns: &[&str]) -> Vec<Vec<&'a str>> {
    rows.iter()
        .map(|row| columns.iter().map(|column| row[*column].as_str()).collect())
        .collect()
}

fn day() -> NaiveDate {
    "2026-11-03".parse().unwrap()
}

fn write_report(repo: &Repo, out: &Path) -> Result<Manifest> {
    let session = Session::start(repo.root.path()).unwrap();
    report(&[repo.experiment()], &repo.toolchains, day(), session, out)
}

#[tokio::test]
async fn the_committed_run_describes_its_trial_and_attempts_that_committed_nothing_are_left_out() {
    let repo = Repo::new();
    let experiment = repo.experiment();
    let opus = fixture::trial(&experiment, "opus");
    commit_second_attempt(&repo, &opus);
    fixture::commit_grade(&repo, &opus).await;
    interrupt(
        &repo,
        &fixture::trial(&experiment, "luna"),
        "luna-command-success.jsonl",
    );
    let out = TempDir::new().unwrap();
    let out = out.path().join("report");

    let manifest = write_report(&repo, &out).unwrap();

    let trials = table(&out, "trials.csv");
    let columns = [
        "experiment",
        "system",
        "model",
        "effort_rank",
        "stage",
        "exit",
        "wall_s",
        "input_tokens",
        "cache_read_tokens",
        "output_tokens",
        "cost_usd",
        "verdict",
        "cases_passed",
        "cases_total",
        "functions",
    ];
    // 1000 input at $4, 2000 cache reads at $0.2 and 500 output at $20, per million tokens.
    let expected = [
        "fixture",
        "claude-code-2.1.316_anthropic_opus_low",
        "opus",
        "2",
        "graded",
        "completed",
        "60",
        "1000",
        "2000",
        "500",
        "0.0144",
        "pass",
        "2",
        "2",
        "",
    ];
    assert_eq!(cells(&trials, &columns), [expected]);

    let usage = table(&out, "usage.csv");
    let columns = ["model", "sample", "request", "cost_usd"];
    assert_eq!(
        cells(&usage, &columns),
        [["claude-opus", "1", "msg_1", "0.0144"]]
    );
    let quota = table(&out, "quota.csv");
    assert_eq!(
        cells(
            &quota,
            &[
                "system",
                "window",
                "first_used_fraction",
                "last_used_fraction"
            ]
        ),
        [[
            "claude-code-2.1.316_anthropic_opus_low",
            "five_hour",
            "0.1",
            "0.25"
        ]]
    );
    let cases = table(&out, "cases.csv");
    assert_eq!(
        cells(&cases, &["case", "outcome"]),
        [["edge", "pass"], ["many", "pass"]]
    );
    for empty in ["functions.csv", "workloads.csv", "timings.csv"] {
        assert!(table(&out, empty).is_empty(), "{empty}");
    }

    let written: Value = serde_json::from_slice(&fs::read(out.join(MANIFEST)).unwrap()).unwrap();
    assert_eq!(written["prices"], "2026-11-03");
    assert_eq!(
        written["experiments"][0]["sessions"],
        json!(["20261103T120000Z-42"])
    );
    assert_eq!(written["tables"]["usage.csv"], 1);
    assert_eq!(manifest.tables()["trials.csv"], 1);
    let mut files: Vec<String> = fs::read_dir(&out)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    files.sort();
    assert_eq!(
        files,
        [
            "cases.csv",
            "functions.csv",
            MANIFEST,
            "quota.csv",
            "timings.csv",
            "trials.csv",
            "usage.csv",
            "workloads.csv"
        ]
    );
    for file in &files {
        let text = fs::read_to_string(out.join(file)).unwrap();
        assert!(!text.contains(DISCARDING_SESSION), "{file}");
        assert!(!text.contains("attempt"), "{file}");
    }
}

#[test]
fn a_committed_run_that_is_no_sample_is_left_out() {
    let repo = Repo::new();
    let experiment = repo.experiment();
    for (model, exit, process) in [
        ("opus", "completed", json!({ "code": 0 })),
        ("luna", "killed", json!("timed-out")),
    ] {
        let trial = fixture::trial(&experiment, model);
        let dir = experiment.results().trial(&trial);
        let partial = PartialDir::begin(&dir.phase(Phase::Run)).unwrap();
        write(&partial.path().join(FINAL), "main.py", "print(input())\n");
        let mut record =
            serde_json::to_value(fixture::record(&repo, &trial, Digest::of_bytes(b"")))
                .expect("a run record serializes");
        record["outcome"]["exit"] = json!(exit);
        record["process"]["exit"] = process;
        record["outcome"]["usage"] = json!([sample("msg_1", 10, 0, 5)]);
        partial.commit(&record).unwrap();
    }
    let out = TempDir::new().unwrap();
    let out = out.path().join("report");

    write_report(&repo, &out).unwrap();
    let trials = table(&out, "trials.csv");
    assert_eq!(cells(&trials, &["model", "exit"]), [["opus", "completed"]]);
    assert_eq!(table(&out, "usage.csv").len(), 1);
}

#[tokio::test]
async fn only_a_current_measurement_is_tabled() {
    let repo = Repo::new();
    let experiment = repo.experiment();
    let [opus, luna] = ["opus", "luna"].map(|model| fixture::trial(&experiment, model));
    for trial in [&opus, &luna] {
        fixture::commit_run(&repo, trial, &[("main.py", "print(input())\n")]);
        fixture::commit_grade(&repo, trial).await;
    }
    commit_measure(&repo, &opus, Protocol::DEFAULT);
    let longer = Protocol {
        warmup: Protocol::DEFAULT.warmup + 1,
        ..Protocol::DEFAULT
    };
    commit_measure(&repo, &luna, longer);
    let out = TempDir::new().unwrap();

    write_report(&repo, out.path()).unwrap();

    let trials = table(out.path(), "trials.csv");
    assert_eq!(
        cells(
            &trials,
            &[
                "model",
                "stage",
                "stale_phase",
                "verdict",
                "measure_session",
                "measure_load_start",
                "measure_load_end",
            ]
        ),
        [
            [
                "opus",
                "measured",
                "",
                "pass",
                "20261103T120000Z-42",
                "0.5",
                "0.75"
            ],
            [
                "luna",
                "measured",
                "measure",
                "pass",
                "20261103T120000Z-42",
                "0.5",
                "0.75"
            ],
        ]
    );
    let opus_id = opus.system.id().to_string();
    let workloads = table(out.path(), "workloads.csv");
    let columns = [
        "system",
        "workload",
        "outcome",
        "blocks",
        "wall_ratio",
        "wall_ratio_low",
        "wall_ratio_high",
        "reference_median_s",
        "reference_min_s",
        "candidate_max_s",
        "instructions_ratio",
        "reference_peak_memory_bytes",
        "candidate_peak_memory_bytes",
    ];
    // Both blocks put the candidate at twice the reference's time, so the interval is a point.
    assert_eq!(
        cells(&workloads, &columns),
        [
            [
                opus_id.as_str(),
                "large",
                "timed",
                "2",
                "2",
                "2",
                "2",
                "0.5",
                "0.5",
                "1",
                "",
                "1048576",
                "2097152",
            ],
            [
                opus_id.as_str(),
                "small",
                "wrong-output",
                "0",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
            ],
        ]
    );
    let timings = table(out.path(), "timings.csv");
    assert_eq!(timings.len(), 8);
    assert!(timings.iter().all(|row| row["system"] == opus_id));
}

#[test]
fn a_report_replaces_the_last_one_and_nothing_else() {
    let repo = Repo::new();
    let parent = TempDir::new().unwrap();
    let out = parent.path().join("report");
    write_report(&repo, &out).unwrap();
    write(&out, "stray.txt", "left by hand");

    write_report(&repo, &out).unwrap();
    assert!(!out.join("stray.txt").exists());
    assert!(out.join(MANIFEST).is_file());
    let siblings: Vec<_> = fs::read_dir(parent.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(siblings, ["report"]);

    let results = parent.path().join("results");
    write(&results, "trials/kept.json", "{}");
    let err = write_report(&repo, &results).unwrap_err();
    assert!(err.to_string().contains("not a report"), "{err}");
    assert!(results.join("trials/kept.json").is_file());
}

#[test]
fn an_experiment_given_twice_is_an_error() {
    let repo = Repo::new();
    let out = TempDir::new().unwrap();
    let session = Session::start(repo.root.path()).unwrap();
    let experiments = [repo.experiment(), repo.experiment()];
    let err = report(&experiments, &repo.toolchains, day(), session, out.path()).unwrap_err();
    assert!(err.to_string().contains("fixture"), "{err}");
}
