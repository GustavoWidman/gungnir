//! The report: the committed records of one or more experiments folded into tidy CSV tables
//! for analysis in Python or R, with a manifest naming where every number came from.
mod csv;
mod price;
mod tables;
#[cfg(test)]
mod tests;

pub use price::{Cost, PriceList, Usd};

use crate::{
    experiment::Experiment,
    lifecycle::{Current, Lifecycle, Stage},
    provenance::{Session, SessionId},
    run::Reason,
    store::replace_dir,
    toolchains::Toolchains,
};
use chrono::{DateTime, NaiveDate, Utc};
use eyre::{Result, WrapErr, ensure};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use tables::{Tables, Trial};

/// `manifest.json`: what a report was made from and how.
#[derive(Debug, Serialize)]
pub struct Manifest {
    written: DateTime<Utc>,
    /// The invocation that wrote the report: the gungnir version and git revision among it.
    session: Session,
    /// The date whose list prices every cost uses.
    prices: NaiveDate,
    experiments: Vec<Reported>,
    /// Each table's row count, by file name.
    tables: BTreeMap<&'static str, usize>,
}

impl Manifest {
    pub fn tables(&self) -> &BTreeMap<&'static str, usize> {
        &self.tables
    }
}

/// One experiment in a report.
#[derive(Debug, Serialize)]
struct Reported {
    name: String,
    results: PathBuf,
    /// Every session that wrote a record the report read.
    sessions: BTreeSet<SessionId>,
}

/// Reads every trial of `experiments`, prices their tokens at the list prices of `prices`,
/// and replaces `out` with the tables and their manifest. Refuses to replace a directory that
/// is neither empty nor a report.
pub fn report(
    experiments: &[Experiment],
    toolchains: &Toolchains,
    prices: NaiveDate,
    session: Session,
    out: &Path,
) -> Result<Manifest> {
    let mut names = BTreeSet::new();
    for experiment in experiments {
        ensure!(
            names.insert(experiment.name()),
            "experiment {} is given twice",
            experiment.name()
        );
    }
    let mut tables = Tables::default();
    let mut reported = Vec::new();
    for experiment in experiments {
        let price_list = PriceList::new(experiment.catalog(), prices);
        let mut sessions = BTreeSet::new();
        for trial in read(experiment, toolchains)? {
            tables
                .add(experiment.name(), &price_list, &trial)
                .wrap_err_with(|| format!("reporting {} {}", experiment.name(), trial.key))?;
            sessions.extend(trial.sessions().cloned());
        }
        reported.push(Reported {
            name: experiment.name().to_owned(),
            results: experiment.results().path().to_owned(),
            sessions,
        });
    }
    let files = tables.files();
    let manifest = Manifest {
        written: Utc::now(),
        session,
        prices,
        experiments: reported,
        tables: files.iter().map(|(name, rows, _)| (*name, *rows)).collect(),
    };
    let texts: Vec<(&str, &str)> = files
        .iter()
        .map(|(name, _, text)| (*name, text.as_str()))
        .collect();
    replace_dir(out, "report", &texts, &manifest)?;
    Ok(manifest)
}

/// Every trial of `experiment` whose committed run is a sample, as its records stand now. A
/// committed run that is no sample, which an older parser or policy committed, is left out
/// until `run` withdraws it.
fn read(experiment: &Experiment, toolchains: &Toolchains) -> Result<Vec<Trial>> {
    let mut trials = Vec::new();
    for key in experiment.trials() {
        let dir = experiment.results().trial(&key);
        let current = Current::of(experiment, &key, toolchains)?;
        let lifecycle = Lifecycle::read(&dir, &current)?;
        if let Some(run) = lifecycle.run()
            && Reason::of(&run.outcome().exit, run.process()).is_some()
        {
            continue;
        }
        let stale = lifecycle.stale;
        let trial = match lifecycle.stage {
            Stage::Pending => continue,
            Stage::Ran { run } => Trial {
                key,
                run: *run,
                grade: None,
                measure: None,
                stale,
            },
            Stage::Graded { run, grade } => Trial {
                key,
                run: *run,
                grade: Some(*grade),
                measure: None,
                stale,
            },
            Stage::Measured {
                run,
                grade,
                measure,
            } => Trial {
                key,
                run: *run,
                grade: Some(*grade),
                measure: Some(*measure),
                stale,
            },
        };
        trials.push(trial);
    }
    Ok(trials)
}
