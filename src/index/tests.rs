use super::*;
use crate::{
    experiment::Experiment,
    measure::Protocol,
    provenance::Session,
    run::fixture::{self, Repo},
    store::{Digest, MANIFEST, PartialDir},
    testing,
    trial::{FINAL, Phase, TrialKey},
};
use serde_json::{Value, json};
use std::fs;
use tempfile::TempDir;

#[test]
fn dimensions_parse_by_name_and_reject_others() {
    for dimension in Dimension::ALL {
        assert_eq!(dimension.as_str().parse::<Dimension>().unwrap(), dimension);
    }
    let err = "runtime".parse::<Dimension>().unwrap_err();
    assert!(err.to_string().contains("pass, tokens"), "{err}");
}

#[test]
fn weights_default_to_one_and_refuse_what_cannot_weigh() {
    let weights = Weights::new([(Dimension::Speed, 0.0), (Dimension::Pass, 2.5)]).unwrap();
    assert_eq!(
        Dimension::ALL.map(|dimension| weights.of(dimension)),
        [2.5, 1.0, 1.0, 0.0, 1.0]
    );
    for (set, message) in [
        (vec![(Dimension::Time, -1.0)], "at least 0"),
        (vec![(Dimension::Time, f64::NAN)], "at least 0"),
        (
            vec![(Dimension::Time, 1.0), (Dimension::Time, 2.0)],
            "weighed twice",
        ),
        (
            Dimension::ALL.map(|dimension| (dimension, 0.0)).to_vec(),
            "weighs 0",
        ),
    ] {
        let err = Weights::new(set).unwrap_err();
        assert!(err.to_string().contains(message), "{err}");
    }
}

/// Commits the run and the passing grade of `model`'s trial.
async fn commit_graded(repo: &Repo, model: &str) {
    let trial = fixture::trial(&repo.experiment(), model);
    fixture::commit_run(repo, &trial, &[("main.py", "print(input())\n")]);
    fixture::commit_grade(repo, &trial).await;
}

#[tokio::test]
async fn only_the_samples_of_generations_graded_completely_enough_count() {
    let repo = Repo::new();
    let experiment = repo.experiment();
    commit_graded(&repo, "opus").await;
    let luna = fixture::trial(&experiment, "luna");
    fixture::commit_run(&repo, &luna, &[("main.py", "print(input())\n")]);

    let read = read(&[experiment], &repo.toolchains, 0.95).unwrap();

    assert_eq!(
        read.samples,
        [Sample {
            model: testing::id("opus"),
            scenario: testing::id("echo"),
            language: testing::id("python"),
            passed: true,
            tokens: 0,
            wall_s: 60.0,
            ccn: None,
            timings: Vec::new(),
        }]
    );
    let generations: Vec<(&str, usize, usize, bool, bool)> = (read.generations.iter())
        .map(|generation| {
            let model = generation.system.model().as_str();
            let complete = generation.is_complete(read.complete);
            let cells = (generation.cells, generation.graded);
            (model, cells.0, cells.1, complete, generation.counted)
        })
        .collect();
    assert_eq!(
        generations,
        [("opus", 1, 1, true, true), ("luna", 1, 0, false, false)]
    );
}

/// The fixture's trial of `model` at `effort`.
fn trial_at(experiment: &Experiment, model: &str, effort: &str) -> TrialKey {
    let trials = experiment.trials().into_iter();
    trials
        .filter(|trial| trial.system.model().as_str() == model)
        .find(|trial| trial.system.effort().as_str() == effort)
        .unwrap()
}

#[tokio::test]
async fn a_generation_counts_only_once_its_model_finished_it_at_every_effort() {
    let repo = Repo::new();
    let both = fixture::EXPERIMENT_TOML.replacen(
        "model = \"opus\"\nefforts = [\"low\"]",
        "model = \"opus\"\nefforts = [\"low\", \"high\"]",
        1,
    );
    testing::write(repo.root.path(), "experiments/fixture.toml", both);
    let experiment = repo.experiment();
    for effort in ["low", "high"] {
        let trial = trial_at(&experiment, "opus", effort);
        fixture::commit_run(&repo, &trial, &[("main.py", "print(input())\n")]);
    }
    fixture::commit_grade(&repo, &trial_at(&experiment, "opus", "low")).await;

    let counted = |read: &Read| -> Vec<(&str, bool, bool)> {
        let opus = read.generations.iter();
        opus.filter(|generation| generation.system.model().as_str() == "opus")
            .map(|generation| {
                let effort = generation.system.effort().as_str();
                (
                    effort,
                    generation.is_complete(read.complete),
                    generation.counted,
                )
            })
            .collect()
    };
    let read = read(std::slice::from_ref(&experiment), &repo.toolchains, 0.95).unwrap();
    assert!(read.samples.is_empty(), "{:?}", read.samples);
    assert_eq!(
        counted(&read),
        [("high", false, false), ("low", true, false)]
    );

    fixture::commit_grade(&repo, &trial_at(&experiment, "opus", "high")).await;
    let read = super::read(&[experiment], &repo.toolchains, 0.95).unwrap();
    assert_eq!(read.samples.len(), 2);
    assert_eq!(counted(&read), [("high", true, true), ("low", true, true)]);
}

#[tokio::test]
async fn tokens_count_every_bucket_once_writes_to_the_one_hour_cache_among_them() {
    let repo = Repo::new();
    let experiment = repo.experiment();
    let opus = fixture::trial(&experiment, "opus");
    let dir = experiment.results().trial(&opus);
    let partial = PartialDir::begin(&dir.phase(Phase::Run)).unwrap();
    let workspace = partial.path().join(FINAL);
    testing::write(&workspace, "main.py", "print(input())\n");
    let snapshot = Digest::of_tree(&workspace, experiment.languages()[0].ignore()).unwrap();
    let mut record = serde_json::to_value(fixture::record(&repo, &opus, snapshot)).unwrap();
    record["outcome"]["usage"] = json!([{
        "request": "msg_1",
        "model": "claude-opus",
        "tokens": {
            "input": 1,
            "cache_read": 20,
            "cache_write": 300,
            "cache_write_long": 4000,
            "output": 50000,
            "reasoning": 10000,
        },
    }]);
    partial.commit(&record).unwrap();
    fixture::commit_grade(&repo, &opus).await;

    let read = read(&[experiment], &repo.toolchains, 0.95).unwrap();
    // Reasoning is part of the output, so it counts once.
    assert_eq!(read.samples[0].tokens, 54321);
}

#[tokio::test]
async fn a_sample_is_timed_on_the_workloads_its_current_measurement_timed() {
    let repo = Repo::new();
    commit_graded(&repo, "opus").await;
    let opus = fixture::trial(&repo.experiment(), "opus");
    fixture::commit_timed(&repo, &opus, Protocol::DEFAULT);

    let read_with = |repo: &Repo| read(&[repo.experiment()], &repo.toolchains, 0.95).unwrap();
    // The small workload printed the wrong output, so only the large one was timed.
    assert_eq!(
        read_with(&repo).samples[0].timings,
        [Timing {
            workload: testing::id("large"),
            candidate_median_s: 1.0,
            reference_median_s: 0.5,
        }]
    );

    let longer = Protocol {
        warmup: Protocol::DEFAULT.warmup + 1,
        ..Protocol::DEFAULT
    };
    fixture::declare_protocol(&repo, longer);
    assert_eq!(read_with(&repo).samples[0].timings, []);
}

#[test]
fn an_experiment_given_twice_is_an_error() {
    let repo = Repo::new();
    let experiments = [repo.experiment(), repo.experiment()];
    let err = read(&experiments, &repo.toolchains, 0.95).unwrap_err();
    assert!(err.to_string().contains("given twice"), "{err}");
}

#[tokio::test]
async fn the_scores_replace_the_output_directory_beside_their_manifest() {
    let repo = Repo::new();
    commit_graded(&repo, "opus").await;
    let read = read(&[repo.experiment()], &repo.toolchains, 0.95).unwrap();
    let weights = Weights::new([(Dimension::Speed, 0.0)]).unwrap();
    let scores = score(&read.samples, &weights, Bootstrap::DEFAULT);
    let parent = TempDir::new().unwrap();
    let out = parent.path().join("index");
    let write_to = |out: &std::path::Path| {
        let session = Session::start(repo.root.path()).unwrap();
        write(out, session, &read, &scores, &weights, Bootstrap::DEFAULT)
    };

    write_to(&out).unwrap();

    // One language is its own average, so whatever it has scores 100. The fixture's session
    // reported no tokens, its Python measures no complexity, and nothing was timed.
    let scores_csv = fs::read_to_string(out.join("scores.csv")).unwrap();
    let mut lines = scores_csv.lines();
    assert_eq!(
        lines.next(),
        Some("scope,language,dimension,score,low,high")
    );
    let rows: Vec<&str> = lines.collect();
    assert_eq!(
        rows,
        [
            "all,python,index,100,100,100",
            "all,python,pass,100,100,100",
            "all,python,tokens,,,",
            "all,python,time,100,100,100",
            "all,python,speed,,,",
            "all,python,simplicity,,,",
            "opus,python,index,100,100,100",
            "opus,python,pass,100,100,100",
            "opus,python,tokens,,,",
            "opus,python,time,100,100,100",
            "opus,python,speed,,,",
            "opus,python,simplicity,,,",
        ]
    );
    let generations = fs::read_to_string(out.join("generations.csv")).unwrap();
    assert_eq!(
        generations.lines().collect::<Vec<_>>(),
        [
            "experiment,system,model,effort,rep,cells,graded,complete,counted",
            "fixture,claude-code-2.1.316_anthropic_opus_low,opus,low,1,1,1,true,true",
            "fixture,codex-0.166.0_openai_luna_low,luna,low,1,1,0,false,false",
        ]
    );
    let manifest: Value = serde_json::from_slice(&fs::read(out.join(MANIFEST)).unwrap()).unwrap();
    assert_eq!(manifest["complete"], 0.95);
    assert_eq!(manifest["samples"], 1);
    assert_eq!(manifest["weights"]["speed"], 0.0);
    assert_eq!(manifest["weights"]["pass"], 1.0);
    assert_eq!(manifest["bootstrap"]["resamples"], 4000);
    assert_eq!(manifest["tables"]["scores.csv"], 12);
    assert_eq!(
        manifest["experiments"][0]["sessions"],
        serde_json::json!(["20261103T120000Z-42"])
    );

    testing::write(&out, "stray.txt", "left by hand");
    write_to(&out).unwrap();
    assert!(!out.join("stray.txt").exists());

    let results = parent.path().join("results");
    testing::write(&results, "trials/kept.json", "{}");
    let err = write_to(&results).unwrap_err();
    assert!(err.to_string().contains("not a gungnir index"), "{err}");
    assert!(results.join("trials/kept.json").is_file());
}
