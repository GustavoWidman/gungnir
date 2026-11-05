//! Readings of a backend's quota that gungnir takes itself, from the host, where a vendor
//! publishes them apart from the agent's transcript. The request carries the backend's key;
//! no error or log line does.
use crate::{catalog::QuotaSource, harness::CredentialMaterial, secret::Redactor, vault::Key};
use chrono::{DateTime, Utc};
use eyre::{Result, WrapErr, bail, ensure, eyre};
use reqwest::header::{AUTHORIZATION, HeaderValue};
use serde::Deserialize;
use std::time::Duration;

/// How long one reading may take before it is an error.
const TIMEOUT: Duration = Duration::from_secs(20);
/// How much of a response body an error quotes.
const QUOTED: usize = 2048;

/// How much of one quota window the account has used, as one reading reports it.
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    /// The window's length, such as `5h`, the way Codex's windows are named.
    pub name: String,
    /// The fraction used: 1.0 once the window admits no more requests.
    pub used: f64,
    /// When the window starts over, if the vendor says.
    pub resets_at: Option<DateTime<Utc>>,
}

/// Reads quota sources over HTTPS.
#[derive(Debug)]
pub struct Reader {
    client: reqwest::Client,
}

impl Reader {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent(concat!("gungnir/", env!("CARGO_PKG_VERSION")))
            .build()
            .wrap_err("building the HTTP client for quota readings")?;
        Ok(Self { client })
    }

    /// The windows `source` reports now, read with `key`, its backend's credential.
    pub async fn read(&self, source: &QuotaSource, key: &Key) -> Result<Vec<Window>> {
        let QuotaSource::ZaiMonitor { url } = source;
        let url = url.as_str();
        let CredentialMaterial::Env { value, .. } = key.material() else {
            bail!("reading {url} takes an API key, and the backend's credential is none");
        };
        let mut authorization = HeaderValue::from_str(value.expose())
            .map_err(|_| eyre!("reading {url}: the API key is not a valid header value"))?;
        authorization.set_sensitive(true);
        let redactor = Redactor::new(key.secrets());
        let failed = |err: reqwest::Error| {
            let message = format!("{:#}", eyre::Report::new(err));
            eyre!("reading {url}: {}", quote(&redactor, message.as_bytes()))
        };
        let response = self
            .client
            .get(url)
            .header(AUTHORIZATION, authorization)
            .send()
            .await
            .map_err(failed)?;
        let status = response.status();
        let body = response.bytes().await.map_err(failed)?;
        ensure!(
            status.is_success(),
            "reading {url}: HTTP {status}: {}",
            quote(&redactor, &body)
        );
        zai_monitor(&body).map_err(|err| {
            eyre!(
                "reading {url}: {err:#}, in the response {}",
                quote(&redactor, &body)
            )
        })
    }
}

/// `bytes` without the secrets `redactor` knows, cut to [`QUOTED`] bytes, as text.
fn quote(redactor: &Redactor, bytes: &[u8]) -> String {
    let redacted = redactor.redact(bytes);
    let cut = &redacted[..redacted.len().min(QUOTED)];
    let mut text = String::from_utf8_lossy(cut).into_owned();
    if cut.len() < redacted.len() {
        text.push_str("...");
    }
    text
}

/// The token windows in a response of Z.ai's monitor. The envelope must be as Z.ai sends it,
/// so that a changed format fails here rather than reading as an unused quota. A limit of
/// another type, such as the monthly web-tool quota (`TIME_LIMIT`) that no agent here spends,
/// or of a unit the code cannot name, is left out.
fn zai_monitor(body: &[u8]) -> Result<Vec<Window>> {
    let envelope: ZaiEnvelope =
        serde_json::from_slice(body).map_err(|err| eyre!("not the monitor's envelope: {err}"))?;
    ensure!(
        envelope.success && envelope.code == 200,
        "the monitor answered code {}: {}",
        envelope.code,
        envelope.msg.as_deref().unwrap_or("no message")
    );
    let data = envelope
        .data
        .ok_or_else(|| eyre!("the monitor answered no data"))?;
    let mut windows = Vec::new();
    for limit in data.limits {
        if limit.kind != "TOKENS_LIMIT" {
            continue;
        }
        let (Some(unit), Some(number)) = (limit.unit, limit.number) else {
            bail!("a TOKENS_LIMIT has no unit or number");
        };
        // Unit 3 is hours, as in the plan's 5-hour window; unit 6 is days, which a weekly
        // window reports as number 7.
        let name = match unit {
            3 => format!("{number}h"),
            6 => format!("{number}d"),
            _ => continue,
        };
        let percentage = limit
            .percentage
            .filter(|percentage| percentage.is_finite() && *percentage >= 0.0)
            .ok_or_else(|| eyre!("the {name} TOKENS_LIMIT has no percentage used"))?;
        let resets_at = limit
            .next_reset_time
            .map(|ms| {
                DateTime::from_timestamp_millis(ms)
                    .ok_or_else(|| eyre!("the {name} TOKENS_LIMIT resets at {ms}, not a time"))
            })
            .transpose()?;
        windows.push(Window {
            name,
            used: percentage / 100.0,
            resets_at,
        });
    }
    Ok(windows)
}

#[derive(Deserialize)]
struct ZaiEnvelope {
    code: i64,
    msg: Option<String>,
    success: bool,
    data: Option<ZaiData>,
}

#[derive(Deserialize)]
struct ZaiData {
    limits: Vec<ZaiLimit>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ZaiLimit {
    #[serde(rename = "type")]
    kind: String,
    unit: Option<u32>,
    number: Option<u32>,
    percentage: Option<f64>,
    next_reset_time: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secret::Secret;

    /// A response captured from a Pro plan whose 5-hour window was spent.
    const SPENT: &str = include_str!("../tests/fixtures/quota/zai-limit-spent.json");

    fn at(text: &str) -> DateTime<Utc> {
        text.parse().unwrap()
    }

    #[test]
    fn the_token_window_is_named_by_its_length_and_the_web_tool_quota_left_out() {
        assert_eq!(
            zai_monitor(SPENT.as_bytes()).unwrap(),
            [Window {
                name: "5h".to_owned(),
                used: 1.0,
                resets_at: Some(at("2026-11-05T08:57:03.688Z")),
            }]
        );
    }

    #[test]
    fn a_weekly_window_counts_and_a_unit_without_a_name_is_left_out() {
        let body = SPENT
            .replace(r#""percentage":100,"#, r#""percentage":44,"#)
            .replace(
                r#"],"level""#,
                r#",{"type":"TOKENS_LIMIT","unit":6,"number":7,"percentage":12.5,"nextResetTime":null},{"type":"TOKENS_LIMIT","unit":9,"number":1,"percentage":99}],"level""#,
            );
        let windows = zai_monitor(body.as_bytes()).unwrap();
        let named: Vec<(&str, f64, bool)> = windows
            .iter()
            .map(|w| (w.name.as_str(), w.used, w.resets_at.is_some()))
            .collect();
        assert_eq!(named, [("5h", 0.44, true), ("7d", 0.125, false)]);
    }

    #[test]
    fn a_refused_or_changed_response_is_an_error_that_says_why() {
        let refused = r#"{"code":1001,"msg":"Authorization token expired","success":false}"#;
        let message = format!("{:#}", zai_monitor(refused.as_bytes()).unwrap_err());
        assert!(message.contains("1001"), "{message}");
        assert!(message.contains("token expired"), "{message}");

        for changed in [
            "<html>Bad gateway</html>".to_owned(),
            SPENT.replace(r#""limits""#, r#""quotas""#),
            SPENT.replace(r#""percentage":100,"#, ""),
            SPENT.replace(r#""success":true"#, r#""ok":true"#),
        ] {
            assert!(zai_monitor(changed.as_bytes()).is_err(), "{changed}");
        }
    }

    #[test]
    fn a_quoted_body_never_shows_the_key_and_stays_short() {
        let key = Secret::new("zai-key.123".to_owned());
        let redactor = Redactor::new([&key]);
        let quoted = quote(
            &redactor,
            format!("bad key zai-key.123 {}", "x".repeat(5000)).as_bytes(),
        );
        assert!(!quoted.contains("zai-key.123"), "{quoted}");
        assert!(quoted.starts_with("bad key [REDACTED] x"), "{quoted}");
        assert!(quoted.len() < QUOTED + 10, "{}", quoted.len());
    }
}
