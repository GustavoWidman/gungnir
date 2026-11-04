use super::{
    scripted::{Scripted, finished},
    *,
};
use crate::{
    id::{LanguageId, ScenarioId},
    sandbox::{Collected, Exit},
    testing::{self, id, write},
};
use eyre::eyre;
use std::{
    os::unix::fs::symlink,
    sync::{Arc, Mutex},
};
use tempfile::TempDir;

/// The `echo` scenario with hidden cases `edge` and `many`, compared by whitespace, and a
/// `python` language whose two build steps are `python3 -m compileall -q .` and `true`.
struct Fixture {
    root: TempDir,
    languages: BTreeMap<LanguageId, Language>,
    scenarios: BTreeMap<ScenarioId, Scenario>,
    toolchains: Toolchains,
}

fn fixture() -> Fixture {
    let root = TempDir::new().unwrap();
    testing::language(root.path(), "python");
    write(
        root.path(),
        "languages/python/language.toml",
        testing::LANGUAGE_TOML.replace("lizard = \"python\"\n", ""),
    );
    testing::scenario(root.path(), "echo", &["python"]);
    for (dir, env) in [
        ("toolchains/base", "aaaa-gungnir-base"),
        ("toolchains/language/python", "bbbb-gungnir-language-python"),
    ] {
        write(
            root.path(),
            &format!("{dir}/closure"),
            format!("/nix/store/{env}\n"),
        );
        symlink(
            format!("/nix/store/{env}/bin"),
            root.path().join(dir).join("bin"),
        )
        .unwrap();
    }
    let languages = Language::load_all(&root.path().join("languages")).unwrap();
    let scenarios = Scenario::load_all(&root.path().join("scenarios"), &languages).unwrap();
    let toolchains = Toolchains::load(&root.path().join("toolchains")).unwrap();
    Fixture {
        root,
        languages,
        scenarios,
        toolchains,
    }
}

impl Fixture {
    fn workspace(&self) -> PathBuf {
        self.root.path().join("scenarios/echo/reference/python")
    }

    fn hidden(&self) -> PathBuf {
        self.root.path().join("scenarios/echo/hidden")
    }

    /// Judges the workspace, returning the evidence directory alongside.
    async fn judge(&self, backend: &Scripted) -> (Result<Judgement>, TempDir) {
        let evidence = TempDir::new().unwrap();
        let judgement = judge(
            &self.languages[&id::<LanguageId>("python")],
            &self.toolchains,
            &self.workspace(),
            &self.scenarios[&id::<ScenarioId>("echo")],
            backend,
            &Stop::default(),
            evidence.path(),
        )
        .await;
        (judgement, evidence)
    }
}

fn is_case(argv: &[String]) -> bool {
    argv == ["python3", "main.py"]
}

/// The name of the case whose input is on stdin.
fn case_name(io: &Io) -> String {
    let Input::File(path) = &io.stdin else {
        panic!("a case reads its input from a host file");
    };
    path.file_stem().unwrap().to_str().unwrap().to_owned()
}

/// A program whose build steps print `built` and whose run echoes its input.
fn echo(_: &Spec, argv: &[String], io: Io) -> Result<Finished, Fault> {
    if !is_case(argv) {
        return Ok(finished(Exit::Code(0), b"built\n"));
    }
    let Input::File(path) = io.stdin else {
        panic!("a case reads its input from a host file");
    };
    Ok(finished(Exit::Code(0), &fs::read(path).unwrap()))
}

/// Builds cleanly, then ends each case as `ending` decides from the case's name.
fn cases_end(ending: impl Fn(&str) -> Result<Finished, Fault> + Send + Sync + 'static) -> Scripted {
    Scripted::new(move |spec, argv, io| {
        if is_case(argv) {
            ending(&case_name(&io))
        } else {
            echo(spec, argv, io)
        }
    })
}

fn outcomes(judgement: &Judgement) -> Vec<(&str, &Outcome)> {
    judgement
        .cases()
        .iter()
        .map(|(id, case)| (id.as_str(), &case.outcome))
        .collect()
}

#[tokio::test]
async fn a_program_that_prints_the_expected_output_passes() {
    let fixture = fixture();
    let (judgement, evidence) = fixture.judge(&Scripted::new(echo)).await;
    let judgement = judgement.unwrap();

    assert_eq!(judgement.verdict(), Verdict::Pass);
    assert_eq!(judgement.build().outcome, BuildOutcome::Built);
    assert_eq!(judgement.complexity(), None);
    assert_eq!(
        outcomes(&judgement),
        [("edge", &Outcome::Pass), ("many", &Outcome::Pass)]
    );
    assert_eq!(
        fs::read_to_string(evidence.path().join("build.log")).unwrap(),
        "$ python3 -m compileall -q .\nbuilt\n$ true\nbuilt\n"
    );
    assert!(!evidence.path().join("cases").exists());
}

#[tokio::test]
async fn sandboxes_are_offline_and_see_only_a_clean_copy_of_the_workspace() {
    let fixture = fixture();
    let workspace = fixture.workspace();
    write(
        &workspace,
        "__pycache__/main.cpython.pyc",
        "stale build output",
    );
    symlink(fixture.hidden().join("edge.out"), workspace.join("answers")).unwrap();
    let inputs = Arc::new(Mutex::new(Vec::new()));
    let backend = Scripted::new({
        let inputs = Arc::clone(&inputs);
        move |spec, argv, io| {
            let work = &spec.mounts[0].host;
            assert!(work.join("main.py").is_file());
            assert!(!work.join("__pycache__").exists());
            assert!(work.join("answers").is_symlink());
            if let Input::File(path) = &io.stdin {
                inputs.lock().unwrap().push(path.clone());
            }
            echo(spec, argv, io)
        }
    });
    let (judgement, _evidence) = fixture.judge(&backend).await;
    assert_eq!(judgement.unwrap().verdict(), Verdict::Pass);

    let hidden = fixture.hidden();
    let mut writable = Vec::new();
    for (spec, _) in backend.calls() {
        assert!(matches!(spec.network, Network::Offline));
        let [mount] = spec.mounts.as_slice() else {
            panic!("one mount, the workspace: {:?}", spec.mounts);
        };
        assert_eq!(mount.guest, Path::new(WORK));
        assert!(!hidden.starts_with(&mount.host) && !mount.host.starts_with(&hidden));
        assert_ne!(mount.host, workspace);
        writable.push(mount.writable);
    }
    assert_eq!(writable, [true, true, false, false]);
    assert_eq!(
        *inputs.lock().unwrap(),
        [hidden.join("edge.in"), hidden.join("many.in")]
    );
}

#[tokio::test]
async fn each_way_a_case_can_end_is_classified() {
    let over_cap = || {
        let mut flood = finished(Exit::Code(0), b"aaaa");
        flood.stdout = Collected {
            bytes: b"aaaa".to_vec(),
            total: STDOUT_CAP + 1,
            truncated: true,
        };
        Ok(flood)
    };
    type Ending = fn() -> Result<Finished, Fault>;
    let endings: [(Ending, Outcome, Failure); 7] = [
        (
            || Ok(finished(Exit::Code(0), b"nope\n")),
            Outcome::WrongAnswer,
            Failure::WrongAnswer,
        ),
        (over_cap, Outcome::OutputOverCap, Failure::OutputOverCap),
        (
            || Ok(finished(Exit::Code(3), b"nope\n")),
            Outcome::Abnormal(Abnormal::Crashed(Crash::Code(3))),
            Failure::Crash,
        ),
        (
            || Ok(finished(Exit::Signal(11), b"nope\n")),
            Outcome::Abnormal(Abnormal::Crashed(Crash::Signal(11))),
            Failure::Crash,
        ),
        (
            || Ok(finished(Exit::TimedOut, b"nope\n")),
            Outcome::Abnormal(Abnormal::TimedOut),
            Failure::Timeout,
        ),
        (
            || Ok(finished(Exit::OutOfMemory, b"nope\n")),
            Outcome::Abnormal(Abnormal::OutOfMemory),
            Failure::OutOfMemory,
        ),
        (
            || Err(eyre!("bwrap: cannot mount").into()),
            Outcome::Fault("sandbox fault: bwrap: cannot mount".into()),
            Failure::Environment,
        ),
    ];
    for (ending, outcome, failure) in endings {
        let fixture = fixture();
        let (judgement, evidence) = fixture.judge(&cases_end(move |_| ending())).await;
        let judgement = judgement.unwrap();
        assert_eq!(
            outcomes(&judgement),
            [("edge", &outcome), ("many", &outcome)]
        );
        assert_eq!(judgement.verdict(), Verdict::Fail(failure));

        let kept = fs::read(evidence.path().join("cases/edge.out")).ok();
        let expected = match outcome {
            Outcome::Fault(_) => None,
            Outcome::OutputOverCap => Some(b"aaaa".to_vec()),
            _ => Some(b"nope\n".to_vec()),
        };
        assert_eq!(kept, expected, "{outcome:?}");
    }
}

#[tokio::test]
async fn the_verdict_is_the_failure_with_the_most_blame() {
    type ByCase = fn(&str) -> Result<Finished, Fault>;
    let mixes: [(ByCase, Failure); 3] = [
        (
            |case| match case {
                "edge" => Ok(finished(Exit::Code(0), b"wrong\n")),
                _ => Ok(finished(Exit::Signal(6), b"")),
            },
            Failure::Crash,
        ),
        (
            |case| match case {
                "edge" => Ok(finished(Exit::Code(0), b"wrong\n")),
                _ => Ok(finished(Exit::TimedOut, b"")),
            },
            Failure::Timeout,
        ),
        (
            |case| match case {
                "edge" => Err(eyre!("no systemd").into()),
                _ => Ok(finished(Exit::Signal(6), b"")),
            },
            Failure::Environment,
        ),
    ];
    for (ending, failure) in mixes {
        let fixture = fixture();
        let (judgement, _evidence) = fixture.judge(&cases_end(ending)).await;
        assert_eq!(judgement.unwrap().verdict(), Verdict::Fail(failure));
    }
}

#[tokio::test]
async fn a_failed_build_stops_at_its_step_and_runs_no_case() {
    let fixture = fixture();
    let backend = Scripted::new(|_, argv, _| {
        let exit = if argv.ends_with(&["true".to_owned()]) {
            Exit::Code(2)
        } else {
            Exit::Code(0)
        };
        Ok(finished(exit, b"error: no\n"))
    });
    let (judgement, evidence) = fixture.judge(&backend).await;
    let judgement = judgement.unwrap();

    assert_eq!(
        judgement.build().outcome,
        BuildOutcome::Failed {
            step: 1,
            ending: Abnormal::Crashed(Crash::Code(2)),
        }
    );
    assert!(judgement.cases().is_empty());
    assert_eq!(judgement.verdict(), Verdict::Fail(Failure::BuildError));
    assert_eq!(backend.calls().len(), 2);
    assert_eq!(
        fs::read_to_string(evidence.path().join("build.log")).unwrap(),
        "$ python3 -m compileall -q .\nerror: no\n$ true\nerror: no\n"
    );
}

#[tokio::test]
async fn a_build_the_sandbox_cannot_run_is_an_environment_fault() {
    let fixture = fixture();
    let backend = Scripted::new(|_, _, _| Err(eyre!("bwrap: no user namespaces").into()));
    let (judgement, _evidence) = fixture.judge(&backend).await;
    let judgement = judgement.unwrap();

    let BuildOutcome::Fault { step: 0, message } = &judgement.build().outcome else {
        panic!("{:?}", judgement.build());
    };
    assert!(message.contains("no user namespaces"), "{message}");
    assert_eq!(judgement.verdict(), Verdict::Fail(Failure::Environment));
    assert_eq!(backend.calls().len(), 1);
}

#[tokio::test]
async fn a_case_killed_by_gungnir_leaves_no_judgement() {
    let fixture = fixture();
    let backend = cases_end(|_| Ok(finished(Exit::Killed, b"")));
    let (judgement, _evidence) = fixture.judge(&backend).await;
    let error = judgement.unwrap_err();
    assert!(format!("{error:#}").contains("interrupted"), "{error:#}");
}
