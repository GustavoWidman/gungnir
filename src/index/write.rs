//! The index as CSV tables for scripts, beside a manifest naming what it was made from.
use super::{Bootstrap, Dimension, Generation, Read, Scores, Weights};
use crate::{
    provenance::Session,
    report::{
        Reported,
        csv::{encode, row},
    },
    store::replace_dir,
};
use chrono::{DateTime, Utc};
use eyre::Result;
use serde::Serialize;
use std::{collections::BTreeMap, path::Path};

/// The scope of scores across models.
const ALL: &str = "all";

row! {
    /// One score of one language in one scope.
    pub struct ScoreRow {
        /// `all` for the scores across models, or the model's id.
        pub scope: String,
        pub language: String,
        /// A dimension, or `index`.
        pub dimension: &'static str,
        /// Empty where the scope has no value, such as speed for a model never measured.
        pub score: Option<f64>,
        /// The 95% bootstrap interval.
        pub low: Option<f64>,
        pub high: Option<f64>,
    }
}

row! {
    /// One generation and whether its samples count.
    pub struct GenerationRow {
        pub experiment: String,
        pub system: String,
        pub model: String,
        pub effort: &'static str,
        pub rep: u32,
        pub cells: usize,
        pub graded: usize,
        pub complete: bool,
        pub counted: bool,
    }
}

/// `manifest.json`: what an index was made from and how.
#[derive(Debug, Serialize)]
struct Manifest<'a> {
    written: DateTime<Utc>,
    /// The invocation that wrote the index: the gungnir version and git revision among it.
    session: Session,
    experiments: &'a [Reported],
    /// The share of a generation's cells that must be graded for its samples to count.
    complete: f64,
    samples: usize,
    weights: BTreeMap<&'static str, f64>,
    bootstrap: Bootstrap,
    /// Each table's row count, by file name.
    tables: BTreeMap<&'static str, usize>,
}

/// Replaces `out` with `scores.csv`, every score in every scope with its interval, and
/// `generations.csv`, every generation `read` saw and whether it counted, under the manifest.
pub fn write(
    out: &Path,
    session: Session,
    read: &Read,
    scores: &Scores,
    weights: &Weights,
    bootstrap: Bootstrap,
) -> Result<()> {
    let score_rows = score_rows(scores);
    let generation_rows: Vec<GenerationRow> = (read.generations.iter())
        .map(|generation| GenerationRow::of(generation, read.complete))
        .collect();
    let files = [
        ("scores.csv", score_rows.len(), encode(&score_rows)),
        (
            "generations.csv",
            generation_rows.len(),
            encode(&generation_rows),
        ),
    ];
    let manifest = Manifest {
        written: Utc::now(),
        session,
        experiments: &read.experiments,
        complete: read.complete,
        samples: read.samples.len(),
        weights: weights.by_name(),
        bootstrap,
        tables: files.iter().map(|(name, rows, _)| (*name, *rows)).collect(),
    };
    let texts: Vec<(&str, &str)> = (files.iter())
        .map(|(name, _, text)| (*name, text.as_str()))
        .collect();
    replace_dir(out, "gungnir index", &texts, &manifest)
}

/// The scores across models, then each model's, each language in index order with the index
/// before the dimensions.
fn score_rows(scores: &Scores) -> Vec<ScoreRow> {
    let scopes = std::iter::once((ALL.to_owned(), &scores.all))
        .chain((scores.models.iter()).map(|(model, cards)| (model.to_string(), cards)));
    let mut rows = Vec::new();
    for (scope, cards) in scopes {
        for language in &scores.languages {
            let Some(card) = cards.get(language) else {
                continue;
            };
            let facets = std::iter::once(("index", card.index)).chain(
                Dimension::ALL.map(|dimension| (dimension.as_str(), card.dimension(dimension))),
            );
            rows.extend(facets.map(|(dimension, estimate)| ScoreRow {
                scope: scope.clone(),
                language: language.to_string(),
                dimension,
                score: estimate.map(|estimate| estimate.score),
                low: estimate.map(|estimate| estimate.low),
                high: estimate.map(|estimate| estimate.high),
            }));
        }
    }
    rows
}

impl GenerationRow {
    fn of(generation: &Generation, complete: f64) -> Self {
        let system = &generation.system;
        Self {
            experiment: generation.experiment.clone(),
            system: system.id().to_string(),
            model: system.model().to_string(),
            effort: system.effort().as_str(),
            rep: generation.rep.get(),
            cells: generation.cells,
            graded: generation.graded,
            complete: generation.is_complete(complete),
            counted: generation.counted,
        }
    }
}
