use super::*;
use crate::{
    grade::fake::{self, Scripted, finished},
    id::LanguageId,
    provenance::SessionId,
    sandbox::{Collected, Exit, Fault, Network, WORK},
    store::{PartialDir, read_record},
    testing::{self, id, write},
};
use eyre::eyre;
use serde_json::json;
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};

const CORE: u32 = 3;
const SETTINGS: Settings = Settings {
    core: CORE,
    blocks: NonZeroU32::new(2).unwrap(),
    runs: NonZeroU32::new(3).unwrap(),
    warmup: 1,
    memory: 1 << 30,
};

/// The fixture `python` language, which builds with `python3 -m compileall -q .` and `true`
/// and runs `python3 main.py`, and the workloads `large` and `small`, whose bytes are their
/// names. Each program's `main.py` holds a word that tells [`script`] how the program
/// behaves.
struct Fixture {
    root: TempDir,
    language: Language,
    toolchains: Toolchains,
    workloads: BTreeMap<WorkloadId, Workload>,
}

fn fixture() -> Fixture {
    let root = TempDir::new().unwrap();
    testing::language(root.path(), "python");
    let mut languages = Language::load_all(&root.path().join("languages")).unwrap();
    let language = languages.remove(&id::<LanguageId>("python")).unwrap();
    let toolchains = fake::toolchains(&root.path().join("toolchains"), &["python"]);
    let workloads = ["large", "small"]
        .map(|name| {
            let bytes = format!("{name}\n");
            write(root.path(), &format!("workloads/{name}.in"), &bytes);
            let workload = Workload {
                path: root.path().join(format!("workloads/{name}.in")),
                digest: Digest::of_bytes(bytes.as_bytes()),
            };
            (id(name), workload)
        })
        .into();
    Fixture {
        root,
        language,
        toolchains,
        workloads,
    }
}

impl Fixture {
    /// A workspace whose `main.py` holds `behaviour`.
    fn workspace(&self, behaviour: &str) -> PathBuf {
        let dir = self.root.path().join("programs").join(behaviour);
        write(&dir, "main.py", behaviour);
        dir
    }

    async fn program<'a>(
        &self,
        backend: &'a Scripted,
        stop: &'a Stop,
        behaviour: &str,
    ) -> Program<'a> {
        let workspace = self.workspace(behaviour);
        Program::build(&self.language, &self.toolchains, &workspace, backend, stop)
            .await
            .unwrap()
    }

    /// Builds both programs and compares them, returning the evidence directory alongside.
    async fn compare(
        &self,
        backend: &Scripted,
        reference: &str,
        candidate: &str,
    ) -> (Result<Comparison>, TempDir) {
        let stop = Stop::default();
        let evidence = TempDir::new().unwrap();
        let reference = self.program(backend, &stop, reference).await;
        let candidate = self.program(backend, &stop, candidate).await;
        let comparison = compare(
            &reference,
            &candidate,
            &self.workloads,
            &SETTINGS,
            backend,
            &stop,
            evidence.path(),
        )
        .await;
        (comparison, evidence)
    }
}

/// The behaviour written in the `main.py` of the program `spec` runs.
fn behaviour(spec: &Spec) -> String {
    fs::read_to_string(spec.mounts[0].host.join("main.py")).unwrap()
}

/// hyperfine's export of `runs` runs that each took `wall` seconds.
fn export(wall: f64, runs: usize) -> Vec<u8> {
    let run = json!({
        "time_wall_clock": {"value": wall, "unit": "second"},
        "time_user": {"value": wall, "unit": "second"},
        "time_system": {"value": 0.0, "unit": "second"},
        "memory_peak_resident": {"value": 1048576.0, "unit": "byte"},
        "exit_code": 0,
    });
    let measurements = vec![run; runs];
    let export = json!({"schema_version": 2, "results": [{"measurements": measurements}]});
    serde_json::to_vec(&export).unwrap()
}

/// Builds cleanly. `echo` programs print their input and time as 1 second, `slow-echo` ones
/// as 2. `wrong` prints something else, `flood` more than the check keeps, `crash`, `hang`
/// and `hog` end abnormally, and `flaky` passes its check but fails under hyperfine.
fn script(spec: &Spec, argv: &[String], io: Io) -> Result<Finished, Fault> {
    let behaviour = behaviour(spec);
    match argv[0].as_str() {
        "sh" => Ok(finished(Exit::Code(0), b"built\n")),
        "hyperfine" => match behaviour.as_str() {
            "flaky" => {
                let mut failed = finished(Exit::Code(1), b"");
                failed.stderr.bytes = b"Error: Command terminated with exit code 3".to_vec();
                Ok(failed)
            }
            _ => {
                let wall = if behaviour == "slow-echo" { 2.0 } else { 1.0 };
                let runs = &argv[argv.iter().position(|arg| arg == "--runs").unwrap() + 1];
                Ok(finished(
                    Exit::Code(0),
                    &export(wall, runs.parse().unwrap()),
                ))
            }
        },
        _ => {
            let Input::File(input) = io.stdin else {
                panic!("a check run reads its workload from a host file");
            };
            let input = fs::read(input).unwrap();
            Ok(match behaviour.as_str() {
                "wrong" => finished(Exit::Code(0), b"nope\n"),
                "crash" => finished(Exit::Code(3), &input),
                "hang" => finished(Exit::TimedOut, b""),
                "hog" => finished(Exit::OutOfMemory, b""),
                "flood" => Finished {
                    stdout: Collected {
                        bytes: input,
                        total: OUTPUT_CAP + 1,
                        truncated: true,
                    },
                    ..finished(Exit::Code(0), b"")
                },
                _ => finished(Exit::Code(0), &input),
            })
        }
    }
}

/// The behaviours of the programs hyperfine timed, in order, by the workload they ran on.
type Timings = Arc<Mutex<BTreeMap<String, Vec<String>>>>;

/// A backend that runs [`script`] and notes every timing as it starts.
fn backend() -> (Scripted, Timings) {
    let timings = Timings::default();
    let noted = Arc::clone(&timings);
    let backend = Scripted::new(move |spec, argv, io| {
        if argv[0] == "hyperfine" {
            let workload = spec.mounts[1].host.file_stem().unwrap().to_str().unwrap();
            let mut noted = noted.lock().unwrap();
            noted
                .entry(workload.to_owned())
                .or_default()
                .push(behaviour(spec));
        }
        script(spec, argv, io)
    });
    (backend, timings)
}

fn summaries(comparison: &Comparison) -> Vec<(&str, Summary)> {
    comparison
        .workloads
        .iter()
        .map(|(id, outcome)| {
            let Outcome::Timed(timed) = outcome else {
                panic!("{id} was not timed: {outcome:?}");
            };
            (id.as_str(), timed.summary())
        })
        .collect()
}

#[tokio::test]
async fn matching_outputs_are_timed_in_abba_blocks() {
    let fixture = fixture();
    let (backend, timings) = backend();
    let (comparison, evidence) = fixture.compare(&backend, "echo", "slow-echo").await;
    let comparison = comparison.unwrap();

    for (id, summary) in summaries(&comparison) {
        assert_eq!(summary.reference, Spread::of([1.0; 6]), "{id}");
        assert_eq!(summary.candidate, Spread::of([2.0; 6]), "{id}");
        assert!((summary.ratio.geomean - 2.0).abs() < 1e-12, "{id}");
        let (low, high) = summary.ratio.interval.unwrap();
        assert!(
            (low - 2.0).abs() < 1e-12 && (high - 2.0).abs() < 1e-12,
            "{id}"
        );
    }
    let abba = ["echo", "slow-echo", "slow-echo", "echo"].map(String::from);
    for (workload, order) in timings.lock().unwrap().iter() {
        assert_eq!(*order, abba, "{workload}");
    }

    for workload in ["large", "small"] {
        let dir = evidence.path().join("hyperfine").join(workload);
        let mut kept: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        kept.sort();
        let names = ["0-candidate", "0-reference", "1-candidate", "1-reference"];
        assert_eq!(kept, names.map(|name| format!("{name}.json")));
        for name in names {
            let wall = if name.ends_with("candidate") {
                2.0
            } else {
                1.0
            };
            let bytes = fs::read(dir.join(format!("{name}.json"))).unwrap();
            assert_eq!(bytes, export(wall, 3), "{workload}/{name}");
        }
    }
}

#[tokio::test]
async fn every_run_is_offline_read_only_and_pinned_with_hyperfine_beside_the_language() {
    let fixture = fixture();
    let backend = Scripted::new(script);
    let (comparison, _evidence) = fixture.compare(&backend, "echo", "echo").await;
    comparison.unwrap();

    let measure = fixture.toolchains.measure();
    let mut runs = 0;
    for (spec, argv) in backend.calls() {
        if argv[0] == "sh" {
            assert_eq!(spec.limits.cpu, None, "builds are not pinned");
            assert!(spec.mounts[0].writable);
            continue;
        }
        runs += 1;
        assert!(matches!(spec.network, Network::Offline));
        assert_eq!(spec.limits.cpu, Some(CORE));
        let workload = &spec.mounts[1].host;
        assert!(fixture.workloads.values().any(|w| &w.path == workload));
        assert_eq!(
            spec.mounts,
            [
                Mount::read_only(&spec.mounts[0].host, WORK),
                Mount::read_only(workload, WORKLOAD),
            ]
        );
        assert!(spec.closure.contains(measure.root()));
        assert!(spec.path.contains(&measure.bin()));
        if argv[0] == "hyperfine" {
            let expected = hyperfine::argv(&["python3".into(), "main.py".into()], &SETTINGS);
            let expected: Vec<String> = expected
                .into_iter()
                .map(|arg| arg.into_string().unwrap())
                .collect();
            assert_eq!(argv, expected);
            assert!(spec.limits.memory > SETTINGS.memory);
        } else {
            assert_eq!(argv, ["python3", "main.py"]);
            assert_eq!(spec.limits.memory, SETTINGS.memory);
        }
    }
    assert_eq!(
        runs,
        2 * (2 + 2 * 2),
        "two check runs and four timings per workload"
    );
}

#[tokio::test]
async fn a_candidate_that_fails_its_check_run_is_never_timed() {
    let failures = [
        ("wrong", Failure::WrongOutput),
        ("flood", Failure::OutputOverCap),
        (
            "crash",
            Failure::Abnormal(Abnormal::Crashed(Crash::Code(3))),
        ),
        ("hang", Failure::Abnormal(Abnormal::TimedOut)),
        ("hog", Failure::Abnormal(Abnormal::OutOfMemory)),
    ];
    for (behaviour, failure) in failures {
        let fixture = fixture();
        let (backend, timings) = backend();
        let (comparison, _evidence) = fixture.compare(&backend, "echo", behaviour).await;
        let comparison = comparison.unwrap();

        let outcome = Outcome::CandidateFailed(failure.clone());
        for (id, actual) in &comparison.workloads {
            assert_eq!(actual, &outcome, "{behaviour} on {id}");
        }
        assert!(timings.lock().unwrap().is_empty(), "{behaviour} was timed");
        let json = serde_json::to_string(&comparison).unwrap();
        assert_eq!(
            serde_json::from_str::<Comparison>(&json).unwrap(),
            comparison
        );
    }
}

#[tokio::test]
async fn a_candidate_that_fails_a_timed_run_is_timed_no_further() {
    let fixture = fixture();
    let (backend, timings) = backend();
    let (comparison, _evidence) = fixture.compare(&backend, "echo", "flaky").await;

    for (id, outcome) in comparison.unwrap().workloads {
        let Outcome::CandidateFailed(Failure::TimedRun(why)) = &outcome else {
            panic!("{id}: {outcome:?}");
        };
        assert!(why.contains("hyperfine exited 1"), "{why}");
        assert!(why.contains("terminated with exit code 3"), "{why}");
    }
    for (workload, order) in timings.lock().unwrap().iter() {
        assert_eq!(*order, ["echo", "flaky"], "{workload}");
    }
}

#[tokio::test]
async fn a_reference_that_fails_leaves_no_comparison() {
    for (reference, needle) in [
        ("crash", "the reference failed its check run: exited 3"),
        (
            "flaky",
            "the reference failed a timed run: hyperfine exited 1",
        ),
    ] {
        let fixture = fixture();
        let backend = Scripted::new(script);
        let (comparison, _evidence) = fixture.compare(&backend, reference, "echo").await;
        let message = format!("{:#}", comparison.unwrap_err());
        assert!(message.contains("workload large"), "{message}");
        assert!(message.contains(needle), "{message}");
    }
}

#[tokio::test]
async fn a_sandbox_fault_or_a_stop_leaves_no_comparison() {
    let fixture = fixture();
    let backend = Scripted::new(|spec, argv, io| match argv[0].as_str() {
        "hyperfine" => Err(eyre!("taskset: invalid CPU").into()),
        _ => script(spec, argv, io),
    });
    let (comparison, _evidence) = fixture.compare(&backend, "echo", "echo").await;
    let message = format!("{:#}", comparison.unwrap_err());
    assert!(
        message.contains("sandbox fault: taskset: invalid CPU"),
        "{message}"
    );

    let stop = Stop::default();
    let backend = Scripted::new(script);
    let program = fixture.program(&backend, &stop, "echo").await;
    stop.escalate();
    let evidence = TempDir::new().unwrap();
    let workloads = &fixture.workloads;
    let stopped = compare(
        &program,
        &program,
        workloads,
        &SETTINGS,
        &backend,
        &stop,
        evidence.path(),
    );
    let message = format!("{:#}", stopped.await.unwrap_err());
    assert!(message.contains("interrupted"), "{message}");
}

#[tokio::test]
async fn a_program_that_does_not_build_is_an_error_quoting_its_log() {
    let fixture = fixture();
    let backend = Scripted::new(|_, argv, _| {
        let exit = if argv.ends_with(&["true".to_owned()]) {
            2
        } else {
            0
        };
        Ok(finished(Exit::Code(exit), b"error: no\n"))
    });
    let workspace = fixture.workspace("echo");
    let stop = Stop::default();
    let built = Program::build(
        &fixture.language,
        &fixture.toolchains,
        &workspace,
        &backend,
        &stop,
    );
    let message = format!("{:#}", built.await.err().unwrap());
    assert!(message.contains("did not build"), "{message}");
    assert!(message.ends_with("$ true\nerror: no"), "{message}");
}

#[tokio::test]
async fn a_committed_measurement_reads_back_beside_its_evidence() {
    let fixture = fixture();
    let (backend, stop) = (Scripted::new(script), Stop::default());
    let reference = fixture.program(&backend, &stop, "echo").await;
    let candidate = fixture.program(&backend, &stop, "slow-echo").await;
    let results = TempDir::new().unwrap();
    let target = results.path().join("r1/measure");

    let partial = PartialDir::begin(&target).unwrap();
    let workloads = &fixture.workloads;
    let comparison = compare(
        &reference,
        &candidate,
        workloads,
        &SETTINGS,
        &backend,
        &stop,
        partial.path(),
    )
    .await
    .unwrap();
    assert_eq!(comparison.machine.core, CORE);
    assert!(comparison.machine.cpus >= 1 && !comparison.machine.kernel.is_empty());
    let inputs = MeasureInputs::new(
        &fixture.language,
        &fixture.toolchains,
        &reference,
        &candidate,
        workloads,
        &SETTINGS,
    )
    .unwrap();
    let session: SessionId = serde_json::from_str("\"20261103T120000Z-42\"").unwrap();
    let record = MeasureRecord::new(session, inputs, comparison.clone());
    partial.commit(&record).unwrap();

    let stored = read_record::<MeasureRecord>(&target).unwrap().unwrap();
    assert_eq!(stored, record);
    assert_eq!(stored.workloads(), &comparison.workloads);
    assert_eq!(stored.machine(), &comparison.machine);
    for workload in ["large", "small"] {
        let kept = fs::read_dir(target.join("hyperfine").join(workload)).unwrap();
        assert_eq!(kept.count(), 4, "{workload}");
    }
    assert!(target.join("record.json").is_file());
}

#[tokio::test]
async fn stored_inputs_go_stale_exactly_when_timing_could_change() {
    let fixture = fixture();
    let (backend, stop) = (Scripted::new(script), Stop::default());
    let reference = fixture.program(&backend, &stop, "echo").await;
    let inputs = |candidate: &Program<'_>, workloads: &BTreeMap<_, _>, settings: &Settings| {
        let (language, toolchains) = (&fixture.language, &fixture.toolchains);
        MeasureInputs::new(
            language, toolchains, &reference, candidate, workloads, settings,
        )
        .unwrap()
    };
    let candidate = fixture.program(&backend, &stop, "echo").await;
    let json = serde_json::to_string(&inputs(&candidate, &fixture.workloads, &SETTINGS));
    let stored: MeasureInputs = serde_json::from_str(&json.unwrap()).unwrap();
    assert_eq!(stored, inputs(&candidate, &fixture.workloads, &SETTINGS));

    let rebuilt = fixture.program(&backend, &stop, "echo").await;
    assert_eq!(stored, inputs(&rebuilt, &fixture.workloads, &SETTINGS));
    write(
        &fixture.workspace("echo"),
        "__pycache__/main.pyc",
        "build output",
    );
    let with_output = fixture.program(&backend, &stop, "echo").await;
    assert_eq!(stored, inputs(&with_output, &fixture.workloads, &SETTINGS));

    let edited = fixture.program(&backend, &stop, "slow-echo").await;
    assert_ne!(stored, inputs(&edited, &fixture.workloads, &SETTINGS));
    let mut workloads = fixture.workloads.clone();
    let large = workloads.get_mut(&id::<WorkloadId>("large")).unwrap();
    large.digest = Digest::of_bytes(b"other");
    assert_ne!(stored, inputs(&candidate, &workloads, &SETTINGS));
    let more_blocks = Settings {
        blocks: NonZeroU32::new(4).unwrap(),
        ..SETTINGS
    };
    assert_ne!(stored, inputs(&candidate, &fixture.workloads, &more_blocks));
}
