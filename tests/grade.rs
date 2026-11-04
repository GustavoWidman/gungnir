//! Grading and workload generation against real bubblewrap, systemd, toolchains and lizard, on
//! copies of the repository's own scenarios and references. Run them in the dev shell with
//! `cargo test -- --ignored`.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use eyre::Result;
use gungnir::{
    grade::{self, Abnormal, BuildOutcome, Crash, Failure, Judgement, Outcome, Verdict},
    id::{LanguageId, ScenarioId},
    language::Language,
    sandbox::Bubblewrap,
    scenario::Scenario,
    stop::Stop,
    store::Digest,
    toolchains::Toolchains,
    workload::{self, Ensured},
};
use tempfile::TempDir;

/// The hidden cases of word-freq the fixture keeps: an empty text, a tie, and 500 words.
const CASES: [&str; 3] = ["001-empty-text", "006-all-ties", "022-all-distinct"];

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// word-freq cut down to three hidden cases with a 2s and 256MiB limit, and the repository's
/// languages and toolchains.
struct Rig {
    dir: TempDir,
    languages: BTreeMap<LanguageId, Language>,
    scenario: Scenario,
    toolchains: Toolchains,
    backend: Bubblewrap,
}

impl Rig {
    fn new() -> Result<Self> {
        let dir = TempDir::new()?;
        let source = repo().join("scenarios/word-freq");
        let target = dir.path().join("scenarios/word-freq");
        for part in ["examples", "reference"] {
            copy_tree(&source.join(part), &target.join(part))?;
        }
        for file in ["spec.md", "workloads.sha256"] {
            fs::copy(source.join(file), target.join(file))?;
        }
        fs::create_dir_all(target.join("hidden"))?;
        for case in CASES {
            for extension in ["in", "out"] {
                let name = format!("{case}.{extension}");
                fs::copy(
                    source.join("hidden").join(&name),
                    target.join("hidden").join(name),
                )?;
            }
        }
        let manifest = fs::read_to_string(source.join("scenario.toml"))?
            .replace("time = \"10s\"", "time = \"2s\"")
            .replace("memory = \"1GiB\"", "memory = \"256MiB\"");
        fs::write(target.join("scenario.toml"), manifest)?;

        let languages = Language::load_all(&repo().join("languages"))?;
        let mut scenarios = Scenario::load_all(&dir.path().join("scenarios"), &languages)?;
        let scenario = scenarios
            .remove(&"word-freq".parse::<ScenarioId>()?)
            .expect("word-freq");
        assert_eq!(scenario.limits().time().as_secs(), 2, "the fixture's limit");
        let toolchains = Toolchains::from_env()?;
        let backend = Bubblewrap::new(toolchains.base().clone())?;
        Ok(Self {
            dir,
            languages,
            scenario,
            toolchains,
            backend,
        })
    }

    /// A copy of the `language` reference with `edit` applied to the source file at `source`.
    fn broken(
        &self,
        language: &str,
        source: &str,
        edit: impl Fn(&str) -> String,
    ) -> Result<TempDir> {
        let workspace = TempDir::new()?;
        let reference = self
            .dir
            .path()
            .join("scenarios/word-freq/reference")
            .join(language);
        copy_tree(&reference, workspace.path())?;
        let path = workspace.path().join(source);
        fs::write(&path, edit(&fs::read_to_string(&path)?))?;
        Ok(workspace)
    }

    async fn judge(&self, language: &str, workspace: &Path) -> Result<(Judgement, TempDir)> {
        let evidence = TempDir::new()?;
        let language = &self.languages[&language.parse::<LanguageId>()?];
        let judgement = grade::judge(
            language,
            &self.toolchains,
            workspace,
            &self.scenario,
            &self.backend,
            &Stop::default(),
            evidence.path(),
        )
        .await?;
        Ok((judgement, evidence))
    }
}

/// Replaces `from`, which the reference must hold, with `to`.
fn replace(from: &'static str, to: &'static str) -> impl Fn(&str) -> String {
    move |source| {
        assert!(source.contains(from), "the reference lacks {from:?}");
        source.replace(from, to)
    }
}

fn outcomes(judgement: &Judgement) -> Vec<&Outcome> {
    judgement
        .cases()
        .values()
        .map(|case| &case.outcome)
        .collect()
}

#[tokio::test]
#[ignore = "needs bwrap, lizard and GUNGNIR_TOOLCHAINS"]
async fn references_pass_and_their_complexity_is_measured() -> Result<()> {
    let rig = Rig::new()?;
    for (language, source, function) in [
        ("python", "main.py", "main"),
        ("rust", "src/main.rs", "main"),
    ] {
        let reference = rig
            .dir
            .path()
            .join("scenarios/word-freq/reference")
            .join(language);
        let (judgement, evidence) = rig.judge(language, &reference).await?;
        let log = fs::read_to_string(evidence.path().join("build.log"))?;
        assert_eq!(judgement.verdict(), Verdict::Pass, "{language}: {log}");
        assert_eq!(outcomes(&judgement), [&Outcome::Pass; 3]);

        let complexity = judgement.complexity().expect("lizard measures it");
        let sources: Vec<&str> = complexity.files.keys().map(String::as_str).collect();
        assert_eq!(sources, [source], "{language}");
        let file = &complexity.files[source];
        assert!(file.nloc > 0);
        let main = file
            .functions
            .iter()
            .find(|candidate| candidate.name == function)
            .expect("main is measured");
        assert!(
            main.ccn >= 1 && main.nloc > 0 && main.tokens > 0,
            "{main:?}"
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "needs bwrap, lizard and GUNGNIR_TOOLCHAINS"]
async fn broken_references_are_classified_by_how_they_fail() -> Result<()> {
    let rig = Rig::new()?;
    let main = "if __name__ == \"__main__\":\n    main()\n";
    let breaks: [(&str, &'static str, Outcome, Failure); 6] = [
        (
            "an extra line",
            "if __name__ == \"__main__\":\n    main()\n    print(\"extra\")\n",
            Outcome::WrongAnswer,
            Failure::WrongAnswer,
        ),
        (
            "an exit code",
            "raise SystemExit(3)\n",
            Outcome::Abnormal(Abnormal::Crashed(Crash::Code(3))),
            Failure::Crash,
        ),
        (
            "a segfault",
            "import os\nos.kill(os.getpid(), 11)\n",
            Outcome::Abnormal(Abnormal::Crashed(Crash::Signal(11))),
            Failure::Crash,
        ),
        (
            "an endless loop",
            "while True:\n    pass\n",
            Outcome::Abnormal(Abnormal::TimedOut),
            Failure::Timeout,
        ),
        (
            "a gigabyte",
            "blob = b\"x\" * (1 << 30)\n",
            Outcome::Abnormal(Abnormal::OutOfMemory),
            Failure::OutOfMemory,
        ),
        (
            "a flood",
            "import sys\nsys.stdout.write(\"x\" * (9 << 20))\n",
            Outcome::OutputOverCap,
            Failure::OutputOverCap,
        ),
    ];
    for (what, ending, outcome, failure) in breaks {
        let workspace = rig.broken("python", "main.py", replace(main, ending))?;
        let (judgement, evidence) = rig.judge("python", workspace.path()).await?;
        assert_eq!(judgement.build().outcome, BuildOutcome::Built, "{what}");
        assert_eq!(outcomes(&judgement), [&outcome; 3], "{what}");
        assert_eq!(judgement.verdict(), Verdict::Fail(failure), "{what}");
        for case in CASES {
            let kept = evidence.path().join("cases").join(format!("{case}.out"));
            assert!(kept.is_file(), "{what}: no {}", kept.display());
        }
        if outcome == Outcome::WrongAnswer {
            let kept = fs::read_to_string(evidence.path().join("cases/006-all-ties.out"))?;
            assert!(kept.ends_with("extra\n"), "{kept}");
        }
    }
    Ok(())
}

#[tokio::test]
#[ignore = "needs bwrap, lizard and GUNGNIR_TOOLCHAINS"]
async fn a_reference_that_does_not_compile_is_a_build_error() -> Result<()> {
    let rig = Rig::new()?;
    let workspace = rig.broken("rust", "src/main.rs", |source| format!("{source}\nfn {{\n"))?;
    let (judgement, evidence) = rig.judge("rust", workspace.path()).await?;

    let BuildOutcome::Failed { step: 0, ending } = judgement.build().outcome else {
        panic!("{:?}", judgement.build());
    };
    assert_eq!(ending, Abnormal::Crashed(Crash::Code(101)));
    assert!(judgement.cases().is_empty());
    assert_eq!(judgement.verdict(), Verdict::Fail(Failure::BuildError));
    let log = fs::read_to_string(evidence.path().join("build.log"))?;
    assert!(
        log.starts_with("$ cargo build --release --offline --quiet\n"),
        "{log}"
    );
    assert!(log.contains("could not compile `solution`"), "{log}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs bwrap, lizard and GUNGNIR_TOOLCHAINS"]
async fn deleted_workloads_regenerate_to_their_pinned_digests() -> Result<()> {
    let rig = Rig::new()?;
    let dir = rig.dir.path().join("scenarios/word-freq");
    copy_tree(&repo().join("scenarios/word-freq/gen"), &dir.join("gen"))?;
    let manifest = fs::read(dir.join("workloads.sha256"))?;
    let stop = Stop::default();
    let ensure = async |scenario: &Scenario| {
        workload::ensure(scenario, &rig.toolchains, &rig.backend, &stop).await
    };

    assert_eq!(ensure(&rig.scenario).await?, Ensured::Regenerated);
    for (id, workload) in rig.scenario.workloads() {
        let digest = Digest::of_bytes(&fs::read(&workload.path)?);
        assert_eq!(digest, workload.digest, "{id}");
    }
    assert_eq!(fs::read(dir.join("workloads.sha256"))?, manifest);
    assert_eq!(ensure(&rig.scenario).await?, Ensured::Present);

    let pinned = Digest::of_bytes(b"not the zipf workload");
    let edited = String::from_utf8(manifest)?
        .lines()
        .map(|line| match line.split_once("  ") {
            Some((_, "zipf.in")) => format!("{pinned}  zipf.in\n"),
            _ => format!("{line}\n"),
        })
        .collect::<String>();
    fs::write(dir.join("workloads.sha256"), &edited)?;
    let scenario = Scenario::load_all(&rig.dir.path().join("scenarios"), &rig.languages)?
        .remove(&"word-freq".parse::<ScenarioId>()?)
        .expect("word-freq");
    let message = format!("{:#}", ensure(&scenario).await.unwrap_err());
    let zipf = &rig.scenario.workloads()[&"zipf".parse()?];
    for needle in [
        "workloads/zipf.in".to_owned(),
        pinned.to_string(),
        zipf.digest.to_string(),
    ] {
        assert!(
            message.contains(&needle),
            "{needle} missing from: {message}"
        );
    }
    assert_eq!(fs::read_to_string(dir.join("workloads.sha256"))?, edited);
    Ok(())
}
