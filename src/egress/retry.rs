//! When the relay tries a request again: the statuses it retries, and the jittered exponential
//! backoff that spaces the tries within a bound on the whole wait.

use std::time::Duration;

use chrono::{DateTime, Utc};

/// The shortest wait the relay honours, so that a `retry-after` of zero cannot make it spin.
const MIN_WAIT: Duration = Duration::from_secs(1);

/// Whether a response with `status`, which arrived before any of its body, is worth trying
/// again: the endpoint is throttling or briefly unavailable.
pub(super) fn retries(status: u16) -> bool {
    matches!(status, 429 | 502 | 503 | 504)
}

/// How the relay spaces the tries of one request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Backoff {
    /// The longest wait after the first try. Each later wait may be twice as long, up to `cap`.
    pub first: Duration,
    pub cap: Duration,
    /// How long the relay may hold one request, from its first try to the answer it gives the
    /// agent. A wait that would end later is not taken, and the agent gets the last answer.
    pub budget: Duration,
}

impl Backoff {
    /// Spaced for an endpoint that refuses bursts and admits requests a few seconds apart, and
    /// bounded well within an agent's idle budget.
    pub const DEFAULT: Self = Self {
        first: Duration::from_secs(8),
        cap: Duration::from_secs(60),
        budget: Duration::from_secs(10 * 60),
    };

    /// The wait before trying again after `tries` tries, `elapsed` after the first began:
    /// `retry_after` when the endpoint named one, and otherwise a uniform draw from the upper
    /// half of the nominal wait, placed by `jitter` in `[0, 1)`. `None` once the wait would end
    /// past the budget.
    pub fn wait(
        &self,
        tries: u32,
        retry_after: Option<Duration>,
        elapsed: Duration,
        jitter: f64,
    ) -> Option<Duration> {
        let doublings = tries.saturating_sub(1).min(31);
        let nominal = self.first.saturating_mul(1 << doublings).min(self.cap);
        let wait = match retry_after {
            Some(named) => named.max(MIN_WAIT),
            None => nominal.mul_f64(0.5 + jitter.clamp(0.0, 1.0) / 2.0),
        };
        (elapsed.saturating_add(wait) <= self.budget).then_some(wait)
    }
}

/// The wait a `retry-after` value names at `now`: a number of seconds, or an HTTP date.
pub(super) fn retry_after(value: &[u8], now: DateTime<Utc>) -> Option<Duration> {
    let value = std::str::from_utf8(value).ok()?.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let at = DateTime::parse_from_rfc2822(value).ok()?;
    Some(
        (at.with_timezone(&Utc) - now)
            .to_std()
            .unwrap_or(Duration::ZERO),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: Duration = Duration::from_secs(1);

    #[test]
    fn retries_throttling_and_gateway_failures_only() {
        for status in [429, 502, 503, 504] {
            assert!(retries(status), "{status}");
        }
        for status in [200, 400, 401, 404, 413, 500, 501] {
            assert!(!retries(status), "{status}");
        }
    }

    #[test]
    fn waits_double_up_to_the_cap_within_their_upper_half() {
        let backoff = Backoff::DEFAULT;
        let longest: Vec<_> = (1..=6)
            .map(|tries| backoff.wait(tries, None, Duration::ZERO, 1.0).unwrap())
            .collect();
        assert_eq!(longest, [8, 16, 32, 60, 60, 60].map(|s| s * SECOND));
        assert_eq!(backoff.wait(1, None, Duration::ZERO, 0.0), Some(4 * SECOND));
        assert_eq!(
            backoff.wait(2, None, Duration::ZERO, 0.5),
            Some(Duration::from_secs(12))
        );
        assert_eq!(
            backoff.wait(u32::MAX, None, Duration::ZERO, 1.0),
            Some(60 * SECOND)
        );
    }

    #[test]
    fn a_named_retry_after_replaces_the_backoff_but_never_spins() {
        let backoff = Backoff::DEFAULT;
        assert_eq!(
            backoff.wait(5, Some(3 * SECOND), Duration::ZERO, 1.0),
            Some(3 * SECOND)
        );
        assert_eq!(
            backoff.wait(1, Some(Duration::ZERO), Duration::ZERO, 1.0),
            Some(SECOND)
        );
    }

    #[test]
    fn no_wait_ends_past_the_budget() {
        let backoff = Backoff::DEFAULT;
        let budget = backoff.budget;
        assert_eq!(
            backoff.wait(9, None, budget - 60 * SECOND, 1.0),
            Some(60 * SECOND)
        );
        assert_eq!(backoff.wait(9, None, budget - 59 * SECOND, 1.0), None);
        assert_eq!(
            backoff.wait(1, Some(budget + SECOND), Duration::ZERO, 0.0),
            None
        );
        assert_eq!(backoff.wait(1, None, budget, 0.0), None);
    }

    #[test]
    fn reads_retry_after_as_seconds_or_a_date() {
        let now = DateTime::parse_from_rfc3339("2026-11-06T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(retry_after(b"3", now), Some(3 * SECOND));
        assert_eq!(retry_after(b" 120 ", now), Some(120 * SECOND));
        assert_eq!(
            retry_after(b"Fri, 06 Nov 2026 12:00:30 GMT", now),
            Some(30 * SECOND)
        );
        assert_eq!(
            retry_after(b"Fri, 06 Nov 2026 11:59:00 GMT", now),
            Some(Duration::ZERO)
        );
        assert_eq!(retry_after(b"soon", now), None);
        assert_eq!(retry_after(b"-1", now), None);
    }
}
