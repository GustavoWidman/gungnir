//! Codex, run headless as `codex exec --json` with the prompt on stdin.
use super::{
    CredentialMaterial, Driver, Event, Granularity, HomeFile, Launch, LaunchRequest, TokenUsage,
    UNPARSABLE, UsageSample, mentions, plain, utf8,
};
use crate::id::{Effort, HarnessVersion};
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
        1
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
            "--ephemeral",
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
        harness::{AgentOutcome, Ending, Exit, Fold, Harness, ProcessEnd},
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
            .replay(&transcript(name))
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
                "--ephemeral",
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
}
