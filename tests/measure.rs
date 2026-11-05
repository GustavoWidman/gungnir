//! Measuring against real bubblewrap, systemd, hyperfine and toolchains, on copies of the
//! word-freq Python reference. Run them in the dev shell with `cargo test -- --ignored`. The
//! tests compare in two lanes on two CPUs, as the measure phase does, so two comparisons run at
//! once and never on one CPU.

use std::{
    collections::BTreeMap,
    fs,
    num::NonZeroU32,
    path::Path,
    sync::{LazyLock, Mutex},
};

use eyre::Result;
use gungnir::{
    id::{LanguageId, WorkloadId},
    language::Language,
    measure::{self, Comparison, Failure, Outcome, Program, Protocol, Settings},
    sandbox::Bubblewrap,
    scenario::Workload,
    stop::Stop,
    store::Digest,
    toolchains::Toolchains,
};
use tempfile::TempDir;
use tokio::sync::{Semaphore, SemaphorePermit};

/// The CPUs the comparisons run on: the last two this process may use, or the one it may use.
fn lane_cpus() -> Vec<u32> {
    let cpus = std::thread::available_parallelism().map_or(1, |cpus| cpus.get());
    let cpus = u32::try_from(cpus).expect("a CPU count");
    (cpus.saturating_sub(2)..cpus).collect()
}

/// The CPUs no test holds.
static CPUS: LazyLock<Mutex<Vec<u32>>> = LazyLock::new(|| Mutex::new(lane_cpus()));
static FREE: LazyLock<Semaphore> = LazyLock::new(|| Semaphore::new(lane_cpus().len()));

/// CPUs held for the comparisons of one test, each the only one running on its CPU.
struct Lanes {
    cores: Vec<u32>,
    _permit: SemaphorePermit<'static>,
}

impl Lanes {
    /// Waits until `count` CPUs are free and holds them.
    async fn hold(count: u32) -> Self {
        let total = lane_cpus().len();
        assert!(
            usize::try_from(count).is_ok_and(|count| count <= total),
            "this process may use {total} CPUs, not {count}"
        );
        let permit = FREE
            .acquire_many(count)
            .await
            .expect("the semaphore is open");
        let mut free = CPUS.lock().expect("the CPUs");
        let at = free.len() - usize::try_from(count).expect("a count");
        Self {
            cores: free.split_off(at),
            _permit: permit,
        }
    }
}

impl Drop for Lanes {
    fn drop(&mut self) {
        CPUS.lock().expect("the CPUs").append(&mut self.cores);
    }
}

fn settings(core: u32) -> Settings {
    Settings {
        core,
        protocol: Protocol {
            blocks: NonZeroU32::new(2).expect("non-zero"),
            runs: NonZeroU32::new(3).expect("non-zero"),
            warmup: 1,
            clock_mhz: None,
        },
        memory: 512 << 20,
    }
}

/// Whether the kernel lets hyperfine count user-space instructions.
fn counters_allowed() -> bool {
    fs::read_to_string("/proc/sys/kernel/perf_event_paranoid")
        .ok()
        .and_then(|level| level.trim().parse::<i32>().ok())
        .is_some_and(|level| level <= 2)
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

    /// Builds both workspaces and compares them on a CPU no other comparison holds, returning
    /// the evidence directory alongside.
    async fn compare(&self, reference: &Path, candidate: &Path) -> Result<(Comparison, TempDir)> {
        let lane = Lanes::hold(1).await;
        self.compare_on(lane.cores[0], reference, candidate).await
    }

    /// The same, with every run pinned to `core`, which the caller holds.
    async fn compare_on(
        &self,
        core: u32,
        reference: &Path,
        candidate: &Path,
    ) -> Result<(Comparison, TempDir)> {
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
            &settings(core),
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
        let summary = timed.summary();
        let ratio = summary.ratio;
        let (low, high) = ratio.interval.expect("two blocks give an interval");
        assert!(
            low.is_finite() && high.is_finite() && low <= ratio.geomean && ratio.geomean <= high
        );
        let peaks = summary.peak_memory;
        for peak in [peaks.reference, peaks.candidate] {
            let peak = peak.expect("GNU time reports every output check");
            assert!((1 << 20..512 << 20).contains(&peak), "{peaks:?}");
        }
        if counters_allowed() {
            let instructions = summary.instructions.expect("counted instructions");
            assert!((0.95..1.05).contains(&instructions), "{instructions}");
        } else {
            assert_eq!(summary.instructions, None);
        }
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
async fn a_candidate_that_holds_more_memory_peaks_higher() -> Result<()> {
    let rig = Rig::new()?;
    let reference = rig.workspace("reference", unchanged)?;
    let hoarder = rig.workspace("hoarder", |source| {
        format!("hoard = bytearray(64 << 20)\n{source}")
    })?;
    let (comparison, _evidence) = rig.compare(&reference, &hoarder).await?;

    let peaks = timed(&comparison, "short").peak_memory;
    let (reference, hoarder) = (
        peaks.reference.expect("a reference peak"),
        peaks.candidate.expect("a candidate peak"),
    );
    let grown = hoarder - reference;
    assert!((60 << 20..72 << 20).contains(&grown), "{peaks:?}");
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

/// A workspace that prints the CPUs it may run on, and one that prints `[core]`.
fn affinity_and_expected(rig: &Rig, core: u32) -> Result<(std::path::PathBuf, std::path::PathBuf)> {
    let affinity = rig.workspace(&format!("affinity-{core}"), |_| {
        "import os\nprint(sorted(os.sched_getaffinity(0)))\n".to_owned()
    })?;
    let expected = rig.workspace(&format!("expected-{core}"), |_| {
        format!("print([{core}])\n")
    })?;
    Ok((affinity, expected))
}

#[tokio::test]
#[ignore = "needs bwrap and GUNGNIR_TOOLCHAINS"]
async fn every_run_is_pinned_to_the_configured_core() -> Result<()> {
    let rig = Rig::new()?;
    let lane = Lanes::hold(1).await;
    let core = lane.cores[0];
    let (affinity, expected) = affinity_and_expected(&rig, core)?;
    let (comparison, _evidence) = rig.compare_on(core, &affinity, &expected).await?;

    timed(&comparison, "short");
    timed(&comparison, "long");
    assert_eq!(comparison.machine.core, core);
    Ok(())
}

#[tokio::test]
#[ignore = "needs bwrap and GUNGNIR_TOOLCHAINS"]
async fn two_comparisons_at_once_each_stay_on_their_own_core() -> Result<()> {
    let rig = Rig::new()?;
    let lanes = Lanes::hold(2).await;
    let [first, second] = lanes.cores[..] else {
        panic!("two CPUs to hold, got {:?}", lanes.cores);
    };
    let (first_affinity, first_expected) = affinity_and_expected(&rig, first)?;
    let (second_affinity, second_expected) = affinity_and_expected(&rig, second)?;
    let (on_first, on_second) = tokio::join!(
        rig.compare_on(first, &first_affinity, &first_expected),
        rig.compare_on(second, &second_affinity, &second_expected),
    );
    for ((comparison, _evidence), core) in [(on_first?, first), (on_second?, second)] {
        timed(&comparison, "short");
        timed(&comparison, "long");
        assert_eq!(comparison.machine.core, core);
    }
    Ok(())
}
