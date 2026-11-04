//! The lanes: one per backend, each starting its pending trials in order, up to the backend's
//! concurrency, behind a gate that only that backend's rate limits close.
use super::{
    AgentExit, Attempted, RunRecord,
    gate::{Event, Gate},
};
use crate::{
    id::BackendId,
    stop::{Stage, Stop},
    trial::TrialKey,
};
use chrono::Utc;
use eyre::{Result, eyre};
use futures_util::{StreamExt, stream::FuturesUnordered};
use std::{
    collections::{BTreeMap, VecDeque},
    num::NonZeroUsize,
    time::Duration,
};
use tokio::time::Instant;
use tracing::{error, info, warn};

/// What became of the attempts one invocation scheduled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    pub committed: usize,
    pub discarded: usize,
    pub failed: usize,
    pub interrupted: usize,
    /// Trials not started because the invocation drained first.
    pub unstarted: usize,
}

struct Lane {
    limit: usize,
    queue: VecDeque<TrialKey>,
    gate: Gate,
    running: usize,
}

/// Runs `attempt` for each of `trials`, in order within each backend's lane, until every trial
/// is committed or failed, or `stop` drains. A rate-limited trial goes back to the front of its
/// lane. A failed or interrupted one stays pending for the next invocation.
pub(super) async fn schedule(
    trials: Vec<TrialKey>,
    limits: &BTreeMap<BackendId, NonZeroUsize>,
    stop: &Stop,
    attempt: impl AsyncFn(&TrialKey) -> Result<Attempted>,
) -> Result<Tally> {
    let mut lanes: BTreeMap<&BackendId, Lane> = limits
        .iter()
        .map(|(backend, limit)| {
            let lane = Lane {
                limit: limit.get(),
                queue: VecDeque::new(),
                gate: Gate::Open,
                running: 0,
            };
            (backend, lane)
        })
        .collect();
    for trial in trials {
        let backend = trial.system.backend();
        lanes
            .get_mut(backend)
            .ok_or_else(|| eyre!("no concurrency limit for backend {backend}"))?
            .queue
            .push_back(trial);
    }
    let attempt = &attempt;
    let mut running = FuturesUnordered::new();
    let mut tally = Tally::default();
    loop {
        let now = Instant::now();
        if stop.stage() == Stage::Running {
            for (backend, lane) in &mut lanes {
                for _ in 0..lane.gate.room(now, lane.running, lane.limit) {
                    let Some(trial) = lane.queue.pop_front() else {
                        break;
                    };
                    if matches!(lane.gate, Gate::Paused { .. }) {
                        info!("{backend}: probing the quota with {trial}");
                    }
                    lane.gate = lane.gate.next(Event::Started, now);
                    lane.running += 1;
                    running.push(async move {
                        let result = attempt(&trial).await;
                        (trial, result)
                    });
                }
            }
        }
        let idle = stop.stage() != Stage::Running || lanes.values().all(|l| l.queue.is_empty());
        if running.is_empty() && idle {
            break;
        }
        let wake = lanes
            .values()
            .filter(|lane| !lane.queue.is_empty())
            .filter_map(|lane| lane.gate.wakes())
            .min();
        tokio::select! {
            Some((trial, result)) = running.next(), if !running.is_empty() => {
                let backend = trial.system.backend();
                let lane = lanes
                    .get_mut(backend)
                    .ok_or_else(|| eyre!("no lane for backend {backend}"))?;
                lane.running -= 1;
                let before = lane.gate;
                let event = settle(&mut tally, &trial, result);
                lane.gate = lane.gate.next(event, Instant::now());
                log_gate(backend, before, lane.gate);
                if let Event::Limited { .. } = event {
                    lane.queue.push_front(trial);
                }
            }
            () = sleep_until(wake), if wake.is_some() => {}
            () = stop.reached(Stage::Draining), if stop.stage() == Stage::Running => {}
            else => break,
        }
    }
    tally.unstarted = lanes.values().map(|lane| lane.queue.len()).sum();
    Ok(tally)
}

/// Counts and logs how an attempt of `trial` ended, and names it for the lane's gate.
fn settle(tally: &mut Tally, trial: &TrialKey, result: Result<Attempted>) -> Event {
    match result {
        Ok(Attempted::Committed(record)) => {
            tally.committed += 1;
            info!(
                "{trial}: committed attempt {}, {}",
                record.attempt(),
                summary(&record)
            );
            Event::Passed
        }
        Ok(Attempted::RateLimited { attempt, resets_at }) => {
            tally.discarded += 1;
            let resets_in = resets_at.and_then(|at| (at - Utc::now()).to_std().ok());
            match resets_at {
                Some(at) => warn!(
                    "{trial}: rate limited, attempt {attempt} discarded; quota resets at {at}"
                ),
                None => warn!("{trial}: rate limited, attempt {attempt} discarded"),
            }
            Event::Limited { resets_in }
        }
        Ok(Attempted::Interrupted { attempt }) => {
            tally.interrupted += 1;
            warn!("{trial}: attempt {attempt} interrupted");
            Event::Passed
        }
        Err(err) => {
            tally.failed += 1;
            error!("{trial}: attempt failed: {err:#}");
            Event::Passed
        }
    }
}

fn log_gate(backend: &BackendId, before: Gate, after: Gate) {
    match (before, after) {
        (_, Gate::Paused { until, .. }) if before.wakes() != Some(until) => {
            let pause = until.saturating_duration_since(Instant::now());
            warn!(
                "{backend}: paused for {} after a rate limit",
                humantime::format_duration(Duration::from_secs(pause.as_secs()))
            );
        }
        (Gate::Probing { .. }, Gate::Open) => info!("{backend}: open again"),
        _ => {}
    }
}

/// How the agent ended, its turns and tokens, and its wall time.
fn summary(record: &RunRecord) -> String {
    let outcome = record.outcome();
    let exit = match &outcome.exit {
        AgentExit::Completed => "completed".to_owned(),
        AgentExit::Refused => "refused".to_owned(),
        AgentExit::Errored { message } => format!("errored ({message})"),
        AgentExit::Crashed => "crashed".to_owned(),
        AgentExit::Killed => "killed".to_owned(),
    };
    let turns = outcome.turns.map_or_else(
        || "turns unknown".to_owned(),
        |turns| format!("{turns} turns"),
    );
    let total = outcome.total();
    let wall = Duration::from_secs(record.process().wall.as_secs());
    format!(
        "{exit}, {turns}, tokens {} input {} cache-read {} cache-write {} output, {}",
        total.input,
        total.cache_read,
        total.cache_write + total.cache_write_long,
        total.output,
        humantime::format_duration(wall)
    )
}

async fn sleep_until(wake: Option<Instant>) {
    match wake {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        experiment::Experiment,
        id::AttemptNo,
        run::fixture::{self, Repo},
        store::Digest,
    };
    use std::sync::Mutex;

    const MINUTE: Duration = Duration::from_secs(60);

    /// The fixture's 16-trial experiment: 8 on `anthropic` at concurrency 2 and 8 on `openai`
    /// at concurrency 1, and a record that every committed attempt returns.
    struct Bench {
        experiment: Experiment,
        record: RunRecord,
        t0: Instant,
        log: Mutex<Vec<Start>>,
    }

    /// An attempt that started, when, and how many of its lane's attempts it joined.
    #[derive(Clone, Debug)]
    struct Start {
        trial: TrialKey,
        at: Duration,
        alongside: usize,
    }

    impl Bench {
        fn new() -> Self {
            let repo = Repo::new();
            let experiment =
                Experiment::load(&repo.root.path().join("experiments/trial.toml")).unwrap();
            let fixture = repo.experiment();
            let record = fixture::record(
                &repo,
                &fixture::trial(&fixture, "opus"),
                Digest::of_bytes(b""),
            );
            Self {
                experiment,
                record,
                t0: Instant::now(),
                log: Mutex::default(),
            }
        }

        async fn schedule(
            &self,
            stop: &Stop,
            ending: impl Fn(&TrialKey, usize) -> (Duration, Attempted),
        ) -> Tally {
            let running: Mutex<BTreeMap<BackendId, usize>> = Mutex::default();
            schedule(
                self.experiment.trials(),
                self.experiment.concurrency(),
                stop,
                async |trial: &TrialKey| {
                    let backend = trial.system.backend().clone();
                    let alongside = {
                        let mut running = running.lock().unwrap();
                        let count = running.entry(backend.clone()).or_default();
                        *count += 1;
                        *count - 1
                    };
                    let earlier = {
                        let mut log = self.log.lock().unwrap();
                        let earlier = log.iter().filter(|start| &start.trial == trial).count();
                        log.push(Start {
                            trial: trial.clone(),
                            at: Instant::now() - self.t0,
                            alongside,
                        });
                        earlier
                    };
                    let (takes, attempted) = ending(trial, earlier);
                    tokio::time::sleep(takes).await;
                    *running.lock().unwrap().get_mut(&backend).unwrap() -= 1;
                    Ok(attempted)
                },
            )
            .await
            .unwrap()
        }

        fn committed(&self) -> Attempted {
            Attempted::Committed(Box::new(self.record.clone()))
        }

        /// The starts on `backend`, in order.
        fn starts(&self, backend: &str) -> Vec<Start> {
            self.log
                .lock()
                .unwrap()
                .iter()
                .filter(|start| start.trial.system.backend().as_str() == backend)
                .cloned()
                .collect()
        }

        fn trials(&self, backend: &str) -> Vec<TrialKey> {
            let trials = self.experiment.trials().into_iter();
            trials
                .filter(|trial| trial.system.backend().as_str() == backend)
                .collect()
        }
    }

    fn trials(starts: &[Start]) -> Vec<TrialKey> {
        starts.iter().map(|start| start.trial.clone()).collect()
    }

    #[tokio::test(start_paused = true)]
    async fn each_lane_starts_its_trials_in_order_up_to_its_concurrency() {
        let bench = Bench::new();
        let tally = bench
            .schedule(&Stop::default(), |_, _| (10 * MINUTE, bench.committed()))
            .await;
        assert_eq!(
            tally,
            Tally {
                committed: 16,
                ..Tally::default()
            }
        );
        for (backend, limit) in [("anthropic", 2), ("openai", 1)] {
            let starts = bench.starts(backend);
            assert_eq!(trials(&starts), bench.trials(backend), "{backend}");
            let most = starts.iter().map(|start| start.alongside + 1).max();
            assert_eq!(most, Some(limit), "{backend}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_rate_limit_pauses_only_its_lane_until_a_probe_reopens_it() {
        let bench = Bench::new();
        let first = bench.trials("anthropic")[0].clone();
        let tally = bench
            .schedule(&Stop::default(), |trial, earlier| {
                if trial == &first && earlier == 0 {
                    let resets_at = Utc::now() + chrono::TimeDelta::seconds(30 * 60 + 5);
                    let limited = Attempted::RateLimited {
                        attempt: AttemptNo::FIRST,
                        resets_at: Some(resets_at),
                    };
                    (5 * MINUTE, limited)
                } else {
                    (10 * MINUTE, bench.committed())
                }
            })
            .await;
        assert_eq!(
            tally,
            Tally {
                committed: 16,
                discarded: 1,
                ..Tally::default()
            }
        );

        let anthropic = bench.starts("anthropic");
        let minutes: Vec<u64> = anthropic.iter().map(|s| s.at.as_secs() / 60).collect();
        assert_eq!(minutes[..5], [0, 0, 36, 46, 46]);
        assert_eq!(anthropic[2].trial, first, "the limited trial probes first");
        assert_eq!(anthropic[2].alongside, 0, "the probe runs alone");
        let order = trials(&anthropic);
        assert_eq!(order[..2], bench.trials("anthropic")[..2]);
        assert_eq!(order[3..], bench.trials("anthropic")[2..]);

        let openai: Vec<u64> = bench
            .starts("openai")
            .iter()
            .map(|start| start.at.as_secs() / 60)
            .collect();
        assert_eq!(openai, [0, 10, 20, 30, 40, 50, 60, 70]);
    }

    #[tokio::test(start_paused = true)]
    async fn draining_starts_nothing_new_and_lets_running_attempts_finish() {
        let bench = Bench::new();
        let stop = Stop::default();
        let draining = async {
            tokio::time::sleep(MINUTE).await;
            stop.escalate();
        };
        let (tally, ()) = tokio::join!(
            bench.schedule(&stop, |_, _| (10 * MINUTE, bench.committed())),
            draining
        );
        assert_eq!(
            tally,
            Tally {
                committed: 3,
                unstarted: 13,
                ..Tally::default()
            }
        );
        assert_eq!(bench.log.lock().unwrap().len(), 3);
    }
}
