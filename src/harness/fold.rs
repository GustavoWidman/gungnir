//! The fold of one attempt's events into its ending. It is fed line by line while the agent
//! runs and replays a stored transcript the same way.
use super::{Event, TokenUsage, UsageSample};
use chrono::{DateTime, Utc};
use std::{collections::BTreeMap, time::Duration};

/// How an attempt ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Ending {
    /// A sample, whatever the agent achieved.
    Finished(AgentOutcome),
    /// The account's quota ended the session. The attempt is discarded and its usage is
    /// overhead.
    RateLimited {
        resets_at: Option<DateTime<Utc>>,
        usage: Vec<UsageSample>,
    },
}

/// What an attempt that counts as a sample produced, apart from its workspace.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentOutcome {
    pub exit: Exit,
    /// Model round trips, when the CLI counts them.
    pub turns: Option<u32>,
    /// In the order each request was first seen.
    pub usage: Vec<UsageSample>,
    /// Time spent waiting on the API, when the CLI reports it.
    pub api_time: Option<Duration>,
    /// The model the session started on, when the CLI says.
    pub model: Option<String>,
    pub quota: BTreeMap<String, QuotaSpan>,
    /// How many lines of each kind the driver did not recognise.
    pub unrecognised: BTreeMap<String, u32>,
}

impl AgentOutcome {
    pub fn total(&self) -> TokenUsage {
        self.usage.iter().map(|sample| sample.tokens).sum()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Exit {
    /// The CLI reported the end of the session.
    Completed,
    /// The model declined the task.
    Refused,
    /// The CLI reported an error that ended the session.
    Errored { message: String },
    /// The process ended on its own without a final report.
    Crashed,
    /// The runner ended the process before a final report.
    Killed,
}

/// How the agent's process ended, as the runner saw it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessEnd {
    /// It exited or died on its own.
    Exited,
    /// The runner killed it, at a time limit, on a rate limit or on a stop.
    Killed,
}

/// The first and last utilization seen for one quota window during an attempt.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct QuotaSpan {
    pub first: f64,
    pub last: f64,
}

#[derive(Debug, Default)]
pub struct Fold {
    model: Option<String>,
    usage: Vec<UsageSample>,
    quota: BTreeMap<String, QuotaSpan>,
    rate_limit: Option<RateLimit>,
    refused: bool,
    report: Option<Report>,
    unrecognised: BTreeMap<String, u32>,
}

#[derive(Clone, Copy, Debug)]
struct RateLimit {
    resets_at: Option<DateTime<Utc>>,
}

#[derive(Debug)]
struct Report {
    turns: Option<u32>,
    api_time: Option<Duration>,
    error: Option<String>,
}

impl Fold {
    pub fn push(&mut self, event: Event) {
        match event {
            Event::Init { model, .. } => {
                self.model.get_or_insert(model);
            }
            Event::Usage(sample) => self.record(sample),
            Event::SessionUsage { model, tokens } => self.top_up(model, tokens),
            Event::Quota {
                window,
                utilization,
                ..
            } => {
                self.quota
                    .entry(window)
                    .and_modify(|span| span.last = utilization)
                    .or_insert(QuotaSpan {
                        first: utilization,
                        last: utilization,
                    });
            }
            Event::RateLimited { resets_at } => {
                let known = self.rate_limit.and_then(|limit| limit.resets_at);
                self.rate_limit = Some(RateLimit {
                    resets_at: resets_at.or(known),
                });
            }
            Event::Refused => self.refused = true,
            Event::Finished {
                turns,
                api_time,
                error,
            } => {
                self.report = Some(Report {
                    turns,
                    api_time,
                    error,
                });
            }
            Event::Noise => {}
            Event::Unrecognised(kind) => *self.unrecognised.entry(kind).or_default() += 1,
        }
    }

    /// Whether a rate limit has ended the session, so the runner can stop the process now.
    pub fn rate_limited(&self) -> bool {
        self.rate_limit.is_some()
    }

    /// The ending, once the process is gone and every line and any diagnosis are pushed.
    pub fn finish(self, end: ProcessEnd) -> Ending {
        if let Some(limit) = self.rate_limit {
            return Ending::RateLimited {
                resets_at: limit.resets_at,
                usage: self.usage,
            };
        }
        let exit = match (self.refused, &self.report, end) {
            (true, ..) => Exit::Refused,
            (false, Some(report), _) => report
                .error
                .clone()
                .map_or(Exit::Completed, |message| Exit::Errored { message }),
            (false, None, ProcessEnd::Exited) => Exit::Crashed,
            (false, None, ProcessEnd::Killed) => Exit::Killed,
        };
        Ending::Finished(AgentOutcome {
            exit,
            turns: self.report.as_ref().and_then(|report| report.turns),
            usage: self.usage,
            api_time: self.report.as_ref().and_then(|report| report.api_time),
            model: self.model,
            quota: self.quota,
            unrecognised: self.unrecognised,
        })
    }

    fn record(&mut self, sample: UsageSample) {
        let earlier = sample.request.as_ref().and_then(|id| {
            self.usage
                .iter()
                .position(|seen| seen.request.as_ref() == Some(id))
        });
        match earlier {
            Some(index) => self.usage[index] = sample,
            None => self.usage.push(sample),
        }
    }

    /// Adds a sample for whatever part of `model`'s session total its samples do not cover.
    fn top_up(&mut self, model: String, total: TokenUsage) {
        let covered: TokenUsage = self
            .usage
            .iter()
            .filter(|sample| sample.model.as_deref() == Some(model.as_str()))
            .map(|sample| sample.tokens)
            .sum();
        let rest = total.saturating_sub(covered);
        if rest != TokenUsage::default() {
            self.usage.push(UsageSample {
                request: None,
                model: Some(model),
                tokens: rest,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(request: Option<&str>, model: &str, input: u64, output: u64) -> Event {
        Event::Usage(UsageSample {
            request: request.map(str::to_owned),
            model: Some(model.to_owned()),
            tokens: TokenUsage {
                input,
                output,
                ..TokenUsage::default()
            },
        })
    }

    fn finished(error: Option<&str>) -> Event {
        Event::Finished {
            turns: Some(3),
            api_time: Some(Duration::from_millis(1500)),
            error: error.map(str::to_owned),
        }
    }

    fn fold(events: impl IntoIterator<Item = Event>) -> Fold {
        let mut fold = Fold::default();
        events.into_iter().for_each(|event| fold.push(event));
        fold
    }

    fn outcome(events: impl IntoIterator<Item = Event>, end: ProcessEnd) -> AgentOutcome {
        match fold(events).finish(end) {
            Ending::Finished(outcome) => outcome,
            ending => panic!("expected a finished attempt, got {ending:?}"),
        }
    }

    fn at(secs: i64) -> Option<DateTime<Utc>> {
        DateTime::from_timestamp(secs, 0)
    }

    #[test]
    fn a_repeated_request_replaces_its_earlier_sample() {
        let outcome = outcome(
            [
                usage(Some("a"), "m", 2, 1),
                usage(None, "m", 7, 0),
                usage(Some("a"), "m", 2, 16),
                usage(None, "m", 7, 0),
                finished(None),
            ],
            ProcessEnd::Exited,
        );
        let requests: Vec<Option<&str>> = outcome
            .usage
            .iter()
            .map(|sample| sample.request.as_deref())
            .collect();
        assert_eq!(requests, [Some("a"), None, None]);
        assert_eq!((outcome.total().input, outcome.total().output), (16, 16));
    }

    #[test]
    fn a_session_total_tops_up_what_the_requests_did_not_report() {
        let session = TokenUsage {
            input: 4,
            cache_read: 30,
            output: 78,
            reasoning: Some(12),
            ..TokenUsage::default()
        };
        let outcome = outcome(
            [
                usage(Some("a"), "m", 2, 16),
                usage(Some("b"), "m", 2, 4),
                usage(Some("c"), "other", 9, 9),
                Event::SessionUsage {
                    model: "m".into(),
                    tokens: session,
                },
                finished(None),
            ],
            ProcessEnd::Exited,
        );
        let for_m: TokenUsage = outcome
            .usage
            .iter()
            .filter(|sample| sample.model.as_deref() == Some("m"))
            .map(|sample| sample.tokens)
            .sum();
        assert_eq!(for_m, session);
        assert_eq!(outcome.usage.len(), 4);
        assert_eq!(outcome.usage[3].request, None);
    }

    #[test]
    fn a_session_total_already_covered_adds_nothing() {
        let tokens = TokenUsage {
            input: 2,
            output: 16,
            ..TokenUsage::default()
        };
        let session = Event::SessionUsage {
            model: "m".into(),
            tokens,
        };
        let outcome = outcome(
            [usage(Some("a"), "m", 2, 16), session.clone(), session],
            ProcessEnd::Exited,
        );
        assert_eq!(outcome.usage.len(), 1);
    }

    #[test]
    fn the_final_report_decides_the_exit_before_the_process_does() {
        let cases = [
            (vec![finished(None)], ProcessEnd::Exited, Exit::Completed),
            (vec![finished(None)], ProcessEnd::Killed, Exit::Completed),
            (
                vec![finished(Some("boom"))],
                ProcessEnd::Exited,
                Exit::Errored {
                    message: "boom".into(),
                },
            ),
            (
                vec![Event::Refused, finished(Some("declined"))],
                ProcessEnd::Exited,
                Exit::Refused,
            ),
            (vec![Event::Refused], ProcessEnd::Killed, Exit::Refused),
            (vec![Event::Noise], ProcessEnd::Exited, Exit::Crashed),
            (vec![Event::Noise], ProcessEnd::Killed, Exit::Killed),
        ];
        for (events, end, exit) in cases {
            assert_eq!(
                outcome(events.clone(), end).exit,
                exit,
                "{events:?} {end:?}"
            );
        }
    }

    #[test]
    fn the_report_supplies_turns_and_api_time() {
        let outcome = outcome([finished(None)], ProcessEnd::Exited);
        assert_eq!(outcome.turns, Some(3));
        assert_eq!(outcome.api_time, Some(Duration::from_millis(1500)));
        assert_eq!(
            fold([Event::Noise]).finish(ProcessEnd::Exited),
            Ending::Finished(AgentOutcome {
                exit: Exit::Crashed,
                turns: None,
                usage: vec![],
                api_time: None,
                model: None,
                quota: BTreeMap::new(),
                unrecognised: BTreeMap::new(),
            })
        );
    }

    #[test]
    fn a_rate_limit_overrides_everything_and_keeps_the_usage() {
        let mut live = fold([usage(Some("a"), "m", 5, 5)]);
        assert!(!live.rate_limited());
        live.push(Event::RateLimited {
            resets_at: at(1_793_762_400),
        });
        assert!(live.rate_limited());
        live.push(Event::RateLimited { resets_at: None });
        live.push(finished(Some("usage limit reached")));

        let Ending::RateLimited { resets_at, usage } = live.finish(ProcessEnd::Killed) else {
            panic!("expected a rate limit");
        };
        assert_eq!(resets_at, at(1_793_762_400));
        assert_eq!(usage.len(), 1);
    }

    #[test]
    fn quota_keeps_the_first_and_last_utilization_per_window() {
        let quota = |window: &str, utilization: f64| Event::Quota {
            window: window.into(),
            utilization,
            resets_at: None,
            allowed: true,
        };
        let outcome = outcome(
            [
                quota("five_hour", 0.17),
                quota("seven_day", 0.53),
                quota("five_hour", 0.2),
                quota("five_hour", 0.25),
            ],
            ProcessEnd::Exited,
        );
        assert_eq!(
            outcome.quota["five_hour"],
            QuotaSpan {
                first: 0.17,
                last: 0.25
            }
        );
        assert_eq!(
            outcome.quota["seven_day"],
            QuotaSpan {
                first: 0.53,
                last: 0.53
            }
        );
    }

    #[test]
    fn the_first_init_names_the_model_and_unknown_kinds_are_counted() {
        let init = |model: &str| Event::Init {
            model: model.into(),
            tools: vec![],
        };
        let unknown = |kind: &str| Event::Unrecognised(kind.into());
        let outcome = outcome(
            [
                init("claude-haiku-5-5"),
                init("other"),
                unknown("telemetry"),
                unknown("system/new"),
                unknown("telemetry"),
            ],
            ProcessEnd::Exited,
        );
        assert_eq!(outcome.model.as_deref(), Some("claude-haiku-5-5"));
        assert_eq!(
            outcome.unrecognised,
            BTreeMap::from([("system/new".into(), 1), ("telemetry".into(), 2)])
        );
    }
}
