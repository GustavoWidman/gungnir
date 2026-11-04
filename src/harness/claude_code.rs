//! Claude Code, run headless as `claude -p` with stream-json output.
use super::{
    CredentialMaterial, Driver, Event, Granularity, Launch, LaunchRequest, TokenUsage, UNPARSABLE,
    UsageSample, mentions, plain, utf8,
};
use crate::{
    id::{Effort, HarnessVersion},
    secret::EnvValue,
};
use chrono::{DateTime, Utc};
use eyre::{Result, bail, eyre};
use serde::Deserialize;
use std::{collections::BTreeMap, time::Duration};

pub(super) struct ClaudeCode;

/// Rate-limit statuses that mean a window admits no more requests.
const LIMITED_STATUSES: &[&str] = &["rejected"];
/// The `error` Claude Code puts on its own message about a request refused for quota.
const LIMIT_ERRORS: &[&str] = &["rate_limit"];
/// Lowercase phrases in an error that mean the account's quota ended the session.
const LIMIT_PHRASES: &[&str] = &["usage limit", "hit your limit", "rate limit"];
/// The model Claude Code names on messages it writes itself, such as API errors.
const SYNTHETIC_MODEL: &str = "<synthetic>";

impl Driver for ClaudeCode {
    fn name(&self) -> &'static str {
        "claude-code"
    }

    fn parser_version(&self) -> u32 {
        1
    }

    fn granularity(&self) -> Granularity {
        Granularity::Request
    }

    /// `--effort` takes `low` through `max`; there is no `none` or `minimal`.
    fn accepts(&self, effort: Effort) -> bool {
        effort >= Effort::Low
    }

    fn parse_version(&self, stdout: &str) -> Result<HarnessVersion> {
        stdout
            .trim()
            .strip_suffix(" (Claude Code)")
            .ok_or_else(|| eyre!("claude --version printed {stdout:?}"))?
            .parse()
    }

    fn launch(&self, req: &LaunchRequest<'_>) -> Result<Launch> {
        let CredentialMaterial::Env { var, value } = req.credential else {
            bail!("claude-code takes its credential from an environment variable");
        };
        let api_model = req.system.api_model();
        let mut env = vec![
            plain("CLAUDE_CONFIG_DIR", utf8(&req.home.join(".claude"))?),
            plain("DISABLE_AUTOUPDATER", "1"),
            plain("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1"),
            plain("DISABLE_TELEMETRY", "1"),
            plain("DISABLE_ERROR_REPORTING", "1"),
            (var.clone(), EnvValue::Secret(value.clone())),
        ];
        if let Some(endpoint) = req.endpoint {
            env.push(plain("ANTHROPIC_BASE_URL", endpoint.as_str()));
            // Claude Code sends side requests to Claude models by family, which an endpoint
            // serving one model would reject.
            for family in ["OPUS", "SONNET", "HAIKU"] {
                env.push(plain(
                    &format!("ANTHROPIC_DEFAULT_{family}_MODEL"),
                    api_model,
                ));
            }
        }
        // `--bare` would also skip the user's settings, but it refuses OAuth tokens.
        let argv = [
            "claude",
            "-p",
            "--output-format",
            "stream-json",
            "--verbose",
            "--model",
            api_model,
            "--effort",
            req.system.effort().as_str(),
            "--dangerously-skip-permissions",
            "--disallowedTools",
            "WebSearch",
            "WebFetch",
        ];
        Ok(Launch {
            argv: argv.map(str::to_owned).into(),
            env,
            files: vec![],
            stdin: req.prompt.to_owned(),
        })
    }

    fn decode(&self, line: &str) -> Vec<Event> {
        match serde_json::from_str(line) {
            Ok(RawLine::Known(known)) => known.events(),
            Ok(RawLine::Other { kind, subtype }) => vec![Event::Unrecognised(match subtype {
                Some(subtype) => format!("{kind}/{subtype}"),
                None => kind,
            })],
            Err(_) => vec![Event::Unrecognised(UNPARSABLE.into())],
        }
    }

    fn diagnose_exit(&self, stderr_tail: &str) -> Option<Event> {
        mentions(stderr_tail, LIMIT_PHRASES).then_some(Event::RateLimited { resets_at: None })
    }
}

/// A line of a kind this parser knows, or failing that, the kind it names. A known kind whose
/// shape has changed also falls through to `Other`, so drift is counted rather than misread.
#[derive(Deserialize)]
#[serde(untagged)]
enum RawLine {
    Known(Known),
    Other {
        #[serde(rename = "type")]
        kind: String,
        subtype: Option<String>,
    },
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Known {
    System(RawSystem),
    Assistant(RawAssistant),
    User {},
    RateLimitEvent { rate_limit_info: RawRateLimit },
    Result(RawResult),
}

#[derive(Deserialize)]
#[serde(tag = "subtype", rename_all = "snake_case")]
enum RawSystem {
    Init { model: String, tools: Vec<String> },
    ThinkingTokens {},
    ApiRetry {},
    CompactBoundary {},
}

/// One content block of one API response. A response with several blocks arrives as several
/// lines with the same id, each carrying the usage known when its block ended.
#[derive(Deserialize)]
struct RawAssistant {
    message: RawMessage,
    error: Option<String>,
}

#[derive(Deserialize)]
struct RawMessage {
    id: String,
    model: String,
    stop_reason: Option<String>,
    usage: RawUsage,
}

#[derive(Deserialize)]
struct RawUsage {
    input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_creation: Option<RawCacheCreation>,
    output_tokens: Option<u64>,
}

#[derive(Deserialize)]
struct RawCacheCreation {
    ephemeral_1h_input_tokens: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRateLimit {
    status: String,
    resets_at: Option<i64>,
    #[serde(default)]
    unified_windows: BTreeMap<String, RawWindow>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawWindow {
    utilization: Option<f64>,
    resets_at: Option<i64>,
    status: Option<String>,
}

/// The session's final line. Its `total_cost_usd` is priced at Claude rates whatever the
/// endpoint, so it is not read.
#[derive(Deserialize)]
struct RawResult {
    subtype: String,
    #[serde(default)]
    is_error: bool,
    num_turns: Option<u32>,
    duration_api_ms: Option<u64>,
    stop_reason: Option<String>,
    result: Option<String>,
    #[serde(default)]
    errors: Vec<String>,
    api_error_status: Option<u16>,
    #[serde(default, rename = "modelUsage")]
    model_usage: BTreeMap<String, RawModelUsage>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawModelUsage {
    input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_creation_ephemeral_1h_input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    thinking_tokens: Option<u64>,
}

impl Known {
    fn events(self) -> Vec<Event> {
        match self {
            Self::System(RawSystem::Init { model, tools }) => vec![Event::Init { model, tools }],
            Self::System(_) | Self::User {} => vec![Event::Noise],
            Self::Assistant(assistant) => assistant.events(),
            Self::RateLimitEvent { rate_limit_info } => rate_limit_info.events(),
            Self::Result(result) => result.events(),
        }
    }
}

impl RawAssistant {
    fn events(self) -> Vec<Event> {
        if self.message.model == SYNTHETIC_MODEL {
            let limited = self
                .error
                .is_some_and(|error| LIMIT_ERRORS.contains(&error.as_str()));
            return vec![if limited {
                Event::RateLimited { resets_at: None }
            } else {
                Event::Noise
            }];
        }
        let refused = self.message.stop_reason.as_deref() == Some("refusal");
        let usage = Event::Usage(UsageSample {
            request: Some(self.message.id),
            model: Some(self.message.model),
            tokens: self.message.usage.tokens(),
        });
        if refused {
            vec![usage, Event::Refused]
        } else {
            vec![usage]
        }
    }
}

impl RawUsage {
    /// Claude Code copies this usage from the start of the response, so `output_tokens`
    /// undercounts until the result's session total tops it up.
    fn tokens(&self) -> TokenUsage {
        let written = self.cache_creation_input_tokens.unwrap_or(0);
        let long = self
            .cache_creation
            .as_ref()
            .and_then(|cache| cache.ephemeral_1h_input_tokens)
            .unwrap_or(0);
        TokenUsage {
            input: self.input_tokens.unwrap_or(0),
            cache_read: self.cache_read_input_tokens.unwrap_or(0),
            cache_write: written.saturating_sub(long),
            cache_write_long: long,
            output: self.output_tokens.unwrap_or(0),
            reasoning: None,
        }
    }
}

impl RawRateLimit {
    fn events(self) -> Vec<Event> {
        let mut events: Vec<Event> = self
            .unified_windows
            .into_iter()
            .filter_map(|(window, raw)| {
                Some(Event::Quota {
                    window,
                    utilization: raw.utilization?,
                    resets_at: raw.resets_at.and_then(timestamp),
                    allowed: !raw.status.is_some_and(|status| limited(&status)),
                })
            })
            .collect();
        if limited(&self.status) {
            events.push(Event::RateLimited {
                resets_at: self.resets_at.and_then(timestamp),
            });
        }
        events
    }
}

impl RawResult {
    fn events(self) -> Vec<Event> {
        let mut events: Vec<Event> = self
            .model_usage
            .into_iter()
            .map(|(model, usage)| Event::SessionUsage {
                model,
                tokens: usage.tokens(),
            })
            .collect();
        if self.stop_reason.as_deref() == Some("refusal") {
            events.push(Event::Refused);
        }
        // A result can say `success` and still be an error.
        let failed = self.is_error || self.subtype != "success";
        let error = failed.then(|| {
            self.result
                .filter(|text| !text.is_empty())
                .or_else(|| (!self.errors.is_empty()).then(|| self.errors.join("; ")))
                .unwrap_or(self.subtype)
        });
        if let Some(message) = &error
            && (self.api_error_status == Some(429) || mentions(message, LIMIT_PHRASES))
        {
            events.push(Event::RateLimited { resets_at: None });
        }
        events.push(Event::Finished {
            turns: self.num_turns,
            api_time: self.duration_api_ms.map(Duration::from_millis),
            error,
        });
        events
    }
}

impl RawModelUsage {
    fn tokens(&self) -> TokenUsage {
        let written = self.cache_creation_input_tokens.unwrap_or(0);
        let long = self.cache_creation_ephemeral_1h_input_tokens.unwrap_or(0);
        TokenUsage {
            input: self.input_tokens.unwrap_or(0),
            cache_read: self.cache_read_input_tokens.unwrap_or(0),
            cache_write: written.saturating_sub(long),
            cache_write_long: long,
            output: self.output_tokens.unwrap_or(0),
            // Claude Code reports zero both for a model that did not think and for an
            // endpoint that does not count thinking apart from output, such as Z.ai.
            reasoning: self.thinking_tokens.filter(|&tokens| tokens > 0),
        }
    }
}

fn limited(status: &str) -> bool {
    LIMITED_STATUSES.contains(&status)
}

fn timestamp(seconds: i64) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp(seconds, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        harness::{AgentOutcome, Ending, Exit, Fold, Harness, ProcessEnd},
        secret::Secret,
        testing::{self, id},
    };
    use std::{fs, path::Path};

    fn harness() -> Harness {
        Harness(&ClaudeCode)
    }

    fn transcript(name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/harness/claude-code")
            .join(name);
        fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
    }

    fn replay(name: &str, end: ProcessEnd) -> Ending {
        harness().replay(&transcript(name), &[]).finish(end)
    }

    fn outcome(name: &str) -> AgentOutcome {
        match replay(name, ProcessEnd::Exited) {
            Ending::Finished(outcome) => outcome,
            ending => panic!("{name} did not finish: {ending:?}"),
        }
    }

    fn requests(outcome: &AgentOutcome) -> Vec<Option<&str>> {
        outcome
            .usage
            .iter()
            .map(|sample| sample.request.as_deref())
            .collect()
    }

    fn launch(model: &str, backend: &str, var: &str) -> Result<Launch> {
        let catalog = testing::catalog();
        let system = catalog.system(&id(model), &id(backend), Effort::High, id("2.1.316"))?;
        let credential = CredentialMaterial::Env {
            var: var.into(),
            value: Secret::new("sk-test".into()),
        };
        harness().launch(&LaunchRequest {
            system: &system,
            endpoint: catalog.backend(&id(backend)).unwrap().endpoint(),
            credential: &credential,
            home: Path::new("/home/agent"),
            workspace: Path::new("/work"),
            prompt: "Reply with the single word ok",
        })
    }

    fn env(launch: &Launch) -> BTreeMap<&str, &EnvValue> {
        launch
            .env
            .iter()
            .map(|(name, value)| (name.as_str(), value))
            .collect()
    }

    fn plain(value: &str) -> EnvValue {
        EnvValue::Plain(value.into())
    }

    #[test]
    fn launches_print_mode_with_the_prompt_on_stdin() {
        let launch = launch("opus", "anthropic", "TOKEN").unwrap();
        assert_eq!(
            launch.argv,
            [
                "claude",
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--model",
                "claude-opus",
                "--effort",
                "high",
                "--dangerously-skip-permissions",
                "--disallowedTools",
                "WebSearch",
                "WebFetch",
            ]
        );
        assert_eq!(launch.stdin, "Reply with the single word ok");
        assert!(launch.files.is_empty());

        let env = env(&launch);
        assert_eq!(env["CLAUDE_CONFIG_DIR"], &plain("/home/agent/.claude"));
        for quiet in [
            "DISABLE_AUTOUPDATER",
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
            "DISABLE_TELEMETRY",
            "DISABLE_ERROR_REPORTING",
        ] {
            assert_eq!(env[quiet], &plain("1"), "{quiet}");
        }
        assert_eq!(
            env["TOKEN"],
            &EnvValue::Secret(Secret::new("sk-test".into()))
        );
        assert!(!env.contains_key("ANTHROPIC_BASE_URL"));
        assert!(!env.contains_key("ANTHROPIC_DEFAULT_HAIKU_MODEL"));
    }

    #[test]
    fn an_endpoint_backend_gets_every_model_family_mapped_to_its_model() {
        let launch = launch("glm", "zai", "KEY").unwrap();
        let env = env(&launch);
        assert_eq!(
            env["ANTHROPIC_BASE_URL"],
            &plain("https://api.z.ai/api/anthropic")
        );
        assert_eq!(env["KEY"], &EnvValue::Secret(Secret::new("sk-test".into())));
        for family in ["OPUS", "SONNET", "HAIKU"] {
            let var = format!("ANTHROPIC_DEFAULT_{family}_MODEL");
            assert_eq!(env[var.as_str()], &plain("glm"), "{var}");
        }
        assert_eq!(launch.argv[6], "glm");
    }

    #[test]
    fn a_codex_home_credential_cannot_launch_claude_code() {
        let catalog = testing::catalog();
        let system = catalog
            .system(&id("opus"), &id("anthropic"), Effort::Low, id("2.1.316"))
            .unwrap();
        let credential = CredentialMaterial::CodexAuth(Secret::new("{}".into()));
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
    fn carries_low_through_max() {
        let carried: Vec<Effort> = [
            Effort::None,
            Effort::Minimal,
            Effort::Low,
            Effort::Medium,
            Effort::High,
            Effort::Xhigh,
            Effort::Max,
        ]
        .into_iter()
        .filter(|&effort| harness().accepts(effort))
        .collect();
        assert_eq!(
            carried,
            [
                Effort::Low,
                Effort::Medium,
                Effort::High,
                Effort::Xhigh,
                Effort::Max
            ]
        );
    }

    #[test]
    fn reads_the_version_line() {
        let version = harness().parse_version("2.1.316 (Claude Code)\n").unwrap();
        assert_eq!(version.as_str(), "2.1.316");
        assert!(harness().parse_version("codex-cli 0.166.0").is_err());
    }

    #[test]
    fn a_haiku_run_with_a_tool_call_totals_what_the_result_reports() {
        let outcome = outcome("haiku-bash-success.jsonl");
        assert_eq!(outcome.exit, Exit::Completed);
        assert_eq!(outcome.turns, Some(2));
        assert_eq!(outcome.model.as_deref(), Some("claude-haiku-5-5"));
        assert_eq!(outcome.api_time, Some(Duration::from_millis(1634)));
        assert_eq!(
            outcome.total(),
            TokenUsage {
                input: 4,
                cache_read: 28362,
                cache_write: 0,
                cache_write_long: 6147,
                output: 78,
                reasoning: None,
            }
        );
        assert_eq!(
            requests(&outcome),
            [
                Some("msg_011CggrGYBbFoFktY7FQgNGz"),
                Some("msg_011CggrGdBUeUDCtfMm5V2z8"),
                None
            ]
        );
        assert_eq!(outcome.usage[0].tokens.cache_write_long, 6042);
        assert_eq!(outcome.quota["five_hour"].last, 0.17);
        assert_eq!(outcome.quota["seven_day"].first, 0.53);
        assert!(outcome.unrecognised.is_empty());
    }

    #[test]
    fn glm_runs_through_zai_total_the_result_and_show_the_effort() {
        let low = outcome("glm-flash-low.jsonl");
        let max = outcome("glm-flash-max.jsonl");
        for run in [&low, &max] {
            assert_eq!(run.exit, Exit::Completed);
            assert_eq!(run.turns, Some(1));
            assert_eq!(run.model.as_deref(), Some("glm-5.3-flash"));
            assert_eq!(run.total().input, 10741);
            assert_eq!(run.total().reasoning, None);
            assert!(run.quota.is_empty());
            assert!(run.unrecognised.is_empty());
        }
        assert_eq!(low.total().output, 84);
        assert_eq!(max.total().output, 1437);
    }

    #[test]
    fn an_unknown_model_is_an_error_that_spent_nothing() {
        let outcome = outcome("unknown-model-error.jsonl");
        let Exit::Errored { message } = &outcome.exit else {
            panic!("expected an error, got {:?}", outcome.exit);
        };
        assert!(message.contains("claude-nonexistent-9"), "{message}");
        assert!(outcome.usage.is_empty());
    }

    #[test]
    fn a_refusal_stop_reason_is_a_refusal() {
        let outcome = outcome("synthetic-refusal.jsonl");
        assert_eq!(outcome.exit, Exit::Refused);
        assert_eq!(outcome.total().output, 9);
    }

    /// Folds `name` line by line, as the runner does live, and returns the index of the line
    /// after which the fold first reported a rate limit.
    fn feed(name: &str) -> (Fold, Option<usize>) {
        let mut fold = Fold::default();
        let mut limited_at = None;
        for (index, line) in transcript(name).lines().enumerate() {
            for event in harness().decode(line) {
                fold.push(event);
            }
            if fold.rate_limited() {
                limited_at.get_or_insert(index);
            }
        }
        (fold, limited_at)
    }

    #[test]
    fn a_rejected_window_rate_limits_the_run_as_soon_as_it_arrives() {
        let (fold, limited_at) = feed("synthetic-rate-limit-event.jsonl");
        assert_eq!(
            limited_at,
            Some(4),
            "the rejected rate_limit_event is line 4"
        );

        let Ending::RateLimited { resets_at, usage } = fold.finish(ProcessEnd::Killed) else {
            panic!("expected a rate limit");
        };
        assert_eq!(resets_at, timestamp(1_793_762_400));
        let output: u64 = usage.iter().map(|sample| sample.tokens.output).sum();
        assert_eq!(output, 40);
    }

    #[test]
    fn an_error_result_naming_the_usage_limit_rate_limits_the_run() {
        let (fold, limited_at) = feed("synthetic-usage-limit-result.jsonl");
        assert_eq!(
            limited_at,
            Some(2),
            "the API retry on line 1 is not a rate limit"
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
    fn unknown_kinds_are_named_and_known_noise_is_not() {
        let decode = |line: &str| harness().decode(line);
        assert_eq!(
            decode(r#"{"type":"telemetry","n":1}"#),
            [Event::Unrecognised("telemetry".into())]
        );
        assert_eq!(
            decode(r#"{"type":"system","subtype":"plan_mode"}"#),
            [Event::Unrecognised("system/plan_mode".into())]
        );
        assert_eq!(
            decode(r#"{"type":"result","num_turns":"two"}"#),
            [Event::Unrecognised("result".into())]
        );
        assert_eq!(decode("not json"), [Event::Unrecognised(UNPARSABLE.into())]);
        assert_eq!(
            decode(r#"{"type":"system","subtype":"thinking_tokens","estimated_tokens":3}"#),
            [Event::Noise]
        );
    }

    #[test]
    fn a_usage_limit_on_stderr_explains_an_exit() {
        let diagnosis = harness().diagnose_exit("Error: Claude AI usage limit reached\n");
        assert_eq!(diagnosis, Some(Event::RateLimited { resets_at: None }));
        assert_eq!(harness().diagnose_exit("Error: invalid flag --foo\n"), None);
    }
}
