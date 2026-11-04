//! The catalog in `gungnir.toml`: harnesses, the backends that serve models through them,
//! and the models with their effort levels and dated prices.
use crate::{
    harness::Harness,
    id::{BackendId, Effort, HarnessName, HarnessVersion, ModelId},
    store::read_toml,
};
use chrono::NaiveDate;
use eyre::{Result, WrapErr, bail, ensure, eyre};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::{Path, PathBuf},
};

#[cfg(test)]
mod tests;

/// Everything `gungnir.toml` declares, cross-checked at load: the harnesses are exactly the
/// registered drivers, every backend uses a declared harness and every model is served only by
/// declared backends.
#[derive(Debug)]
pub struct Catalog {
    harnesses: BTreeMap<HarnessName, Harness>,
    backends: BTreeMap<BackendId, Backend>,
    models: BTreeMap<ModelId, Model>,
}

impl Catalog {
    pub fn load(path: &Path) -> Result<Self> {
        let raw: RawCatalog = read_toml(path)?;
        raw.resolve()
            .wrap_err_with(|| format!("invalid catalog {}", path.display()))
    }

    pub fn harnesses(&self) -> impl Iterator<Item = &HarnessName> {
        self.harnesses.keys()
    }

    pub fn backend(&self, id: &BackendId) -> Option<&Backend> {
        self.backends.get(id)
    }

    pub fn model(&self, id: &ModelId) -> Option<&Model> {
        self.models.get(id)
    }

    /// The model `backend` serves under the API name `api_model`, if the catalog lists one.
    pub fn served(&self, backend: &BackendId, api_model: &str) -> Option<&Model> {
        self.models.values().find(|model| {
            model
                .served_by
                .get(backend)
                .is_some_and(|name| name == api_model)
        })
    }

    /// The system that runs `model_id` through `backend_id` at `effort`, with the backend's
    /// harness at `harness_version`. Fails unless the backend serves the model, the model
    /// offers the effort and the harness can carry it.
    pub fn system(
        &self,
        model_id: &ModelId,
        backend_id: &BackendId,
        effort: Effort,
        harness_version: HarnessVersion,
    ) -> Result<System> {
        let backend = self
            .backend(backend_id)
            .ok_or_else(|| eyre!("backend {backend_id} has no [backends.{backend_id}] table"))?;
        let model = self
            .model(model_id)
            .ok_or_else(|| eyre!("model {model_id} has no [[models]] entry"))?;
        let api_model = model.served_by.get(backend_id).ok_or_else(|| {
            eyre!(
                "model {model_id} is not served by backend {backend_id}, only by {}",
                list(model.backends())
            )
        })?;
        ensure!(
            model.efforts.contains(&effort),
            "model {model_id} does not offer effort {effort}, only {}",
            list(model.efforts())
        );
        ensure!(
            backend.driver.accepts(effort),
            "backend {backend_id} runs harness {}, which cannot carry effort {effort}",
            backend.harness
        );
        Ok(System {
            harness: backend.harness.clone(),
            harness_version,
            backend: backend_id.clone(),
            model: model_id.clone(),
            api_model: api_model.clone(),
            effort,
        })
    }
}

/// One account behind one endpoint, reached through one harness.
#[derive(Debug)]
pub struct Backend {
    harness: HarnessName,
    driver: Harness,
    endpoint: Option<Endpoint>,
    credential: Credential,
    egress: Vec<String>,
}

impl Backend {
    pub fn harness(&self) -> &HarnessName {
        &self.harness
    }

    /// The API base URL when the harness should not use its vendor's default.
    pub fn endpoint(&self) -> Option<&Endpoint> {
        self.endpoint.as_ref()
    }

    pub fn credential(&self) -> &Credential {
        &self.credential
    }

    /// Hosts the agent may reach on port 443. Never empty.
    pub fn egress(&self) -> &[String] {
        &self.egress
    }
}

/// An `https://` base URL whose host is one of its backend's egress hosts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    url: String,
    host: String,
}

impl Endpoint {
    fn parse(url: String) -> Result<Self> {
        let host = url
            .strip_prefix("https://")
            .map(|rest| rest.split_once('/').map_or(rest, |(host, _)| host))
            .filter(|host| is_hostname(host))
            .ok_or_else(|| eyre!("endpoint = {url:?} is not an https:// URL with a DNS host"))?
            .to_owned();
        Ok(Self { url, host })
    }

    pub fn as_str(&self) -> &str {
        &self.url
    }

    pub fn host(&self) -> &str {
        &self.host
    }
}

/// How gungnir authenticates to a backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Credential {
    /// A file whose contents become the environment variable `var`.
    EnvToken { var: String, file: UserPath },
    /// Like `EnvToken`, for an API key sent to the backend's endpoint, which is always set.
    ApiKey { var: String, file: UserPath },
    /// A dedicated `CODEX_HOME` whose `auth.json` seeds each attempt's isolated home.
    CodexHome { dir: UserPath },
}

/// A path that is absolute or starts with `~/`, kept as written until [`UserPath::expand`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserPath(PathBuf);

impl UserPath {
    fn parse(key: &str, path: String) -> Result<Self> {
        ensure!(
            path.starts_with("~/") || path.starts_with('/'),
            "{key} = {path:?} must be absolute or start with ~/"
        );
        Ok(Self(path.into()))
    }

    /// The path with a leading `~` replaced by `home`.
    pub fn expand(&self, home: &Path) -> PathBuf {
        match self.0.strip_prefix("~") {
            Ok(rest) => home.join(rest),
            Err(_) => self.0.clone(),
        }
    }
}

impl fmt::Display for UserPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.display().fmt(f)
    }
}

#[derive(Debug)]
pub struct Model {
    efforts: BTreeSet<Effort>,
    served_by: BTreeMap<BackendId, String>,
    prices: BTreeMap<NaiveDate, Tiers>,
}

impl Model {
    /// The effort levels the model accepts, by rank. Never empty.
    pub fn efforts(&self) -> impl Iterator<Item = Effort> + '_ {
        self.efforts.iter().copied()
    }

    /// The backends serving the model. Never empty.
    pub fn backends(&self) -> impl Iterator<Item = &BackendId> {
        self.served_by.keys()
    }

    /// The rates in effect on `day` for a request whose prompt has `prompt_tokens` tokens,
    /// or `None` before the first price date.
    pub fn rates(&self, day: NaiveDate, prompt_tokens: u64) -> Option<&Rates> {
        let (_, tiers) = self.prices.range(..=day).next_back()?;
        let above = tiers.above.range(..prompt_tokens).next_back();
        Some(above.map_or(&tiers.base, |(_, rates)| rates))
    }
}

/// The prices published on one date: a base tier and tiers for prompts above a token count.
#[derive(Debug)]
struct Tiers {
    base: Rates,
    above: BTreeMap<u64, Rates>,
}

/// The price of each kind of token. A rate the vendor does not list is `None`, never guessed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rates {
    pub input: Rate,
    pub output: Rate,
    pub cache_read: Rate,
    pub cache_write: Option<Rate>,
    pub cache_write_1h: Option<Rate>,
}

/// A price per token, exact to the picodollar, so that costs add up without rounding. One USD
/// per million tokens, as the catalog writes rates, is a million picodollars per token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rate(u64);

impl Rate {
    pub fn picodollars(self) -> u64 {
        self.0
    }
}

/// A harness at a pinned version, a backend, a model and an effort: the subject of every
/// result. Only [`Catalog::system`] constructs one, so the backend always serves the model
/// at that effort and its harness can carry it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct System {
    harness: HarnessName,
    harness_version: HarnessVersion,
    backend: BackendId,
    model: ModelId,
    api_model: String,
    effort: Effort,
}

impl System {
    pub fn id(&self) -> SystemId {
        SystemId(format!(
            "{}-{}_{}_{}_{}",
            self.harness, self.harness_version, self.backend, self.model, self.effort
        ))
    }

    pub fn harness(&self) -> &HarnessName {
        &self.harness
    }

    pub fn harness_version(&self) -> &HarnessVersion {
        &self.harness_version
    }

    pub fn backend(&self) -> &BackendId {
        &self.backend
    }

    pub fn model(&self) -> &ModelId {
        &self.model
    }

    /// The model name the backend's API expects.
    pub fn api_model(&self) -> &str {
        &self.api_model
    }

    pub fn effort(&self) -> Effort {
        self.effort
    }
}

/// `<harness>-<version>_<backend>_<model>_<effort>`, safe as a path component. Derived from
/// a [`System`] and never parsed back.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct SystemId(String);

impl fmt::Display for SystemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn list<T: fmt::Display>(items: impl IntoIterator<Item = T>) -> String {
    let items: Vec<String> = items.into_iter().map(|item| item.to_string()).collect();
    items.join(", ")
}

fn is_hostname(host: &str) -> bool {
    host.len() <= 253
        && host.split('.').all(|label| {
            (1..=63).contains(&label.len())
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

fn is_env_var(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCatalog {
    sandbox: RawPackages,
    measure: RawPackages,
    profile: RawPackages,
    harnesses: BTreeMap<HarnessName, RawHarness>,
    backends: BTreeMap<BackendId, RawBackend>,
    models: Vec<RawModel>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPackages {
    packages: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHarness {
    package: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBackend {
    harness: HarnessName,
    endpoint: Option<String>,
    credential: RawCredential,
    egress: Vec<String>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
enum RawCredential {
    EnvToken { var: String, file: String },
    ApiKey { var: String, file: String },
    CodexHome { dir: String },
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct RawModel {
    id: ModelId,
    efforts: Vec<Effort>,
    served_by: BTreeMap<BackendId, String>,
    prices: Vec<RawPrice>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct RawPrice {
    since: NaiveDate,
    above_prompt_tokens: Option<u64>,
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: Option<f64>,
    cache_write_1h: Option<f64>,
}

impl RawCatalog {
    fn resolve(self) -> Result<Catalog> {
        // nix/toolchains.nix builds from the package names; Rust only checks they are there.
        ensure!(
            !self.sandbox.packages.is_empty(),
            "[sandbox] packages is empty"
        );
        ensure!(
            !self.measure.packages.is_empty(),
            "[measure] packages is empty"
        );
        ensure!(
            !self.profile.packages.is_empty(),
            "[profile] packages is empty"
        );
        for (name, harness) in &self.harnesses {
            ensure!(
                !harness.package.is_empty(),
                "[harnesses.{name}] package is empty"
            );
        }
        let harnesses = self
            .harnesses
            .into_keys()
            .map(|name| {
                let driver = Harness::named(&name).ok_or_else(|| {
                    eyre!(
                        "[harnesses.{name}] has no driver; the drivers are {}",
                        list(Harness::all().map(Harness::name))
                    )
                })?;
                Ok((name, driver))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        if let Some(driver) = Harness::all()
            .find(|driver| !harnesses.keys().any(|name| name.as_str() == driver.name()))
        {
            bail!("driver {0} has no [harnesses.{0}] table", driver.name());
        }
        let backends = self
            .backends
            .into_iter()
            .map(|(id, raw)| {
                let backend = raw
                    .resolve(&harnesses)
                    .wrap_err_with(|| format!("[backends.{id}]"))?;
                Ok((id, backend))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        let mut models = BTreeMap::new();
        for raw in self.models {
            let id = raw.id.clone();
            let model = raw
                .resolve(&backends)
                .wrap_err_with(|| format!("[[models]] id = \"{id}\""))?;
            ensure!(
                models.insert(id.clone(), model).is_none(),
                "[[models]] id = \"{id}\" is declared twice"
            );
        }
        Ok(Catalog {
            harnesses,
            backends,
            models,
        })
    }
}

impl RawBackend {
    fn resolve(self, harnesses: &BTreeMap<HarnessName, Harness>) -> Result<Backend> {
        let driver = *harnesses.get(&self.harness).ok_or_else(|| {
            eyre!(
                "harness = \"{0}\" names no [harnesses.{0}] table",
                self.harness
            )
        })?;
        ensure!(!self.egress.is_empty(), "egress is empty");
        if let Some(host) = self.egress.iter().find(|host| !is_hostname(host)) {
            bail!("egress host {host:?} is not a lowercase DNS name");
        }
        let endpoint = self.endpoint.map(Endpoint::parse).transpose()?;
        if let Some(endpoint) = &endpoint {
            ensure!(
                self.egress.contains(&endpoint.host),
                "endpoint = {:?} has host {}, which egress does not allow",
                endpoint.url,
                endpoint.host
            );
        }
        let credential = self.credential.resolve()?;
        if matches!(credential, Credential::ApiKey { .. }) {
            ensure!(
                endpoint.is_some(),
                "credential kind \"api-key\" needs an endpoint"
            );
        }
        Ok(Backend {
            harness: self.harness,
            driver,
            endpoint,
            credential,
            egress: self.egress,
        })
    }
}

impl RawCredential {
    fn resolve(self) -> Result<Credential> {
        let check_var = |var: String| {
            ensure!(
                is_env_var(&var),
                "credential var = {var:?} is not an environment variable name"
            );
            Ok(var)
        };
        Ok(match self {
            Self::EnvToken { var, file } => Credential::EnvToken {
                var: check_var(var)?,
                file: UserPath::parse("credential file", file)?,
            },
            Self::ApiKey { var, file } => Credential::ApiKey {
                var: check_var(var)?,
                file: UserPath::parse("credential file", file)?,
            },
            Self::CodexHome { dir } => Credential::CodexHome {
                dir: UserPath::parse("credential dir", dir)?,
            },
        })
    }
}

impl RawModel {
    fn resolve(self, backends: &BTreeMap<BackendId, Backend>) -> Result<Model> {
        ensure!(!self.efforts.is_empty(), "efforts is empty");
        let mut efforts = BTreeSet::new();
        for effort in self.efforts {
            ensure!(efforts.insert(effort), "efforts lists {effort} twice");
        }
        ensure!(!self.served_by.is_empty(), "served-by is empty");
        for (backend, api_model) in &self.served_by {
            ensure!(
                backends.contains_key(backend),
                "served-by names backend {backend}, which has no [backends.{backend}] table"
            );
            ensure!(!api_model.is_empty(), "served-by.{backend} is empty");
        }
        Ok(Model {
            efforts,
            served_by: self.served_by,
            prices: price_table(self.prices)?,
        })
    }
}

fn price_table(prices: Vec<RawPrice>) -> Result<BTreeMap<NaiveDate, Tiers>> {
    ensure!(!prices.is_empty(), "prices is empty");
    let mut bases = BTreeMap::new();
    let mut above: BTreeMap<NaiveDate, BTreeMap<u64, Rates>> = BTreeMap::new();
    for price in prices {
        let rates = price.rates()?;
        match price.above_prompt_tokens {
            None => ensure!(
                bases.insert(price.since, rates).is_none(),
                "prices since {} has two base tiers",
                price.since
            ),
            Some(tokens) => ensure!(
                above
                    .entry(price.since)
                    .or_default()
                    .insert(tokens, rates)
                    .is_none(),
                "prices since {} has two tiers above-prompt-tokens = {tokens}",
                price.since
            ),
        }
    }
    if let Some(day) = above.keys().find(|day| !bases.contains_key(day)) {
        bail!("prices since {day} has tiers above a prompt size but no base tier");
    }
    Ok(bases
        .into_iter()
        .map(|(day, base)| {
            let above = above.remove(&day).unwrap_or_default();
            (day, Tiers { base, above })
        })
        .collect())
}

impl RawPrice {
    fn rates(&self) -> Result<Rates> {
        let rate = |usd_per_million: f64| {
            ensure!(
                usd_per_million.is_finite() && usd_per_million >= 0.0,
                "prices since {}: rate {usd_per_million} is not a finite non-negative number",
                self.since
            );
            let picodollars = usd_per_million * 1e6;
            ensure!(
                (picodollars - picodollars.round()).abs() < 1e-6,
                "prices since {}: rate {usd_per_million} is finer than a picodollar per token",
                self.since
            );
            Ok(Rate(picodollars.round() as u64))
        };
        Ok(Rates {
            input: rate(self.input)?,
            output: rate(self.output)?,
            cache_read: rate(self.cache_read)?,
            cache_write: self.cache_write.map(rate).transpose()?,
            cache_write_1h: self.cache_write_1h.map(rate).transpose()?,
        })
    }
}
