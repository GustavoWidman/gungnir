//! The command line.
use crate::experiment::Experiment;
use clap::{Parser, Subcommand};
use eyre::Result;
use std::{fmt, path::PathBuf};

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
}

impl Cli {
    pub fn run(self) -> Result<()> {
        match self.command {
            Command::Plan { experiment } => {
                print!("{}", Plan(&Experiment::load(&experiment)?));
                Ok(())
            }
        }
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
    use crate::testing::{self, write};
    use tempfile::TempDir;

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
