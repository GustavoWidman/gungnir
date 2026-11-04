//! Measuring against real bubblewrap, systemd, hyperfine and toolchains, on copies of the
//! word-freq Python reference. Run them in the dev shell with `cargo test -- --ignored`.

use std::{collections::BTreeMap, fs, num::NonZeroU32, path::Path};

use eyre::Result;
use gungnir::{
    id::{LanguageId, WorkloadId},
    language::Language,
    measure::{self, Comparison, Failure, Outcome, Program, Settings},
    sandbox::Bubblewrap,
    scenario::Workload,
    stop::Stop,
    store::Digest,
    toolchains::Toolchains,
};
use tempfile::TempDir;
use tokio::sync::Mutex;

/// The last CPU this process may use, which every run is pinned to.
fn core() -> u32 {
    let cpus = std::thread::available_parallelism().map_or(1, |cpus| cpus.get());
    u32::try_from(cpus - 1).expect("a CPU index")
}

fn settings() -> Settings {
    Settings {
        core: core(),
        blocks: NonZeroU32::new(2).expect("non-zero"),
        runs: NonZeroU32::new(3).expect("non-zero"),
        warmup: 1,
        memory: 512 << 20,
    }
}

/// The Python language, a short and a longer word-freq workload, and the real backend.
struct Rig {
    dir: TempDir,
    language: Language,
    toolchains: Toolchains,
    backend: Bubblewrap,
    workloads: BTreeMap<WorkloadId, Workload>,
}

impl Rig {
    fn new() -> Result<Self> {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut languages = Language::load_all(&repo.join("languages"))?;
        let language = languages
            .remove(&"python".parse::<LanguageId>()?)
            .expect("python");
        let dir = TempDir::new()?;
        let words = ["the", "cat", "and", "dog", "sat", "on", "a", "mat"];
        let long: Vec<&str> = (0..20_000).map(|i| words[i * 7 % words.len()]).collect();
        let mut workloads = BTreeMap::new();
        for (name, text) in [
            ("short", "3\nThe cat and the dog.\n".to_owned()),
            ("long", format!("5\n{}\n", long.join(" "))),
        ] {
            let path = dir.path().join(format!("{name}.in"));
            fs::write(&path, &text)?;
            let digest = Digest::of_bytes(text.as_bytes());
            workloads.insert(name.parse()?, Workload { path, digest });
        }
        let toolchains = Toolchains::from_env()?;
        let backend = Bubblewrap::new(toolchains.base().clone())?;
        Ok(Self {
            dir,
            language,
            toolchains,
            backend,
            workloads,
        })
    }

    /// A copy of the word-freq Python reference whose `main.py` is `edit` of the original.
    fn workspace(&self, name: &str, edit: impl Fn(&str) -> String) -> Result<std::path::PathBuf> {
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("scenarios/word-freq/reference/python");
        let target = self.dir.path().join(name);
        fs::create_dir_all(&target)?;
        for file in ["main.py", "pyproject.toml"] {
            fs::copy(source.join(file), target.join(file))?;
        }
        let main = target.join("main.py");
        fs::write(&main, edit(&fs::read_to_string(&main)?))?;
        Ok(target)
    }

    /// Builds both workspaces and compares them, returning the evidence directory alongside.
    /// Comparisons run one at a time, as the measure phase runs them, since every test pins
    /// its runs to the same CPU.
    async fn compare(&self, reference: &Path, candidate: &Path) -> Result<(Comparison, TempDir)> {
        static ONE_AT_A_TIME: Mutex<()> = Mutex::const_new(());
        let _turn = ONE_AT_A_TIME.lock().await;
        let stop = Stop::default();
        let (language, toolchains) = (&self.language, &self.toolchains);
        let reference =
            Program::build(language, toolchains, reference, &self.backend, &stop).await?;
        let candidate =
            Program::build(language, toolchains, candidate, &self.backend, &stop).await?;
        let evidence = TempDir::new()?;
        let comparison = measure::compare(
            &reference,
            &candidate,
            &self.workloads,
            &settings(),
            &self.backend,
            &stop,
            evidence.path(),
        )
        .await?;
        Ok((comparison, evidence))
    }
}

fn unchanged(source: &str) -> String {
    source.to_owned()
}

fn timed(comparison: &Comparison, workload: &str) -> measure::Timed {
    match &comparison.workloads[&workload.parse().expect("workload id")] {
        Outcome::Timed(timed) => timed.clone(),
        outcome => panic!("{workload} was not timed: {outcome:?}"),
    }
}

#[tokio::test]
#[ignore = "needs bwrap and GUNGNIR_TOOLCHAINS"]
async fn two_builds_of_the_reference_are_timed_in_every_block() -> Result<()> {
    let rig = Rig::new()?;
    let reference = rig.workspace("reference", unchanged)?;
    let (comparison, evidence) = rig.compare(&reference, &reference).await?;

    for workload in ["long", "short"] {
        let timed = timed(&comparison, workload);
        assert_eq!(timed.blocks.len(), 2);
        for block in &timed.blocks {
            for sample in block.reference.iter().chain(&block.candidate) {
                assert!(sample.wall > 0.0 && sample.wall < 5.0, "{sample:?}");
                assert!(sample.peak_memory > 1 << 20, "{sample:?}");
            }
            assert_eq!((block.reference.len(), block.candidate.len()), (3, 3));
        }
        let ratio = timed.summary().ratio;
        let (low, high) = ratio.interval.expect("two blocks give an interval");
        assert!(
            low.is_finite() && high.is_finite() && low <= ratio.geomean && ratio.geomean <= high
        );
        for name in ["0-reference", "0-candidate", "1-candidate", "1-reference"] {
            let export = evidence
                .path()
                .join(format!("hyperfine/{workload}/{name}.json"));
            assert!(fs::read_to_string(&export)?.contains("\"schema_version\": 2"));
        }
    }
    Ok(())
}

#[tokio::test]
#[ignore = "needs bwrap and GUNGNIR_TOOLCHAINS"]
async fn a_candidate_that_sleeps_is_measured_slower() -> Result<()> {
    let rig = Rig::new()?;
    let reference = rig.workspace("reference", unchanged)?;
    let slow = rig.workspace("slow", |source| {
        format!("import time\ntime.sleep(0.25)\n{source}")
    })?;
    let (comparison, _evidence) = rig.compare(&reference, &slow).await?;

    let summary = timed(&comparison, "short").summary();
    let (low, _) = summary.ratio.interval.expect("an interval");
    assert!(summary.ratio.geomean > 2.0, "{summary:?}");
    assert!(low > 1.5, "{summary:?}");
    assert!(summary.candidate.min > 0.25, "{summary:?}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs bwrap and GUNGNIR_TOOLCHAINS"]
async fn a_candidate_that_prints_something_else_is_not_timed() -> Result<()> {
    let rig = Rig::new()?;
    let reference = rig.workspace("reference", unchanged)?;
    let extra = rig.workspace("extra", |source| format!("{source}\nprint(\"extra\")\n"))?;
    let (comparison, evidence) = rig.compare(&reference, &extra).await?;

    for (id, outcome) in &comparison.workloads {
        assert_eq!(
            outcome,
            &Outcome::CandidateFailed(Failure::WrongOutput),
            "{id}"
        );
    }
    assert!(!evidence.path().join("hyperfine").exists());
    Ok(())
}

#[tokio::test]
#[ignore = "needs bwrap and GUNGNIR_TOOLCHAINS"]
async fn every_run_is_pinned_to_the_configured_core() -> Result<()> {
    let rig = Rig::new()?;
    let affinity = rig.workspace("affinity", |_| {
        "import os\nprint(sorted(os.sched_getaffinity(0)))\n".to_owned()
    })?;
    let expected = rig.workspace("expected", |_| format!("print([{}])\n", core()))?;
    let (comparison, _evidence) = rig.compare(&affinity, &expected).await?;

    timed(&comparison, "short");
    timed(&comparison, "long");
    Ok(())
}
