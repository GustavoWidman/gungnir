//! Codex, run headless as `codex exec --json` with the prompt on stdin. Its quota is read
//! afterwards from the session rollout it writes under `CODEX_HOME`.
use super::{
    CredentialMaterial, Driver, Event, Granularity, HomeFile, Launch, LaunchRequest, TokenUsage,
    UNPARSABLE, UsageSample, mentions, plain, utf8,
};
use crate::id::{Effort, HarnessVersion};
use chrono::DateTime;
use eyre::{Result, bail, eyre};
use serde::Deserialize;
use std::path::Path;

pub(super) struct Codex;

/// `CODEX_HOME`, relative to the isolated home.
const CODEX_HOME: &str = ".codex";

/// Features Codex 0.166.0 enables by default that reach services other than the model or
/// change what the model is offered. The other default features did nothing observable in a
/// blank home once these were off.
const DISABLED_FEATURES: &[&str] = &[
    // Loads the account's ChatGPT connectors, hundreds of tools over chatgpt.com.
    "apps",
    // Offers a server-side image generation tool.
    "image_generation",
    // Syncs the account's installed plugins from chatgpt.com, oaiusercontent.com and GitHub.
    "plugins",
];

/// Lowercase phrases that mean the account's quota ended the session, wherever they appear.
const USAGE_LIMIT: &[&str] = &["usage limit", "usage_limit"];
/// Lowercase phrases that mean the same once the turn has failed. Codex also prints them while
/// it retries, so in an `error` line alone they end nothing.
const TOO_MANY_REQUESTS: &[&str] = &[
    "429 too many requests",
    "\"status\":429",
    "rate_limit_exceeded",
];

impl Driver for Codex {
    fn name(&self) -> &'static str {
        "codex"
    }

    fn parser_version(&self) -> u32 {
        2
    }

    fn granularity(&self) -> Granularity {
        Granularity::Turn
    }

    fn accepts(&self, _effort: Effort) -> bool {
        true
    }

    fn parse_version(&self, stdout: &str) -> Result<HarnessVersion> {
        stdout
            .trim()
            .strip_prefix("codex-cli ")
            .ok_or_else(|| eyre!("codex --version printed {stdout:?}"))?
            .parse()
    }

    fn launch(&self, req: &LaunchRequest<'_>) -> Result<Launch> {
        let CredentialMaterial::CodexAuth(auth) = req.credential else {
            bail!("codex takes its credential from a codex-home auth.json");
        };
        if let Some(endpoint) = req.endpoint {
            bail!(
                "codex cannot use endpoint {}; it talks to OpenAI only",
                endpoint.as_str()
            );
        }
        let mut argv: Vec<String> = [
            "codex",
            "exec",
            "--json",
            "--skip-git-repo-check",
            "--dangerously-bypass-approvals-and-sandbox",
            "-m",
            req.system.api_model(),
            "-c",
            &format!("model_reasoning_effort={}", req.system.effort()),
            "-c",
            "web_search=disabled",
            "-c",
            "analytics.enabled=false",
        ]
        .map(str::to_owned)
        .into();
        for feature in DISABLED_FEATURES {
            argv.extend(["--disable".to_owned(), (*feature).to_owned()]);
        }
        argv.extend(["-C".to_owned(), utf8(req.workspace)?, "-".to_owned()]);
        Ok(Launch {
            argv,
            env: vec![plain("CODEX_HOME", utf8(&req.home.join(CODEX_HOME))?)],
            files: vec![HomeFile {
                path: Path::new(CODEX_HOME).join("auth.json"),
                contents: auth.clone(),
            }],
            stdin: req.prompt.to_owned(),
        })
    }

    fn decode(&self, line: &str) -> Vec<Event> {
        match serde_json::from_str(line) {
            Ok(RawLine::Known(known)) => known.events(),
            Ok(RawLine::Other { kind }) => vec![Event::Unrecognised(kind)],
            Err(_) => vec![Event::Unrecognised(UNPARSABLE.into())],
        }
    }

    fn diagnose_exit(&self, stderr_tail: &str) -> Option<Event> {
        mentions(stderr_tail, USAGE_LIMIT).then_some(Event::RateLimited { resets_at: None })
    }

    fn artifacts(&self) -> &'static [&'static str] {
        &[".codex/sessions/**/rollout-*.jsonl"]
    }

    fn decode_artifact(&self, line: &str) -> Vec<Event> {
        let Ok(RolloutLine { payload }) = serde_json::from_str(line) else {
            return Vec::new();
        };
        match payload {
            RolloutPayload::Known(RolloutEvent::TokenCount { rate_limits }) => {
                rate_limits.map_or_else(Vec::new, RawRateLimits::events)
            }
            // A token count that no longer parses is named, so a format change cannot silently
            // stop the quota being read.
            RolloutPayload::Other { kind } if kind == "token_count" => {
                vec![Event::Unrecognised(kind)]
            }
            RolloutPayload::Other { .. } => Vec::new(),
        }
    }
}

/// A line of a kind this parser knows, or failing that, the kind it names.
#[derive(Deserialize)]
#[serde(untagged)]
enum RawLine {
    Known(Known),
    Other {
        #[serde(rename = "type")]
        kind: String,
    },
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum Known {
    #[serde(
        rename = "thread.started",
        alias = "turn.started",
        alias = "item.started",
        alias = "item.updated",
        alias = "item.completed"
    )]
    Progress {},
    #[serde(rename = "turn.completed")]
    TurnCompleted { usage: RawUsage },
    #[serde(rename = "turn.failed")]
    TurnFailed { error: RawError },
    #[serde(rename = "error")]
    Error { message: String },
}

#[derive(Deserialize)]
struct RawUsage {
    input_tokens: u64,
    cached_input_tokens: u64,
    #[serde(default)]
    cache_write_input_tokens: u64,
    output_tokens: u64,
    reasoning_output_tokens: Option<u64>,
}

#[derive(Deserialize)]
struct RawError {
    message: String,
}

#[derive(Deserialize)]
struct RolloutLine {
    payload: RolloutPayload,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RolloutPayload {
    Known(RolloutEvent),
    Other {
        #[serde(rename = "type")]
        kind: String,
    },
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum RolloutEvent {
    TokenCount { rate_limits: Option<RawRateLimits> },
}

#[derive(Deserialize)]
struct RawRateLimits {
    primary: Option<RawWindow>,
    secondary: Option<RawWindow>,
    rate_limit_reached_type: Option<String>,
}

#[derive(Deserialize)]
struct RawWindow {
    used_percent: f64,
    window_minutes: u32,
    resets_at: Option<i64>,
}

impl RawRateLimits {
    fn events(self) -> Vec<Event> {
        let allowed = self.rate_limit_reached_type.is_none();
        [self.primary, self.secondary]
            .into_iter()
            .flatten()
            .map(|window| Event::Quota {
                window: window_name(window.window_minutes),
                utilization: window.used_percent / 100.0,
                resets_at: window
                    .resets_at
                    .and_then(|seconds| DateTime::from_timestamp(seconds, 0)),
                allowed,
            })
            .collect()
    }
}

/// Names a window by its length, because which window Codex calls primary depends on the plan.
fn window_name(minutes: u32) -> String {
    const DAY: u32 = 24 * 60;
    match minutes {
        m if m % DAY == 0 => format!("{}d", m / DAY),
        m if m % 60 == 0 => format!("{}h", m / 60),
        m => format!("{m}m"),
    }
}

impl Known {
    fn events(self) -> Vec<Event> {
        match self {
            Self::Progress {} => vec![Event::Noise],
            Self::TurnCompleted { usage } => vec![
                Event::Usage(UsageSample {
                    request: None,
                    model: None,
                    tokens: usage.tokens(),
                }),
                Event::Finished {
                    turns: None,
                    api_time: None,
                    error: None,
                },
            ],
            Self::TurnFailed { error } => {
                let limited = mentions(&error.message, USAGE_LIMIT)
                    || mentions(&error.message, TOO_MANY_REQUESTS);
                let finished = Event::Finished {
                    turns: None,
                    api_time: None,
                    error: Some(error.message),
                };
                if limited {
                    vec![Event::RateLimited { resets_at: None }, finished]
                } else {
                    vec![finished]
                }
            }
            Self::Error { message } if mentions(&message, USAGE_LIMIT) => {
                vec![Event::RateLimited { resets_at: None }]
            }
            Self::Error { .. } => vec![Event::Noise],
        }
    }
}

impl RawUsage {
    /// OpenAI counts cache reads and writes inside `input_tokens`.
    fn tokens(&self) -> TokenUsage {
        let cached = self.cached_input_tokens + self.cache_write_input_tokens;
        TokenUsage {
            input: self.input_tokens.saturating_sub(cached),
            cache_read: self.cached_input_tokens,
            cache_write: self.cache_write_input_tokens,
            cache_write_long: 0,
            output: self.output_tokens,
            reasoning: self.reasoning_output_tokens,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        harness::{AgentOutcome, Ending, Exit, Fold, Harness, ProcessEnd, QuotaSpan},
        secret::{EnvValue, Secret},
        testing::{self, id},
    };
    use std::fs;

    fn harness() -> Harness {
        Harness(&Codex)
    }

    fn transcript(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/harness/codex")
            .join(name);
        fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
    }

    fn outcome(name: &str) -> AgentOutcome {
        match harness()
            .replay(&transcript(name), &[])
            .finish(ProcessEnd::Exited)
        {
            Ending::Finished(outcome) => outcome,
            ending => panic!("{name} did not finish: {ending:?}"),
        }
    }

    fn launch(effort: Effort, endpoint_of: Option<&str>) -> Result<Launch> {
        let catalog = testing::catalog();
        let system = catalog.system(&id("luna"), &id("openai"), effort, id("0.166.0"))?;
        let credential = CredentialMaterial::CodexAuth(Secret::new(r#"{"tokens":{}}"#.into()));
        harness().launch(&LaunchRequest {
            system: &system,
            endpoint: endpoint_of.and_then(|backend| catalog.backend(&id(backend))?.endpoint()),
            credential: &credential,
            home: Path::new("/home/agent"),
            workspace: Path::new("/work"),
            prompt: "Reply with the single word ok",
        })
    }

    #[test]
    fn launches_exec_with_the_prompt_on_stdin_and_auth_in_codex_home() {
        let launch = launch(Effort::None, None).unwrap();
        assert_eq!(
            launch.argv,
            [
                "codex",
                "exec",
                "--json",
                "--skip-git-repo-check",
                "--dangerously-bypass-approvals-and-sandbox",
                "-m",
                "gpt-luna",
                "-c",
                "model_reasoning_effort=none",
                "-c",
                "web_search=disabled",
                "-c",
                "analytics.enabled=false",
                "--disable",
                "apps",
                "--disable",
                "image_generation",
                "--disable",
                "plugins",
                "-C",
                "/work",
                "-",
            ]
        );
        assert_eq!(launch.stdin, "Reply with the single word ok");
        assert_eq!(
            launch.env,
            [(
                "CODEX_HOME".to_owned(),
                EnvValue::Plain("/home/agent/.codex".into())
            )]
        );
        let [file] = launch.files.as_slice() else {
            panic!("expected one file, got {:?}", launch.files);
        };
        assert_eq!(file.path, Path::new(".codex/auth.json"));
        assert_eq!(file.contents.expose(), r#"{"tokens":{}}"#);
    }

    #[test]
    fn only_a_codex_home_credential_and_the_default_endpoint_launch() {
        assert!(launch(Effort::Low, Some("zai")).is_err());

        let catalog = testing::catalog();
        let system = catalog
            .system(&id("luna"), &id("openai"), Effort::Low, id("0.166.0"))
            .unwrap();
        let credential = CredentialMaterial::Env {
            var: "OPENAI_API_KEY".into(),
            value: Secret::new("sk-test".into()),
        };
        let launched = harness().launch(&LaunchRequest {
            system: &system,
            endpoint: None,
            credential: &credential,
            home: Path::new("/home/agent"),
            workspace: Path::new("/work"),
            prompt: "",
        });
        assert!(launched.is_err());
    }

    #[test]
    fn carries_every_effort() {
        for effort in [
            Effort::None,
            Effort::Minimal,
            Effort::Low,
            Effort::Medium,
            Effort::High,
            Effort::Xhigh,
            Effort::Max,
        ] {
            assert!(harness().accepts(effort), "{effort}");
        }
    }

    #[test]
    fn reads_the_version_line() {
        let version = harness().parse_version("codex-cli 0.166.0\n").unwrap();
        assert_eq!(version.as_str(), "0.166.0");
        assert!(harness().parse_version("2.1.316 (Claude Code)").is_err());
    }

    #[test]
    fn a_luna_run_with_a_command_keeps_cache_reads_out_of_input() {
        let outcome = outcome("luna-command-success.jsonl");
        assert_eq!(outcome.exit, Exit::Completed);
        assert_eq!(outcome.turns, None);
        assert_eq!(outcome.model, None);
        assert_eq!(
            outcome.total(),
            TokenUsage {
                input: 22236 - 19968,
                cache_read: 19968,
                cache_write: 0,
                cache_write_long: 0,
                output: 38,
                reasoning: Some(0),
            }
        );
        assert_eq!(outcome.usage.len(), 1);
        assert!(outcome.unrecognised.is_empty());
    }

    #[test]
    fn a_luna_run_at_effort_none_completes() {
        let outcome = outcome("luna-none-success.jsonl");
        assert_eq!(outcome.exit, Exit::Completed);
        assert_eq!((outcome.total().input, outcome.total().output), (11017, 5));
    }

    #[test]
    fn an_unsupported_effort_is_an_error_not_a_rate_limit() {
        let outcome = outcome("unsupported-effort-error.jsonl");
        let Exit::Errored { message } = &outcome.exit else {
            panic!("expected an error, got {:?}", outcome.exit);
        };
        assert!(message.contains("'minimal' is not supported"), "{message}");
        assert!(outcome.usage.is_empty());
    }

    #[test]
    fn a_usage_limit_rate_limits_the_run_but_a_retried_429_does_not() {
        let mut fold = Fold::default();
        let mut limited_at = None;
        for (index, line) in transcript("synthetic-usage-limit.jsonl")
            .lines()
            .enumerate()
        {
            for event in harness().decode(line) {
                fold.push(event);
            }
            if fold.rate_limited() {
                limited_at.get_or_insert(index);
            }
        }
        assert_eq!(
            limited_at,
            Some(4),
            "the retry notice on line 3 ends nothing"
        );
        assert_eq!(
            fold.finish(ProcessEnd::Exited),
            Ending::RateLimited {
                resets_at: None,
                usage: vec![]
            }
        );
    }

    #[test]
    fn a_failed_turn_with_a_429_is_a_rate_limit() {
        let line = r#"{"type":"turn.failed","error":{"message":"exceeded retry limit, last status: 429 Too Many Requests"}}"#;
        assert_eq!(
            harness().decode(line)[0],
            Event::RateLimited { resets_at: None }
        );
    }

    #[test]
    fn unknown_kinds_are_named() {
        let decode = |line: &str| harness().decode(line);
        assert_eq!(
            decode(r#"{"type":"thread.archived"}"#),
            [Event::Unrecognised("thread.archived".into())]
        );
        assert_eq!(
            decode(r#"{"type":"turn.completed","usage":{}}"#),
            [Event::Unrecognised("turn.completed".into())]
        );
        assert_eq!(decode(""), [Event::Unrecognised(UNPARSABLE.into())]);
        assert_eq!(
            decode(r#"{"type":"item.updated","item":{"id":"item_3","type":"todo_list"}}"#),
            [Event::Noise]
        );
    }

    #[test]
    fn a_usage_limit_on_stderr_explains_an_exit() {
        let diagnosis = harness().diagnose_exit("Error: You've hit your usage limit.\n");
        assert_eq!(diagnosis, Some(Event::RateLimited { resets_at: None }));
        assert_eq!(harness().diagnose_exit("Error: bad config\n"), None);
    }

    fn token_count(rate_limits: &str) -> String {
        format!(
            r#"{{"timestamp":"2026-11-04T01:13:05.786Z","type":"event_msg","payload":{{"type":"token_count","info":null,"rate_limits":{rate_limits}}}}}"#
        )
    }

    #[test]
    fn the_rollout_is_where_codex_writes_its_session() {
        let [pattern] = harness().artifacts() else {
            panic!("expected one artifact, got {:?}", harness().artifacts());
        };
        let glob = globset::Glob::new(pattern).unwrap().compile_matcher();
        assert!(glob.is_match(
            ".codex/sessions/2026/11/04/rollout-2026-11-04T01-13-04-019a4c5e-0000-7000-8000-000000000000.jsonl"
        ));
        assert!(!glob.is_match(".codex/auth.json"));
    }

    #[test]
    fn a_luna_rollout_reports_the_weekly_quota_and_nothing_else() {
        let events: Vec<Event> = transcript("luna-low-rollout.jsonl")
            .lines()
            .flat_map(|line| harness().decode_artifact(line))
            .collect();
        assert_eq!(
            events,
            [Event::Quota {
                window: "7d".into(),
                utilization: 0.59,
                resets_at: DateTime::from_timestamp(1_793_775_613, 0),
                allowed: true,
            }]
        );
    }

    #[test]
    fn a_reached_limit_disallows_every_window_named_by_its_length() {
        let line = token_count(
            r#"{"limit_id":"codex","primary":{"used_percent":100.0,"window_minutes":300,"resets_at":1793762400},"secondary":{"used_percent":72.5,"window_minutes":10080,"resets_at":null},"credits":null,"plan_type":"plus","rate_limit_reached_type":"primary"}"#,
        );
        assert_eq!(
            harness().decode_artifact(&line),
            [
                Event::Quota {
                    window: "5h".into(),
                    utilization: 1.0,
                    resets_at: DateTime::from_timestamp(1_793_762_400, 0),
                    allowed: false,
                },
                Event::Quota {
                    window: "7d".into(),
                    utilization: 0.725,
                    resets_at: None,
                    allowed: false,
                },
            ]
        );
    }

    #[test]
    fn a_token_count_without_rate_limits_says_nothing_but_a_changed_one_is_named() {
        assert_eq!(harness().decode_artifact(&token_count("null")), []);
        assert_eq!(
            harness().decode_artifact(&token_count(r#"{"primary":{"used_percent":"59"}}"#)),
            [Event::Unrecognised("token_count".into())]
        );
        assert_eq!(harness().decode_artifact("not json"), []);
    }

    #[test]
    fn replay_folds_the_rollout_quota_into_the_outcome() {
        let rollout = transcript("luna-low-rollout.jsonl");
        let later = token_count(
            r#"{"primary":{"used_percent":61.0,"window_minutes":10080,"resets_at":1793775613},"secondary":null,"rate_limit_reached_type":null}"#,
        );
        let ending = harness()
            .replay(&transcript("luna-none-success.jsonl"), &[&rollout, &later])
            .finish(ProcessEnd::Exited);
        let Ending::Finished(outcome) = ending else {
            panic!("expected a finished attempt, got {ending:?}");
        };
        assert_eq!(outcome.exit, Exit::Completed);
        assert_eq!(
            outcome.quota,
            [(
                "7d".to_owned(),
                QuotaSpan {
                    first: 0.59,
                    last: 0.61
                }
            )]
            .into()
        );
        assert!(outcome.unrecognised.is_empty());
    }
}
