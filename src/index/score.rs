//! The index as a pure function of the sessions it reads. Inside, NaN stands for a missing
//! value, which every mean skips, as the definition averages over what exists.
use super::{Dimension, Weights};
use crate::id::{LanguageId, ModelId, ScenarioId, WorkloadId};
use rand::{Rng, RngExt, SeedableRng};
use rand_pcg::Pcg64;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// One session the index reads: a trial whose committed run is a sample, with a current grade.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub model: ModelId,
    pub scenario: ScenarioId,
    pub language: LanguageId,
    /// Whether the program passed every hidden case.
    pub passed: bool,
    /// Input, cache reads, writes to either cache and output.
    pub tokens: u64,
    pub wall_s: f64,
    /// The program's total cyclomatic complexity, when lizard measured it.
    pub ccn: Option<u64>,
    /// Each workload the trial's current measurement timed; empty without one.
    pub timings: Vec<Timing>,
}

/// The candidate's and the reference's median wall time on one timed workload.
#[derive(Clone, Debug, PartialEq)]
pub struct Timing {
    pub workload: WorkloadId,
    pub candidate_median_s: f64,
    pub reference_median_s: f64,
}

/// The resampling behind every interval. A fixed seed makes the same samples give the same
/// intervals.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Bootstrap {
    pub resamples: usize,
    pub seed: u64,
    /// The share of resamples the interval covers.
    pub level: f64,
}

impl Bootstrap {
    pub const DEFAULT: Self = Self {
        resamples: 4000,
        seed: 0,
        level: 0.95,
    };
}

/// A score and its interval, where 100 is an average language and higher is better.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Estimate {
    pub score: f64,
    pub low: f64,
    pub high: f64,
}

/// One language's scores in one scope, across models or for one model: the index and each
/// dimension. `None` where the scope has no value, such as speed for a model never measured.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub index: Option<Estimate>,
    dimensions: [Option<Estimate>; Dimension::ALL.len()],
}

impl Card {
    pub fn dimension(&self, dimension: Dimension) -> Option<Estimate> {
        self.dimensions[dimension.position()]
    }
}

/// Every language's scores, across models and for each model.
#[derive(Clone, Debug, PartialEq)]
pub struct Scores {
    /// By descending index across models, then by id; a language without one comes last.
    pub languages: Vec<LanguageId>,
    pub all: BTreeMap<LanguageId, Card>,
    pub models: BTreeMap<ModelId, BTreeMap<LanguageId, Card>>,
}

/// Scores every language of `samples`, folding the dimensions into each index by `weights`,
/// with intervals from `bootstrap`.
pub fn score(samples: &[Sample], weights: &Weights, bootstrap: Bootstrap) -> Scores {
    let grid = Grid::of(samples);
    let every: Vec<usize> = (0..grid.scenarios.len()).collect();
    let point = grid.table(&every, None, weights);
    let mut draws = vec![Vec::with_capacity(bootstrap.resamples); point.len()];
    if !every.is_empty() {
        let mut rng = Pcg64::seed_from_u64(bootstrap.seed);
        for _ in 0..bootstrap.resamples {
            let pick: Vec<usize> = every
                .iter()
                .map(|_| rng.random_range(0..every.len()))
                .collect();
            let drawn = grid.table(&pick, Some(&mut rng), weights);
            for (column, value) in draws.iter_mut().zip(drawn) {
                if !value.is_nan() {
                    column.push(value);
                }
            }
        }
    }
    let tail = (1.0 - bootstrap.level) / 2.0;
    let estimates: Vec<Option<Estimate>> = point
        .iter()
        .zip(&mut draws)
        .map(|(&point, column)| {
            column.sort_by(f64::total_cmp);
            let scale = |log: f64| 100.0 * log.exp();
            (!point.is_nan()).then(|| Estimate {
                score: scale(point),
                low: scale(quantile(column, tail)),
                high: scale(quantile(column, 1.0 - tail)),
            })
        })
        .collect();
    grid.scores(&estimates)
}

/// The facets of a scope's scores: each dimension, then the index.
const FACETS: usize = Dimension::ALL.len() + 1;
const INDEX: usize = Dimension::ALL.len();

/// The sessions of each cell, by model, scenario and language.
struct Grid {
    models: Vec<ModelId>,
    scenarios: Vec<ScenarioId>,
    languages: Vec<LanguageId>,
    cells: Vec<Vec<Observed>>,
}

/// One session's value on each dimension: 1 or 0 for the pass, the logarithm of the others,
/// and NaN for a dimension it lacks.
type Observed = [f64; Dimension::ALL.len()];

impl Grid {
    fn of(samples: &[Sample]) -> Self {
        fn sorted<'a, T: Ord + Clone + 'a>(ids: impl Iterator<Item = &'a T>) -> Vec<T> {
            ids.collect::<BTreeSet<_>>().into_iter().cloned().collect()
        }
        let mut fastest = BTreeMap::new();
        for session in samples.iter().filter(|session| session.passed) {
            for timing in &session.timings {
                let best = fastest
                    .entry((&session.scenario, &timing.workload))
                    .or_insert(f64::INFINITY);
                *best = timing.reference_median_s.min(*best);
            }
        }
        let models = sorted(samples.iter().map(|s| &s.model));
        let scenarios = sorted(samples.iter().map(|s| &s.scenario));
        let languages = sorted(samples.iter().map(|s| &s.language));
        let mut grid = Self {
            cells: vec![Vec::new(); models.len() * scenarios.len() * languages.len()],
            models,
            scenarios,
            languages,
        };
        for session in samples {
            let cell = grid.cell(
                grid.models.partition_point(|id| id < &session.model),
                grid.scenarios.partition_point(|id| id < &session.scenario),
                grid.languages.partition_point(|id| id < &session.language),
            );
            grid.cells[cell].push(observe(session, &fastest));
        }
        grid
    }

    fn cell(&self, model: usize, scenario: usize, language: usize) -> usize {
        (model * self.scenarios.len() + scenario) * self.languages.len() + language
    }

    /// Where a scope's facet of a language sits in a table. The scopes are the models in order,
    /// then all of them.
    fn at(&self, scope: usize, facet: usize, language: usize) -> usize {
        (scope * FACETS + facet) * self.languages.len() + language
    }

    /// Every score of every scope, as a mean log advantage, from the scenarios `pick` names:
    /// each once, or with their sessions resampled when given `rng`.
    fn table(&self, pick: &[usize], mut rng: Option<&mut Pcg64>, weights: &Weights) -> Vec<f64> {
        let languages = self.languages.len();
        let mut table = vec![f64::NAN; (self.models.len() + 1) * FACETS * languages];
        let mut sum = vec![0.0; table.len()];
        let mut count = vec![0_u32; table.len()];
        let mut values = vec![f64::NAN; Dimension::ALL.len() * languages];
        for &scenario in pick {
            for model in 0..self.models.len() {
                values.fill(f64::NAN);
                for language in 0..languages {
                    let cell = &self.cells[self.cell(model, scenario, language)];
                    if cell.is_empty() {
                        continue;
                    }
                    let value = match rng.as_deref_mut() {
                        None => summarize(cell.iter()),
                        Some(rng) => summarize(resample(cell, rng)),
                    };
                    for (dimension, value) in value.into_iter().enumerate() {
                        values[dimension * languages + language] = value;
                    }
                }
                for dimension in Dimension::ALL {
                    let row = &values[dimension.position() * languages..][..languages];
                    let center = mean(row.iter().copied());
                    let sign = if dimension.larger_is_better() {
                        1.0
                    } else {
                        -1.0
                    };
                    for (language, value) in row.iter().enumerate() {
                        if !value.is_nan() {
                            let at = self.at(model, dimension.position(), language);
                            sum[at] += sign * (value - center);
                            count[at] += 1;
                        }
                    }
                }
            }
        }
        for ((value, sum), count) in table.iter_mut().zip(sum).zip(count) {
            if count > 0 {
                *value = sum / f64::from(count);
            }
        }
        self.fold(&mut table, weights);
        table
    }

    /// Fills in each dimension across models, the mean over the models that have it, and every
    /// scope's index, the weighted mean over the dimensions it has.
    fn fold(&self, table: &mut [f64], weights: &Weights) {
        let all = self.models.len();
        for language in 0..self.languages.len() {
            for dimension in 0..Dimension::ALL.len() {
                let models = (0..all).map(|model| table[self.at(model, dimension, language)]);
                table[self.at(all, dimension, language)] = mean(models);
            }
            for scope in 0..=all {
                let weighed = Dimension::ALL.map(|dimension| {
                    let value = table[self.at(scope, dimension.position(), language)];
                    (value, weights.of(dimension))
                });
                table[self.at(scope, INDEX, language)] = weighted_mean(weighed);
            }
        }
    }

    fn scores(&self, estimates: &[Option<Estimate>]) -> Scores {
        let card = |scope: usize| -> BTreeMap<LanguageId, Card> {
            let estimate = |facet, language| estimates[self.at(scope, facet, language)];
            (self.languages.iter().enumerate())
                .map(|(language, id)| {
                    let card = Card {
                        index: estimate(INDEX, language),
                        dimensions: Dimension::ALL
                            .map(|dimension| estimate(dimension.position(), language)),
                    };
                    (id.clone(), card)
                })
                .collect()
        };
        let all = card(self.models.len());
        let mut languages = self.languages.clone();
        let index = |language: &LanguageId| all[language].index.map(|index| index.score);
        languages.sort_by(|a, b| match (index(a), index(b)) {
            (Some(a), Some(b)) => b.total_cmp(&a),
            (a, b) => b.is_some().cmp(&a.is_some()),
        });
        Scores {
            languages,
            all,
            models: (self.models.iter().enumerate())
                .map(|(model, id)| (id.clone(), card(model)))
                .collect(),
        }
    }
}

/// What `session` counts on each dimension, its speed against the `fastest` reference median
/// of each scenario's workload.
fn observe(session: &Sample, fastest: &BTreeMap<(&ScenarioId, &WorkloadId), f64>) -> Observed {
    let speed = || {
        mean(session.timings.iter().map(|timing| {
            let best = fastest.get(&(&session.scenario, &timing.workload));
            best.map_or(f64::NAN, |best| (timing.candidate_median_s / best).ln())
        }))
    };
    Dimension::ALL.map(|dimension| match dimension {
        _ if dimension.passing_only() && !session.passed => f64::NAN,
        Dimension::Pass => f64::from(u8::from(session.passed)),
        Dimension::Tokens if session.tokens == 0 => f64::NAN,
        Dimension::Tokens => (session.tokens as f64).ln(),
        Dimension::Time => session.wall_s.ln(),
        Dimension::Speed => speed(),
        Dimension::Simplicity => session.ccn.map_or(f64::NAN, |ccn| (ccn.max(1) as f64).ln()),
    })
}

/// A cell's value on each dimension: the logarithm of the smoothed pass rate
/// `(k + 1/2) / (n + 1)`, and the mean of every other dimension over the sessions that have it.
fn summarize<'a>(sessions: impl Iterator<Item = &'a Observed>) -> Observed {
    let mut sum = [0.0; Dimension::ALL.len()];
    let mut count = [0_u32; Dimension::ALL.len()];
    for session in sessions {
        for ((sum, count), value) in sum.iter_mut().zip(&mut count).zip(session) {
            if !value.is_nan() {
                *sum += value;
                *count += 1;
            }
        }
    }
    Dimension::ALL.map(|dimension| {
        let (sum, count) = (sum[dimension.position()], count[dimension.position()]);
        match dimension {
            Dimension::Pass => ((sum + 0.5) / (f64::from(count) + 1.0)).ln(),
            _ if count == 0 => f64::NAN,
            _ => sum / f64::from(count),
        }
    })
}

/// As many sessions as `cell` holds, drawn from it with replacement.
fn resample<'a>(cell: &'a [Observed], rng: &mut impl Rng) -> impl Iterator<Item = &'a Observed> {
    (0..cell.len()).map(move |_| &cell[rng.random_range(0..cell.len())])
}

/// The mean of the values that are not NaN, or NaN without one.
fn mean(values: impl IntoIterator<Item = f64>) -> f64 {
    weighted_mean(values.into_iter().map(|value| (value, 1.0)))
}

/// The mean of the values that are not NaN, each by its weight, or NaN when their weights sum
/// to zero.
fn weighted_mean(values: impl IntoIterator<Item = (f64, f64)>) -> f64 {
    let (sum, weight) = values
        .into_iter()
        .filter(|(value, _)| !value.is_nan())
        .fold((0.0, 0.0), |(sum, total), (value, weight)| {
            (sum + weight * value, total + weight)
        });
    if weight > 0.0 { sum / weight } else { f64::NAN }
}

/// The `q` quantile of `sorted`, interpolating linearly between the two nearest values, or NaN
/// when it is empty.
fn quantile(sorted: &[f64], q: f64) -> f64 {
    let Some(last) = sorted.len().checked_sub(1) else {
        return f64::NAN;
    };
    let rank = last as f64 * q;
    let below = rank.floor() as usize;
    let above = rank.ceil() as usize;
    sorted[below] + (rank - below as f64) * (sorted[above] - sorted[below])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    /// A session with complexity 10 when it passed and no measurement.
    fn session(model: &str, scenario: &str, language: &str, passed: bool, tokens: u64) -> Sample {
        Sample {
            model: testing::id(model),
            scenario: testing::id(scenario),
            language: testing::id(language),
            passed,
            tokens,
            wall_s: 60.0,
            ccn: passed.then_some(10),
            timings: Vec::new(),
        }
    }

    fn timed(mut session: Sample, workload: &str, candidate: f64, reference: f64) -> Sample {
        session.timings.push(Timing {
            workload: testing::id(workload),
            candidate_median_s: candidate,
            reference_median_s: reference,
        });
        session
    }

    fn language(id: &str) -> LanguageId {
        testing::id(id)
    }

    fn model(id: &str) -> ModelId {
        testing::id(id)
    }

    #[track_caller]
    fn close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9 * expected.abs().max(1.0),
            "{actual} is not {expected}"
        );
    }

    /// Asserts a score whose interval collapses onto it, as when every resample draws the same
    /// values.
    #[track_caller]
    fn exactly(estimate: Option<Estimate>, expected: f64) {
        let estimate = estimate.expect("a score");
        for value in [estimate.score, estimate.low, estimate.high] {
            close(value, expected);
        }
    }

    #[test]
    fn a_cell_that_never_passed_scores_on_what_every_session_has() {
        let sessions = [
            session("luna", "ledger", "python", true, 100),
            session("luna", "ledger", "python", true, 100),
            session("luna", "ledger", "rust", false, 400),
            session("luna", "ledger", "rust", false, 400),
        ];
        let scores = score(&sessions, &Weights::default(), Bootstrap::DEFAULT);

        assert_eq!(scores.languages, [language("python"), language("rust")]);
        let (python, rust) = (
            &scores.all[&language("python")],
            &scores.all[&language("rust")],
        );
        // ln(2.5/3) against ln(0.5/3): each lies ln(5)/2 from their mean.
        exactly(python.dimension(Dimension::Pass), 100.0 * 5_f64.sqrt());
        exactly(rust.dimension(Dimension::Pass), 100.0 / 5_f64.sqrt());
        // Four times the tokens is half the mean's on one side and twice on the other.
        exactly(python.dimension(Dimension::Tokens), 200.0);
        exactly(rust.dimension(Dimension::Tokens), 50.0);
        exactly(python.dimension(Dimension::Time), 100.0);
        // Only passing programs have a complexity, so Python is the only language that does.
        exactly(python.dimension(Dimension::Simplicity), 100.0);
        assert_eq!(rust.dimension(Dimension::Simplicity), None);
        assert_eq!(python.dimension(Dimension::Speed), None);
        // Python's index averages four dimensions, Rust's the three it has.
        let python_index = (5_f64.sqrt() * 2.0).powf(1.0 / 4.0);
        exactly(python.index, 100.0 * python_index);
        exactly(rust.index, 100.0 * (5_f64.sqrt() * 2.0).powf(-1.0 / 3.0));
        assert_eq!(scores.models[&model("luna")], scores.all);
    }

    /// Two models on one scenario. Luna's programs were measured: on the one workload Python's
    /// takes 2s against its reference's 1s and Rust's 0.5s against 0.25s, so the fastest
    /// reference is Rust's and the programs are 8 and 2 times slower than it. Haiku's were not.
    fn measured_and_not() -> Vec<Sample> {
        vec![
            timed(
                session("luna", "ledger", "python", true, 100),
                "large",
                2.0,
                1.0,
            ),
            timed(
                session("luna", "ledger", "rust", true, 100),
                "large",
                0.5,
                0.25,
            ),
            session("haiku", "ledger", "python", true, 100),
            session("haiku", "ledger", "rust", true, 100),
        ]
    }

    #[test]
    fn a_model_without_measurements_lacks_speed_and_the_others_carry_it() {
        let scores = score(&measured_and_not(), &Weights::default(), Bootstrap::DEFAULT);

        let haiku = &scores.models[&model("haiku")];
        let luna = &scores.models[&model("luna")];
        for language in [language("python"), language("rust")] {
            assert_eq!(haiku[&language].dimension(Dimension::Speed), None);
            exactly(haiku[&language].index, 100.0);
        }
        // ln 8 and ln 2 lie ln 2 either side of their mean.
        exactly(luna[&language("python")].dimension(Dimension::Speed), 50.0);
        exactly(luna[&language("rust")].dimension(Dimension::Speed), 200.0);
        let (python, rust) = (
            &scores.all[&language("python")],
            &scores.all[&language("rust")],
        );
        exactly(python.dimension(Dimension::Speed), 50.0);
        exactly(rust.dimension(Dimension::Speed), 200.0);
        // Every other dimension ties, so speed is one fifth of each index.
        exactly(python.index, 100.0 * 2_f64.powf(-1.0 / 5.0));
        exactly(rust.index, 100.0 * 2_f64.powf(1.0 / 5.0));
        exactly(luna[&language("rust")].index, 100.0 * 2_f64.powf(1.0 / 5.0));
        assert_eq!(scores.languages, [language("rust"), language("python")]);
    }

    #[test]
    fn weights_fold_the_dimensions_into_each_index_and_leave_the_dimensions_alone() {
        let sessions = measured_and_not();
        let even = score(&sessions, &Weights::default(), Bootstrap::DEFAULT);
        let rust = |scores: &Scores| scores.all[&language("rust")].clone();

        let without_speed = Weights::new([(Dimension::Speed, 0.0)]).unwrap();
        let scores = score(&sessions, &without_speed, Bootstrap::DEFAULT);
        exactly(rust(&scores).index, 100.0);
        exactly(
            scores.models[&model("luna")][&language("rust")].index,
            100.0,
        );
        for dimension in Dimension::ALL {
            assert_eq!(
                rust(&scores).dimension(dimension),
                rust(&even).dimension(dimension)
            );
        }

        let double_speed = Weights::new([(Dimension::Speed, 2.0)]).unwrap();
        let scores = score(&sessions, &double_speed, Bootstrap::DEFAULT);
        exactly(rust(&scores).index, 100.0 * 2_f64.powf(2.0 / 6.0));

        let only_speed =
            Weights::new(Dimension::ALL.map(|d| (d, f64::from(u8::from(d == Dimension::Speed)))))
                .unwrap();
        let scores = score(&sessions, &only_speed, Bootstrap::DEFAULT);
        exactly(rust(&scores).index, 200.0);
        // Haiku has no speed, so nothing it has weighs anything.
        assert_eq!(
            scores.models[&model("haiku")][&language("rust")].index,
            None
        );
    }

    #[test]
    fn speed_is_the_geometric_mean_over_workloads_against_the_fastest_reference() {
        let python = session("luna", "ledger", "python", true, 100);
        let python = timed(timed(python, "large", 2.0, 1.0), "small", 0.5, 1.0);
        let rust = session("luna", "ledger", "rust", true, 100);
        let rust = timed(timed(rust, "large", 1.0, 0.5), "small", 0.5, 0.25);
        let scores = score(&[python, rust], &Weights::default(), Bootstrap::DEFAULT);
        // Against the fastest references, 0.5s and 0.25s, Python's program is 4 and 2 times
        // slower and Rust's 2 and 2: geometric means of sqrt 8 and 2, sqrt 2 apart.
        let all = &scores.all;
        exactly(
            all[&language("python")].dimension(Dimension::Speed),
            100.0 / 2_f64.powf(0.25),
        );
        exactly(
            all[&language("rust")].dimension(Dimension::Speed),
            100.0 * 2_f64.powf(0.25),
        );
    }

    #[test]
    fn complexity_counts_at_least_one_and_the_pass_rate_is_smoothed() {
        let mut flat = session("luna", "ledger", "python", true, 100);
        flat.ccn = Some(0);
        let mut branchy = session("luna", "ledger", "rust", true, 100);
        branchy.ccn = Some(4);
        let failed = session("luna", "ledger", "rust", false, 100);
        let scores = score(
            &[flat, branchy, failed],
            &Weights::default(),
            Bootstrap::DEFAULT,
        );
        let all = &scores.all;
        let (python, rust) = (&all[&language("python")], &all[&language("rust")]);
        close(
            python.dimension(Dimension::Simplicity).unwrap().score,
            200.0,
        );
        close(rust.dimension(Dimension::Simplicity).unwrap().score, 50.0);
        // One pass of one, 1.5/2, against one of two, 1.5/3.
        let ratio = (1.5_f64 / 2.0 / (1.5 / 3.0)).sqrt();
        close(
            python.dimension(Dimension::Pass).unwrap().score,
            100.0 * ratio,
        );
        close(
            rust.dimension(Dimension::Pass).unwrap().score,
            100.0 / ratio,
        );
    }

    #[test]
    fn advantages_average_over_the_scenarios_where_the_cell_exists() {
        let sessions = [
            session("luna", "ledger", "python", true, 100),
            session("luna", "ledger", "rust", true, 400),
            session("luna", "tiny-vm", "python", true, 100),
            session("luna", "tiny-vm", "go", true, 100),
        ];
        let scores = score(&sessions, &Weights::default(), Bootstrap::DEFAULT);
        let tokens = |id| {
            scores.all[&language(id)]
                .dimension(Dimension::Tokens)
                .unwrap()
        };
        // Python is twice as good as the mean on ledger and even on tiny-vm; Rust and Go each
        // have one scenario.
        close(tokens("python").score, 100.0 * 2_f64.sqrt());
        close(tokens("rust").score, 50.0);
        close(tokens("go").score, 100.0);
    }

    /// Four scenarios whose sessions vary, so that resamples differ.
    fn varied() -> Vec<Sample> {
        let mut sessions = Vec::new();
        for (s, scenario) in ["bigint-calc", "ledger", "regex-lite", "tiny-vm"]
            .into_iter()
            .enumerate()
        {
            for (l, language) in ["c", "python", "rust"].into_iter().enumerate() {
                for k in 0..3_u64 {
                    let passed = (s + l + k as usize) % 3 != 0;
                    let tokens = 1000 * (1 + s as u64) + 300 * l as u64 + 70 * k;
                    let mut session = session("luna", scenario, language, passed, tokens);
                    session.wall_s = 30.0 + 7.0 * (s * l) as f64 + k as f64;
                    if passed {
                        let candidate = 0.1 * (1 + l + k as usize) as f64;
                        session = timed(session, "large", candidate, 0.1 + 0.05 * l as f64);
                    }
                    sessions.push(session);
                }
            }
        }
        sessions
    }

    #[test]
    fn the_bootstrap_is_the_same_for_the_same_sessions_and_seed() {
        let sessions = varied();
        let weights = Weights::default();
        let first = score(&sessions, &weights, Bootstrap::DEFAULT);
        assert_eq!(score(&sessions, &weights, Bootstrap::DEFAULT), first);

        let reseeded = Bootstrap {
            seed: 1,
            ..Bootstrap::DEFAULT
        };
        let other = score(&sessions, &weights, reseeded);
        let python = |scores: &Scores| scores.all[&language("python")].index.unwrap();
        assert_eq!(python(&other).score, python(&first).score);
        assert_ne!(
            (python(&other).low, python(&other).high),
            (python(&first).low, python(&first).high)
        );
        for scores in [&first, &other] {
            for card in scores.all.values() {
                let index = card.index.unwrap();
                assert!(
                    index.low < index.score && index.score < index.high,
                    "{index:?}"
                );
            }
        }
    }

    #[test]
    fn a_session_that_reported_no_tokens_lacks_them() {
        let sessions = [
            session("luna", "ledger", "python", true, 0),
            session("luna", "ledger", "python", true, 100),
            session("luna", "ledger", "rust", true, 400),
            session("luna", "ledger", "go", true, 0),
        ];
        let scores = score(&sessions, &Weights::default(), Bootstrap::DEFAULT);
        let tokens = |id| scores.all[&language(id)].dimension(Dimension::Tokens);
        close(tokens("python").unwrap().score, 200.0);
        close(tokens("rust").unwrap().score, 50.0);
        assert_eq!(tokens("go"), None);
    }

    #[test]
    fn no_sessions_score_nothing() {
        let scores = score(&[], &Weights::default(), Bootstrap::DEFAULT);
        assert!(scores.languages.is_empty() && scores.all.is_empty() && scores.models.is_empty());
    }

    #[test]
    fn quantiles_interpolate_between_neighbours() {
        let sorted = [1.0, 2.0, 3.0, 4.0];
        close(quantile(&sorted, 0.025), 1.075);
        close(quantile(&sorted, 0.5), 2.5);
        close(quantile(&sorted, 1.0), 4.0);
        assert!(quantile(&[], 0.5).is_nan());
    }
}
