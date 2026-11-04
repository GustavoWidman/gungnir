//! Summary statistics of timed runs, as pure functions.

/// The median, fastest and slowest of some times, in seconds. All three are NaN when there
/// are no times.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spread {
    pub median: f64,
    pub min: f64,
    pub max: f64,
}

impl Spread {
    pub fn of(values: impl IntoIterator<Item = f64>) -> Self {
        let mut sorted: Vec<f64> = values.into_iter().collect();
        sorted.sort_by(f64::total_cmp);
        Self {
            median: median_of_sorted(&sorted),
            min: sorted.first().copied().unwrap_or(f64::NAN),
            max: sorted.last().copied().unwrap_or(f64::NAN),
        }
    }
}

/// The median of `values`, or NaN when there are none.
pub fn median(values: impl IntoIterator<Item = f64>) -> f64 {
    Spread::of(values).median
}

fn median_of_sorted(sorted: &[f64]) -> f64 {
    match sorted.len() {
        0 => f64::NAN,
        len if len % 2 == 1 => sorted[len / 2],
        len => (sorted[len / 2 - 1] + sorted[len / 2]) / 2.0,
    }
}

/// How many times longer the candidate takes than the reference: the geometric mean of the
/// per-block ratios, with a two-sided 95% t interval over the blocks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ratio {
    pub geomean: f64,
    /// `None` from a single block, which shows no spread.
    pub interval: Option<(f64, f64)>,
}

impl Ratio {
    /// From the natural log of each block's ratio. The logs are averaged rather than the
    /// ratios, so a block twice as slow and one twice as fast cancel out.
    pub fn of(logs: &[f64]) -> Self {
        let count = logs.len() as f64;
        let mean = logs.iter().sum::<f64>() / count;
        let interval = (logs.len() >= 2).then(|| {
            let variance = logs.iter().map(|log| (log - mean).powi(2)).sum::<f64>() / (count - 1.0);
            let half = t975(logs.len() - 1) * (variance / count).sqrt();
            ((mean - half).exp(), (mean + half).exp())
        });
        Self {
            geomean: mean.exp(),
            interval,
        }
    }
}

/// The 97.5th percentile of Student's t with `df` degrees of freedom. Tabulated up to 30,
/// beyond which the Cornish-Fisher expansion is within 1e-6 of it.
fn t975(df: usize) -> f64 {
    const TABLE: [f64; 30] = [
        12.706205, 4.302653, 3.182446, 2.776445, 2.570582, 2.446912, 2.364624, 2.306004, 2.262157,
        2.228139, 2.200985, 2.178813, 2.160369, 2.144787, 2.131450, 2.119905, 2.109816, 2.100922,
        2.093024, 2.085963, 2.079614, 2.073873, 2.068658, 2.063899, 2.059539, 2.055529, 2.051831,
        2.048407, 2.045230, 2.042272,
    ];
    if let Some(t) = df.checked_sub(1).and_then(|index| TABLE.get(index)) {
        return *t;
    }
    let z: f64 = 1.959_963_984_540_054;
    let df = df as f64;
    z + (z.powi(3) + z) / (4.0 * df)
        + (5.0 * z.powi(5) + 16.0 * z.powi(3) + 3.0 * z) / (96.0 * df.powi(2))
        + (3.0 * z.powi(7) + 19.0 * z.powi(5) + 17.0 * z.powi(3) - 15.0 * z) / (384.0 * df.powi(3))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: f64, expected: f64) -> bool {
        (actual - expected).abs() <= 1e-9 * expected.abs().max(1.0)
    }

    #[test]
    fn a_spread_is_the_median_and_the_extremes() {
        let odd = Spread::of([3.0, 1.0, 2.0]);
        assert_eq!((odd.median, odd.min, odd.max), (2.0, 1.0, 3.0));
        let even = Spread::of([4.0, 1.0, 3.0, 2.0]);
        assert_eq!((even.median, even.min, even.max), (2.5, 1.0, 4.0));
        assert_eq!(median([7.0]), 7.0);
    }

    #[test]
    fn no_times_spread_to_nan_rather_than_panic() {
        let empty = Spread::of([]);
        assert!(empty.median.is_nan() && empty.min.is_nan() && empty.max.is_nan());
    }

    #[test]
    fn the_ratio_is_the_geometric_mean_of_the_block_ratios() {
        let doubled_and_halved = Ratio::of(&[2f64.ln(), 0.5f64.ln()]);
        assert!(close(doubled_and_halved.geomean, 1.0));

        let ratio = Ratio::of(&[1.1f64.ln(), 1.2f64.ln(), 1.3f64.ln()]);
        assert!(close(ratio.geomean, (1.1f64 * 1.2 * 1.3).cbrt()));
    }

    #[test]
    fn the_interval_is_a_t_interval_on_the_log_scale() {
        // Logs 0.1 and 0.3: mean 0.2, standard error 0.1, and t(1) = 12.706205.
        let (low, high) = Ratio::of(&[0.1, 0.3]).interval.unwrap();
        assert!(close(low, (0.2f64 - 1.270_620_5).exp()), "{low}");
        assert!(close(high, (0.2f64 + 1.270_620_5).exp()), "{high}");

        // Five blocks around 0 with squares summing to 0.025: variance 0.025 / 4.
        let logs = [-0.1, -0.05, 0.0, 0.05, 0.1];
        let half = 2.776445 * (0.025f64 / 4.0).sqrt() / 5f64.sqrt();
        let (low, high) = Ratio::of(&logs).interval.unwrap();
        assert!(
            close(low, (-half).exp()) && close(high, half.exp()),
            "{low} {high}"
        );
    }

    #[test]
    fn identical_blocks_have_a_point_interval_and_one_block_has_none() {
        let same = Ratio::of(&[0.25; 4]);
        let (low, high) = same.interval.unwrap();
        assert!(close(low, 0.25f64.exp()) && close(high, 0.25f64.exp()));
        assert_eq!(Ratio::of(&[0.25]).interval, None);
    }

    #[test]
    fn t_critical_values_match_published_tables() {
        for (df, expected) in [
            (1, 12.706),
            (5, 2.571),
            (30, 2.042),
            (60, 2.000),
            (120, 1.980),
        ] {
            assert!((t975(df) - expected).abs() < 5e-4, "t({df}) = {}", t975(df));
        }
        assert!((t975(100_000) - 1.960).abs() < 5e-4);
    }
}
