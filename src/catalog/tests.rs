use super::*;
use crate::testing::id;
use tempfile::TempDir;

const CATALOG: &str = r#"
[sandbox]
packages = ["coreutils"]

[harnesses.claude-code]
package = "claude-code"

[backends.anthropic]
harness = "claude-code"
credential = { kind = "env-token", var = "TOKEN", file = "~/.config/gungnir/token" }
egress = ["api.anthropic.com"]

[backends.proxy]
harness = "claude-code"
endpoint = "https://api.example.com/anthropic"
credential = { kind = "api-key", var = "KEY", file = "/etc/gungnir/key" }
egress = ["api.example.com"]

[[models]]
id = "m"
efforts = ["low", "high"]
served-by = { anthropic = "m-api", proxy = "m-proxy" }
prices = [
  { since = "2026-01-01", input = 1.0, output = 2.0, cache-read = 0.1 },
  { since = "2026-01-01", above-prompt-tokens = 1000, input = 3.0, output = 6.0, cache-read = 0.3 },
  { since = "2026-06-01", input = 0.5, output = 1.0, cache-read = 0.05, cache-write = 0.6 },
]
"#;

fn load(text: &str) -> Result<Catalog> {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("gungnir.toml");
    std::fs::write(&path, text).unwrap();
    Catalog::load(&path)
}

/// Loading `CATALOG` with `from` replaced by `to` fails with a message naming the file and
/// every one of `needles`.
fn rejects(from: &str, to: &str, needles: &[&str]) {
    assert!(CATALOG.contains(from), "fixture lacks {from:?}");
    let err = load(&CATALOG.replacen(from, to, 1)).unwrap_err();
    let message = format!("{err:#}");
    assert!(message.contains("gungnir.toml"), "{message}");
    for needle in needles {
        assert!(
            message.contains(needle),
            "{needle:?} missing from: {message}"
        );
    }
}

fn day(text: &str) -> NaiveDate {
    text.parse().unwrap()
}

#[test]
fn loads_the_repository_catalog() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("gungnir.toml");
    let catalog = Catalog::load(&path).unwrap();

    let harnesses: Vec<&str> = catalog.harnesses().map(HarnessName::as_str).collect();
    assert_eq!(harnesses, ["claude-code", "codex"]);

    let zai = catalog.backend(&id("zai")).unwrap();
    assert_eq!(zai.harness().as_str(), "claude-code");
    assert_eq!(zai.endpoint().unwrap().host(), "api.z.ai");
    assert!(
        matches!(zai.credential(), Credential::ApiKey { var, .. } if var == "ANTHROPIC_AUTH_TOKEN")
    );

    let haiku = catalog.model(&id("claude-haiku-5-5")).unwrap();
    assert_eq!(haiku.rates(day("2026-11-03"), 100_000).unwrap().input, 0.1);
    assert_eq!(haiku.rates(day("2026-11-03"), 100_001).unwrap().input, 0.5);

    let glm = catalog
        .system(&id("glm-5.3"), &id("zai"), Effort::High, id("2.1.316"))
        .unwrap();
    assert_eq!(glm.id().to_string(), "claude-code-2.1.316_zai_glm-5.3_high");
    assert_eq!(glm.api_model(), "glm-5.3");
}

#[test]
fn systems_need_a_serving_backend_and_an_offered_effort() {
    let catalog = load(CATALOG).unwrap();
    let system = catalog
        .system(&id("m"), &id("proxy"), Effort::Low, id("1.0.0"))
        .unwrap();
    assert_eq!(system.id().to_string(), "claude-code-1.0.0_proxy_m_low");
    assert_eq!(system.api_model(), "m-proxy");

    let err = catalog
        .system(&id("m"), &id("proxy"), Effort::Max, id("1.0.0"))
        .unwrap_err();
    assert!(err.to_string().contains("low, high"), "{err}");
    let unknown_model = catalog.system(&id("n"), &id("proxy"), Effort::Low, id("1.0.0"));
    assert!(unknown_model.is_err());
    let unknown_backend = catalog.system(&id("m"), &id("zai"), Effort::Low, id("1.0.0"));
    assert!(unknown_backend.is_err());
}

#[test]
fn rates_follow_the_price_date_and_prompt_tier() {
    let catalog = load(CATALOG).unwrap();
    let model = catalog.model(&id("m")).unwrap();
    assert_eq!(model.rates(day("2025-12-31"), 10), None);
    assert_eq!(model.rates(day("2026-01-01"), 1000).unwrap().input, 1.0);
    assert_eq!(model.rates(day("2026-05-31"), 1001).unwrap().input, 3.0);
    let june = model.rates(day("2026-06-01"), 1001).unwrap();
    assert_eq!((june.input, june.cache_write), (0.5, Some(0.6)));
}

#[test]
fn credential_paths_expand_home_only_when_used() {
    let catalog = load(CATALOG).unwrap();
    let home = Path::new("/home/someone");
    let file = |backend: &str| match catalog.backend(&id(backend)).unwrap().credential() {
        Credential::EnvToken { file, .. } | Credential::ApiKey { file, .. } => file.clone(),
        Credential::CodexHome { .. } => unreachable!(),
    };
    assert_eq!(file("anthropic").to_string(), "~/.config/gungnir/token");
    assert_eq!(
        file("anthropic").expand(home),
        Path::new("/home/someone/.config/gungnir/token")
    );
    assert_eq!(file("proxy").expand(home), Path::new("/etc/gungnir/key"));
}

#[test]
fn backends_must_name_a_declared_harness() {
    rejects(
        "[harnesses.claude-code]",
        "[harnesses.codex]",
        &["[backends.anthropic]", "claude-code"],
    );
}

#[test]
fn models_must_be_served_by_declared_backends() {
    rejects(
        "proxy = \"m-proxy\"",
        "openai = \"m-proxy\"",
        &["id = \"m\"", "openai"],
    );
}

#[test]
fn effort_lists_must_be_non_empty_known_and_unique() {
    rejects(
        "efforts = [\"low\", \"high\"]",
        "efforts = []",
        &["efforts is empty"],
    );
    rejects(
        "efforts = [\"low\", \"high\"]",
        "efforts = [\"low\", \"low\"]",
        &["id = \"m\"", "low twice"],
    );
    rejects("\"low\", \"high\"", "\"low\", \"turbo\"", &["turbo"]);
}

#[test]
fn prices_must_be_well_formed() {
    rejects(
        "above-prompt-tokens = 1000, ",
        "",
        &["id = \"m\"", "2026-01-01", "two base tiers"],
    );
    rejects(
        "{ since = \"2026-06-01\", input",
        "{ since = \"2026-06-01\", above-prompt-tokens = 5, input",
        &["2026-06-01", "no base tier"],
    );
    rejects("input = 1.0", "input = -1.0", &["-1"]);
}

#[test]
fn endpoints_must_be_https_hosts_the_egress_allows() {
    rejects(
        "egress = [\"api.example.com\"]",
        "egress = [\"other.example.com\"]",
        &["[backends.proxy]", "api.example.com"],
    );
    rejects(
        "https://api.example.com",
        "http://api.example.com",
        &["[backends.proxy]", "http://api.example.com/anthropic"],
    );
    rejects(
        "endpoint = \"https://api.example.com/anthropic\"\n",
        "",
        &["[backends.proxy]", "api-key", "endpoint"],
    );
}

#[test]
fn credentials_must_name_a_variable_and_a_home_or_absolute_path() {
    rejects(
        "file = \"~/.config/gungnir/token\"",
        "file = \"token\"",
        &["[backends.anthropic]", "\"token\""],
    );
    rejects("var = \"TOKEN\"", "var = \"my token\"", &["\"my token\""]);
}

#[test]
fn unknown_keys_are_rejected() {
    rejects(
        "egress = [\"api.anthropic.com\"]",
        "egres = [\"api.anthropic.com\"]",
        &["egres"],
    );
}
