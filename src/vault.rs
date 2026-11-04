//! Credentials. Each backend's credential is read from the file `gungnir.toml` names, handed
//! to one attempt with the exact strings to redact, and staged into an isolated home that is
//! removed after the attempt. A Codex login the CLI refreshed during an attempt is written
//! back to its master copy.
use crate::{
    catalog::{Catalog, Credential, Endpoint},
    harness::{CredentialMaterial, HomeFile},
    id::BackendId,
    secret::Secret,
    store::write_atomic,
};
use chrono::{DateTime, TimeDelta, Utc};
use eyre::{Result, WrapErr, ensure, eyre};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::{
    collections::BTreeMap,
    fs::{self, DirBuilder, File, OpenOptions},
    io::{ErrorKind, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
};
use tempfile::TempDir;

/// The file a Codex home keeps its login in.
const CODEX_LOGIN: &str = "auth.json";
/// Beside a Codex master login, the lock that serializes write-backs from every gungnir process.
const CODEX_LOCK: &str = "gungnir.lock";
/// How far ahead of this machine's clock a refreshed login's timestamp may be.
const CLOCK_SKEW: TimeDelta = TimeDelta::minutes(5);

/// The credentials of the backends one invocation uses, and the private directory their
/// isolated homes live in.
#[derive(Debug)]
pub struct Vault {
    backends: BTreeMap<BackendId, Entry>,
    root: PathBuf,
}

#[derive(Debug)]
struct Entry {
    source: Source,
    endpoint: Option<Endpoint>,
}

/// Where a backend's credential is read from, with `~` expanded.
#[derive(Debug)]
enum Source {
    /// A file whose contents, trimmed, are the value of `var`.
    Token { var: String, file: PathBuf },
    /// A Codex master login, which each attempt gets a copy of.
    Codex { login: PathBuf },
}

/// What one attempt authenticates with, read as the attempt starts.
#[derive(Debug)]
pub struct Key {
    material: CredentialMaterial,
    endpoint: Option<Endpoint>,
    secrets: Vec<Secret<String>>,
    /// The Codex master login `material` was read from.
    master: Option<PathBuf>,
}

impl Key {
    /// What the harness launches with.
    pub fn material(&self) -> &CredentialMaterial {
        &self.material
    }

    /// The backend's API base URL, always set for an `api-key` credential.
    pub fn endpoint(&self) -> Option<&Endpoint> {
        self.endpoint.as_ref()
    }

    /// Every string that must never reach a file gungnir keeps. For a Codex login, each token
    /// in it rather than the whole file.
    pub fn secrets(&self) -> &[Secret<String>] {
        &self.secrets
    }
}

impl Vault {
    /// Opens the credentials of `backends`, reading each once so that a missing or empty one
    /// fails here. `~` in a credential path is `home`. Isolated homes live under `root`, which
    /// must belong to this process alone: anything already in it was left by an interrupted
    /// run, and is removed.
    pub fn open<'a>(
        catalog: &Catalog,
        backends: impl IntoIterator<Item = &'a BackendId>,
        home: &Path,
        root: PathBuf,
    ) -> Result<Self> {
        let mut entries = BTreeMap::new();
        for id in backends {
            let backend = catalog
                .backend(id)
                .ok_or_else(|| eyre!("backend {id} has no [backends.{id}] table"))?;
            let source = match backend.credential() {
                Credential::EnvToken { var, file } | Credential::ApiKey { var, file } => {
                    Source::Token {
                        var: var.clone(),
                        file: file.expand(home),
                    }
                }
                Credential::CodexHome { dir } => Source::Codex {
                    login: dir.expand(home).join(CODEX_LOGIN),
                },
            };
            let endpoint = backend.endpoint().cloned();
            entries.insert(id.clone(), Entry { source, endpoint });
        }
        let vault = Self {
            backends: entries,
            root,
        };
        for id in vault.backends.keys() {
            vault
                .key(id)
                .wrap_err_with(|| format!("reading the credential of backend {id}"))?;
        }
        match fs::remove_dir_all(&vault.root) {
            Err(err) if err.kind() != ErrorKind::NotFound => {
                return Err(err).wrap_err_with(|| format!("removing {}", vault.root.display()));
            }
            _ => private_dir(&vault.root)?,
        }
        Ok(vault)
    }

    /// Reads `backend`'s credential as it is now, so each attempt starts from the latest
    /// login any earlier attempt wrote back.
    pub fn key(&self, backend: &BackendId) -> Result<Key> {
        let entry = self
            .backends
            .get(backend)
            .ok_or_else(|| eyre!("the vault holds no credential for backend {backend}"))?;
        let endpoint = entry.endpoint.clone();
        Ok(match &entry.source {
            Source::Token { var, file } => {
                let value = read_token(file)?;
                Key {
                    material: CredentialMaterial::Env {
                        var: var.clone(),
                        value: value.clone(),
                    },
                    endpoint,
                    secrets: vec![value],
                    master: None,
                }
            }
            Source::Codex { login } => {
                let Login { text, tokens, .. } = Login::read(login)?;
                Key {
                    material: CredentialMaterial::CodexAuth(text),
                    endpoint,
                    secrets: tokens,
                    master: Some(login.clone()),
                }
            }
        })
    }

    /// Creates an isolated home holding `files`, the files the launch for an attempt with
    /// `key` places in it. The home and everything in it are readable by this user alone.
    pub fn home(&self, key: &Key, files: &[HomeFile]) -> Result<Home> {
        let dir = tempfile::Builder::new()
            .prefix("attempt-")
            .tempdir_in(&self.root)
            .wrap_err_with(|| format!("creating a home under {}", self.root.display()))?;
        let home = dir.path().join("home");
        private_dir(&home)?;
        let mut copy = None;
        for file in files {
            let path = home.join(&file.path);
            if let Some(parent) = path.parent() {
                private_dir(parent)?;
            }
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
                .and_then(|mut opened| opened.write_all(file.contents.expose().as_bytes()))
                .wrap_err_with(|| format!("writing {}", path.display()))?;
            if let (Some(master), CredentialMaterial::CodexAuth(login)) =
                (&key.master, &key.material)
                && file.contents == *login
            {
                copy = Some(Copy {
                    path,
                    master: master.clone(),
                    original: login.clone(),
                    known: key.secrets.clone(),
                });
            }
        }
        if let Some(master) = &key.master {
            ensure!(
                copy.is_some(),
                "the launch places no copy of {} in the home",
                master.display()
            );
        }
        Ok(Home { dir, copy })
    }
}

/// An attempt's isolated home, inside a private directory that also holds the attempt's
/// egress socket. The directory is removed when the home is closed or dropped.
#[derive(Debug)]
pub struct Home {
    dir: TempDir,
    copy: Option<Copy>,
}

impl Home {
    /// The home directory, to mount as the agent's home.
    pub fn path(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    /// Where the attempt's egress proxy listens: beside the home, never inside it.
    pub fn socket(&self) -> PathBuf {
        self.dir.path().join("egress.sock")
    }

    /// Writes a login the CLI refreshed back to its master, then removes the home. Returns
    /// the secrets in the refreshed login that the key did not hold, which must be redacted
    /// too.
    pub fn close(self) -> Result<Vec<Secret<String>>> {
        let fresh = match &self.copy {
            Some(copy) => copy.settle()?,
            None => Vec::new(),
        };
        let path = self.dir.path().to_owned();
        self.dir
            .close()
            .wrap_err_with(|| format!("removing {}", path.display()))?;
        Ok(fresh)
    }
}

/// A Codex login copied into a home, and the master it came from.
#[derive(Debug)]
struct Copy {
    path: PathBuf,
    master: PathBuf,
    original: Secret<String>,
    known: Vec<Secret<String>>,
}

impl Copy {
    /// Writes the copy over the master when the CLI refreshed it more recently than the master
    /// was, holding the master's lock so that concurrent write-backs keep the newest login.
    /// Returns the copy's tokens that the original did not hold.
    fn settle(&self) -> Result<Vec<Secret<String>>> {
        let text = fs::read_to_string(&self.path)
            .wrap_err_with(|| format!("reading {}", self.path.display()))?;
        if text == *self.original.expose() {
            return Ok(Vec::new());
        }
        let copy = Login::parse(Secret::new(text))
            .wrap_err_with(|| format!("parsing the attempt's copy of {}", self.master.display()))?;
        let _lock = lock(&self.master.with_file_name(CODEX_LOCK))?;
        let master = Login::read(&self.master)?;
        if copy.refreshed > master.refreshed {
            ensure!(
                copy.account == master.account,
                "the attempt refreshed a login for another account than {}",
                self.master.display()
            );
            ensure!(
                copy.refreshed <= Some(Utc::now() + CLOCK_SKEW),
                "the attempt's login claims a last_refresh in the future"
            );
            write_atomic(&self.master, copy.text.expose().as_bytes())?;
        }
        Ok(copy
            .tokens
            .into_iter()
            .filter(|token| !self.known.contains(token))
            .collect())
    }
}

/// A Codex `auth.json`: its text, and the fields gungnir reads.
struct Login {
    text: Secret<String>,
    account: Option<String>,
    refreshed: Option<DateTime<Utc>>,
    /// The API key and every `*_token` value.
    tokens: Vec<Secret<String>>,
}

#[derive(Deserialize)]
struct RawLogin {
    #[serde(rename = "OPENAI_API_KEY")]
    api_key: Option<String>,
    tokens: Option<Map<String, Value>>,
    last_refresh: Option<DateTime<Utc>>,
}

impl Login {
    fn read(path: &Path) -> Result<Self> {
        let text =
            fs::read_to_string(path).wrap_err_with(|| format!("reading {}", path.display()))?;
        Self::parse(Secret::new(text)).wrap_err_with(|| format!("parsing {}", path.display()))
    }

    fn parse(text: Secret<String>) -> Result<Self> {
        let raw: RawLogin = serde_json::from_str(text.expose())?;
        let fields = raw.tokens.unwrap_or_default();
        let account = fields
            .get("account_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let tokens = fields
            .iter()
            .filter(|(name, _)| name.ends_with("_token"))
            .filter_map(|(_, value)| value.as_str().map(str::to_owned))
            .chain(raw.api_key)
            .filter(|token| !token.is_empty())
            .map(Secret::new)
            .collect();
        Ok(Self {
            text,
            account,
            refreshed: raw.last_refresh,
            tokens,
        })
    }
}

fn read_token(path: &Path) -> Result<Secret<String>> {
    let text = fs::read_to_string(path).wrap_err_with(|| format!("reading {}", path.display()))?;
    let token = text.trim();
    ensure!(!token.is_empty(), "{} is empty", path.display());
    Ok(Secret::new(token.to_owned()))
}

/// Creates `path` and any missing parents, each readable by this user alone.
fn private_dir(path: &Path) -> Result<()> {
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .wrap_err_with(|| format!("creating {}", path.display()))
}

/// Takes the lock at `path`, waiting for its holder.
fn lock(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .wrap_err_with(|| format!("opening {}", path.display()))?;
    file.lock()
        .wrap_err_with(|| format!("locking {}", path.display()))?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{self, id, write};
    use std::os::unix::fs::PermissionsExt;

    const LOGIN: &str = r#"{
  "auth_mode": "chatgpt",
  "OPENAI_API_KEY": null,
  "tokens": {
    "id_token": "eyJ.id.one",
    "access_token": "eyJ.access.one",
    "refresh_token": "rt.refresh.one",
    "account_id": "acct-1"
  },
  "last_refresh": "2026-11-03T20:04:12.123456789Z"
}"#;

    /// A user home with the fixture catalog's three credentials, and a runtime directory.
    struct Fixture {
        home: tempfile::TempDir,
        runtime: tempfile::TempDir,
    }

    fn fixture() -> Fixture {
        let home = tempfile::tempdir().unwrap();
        write(home.path(), "token", "sk-ant-oat01-secret\n");
        write(home.path(), "key", "glm-key");
        write(home.path(), "codex/auth.json", LOGIN);
        Fixture {
            home,
            runtime: tempfile::tempdir().unwrap(),
        }
    }

    impl Fixture {
        fn root(&self) -> PathBuf {
            self.runtime.path().join("gungnir/scope")
        }

        fn open(&self) -> Result<Vault> {
            let backends: Vec<BackendId> =
                ["anthropic", "openai", "zai"].map(id::<BackendId>).into();
            Vault::open(
                &testing::catalog(),
                &backends,
                self.home.path(),
                self.root(),
            )
        }

        fn master(&self) -> PathBuf {
            self.home.path().join("codex/auth.json")
        }

        /// A home for an attempt on `openai`, with the login where Codex expects it.
        fn codex_home(&self, vault: &Vault) -> (Key, Home) {
            let key = vault.key(&id("openai")).unwrap();
            let CredentialMaterial::CodexAuth(login) = key.material() else {
                panic!("expected a Codex login");
            };
            let files = [HomeFile {
                path: PathBuf::from(".codex/auth.json"),
                contents: login.clone(),
            }];
            let home = vault.home(&key, &files).unwrap();
            (key, home)
        }
    }

    fn exposed(secrets: &[Secret<String>]) -> Vec<&str> {
        let mut values: Vec<&str> = secrets.iter().map(|s| s.expose().as_str()).collect();
        values.sort_unstable();
        values
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    /// `LOGIN` refreshed at `at` with new tokens named by `suffix`.
    fn refreshed(at: &str, suffix: &str) -> String {
        LOGIN
            .replace("2026-11-03T20:04:12.123456789Z", at)
            .replace(".one", suffix)
    }

    #[test]
    fn token_files_become_secret_variables_and_api_keys_bring_their_endpoint() {
        let fixture = fixture();
        let vault = fixture.open().unwrap();

        let anthropic = vault.key(&id("anthropic")).unwrap();
        let CredentialMaterial::Env { var, value } = anthropic.material() else {
            panic!("expected a variable");
        };
        assert_eq!(
            (var.as_str(), value.expose().as_str()),
            ("TOKEN", "sk-ant-oat01-secret")
        );
        assert_eq!(exposed(anthropic.secrets()), ["sk-ant-oat01-secret"]);
        assert_eq!(anthropic.endpoint(), None);

        let zai = vault.key(&id("zai")).unwrap();
        assert_eq!(exposed(zai.secrets()), ["glm-key"]);
        assert_eq!(
            zai.endpoint().map(Endpoint::as_str),
            Some("https://api.z.ai/api/anthropic")
        );
    }

    #[test]
    fn a_missing_or_empty_credential_fails_at_open_naming_its_file() {
        let fixture = fixture();
        fs::remove_file(fixture.home.path().join("key")).unwrap();
        let message = format!("{:#}", fixture.open().unwrap_err());
        assert!(message.contains("backend zai"), "{message}");
        assert!(message.contains("key"), "{message}");

        write(fixture.home.path(), "key", "\n");
        let message = format!("{:#}", fixture.open().unwrap_err());
        assert!(message.contains("is empty"), "{message}");
    }

    #[test]
    fn a_codex_login_is_redacted_token_by_token() {
        let fixture = fixture();
        let vault = fixture.open().unwrap();
        let key = vault.key(&id("openai")).unwrap();
        assert_eq!(
            exposed(key.secrets()),
            ["eyJ.access.one", "eyJ.id.one", "rt.refresh.one"]
        );
        let CredentialMaterial::CodexAuth(login) = key.material() else {
            panic!("expected a Codex login");
        };
        assert_eq!(login.expose(), LOGIN);
    }

    #[test]
    fn homes_are_private_hold_the_launch_files_and_vanish_after_the_attempt() {
        let fixture = fixture();
        let vault = fixture.open().unwrap();
        assert_eq!(mode(&fixture.root()), 0o700);

        let (_, home) = fixture.codex_home(&vault);
        let copy = home.path().join(".codex/auth.json");
        assert_eq!(fs::read_to_string(&copy).unwrap(), LOGIN);
        assert_eq!(mode(&copy), 0o600);
        for dir in [home.path(), home.path().join(".codex")] {
            assert_eq!(mode(&dir), 0o700, "{}", dir.display());
        }
        assert_eq!(home.socket().parent(), home.path().parent());
        assert!(home.path().starts_with(fixture.root()));

        let closed = home.path();
        assert!(home.close().unwrap().is_empty());
        assert!(!closed.exists());

        let (_, dropped) = fixture.codex_home(&vault);
        let path = dropped.path();
        drop(dropped);
        assert!(!path.exists());
        assert_eq!(fs::read_dir(fixture.root()).unwrap().count(), 0);
    }

    #[test]
    fn a_refreshed_login_goes_back_to_the_master_only_when_newer() {
        let fixture = fixture();
        let vault = fixture.open().unwrap();
        let (_, first) = fixture.codex_home(&vault);
        let (_, second) = fixture.codex_home(&vault);
        let newest = refreshed("2026-11-03T22:00:00Z", ".two");
        write(&first.path(), ".codex/auth.json", &newest);
        write(
            &second.path(),
            ".codex/auth.json",
            refreshed("2026-11-03T21:00:00Z", ".three"),
        );

        let fresh = first.close().unwrap();
        assert_eq!(
            exposed(&fresh),
            ["eyJ.access.two", "eyJ.id.two", "rt.refresh.two"]
        );
        assert_eq!(fs::read_to_string(fixture.master()).unwrap(), newest);

        let fresh = second.close().unwrap();
        assert_eq!(fresh.len(), 3);
        assert_eq!(fs::read_to_string(fixture.master()).unwrap(), newest);
        assert_eq!(mode(&fixture.master()), 0o600);

        let next = vault.key(&id("openai")).unwrap();
        assert_eq!(
            exposed(next.secrets()),
            ["eyJ.access.two", "eyJ.id.two", "rt.refresh.two"]
        );
    }

    #[test]
    fn a_refreshed_login_for_another_account_is_refused() {
        let fixture = fixture();
        let vault = fixture.open().unwrap();
        let (_, home) = fixture.codex_home(&vault);
        write(
            &home.path(),
            ".codex/auth.json",
            refreshed("2026-11-03T22:00:00Z", ".two").replace("acct-1", "acct-2"),
        );
        let message = format!("{:#}", home.close().unwrap_err());
        assert!(message.contains("another account"), "{message}");
        assert_eq!(fs::read_to_string(fixture.master()).unwrap(), LOGIN);
    }

    #[test]
    fn opening_sweeps_what_an_interrupted_run_left() {
        let fixture = fixture();
        write(&fixture.root(), "attempt-1/home/.codex/auth.json", LOGIN);
        fixture.open().unwrap();
        assert_eq!(fs::read_dir(fixture.root()).unwrap().count(), 0);
    }
}
