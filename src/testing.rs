//! Fixture helpers shared by the unit tests.
use std::{fmt::Debug, fs, path::Path, str::FromStr};

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
sources = ["**/*.py"]
ignore = ["**/__pycache__/**"]
lizard = "python"

[env]
PYTHONDONTWRITEBYTECODE = "1"
"#;

/// Writes a valid profile for language `id` under `root/languages/`.
pub fn language(root: &Path, id: &str) {
    let dir = format!("languages/{id}");
    write(root, &format!("{dir}/language.toml"), LANGUAGE_TOML);
    write(
        root,
        &format!("{dir}/preamble.md"),
        "Build with `{build}`. Run with `{run}`.\n",
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
