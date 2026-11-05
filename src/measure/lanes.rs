//! The lanes of the measure phase: one per measurement core, each running one comparison at a
//! time with every run of it pinned to that core.
use crate::stop::{Stage, Stop};
use eyre::{Result, ensure};
use futures_util::{StreamExt, stream::FuturesUnordered};
use std::{collections::BTreeSet, fmt};

/// The CPUs measuring pins its comparisons to, one comparison per CPU at a time: at least one,
/// and none twice, so no two lanes ever share a CPU.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cores(BTreeSet<u32>);

impl Cores {
    /// The CPUs `cpus`, refusing none or one listed twice.
    pub fn new(cpus: impl IntoIterator<Item = u32>) -> Result<Self> {
        let mut cores = BTreeSet::new();
        for cpu in cpus {
            ensure!(cores.insert(cpu), "CPU {cpu} is listed twice");
        }
        ensure!(!cores.is_empty(), "no CPU is listed");
        Ok(Self(cores))
    }

    /// The one CPU `cpu`.
    pub fn one(cpu: u32) -> Self {
        Self(BTreeSet::from([cpu]))
    }

    /// The CPUs in ascending order.
    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.0.iter().copied()
    }
}

impl fmt::Display for Cores {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let cpus: Vec<String> = self.0.iter().map(u32::to_string).collect();
        f.write_str(&cpus.join(","))
    }
}

/// Runs `run` on each of `jobs` in order, each on the lowest of `cores` that no running job
/// holds, so as many jobs run at once as there are cores and never two on one core. Hands each
/// job and its result to `done` as it ends. Once `stop` drains, nothing more starts, and the
/// jobs already running run to their end. Returns how many jobs never started.
pub(super) async fn on_cores<J, R>(
    jobs: impl IntoIterator<Item = J>,
    cores: &Cores,
    stop: &Stop,
    run: impl AsyncFn(&J, u32) -> R,
    mut done: impl FnMut(J, R),
) -> usize {
    let mut free = cores.0.clone();
    let mut queue = jobs.into_iter();
    let mut running = FuturesUnordered::new();
    let run = &run;
    loop {
        while stop.stage() == Stage::Running
            && let Some(&core) = free.first()
            && let Some(job) = queue.next()
        {
            free.remove(&core);
            running.push(async move {
                let result = run(&job, core).await;
                (job, core, result)
            });
        }
        let Some((job, core, result)) = running.next().await else {
            break;
        };
        free.insert(core);
        done(job, result);
    }
    queue.count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, time::Duration};

    /// What the jobs of one call saw.
    #[derive(Default)]
    struct Seen {
        started: Vec<usize>,
        busy: BTreeSet<u32>,
        most: usize,
        cores: BTreeSet<u32>,
    }

    /// Runs `jobs` jobs on `cores`, job `n` taking `n % 3 + 1` milliseconds, and `escalate`
    /// on `stop` when job `at` starts, if any. Returns what the jobs saw, the jobs `done` got
    /// in the order they ended, and how many never started.
    async fn play(
        jobs: usize,
        cores: &Cores,
        stop: &Stop,
        escalate: Option<usize>,
    ) -> (Seen, Vec<usize>, usize) {
        let seen = RefCell::new(Seen::default());
        let mut ended = Vec::new();
        let unstarted = on_cores(
            0..jobs,
            cores,
            stop,
            async |&job: &usize, core: u32| {
                {
                    let mut seen = seen.borrow_mut();
                    assert!(seen.busy.insert(core), "job {job} on busy CPU {core}");
                    seen.most = seen.most.max(seen.busy.len());
                    seen.started.push(job);
                    seen.cores.insert(core);
                }
                if escalate == Some(job) {
                    stop.escalate();
                }
                let millis = u64::try_from(job % 3 + 1).unwrap();
                tokio::time::sleep(Duration::from_millis(millis)).await;
                seen.borrow_mut().busy.remove(&core);
                job * 10
            },
            |job, result| {
                assert_eq!(result, job * 10);
                ended.push(job);
            },
        )
        .await;
        (seen.into_inner(), ended, unstarted)
    }

    #[tokio::test(start_paused = true)]
    async fn jobs_start_in_order_one_per_core_and_each_runs_once() {
        let cores = Cores::new([8, 10, 12]).unwrap();
        let (seen, mut ended, unstarted) = play(10, &cores, &Stop::default(), None).await;
        assert_eq!(seen.started, (0..10).collect::<Vec<_>>());
        assert_eq!(seen.most, 3);
        assert_eq!(seen.cores, BTreeSet::from([8, 10, 12]));
        ended.sort_unstable();
        assert_eq!(ended, (0..10).collect::<Vec<_>>());
        assert_eq!(unstarted, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn one_core_runs_the_jobs_one_at_a_time() {
        let (seen, ended, unstarted) = play(4, &Cores::one(15), &Stop::default(), None).await;
        assert_eq!(seen.most, 1);
        assert_eq!(ended, [0, 1, 2, 3]);
        assert_eq!(unstarted, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_drain_lets_the_running_jobs_finish_and_starts_no_more() {
        let stop = Stop::default();
        let cores = Cores::new([8, 10]).unwrap();
        let (seen, mut ended, unstarted) = play(10, &cores, &stop, Some(0)).await;
        assert_eq!(stop.stage(), Stage::Draining);
        assert_eq!(seen.started, [0, 1]);
        ended.sort_unstable();
        assert_eq!(ended, [0, 1]);
        assert_eq!(unstarted, 8);

        let (seen, ended, unstarted) = play(3, &cores, &stop, None).await;
        assert!(seen.started.is_empty());
        assert!(ended.is_empty());
        assert_eq!(unstarted, 3);
    }

    #[test]
    fn cores_are_listed_once_and_at_least_one() {
        assert_eq!(Cores::new([14, 8, 10]).unwrap().to_string(), "8,10,14");
        let message = Cores::new([8, 9, 8]).unwrap_err().to_string();
        assert!(message.contains("CPU 8 is listed twice"), "{message}");
        assert!(Cores::new([]).is_err());
    }
}
