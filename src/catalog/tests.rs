use super::*;
use crate::testing::id;
use tempfile::TempDir;

const CATALOG: &str = r#"
[sandbox]
packages = ["coreutils"]

[measure]
packages = ["hyperfine"]

[profile]
packages = ["perf"]

[harnesses.claude-code]
package = "claude-code"

[harnesses.codex]
package = "codex"

[backends.anthropic]
harness = "claude-code"
credential = { kind = "env-token", var = "TOKEN", file = "~/.config/gungnir/token" }
egress = ["api.anthropic.com"]

[backends.proxy]
harness = "claude-code"
endpoint = "https://api.example.com/anthropic"
credential = { kind = "api-key", var = "KEY", file = "/etc/gungnir/key" }
egress = ["api.example.com"]
quota = { kind = "zai-monitor", url = "https://api.example.com/monitor/quota" }

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
    let relaying: Vec<&str> = [
        "anthropic",
        "openai",
        "zai",
        "nvidia",
        "cliproxy",
        "lmstudio",
    ]
    .into_iter()
    .filter(|backend| catalog.backend(&id(backend)).unwrap().relays())
    .collect();
    assert_eq!(relaying, ["nvidia", "cliproxy", "lmstudio"]);
    assert!(
        matches!(zai.credential(), Credential::ApiKey { var, .. } if var == "ANTHROPIC_AUTH_TOKEN")
    );
    let Some(QuotaSource::ZaiMonitor { url }) = zai.quota() else {
        panic!("zai has no quota source");
    };
    assert_eq!(
        url.as_str(),
        "https://api.z.ai/api/monitor/usage/quota/limit"
    );
    assert_eq!(catalog.backend(&id("anthropic")).unwrap().quota(), None);

    let haiku = catalog.model(&id("claude-haiku-5-5")).unwrap();
    assert_eq!(
        haiku.rates(day("2026-11-03"), 100_000).unwrap().input,
        Rate(100_000)
    );
    assert_eq!(
        haiku.rates(day("2026-11-03"), 100_001).unwrap().input,
        Rate(500_000)
    );
    let glm = catalog.model(&id("glm-5.3")).unwrap();
    assert_eq!(
        glm.rates(day("2026-11-03"), 0).unwrap().cache_read,
        Rate(260_000)
    );

    let glm = catalog
        .system(&id("glm-5.3"), &id("zai"), Effort::High, id("2.1.316"))
        .unwrap();
    assert_eq!(glm.id().to_string(), "claude-code-2.1.316_zai_glm-5.3_high");
    assert_eq!(glm.api_model(), "glm-5.3");
    for model in ["glm-5.3", "glm-5.3-flash"] {
        let ceiling = catalog.model(&id(model)).unwrap().max_output_tokens();
        assert_eq!(ceiling.map(u32::from), Some(131_072), "{model}");
    }
    assert_eq!(glm.max_output_tokens().map(u32::from), Some(131_072));
    assert_eq!(haiku.max_output_tokens().map(u32::from), Some(128_000));

    let nvidia = catalog.backend(&id("nvidia")).unwrap();
    assert_eq!(nvidia.harness().as_str(), "codex");
    assert_eq!(
        nvidia.endpoint().unwrap().as_str(),
        "https://integrate.api.nvidia.com/v1"
    );
    assert!(
        matches!(nvidia.credential(), Credential::ApiKey { var, .. } if var == "NVIDIA_API_KEY")
    );
    assert_eq!(nvidia.quota(), None);
    let nemotron = catalog.model(&id("nemotron-3.5-super-vl")).unwrap();
    let free = nemotron.rates(day("2026-11-06"), 1_000_000).unwrap();
    assert_eq!(
        [free.input, free.output, free.cache_read],
        [Rate(0), Rate(0), Rate(0)]
    );
    assert_eq!(free.cache_write, Some(Rate(0)));
    let nemotron = catalog
        .system(
            &id("nemotron-3.5-super-vl"),
            &id("nvidia"),
            Effort::None,
            id("0.166.0"),
        )
        .unwrap();
    assert_eq!(
        nemotron.id().to_string(),
        "codex-0.166.0_nvidia_nemotron-3.5-super-vl_none"
    );
    assert_eq!(
        nemotron.api_model(),
        "nvidia/nemotron-3.5-super-vl-120b-a12b"
    );

    for (backend, host) in [
        ("cliproxy", "lab.tail4a3ea.ts.net"),
        ("lmstudio", "192.168.101.190"),
    ] {
        let endpoint = catalog.backend(&id(backend)).unwrap().endpoint().unwrap();
        assert_eq!(endpoint.host(), host);
    }
    let gemini = catalog.model(&id("gemini-3.8-flash")).unwrap();
    let rates = |on: &str| {
        let rates = gemini.rates(day(on), 1_000_000).unwrap();
        [rates.input, rates.output, rates.cache_read]
    };
    assert_eq!(
        rates("2026-12-31"),
        [Rate(750_000), Rate(3_750_000), Rate(75_000)]
    );
    assert_eq!(
        rates("2027-01-01"),
        [Rate(1_500_000), Rate(7_500_000), Rate(150_000)]
    );
    let gemini = catalog
        .system(
            &id("gemini-3.8-flash"),
            &id("cliproxy"),
            Effort::High,
            id("0.166.0"),
        )
        .unwrap();
    assert_eq!(
        gemini.id().to_string(),
        "codex-0.166.0_cliproxy_gemini-3.8-flash_high"
    );
    assert_eq!(gemini.api_model(), "gemini-3.8-flash-high");
    let ornith = catalog
        .system(
            &id("ornith-1.5-35b-a3b"),
            &id("lmstudio"),
            Effort::None,
            id("0.166.0"),
        )
        .unwrap();
    assert_eq!(ornith.api_model(), "ornith-1.5-35b-a3b@q8_0");
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
fn a_model_may_state_the_most_output_tokens_one_response_carries() {
    let absent = load(CATALOG).unwrap();
    assert_eq!(absent.model(&id("m")).unwrap().max_output_tokens(), None);
    let system = absent
        .system(&id("m"), &id("proxy"), Effort::High, id("1.0.0"))
        .unwrap();
    assert_eq!(system.max_output_tokens(), None);

    let stated = load(&CATALOG.replacen(
        "id = \"m\"\n",
        "id = \"m\"\nmax-output-tokens = 131072\n",
        1,
    ))
    .unwrap();
    let tokens = NonZeroU32::new(131_072);
    assert_eq!(stated.model(&id("m")).unwrap().max_output_tokens(), tokens);
    for backend in ["proxy", "anthropic"] {
        let ceiled = stated
            .system(&id("m"), &id(backend), Effort::High, id("1.0.0"))
            .unwrap();
        assert_eq!(ceiled.max_output_tokens(), tokens, "{backend}");
        let plain = absent
            .system(&id("m"), &id(backend), Effort::High, id("1.0.0"))
            .unwrap();
        assert_eq!(ceiled.id(), plain.id(), "{backend}");
    }
}

#[test]
fn an_output_ceiling_must_be_a_positive_integer() {
    for (value, needles) in [
        (
            "0",
            &["id = \"m\"", "max-output-tokens = 0", "positive"][..],
        ),
        ("-1", &["id = \"m\"", "max-output-tokens = -1", "positive"]),
        (
            "4294967296",
            &["id = \"m\"", "max-output-tokens = 4294967296", "4294967295"],
        ),
        ("1.5", &["max-output-tokens = 1.5", "floating point"]),
        ("\"lots\"", &["max-output-tokens = \"lots\"", "string"]),
    ] {
        rejects(
            "id = \"m\"\n",
            &format!("id = \"m\"\nmax-output-tokens = {value}\n"),
            needles,
        );
    }
}

#[test]
fn a_model_is_found_by_the_name_its_backend_serves_it_under() {
    let catalog = load(CATALOG).unwrap();
    let model = catalog.model(&id("m")).unwrap();
    let served = |backend: &str, name: &str| catalog.served(&id(backend), name);
    assert!(served("proxy", "m-proxy").is_some_and(|found| std::ptr::eq(found, model)));
    assert!(served("anthropic", "m-api").is_some_and(|found| std::ptr::eq(found, model)));
    assert!(served("anthropic", "m-proxy").is_none());
    assert!(served("zai", "m-api").is_none());
}

#[test]
fn rates_follow_the_price_date_and_prompt_tier() {
    let catalog = load(CATALOG).unwrap();
    let model = catalog.model(&id("m")).unwrap();
    assert_eq!(model.rates(day("2025-12-31"), 10), None);
    assert_eq!(
        model.rates(day("2026-01-01"), 1000).unwrap().input,
        Rate(1_000_000)
    );
    assert_eq!(
        model.rates(day("2026-05-31"), 1001).unwrap().input,
        Rate(3_000_000)
    );
    let june = model.rates(day("2026-06-01"), 1001).unwrap();
    assert_eq!(
        (june.input, june.cache_write),
        (Rate(500_000), Some(Rate(600_000)))
    );
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
        "harness = \"claude-code\"",
        "harness = \"gemini\"",
        &["[backends.anthropic]", "gemini"],
    );
}

#[test]
fn harness_tables_must_match_the_drivers() {
    rejects(
        "[harnesses.codex]\npackage = \"codex\"\n",
        "",
        &["driver codex", "[harnesses.codex]"],
    );
    rejects(
        "[harnesses.codex]",
        "[harnesses.gemini]",
        &["[harnesses.gemini]", "no driver", "claude-code, codex"],
    );
}

#[test]
fn systems_need_an_effort_the_harness_can_carry() {
    let offers_none = CATALOG.replacen(
        "efforts = [\"low\", \"high\"]",
        "efforts = [\"none\", \"low\", \"high\"]",
        1,
    );
    let catalog = load(&offers_none).unwrap();
    let err = catalog
        .system(&id("m"), &id("proxy"), Effort::None, id("1.0.0"))
        .unwrap_err();
    let message = err.to_string();
    for needle in ["proxy", "claude-code", "effort none"] {
        assert!(
            message.contains(needle),
            "{needle:?} missing from: {message}"
        );
    }
    assert!(
        catalog
            .system(&id("m"), &id("proxy"), Effort::Low, id("1.0.0"))
            .is_ok()
    );
}

#[test]
fn package_lists_must_be_non_empty() {
    let tables = [
        ("sandbox", "coreutils"),
        ("measure", "hyperfine"),
        ("profile", "perf"),
    ];
    for (table, package) in tables {
        rejects(
            &format!("[{table}]\npackages = [\"{package}\"]"),
            &format!("[{table}]\npackages = []"),
            &[&format!("[{table}] packages is empty")],
        );
    }
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
    rejects(
        "input = 1.0",
        "input = 0.0000001",
        &["2026-01-01", "0.0000001", "picodollar"],
    );
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
        "https://api.example.com/anthropic",
        "https://api.example.com:8443/anthropic",
        &["[backends.proxy]", "https://api.example.com:8443/anthropic"],
    );
    rejects(
        "endpoint = \"https://api.example.com/anthropic\"\n",
        "",
        &["[backends.proxy]", "api-key", "endpoint"],
    );
}

/// `CATALOG` with a Codex backend `local` that relays to `endpoint` and allows `egress`.
fn relaying(endpoint: &str, egress: &str) -> Result<Catalog> {
    load(&format!(
        "{CATALOG}\n[backends.local]\nharness = \"codex\"\nendpoint = \"{endpoint}\"\n\
         credential = {{ kind = \"api-key\", var = \"KEY\", file = \"/etc/gungnir/key\" }}\n\
         egress = [\"{egress}\"]\nrelay = true\n"
    ))
}

#[test]
fn a_relayed_endpoint_may_be_plain_http_name_an_ipv4_address_and_add_a_port() {
    for (endpoint, host) in [
        ("http://192.168.1.20:1234/v1", "192.168.1.20"),
        ("https://lab.example.net:8317/v1", "lab.example.net"),
        ("http://lab.example.net/v1", "lab.example.net"),
        ("https://api.example.org", "api.example.org"),
    ] {
        let catalog = relaying(endpoint, host).unwrap();
        let parsed = catalog.backend(&id("local")).unwrap().endpoint().unwrap();
        assert_eq!((parsed.as_str(), parsed.host()), (endpoint, host));
    }

    let err = relaying("http://192.168.1.20:1234/v1", "192.168.1.21").unwrap_err();
    let message = format!("{err:#}");
    for needle in ["[backends.local]", "has host 192.168.1.20", "egress"] {
        assert!(
            message.contains(needle),
            "{needle:?} missing from: {message}"
        );
    }

    for endpoint in [
        "ftp://lab.example.net/v1",
        "lab.example.net:8317/v1",
        "http://lab.example.net:0/v1",
        "http://lab.example.net:65536/v1",
        "http://lab.example.net:/v1",
        "http://lab.example.net:https/v1",
        "http://[::1]:1234/v1",
        "http://1.2.3/v1",
        "http://192.168.001.20/v1",
        "http://Lab.example.net/v1",
        "http://key@lab.example.net/v1",
        "http://lab.example.net/v1?key=1",
        "http://lab.example.net/v1#top",
    ] {
        let err = relaying(endpoint, "lab.example.net").unwrap_err();
        let message = format!("{err:#}");
        for needle in ["[backends.local]", endpoint] {
            assert!(
                message.contains(needle),
                "{needle:?} missing from: {message}"
            );
        }
    }
}

#[test]
fn a_quota_source_sends_the_api_key_only_to_its_endpoint_host() {
    rejects(
        "https://api.example.com/monitor/quota",
        "https://monitor.example.com/quota",
        &["[backends.proxy]", "monitor.example.com", "api.example.com"],
    );
    rejects(
        "https://api.example.com/monitor/quota",
        "http://api.example.com/monitor/quota",
        &[
            "[backends.proxy]",
            "quota url",
            "http://api.example.com/monitor/quota",
        ],
    );
    rejects(
        "egress = [\"api.anthropic.com\"]",
        "egress = [\"api.anthropic.com\"]\nquota = { kind = \"zai-monitor\", url = \"https://api.anthropic.com/quota\" }",
        &["[backends.anthropic]", "api-key"],
    );
    rejects("kind = \"zai-monitor\"", "kind = \"gauge\"", &["gauge"]);
    rejects(
        "url = \"https://api.example.com/monitor/quota\"",
        "uri = \"https://api.example.com/monitor/quota\"",
        &["uri"],
    );
}

#[test]
fn a_backend_relays_only_an_api_key_through_a_harness_that_can() {
    rejects(
        "egress = [\"api.anthropic.com\"]",
        "egress = [\"api.anthropic.com\"]\nrelay = true",
        &["[backends.anthropic]", "relay", "api-key"],
    );
    rejects(
        "egress = [\"api.example.com\"]",
        "egress = [\"api.example.com\"]\nrelay = true",
        &["[backends.proxy]", "relay", "claude-code"],
    );
    rejects(
        "egress = [\"api.example.com\"]",
        "egress = [\"api.example.com\"]\nrelay = \"yes\"",
        &["relay"],
    );
    let catalog = load(&CATALOG.replace(
        "egress = [\"api.example.com\"]",
        "egress = [\"api.example.com\"]\nrelay = false",
    ))
    .unwrap();
    assert!(!catalog.backend(&id("proxy")).unwrap().relays());
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
