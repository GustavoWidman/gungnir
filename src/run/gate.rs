//! A backend lane's gate: whether the lane may start attempts, as a pure function of what its
//! attempts did. A rate limit pauses the lane until its quota resets, and when the pause ends
//! one probe attempt runs alone before the lane opens again.
use std::time::Duration;
use tokio::time::Instant;

/// The first pause after a rate limit whose reset time is unknown. Each probe that is rate
/// limited again doubles the pause, up to [`BACKOFF_MAX`].
pub const BACKOFF_MIN: Duration = Duration::from_secs(5 * 60);
pub const BACKOFF_MAX: Duration = Duration::from_secs(60 * 60);
/// Waited past a reset time the vendor named, so that a clock slightly behind the vendor's does
/// not probe a quota that has not reset yet.
pub const RESET_MARGIN: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    /// Attempts start up to the lane's concurrency.
    Open,
    /// Nothing starts before `until`, and then only a probe. `backoff` is the pause a rate
    /// limit with no reset time gets, which a rate-limited probe doubles.
    Paused { until: Instant, backoff: Duration },
    /// A probe is running. Nothing starts until it ends.
    Probing { backoff: Duration },
}

/// What happened in a lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    /// An attempt started.
    Started,
    /// An attempt ended without a rate limit.
    Passed,
    /// An attempt was rate limited. `resets_in` is how long until the quota resets, when the
    /// vendor named a time that is still ahead.
    Limited { resets_in: Option<Duration> },
}

impl Gate {
    /// How many more attempts may start at `now`, with `running` of the lane's `limit` in
    /// flight.
    pub fn room(self, now: Instant, running: usize, limit: usize) -> usize {
        let free = limit.saturating_sub(running);
        match self {
            Self::Open => free,
            Self::Paused { until, .. } if now >= until => free.min(1),
            Self::Paused { .. } | Self::Probing { .. } => 0,
        }
    }

    /// The gate after `event` at `now`.
    pub fn next(self, event: Event, now: Instant) -> Self {
        match (self, event) {
            (Self::Paused { until, backoff }, Event::Started) if now >= until => {
                Self::Probing { backoff }
            }
            (Self::Probing { .. }, Event::Passed) => Self::Open,
            (gate, Event::Limited { resets_in }) => {
                let backoff = match gate {
                    Self::Open => BACKOFF_MIN,
                    Self::Paused { backoff, .. } => backoff,
                    Self::Probing { backoff } => (backoff * 2).min(BACKOFF_MAX),
                };
                let pause = resets_in.map_or(backoff, |resets_in| resets_in + RESET_MARGIN);
                let until = match gate {
                    Self::Paused { until, .. } => until.max(now + pause),
                    Self::Open | Self::Probing { .. } => now + pause,
                };
                Self::Paused { until, backoff }
            }
            (gate, Event::Started | Event::Passed) => gate,
        }
    }

    /// When a pause ends, letting a probe start.
    pub fn wakes(self) -> Option<Instant> {
        match self {
            Self::Paused { until, .. } => Some(until),
            Self::Open | Self::Probing { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: Duration = Duration::from_secs(60);

    fn limited(resets_in: Option<Duration>) -> Event {
        Event::Limited { resets_in }
    }

    #[test]
    fn an_open_lane_fills_its_concurrency() {
        let now = Instant::now();
        assert_eq!(Gate::Open.room(now, 0, 3), 3);
        assert_eq!(Gate::Open.room(now, 2, 3), 1);
        assert_eq!(Gate::Open.room(now, 3, 3), 0);
        assert_eq!(Gate::Open.next(Event::Started, now), Gate::Open);
        assert_eq!(Gate::Open.next(Event::Passed, now), Gate::Open);
    }

    #[test]
    fn a_rate_limit_pauses_until_the_reset_and_then_lets_one_probe_through() {
        let t0 = Instant::now();
        let paused = Gate::Open.next(limited(Some(30 * MINUTE)), t0);
        assert_eq!(paused.wakes(), Some(t0 + 31 * MINUTE));
        assert_eq!(paused.room(t0 + 30 * MINUTE, 0, 3), 0);
        assert_eq!(paused.room(t0 + 31 * MINUTE, 0, 3), 1);
        assert_eq!(paused.room(t0 + 31 * MINUTE, 3, 3), 0);

        let probing = paused.next(Event::Started, t0 + 31 * MINUTE);
        assert!(matches!(probing, Gate::Probing { .. }));
        assert_eq!(probing.room(t0 + 32 * MINUTE, 1, 3), 0);
        assert_eq!(probing.wakes(), None);
        assert_eq!(probing.next(Event::Passed, t0 + 40 * MINUTE), Gate::Open);
    }

    #[test]
    fn unknown_resets_back_off_from_five_minutes_doubling_to_an_hour() {
        let mut now = Instant::now();
        let mut gate = Gate::Open;
        let mut pauses = Vec::new();
        for _ in 0..6 {
            gate = gate.next(limited(None), now);
            let until = gate.wakes().unwrap();
            pauses.push((until - now).as_secs() / 60);
            now = until;
            gate = gate.next(Event::Started, now);
        }
        assert_eq!(pauses, [5, 10, 20, 40, 60, 60]);

        gate = gate.next(Event::Passed, now);
        assert_eq!(gate, Gate::Open);
        let again = gate.next(limited(None), now);
        assert_eq!(again.wakes(), Some(now + 5 * MINUTE));
    }

    #[test]
    fn a_limit_while_paused_may_extend_the_pause_but_never_doubles_it() {
        let t0 = Instant::now();
        let paused = Gate::Open.next(limited(None), t0);
        let later = paused.next(limited(None), t0 + 2 * MINUTE);
        assert_eq!(later.wakes(), Some(t0 + 7 * MINUTE));
        let shorter = later.next(limited(Some(MINUTE)), t0 + 3 * MINUTE);
        assert_eq!(shorter.wakes(), Some(t0 + 7 * MINUTE));
        assert_eq!(
            shorter.next(Event::Passed, t0 + 4 * MINUTE),
            shorter,
            "an attempt from before the pause does not reopen the lane"
        );
    }
}
