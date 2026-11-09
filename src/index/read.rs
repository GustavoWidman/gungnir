//! The samples the index reads from the committed records of experiments: every trial whose
//! committed run is a sample and whose grade is current, as the report reads them, from the
//! generations graded completely enough at every effort of their model.
use super::{Sample, Timing};
use crate::{
    catalog::{System, SystemId},
    experiment::Experiment,
    grade::{GradeRecord, Verdict},
    id::{ModelId, Rep},
    measure::{self, MeasureRecord},
    report::{self, Reported, Trial},
    toolchains::Toolchains,
};
use eyre::{Result, ensure};
use std::collections::{BTreeMap, BTreeSet};

/// One repetition of one system in one experiment: a program for each scenario and language.
#[derive(Clone, Debug, PartialEq)]
pub struct Generation {
    pub experiment: String,
    pub system: System,
    pub rep: Rep,
    /// The scenario and language cells the experiment asks of it.
    pub cells: usize,
    /// The cells whose committed run is a sample with a current grade.
    pub graded: usize,
    /// Whether its samples count: it and the same repetition of every other system of its model
    /// in the experiment are complete, so a model's scores never lean toward the efforts that
    /// reached a repetition first.
    pub counted: bool,
}

impl Generation {
    /// Whether at least `share` of its cells are graded.
    pub fn is_complete(&self, share: f64) -> bool {
        self.graded as f64 >= share * self.cells as f64
    }
}

/// What the index read from experiments.
#[derive(Debug)]
pub struct Read {
    /// The samples of the counted generations.
    pub samples: Vec<Sample>,
    /// Every generation of every experiment, complete or not.
    pub generations: Vec<Generation>,
    /// The share of a generation's cells that must be graded for its samples to count.
    pub complete: f64,
    /// Each experiment with the sessions that wrote the records of its samples.
    pub(crate) experiments: Vec<Reported>,
}

/// Reads the samples of `experiments`, keeping the generations of which at least `complete` of
/// the cells are graded, alongside the same repetition of every other system of their model.
pub fn read(experiments: &[Experiment], toolchains: &Toolchains, complete: f64) -> Result<Read> {
    let mut names = BTreeSet::new();
    for experiment in experiments {
        ensure!(
            names.insert(experiment.name()),
            "experiment {} is given twice",
            experiment.name()
        );
    }
    let mut read = Read {
        samples: Vec::new(),
        generations: Vec::new(),
        complete,
        experiments: Vec::new(),
    };
    for experiment in experiments {
        let mut generations: BTreeMap<(SystemId, Rep), Generation> = BTreeMap::new();
        for key in experiment.trials() {
            let generation = generations
                .entry((key.system.id(), key.rep))
                .or_insert_with(|| Generation {
                    experiment: experiment.name().to_owned(),
                    system: key.system.clone(),
                    rep: key.rep,
                    cells: 0,
                    graded: 0,
                    counted: false,
                });
            generation.cells += 1;
        }
        let mut graded = Vec::new();
        for trial in report::read(experiment, toolchains)? {
            let Some(grade) = trial.current_grade() else {
                continue;
            };
            let sample = sample(&trial, grade);
            let generation = (trial.key.system.id(), trial.key.rep);
            if let Some(generation) = generations.get_mut(&generation) {
                generation.graded += 1;
            }
            graded.push((generation, sample, trial));
        }
        count_whole_repetitions(&mut generations, complete);
        let mut sessions = BTreeSet::new();
        for (generation, sample, trial) in graded {
            let generation = generations.get(&generation);
            if generation.is_some_and(|generation| generation.counted) {
                read.samples.push(sample);
                sessions.extend(trial.sessions().cloned());
            }
        }
        read.generations.extend(generations.into_values());
        read.experiments.push(Reported {
            name: experiment.name().to_owned(),
            results: experiment.results().path().to_owned(),
            sessions,
        });
    }
    Ok(read)
}

/// Marks counted each complete generation whose repetition is complete for every system of its
/// model in the experiment.
fn count_whole_repetitions(generations: &mut BTreeMap<(SystemId, Rep), Generation>, share: f64) {
    let mut systems: BTreeMap<ModelId, BTreeSet<SystemId>> = BTreeMap::new();
    let mut finished: BTreeMap<(ModelId, Rep), usize> = BTreeMap::new();
    for ((system, rep), generation) in generations.iter() {
        let model = generation.system.model();
        systems
            .entry(model.clone())
            .or_default()
            .insert(system.clone());
        if generation.is_complete(share) {
            *finished.entry((model.clone(), *rep)).or_default() += 1;
        }
    }
    for ((_, rep), generation) in generations.iter_mut() {
        let model = generation.system.model();
        let whole = systems.get(model).map_or(0, BTreeSet::len);
        generation.counted = finished.get(&(model.clone(), *rep)) == Some(&whole);
    }
}

/// What a graded trial counts in the index.
fn sample(trial: &Trial, grade: &GradeRecord) -> Sample {
    let judgement = grade.judgement();
    let ccn = judgement.complexity().map(|complexity| {
        let functions = complexity.files.values().flat_map(|file| &file.functions);
        functions.map(|function| u64::from(function.ccn)).sum()
    });
    Sample {
        model: trial.key.system.model().clone(),
        scenario: trial.key.scenario.clone(),
        language: trial.key.language.clone(),
        passed: judgement.verdict() == Verdict::Pass,
        tokens: trial.run.outcome().total().count(),
        wall_s: trial.run.process().wall.as_secs_f64(),
        ccn,
        timings: trial.current_measure().map(timings).unwrap_or_default(),
    }
}

/// The medians of every workload `measure` timed.
fn timings(measure: &MeasureRecord) -> Vec<Timing> {
    let workloads = measure.workloads().iter();
    workloads
        .filter_map(|(workload, outcome)| match outcome {
            measure::Outcome::Timed(timed) => {
                let summary = timed.summary();
                Some(Timing {
                    workload: workload.clone(),
                    candidate_median_s: summary.candidate.median,
                    reference_median_s: summary.reference.median,
                })
            }
            measure::Outcome::CandidateFailed(_) => None,
        })
        .collect()
}
