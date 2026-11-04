//! The transient systemd scope each sandbox runs in, and what systemd saw happen to it.

use std::{
    sync::{
        LazyLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use eyre::{Result, WrapErr, bail, ensure};
use tokio::process::Command;

/// How long systemd gets to retire a scope after its processes are gone.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(10);
const SETTLE_POLL: Duration = Duration::from_millis(10);

pub(super) struct Scope {
    unit: String,
}

impl Scope {
    /// A scope with a name no other sandbox of this machine has used since its systemd started:
    /// the counter is shared by every backend in the process, and the pid and launch time keep
    /// apart earlier runs whose failed scopes systemd may still hold.
    pub fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        static PREFIX: LazyLock<String> = LazyLock::new(|| {
            let launched = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            format!("gungnir-{}-{launched}", std::process::id())
        });
        let index = NEXT.fetch_add(1, Ordering::Relaxed);
        Self {
            unit: format!("{}-{index}", *PREFIX),
        }
    }

    /// The name `systemd-run --unit` takes, without the `.scope` suffix.
    pub fn unit(&self) -> &str {
        &self.unit
    }

    /// Waits until systemd has retired the scope, clears a failed one, and reports whether the
    /// kernel OOM-killed any process in it. systemd keeps a scope it stopped for an OOM kill
    /// as failed, so the verdict survives the processes.
    pub async fn settle(&self) -> Result<bool> {
        let deadline = Instant::now() + SETTLE_TIMEOUT;
        loop {
            match state(&self.show().await?) {
                State::Running if Instant::now() < deadline => {
                    tokio::time::sleep(SETTLE_POLL).await;
                }
                State::Running => bail!("scope {}.scope outlived its sandbox", self.unit),
                State::Gone { oom } => return Ok(oom),
                State::Failed { oom } => {
                    self.systemctl(&["reset-failed"]).await?;
                    return Ok(oom);
                }
            }
        }
    }

    async fn show(&self) -> Result<String> {
        self.systemctl(&["show", "--property=ActiveState", "--property=Result"])
            .await
    }

    async fn systemctl(&self, args: &[&str]) -> Result<String> {
        let unit = format!("{}.scope", self.unit);
        let output = Command::new("systemctl")
            .arg("--user")
            .args(args)
            .arg(&unit)
            .output()
            .await
            .wrap_err("running systemctl")?;
        ensure!(
            output.status.success(),
            "systemctl --user {} {unit} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

#[derive(Debug, PartialEq, Eq)]
enum State {
    Running,
    /// Inactive, or already collected: systemd reports a collected unit as inactive.
    Gone {
        oom: bool,
    },
    /// Kept by systemd until reset.
    Failed {
        oom: bool,
    },
}

/// Reads `systemctl show --property=ActiveState --property=Result` output.
fn state(show: &str) -> State {
    let property = |name: &str| {
        show.lines()
            .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
            .unwrap_or_default()
    };
    let oom = property("Result") == "oom-kill";
    match property("ActiveState") {
        "inactive" => State::Gone { oom },
        "failed" => State::Failed { oom },
        _ => State::Running,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_scope_state() {
        assert_eq!(
            state("ActiveState=active\nResult=success\n"),
            State::Running
        );
        assert_eq!(
            state("ActiveState=deactivating\nResult=success\n"),
            State::Running
        );
        assert_eq!(
            state("Result=success\nActiveState=inactive\n"),
            State::Gone { oom: false }
        );
        assert_eq!(
            state("ActiveState=failed\nResult=oom-kill\n"),
            State::Failed { oom: true }
        );
        assert_eq!(
            state("ActiveState=failed\nResult=timeout\n"),
            State::Failed { oom: false }
        );
    }

    #[test]
    fn scope_names_are_unique() {
        let first = Scope::next();
        let second = Scope::next();
        assert_ne!(first.unit(), second.unit());
        assert!(first.unit().starts_with("gungnir-"));
    }
}
