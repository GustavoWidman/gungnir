//! Where and how one gungnir invocation ran, recorded once per invocation so every record
//! can name the session that produced it.
use crate::{store::write_json, trial::ResultsDir};
use chrono::{DateTime, Utc};
use eyre::{Result, WrapErr, ensure};
use serde::{Deserialize, Serialize};
use std::{fmt, fs, path::Path, process::Command};

/// `<UTC start time>-<pid>`: unique per invocation on one machine and sorted by start time.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(String);

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One invocation's provenance: the gungnir build, the repository state and the host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    id: SessionId,
    started: DateTime<Utc>,
    gungnir_version: String,
    git: Option<GitState>,
    kernel: String,
    cpu: Option<String>,
}

/// The repository's checked-out revision and whether the working tree differs from it,
/// untracked files included.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitState {
    pub revision: String,
    pub dirty: bool,
}

impl Session {
    /// Probes the host and the repository at `repo`. A tree without `.git`, such as a
    /// packaged source copy, records no git state.
    pub fn start(repo: &Path) -> Result<Self> {
        let started = Utc::now();
        let id = format!(
            "{}-{}",
            started.format("%Y%m%dT%H%M%SZ"),
            std::process::id()
        );
        Ok(Self {
            id: SessionId(id),
            started,
            gungnir_version: env!("CARGO_PKG_VERSION").to_owned(),
            git: git_state(repo)?,
            kernel: read_proc("/proc/sys/kernel/osrelease")?.trim().to_owned(),
            cpu: cpu_model(&read_proc("/proc/cpuinfo")?),
        })
    }

    pub fn id(&self) -> &SessionId {
        &self.id
    }

    pub fn git(&self) -> Option<&GitState> {
        self.git.as_ref()
    }

    /// Writes the session to its file under `results`.
    pub fn write(&self, results: &ResultsDir) -> Result<()> {
        write_json(&results.session(&self.id), self)
    }
}

fn git_state(repo: &Path) -> Result<Option<GitState>> {
    if !repo.join(".git").exists() {
        return Ok(None);
    }
    Ok(Some(GitState {
        revision: git(repo, &["rev-parse", "HEAD"])?,
        dirty: !git(repo, &["status", "--porcelain"])?.is_empty(),
    }))
}

fn git(repo: &Path, args: &[&str]) -> Result<String> {
    let command = format!("git {} in {}", args.join(" "), repo.display());
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .wrap_err_with(|| format!("running {command}"))?;
    ensure!(
        output.status.success(),
        "{command} failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    let stdout = String::from_utf8(output.stdout).wrap_err_with(|| command.clone())?;
    Ok(stdout.trim().to_owned())
}

fn read_proc(path: &str) -> Result<String> {
    fs::read_to_string(path).wrap_err_with(|| format!("reading {path}"))
}

/// The first `model name` in `/proc/cpuinfo`, which some architectures do not report.
pub(crate) fn cpu_model(cpuinfo: &str) -> Option<String> {
    cpuinfo.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        (key.trim() == "model name").then(|| value.trim().to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{store::read_json, testing::write};
    use tempfile::TempDir;

    #[test]
    fn a_session_is_written_under_sessions_and_reads_back() {
        let repo = TempDir::new().unwrap();
        let results = TempDir::new().unwrap();
        let session = Session::start(repo.path()).unwrap();
        assert_eq!(session.git(), None);
        assert!(!session.kernel.is_empty());
        assert_eq!(session.gungnir_version, env!("CARGO_PKG_VERSION"));

        session
            .write(&ResultsDir::new(results.path().to_owned()))
            .unwrap();
        let path = results
            .path()
            .join("sessions")
            .join(format!("{}.json", session.id()));
        assert_eq!(read_json::<Session>(&path).unwrap(), Some(session));
    }

    #[test]
    fn git_state_names_the_revision_and_notices_changes() {
        if Command::new("git").arg("--version").output().is_err() {
            eprintln!("skipped: git is not on PATH");
            return;
        }
        let repo = TempDir::new().unwrap();
        let run = |args: &[&str]| git(repo.path(), args).unwrap();
        run(&["init", "--quiet"]);
        write(repo.path(), "gungnir.toml", "");
        run(&["add", "gungnir.toml"]);
        run(&[
            "-c",
            "user.name=gungnir",
            "-c",
            "user.email=gungnir@localhost",
            "commit",
            "--quiet",
            "--message=fixture",
        ]);

        let clean = Session::start(repo.path()).unwrap();
        let state = clean.git().unwrap();
        assert_eq!(state.revision, run(&["rev-parse", "HEAD"]));
        assert!(!state.dirty);

        write(repo.path(), "scenarios/new/spec.md", "");
        assert!(Session::start(repo.path()).unwrap().git().unwrap().dirty);
    }
}
