//! Tasks, one directory per scenario under `scenarios/`.
use crate::{
    id::{CaseId, LanguageId, ScenarioId, WorkloadId},
    language::Language,
    store::{Digest, read_toml, subdirs},
};
use eyre::{Result, WrapErr, bail, ensure, eyre};
use globset::GlobSet;
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

/// A language-neutral task: the brief the agent sees, the hidden cases and workloads it is
/// judged on, and a reference solution per supported language.
#[derive(Debug)]
pub struct Scenario {
    id: ScenarioId,
    title: String,
    compare: Compare,
    limits: CaseLimits,
    brief: Brief,
    hidden: BTreeMap<CaseId, CaseFiles>,
    hidden_digest: Digest,
    workloads: BTreeMap<WorkloadId, Workload>,
    workloads_digest: Digest,
    references: BTreeMap<LanguageId, Reference>,
}

impl Scenario {
    /// Loads every directory under `dir` as a scenario named by the directory. A reference
    /// must be for one of `languages`, whose ignore globs its digest skips.
    pub fn load_all(
        dir: &Path,
        languages: &BTreeMap<LanguageId, Language>,
    ) -> Result<BTreeMap<ScenarioId, Self>> {
        subdirs::<ScenarioId>(dir)?
            .into_iter()
            .map(|(id, path)| {
                let scenario = Self::load(id.clone(), &path, languages)
                    .wrap_err_with(|| format!("scenario {}", path.display()))?;
                Ok((id, scenario))
            })
            .collect()
    }

    fn load(
        id: ScenarioId,
        dir: &Path,
        languages: &BTreeMap<LanguageId, Language>,
    ) -> Result<Self> {
        let manifest = dir.join("scenario.toml");
        let raw: RawScenario = read_toml(&manifest)?;
        let (title, compare, limits) = raw
            .resolve()
            .wrap_err_with(|| format!("invalid {}", manifest.display()))?;
        let hidden_dir = dir.join("hidden");
        let workloads_manifest = dir.join(WORKLOAD_DIGESTS);
        let workloads_bytes = fs::read(&workloads_manifest)
            .wrap_err_with(|| format!("reading {}", workloads_manifest.display()))?;
        Ok(Self {
            id,
            title,
            compare,
            limits,
            brief: Brief::load(dir)?,
            hidden: case_files(&hidden_dir)?,
            hidden_digest: Digest::of_tree(&hidden_dir, &GlobSet::empty())?,
            workloads: workloads(dir, &workloads_bytes)
                .wrap_err_with(|| format!("invalid {}", workloads_manifest.display()))?,
            workloads_digest: Digest::of_bytes(&workloads_bytes),
            references: references(&dir.join("reference"), languages)?,
        })
    }

    pub fn id(&self) -> &ScenarioId {
        &self.id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn compare(&self) -> Compare {
        self.compare
    }

    pub fn limits(&self) -> CaseLimits {
        self.limits
    }

    /// The only view of the scenario the run phase receives.
    pub fn brief(&self) -> &Brief {
        &self.brief
    }

    /// The hidden cases. Never empty.
    pub fn hidden(&self) -> &BTreeMap<CaseId, CaseFiles> {
        &self.hidden
    }

    pub fn hidden_digest(&self) -> Digest {
        self.hidden_digest
    }

    /// The timing inputs named by `workloads.sha256`. Never empty. Their files are generated
    /// on demand, so a workload's file may not exist yet.
    pub fn workloads(&self) -> &BTreeMap<WorkloadId, Workload> {
        &self.workloads
    }

    /// The digest of `workloads.sha256`, which pins every workload's bytes.
    pub fn workloads_digest(&self) -> Digest {
        self.workloads_digest
    }

    /// The languages with a reference solution, which are the languages the scenario
    /// supports. Never empty.
    pub fn languages(&self) -> impl Iterator<Item = &LanguageId> {
        self.references.keys()
    }

    pub fn reference(&self, language: &LanguageId) -> Option<&Reference> {
        self.references.get(language)
    }
}

/// The agent-visible part of a scenario: the spec and the visible examples, held in memory
/// rather than as paths, so nothing reachable from a `Brief` leads to the hidden cases.
#[derive(Debug)]
pub struct Brief {
    spec: String,
    examples: BTreeMap<CaseId, Example>,
    digest: Digest,
}

impl Brief {
    fn load(scenario_dir: &Path) -> Result<Self> {
        let spec_path = scenario_dir.join("spec.md");
        let spec = fs::read_to_string(&spec_path)
            .wrap_err_with(|| format!("reading {}", spec_path.display()))?;
        let examples = case_files(&scenario_dir.join("examples"))?
            .into_iter()
            .map(|(id, files)| Ok((id, files.read()?)))
            .collect::<Result<BTreeMap<_, _>>>()?;

        let example_files = examples.iter().flat_map(|(id, example)| {
            [
                (format!("examples/{id}.in"), example.input.as_slice()),
                (format!("examples/{id}.out"), example.output.as_slice()),
            ]
        });
        let spec_file = ("spec.md".to_owned(), spec.as_bytes());
        let digest = Digest::of_files(std::iter::once(spec_file).chain(example_files));
        Ok(Self {
            spec,
            examples,
            digest,
        })
    }

    pub fn spec(&self) -> &str {
        &self.spec
    }

    /// The visible examples. Never empty.
    pub fn examples(&self) -> &BTreeMap<CaseId, Example> {
        &self.examples
    }

    /// The digest of `spec.md` and `examples/` as files.
    pub fn digest(&self) -> Digest {
        self.digest
    }
}

/// A visible example: stdin and the expected stdout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Example {
    pub input: Vec<u8>,
    pub output: Vec<u8>,
}

/// A timing input: where its file lives once generated, and the SHA-256 its bytes must have.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workload {
    pub path: PathBuf,
    pub digest: Digest,
}

/// A case on disk: the stdin file and the expected stdout file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaseFiles {
    pub input: PathBuf,
    pub output: PathBuf,
}

impl CaseFiles {
    fn read(&self) -> Result<Example> {
        let read =
            |path: &Path| fs::read(path).wrap_err_with(|| format!("reading {}", path.display()));
        Ok(Example {
            input: read(&self.input)?,
            output: read(&self.output)?,
        })
    }
}

/// A reference solution for one language.
#[derive(Debug)]
pub struct Reference {
    dir: PathBuf,
    digest: Digest,
}

impl Reference {
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The digest of the solution's files, skipping the language's ignore globs.
    pub fn digest(&self) -> Digest {
        self.digest
    }
}

/// How a program's stdout is judged against the expected output.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Compare {
    /// Byte for byte.
    Exact,
    /// The same whitespace-separated tokens, however they are spaced.
    Whitespace,
    /// The same tokens, where numbers may differ within the tolerance.
    Numeric(Tolerance),
}

/// Two numbers match when they are within `abs` of each other or within `rel` of the
/// expected value's magnitude. Both are finite and non-negative.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tolerance {
    pub abs: f64,
    pub rel: f64,
}

/// What one case may use. Both limits are positive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaseLimits {
    time: Duration,
    memory_bytes: u64,
}

impl CaseLimits {
    pub fn time(&self) -> Duration {
        self.time
    }

    pub fn memory_bytes(&self) -> u64 {
        self.memory_bytes
    }
}

/// Reads `dir` as `NAME.in` and `NAME.out` pairs, rejecting any other entry.
fn case_files(dir: &Path) -> Result<BTreeMap<CaseId, CaseFiles>> {
    let mut inputs = BTreeSet::new();
    let mut outputs = BTreeSet::new();
    for (path, stem, extension) in plain_files(dir)? {
        let id: CaseId = stem
            .parse()
            .wrap_err_with(|| format!("{}", path.display()))?;
        match extension.as_str() {
            "in" => inputs.insert(id),
            "out" => outputs.insert(id),
            _ => bail!("{} is not a NAME.in or NAME.out file", path.display()),
        };
    }
    if let Some(id) = inputs.difference(&outputs).next() {
        bail!("{} is missing", dir.join(format!("{id}.out")).display());
    }
    if let Some(id) = outputs.difference(&inputs).next() {
        bail!("{} is missing", dir.join(format!("{id}.in")).display());
    }
    ensure!(!inputs.is_empty(), "{} has no cases", dir.display());
    Ok(inputs
        .into_iter()
        .map(|id| {
            let files = CaseFiles {
                input: dir.join(format!("{id}.in")),
                output: dir.join(format!("{id}.out")),
            };
            (id, files)
        })
        .collect())
}

/// The manifest, in `sha256sum` format, naming every workload of a scenario.
const WORKLOAD_DIGESTS: &str = "workloads.sha256";

/// Parses `sha256sum` lines (`<hex digest>  NAME.in`) into workloads under `scenario_dir/workloads`.
fn workloads(scenario_dir: &Path, manifest: &[u8]) -> Result<BTreeMap<WorkloadId, Workload>> {
    let text = std::str::from_utf8(manifest).wrap_err("not UTF-8")?;
    let mut workloads = BTreeMap::new();
    for (number, line) in text.lines().enumerate().map(|(i, line)| (i + 1, line)) {
        let (digest, name) = line
            .split_once("  ")
            .ok_or_else(|| eyre!("line {number} is not `<sha256>  NAME.in`: {line:?}"))?;
        let digest: Digest = digest
            .parse()
            .wrap_err_with(|| format!("line {number}: {digest:?}"))?;
        let stem = name
            .strip_suffix(".in")
            .ok_or_else(|| eyre!("line {number}: {name:?} is not a NAME.in file"))?;
        let id: WorkloadId = stem
            .parse()
            .wrap_err_with(|| format!("line {number}: {name:?}"))?;
        let path = scenario_dir.join("workloads").join(name);
        ensure!(
            workloads.insert(id, Workload { path, digest }).is_none(),
            "line {number}: {name} is listed twice"
        );
    }
    ensure!(!workloads.is_empty(), "lists no workloads");
    Ok(workloads)
}

/// The regular files in `dir` as (path, stem, extension). Anything else is an error.
fn plain_files(dir: &Path) -> Result<Vec<(PathBuf, String, String)>> {
    let listing = fs::read_dir(dir).wrap_err_with(|| format!("listing {}", dir.display()))?;
    let mut files = Vec::new();
    for entry in listing {
        let path = entry
            .wrap_err_with(|| format!("listing {}", dir.display()))?
            .path();
        let kind = fs::symlink_metadata(&path)
            .wrap_err_with(|| format!("inspecting {}", path.display()))?
            .file_type();
        ensure!(kind.is_file(), "{} is not a regular file", path.display());
        let (stem, extension) = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.rsplit_once('.'))
            .map(|(stem, extension)| (stem.to_owned(), extension.to_owned()))
            .ok_or_else(|| eyre!("{} has no NAME.extension file name", path.display()))?;
        files.push((path, stem, extension));
    }
    Ok(files)
}

fn references(
    dir: &Path,
    languages: &BTreeMap<LanguageId, Language>,
) -> Result<BTreeMap<LanguageId, Reference>> {
    let references = subdirs(dir)?
        .into_iter()
        .map(|(id, dir)| {
            let language = languages
                .get(&id)
                .ok_or_else(|| eyre!("{} names no language under languages/", dir.display()))?;
            let digest = Digest::of_tree(&dir, language.ignore())?;
            Ok((id, Reference { dir, digest }))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    ensure!(!references.is_empty(), "{} has no languages", dir.display());
    Ok(references)
}

/// `"512KiB"`, `"256MiB"` or `"1GiB"` in bytes.
fn parse_memory(text: &str) -> Result<u64> {
    let digits = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let (number, unit) = text.split_at(digits);
    let scale: u64 = match unit {
        "KiB" => 1 << 10,
        "MiB" => 1 << 20,
        "GiB" => 1 << 30,
        _ => bail!("limits.memory = {text:?} is not a whole number of KiB, MiB or GiB"),
    };
    number
        .parse::<u64>()
        .ok()
        .and_then(|number| number.checked_mul(scale))
        .filter(|bytes| *bytes > 0)
        .ok_or_else(|| eyre!("limits.memory = {text:?} is not a positive size"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawScenario {
    title: String,
    compare: RawCompare,
    limits: RawLimits,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
enum RawCompare {
    Exact,
    Whitespace,
    Numeric { abs: f64, rel: f64 },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLimits {
    time: String,
    memory: String,
}

impl RawScenario {
    fn resolve(self) -> Result<(String, Compare, CaseLimits)> {
        ensure!(!self.title.trim().is_empty(), "title is empty");
        let compare = match self.compare {
            RawCompare::Exact => Compare::Exact,
            RawCompare::Whitespace => Compare::Whitespace,
            RawCompare::Numeric { abs, rel } => {
                for (key, value) in [("abs", abs), ("rel", rel)] {
                    ensure!(
                        value.is_finite() && value >= 0.0,
                        "compare.numeric.{key} = {value} is not a finite non-negative number"
                    );
                }
                Compare::Numeric(Tolerance { abs, rel })
            }
        };
        let time = humantime::parse_duration(&self.limits.time)
            .wrap_err_with(|| format!("limits.time = {:?}", self.limits.time))?;
        ensure!(
            !time.is_zero(),
            "limits.time = {:?} is zero",
            self.limits.time
        );
        let limits = CaseLimits {
            time,
            memory_bytes: parse_memory(&self.limits.memory)?,
        };
        Ok((self.title, compare, limits))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{self, id, write};
    use tempfile::TempDir;

    fn fixture() -> TempDir {
        let root = TempDir::new().unwrap();
        testing::language(root.path(), "python");
        testing::language(root.path(), "rust");
        testing::scenario(root.path(), "echo", &["python", "rust"]);
        root
    }

    fn load(root: &Path) -> Result<BTreeMap<ScenarioId, Scenario>> {
        let languages = Language::load_all(&root.join("languages"))?;
        Scenario::load_all(&root.join("scenarios"), &languages)
    }

    fn echo(root: &Path) -> Scenario {
        load(root).unwrap().remove(&id("echo")).unwrap()
    }

    fn rejects(edit: impl FnOnce(&Path), needles: &[&str]) {
        let root = fixture();
        edit(root.path());
        let message = format!("{:#}", load(root.path()).unwrap_err());
        for needle in needles {
            assert!(
                message.contains(needle),
                "{needle:?} missing from: {message}"
            );
        }
    }

    fn edit_manifest(root: &Path, from: &str, to: &str) {
        assert!(
            testing::SCENARIO_TOML.contains(from),
            "fixture lacks {from:?}"
        );
        write(
            root,
            "scenarios/echo/scenario.toml",
            testing::SCENARIO_TOML.replace(from, to),
        );
    }

    #[test]
    fn loads_a_scenario_with_its_cases_workloads_and_references() {
        let root = fixture();
        let echo = echo(root.path());
        assert_eq!(echo.title(), "Echo");
        assert_eq!(echo.compare(), Compare::Whitespace);
        assert_eq!(echo.limits().time(), Duration::from_secs(10));
        assert_eq!(echo.limits().memory_bytes(), 1 << 30);

        let brief = echo.brief();
        assert_eq!(brief.spec(), "Echo each line of stdin.\n");
        let basic = &brief.examples()[&id::<CaseId>("basic")];
        assert_eq!(
            (basic.input.as_slice(), basic.output.as_slice()),
            (&b"a\n"[..], &b"a\n"[..])
        );

        let hidden: Vec<&str> = echo.hidden().keys().map(CaseId::as_str).collect();
        assert_eq!(hidden, ["edge", "many"]);
        let edge = &echo.hidden()[&id::<CaseId>("edge")];
        assert_eq!(fs::read(&edge.output).unwrap(), b"\n");

        let large = &echo.workloads()[&id::<WorkloadId>("large")];
        assert_eq!(echo.workloads().len(), 1);
        assert!(large.path.ends_with("scenarios/echo/workloads/large.in"));
        assert_eq!(large.digest, Digest::of_bytes(b"x\n"));

        let languages: Vec<&str> = echo.languages().map(LanguageId::as_str).collect();
        assert_eq!(languages, ["python", "rust"]);
        assert!(echo.reference(&id("zig")).is_none());
    }

    #[test]
    fn numeric_compare_carries_its_tolerance() {
        let root = fixture();
        edit_manifest(
            root.path(),
            "compare = \"whitespace\"",
            "compare = { numeric = { abs = 1e-6, rel = 1e-9 } }",
        );
        let tolerance = Tolerance {
            abs: 1e-6,
            rel: 1e-9,
        };
        assert_eq!(echo(root.path()).compare(), Compare::Numeric(tolerance));
    }

    #[test]
    fn brief_digest_covers_the_spec_and_examples_but_not_hidden_cases() {
        let root = fixture();
        let before = echo(root.path());

        write(root.path(), "scenarios/echo/hidden/edge.out", "changed\n");
        let hidden_edit = echo(root.path());
        assert_eq!(hidden_edit.brief().digest(), before.brief().digest());
        assert_ne!(hidden_edit.hidden_digest(), before.hidden_digest());

        write(root.path(), "scenarios/echo/examples/basic.out", "b\n");
        let example_edit = echo(root.path());
        assert_ne!(example_edit.brief().digest(), before.brief().digest());
        assert_eq!(example_edit.workloads_digest(), before.workloads_digest());
    }

    #[test]
    fn reference_digests_skip_the_language_ignore_globs() {
        let root = fixture();
        let python = id::<LanguageId>("python");
        let before = echo(root.path()).reference(&python).unwrap().digest();

        write(
            root.path(),
            "scenarios/echo/reference/python/__pycache__/main.pyc",
            "x",
        );
        assert_eq!(
            echo(root.path()).reference(&python).unwrap().digest(),
            before
        );

        write(
            root.path(),
            "scenarios/echo/reference/python/main.py",
            "print(input().upper())\n",
        );
        assert_ne!(
            echo(root.path()).reference(&python).unwrap().digest(),
            before
        );
    }

    #[test]
    fn cases_come_in_complete_pairs() {
        rejects(
            |root| fs::remove_file(root.join("scenarios/echo/examples/basic.out")).unwrap(),
            &["scenarios/echo", "examples/basic.out is missing"],
        );
        rejects(
            |root| fs::remove_file(root.join("scenarios/echo/hidden/many.in")).unwrap(),
            &["hidden/many.in is missing"],
        );
        rejects(
            |root| write(root, "scenarios/echo/hidden/notes.txt", ""),
            &["hidden/notes.txt"],
        );
        rejects(
            |root| write(root, "scenarios/echo/hidden/Big.in", ""),
            &["hidden/Big.in", "\"Big\""],
        );
    }

    #[test]
    fn every_part_of_the_layout_is_required() {
        for part in [
            "spec.md",
            "examples",
            "hidden",
            "workloads.sha256",
            "reference",
        ] {
            rejects(
                |root| {
                    let path = root.join("scenarios/echo").join(part);
                    if path.is_dir() {
                        fs::remove_dir_all(path).unwrap();
                    } else {
                        fs::remove_file(path).unwrap();
                    }
                },
                &[part],
            );
        }
    }

    #[test]
    fn workload_files_are_optional_but_their_digests_are_pinned() {
        let root = fixture();
        let before = echo(root.path());
        fs::remove_dir_all(root.path().join("scenarios/echo/workloads")).unwrap();
        assert_eq!(
            echo(root.path()).workloads_digest(),
            before.workloads_digest()
        );

        write(
            root.path(),
            "scenarios/echo/workloads.sha256",
            format!("{}  large.in\n", Digest::of_bytes(b"y\n")),
        );
        assert_ne!(
            echo(root.path()).workloads_digest(),
            before.workloads_digest()
        );
    }

    #[test]
    fn workload_manifest_lines_are_validated() {
        rejects(
            |root| write(root, "scenarios/echo/workloads.sha256", "abc  large.in\n"),
            &["workloads.sha256", "line 1", "\"abc\""],
        );
        rejects(
            |root| {
                let line = format!("{}  large.txt\n", Digest::of_bytes(b"x"));
                write(root, "scenarios/echo/workloads.sha256", line)
            },
            &["line 1", "large.txt"],
        );
        rejects(
            |root| {
                let line = format!("{}  large.in\n", Digest::of_bytes(b"x"));
                write(root, "scenarios/echo/workloads.sha256", line.repeat(2))
            },
            &["line 2", "listed twice"],
        );
        rejects(
            |root| write(root, "scenarios/echo/workloads.sha256", ""),
            &["lists no workloads"],
        );
    }

    #[test]
    fn references_must_be_for_known_languages() {
        rejects(
            |root| write(root, "scenarios/echo/reference/cobol/main.cob", ""),
            &["reference/cobol", "languages/"],
        );
    }

    #[test]
    fn manifest_values_are_validated() {
        rejects(
            |root| edit_manifest(root, "memory = \"1GiB\"", "memory = \"1GB\""),
            &["scenario.toml", "limits.memory = \"1GB\""],
        );
        rejects(
            |root| edit_manifest(root, "time = \"10s\"", "time = \"soon\""),
            &["scenario.toml", "limits.time = \"soon\""],
        );
        rejects(
            |root| {
                edit_manifest(
                    root,
                    "compare = \"whitespace\"",
                    "compare = { numeric = { abs = -1.0, rel = 0.0 } }",
                )
            },
            &["compare.numeric.abs = -1"],
        );
        rejects(
            |root| edit_manifest(root, "compare = \"whitespace\"", "compare = \"fuzzy\""),
            &["scenario.toml", "fuzzy"],
        );
    }
}
