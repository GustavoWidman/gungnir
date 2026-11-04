//! Workload files: timing inputs that each scenario's `gen/gen.py workloads` writes on demand,
//! pinned byte for byte by its `workloads.sha256`.
use crate::{
    sandbox::{Backend, Input, Io, Kill, Limits, Mount, Network, Output, Spec},
    scenario::Scenario,
    stop::Stop,
    store::Digest,
    toolchains::Toolchains,
};
use eyre::{Result, WrapErr, ensure, eyre};
use std::{
    ffi::OsString,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    time::Duration,
};

/// The generator is Python, so it runs on the python language's toolchain.
const GENERATOR_LANGUAGE: &str = "python";
const MANIFEST: &str = "workloads.sha256";
/// Where the scenario directory appears inside the generator's sandbox.
const GUEST: &str = "/scenario";
const LIMITS: Limits = Limits {
    wall: Duration::from_secs(600),
    memory: 4 << 30,
    tasks: 64,
    cpu: None,
};
const STDERR_CAP: u64 = 64 << 10;

/// What [`ensure`] had to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ensured {
    /// Every workload was in place with its pinned digest.
    Present,
    /// A workload was missing or differed, so the generator rewrote them all, and each now has
    /// its pinned digest.
    Regenerated,
}

/// Makes every workload of `scenario` exist with the digest `workloads.sha256` pins, running
/// the scenario's trusted generator in an offline sandbox when one is missing or differs. A
/// regenerated workload that still differs is an error naming the file and both digests.
pub async fn ensure(
    scenario: &Scenario,
    toolchains: &Toolchains,
    backend: &dyn Backend,
    stop: &Stop,
) -> Result<Ensured> {
    let mut pinned = true;
    for workload in scenario.workloads().values() {
        pinned &= digest(&workload.path)? == Some(workload.digest);
    }
    if pinned {
        return Ok(Ensured::Present);
    }
    generate(scenario, toolchains, backend, stop).await?;
    for workload in scenario.workloads().values() {
        let path = workload.path.display();
        let digest = digest(&workload.path)?
            .ok_or_else(|| eyre!("gen/gen.py workloads did not write {path}"))?;
        ensure!(
            digest == workload.digest,
            "{path} regenerated with SHA-256 {digest}, but {MANIFEST} pins {}",
            workload.digest
        );
    }
    Ok(Ensured::Regenerated)
}

fn digest(path: &Path) -> Result<Option<Digest>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(Digest::of_bytes(&bytes))),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).wrap_err_with(|| format!("reading {}", path.display())),
    }
}

/// Runs `gen/gen.py workloads` with only `workloads/` writable. The generator also rewrites
/// `workloads.sha256`; it writes a scratch copy instead, so the pins stay as committed.
async fn generate(
    scenario: &Scenario,
    toolchains: &Toolchains,
    backend: &dyn Backend,
    stop: &Stop,
) -> Result<()> {
    let (dir, workloads) = dirs(scenario)?;
    fs::create_dir_all(&workloads).wrap_err_with(|| format!("creating {}", workloads.display()))?;
    let scratch = tempfile::tempdir().wrap_err("creating generator scratch")?;
    let manifest = scratch.path().join(MANIFEST);
    fs::copy(dir.join(MANIFEST), &manifest)
        .wrap_err_with(|| format!("copying {}", dir.join(MANIFEST).display()))?;

    let python = toolchains.language(GENERATOR_LANGUAGE)?;
    let guest = Path::new(GUEST);
    let spec = Spec {
        network: Network::Offline,
        closure: python.closure().clone(),
        mounts: vec![
            Mount::read_only(&dir, guest),
            Mount::writable(&workloads, guest.join("workloads")),
            Mount::writable(&manifest, guest.join(MANIFEST)),
        ],
        path: vec![python.bin(), toolchains.base().bin()],
        env: Vec::new(),
        cwd: guest.into(),
        limits: LIMITS,
    };
    let argv = ["python3", "gen/gen.py", "workloads"].map(OsString::from);
    let io = Io {
        stdin: Input::Null,
        stdout: Output::Capture { cap: 0 },
        stderr: Output::Capture { cap: STDERR_CAP },
        kill: Kill::default(),
    };
    let finished = backend
        .exec(&spec, &argv, io, stop)
        .await
        .wrap_err_with(|| format!("running gen/gen.py workloads in {}", dir.display()))?;
    ensure!(
        finished.exit.success(),
        "gen/gen.py workloads in {} ended with {:?}: {}",
        dir.display(),
        finished.exit,
        String::from_utf8_lossy(&finished.stderr.bytes).trim()
    );
    Ok(())
}

/// The scenario directory and its `workloads/`, the directory every workload path is in.
fn dirs(scenario: &Scenario) -> Result<(PathBuf, PathBuf)> {
    let workloads = scenario
        .workloads()
        .values()
        .find_map(|workload| workload.path.parent())
        .ok_or_else(|| eyre!("scenario {} lists no workload", scenario.id()))?;
    let dir = workloads
        .parent()
        .ok_or_else(|| eyre!("{} has no parent", workloads.display()))?;
    Ok((dir.to_owned(), workloads.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        grade::fake::{self, Scripted, finished},
        id::ScenarioId,
        language::Language,
        sandbox::{Exit, Finished},
        testing::{self, id, write},
    };
    use tempfile::TempDir;

    /// The `echo` scenario, whose one workload `large.in` is pinned to `x\n`.
    struct Fixture {
        root: TempDir,
        toolchains: Toolchains,
    }

    fn fixture() -> Fixture {
        let root = TempDir::new().unwrap();
        testing::language(root.path(), "python");
        testing::scenario(root.path(), "echo", &["python"]);
        let toolchains = fake::toolchains(&root.path().join("toolchains"), &["python"]);
        Fixture { root, toolchains }
    }

    impl Fixture {
        fn dir(&self) -> PathBuf {
            self.root.path().join("scenarios/echo")
        }

        async fn ensure(&self, backend: &Scripted) -> Result<Ensured> {
            let languages = Language::load_all(&self.root.path().join("languages")).unwrap();
            let scenarios =
                Scenario::load_all(&self.root.path().join("scenarios"), &languages).unwrap();
            let scenario = &scenarios[&id::<ScenarioId>("echo")];
            ensure(scenario, &self.toolchains, backend, &Stop::default()).await
        }
    }

    /// A generator that writes `workload` as `large.in` and rewrites the manifest it sees.
    fn generator(workload: &'static str) -> Scripted {
        Scripted::new(move |spec, _, _| {
            let host = |guest: &str| {
                let mount = spec
                    .mounts
                    .iter()
                    .find(|mount| mount.guest == Path::new(guest));
                mount.expect("mounted").host.clone()
            };
            fs::write(host("/scenario/workloads").join("large.in"), workload).unwrap();
            fs::write(host("/scenario/workloads.sha256"), "rewritten\n").unwrap();
            Ok(finished(Exit::Code(0), b""))
        })
    }

    #[tokio::test]
    async fn pinned_workloads_are_left_alone() {
        let fixture = fixture();
        let backend = generator("x\n");
        assert_eq!(fixture.ensure(&backend).await.unwrap(), Ensured::Present);
        assert!(backend.calls().is_empty());
    }

    #[tokio::test]
    async fn missing_or_differing_workloads_are_regenerated_offline() {
        for workload in [None, Some("stale\n")] {
            let fixture = fixture();
            let dir = fixture.dir();
            let manifest = fs::read(dir.join(MANIFEST)).unwrap();
            fs::remove_dir_all(dir.join("workloads")).unwrap();
            if let Some(stale) = workload {
                write(&dir, "workloads/large.in", stale);
            }
            let backend = generator("x\n");

            assert_eq!(
                fixture.ensure(&backend).await.unwrap(),
                Ensured::Regenerated
            );
            assert_eq!(fs::read(dir.join("workloads/large.in")).unwrap(), b"x\n");
            assert_eq!(fs::read(dir.join(MANIFEST)).unwrap(), manifest);
            let [(spec, argv)] = backend.calls().try_into().unwrap();
            assert_eq!(argv, ["python3", "gen/gen.py", "workloads"]);
            assert!(matches!(spec.network, Network::Offline));
            assert_eq!(spec.mounts[0], Mount::read_only(&dir, GUEST));
            assert_eq!(spec.cwd, Path::new(GUEST));
        }
    }

    #[tokio::test]
    async fn a_regeneration_that_misses_its_pin_names_the_file_and_both_digests() {
        let fixture = fixture();
        fs::remove_file(fixture.dir().join("workloads/large.in")).unwrap();
        let error = fixture.ensure(&generator("y\n")).await.unwrap_err();
        let message = format!("{error:#}");
        for needle in [
            "workloads/large.in".to_owned(),
            Digest::of_bytes(b"y\n").to_string(),
            Digest::of_bytes(b"x\n").to_string(),
        ] {
            assert!(
                message.contains(&needle),
                "{needle} missing from: {message}"
            );
        }
        let manifest = fs::read_to_string(fixture.dir().join(MANIFEST)).unwrap();
        assert!(manifest.contains(&Digest::of_bytes(b"x\n").to_string()));
    }

    #[tokio::test]
    async fn a_failing_generator_is_an_error_with_its_stderr() {
        let fixture = fixture();
        fs::remove_file(fixture.dir().join("workloads/large.in")).unwrap();
        let backend = Scripted::new(|_, _, _| {
            let mut failed: Finished = finished(Exit::Code(1), b"");
            failed.stderr.bytes = b"Traceback: no module named numpy\n".to_vec();
            Ok(failed)
        });
        let message = format!("{:#}", fixture.ensure(&backend).await.unwrap_err());
        assert!(message.contains("no module named numpy"), "{message}");
    }
}
