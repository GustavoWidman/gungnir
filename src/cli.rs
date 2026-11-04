//! The command line.
use crate::{
    experiment::Experiment,
    grade::{self, Judgement, Outcome, Verdict},
    id::{LanguageId, ScenarioId},
    language::Language,
    sandbox::{Backend, Bubblewrap},
    scenario::Scenario,
    stop::{Stage, Stop},
    toolchains::Toolchains,
    workload::{self, Ensured},
};
use clap::{Parser, Subcommand};
use eyre::{Result, WrapErr, bail, ensure, eyre};
use futures_util::{StreamExt, stream};
use std::{collections::BTreeMap, fmt, num::NonZeroUsize, path::PathBuf, time::Duration};

#[derive(Parser)]
#[command(
    version,
    about = "Benchmark coding agents across programming languages and reasoning effort"
)]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate an experiment and print the trial matrix it expands to.
    Plan {
        /// The experiment file, such as experiments/pilot.toml.
        experiment: PathBuf,
    },
    /// Build and grade every reference solution the way trials are graded, after making sure
    /// every workload has its pinned digest. Fails if any reference does not pass.
    Check {
        /// Check only this scenario. Repeat for several.
        #[arg(long = "scenario", value_name = "ID")]
        scenarios: Vec<ScenarioId>,
        /// Check only this language. Repeat for several.
        #[arg(long = "language", value_name = "ID")]
        languages: Vec<LanguageId>,
        /// How many references to build and grade at once.
        #[arg(long, value_name = "N", default_value_t = default_jobs())]
        jobs: NonZeroUsize,
    },
}

impl Cli {
    pub fn run(self) -> Result<()> {
        match self.command {
            Command::Plan { experiment } => {
                print!("{}", Plan(&Experiment::load(&experiment)?));
                Ok(())
            }
            Command::Check {
                scenarios,
                languages,
                jobs,
            } => run_check(&scenarios, &languages, jobs),
        }
    }
}

/// A quarter of the CPUs, since one reference's build may use them all.
fn default_jobs() -> NonZeroUsize {
    std::thread::available_parallelism()
        .ok()
        .and_then(|cpus| NonZeroUsize::new(cpus.get() / 4))
        .unwrap_or(NonZeroUsize::MIN)
}

/// Checks the references of the repository holding the current directory, prints the table,
/// and names every reference that did not pass.
fn run_check(scenarios: &[ScenarioId], languages: &[LanguageId], jobs: NonZeroUsize) -> Result<()> {
    let cwd = std::env::current_dir().wrap_err("reading the current directory")?;
    let root = cwd
        .ancestors()
        .find(|dir| dir.join("gungnir.toml").is_file())
        .ok_or_else(|| {
            eyre!(
                "no directory at or above {} has a gungnir.toml",
                cwd.display()
            )
        })?;
    let all_languages = Language::load_all(&root.join("languages"))?;
    let all_scenarios = Scenario::load_all(&root.join("scenarios"), &all_languages)?;
    let selected = references(&all_scenarios, &all_languages, scenarios, languages)?;
    let toolchains = Toolchains::from_env()?;
    let backend = Bubblewrap::new(toolchains.base().clone())?;

    let runtime = tokio::runtime::Runtime::new().wrap_err("starting the async runtime")?;
    let checked = runtime.block_on(async {
        let stop = Stop::on_ctrl_c()?;
        check(&selected, &toolchains, &backend, &stop, jobs).await
    })?;
    print!("{}", CheckTable(&checked));

    let mut failed = 0;
    for checked in &checked {
        let name = format!("{}/{}", checked.scenario.id(), checked.language.id());
        match (&checked.result, &checked.evidence) {
            (Ok(_), None) => continue,
            (Ok(judgement), Some(evidence)) => eprintln!(
                "{name}: {}; build log and outputs in {}",
                judgement.verdict(),
                evidence.display()
            ),
            (Err(error), _) => eprintln!("{name}: {error:#}"),
        }
        failed += 1;
    }
    ensure!(
        failed == 0,
        "{failed} of {} references failed",
        checked.len()
    );
    Ok(())
}

/// Every scenario's reference solution in every language it supports, narrowed to the
/// scenarios and languages named, when any are.
fn references<'a>(
    scenarios: &'a BTreeMap<ScenarioId, Scenario>,
    languages: &'a BTreeMap<LanguageId, Language>,
    only_scenarios: &[ScenarioId],
    only_languages: &[LanguageId],
) -> Result<Vec<(&'a Scenario, &'a Language)>> {
    for id in only_scenarios {
        ensure!(
            scenarios.contains_key(id),
            "no scenario {id} under scenarios/"
        );
    }
    for id in only_languages {
        ensure!(
            languages.contains_key(id),
            "no language {id} under languages/"
        );
    }
    let selected: Vec<_> = scenarios
        .values()
        .filter(|scenario| only_scenarios.is_empty() || only_scenarios.contains(scenario.id()))
        .flat_map(|scenario| {
            scenario
                .languages()
                .filter(|id| only_languages.is_empty() || only_languages.contains(id))
                .map(move |id| (scenario, &languages[id]))
        })
        .collect();
    if selected.is_empty() {
        bail!("no scenario named has a reference in a language named");
    }
    Ok(selected)
}

/// One reference solution, judged.
struct Checked<'a> {
    scenario: &'a Scenario,
    language: &'a Language,
    result: Result<Judgement>,
    /// Where the build log and failing outputs were kept, when the reference did not pass.
    evidence: Option<PathBuf>,
}

/// Ensures the workloads of every scenario in `selected`, then judges each reference, `jobs`
/// at a time. After the first Ctrl-C, references not yet started are not judged.
async fn check<'a>(
    selected: &[(&'a Scenario, &'a Language)],
    toolchains: &Toolchains,
    backend: &dyn Backend,
    stop: &Stop,
    jobs: NonZeroUsize,
) -> Result<Vec<Checked<'a>>> {
    let scenarios: BTreeMap<_, _> = selected
        .iter()
        .map(|(scenario, _)| (scenario.id(), *scenario))
        .collect();
    for scenario in scenarios.values() {
        if workload::ensure(scenario, toolchains, backend, stop).await? == Ensured::Regenerated {
            eprintln!("{}: regenerated the workloads", scenario.id());
        }
    }
    let judge = |scenario: &'a Scenario, language: &'a Language| async move {
        let judged = async {
            ensure!(
                stop.stage() == Stage::Running,
                "interrupted before it started"
            );
            let reference = scenario
                .reference(language.id())
                .ok_or_else(|| eyre!("no {} reference", language.id()))?;
            let evidence = tempfile::Builder::new()
                .prefix("gungnir-check-")
                .tempdir()
                .wrap_err("creating a directory for evidence")?;
            let workspace = reference.dir();
            let judgement = grade::judge(
                language,
                toolchains,
                workspace,
                scenario,
                backend,
                stop,
                evidence.path(),
            )
            .await?;
            Ok((judgement, evidence))
        };
        let (result, evidence) = match judged.await {
            Ok((judgement, _)) if judgement.verdict() == Verdict::Pass => (Ok(judgement), None),
            Ok((judgement, evidence)) => (Ok(judgement), Some(evidence.keep())),
            Err(error) => (Err(error), None),
        };
        Checked {
            scenario,
            language,
            result,
            evidence,
        }
    };
    let mut checked: Vec<Checked> = stream::iter(selected)
        .map(|&(scenario, language)| judge(scenario, language))
        .buffer_unordered(jobs.get())
        .collect()
        .await;
    checked.sort_by_key(|checked| (checked.scenario.id(), checked.language.id()));
    Ok(checked)
}

/// One row per reference: its build time, how many hidden cases passed, its verdict, and its
/// slowest case.
struct CheckTable<'a>(&'a [Checked<'a>]);

impl fmt::Display for CheckTable<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let seconds = |wall: Duration| format!("{:.2}s", wall.as_secs_f64());
        let header = [
            "scenario", "language", "build", "cases", "verdict", "slowest",
        ];
        let mut rows = vec![header.map(String::from)];
        for checked in self.0 {
            let scenario = checked.scenario.id().to_string();
            let language = checked.language.id().to_string();
            let Ok(judgement) = &checked.result else {
                let none = || "-".to_owned();
                rows.push([scenario, language, none(), none(), "error".into(), none()]);
                continue;
            };
            let cases = judgement.cases();
            let passed = cases
                .values()
                .filter(|case| case.outcome == Outcome::Pass)
                .count();
            let slowest = cases.iter().max_by_key(|(_, case)| case.wall).map_or_else(
                || "-".to_owned(),
                |(id, case)| format!("{id} {}", seconds(case.wall)),
            );
            rows.push([
                scenario,
                language,
                seconds(judgement.build().wall),
                format!("{passed}/{}", checked.scenario.hidden().len()),
                judgement.verdict().to_string(),
                slowest,
            ]);
        }
        let widths: [usize; 6] = std::array::from_fn(|column| {
            rows.iter().map(|row| row[column].len()).max().unwrap_or(0)
        });
        for row in &rows {
            let cells: Vec<String> = row
                .iter()
                .zip(widths)
                .map(|(cell, width)| format!("{cell:width$}"))
                .collect();
            writeln!(f, "{}", cells.join("  ").trim_end())?;
        }
        Ok(())
    }
}

/// An experiment's matrix as aligned text, one row per model with its efforts.
struct Plan<'a>(&'a Experiment);

impl fmt::Display for Plan<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let experiment = self.0;
        let budget = experiment.budget();
        let concurrency: Vec<String> = experiment
            .concurrency()
            .iter()
            .map(|(backend, limit)| format!("{backend} {limit}"))
            .collect();
        writeln!(f, "experiment   {}", experiment.name())?;
        writeln!(f, "results      {}", experiment.results().path().display())?;
        writeln!(
            f,
            "budget       {} wall, {} idle",
            humantime::format_duration(budget.wall()),
            humantime::format_duration(budget.idle())
        )?;
        writeln!(f, "concurrency  {}", concurrency.join(", "))?;

        let systems = experiment.systems();
        let rows: Vec<[String; 4]> = systems
            .chunk_by(|a, b| {
                (a.harness(), a.harness_version(), a.backend(), a.model())
                    == (b.harness(), b.harness_version(), b.backend(), b.model())
            })
            .map(|group| {
                let first = &group[0];
                let efforts: Vec<&str> = group
                    .iter()
                    .map(|system| system.effort().as_str())
                    .collect();
                [
                    format!("{} {}", first.harness(), first.harness_version()),
                    first.backend().to_string(),
                    first.model().to_string(),
                    efforts.join(" "),
                ]
            })
            .collect();
        let width = |column: usize| rows.iter().map(|row| row[column].len()).max().unwrap_or(0);
        let (harness_width, backend_width, model_width) = (width(0), width(1), width(2));
        writeln!(f, "systems      {}", systems.len())?;
        for [harness, backend, model, efforts] in &rows {
            writeln!(
                f,
                "  {harness:harness_width$}  {backend:backend_width$}  {model:model_width$}  {efforts}"
            )?;
        }

        let scenarios: Vec<&str> = experiment
            .scenarios()
            .iter()
            .map(|s| s.id().as_str())
            .collect();
        let languages: Vec<&str> = experiment
            .languages()
            .iter()
            .map(|l| l.id().as_str())
            .collect();
        writeln!(f, "scenarios    {}", scenarios.join(" "))?;
        writeln!(f, "languages    {}", languages.join(" "))?;
        writeln!(f, "repetitions  {}", experiment.repetitions())?;
        writeln!(f, "trials       {}", experiment.trials().len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        grade::fake::{self, Scripted, finished},
        sandbox::{Exit, Input},
        testing::{self, id, write},
    };
    use std::fs;
    use tempfile::TempDir;

    /// The fixture repository with languages `python` and `rust` and no lizard, so judging
    /// needs no host tool, and fake toolchains for both.
    struct CheckFixture {
        _root: TempDir,
        languages: BTreeMap<LanguageId, Language>,
        scenarios: BTreeMap<ScenarioId, Scenario>,
        toolchains: Toolchains,
    }

    fn check_fixture(extra_languages: &[&str]) -> CheckFixture {
        let root = TempDir::new().unwrap();
        testing::repo(root.path());
        for language in ["python", "rust"].iter().chain(extra_languages) {
            testing::language(root.path(), language);
            let toml = testing::LANGUAGE_TOML.replace("lizard = \"python\"\n", "");
            write(
                root.path(),
                &format!("languages/{language}/language.toml"),
                toml,
            );
        }
        let languages = Language::load_all(&root.path().join("languages")).unwrap();
        let scenarios = Scenario::load_all(&root.path().join("scenarios"), &languages).unwrap();
        let toolchains = fake::toolchains(&root.path().join("toolchains"), &["python", "rust"]);
        CheckFixture {
            _root: root,
            languages,
            scenarios,
            toolchains,
        }
    }

    #[tokio::test]
    async fn check_judges_each_reference_and_tables_the_results() {
        let fixture = check_fixture(&[]);
        let backend = Scripted::new(|spec, _, io| {
            let Input::File(input) = io.stdin else {
                return Ok(finished(Exit::Code(0), b"built\n"));
            };
            let rust = fixture_toolchain(&spec.closure, "rust");
            let stdout = if rust {
                b"wrong\n".to_vec()
            } else {
                fs::read(input).unwrap()
            };
            Ok(finished(Exit::Code(0), &stdout))
        });
        let selected = references(&fixture.scenarios, &fixture.languages, &[], &[]).unwrap();
        let jobs = NonZeroUsize::new(2).unwrap();
        let stop = Stop::default();
        let checked = check(&selected, &fixture.toolchains, &backend, &stop, jobs)
            .await
            .unwrap();

        assert_eq!(
            CheckTable(&checked).to_string(),
            "scenario  language  build  cases  verdict       slowest\n\
             echo      python    0.01s  2/2    pass          many 0.01s\n\
             echo      rust      0.01s  0/2    wrong-answer  many 0.01s\n"
        );
        assert!(checked[0].evidence.is_none());
        let evidence = checked[1].evidence.clone().unwrap();
        assert_eq!(
            fs::read(evidence.join("cases/edge.out")).unwrap(),
            b"wrong\n"
        );
        fs::remove_dir_all(evidence).unwrap();
    }

    fn fixture_toolchain(closure: &crate::toolchains::Closure, language: &str) -> bool {
        let env = format!("bbbb-gungnir-language-{language}");
        closure.paths().any(|path| path.ends_with(&env))
    }

    #[test]
    fn check_selects_references_by_scenario_and_language() {
        let fixture = check_fixture(&["zig"]);
        let select = |scenarios: &[&str], languages: &[&str]| {
            let scenarios: Vec<ScenarioId> = scenarios.iter().map(|s| id(s)).collect();
            let languages: Vec<LanguageId> = languages.iter().map(|l| id(l)).collect();
            references(
                &fixture.scenarios,
                &fixture.languages,
                &scenarios,
                &languages,
            )
            .map(|selected| {
                selected
                    .iter()
                    .map(|(scenario, language)| format!("{}/{}", scenario.id(), language.id()))
                    .collect::<Vec<_>>()
            })
        };
        assert_eq!(select(&[], &[]).unwrap(), ["echo/python", "echo/rust"]);
        assert_eq!(select(&["echo"], &["rust"]).unwrap(), ["echo/rust"]);
        for (scenarios, languages, needle) in [
            (&["word-freq"][..], &[][..], "word-freq"),
            (&[][..], &["cobol"][..], "cobol"),
            (&[][..], &["zig"][..], "no scenario named has a reference"),
        ] {
            let message = select(scenarios, languages).unwrap_err().to_string();
            assert!(message.contains(needle), "{message}");
        }
    }

    #[test]
    fn plan_lists_each_model_with_its_efforts_and_counts_the_trials() {
        let root = TempDir::new().unwrap();
        testing::repo(root.path());
        write(
            root.path(),
            "experiments/trial.toml",
            testing::EXPERIMENT_TOML
                .replace(
                    "[[systems]]\nmodel = \"luna\"",
                    "[[systems]]\nmodel = \"glm\"\nbackend = \"zai\"\nefforts = [\"low\"]\n\n\
                 [[systems]]\nmodel = \"luna\"",
                )
                .replace("openai = 1", "openai = 1\nzai = 3"),
        );
        let experiment = Experiment::load(&root.path().join("experiments/trial.toml")).unwrap();
        let results = experiment.results().path().display().to_string();

        assert_eq!(
            Plan(&experiment).to_string(),
            format!(
                "experiment   trial-run\n\
                 results      {results}\n\
                 budget       40m wall, 10m idle\n\
                 concurrency  anthropic 2, openai 1, zai 3\n\
                 systems      5\n  \
                 claude-code 2.1.316  anthropic  opus  low high\n  \
                 claude-code 2.1.316  zai        glm   low\n  \
                 codex 0.166.0        openai     luna  none low\n\
                 scenarios    echo\n\
                 languages    python rust\n\
                 repetitions  2\n\
                 trials       20\n"
            )
        );
    }
}
