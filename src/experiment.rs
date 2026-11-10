//! An experiment: the matrix of systems, scenarios, languages and repetitions to run, with
//! the pins, budget and concurrency that govern running it.
use crate::{
    catalog::{Catalog, System, SystemId},
    id::{BackendId, Effort, HarnessName, HarnessVersion, LanguageId, ModelId, Rep, ScenarioId},
    language::Language,
    measure::{Identity, Protocol},
    scenario::Scenario,
    store::read_toml,
    trial::{ResultsDir, TrialKey},
};
use eyre::{Result, WrapErr, bail, ensure, eyre};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    num::{NonZeroU32, NonZeroUsize},
    path::{Component, Path, PathBuf},
    time::Duration,
};

#[cfg(test)]
mod tests;

/// A validated experiment. Every system is served by the catalog at a pinned harness
/// version, every scenario has a reference for every language, and every backend in use
/// has a concurrency limit.
#[derive(Debug)]
pub struct Experiment {
    name: String,
    results: ResultsDir,
    repetitions: NonZeroU32,
    systems: Vec<System>,
    /// The systems whose `[[systems]]` entry sets its own repetitions.
    own_repetitions: BTreeMap<SystemId, NonZeroU32>,
    scenarios: Vec<Scenario>,
    languages: Vec<Language>,
    pins: BTreeMap<HarnessName, HarnessVersion>,
    machine: Option<Identity>,
    protocol: Protocol,
    budget: Budget,
    concurrency: BTreeMap<BackendId, NonZeroUsize>,
    catalog: Catalog,
}

impl Experiment {
    /// Loads the experiment at `path` together with the repository holding it: the nearest
    /// directory above `path` with a `gungnir.toml`, and that directory's `languages/` and
    /// `scenarios/`.
    pub fn load(path: &Path) -> Result<Self> {
        let path =
            fs::canonicalize(path).wrap_err_with(|| format!("reading {}", path.display()))?;
        let raw: RawExperiment = read_toml(&path)?;
        let root = path
            .ancestors()
            .skip(1)
            .find(|dir| dir.join("gungnir.toml").is_file())
            .ok_or_else(|| eyre!("no directory above {} has a gungnir.toml", path.display()))?;
        let catalog = Catalog::load(&root.join("gungnir.toml"))?;
        let languages = Language::load_all(&root.join("languages"))?;
        let scenarios = Scenario::load_all(&root.join("scenarios"), &languages)?;
        raw.resolve(&path, catalog, languages, scenarios)
            .wrap_err_with(|| format!("invalid experiment {}", path.display()))
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn results(&self) -> &ResultsDir {
        &self.results
    }

    /// The repetitions a system gets unless its `[[systems]]` entry sets its own.
    pub fn repetitions(&self) -> NonZeroU32 {
        self.repetitions
    }

    /// The repetitions `system` gets.
    pub fn repetitions_of(&self, system: &System) -> NonZeroU32 {
        self.own_repetitions
            .get(&system.id())
            .copied()
            .unwrap_or(self.repetitions)
    }

    /// The most repetitions any system gets.
    pub fn most_repetitions(&self) -> NonZeroU32 {
        self.systems
            .iter()
            .map(|system| self.repetitions_of(system))
            .max()
            .unwrap_or(self.repetitions)
    }

    /// The systems in the order listed, each entry's efforts by rank.
    pub fn systems(&self) -> &[System] {
        &self.systems
    }

    pub fn scenarios(&self) -> &[Scenario] {
        &self.scenarios
    }

    pub fn languages(&self) -> &[Language] {
        &self.languages
    }

    /// The harness version each harness in use must report.
    pub fn pins(&self) -> &BTreeMap<HarnessName, HarnessVersion> {
        &self.pins
    }

    /// The machine every measurement must be made on, when `[machine]` pins one.
    pub fn machine(&self) -> Option<&Identity> {
        self.machine.as_ref()
    }

    /// The timing protocol every measurement is made under: `[measure]`, or
    /// [`Protocol::DEFAULT`] when the experiment declares none. A measurement made under any
    /// other protocol is stale.
    pub fn protocol(&self) -> Protocol {
        self.protocol
    }

    pub fn budget(&self) -> Budget {
        self.budget
    }

    /// How many attempts may run at once on each backend in use.
    pub fn concurrency(&self) -> &BTreeMap<BackendId, NonZeroUsize> {
        &self.concurrency
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// Every trial, repetition-major: each cell's `r1` comes before any cell's `r2`, so an
    /// interrupted run leaves the matrix evenly covered. Within a repetition the order is
    /// systems, then scenarios, then languages, as listed.
    pub fn trials(&self) -> Vec<TrialKey> {
        let mut trials = Vec::new();
        for rep in Rep::up_to(self.most_repetitions()) {
            for system in &self.systems {
                if rep.get() > self.repetitions_of(system).get() {
                    continue;
                }
                for scenario in &self.scenarios {
                    for language in &self.languages {
                        trials.push(TrialKey {
                            system: system.clone(),
                            scenario: scenario.id().clone(),
                            language: language.id().clone(),
                            rep,
                        });
                    }
                }
            }
        }
        trials
    }
}

/// Time limits on one attempt, both positive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    wall: Duration,
    idle: Duration,
}

impl Budget {
    /// The longest an attempt may run.
    pub fn wall(&self) -> Duration {
        self.wall
    }

    /// The longest an attempt may go without printing a line on stdout.
    pub fn idle(&self) -> Duration {
        self.idle
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExperiment {
    name: String,
    results: PathBuf,
    repetitions: NonZeroU32,
    scenarios: Vec<ScenarioId>,
    languages: Vec<LanguageId>,
    pins: BTreeMap<HarnessName, HarnessVersion>,
    #[serde(default)]
    machine: Option<RawMachine>,
    #[serde(default)]
    measure: Option<RawMeasure>,
    budget: RawBudget,
    concurrency: BTreeMap<BackendId, NonZeroUsize>,
    systems: Vec<RawSystems>,
}

/// `[machine]`: the CPU model and host id that `gungnir machine` prints.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMachine {
    cpu: String,
    host: String,
}

impl RawMachine {
    fn resolve(self) -> Result<Identity> {
        ensure!(!self.cpu.trim().is_empty(), "machine.cpu is empty");
        ensure!(!self.host.trim().is_empty(), "machine.host is empty");
        Ok(Identity {
            cpu: Some(self.cpu),
            host: Some(self.host),
        })
    }
}

/// `[measure]`: the timing protocol. `blocks`, `runs` and `warmup` are required, so that the
/// file states the whole protocol its measurements are made under. Without `clock_mhz` the
/// clock is left free.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMeasure {
    blocks: NonZeroU32,
    runs: NonZeroU32,
    warmup: u32,
    clock_mhz: Option<NonZeroU32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBudget {
    wall: String,
    idle: String,
}

/// A `[[systems]]` entry: one model at one or more efforts, with the experiment's repetitions
/// unless it sets its own.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSystems {
    model: ModelId,
    backend: Option<BackendId>,
    efforts: RawEfforts,
    repetitions: Option<NonZeroU32>,
}

/// `"all"` or a list of effort names, parsed after loading so a typo gets a clear message.
#[derive(Deserialize)]
#[serde(untagged)]
enum RawEfforts {
    Keyword(String),
    Listed(Vec<String>),
}

impl RawExperiment {
    fn resolve(
        self,
        path: &Path,
        catalog: Catalog,
        mut languages: BTreeMap<LanguageId, Language>,
        mut scenarios: BTreeMap<ScenarioId, Scenario>,
    ) -> Result<Experiment> {
        ensure!(!self.name.trim().is_empty(), "name is empty");
        let languages = pick("languages", self.languages, &mut languages, "languages/")?;
        let scenarios = pick("scenarios", self.scenarios, &mut scenarios, "scenarios/")?;
        for scenario in &scenarios {
            for language in &languages {
                ensure!(
                    scenario.reference(language.id()).is_some(),
                    "scenario {} has no reference/{} for language {}",
                    scenario.id(),
                    language.id(),
                    language.id()
                );
            }
        }

        if let Some(harness) = self
            .pins
            .keys()
            .find(|pinned| !catalog.harnesses().any(|known| known == *pinned))
        {
            bail!("[pins] names harness {harness}, which gungnir.toml does not declare");
        }
        let mut systems: Vec<System> = Vec::new();
        let mut own_repetitions = BTreeMap::new();
        for entry in self.systems {
            let model = entry.model.clone();
            let repetitions = entry.repetitions;
            for system in entry
                .resolve(&catalog, &self.pins)
                .wrap_err_with(|| format!("[[systems]] model = \"{model}\""))?
            {
                ensure!(
                    !systems.contains(&system),
                    "system {} is listed twice",
                    system.id()
                );
                if let Some(repetitions) = repetitions {
                    own_repetitions.insert(system.id(), repetitions);
                }
                systems.push(system);
            }
        }
        ensure!(!systems.is_empty(), "[[systems]] is empty");

        let used: BTreeSet<&BackendId> = systems.iter().map(System::backend).collect();
        let limited: BTreeSet<&BackendId> = self.concurrency.keys().collect();
        if let Some(backend) = used.difference(&limited).next() {
            bail!("[concurrency] has no limit for backend {backend}, which the systems use");
        }
        if let Some(backend) = limited.difference(&used).next() {
            bail!("[concurrency] limits backend {backend}, which no system uses");
        }

        let dir = path.parent().unwrap_or(Path::new("/"));
        Ok(Experiment {
            name: self.name,
            results: ResultsDir::new(normalize(&dir.join(&self.results))),
            repetitions: self.repetitions,
            systems,
            own_repetitions,
            scenarios,
            languages,
            pins: self.pins,
            machine: self.machine.map(RawMachine::resolve).transpose()?,
            protocol: self.measure.map_or(Protocol::DEFAULT, |measure| Protocol {
                blocks: measure.blocks,
                runs: measure.runs,
                warmup: measure.warmup,
                clock_mhz: measure.clock_mhz,
            }),
            budget: Budget {
                wall: positive_duration("budget.wall", &self.budget.wall)?,
                idle: positive_duration("budget.idle", &self.budget.idle)?,
            },
            concurrency: self.concurrency,
            catalog,
        })
    }
}

impl RawSystems {
    fn resolve(
        self,
        catalog: &Catalog,
        pins: &BTreeMap<HarnessName, HarnessVersion>,
    ) -> Result<Vec<System>> {
        let model = catalog.model(&self.model).ok_or_else(|| {
            eyre!(
                "model {} has no [[models]] entry in gungnir.toml",
                self.model
            )
        })?;
        let backend = match self.backend {
            Some(backend) => backend,
            None => {
                let backends: Vec<&BackendId> = model.backends().collect();
                match backends.as_slice() {
                    [only] => (*only).clone(),
                    _ => {
                        let names: Vec<&str> = backends.iter().map(|b| b.as_str()).collect();
                        bail!(
                            "model {} is served by {}; choose one with backend = \"...\"",
                            self.model,
                            names.join(", ")
                        )
                    }
                }
            }
        };
        let harness = catalog
            .backend(&backend)
            .ok_or_else(|| eyre!("backend = \"{backend}\" names no backend in gungnir.toml"))?
            .harness();
        let version = pins.get(harness).ok_or_else(|| {
            eyre!("[pins] has no version for harness {harness}, which backend {backend} uses")
        })?;
        let efforts = match self.efforts {
            RawEfforts::Keyword(keyword) => {
                ensure!(
                    keyword == "all",
                    "efforts = {keyword:?} is neither \"all\" nor a list of efforts"
                );
                model.efforts().collect()
            }
            RawEfforts::Listed(names) => listed_efforts(names)?,
        };
        efforts
            .into_iter()
            .map(|effort| catalog.system(&self.model, &backend, effort, version.clone()))
            .collect()
    }
}

fn listed_efforts(names: Vec<String>) -> Result<BTreeSet<Effort>> {
    ensure!(!names.is_empty(), "efforts is empty");
    let mut efforts = BTreeSet::new();
    for name in names {
        let effort: Effort = name.parse()?;
        ensure!(efforts.insert(effort), "efforts lists {effort} twice");
    }
    Ok(efforts)
}

/// Moves the entries `ids` names out of `available`, in the order listed.
fn pick<K, V>(key: &str, ids: Vec<K>, available: &mut BTreeMap<K, V>, dir: &str) -> Result<Vec<V>>
where
    K: Ord + fmt::Display,
{
    ensure!(!ids.is_empty(), "{key} is empty");
    let mut seen = BTreeSet::new();
    ids.into_iter()
        .map(|id| {
            ensure!(!seen.contains(&id), "{key} lists {id} twice");
            let value = available
                .remove(&id)
                .ok_or_else(|| eyre!("{key} lists {id}, which has no directory under {dir}"))?;
            seen.insert(id);
            Ok(value)
        })
        .collect()
}

fn positive_duration(key: &str, text: &str) -> Result<Duration> {
    let duration = humantime::parse_duration(text).wrap_err_with(|| format!("{key} = {text:?}"))?;
    ensure!(!duration.is_zero(), "{key} = {text:?} is zero");
    Ok(duration)
}

/// `path` with `.` and `..` resolved lexically, for a results directory that may not exist.
fn normalize(path: &Path) -> PathBuf {
    let mut normal = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normal.pop();
            }
            other => normal.push(other),
        }
    }
    normal
}
