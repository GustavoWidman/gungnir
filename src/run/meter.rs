//! A metered lane's view of its backend's quota, from the readings gungnir takes of the vendor's
//! own meter, and what that view allows. The lane holds, starting nothing new while its running
//! attempts finish, as long as its latest reading shows a window at or past the hold fraction.
//! It reads the meter before it starts anything once its reading is [`REFRESH`] old, and after a
//! rate limit whose reset the vendor did not name even when its gate leaves no room, so that a
//! spent window holds the lane until it resets instead of leaving it to back off blindly. It
//! never reads more often than once per [`REFRESH`].
use super::gate::RESET_MARGIN;
use crate::quota::Window;
use chrono::{DateTime, Utc};
use std::time::Duration;
use tokio::time::Instant;

/// How old a reading may be when the lane starts an attempt, and how often a held lane reads
/// again, so that a hold also ends when a window that empties gradually falls below it.
pub const REFRESH: Duration = Duration::from_secs(2 * 60);

/// The hold fraction of a backend the command line names none for. In the study-glm experiment
/// a session spent roughly 5% of the Pro plan's 5-hour window, and a lane holds with up to six
/// still running, about half done: holding at 80% leaves them the room to finish.
pub const HOLD: f64 = 0.8;

#[derive(Clone, Debug)]
pub struct Meter {
    hold: f64,
    latest: Option<Reading>,
    /// A rate limit with no reset time ended an attempt since the latest reading.
    limited: bool,
    /// A read is in flight.
    reading: bool,
}

#[derive(Clone, Debug)]
struct Reading {
    at: Instant,
    /// Each window, with its reset on the lane's clock.
    windows: Vec<(Window, Option<Instant>)>,
}

/// What a metered lane may do now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admit {
    /// Start what the gate allows.
    Go,
    /// Start nothing, and read the meter.
    Read,
    /// Start nothing until `until`, or until the read in flight ends when it is `None`.
    Hold { until: Option<Instant> },
}

impl Meter {
    /// A lane that holds at `hold`, the fraction of a window used, and has read nothing yet.
    pub fn new(hold: f64) -> Self {
        Self {
            hold,
            latest: None,
            limited: false,
            reading: false,
        }
    }

    pub fn hold(&self) -> f64 {
        self.hold
    }

    /// What the lane may do at `now`, when its gate has `room` for an attempt or not.
    pub fn admit(&self, now: Instant, room: bool) -> Admit {
        if self.reading {
            return Admit::Hold { until: None };
        }
        if let Some(until) = self.held_until(now) {
            return Admit::Hold { until: Some(until) };
        }
        match self.latest.as_ref().map(|latest| latest.at + REFRESH) {
            Some(due) if now < due && self.limited => Admit::Hold { until: Some(due) },
            Some(due) if now < due => Admit::Go,
            _ if room || self.limited => Admit::Read,
            _ => Admit::Go,
        }
    }

    /// A read started.
    pub fn begin(&mut self) {
        self.reading = true;
    }

    /// An attempt was rate limited, and the vendor did not say until when.
    pub fn limited(&mut self) {
        self.limited = true;
    }

    /// The read in flight ended at `now`, which is `wall` on the vendor's clock, with
    /// `windows`: none when it failed, so that the lane goes on as if it had no meter until
    /// the next reading.
    pub fn record(&mut self, now: Instant, wall: DateTime<Utc>, windows: Vec<Window>) {
        let windows = windows
            .into_iter()
            .map(|window| {
                let resets = window
                    .resets_at
                    .map(|at| now + (at - wall).to_std().unwrap_or_default());
                (window, resets)
            })
            .collect();
        self.latest = Some(Reading { at: now, windows });
        self.reading = false;
        self.limited = false;
    }

    /// The windows of the latest reading.
    pub fn windows(&self) -> impl Iterator<Item = &Window> {
        self.latest
            .iter()
            .flat_map(|latest| latest.windows.iter().map(|(window, _)| window))
    }

    /// A window of the latest reading at or past the hold fraction.
    pub fn spent(&self) -> Option<&Window> {
        self.windows().find(|window| window.used >= self.hold)
    }

    /// Until when the latest reading holds the lane: the latest reset, plus a margin, of the
    /// windows at or past the hold fraction, but never past when the reading is due again.
    fn held_until(&self, now: Instant) -> Option<Instant> {
        let latest = self.latest.as_ref()?;
        let due = latest.at + REFRESH;
        latest
            .windows
            .iter()
            .filter(|(window, _)| window.used >= self.hold)
            .map(|(_, resets)| resets.map_or(due, |resets| (resets + RESET_MARGIN).min(due)))
            .filter(|until| now < *until)
            .max()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: Duration = Duration::from_secs(60);

    fn window(used: f64, resets_at: Option<DateTime<Utc>>) -> Window {
        Window {
            name: "5h".to_owned(),
            used,
            resets_at,
        }
    }

    /// `meter` after a read at `now` that found one 5-hour window `used`, resetting `resets_in`
    /// later.
    fn read(meter: &mut Meter, now: Instant, used: f64, resets_in: Option<Duration>) {
        meter.begin();
        let wall = Utc::now();
        let resets_at = resets_in.map(|left| wall + chrono::TimeDelta::from_std(left).unwrap());
        meter.record(now, wall, vec![window(used, resets_at)]);
    }

    #[test]
    fn a_lane_reads_before_it_starts_and_again_only_once_the_reading_is_old() {
        let t0 = Instant::now();
        let mut meter = Meter::new(0.8);
        assert_eq!(meter.admit(t0, false), Admit::Go, "no room, no read");
        assert_eq!(meter.admit(t0, true), Admit::Read);
        meter.begin();
        assert_eq!(meter.admit(t0, true), Admit::Hold { until: None });

        read(&mut meter, t0, 0.79, Some(60 * MINUTE));
        assert_eq!(meter.admit(t0, true), Admit::Go);
        assert_eq!(meter.admit(t0 + REFRESH - MINUTE, true), Admit::Go);
        assert_eq!(meter.admit(t0 + REFRESH, true), Admit::Read);
        assert_eq!(meter.admit(t0 + REFRESH, false), Admit::Go);
    }

    #[test]
    fn a_window_at_the_hold_holds_until_it_resets_or_the_reading_is_due() {
        let t0 = Instant::now();
        let mut meter = Meter::new(0.8);
        read(&mut meter, t0, 0.8, Some(30 * MINUTE));
        assert_eq!(meter.spent().map(|w| w.used), Some(0.8));
        assert_eq!(
            meter.admit(t0, true),
            Admit::Hold {
                until: Some(t0 + REFRESH)
            }
        );
        assert_eq!(meter.admit(t0 + REFRESH, true), Admit::Read);

        let t1 = t0 + 29 * MINUTE + 30 * Duration::from_secs(1);
        read(&mut meter, t1, 1.0, Some(Duration::from_secs(30)));
        let reset = t1 + Duration::from_secs(30);
        assert_eq!(
            meter.admit(t1, true),
            Admit::Hold {
                until: Some(reset + RESET_MARGIN)
            }
        );
        assert_eq!(
            meter.admit(reset + RESET_MARGIN, true),
            Admit::Go,
            "the window reset, and the reading is not due yet"
        );

        let mut unknown = Meter::new(0.8);
        read(&mut unknown, t0, 1.0, None);
        assert_eq!(
            unknown.admit(t0, true),
            Admit::Hold {
                until: Some(t0 + REFRESH)
            }
        );
    }

    #[test]
    fn a_rate_limit_reads_the_meter_without_room_but_not_sooner_than_the_refresh() {
        let t0 = Instant::now();
        let mut meter = Meter::new(0.8);
        read(&mut meter, t0, 0.5, Some(60 * MINUTE));
        meter.limited();
        assert_eq!(
            meter.admit(t0 + MINUTE, false),
            Admit::Hold {
                until: Some(t0 + REFRESH)
            }
        );
        assert_eq!(meter.admit(t0 + REFRESH, false), Admit::Read);
        read(&mut meter, t0 + REFRESH, 1.0, Some(40 * MINUTE));
        assert!(matches!(
            meter.admit(t0 + REFRESH, false),
            Admit::Hold { until: Some(_) }
        ));
    }

    #[test]
    fn a_failed_read_lets_the_lane_go_until_the_next_reading() {
        let t0 = Instant::now();
        let mut meter = Meter::new(0.8);
        read(&mut meter, t0, 1.0, Some(60 * MINUTE));
        meter.limited();
        meter.begin();
        meter.record(t0 + REFRESH, Utc::now(), Vec::new());
        assert_eq!(meter.admit(t0 + REFRESH, true), Admit::Go);
        assert_eq!(meter.admit(t0 + 2 * REFRESH, true), Admit::Read);
    }
}
