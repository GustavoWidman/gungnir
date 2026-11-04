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
    store::{write_atomic, write_json},
    toolchains::Toolchains,
};
use chrono::{DateTime, NaiveDate, Utc};
use eyre::{Result, WrapErr, ensure, eyre};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};
use tables::{Tables, Trial};

/// Written last, so a directory holding it is a whole report.
const MANIFEST: &str = "manifest.json";

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
    publish(out, &files, &manifest)?;
    Ok(manifest)
}

/// Every trial of `experiment` whose committed run is a sample, as its records stand now. A
/// committed run that is no sample, made before the policy or by an older gungnir, is left out
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

/// Builds the report in `<out>.partial`, its manifest last, then swaps it in for `out`. A crash
/// leaves the old report or the new one at `out`, or neither, with the leftovers beside it
/// swept by the next report.
fn publish(out: &Path, files: &[(&'static str, usize, String)], manifest: &Manifest) -> Result<()> {
    let name = out
        .file_name()
        .ok_or_else(|| eyre!("{} does not name a directory", out.display()))?;
    let beside = |suffix: &str| {
        let mut sibling = name.to_owned();
        sibling.push(suffix);
        out.with_file_name(sibling)
    };
    let (partial, old) = (beside(".partial"), beside(".old"));
    ensure!(
        is_report_or_empty(out)?,
        "{} is not a report; refusing to replace it",
        out.display()
    );
    remove(&partial)?;
    remove(&old)?;
    for (file, _, text) in files {
        write_atomic(&partial.join(file), text.as_bytes())?;
    }
    write_json(&partial.join(MANIFEST), manifest)?;
    match fs::rename(out, &old) {
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        renamed => renamed.wrap_err_with(|| format!("moving {} aside", out.display()))?,
    }
    fs::rename(&partial, out)
        .wrap_err_with(|| format!("renaming {} to {}", partial.display(), out.display()))?;
    remove(&old)
}

/// Whether `dir` is missing, empty or a whole report.
fn is_report_or_empty(dir: &Path) -> Result<bool> {
    match fs::read_dir(dir) {
        Ok(mut entries) => Ok(entries.next().is_none() || dir.join(MANIFEST).is_file()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(true),
        Err(err) => Err(err).wrap_err_with(|| format!("listing {}", dir.display())),
    }
}

fn remove(dir: &Path) -> Result<()> {
    match fs::remove_dir_all(dir) {
        Err(err) if err.kind() != ErrorKind::NotFound => {
            Err(err).wrap_err_with(|| format!("removing {}", dir.display()))
        }
        _ => Ok(()),
    }
}
