//! One attempt against real bubblewrap, systemd, the egress proxy and the harness CLIs, with
//! credentials the vendors refuse, so no quota is spent. They reach the vendors over the
//! network. Run them in the dev shell with `cargo test -- --ignored`.

use std::{fs, path::PathBuf};

use eyre::{Result, eyre};
use gungnir::{
    egress::{Record, Verdict},
    experiment::Experiment,
    id::AttemptNo,
    provenance::Session,
    run::{AgentExit, Attempted, Rig, RunRecord, Task},
    sandbox::Bubblewrap,
    stop::Stop,
    store::read_record,
    toolchains::Toolchains,
    trial::{Phase, ResultsDir, TrialDir},
    vault::Vault,
};
use tempfile::TempDir;

const CLAUDE_TOKEN: &str = "sk-ant-oat01-gungnir-test-not-a-token";
/// A login whose tokens have the shape Codex expects but that no vendor issued. The ID token's
/// payload names a test account.
const CODEX_LOGIN: &str = r#"{"OPENAI_API_KEY":null,"tokens":{"id_token":"eyJhbGciOiAiUlMyNTYiLCAidHlwIjogIkpXVCJ9.eyJlbWFpbCI6ICJ0ZXN0QGV4YW1wbGUuY29tIiwgImh0dHBzOi8vYXBpLm9wZW5haS5jb20vYXV0aCI6IHsiY2hhdGdwdF9hY2NvdW50X2lkIjogImFjY3QtdGVzdCIsICJjaGF0Z3B0X3BsYW5fdHlwZSI6ICJwbHVzIiwgImNoYXRncHRfdXNlcl9pZCI6ICJ1c2VyLXRlc3QifSwgImV4cCI6IDQxMDI0NDQ4MDAsICJpYXQiOiAxNzAwMDAwMDAwfQ.c2lnbmF0dXJl","access_token":"gungnir-test-access-not-a-token","refresh_token":"rt.gungnir-test-refresh-not-a-token","account_id":"acct-test"},"last_refresh":"2026-11-03T00:00:00Z"}"#;

/// What one attempt of the smoke experiment's `model` left behind.
struct Attempt {
    attempted: Attempted,
    trial: TrialDir,
    homes: PathBuf,
    _dirs: [TempDir; 3],
}

/// Makes one attempt of the smoke experiment's `model`, with `contents` as the credential
/// file `credential` under `~/.config/gungnir/`.
async fn attempt(model: &str, credential: &str, contents: &str) -> Result<Attempt> {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let experiment = Experiment::load(&repo.join("experiments/smoke.toml"))?;
    let trial = experiment
        .trials()
        .into_iter()
        .find(|trial| trial.system.model().as_str() == model)
        .ok_or_else(|| eyre!("smoke runs no {model}"))?;
    let toolchains = Toolchains::from_env()?;
    let backend = Bubblewrap::new(toolchains.base().clone())?;
    let home = TempDir::new()?;
    let file = home.path().join(".config/gungnir").join(credential);
    fs::create_dir_all(file.parent().unwrap())?;
    fs::write(&file, contents)?;
    let runtime = TempDir::new()?;
    let homes = runtime.path().join("gungnir");
    let vault = Vault::open(
        experiment.catalog(),
        [trial.system.backend()],
        home.path(),
        homes.clone(),
    )?;
    let results_dir = TempDir::new()?;
    let results = ResultsDir::new(results_dir.path().to_owned());
    let session = Session::start(&repo)?;
    let stop = Stop::default();
    let rig = Rig {
        results: &results,
        toolchains: &toolchains,
        vault: &vault,
        backend: &backend,
        session: session.id(),
        budget: experiment.budget(),
        stop: &stop,
    };
    let attempted = rig.attempt(&Task::of(&experiment, &trial)?).await?;
    Ok(Attempt {
        attempted,
        trial: results.trial(&trial),
        homes,
        _dirs: [home, runtime, results_dir],
    })
}

impl Attempt {
    /// Checks that the attempt committed a run whose agent ended in an error.
    fn assert_errored_run(&self) {
        let Attempted::Committed(record) = &self.attempted else {
            panic!("expected a committed run, got {:?}", self.attempted);
        };
        assert!(
            matches!(record.outcome().exit, AgentExit::Errored { .. }),
            "{:?}",
            record.outcome().exit
        );
        let committed = read_record::<RunRecord>(&self.trial.phase(Phase::Run)).unwrap();
        assert_eq!(committed.as_ref(), Some(&**record));
    }

    /// Whether the agent reached `host` through the proxy.
    fn reached(&self, host: &str) -> Result<bool> {
        let egress = fs::read_to_string(self.trial.attempt(AttemptNo::FIRST).egress())?;
        let records: Vec<Record> = egress
            .lines()
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()?;
        Ok(records.iter().any(|record| {
            record.host.as_deref() == Some(host) && record.verdict == Verdict::Allowed
        }))
    }

    /// Checks that `secret` is in no log the attempt kept and that its home is gone.
    fn assert_clean(&self, secret: &str) -> Result<()> {
        let attempt = self.trial.attempt(AttemptNo::FIRST);
        for file in [attempt.transcript(), attempt.stderr()] {
            let text = fs::read_to_string(&file)?;
            assert!(!text.contains(secret), "{} holds a secret", file.display());
        }
        assert_eq!(fs::read_dir(&self.homes)?.count(), 0);
        Ok(())
    }
}

#[tokio::test]
#[ignore = "needs bwrap, GUNGNIR_TOOLCHAINS and the network"]
async fn claude_code_refused_by_anthropic_commits_an_errored_run() -> Result<()> {
    let attempt = attempt("claude-haiku-5-5", "claude/oauth_token", CLAUDE_TOKEN).await?;
    attempt.assert_errored_run();
    assert!(attempt.reached("api.anthropic.com")?);
    attempt.assert_clean(CLAUDE_TOKEN)
}

#[tokio::test]
#[ignore = "needs bwrap, GUNGNIR_TOOLCHAINS and the network"]
async fn codex_refused_by_openai_commits_an_errored_run() -> Result<()> {
    let attempt = attempt("gpt-6-luna", "codex/auth.json", CODEX_LOGIN).await?;
    attempt.assert_errored_run();
    assert!(attempt.reached("chatgpt.com")?);
    attempt.assert_clean("gungnir-test-access-not-a-token")
}
