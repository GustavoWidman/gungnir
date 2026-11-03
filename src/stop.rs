//! Two-stage interruption. The first Ctrl-C drains, the second kills in-flight sandboxes.

use std::sync::Arc;

use eyre::{Result, WrapErr};
use tokio::{
    signal::unix::{SignalKind, signal},
    sync::watch,
};

/// How far an interruption has gone. Stages only advance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    Running,
    /// Nothing new starts. Work already started runs to its end.
    Draining,
    /// Sandboxes still running are killed.
    Killing,
}

impl Stage {
    fn next(self) -> Self {
        match self {
            Self::Running => Self::Draining,
            Self::Draining | Self::Killing => Self::Killing,
        }
    }
}

/// A shared interruption signal. Clones observe and advance the same stage.
#[derive(Clone, Debug)]
pub struct Stop(Arc<watch::Sender<Stage>>);

impl Default for Stop {
    fn default() -> Self {
        Self(Arc::new(watch::channel(Stage::Running).0))
    }
}

impl Stop {
    /// A stop that advances one stage on each Ctrl-C. Must be called inside a Tokio runtime.
    pub fn on_ctrl_c() -> Result<Self> {
        // One stream registered up front: a fresh `ctrl_c()` per press could miss a quick second one.
        let mut interrupts = signal(SignalKind::interrupt()).wrap_err("listening for Ctrl-C")?;
        let stop = Self::default();
        let escalate = stop.clone();
        tokio::spawn(async move {
            while escalate.stage() < Stage::Killing && interrupts.recv().await.is_some() {
                escalate.escalate();
                match escalate.stage() {
                    Stage::Draining => eprintln!(
                        "gungnir: draining, nothing new starts; press Ctrl-C again to kill running sandboxes"
                    ),
                    _ => eprintln!("gungnir: killing running sandboxes"),
                }
            }
        });
        Ok(stop)
    }

    pub fn stage(&self) -> Stage {
        *self.0.borrow()
    }

    pub fn escalate(&self) {
        self.0.send_if_modified(|stage| {
            let next = stage.next();
            let advanced = next != *stage;
            *stage = next;
            advanced
        });
    }

    /// Resolves once the stop has reached `stage`.
    pub async fn reached(&self, stage: Stage) {
        let mut stages = self.0.subscribe();
        // The sender lives as long as `self`, so waiting cannot fail.
        let _ = stages.wait_for(|current| *current >= stage).await;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::time::timeout;

    use super::*;

    async fn settles(stop: &Stop, stage: Stage) -> bool {
        timeout(Duration::from_millis(50), stop.reached(stage))
            .await
            .is_ok()
    }

    #[tokio::test]
    async fn first_escalation_drains_and_second_kills() {
        let stop = Stop::default();
        let observer = stop.clone();
        assert_eq!(observer.stage(), Stage::Running);
        assert!(!settles(&observer, Stage::Draining).await);

        stop.escalate();
        assert_eq!(observer.stage(), Stage::Draining);
        assert!(settles(&observer, Stage::Draining).await);
        assert!(!settles(&observer, Stage::Killing).await);

        stop.escalate();
        assert_eq!(observer.stage(), Stage::Killing);
        assert!(settles(&observer, Stage::Killing).await);
    }

    #[tokio::test]
    async fn each_ctrl_c_advances_one_stage() -> Result<()> {
        use nix::sys::signal::{Signal, raise};

        let stop = Stop::on_ctrl_c()?;

        raise(Signal::SIGINT)?;
        assert!(settles(&stop, Stage::Draining).await);
        assert_eq!(stop.stage(), Stage::Draining);

        raise(Signal::SIGINT)?;
        assert!(settles(&stop, Stage::Killing).await);
        Ok(())
    }

    #[tokio::test]
    async fn killing_is_final() {
        let stop = Stop::default();
        for _ in 0..4 {
            stop.escalate();
        }
        assert_eq!(stop.stage(), Stage::Killing);
    }

    #[tokio::test]
    async fn a_waiter_wakes_when_the_stage_is_reached() {
        let stop = Stop::default();
        let waiter = tokio::spawn({
            let stop = stop.clone();
            async move { stop.reached(Stage::Killing).await }
        });
        stop.escalate();
        stop.escalate();
        timeout(Duration::from_secs(1), waiter)
            .await
            .expect("waiter woke")
            .expect("waiter task");
    }
}
