//! Fixture helpers shared by the unit tests.
use crate::catalog::Catalog;
use std::{fmt::Debug, fs, path::Path, str::FromStr};
use tempfile::TempDir;

/// Two harnesses and three backends. `glm` is served by two backends, so an experiment
/// must choose one.
pub const CATALOG_TOML: &str = r#"
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
credential = { kind = "env-token", var = "TOKEN", file = "~/token" }
egress = ["api.anthropic.com"]

[backends.openai]
harness = "codex"
credential = { kind = "codex-home", dir = "~/codex" }
egress = ["api.openai.com"]

[backends.zai]
harness = "claude-code"
endpoint = "https://api.z.ai/api/anthropic"
credential = { kind = "api-key", var = "KEY", file = "~/key" }
egress = ["api.z.ai"]

[[models]]
id = "opus"
efforts = ["low", "high", "max"]
served-by = { anthropic = "claude-opus" }
prices = [{ since = "2026-01-01", input = 4.0, output = 20.0, cache-read = 0.2 }]

[[models]]
id = "luna"
efforts = ["none", "low"]
served-by = { openai = "gpt-luna" }
prices = [{ since = "2026-01-01", input = 0.1, output = 0.5, cache-read = 0.01 }]

[[models]]
id = "glm"
efforts = ["low", "high"]
served-by = { zai = "glm", anthropic = "glm-relay" }
prices = [{ since = "2026-01-01", input = 1.0, output = 4.0, cache-read = 0.2 }]
"#;

/// Four systems over two backends, on one scenario in two languages, twice: 16 trials.
pub const EXPERIMENT_TOML: &str = r#"
name = "trial-run"
results = "../runs/trial-run"
repetitions = 2
scenarios = ["echo"]
languages = ["python", "rust"]

[pins]
claude-code = "2.1.316"
codex = "0.166.0"

[budget]
wall = "40m"
idle = "10m"

[concurrency]
anthropic = 2
openai = 1

[[systems]]
model = "opus"
efforts = ["high", "low"]

[[systems]]
model = "luna"
efforts = "all"
"#;

/// Writes a repository with the fixture catalog, languages `python` and `rust`, scenario
/// `echo` with references for both, and `experiments/trial.toml`.
pub fn repo(root: &Path) {
    write(root, "gungnir.toml", CATALOG_TOML);
    language(root, "python");
    language(root, "rust");
    scenario(root, "echo", &["python", "rust"]);
    write(root, "experiments/trial.toml", EXPERIMENT_TOML);
}

/// The catalog in [`CATALOG_TOML`].
pub fn catalog() -> Catalog {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "gungnir.toml", CATALOG_TOML);
    Catalog::load(&dir.path().join("gungnir.toml")).unwrap()
}

/// Parses an identifier the test knows to be valid.
pub fn id<T: FromStr<Err: Debug>>(value: &str) -> T {
    value.parse().unwrap()
}

/// Writes `contents` to `root/rel`, creating parent directories.
pub fn write(root: &Path, rel: &str, contents: impl AsRef<[u8]>) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

pub const LANGUAGE_TOML: &str = r#"
packages = ["python3"]
build = [["python3", "-m", "compileall", "-q", "."], ["true"]]
run = ["python3", "main.py"]
version = ["python3", "--version"]
sources = ["**/*.py"]
ignore = ["**/__pycache__/**"]
lizard = "python"

[env]
PYTHONDONTWRITEBYTECODE = "1"

[profile]
call-graph = "fp"
build-env = { PYTHONOPTIMIZE = "1" }
env = { PYTHONPERFSUPPORT = "1" }
shows = "Python functions."
"#;

/// Writes a valid profile for language `id` under `root/languages/`.
pub fn language(root: &Path, id: &str) {
    let dir = format!("languages/{id}");
    write(root, &format!("{dir}/language.toml"), LANGUAGE_TOML);
    write(
        root,
        &format!("{dir}/preamble.md"),
        "Use version {version}. Build with `{build}`. Run with `{run}`.\n",
    );
    write(root, &format!("{dir}/template/main.py"), "");
}

pub const SCENARIO_TOML: &str = r#"
title = "Echo"
compare = "whitespace"

[limits]
time = "10s"
memory = "1GiB"
"#;

/// Writes a valid scenario `id` under `root/scenarios/`, with a reference for each of
/// `languages`.
pub fn scenario(root: &Path, id: &str, languages: &[&str]) {
    let dir = format!("scenarios/{id}");
    let files = [
        ("scenario.toml", SCENARIO_TOML),
        ("spec.md", "Echo each line of stdin.\n"),
        ("examples/basic.in", "a\n"),
        ("examples/basic.out", "a\n"),
        ("hidden/edge.in", "\n"),
        ("hidden/edge.out", "\n"),
        ("hidden/many.in", "a\nb\n"),
        ("hidden/many.out", "a\nb\n"),
        ("workloads/large.in", "x\n"),
        (
            "workloads.sha256",
            "73cb3858a687a8494ca3323053016282f3dad39d42cf62ca4e79dda2aac7d9ac  large.in\n",
        ),
    ];
    for (path, contents) in files {
        write(root, &format!("{dir}/{path}"), contents);
    }
    for language in languages {
        write(
            root,
            &format!("{dir}/reference/{language}/main.py"),
            "print(input())\n",
        );
    }
}
