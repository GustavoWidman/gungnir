//! Toolchain profiles, one directory per language under `languages/`.
use crate::{
    id::LanguageId,
    scenario::Brief,
    store::{read_toml, subdirs},
};
use eyre::{Result, WrapErr, bail, ensure};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fmt, fs, mem,
    path::{Path, PathBuf},
};

/// How to build and run a program in one language, which files are its sources, and the
/// prompt preamble that tells the agent so.
#[derive(Debug)]
pub struct Language {
    id: LanguageId,
    build: Vec<Argv>,
    run: Argv,
    version: Argv,
    sources: GlobSet,
    ignore: GlobSet,
    lizard: Option<String>,
    env: BTreeMap<String, String>,
    preamble: Vec<Piece>,
    template: PathBuf,
}

impl Language {
    /// Loads every directory under `dir` as a language named by the directory.
    pub fn load_all(dir: &Path) -> Result<BTreeMap<LanguageId, Self>> {
        subdirs::<LanguageId>(dir)?
            .into_iter()
            .map(|(id, path)| {
                let language = Self::load(id.clone(), &path)
                    .wrap_err_with(|| format!("language {}", path.display()))?;
                Ok((id, language))
            })
            .collect()
    }

    fn load(id: LanguageId, dir: &Path) -> Result<Self> {
        let manifest = dir.join("language.toml");
        let raw: RawLanguage = read_toml(&manifest)?;
        let preamble_path = dir.join("preamble.md");
        let preamble = fs::read_to_string(&preamble_path)
            .wrap_err_with(|| format!("reading {}", preamble_path.display()))
            .and_then(|text| {
                parse_preamble(&text).wrap_err_with(|| format!("{}", preamble_path.display()))
            })?;
        let template = dir.join("template");
        ensure!(
            template.is_dir(),
            "{} is not a directory",
            template.display()
        );
        raw.resolve(id, preamble, template)
            .wrap_err_with(|| format!("invalid {}", manifest.display()))
    }

    pub fn id(&self) -> &LanguageId {
        &self.id
    }

    /// The build steps, run in order in the workspace. Never empty.
    pub fn build(&self) -> &[Argv] {
        &self.build
    }

    /// The command that runs the built program, reading a case on stdin.
    pub fn run(&self) -> &Argv {
        &self.run
    }

    /// The command that prints the toolchain's version as the preamble's `{version}` states it.
    pub fn version(&self) -> &Argv {
        &self.version
    }

    /// Workspace-relative paths that hold source code.
    pub fn sources(&self) -> &GlobSet {
        &self.sources
    }

    /// Workspace-relative paths that hold build output, never part of a digest.
    pub fn ignore(&self) -> &GlobSet {
        &self.ignore
    }

    /// The language name lizard knows, when lizard can measure the language.
    pub fn lizard(&self) -> Option<&str> {
        self.lizard.as_deref()
    }

    /// Environment variables set for every build and run.
    pub fn env(&self) -> &BTreeMap<String, String> {
        &self.env
    }

    /// The files scaffolded into every workspace.
    pub fn template(&self) -> &Path {
        &self.template
    }

    /// The agent's prompt: the preamble with `{build}`, `{run}` and the toolchain's `version`
    /// filled in, a blank line, and the scenario's spec.
    pub fn prompt(&self, brief: &Brief, version: &Version) -> String {
        let build = self
            .build
            .iter()
            .map(Argv::to_string)
            .collect::<Vec<_>>()
            .join(" && ");
        let run = self.run.to_string();
        let mut prompt = String::new();
        for piece in &self.preamble {
            prompt.push_str(match piece {
                Piece::Text(text) => text,
                Piece::Build => &build,
                Piece::Run => &run,
                Piece::Version => version.as_str(),
            });
        }
        prompt.truncate(prompt.trim_end().len());
        prompt.push_str("\n\n");
        prompt.push_str(brief.spec());
        prompt
    }
}

/// A program and its arguments. The program is never empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Argv(Vec<String>);

impl Argv {
    fn parse(key: &str, args: Vec<String>) -> Result<Self> {
        ensure!(
            args.first().is_some_and(|program| !program.is_empty()),
            "{key} = {args:?} names no program"
        );
        Ok(Self(args))
    }

    pub fn args(&self) -> &[String] {
        &self.0
    }
}

/// The arguments joined by spaces, as the prompt shows them.
impl fmt::Display for Argv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.join(" "))
    }
}

/// What a language's toolchain printed for its version command: one line, which the prompt
/// states wherever the preamble says `{version}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version(String);

impl Version {
    /// Parses what the version command printed, around which whitespace does not count.
    pub fn parse(printed: &str) -> Result<Self> {
        let line = printed.trim();
        ensure!(
            !line.is_empty() && !line.contains(char::is_control),
            "printed {printed:?}, which is not one line naming a version"
        );
        Ok(Self(line.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A run of preamble text, or a placeholder filled when rendering.
#[derive(Debug, PartialEq)]
enum Piece {
    Text(String),
    Build,
    Run,
    Version,
}

/// Splits a preamble into text and placeholders. A placeholder is `{name}` with `name` made
/// of lowercase letters and underscores, and only `{build}`, `{run}` and `{version}` exist.
/// Any other brace, such as `{}` or `{ x }` in a code sample, is plain text. The preamble must
/// say `{version}`, so the version it states always comes from the toolchain.
fn parse_preamble(mut text: &str) -> Result<Vec<Piece>> {
    let mut pieces = Vec::new();
    let mut literal = String::new();
    while let Some(open) = text.find('{') {
        literal.push_str(&text[..open]);
        let rest = &text[open + 1..];
        let name_len = rest
            .find(|c: char| !(c.is_ascii_lowercase() || c == '_'))
            .unwrap_or(rest.len());
        let (name, after) = rest.split_at(name_len);
        match after.strip_prefix('}') {
            Some(after) if !name.is_empty() => {
                let piece = match name {
                    "build" => Piece::Build,
                    "run" => Piece::Run,
                    "version" => Piece::Version,
                    other => bail!(
                        "unknown placeholder {{{other}}}; only {{build}}, {{run}} and {{version}} \
                         exist"
                    ),
                };
                pieces.push(Piece::Text(mem::take(&mut literal)));
                pieces.push(piece);
                text = after;
            }
            _ => {
                literal.push('{');
                text = rest;
            }
        }
    }
    literal.push_str(text);
    pieces.push(Piece::Text(literal));
    ensure!(
        pieces.contains(&Piece::Version),
        "the preamble never says {{version}}; state the language's version through it, so the \
         prompt names the version the toolchain reports"
    );
    Ok(pieces)
}

/// Globs where `*` stops at `/`, as in `.gitignore`.
fn globs(key: &str, patterns: &[String]) -> Result<GlobSet> {
    let mut set = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = GlobBuilder::new(pattern)
            .literal_separator(true)
            .build()
            .wrap_err_with(|| format!("{key} glob {pattern:?}"))?;
        set.add(glob);
    }
    set.build().wrap_err_with(|| format!("{key} globs"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLanguage {
    packages: Vec<String>,
    build: Vec<Vec<String>>,
    run: Vec<String>,
    version: Vec<String>,
    sources: Vec<String>,
    #[serde(default)]
    ignore: Vec<String>,
    lizard: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

impl RawLanguage {
    fn resolve(self, id: LanguageId, preamble: Vec<Piece>, template: PathBuf) -> Result<Language> {
        // nix/toolchains.nix builds from the package names; Rust only checks they are there.
        ensure!(!self.packages.is_empty(), "packages is empty");
        ensure!(!self.build.is_empty(), "build is empty");
        let build = self
            .build
            .into_iter()
            .map(|args| Argv::parse("build step", args))
            .collect::<Result<_>>()?;
        ensure!(!self.sources.is_empty(), "sources is empty");
        if let Some(lizard) = &self.lizard {
            ensure!(!lizard.is_empty(), "lizard is empty");
        }
        Ok(Language {
            id,
            build,
            run: Argv::parse("run", self.run)?,
            version: Argv::parse("version", self.version)?,
            sources: globs("sources", &self.sources)?,
            ignore: globs("ignore", &self.ignore)?,
            lizard: self.lizard,
            env: self.env,
            preamble,
            template,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        scenario::Scenario,
        testing::{self, id, write},
    };
    use tempfile::TempDir;

    fn load(root: &Path) -> Result<BTreeMap<LanguageId, Language>> {
        Language::load_all(&root.join("languages"))
    }

    fn rejects(edit: impl FnOnce(&Path), needles: &[&str]) {
        let root = TempDir::new().unwrap();
        testing::language(root.path(), "python");
        edit(root.path());
        let message = format!("{:#}", load(root.path()).unwrap_err());
        for needle in needles {
            assert!(
                message.contains(needle),
                "{needle:?} missing from: {message}"
            );
        }
    }

    #[test]
    fn loads_a_profile_named_by_its_directory() {
        let root = TempDir::new().unwrap();
        testing::language(root.path(), "python");
        testing::language(root.path(), "python-pypy");
        write(root.path(), "languages/README.md", "not a language");

        let languages = load(root.path()).unwrap();
        let ids: Vec<&str> = languages.keys().map(LanguageId::as_str).collect();
        assert_eq!(ids, ["python", "python-pypy"]);

        let python = &languages[&id::<LanguageId>("python")];
        let build: Vec<String> = python.build().iter().map(Argv::to_string).collect();
        assert_eq!(build, ["python3 -m compileall -q .", "true"]);
        assert_eq!(python.run().args(), ["python3", "main.py"]);
        assert_eq!(python.lizard(), Some("python"));
        assert_eq!(python.env()["PYTHONDONTWRITEBYTECODE"], "1");
        assert!(python.template().join("main.py").is_file());
    }

    #[test]
    fn globs_do_not_cross_directories_with_a_single_star() {
        let root = TempDir::new().unwrap();
        testing::language(root.path(), "python");
        write(
            root.path(),
            "languages/python/language.toml",
            testing::LANGUAGE_TOML.replace("**/*.py", "*.py"),
        );
        let python = &load(root.path()).unwrap()[&id::<LanguageId>("python")];
        assert!(python.sources().is_match("main.py"));
        assert!(!python.sources().is_match("pkg/util.py"));
        assert!(python.ignore().is_match("pkg/__pycache__/util.pyc"));
    }

    #[test]
    fn prompt_fills_the_preamble_then_appends_the_spec() {
        let root = TempDir::new().unwrap();
        testing::language(root.path(), "python");
        write(
            root.path(),
            "languages/python/preamble.md",
            "Python {version}\nBuild: {build}\nRun: {run}\nKeep {} and { x } and {build\n\n",
        );
        testing::scenario(root.path(), "echo", &["python"]);
        let languages = load(root.path()).unwrap();
        let scenarios = Scenario::load_all(&root.path().join("scenarios"), &languages).unwrap();
        let python = &languages[&id::<LanguageId>("python")];
        let version = Version::parse("3.15\n").unwrap();

        let prompt = python.prompt(scenarios[&id("echo")].brief(), &version);
        assert_eq!(
            prompt,
            "Python 3.15\nBuild: python3 -m compileall -q . && true\nRun: python3 main.py\n\
             Keep {} and { x } and {build\n\n\
             Echo each line of stdin.\n"
        );
        assert_eq!(python.version().args(), ["python3", "--version"]);
    }

    #[test]
    fn unknown_placeholders_fail_at_load() {
        rejects(
            |root| write(root, "languages/python/preamble.md", "Install {packages}."),
            &["preamble.md", "{packages}"],
        );
    }

    #[test]
    fn a_preamble_states_its_version_only_through_the_placeholder() {
        rejects(
            |root| {
                write(
                    root,
                    "languages/python/preamble.md",
                    "Python 3.14 with `{run}`.",
                )
            },
            &["preamble.md", "never says {version}"],
        );
        rejects(
            |root| {
                let toml =
                    testing::LANGUAGE_TOML.replace("version = [\"python3\", \"--version\"]\n", "");
                write(root, "languages/python/language.toml", toml);
            },
            &["language.toml", "version"],
        );
    }

    #[test]
    fn a_version_is_one_line_without_its_surrounding_whitespace() {
        assert_eq!(
            Version::parse(" 1.101-nightly\n").unwrap().as_str(),
            "1.101-nightly"
        );
        for printed in ["", "\n", "3.14\n3.15\n"] {
            assert!(Version::parse(printed).is_err(), "{printed:?} accepted");
        }
    }

    #[test]
    fn build_needs_at_least_one_step_and_every_step_a_program() {
        rejects(
            |root| {
                let toml = testing::LANGUAGE_TOML.replace(
                    "build = [[\"python3\", \"-m\", \"compileall\", \"-q\", \".\"], [\"true\"]]",
                    "build = []",
                );
                write(root, "languages/python/language.toml", toml);
            },
            &["language.toml", "build is empty"],
        );
        rejects(
            |root| {
                let toml = testing::LANGUAGE_TOML.replace("[\"true\"]", "[]");
                write(root, "languages/python/language.toml", toml);
            },
            &["language.toml", "build step = []"],
        );
    }

    #[test]
    fn preamble_and_template_are_required() {
        rejects(
            |root| fs::remove_file(root.join("languages/python/preamble.md")).unwrap(),
            &["preamble.md"],
        );
        rejects(
            |root| fs::remove_dir_all(root.join("languages/python/template")).unwrap(),
            &["template"],
        );
    }

    #[test]
    fn directory_names_must_be_language_ids() {
        rejects(
            |root| {
                fs::rename(root.join("languages/python"), root.join("languages/Python")).unwrap()
            },
            &["languages/Python", "\"Python\""],
        );
    }
}
