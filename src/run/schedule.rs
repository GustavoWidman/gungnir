//! The lanes: one per backend, each starting its pending trials in order, up to the backend's
//! concurrency, behind a gate that only that backend's rate limits close and, for a backend
//! with a quota source, a meter that holds the lane while its quota is spent. A trial whose
//! attempt is no sample goes back into its lane until one is.
use super::{
    AgentExit, Attempted, Reason, RunRecord,
    gate::{Event, Gate},
    meter::{self, Admit, Meter},
};
use crate::{
    id::BackendId,
    quota::Window,
    stop::{Stage, Stop},
    trial::TrialKey,
};
use chrono::{DateTime, Utc};
use eyre::{Result, eyre};
use futures_util::{StreamExt, stream::FuturesUnordered};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    num::{NonZeroU32, NonZeroUsize},
    time::Duration,
};
use tokio::time::Instant;
use tracing::{error, info, warn};

/// What became of the attempts and trials one invocation scheduled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tally {
    /// Attempts committed as their trial's sample.
    pub committed: usize,
    /// Attempts discarded as no sample.
    pub discarded: usize,
    /// Attempts that failed before they could be committed or discarded.
    pub failed: usize,
    /// Trials still without a committed run when the invocation ended.
    pub pending: usize,
}

/// How much of each backend's account one invocation may spend. A backend whose guard trips
/// starts nothing more; its running attempts finish and its other trials stay pending.
#[derive(Clone, Debug, Default)]
pub struct Guard {
    /// Stop a backend once any of its quota windows reports at least this fraction used.
    pub ceilings: BTreeMap<BackendId, Ceiling>,
    /// Stop starting attempts on a backend at this time.
    pub deadlines: BTreeMap<BackendId, DateTime<Utc>>,
    /// Stop a backend after this many attempts in a row ended in an error, a crash, a sandbox
    /// fault or a failure of gungnir's own, since an expired login or an outage would otherwise
    /// turn every remaining trial into an error.
    pub max_errors: Option<u32>,
    /// Leave a trial pending once this many of its attempts were discarded or failed, rate
    /// limits aside, so that a trial its agent can never finish does not run forever.
    pub max_attempts: Option<NonZeroU32>,
    /// Start nothing on a backend whose quota source reads a window at least this fraction
    /// used, until the window resets or a reading shows it below. [`meter::HOLD`] for a backend
    /// not named.
    pub holds: BTreeMap<BackendId, f64>,
}

/// A quota ceiling on one window, or on every window when `window` is `None`, counted only from
/// attempts that started, and readings taken, at or after `from`, so that a ceiling meant for a
/// quota window that has not reset yet ignores the window before it.
#[derive(Clone, Debug, PartialEq)]
pub struct Ceiling {
    pub window: Option<String>,
    pub fraction: f64,
    pub from: Option<DateTime<Utc>>,
}

/// The quota windows one attempt or one reading reported, each with the fraction used, and
/// when that attempt started or that reading was taken.
struct Used<'a> {
    windows: Vec<(&'a str, f64)>,
    at: DateTime<Utc>,
}

impl Guard {
    /// The fraction of a quota window at which a metered `backend` holds.
    fn hold(&self, backend: &BackendId) -> f64 {
        self.holds.get(backend).copied().unwrap_or(meter::HOLD)
    }

    /// Why `backend` must start nothing more, given the quota its latest attempt or reading
    /// reported, the errors in a row it has had, and the time.
    fn stops(
        &self,
        backend: &BackendId,
        quota: Option<&Used>,
        errors: u32,
        now: DateTime<Utc>,
    ) -> Option<String> {
        if let (Some(ceiling), Some(Used { windows, at })) = (self.ceilings.get(backend), quota)
            && ceiling.from.is_none_or(|from| *at >= from)
            && let Some((window, used)) = windows
                .iter()
                .filter(|(window, _)| ceiling.window.as_deref().is_none_or(|only| only == *window))
                .find(|(_, used)| *used >= ceiling.fraction)
        {
            return Some(format!(
                "its {window} quota is {:.0}% used, at or past the {:.0}% ceiling",
                used * 100.0,
                ceiling.fraction * 100.0
            ));
        }
        if let Some(deadline) = self.deadlines.get(backend)
            && now >= *deadline
        {
            return Some(format!("its deadline {deadline} has passed"));
        }
        if let Some(max) = self.max_errors
            && errors >= max
        {
            return Some(format!("{errors} attempts in a row errored or failed"));
        }
        None
    }
}

struct Lane {
    limit: usize,
    queue: VecDeque<TrialKey>,
    gate: Gate,
    running: usize,
    /// Attempts in a row that ended in an error, a crash, a sandbox fault or a failure of
    /// gungnir's own.
    errors: u32,
    /// Why the lane starts nothing more, once its guard tripped.
    stopped: Option<String>,
    /// What the lane knows of its quota, when its backend has a quota source.
    meter: Option<Meter>,
}

impl Lane {
    /// When the lane may next start an attempt or read its meter, if anything but an attempt
    /// or a read ending can let it.
    fn wakes(&self, now: Instant) -> Option<Instant> {
        let room = self.gate.room(now, self.running, self.limit) > 0;
        match self.meter.as_ref().map(|meter| meter.admit(now, room)) {
            Some(Admit::Hold { until }) => until,
            Some(Admit::Read) => Some(now),
            Some(Admit::Go) | None => self.gate.wakes(),
        }
    }
}

/// Where a trial whose attempt left it pending goes back into its lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Retry {
    /// Ahead of the lane's other trials: the account ended the attempt, not the trial.
    First,
    /// Behind them, so that the lane's other trials run between this one's attempts.
    Last,
}

/// Runs `attempt` for each of `trials`, in order within each backend's lane, until every trial
/// is committed, `stop` drains, or the guard stops each lane that still has trials. A trial
/// whose attempt was not committed goes back into its lane as [`retry`] says, until it has
/// reached the guard's cap on attempts. The lane of each backend in `metered` takes its quota
/// readings from `read`.
pub(super) async fn schedule(
    trials: Vec<TrialKey>,
    limits: &BTreeMap<BackendId, NonZeroUsize>,
    guard: &Guard,
    stop: &Stop,
    attempt: impl AsyncFn(&TrialKey) -> Result<Attempted>,
    metered: &BTreeSet<BackendId>,
    read: impl AsyncFn(&BackendId) -> Result<Vec<Window>>,
) -> Result<Tally> {
    let scheduled = trials.len();
    let mut lanes: BTreeMap<&BackendId, Lane> = limits
        .iter()
        .map(|(backend, limit)| {
            let lane = Lane {
                limit: limit.get(),
                queue: VecDeque::new(),
                gate: Gate::Open,
                running: 0,
                errors: 0,
                stopped: None,
                meter: metered
                    .contains(backend)
                    .then(|| Meter::new(guard.hold(backend))),
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
    let read = &read;
    let mut running = FuturesUnordered::new();
    let mut reads = FuturesUnordered::new();
    let mut tally = Tally::default();
    let mut retried: HashMap<TrialKey, u32> = HashMap::new();
    loop {
        let now = Instant::now();
        if stop.stage() == Stage::Running {
            for (&backend, lane) in &mut lanes {
                if lane.stopped.is_none()
                    && let Some(reason) = guard.stops(backend, None, lane.errors, Utc::now())
                {
                    warn!("{backend}: starting nothing more, because {reason}");
                    lane.stopped = Some(reason);
                }
                if lane.stopped.is_some() || lane.queue.is_empty() {
                    continue;
                }
                let room = lane.gate.room(now, lane.running, lane.limit);
                if let Some(meter) = &mut lane.meter {
                    match meter.admit(now, room > 0) {
                        Admit::Go => {}
                        Admit::Read => {
                            meter.begin();
                            reads.push(async move { (backend, read(backend).await) });
                            continue;
                        }
                        Admit::Hold { .. } => continue,
                    }
                }
                for _ in 0..room {
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
        let idle = stop.stage() != Stage::Running
            || lanes
                .values()
                .all(|lane| lane.queue.is_empty() || lane.stopped.is_some());
        if running.is_empty() && idle {
            break;
        }
        let waiting = || {
            lanes.iter().filter(|(_, lane)| {
                stop.stage() == Stage::Running && !lane.queue.is_empty() && lane.stopped.is_none()
            })
        };
        let deadline = waiting()
            .filter_map(|(backend, _)| guard.deadlines.get(*backend))
            .filter_map(|at| (*at - Utc::now()).to_std().ok())
            .map(|left| Instant::now() + left)
            .min();
        let wake = waiting()
            .filter_map(|(_, lane)| lane.wakes(Instant::now()))
            .chain(deadline)
            .min();
        tokio::select! {
            Some((trial, result)) = running.next(), if !running.is_empty() => {
                let backend = trial.system.backend();
                let lane = lanes
                    .get_mut(backend)
                    .ok_or_else(|| eyre!("no lane for backend {backend}"))?;
                lane.running -= 1;
                lane.errors = errors_after(lane.errors, &result);
                let quota = match &result {
                    Ok(Attempted::Committed(record)) => Some(Used {
                        windows: record
                            .outcome()
                            .quota
                            .iter()
                            .map(|(window, quota)| (window.as_str(), quota.last))
                            .collect(),
                        at: record.started(),
                    }),
                    _ => None,
                };
                if let Some(Used { windows, .. }) = &quota {
                    for (window, used) in windows {
                        info!("{backend}: {window} quota {:.0}% used", used * 100.0);
                    }
                }
                if lane.stopped.is_none()
                    && let Some(reason) = guard.stops(
                        backend,
                        quota.as_ref(),
                        lane.errors,
                        Utc::now(),
                    )
                {
                    warn!("{backend}: starting nothing more, because {reason}");
                    lane.stopped = Some(reason);
                }
                let before = lane.gate;
                let again = retry(&result);
                let event = settle(&mut tally, &trial, result);
                lane.gate = lane.gate.next(event, Instant::now());
                log_gate(backend, before, lane.gate);
                if let (Event::Limited { resets_in: None }, Some(meter)) = (event, &mut lane.meter) {
                    meter.limited();
                }
                match again {
                    Some(Retry::First) => lane.queue.push_front(trial),
                    Some(Retry::Last) => {
                        let tries = retried.entry(trial.clone()).or_default();
                        *tries += 1;
                        if guard.max_attempts.is_some_and(|max| *tries >= max.get()) {
                            warn!(
                                "{trial}: {tries} attempts in this invocation were no sample, \
                                 the cap; leaving it pending"
                            );
                        } else {
                            lane.queue.push_back(trial);
                        }
                    }
                    None => {}
                }
            }
            Some((backend, readings)) = reads.next(), if !reads.is_empty() => {
                let lane = lanes
                    .get_mut(backend)
                    .ok_or_else(|| eyre!("no lane for backend {backend}"))?;
                let Some(meter) = &mut lane.meter else {
                    return Err(eyre!("backend {backend} read a quota it has no source for"));
                };
                let before = meter.clone();
                let windows = readings.unwrap_or_else(|err| {
                    warn!(
                        "{backend}: reading the quota failed, so the lane goes on unmetered \
                         until the next reading: {err:#}"
                    );
                    Vec::new()
                });
                let read = Utc::now();
                let used = Used {
                    windows: windows.iter().map(|w| (w.name.as_str(), w.used)).collect(),
                    at: read,
                };
                let stops = guard.stops(backend, Some(&used), lane.errors, read);
                meter.record(Instant::now(), read, windows);
                log_reading(backend, &before, meter);
                if lane.stopped.is_none()
                    && let Some(reason) = stops
                {
                    warn!("{backend}: starting nothing more, because {reason}");
                    lane.stopped = Some(reason);
                }
            }
            () = sleep_until(wake), if wake.is_some() => {}
            () = stop.reached(Stage::Draining), if stop.stage() == Stage::Running => {}
            else => break,
        }
    }
    tally.pending = scheduled - tally.committed;
    Ok(tally)
}

/// Where the trial of an attempt that ended as `result` goes back into its lane, if anywhere.
/// A stop leaves it pending, since the invocation is ending.
fn retry(result: &Result<Attempted>) -> Option<Retry> {
    match result {
        Ok(Attempted::Committed(_)) => None,
        Ok(Attempted::Discarded { reason, .. }) => match reason {
            Reason::RateLimited { .. } => Some(Retry::First),
            Reason::Stopped => None,
            Reason::Errored { .. }
            | Reason::Crashed
            | Reason::TimedOut { .. }
            | Reason::Fault { .. } => Some(Retry::Last),
        },
        Err(_) => Some(Retry::Last),
    }
}

/// The errors in a row after an attempt that ended as `result`, from `errors` before it. An
/// error, a crash, a fault or a failure of gungnir's own adds one, a session that ran until it
/// finished or ran out of time resets the count, and a rate limit or a stop says nothing either
/// way.
fn errors_after(errors: u32, result: &Result<Attempted>) -> u32 {
    match result {
        Ok(Attempted::Committed(_)) => 0,
        Ok(Attempted::Discarded { reason, .. }) => match reason {
            Reason::Errored { .. } | Reason::Crashed | Reason::Fault { .. } => errors + 1,
            Reason::TimedOut { .. } => 0,
            Reason::RateLimited { .. } | Reason::Stopped => errors,
        },
        Err(_) => errors + 1,
    }
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
        Ok(Attempted::Discarded { attempt, reason }) => {
            tally.discarded += 1;
            warn!("{trial}: attempt {attempt} discarded, {reason}");
            match reason {
                Reason::RateLimited { resets_at } => Event::Limited {
                    resets_in: resets_at.and_then(|at| (at - Utc::now()).to_std().ok()),
                },
                _ => Event::Passed,
            }
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

/// Logs each window of `after`, the meter just read, whose use differs from `before`, and the
/// hold when the reading starts one.
fn log_reading(backend: &BackendId, before: &Meter, after: &Meter) {
    let percent = |window: &Window| (window.used * 100.0).round();
    for window in after.windows() {
        let unchanged = before
            .windows()
            .any(|old| old.name == window.name && percent(old) == percent(window));
        if !unchanged {
            info!(
                "{backend}: {} quota {:.0}% used",
                window.name,
                percent(window)
            );
        }
    }
    if let Some(spent) = after.spent()
        && before.spent().is_none()
    {
        let reset = spent
            .resets_at
            .map_or_else(|| "a time it does not say".to_owned(), |at| at.to_string());
        warn!(
            "{backend}: holding, starting nothing new until a reading shows its {} quota, {:.0}% \
             used, below the {:.0}% hold; it resets at {reset}",
            spent.name,
            percent(spent),
            after.hold() * 100.0
        );
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
        run::{
            fixture::{self, Repo},
            records::Timeout,
        },
        store::Digest,
    };
    use std::sync::Mutex;

    const MINUTE: Duration = Duration::from_secs(60);

    /// The fixture's 16-trial experiment: 8 on `anthropic` at concurrency 2 and 8 on `openai`
    /// at concurrency 1, and a record that every committed attempt returns.
    struct Bench {
        experiment: Experiment,
        record: RunRecord,
        guard: Guard,
        t0: Instant,
        log: Mutex<Vec<Start>>,
        /// When each quota reading was taken.
        reads: Mutex<Vec<Duration>>,
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
                guard: Guard::default(),
                t0: Instant::now(),
                log: Mutex::default(),
                reads: Mutex::default(),
            }
        }

        async fn schedule(
            &self,
            stop: &Stop,
            ending: impl Fn(&TrialKey, usize) -> (Duration, Result<Attempted>),
        ) -> Tally {
            self.metered(stop, ending, None::<fn(Duration) -> Result<Vec<Window>>>)
                .await
        }

        /// Schedules as [`Bench::schedule`] does, with `anthropic` metered by `meter` when
        /// there is one, which reads its quota at each time since the start.
        async fn metered(
            &self,
            stop: &Stop,
            ending: impl Fn(&TrialKey, usize) -> (Duration, Result<Attempted>),
            meter: Option<impl Fn(Duration) -> Result<Vec<Window>>>,
        ) -> Tally {
            let running: Mutex<BTreeMap<BackendId, usize>> = Mutex::default();
            let metered = match meter {
                Some(_) => BTreeSet::from([backend("anthropic")]),
                None => BTreeSet::new(),
            };
            schedule(
                self.experiment.trials(),
                self.experiment.concurrency(),
                &self.guard,
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
                    attempted
                },
                &metered,
                async |backend: &BackendId| {
                    assert_eq!(backend.as_str(), "anthropic");
                    let at = Instant::now() - self.t0;
                    self.reads.lock().unwrap().push(at);
                    meter.as_ref().expect("a meter")(at)
                },
            )
            .await
            .unwrap()
        }

        fn committed(&self) -> Result<Attempted> {
            Ok(Attempted::Committed(Box::new(self.record.clone())))
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

    fn minutes(starts: &[Start]) -> Vec<u64> {
        starts.iter().map(|start| start.at.as_secs() / 60).collect()
    }

    /// A 5-hour window `used` at `at` since the start, which resets `resets` since the start.
    fn five_hours(used: f64, at: Duration, resets: Duration) -> Vec<Window> {
        let left = chrono::TimeDelta::from_std(resets.saturating_sub(at)).unwrap();
        vec![Window {
            name: "5h".to_owned(),
            used,
            resets_at: Some(Utc::now() + left),
        }]
    }

    fn discarded(reason: Reason) -> Result<Attempted> {
        Ok(Attempted::Discarded {
            attempt: AttemptNo::FIRST,
            reason,
        })
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
                    let reason = Reason::RateLimited {
                        resets_at: Some(resets_at),
                    };
                    (5 * MINUTE, discarded(reason))
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
                pending: 13,
                ..Tally::default()
            }
        );
        assert_eq!(bench.log.lock().unwrap().len(), 3);
    }

    fn backend(name: &str) -> BackendId {
        name.parse().unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn a_passed_deadline_stops_only_its_backend() {
        let mut bench = Bench::new();
        let past = Utc::now() - chrono::Duration::minutes(1);
        bench.guard.deadlines.insert(backend("openai"), past);
        let tally = bench
            .schedule(&Stop::default(), |_, _| (10 * MINUTE, bench.committed()))
            .await;
        assert_eq!(tally.committed, 8);
        assert_eq!(tally.pending, 8);
        assert!(bench.starts("openai").is_empty());
        assert_eq!(bench.starts("anthropic").len(), 8);
    }

    #[tokio::test(start_paused = true)]
    async fn a_quota_at_its_ceiling_stops_the_backend_after_running_attempts_finish() {
        let mut bench = Bench::new();
        bench.guard.ceilings.insert(
            backend("anthropic"),
            Ceiling {
                window: Some("seven_day".to_owned()),
                fraction: 0.75,
                from: None,
            },
        );
        let mut spent = bench.record.clone();
        spent.outcome.quota.insert(
            "seven_day".to_owned(),
            crate::run::Quota {
                first: 0.7,
                last: 0.76,
            },
        );
        let tally = bench
            .schedule(&Stop::default(), |trial, _| {
                let record = if trial.system.backend().as_str() == "anthropic" {
                    spent.clone()
                } else {
                    bench.record.clone()
                };
                (10 * MINUTE, Ok(Attempted::Committed(Box::new(record))))
            })
            .await;
        assert_eq!(bench.starts("anthropic").len(), 2, "the two running finish");
        assert_eq!(bench.starts("openai").len(), 8);
        assert_eq!(tally.committed, 10);
        assert_eq!(tally.pending, 6);
    }

    #[tokio::test(start_paused = true)]
    async fn a_ceiling_ignores_attempts_that_started_before_it_applies() {
        let mut bench = Bench::new();
        let later = bench.record.started() + chrono::Duration::hours(1);
        bench.guard.ceilings.insert(
            backend("anthropic"),
            Ceiling {
                window: None,
                fraction: 0.35,
                from: Some(later),
            },
        );
        let mut spent = bench.record.clone();
        spent.outcome.quota.insert(
            "7d".to_owned(),
            crate::run::Quota {
                first: 0.9,
                last: 0.95,
            },
        );
        let tally = bench
            .schedule(&Stop::default(), |_, _| {
                (
                    10 * MINUTE,
                    Ok(Attempted::Committed(Box::new(spent.clone()))),
                )
            })
            .await;
        assert_eq!(tally.committed, 16);
    }

    #[tokio::test(start_paused = true)]
    async fn errors_in_a_row_stop_the_backend_and_a_clean_attempt_resets_the_count() {
        let mut bench = Bench::new();
        bench.guard.max_errors = Some(3);
        let errored = Reason::Errored {
            message: "authentication failed".to_owned(),
        };
        let tally = bench
            .schedule(&Stop::default(), |trial, _| {
                let attempted = if trial.system.backend().as_str() == "openai" {
                    discarded(errored.clone())
                } else {
                    bench.committed()
                };
                (10 * MINUTE, attempted)
            })
            .await;
        assert_eq!(bench.starts("openai").len(), 3);
        assert_eq!(bench.starts("anthropic").len(), 8);
        assert_eq!(tally.pending, 8);
    }

    #[tokio::test(start_paused = true)]
    async fn a_trial_whose_attempt_is_no_sample_runs_again_after_the_rest_of_its_lane() {
        type Ending = fn() -> Result<Attempted>;
        let endings: [(&str, Ending); 6] = [
            ("errored", || {
                discarded(Reason::Errored {
                    message: "overloaded".to_owned(),
                })
            }),
            ("crashed", || discarded(Reason::Crashed)),
            ("wall budget", || {
                discarded(Reason::TimedOut {
                    budget: Timeout::Wall,
                })
            }),
            ("idle budget", || {
                discarded(Reason::TimedOut {
                    budget: Timeout::Idle,
                })
            }),
            ("sandbox fault", || {
                discarded(Reason::Fault {
                    message: "sandbox fault: no scope".to_owned(),
                })
            }),
            ("failed", || Err(eyre!("disk full"))),
        ];
        for (label, ending) in endings {
            let bench = Bench::new();
            let first = bench.trials("anthropic")[0].clone();
            let tally = bench
                .schedule(&Stop::default(), |trial, earlier| {
                    if trial == &first && earlier == 0 {
                        (5 * MINUTE, ending())
                    } else {
                        (10 * MINUTE, bench.committed())
                    }
                })
                .await;
            assert_eq!(tally.committed, 16, "{label}");
            assert_eq!(tally.pending, 0, "{label}");
            let mut expected = bench.trials("anthropic");
            expected.push(first);
            assert_eq!(trials(&bench.starts("anthropic")), expected, "{label}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_stopped_attempt_leaves_its_trial_pending() {
        let bench = Bench::new();
        let first = bench.trials("anthropic")[0].clone();
        let tally = bench
            .schedule(&Stop::default(), |trial, _| {
                if trial == &first {
                    (5 * MINUTE, discarded(Reason::Stopped))
                } else {
                    (10 * MINUTE, bench.committed())
                }
            })
            .await;
        assert_eq!(
            trials(&bench.starts("anthropic")),
            bench.trials("anthropic")
        );
        assert_eq!(
            (tally.committed, tally.discarded, tally.pending),
            (15, 1, 1)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_trial_is_left_pending_once_its_attempts_reach_the_cap() {
        let mut bench = Bench::new();
        bench.guard.max_attempts = NonZeroU32::new(3);
        let first = bench.trials("anthropic")[0].clone();
        let tally = bench
            .schedule(&Stop::default(), |trial, _| {
                if trial == &first {
                    (5 * MINUTE, discarded(Reason::Crashed))
                } else {
                    (10 * MINUTE, bench.committed())
                }
            })
            .await;
        let tries = bench
            .starts("anthropic")
            .iter()
            .filter(|start| start.trial == first)
            .count();
        assert_eq!(tries, 3);
        assert_eq!(
            tally,
            Tally {
                committed: 15,
                discarded: 3,
                failed: 0,
                pending: 1,
            }
        );
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limits_do_not_count_towards_the_cap() {
        let mut bench = Bench::new();
        bench.guard.max_attempts = NonZeroU32::new(1);
        let first = bench.trials("anthropic")[0].clone();
        let tally = bench
            .schedule(&Stop::default(), |trial, earlier| {
                if trial == &first && earlier < 2 {
                    let reason = Reason::RateLimited { resets_at: None };
                    (MINUTE, discarded(reason))
                } else {
                    (10 * MINUTE, bench.committed())
                }
            })
            .await;
        assert_eq!((tally.committed, tally.pending), (16, 0));
    }

    #[tokio::test(start_paused = true)]
    async fn a_metered_lane_starts_nothing_while_its_quota_is_at_the_hold() {
        let bench = Bench::new();
        let reset = 30 * MINUTE;
        let meter = |at: Duration| {
            let used = if at < reset { 0.8 } else { 0.1 };
            Ok(five_hours(
                used,
                at,
                reset + 300 * MINUTE * u32::from(at >= reset),
            ))
        };
        let tally = bench
            .metered(
                &Stop::default(),
                |_, _| (10 * MINUTE, bench.committed()),
                Some(meter),
            )
            .await;
        assert_eq!(tally.committed, 16);
        assert_eq!(
            minutes(&bench.starts("anthropic")),
            [30, 30, 40, 40, 50, 50, 60, 60]
        );
        assert_eq!(
            minutes(&bench.starts("openai")),
            [0, 10, 20, 30, 40, 50, 60, 70],
            "only the metered lane holds"
        );
        let reads = bench.reads.lock().unwrap().clone();
        assert!(
            reads
                .windows(2)
                .all(|pair| pair[1] - pair[0] >= meter::REFRESH),
            "{reads:?}"
        );
        assert_eq!(reads.first(), Some(&Duration::ZERO));
    }

    #[tokio::test(start_paused = true)]
    async fn a_rate_limit_without_a_reset_waits_for_the_spent_window_the_meter_reads() {
        let bench = Bench::new();
        let first = bench.trials("anthropic")[0].clone();
        let reset = 40 * MINUTE;
        let meter = |at: Duration| {
            let used = match at {
                at if at < 5 * MINUTE => 0.5,
                at if at < reset => 1.0,
                _ => 0.0,
            };
            Ok(five_hours(used, at, reset))
        };
        let tally = bench
            .metered(
                &Stop::default(),
                |trial, earlier| {
                    if trial == &first && earlier == 0 {
                        let reason = Reason::RateLimited { resets_at: None };
                        (5 * MINUTE, discarded(reason))
                    } else {
                        (10 * MINUTE, bench.committed())
                    }
                },
                Some(meter),
            )
            .await;
        assert_eq!((tally.committed, tally.discarded), (16, 1));
        let anthropic = bench.starts("anthropic");
        assert_eq!(minutes(&anthropic), [0, 0, 40, 50, 50, 60, 60, 70, 70]);
        assert_eq!(anthropic[2].trial, first, "the limited trial probes first");
        assert_eq!(anthropic[2].alongside, 0, "the probe runs alone");
        let reads = bench.reads.lock().unwrap().clone();
        assert!(
            reads.contains(&(5 * MINUTE)),
            "the rate limit is read at once: {reads:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_lane_whose_quota_cannot_be_read_goes_on_as_if_unmetered() {
        let bench = Bench::new();
        let meter = |_: Duration| Err(eyre!("HTTP 503"));
        let tally = bench
            .metered(
                &Stop::default(),
                |_, _| (10 * MINUTE, bench.committed()),
                Some(meter),
            )
            .await;
        assert_eq!(tally.committed, 16);
        assert_eq!(
            minutes(&bench.starts("anthropic")),
            [0, 0, 10, 10, 20, 20, 30, 30]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_quota_reading_at_its_ceiling_stops_the_backend() {
        for (from, expected) in [
            (None, 0),
            (Some(Utc::now() + chrono::Duration::hours(1)), 8),
        ] {
            let mut bench = Bench::new();
            bench.guard.ceilings.insert(
                backend("anthropic"),
                Ceiling {
                    window: Some("5h".to_owned()),
                    fraction: 0.5,
                    from,
                },
            );
            let meter = |at: Duration| Ok(five_hours(0.6, at, 300 * MINUTE));
            let tally = bench
                .metered(
                    &Stop::default(),
                    |_, _| (10 * MINUTE, bench.committed()),
                    Some(meter),
                )
                .await;
            assert_eq!(bench.starts("anthropic").len(), expected, "{from:?}");
            assert_eq!(tally.committed, 8 + expected, "{from:?}");
        }
    }
}
