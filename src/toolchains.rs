//! The sandbox environments that `nix/toolchains.nix` builds, loaded from
//! `$GUNGNIR_TOOLCHAINS`.

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    fs, io,
    path::{Path, PathBuf},
    time::Duration,
};

use eyre::{Context, ContextCompat, Result, bail, ensure};

use crate::{
    sandbox::{Backend, HOME, Input, Io, Kill, Limits, Network, Output, Spec},
    secret::EnvValue,
    stop::Stop,
};

const VARIABLE: &str = "GUNGNIR_TOOLCHAINS";
const STORE: &str = "/nix/store";
/// What a tool may use to say what it is.
const PROBE_LIMITS: Limits = Limits {
    wall: Duration::from_secs(60),
    memory: 1 << 30,
    tasks: 256,
    cpu: None,
};
const PROBE_OUTPUT: u64 = 64 << 10;

/// Store paths a sandbox binds read-only. Never holds a flake source path, because the flake's
/// copy of this repository in the store contains every scenario's hidden tests.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Closure(BTreeSet<PathBuf>);

impl Closure {
    /// Parses a closureInfo `store-paths` list.
    pub(crate) fn parse(list: &str) -> Result<Self> {
        let paths = list
            .lines()
            .filter(|line| !line.is_empty())
            .map(store_path)
            .collect::<Result<_>>()?;
        Ok(Self(paths))
    }

    pub fn union<'a>(closures: impl IntoIterator<Item = &'a Closure>) -> Self {
        Self(
            closures
                .into_iter()
                .flat_map(|closure| closure.0.iter().cloned())
                .collect(),
        )
    }

    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.0.iter().map(PathBuf::as_path)
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.0.contains(path)
    }
}

fn store_path(line: &str) -> Result<PathBuf> {
    let path = PathBuf::from(line);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|_| path.parent() == Some(Path::new(STORE)))
        .wrap_err_with(|| format!("closure entry {line:?} is not a top-level store path"))?;
    ensure!(
        !name.ends_with("-source"),
        "closure holds the source path {line}, which could expose hidden tests"
    );
    Ok(path)
}

/// One environment: a buildEnv whose `bin` goes on PATH, and the closure that makes it run.
#[derive(Clone, Debug)]
pub struct Toolchain {
    root: PathBuf,
    closure: Closure,
}

impl Toolchain {
    /// Reads `<dir>/bin`, a link to the env's bin directory, and `<dir>/closure`.
    fn load(dir: &Path) -> Result<Self> {
        let bin = dir.join("bin");
        let target = fs::read_link(&bin).wrap_err_with(|| format!("reading {}", bin.display()))?;
        let root = target
            .parent()
            .filter(|_| target.ends_with("bin"))
            .wrap_err_with(|| format!("{} does not link to an env's bin", bin.display()))?;
        let list = dir.join("closure");
        let closure = fs::read_to_string(&list)
            .map_err(eyre::Report::from)
            .and_then(|list| Closure::parse(&list))
            .wrap_err_with(|| format!("loading {}", list.display()))?;
        Self::new(root.to_path_buf(), closure)
    }

    pub(crate) fn new(root: PathBuf, closure: Closure) -> Result<Self> {
        ensure!(
            closure.contains(&root),
            "{} is missing from its own closure",
            root.display()
        );
        Ok(Self { root, closure })
    }

    /// The env's store path. Everything under it resolves inside a sandbox that binds the closure.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub fn bin(&self) -> PathBuf {
        self.root.join("bin")
    }

    pub fn closure(&self) -> &Closure {
        &self.closure
    }
}

/// Every environment built for this checkout: the sandbox base, the measurement tools, one per
/// language, one per harness.
#[derive(Clone, Debug)]
pub struct Toolchains {
    base: Toolchain,
    measure: Toolchain,
    languages: BTreeMap<String, Toolchain>,
    harnesses: BTreeMap<String, Toolchain>,
}

impl Toolchains {
    pub fn from_env() -> Result<Self> {
        let dir = std::env::var_os(VARIABLE).wrap_err_with(|| {
            format!("{VARIABLE} is not set; enter the dev shell or run the packaged gungnir")
        })?;
        Self::load(Path::new(&dir))
    }

    pub fn load(dir: &Path) -> Result<Self> {
        Ok(Self {
            base: Toolchain::load(&dir.join("base"))?,
            measure: Toolchain::load(&dir.join("measure"))?,
            languages: load_all(&dir.join("language"))?,
            harnesses: load_all(&dir.join("harness"))?,
        })
    }

    pub fn base(&self) -> &Toolchain {
        &self.base
    }

    /// The tools that time programs, bound only into measurement sandboxes.
    pub fn measure(&self) -> &Toolchain {
        &self.measure
    }

    pub fn language(&self, id: &str) -> Result<&Toolchain> {
        lookup(&self.languages, "language", id)
    }

    pub fn harness(&self, name: &str) -> Result<&Toolchain> {
        lookup(&self.harnesses, "harness", name)
    }

    /// Runs `argv` with `env` in an offline sandbox holding only `toolchain` over the base, and
    /// returns what it printed. For asking a tool what it is, never for running a program.
    pub async fn probe(
        &self,
        toolchain: &Toolchain,
        env: Vec<(String, EnvValue)>,
        argv: &[impl AsRef<OsStr>],
        backend: &dyn Backend,
        stop: &Stop,
    ) -> Result<String> {
        let spec = Spec {
            network: Network::Offline,
            closure: Closure::union([self.base.closure(), toolchain.closure()]),
            mounts: Vec::new(),
            path: vec![toolchain.bin(), self.base.bin()],
            env,
            cwd: HOME.into(),
            limits: PROBE_LIMITS,
        };
        let io = Io {
            stdin: Input::Null,
            stdout: Output::Capture { cap: PROBE_OUTPUT },
            stderr: Output::Capture { cap: PROBE_OUTPUT },
            kill: Kill::default(),
        };
        let argv: Vec<OsString> = argv.iter().map(|arg| arg.as_ref().to_owned()).collect();
        let finished = backend.exec(&spec, &argv, io, stop).await?;
        ensure!(
            finished.exit.success(),
            "ended with {:?}: {}",
            finished.exit,
            String::from_utf8_lossy(&finished.stderr.bytes).trim()
        );
        Ok(String::from_utf8_lossy(&finished.stdout.bytes).into_owned())
    }
}

fn load_all(dir: &Path) -> Result<BTreeMap<String, Toolchain>> {
    let entries = match fs::read_dir(dir) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        entries => entries.wrap_err_with(|| format!("listing {}", dir.display()))?,
    };
    entries
        .map(|entry| {
            let path = entry?.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                bail!("{} is not a UTF-8 name", path.display());
            };
            Ok((name.to_owned(), Toolchain::load(&path)?))
        })
        .collect()
}

fn lookup<'a>(
    toolchains: &'a BTreeMap<String, Toolchain>,
    kind: &str,
    name: &str,
) -> Result<&'a Toolchain> {
    toolchains.get(name).wrap_err_with(|| {
        format!("no {kind} toolchain named {name:?}; re-enter the dev shell after adding one")
    })
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    fn write_env(dir: &Path, env: &str, closure: &[&str]) -> Result<()> {
        fs::create_dir_all(dir)?;
        symlink(format!("{STORE}/{env}/bin"), dir.join("bin"))?;
        fs::write(dir.join("closure"), closure.join("\n") + "\n")?;
        Ok(())
    }

    fn layout() -> Result<tempfile::TempDir> {
        let dir = tempfile::tempdir()?;
        let libc = "/nix/store/1111-glibc-2.42";
        write_env(
            &dir.path().join("base"),
            "2222-gungnir-base",
            &["/nix/store/2222-gungnir-base", libc],
        )?;
        write_env(
            &dir.path().join("measure"),
            "8888-gungnir-measure",
            &["/nix/store/8888-gungnir-measure", libc],
        )?;
        write_env(
            &dir.path().join("language/zig"),
            "3333-gungnir-language-zig",
            &["/nix/store/3333-gungnir-language-zig", libc],
        )?;
        write_env(
            &dir.path().join("harness/codex"),
            "4444-gungnir-harness-codex",
            &["/nix/store/4444-gungnir-harness-codex", libc],
        )?;
        Ok(dir)
    }

    #[test]
    fn loads_every_environment_with_its_bin_and_closure() -> Result<()> {
        let dir = layout()?;
        let toolchains = Toolchains::load(dir.path())?;

        assert_eq!(
            toolchains.base().bin(),
            Path::new("/nix/store/2222-gungnir-base/bin")
        );
        assert_eq!(
            toolchains.measure().bin(),
            Path::new("/nix/store/8888-gungnir-measure/bin")
        );
        let zig = toolchains.language("zig")?;
        assert_eq!(
            zig.bin(),
            Path::new("/nix/store/3333-gungnir-language-zig/bin")
        );
        assert!(
            zig.closure()
                .contains(Path::new("/nix/store/1111-glibc-2.42"))
        );
        assert!(toolchains.harness("codex").is_ok());
        assert!(toolchains.language("cobol").is_err());
        Ok(())
    }

    #[test]
    fn union_holds_each_path_once() -> Result<()> {
        let dir = layout()?;
        let toolchains = Toolchains::load(dir.path())?;
        let union = Closure::union([
            toolchains.base().closure(),
            toolchains.language("zig")?.closure(),
        ]);
        let paths: Vec<_> = union.paths().collect();
        assert_eq!(
            paths,
            [
                Path::new("/nix/store/1111-glibc-2.42"),
                Path::new("/nix/store/2222-gungnir-base"),
                Path::new("/nix/store/3333-gungnir-language-zig"),
            ]
        );
        Ok(())
    }

    #[test]
    fn refuses_a_closure_with_a_flake_source_path() -> Result<()> {
        let dir = layout()?;
        write_env(
            &dir.path().join("language/python"),
            "5555-gungnir-language-python",
            &[
                "/nix/store/5555-gungnir-language-python",
                "/nix/store/6666-source",
            ],
        )?;
        let error = Toolchains::load(dir.path()).unwrap_err();
        assert!(format!("{error:#}").contains("6666-source"), "{error:#}");
        Ok(())
    }

    #[test]
    fn refuses_entries_outside_the_store() {
        for entry in ["/", "/nix/store", "/home/agent/.ssh", "/nix/store/1-a/sub"] {
            assert!(Closure::parse(entry).is_err(), "{entry} accepted");
        }
    }

    #[test]
    fn refuses_an_env_missing_from_its_closure() -> Result<()> {
        let dir = tempfile::tempdir()?;
        write_env(
            &dir.path().join("base"),
            "7777-gungnir-base",
            &["/nix/store/1111-glibc-2.42"],
        )?;
        let error = Toolchains::load(dir.path()).unwrap_err();
        assert!(
            format!("{error:#}").contains("7777-gungnir-base"),
            "{error:#}"
        );
        Ok(())
    }
}
