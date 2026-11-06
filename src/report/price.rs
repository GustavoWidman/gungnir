//! List prices of the tokens agents spent, from the catalog's dated price table.
use crate::{
    catalog::{Catalog, Rate, System},
    harness::{Granularity, Harness, TokenUsage},
    run::{Sample, Tokens},
};
use chrono::NaiveDate;
use eyre::{Result, eyre};
use std::{fmt, iter::Sum, ops::Add};

/// An amount of US dollars, counted in picodollars so that sums are exact.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Usd(u128);

impl Usd {
    const PICODOLLARS: u128 = 1_000_000_000_000;

    fn of(tokens: u64, rate: Rate) -> Self {
        Self(u128::from(tokens) * u128::from(rate.picodollars()))
    }
}

/// Decimal dollars without trailing zeros, such as `0.03695`.
impl fmt::Display for Usd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (whole, fraction) = (self.0 / Self::PICODOLLARS, self.0 % Self::PICODOLLARS);
        if fraction == 0 {
            return write!(f, "{whole}");
        }
        let digits = format!("{fraction:012}");
        write!(f, "{whole}.{}", digits.trim_end_matches('0'))
    }
}

impl Add for Usd {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self(self.0 + other.0)
    }
}

impl Sum for Usd {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::default(), Add::add)
    }
}

/// What some tokens cost at list price.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cost {
    /// The tokens that have a listed rate.
    priced: Usd,
    /// Tokens without one: in a bucket the vendor lists no rate for, or served by a model the
    /// catalog does not list.
    unpriced: u64,
    /// Whether some tokens were priced at the base tier because the prompt size of the
    /// requests that spent them is unknown.
    base_tier_assumed: bool,
}

impl Cost {
    /// The list price, or `None` when some tokens have no listed rate, since leaving them out
    /// would understate it by an unknown amount.
    pub fn usd(&self) -> Option<Usd> {
        (self.unpriced == 0).then_some(self.priced)
    }

    pub fn unpriced(&self) -> u64 {
        self.unpriced
    }

    pub fn base_tier_assumed(&self) -> bool {
        self.base_tier_assumed
    }
}

impl Add for Cost {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            priced: self.priced + other.priced,
            unpriced: self.unpriced + other.unpriced,
            base_tier_assumed: self.base_tier_assumed || other.base_tier_assumed,
        }
    }
}

impl Sum for Cost {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::default(), Add::add)
    }
}

/// The catalog's list prices in effect on one day.
pub struct PriceList<'a> {
    catalog: &'a Catalog,
    day: NaiveDate,
}

impl<'a> PriceList<'a> {
    pub fn new(catalog: &'a Catalog, day: NaiveDate) -> Self {
        Self { catalog, day }
    }

    /// What `sample`, spent by an agent of `system`, costs. The model the sample names prices
    /// it, or the system's model when it names none. A model with no prices on the day is an
    /// error rather than unpriced, because the day was chosen for the whole report.
    pub fn price(&self, system: &System, sample: &Sample) -> Result<Cost> {
        let tokens = sample.tokens;
        let api_model = sample.model.as_deref().unwrap_or(system.api_model());
        let Some(model) = self.catalog.served(system.backend(), api_model) else {
            return Ok(Cost {
                unpriced: TokenUsage::from(tokens).count(),
                ..Cost::default()
            });
        };
        let rates = |prompt| {
            model.rates(self.day, prompt).ok_or_else(|| {
                eyre!(
                    "model {api_model} on backend {} has no list price on {}",
                    system.backend(),
                    self.day
                )
            })
        };
        let base = rates(0)?;
        let (rates, base_tier_assumed) = match Prompt::of(system, sample) {
            Prompt::Exact(size) => (rates(size)?, false),
            Prompt::AtMost(size) => (base, rates(size)? != base),
            Prompt::Unknown => (base, rates(u64::MAX)? != base),
        };
        let mut cost = Cost {
            priced: Usd::of(tokens.input, rates.input)
                + Usd::of(tokens.cache_read, rates.cache_read)
                + Usd::of(tokens.output, rates.output),
            unpriced: 0,
            base_tier_assumed,
        };
        for (count, rate) in [
            (tokens.cache_write, rates.cache_write),
            (tokens.cache_write_long, rates.cache_write_1h),
        ] {
            match rate {
                Some(rate) => cost.priced = cost.priced + Usd::of(count, rate),
                None => cost.unpriced += count,
            }
        }
        Ok(cost)
    }
}

/// How large the prompt of each request a sample covers was, which decides its price tier.
enum Prompt {
    /// The sample is one request.
    Exact(u64),
    /// The sample sums whole requests, so no one request's prompt was larger.
    AtMost(u64),
    /// The sample is what a session total exceeds its requests by, which no request owns.
    Unknown,
}

impl Prompt {
    fn of(system: &System, sample: &Sample) -> Self {
        let Tokens {
            input,
            cache_read,
            cache_write,
            cache_write_long,
            ..
        } = sample.tokens;
        let size = input + cache_read + cache_write + cache_write_long;
        if sample.request.is_some() {
            return Self::Exact(size);
        }
        match Harness::named(system.harness()).map(Harness::granularity) {
            Some(Granularity::Turn) => Self::AtMost(size),
            Some(Granularity::Request) | None => Self::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{id::Effort, testing};
    use tempfile::TempDir;

    /// Opus through Claude Code, which reports each request, and Luna through Codex, which
    /// reports each turn, both with a tier above 1000 prompt tokens. Opus lists no rate for
    /// the one-hour cache.
    const CATALOG: &str = r#"
[sandbox]
packages = ["coreutils"]

[measure]
packages = ["hyperfine"]

[profile]
packages = ["perf"]

[harnesses.claude-code]
package = "claude-code"

[harnesses.codex]
package = "codex"

[backends.anthropic]
harness = "claude-code"
credential = { kind = "env-token", var = "TOKEN", file = "~/token" }
egress = ["api.anthropic.com"]

[backends.openai]
harness = "codex"
credential = { kind = "codex-home", dir = "~/codex" }
egress = ["api.openai.com"]

[[models]]
id = "opus"
efforts = ["low"]
served-by = { anthropic = "claude-opus" }
prices = [
  { since = "2026-01-01", input = 4.0, output = 20.0, cache-read = 0.2, cache-write = 5.0 },
  { since = "2026-01-01", above-prompt-tokens = 1000, input = 8.0, output = 30.0, cache-read = 0.4, cache-write = 10.0 },
]

[[models]]
id = "haiku"
efforts = ["low"]
served-by = { anthropic = "claude-haiku" }
prices = [{ since = "2026-01-01", input = 0.1, output = 0.5, cache-read = 0.01 }]

[[models]]
id = "luna"
efforts = ["low"]
served-by = { openai = "gpt-luna" }
prices = [
  { since = "2026-01-01", input = 0.1, output = 0.5, cache-read = 0.01 },
  { since = "2026-01-01", above-prompt-tokens = 1000, input = 0.2, output = 0.75, cache-read = 0.01 },
]
"#;

    fn catalog() -> Catalog {
        let dir = TempDir::new().unwrap();
        testing::write(dir.path(), "gungnir.toml", CATALOG);
        Catalog::load(&dir.path().join("gungnir.toml")).unwrap()
    }

    fn system(catalog: &Catalog, model: &str) -> System {
        let backend = if model == "luna" {
            "openai"
        } else {
            "anthropic"
        };
        let version = if model == "luna" {
            "0.166.0"
        } else {
            "2.1.316"
        };
        catalog
            .system(
                &testing::id(model),
                &testing::id(backend),
                Effort::Low,
                testing::id(version),
            )
            .unwrap()
    }

    fn sample(request: Option<&str>, model: Option<&str>, tokens: Tokens) -> Sample {
        Sample {
            request: request.map(str::to_owned),
            model: model.map(str::to_owned),
            tokens,
        }
    }

    fn tokens(input: u64, cache_read: u64, output: u64) -> Tokens {
        Tokens {
            input,
            cache_read,
            output,
            ..Tokens::default()
        }
    }

    fn usd(cost: Cost) -> String {
        cost.usd()
            .map_or("unpriced".to_owned(), |usd| usd.to_string())
    }

    fn day(text: &str) -> NaiveDate {
        text.parse().unwrap()
    }

    #[test]
    fn dollars_print_exactly_without_trailing_zeros() {
        assert_eq!(Usd(0).to_string(), "0");
        assert_eq!(Usd(5_000_000_000_000).to_string(), "5");
        assert_eq!(Usd(12_300_000_000_000).to_string(), "12.3");
        assert_eq!(Usd(300_000).to_string(), "0.0000003");
        assert_eq!(Usd(1).to_string(), "0.000000000001");
        let tenths: Usd = (0..3).map(|_| Usd(100_000_000_000)).sum();
        assert_eq!(tenths.to_string(), "0.3");
    }

    #[test]
    fn a_request_is_priced_at_the_tier_its_own_prompt_reaches() {
        let catalog = catalog();
        let prices = PriceList::new(&catalog, day("2026-11-03"));
        let opus = system(&catalog, "opus");
        let small = sample(Some("msg_1"), Some("claude-opus"), tokens(400, 600, 1000));
        let cost = prices.price(&opus, &small).unwrap();
        // 400 * $4 + 600 * $0.2 + 1000 * $20, per million tokens.
        assert_eq!(usd(cost), "0.02172");
        assert!(!cost.base_tier_assumed());

        let large = sample(Some("msg_2"), Some("claude-opus"), tokens(400, 601, 1000));
        // 400 * $8 + 601 * $0.4 + 1000 * $30, per million tokens.
        assert_eq!(usd(prices.price(&opus, &large).unwrap()), "0.0334404");
    }

    #[test]
    fn a_turn_is_priced_at_the_base_tier_and_flagged_once_it_could_reach_another() {
        let catalog = catalog();
        let prices = PriceList::new(&catalog, day("2026-11-03"));
        let luna = system(&catalog, "luna");
        let small = prices
            .price(&luna, &sample(None, None, tokens(500, 500, 100)))
            .unwrap();
        // 500 * $0.1 + 500 * $0.01 + 100 * $0.5, per million tokens.
        assert_eq!(
            (usd(small), small.base_tier_assumed()),
            ("0.000105".into(), false)
        );

        let large = prices
            .price(&luna, &sample(None, None, tokens(500, 501, 100)))
            .unwrap();
        assert_eq!(
            (usd(large), large.base_tier_assumed()),
            ("0.00010501".into(), true)
        );
    }

    #[test]
    fn a_session_remainder_is_flagged_only_for_a_model_with_tiers() {
        let catalog = catalog();
        let prices = PriceList::new(&catalog, day("2026-11-03"));
        let opus = system(&catalog, "opus");
        let remainder = sample(None, Some("claude-opus"), tokens(0, 0, 1000));
        let cost = prices.price(&opus, &remainder).unwrap();
        assert_eq!((usd(cost), cost.base_tier_assumed()), ("0.02".into(), true));

        let haiku = system(&catalog, "haiku");
        let remainder = sample(None, Some("claude-haiku"), tokens(0, 0, 1000));
        let cost = prices.price(&haiku, &remainder).unwrap();
        assert_eq!(
            (usd(cost), cost.base_tier_assumed()),
            ("0.0005".into(), false)
        );
    }

    #[test]
    fn tokens_without_a_listed_rate_leave_the_cost_unknown() {
        let catalog = catalog();
        let prices = PriceList::new(&catalog, day("2026-11-03"));
        let opus = system(&catalog, "opus");
        let written = Tokens {
            cache_write: 10,
            ..tokens(1, 0, 1)
        };
        let five_minutes = prices
            .price(&opus, &sample(Some("msg_1"), None, written))
            .unwrap();
        // 1 * $4 + 10 * $5 + 1 * $20, per million tokens.
        assert_eq!(usd(five_minutes), "0.000074");

        let one_hour = Tokens {
            cache_write_long: 7,
            ..written
        };
        let cost = prices
            .price(&opus, &sample(Some("msg_1"), None, one_hour))
            .unwrap();
        assert_eq!((usd(cost), cost.unpriced()), ("unpriced".into(), 7));
        let sum = cost + five_minutes;
        assert_eq!((sum.usd(), sum.unpriced()), (None, 7));
    }

    #[test]
    fn a_model_the_catalog_does_not_list_leaves_every_token_unpriced() {
        let catalog = catalog();
        let prices = PriceList::new(&catalog, day("2026-11-03"));
        let opus = system(&catalog, "opus");
        let other = Tokens {
            reasoning: Some(5),
            ..tokens(1, 2, 30)
        };
        let cost = prices
            .price(&opus, &sample(Some("msg_1"), Some("claude-other"), other))
            .unwrap();
        assert_eq!((cost.usd(), cost.unpriced()), (None, 33));
    }

    #[test]
    fn a_day_before_a_model_s_first_price_is_an_error() {
        let catalog = catalog();
        let prices = PriceList::new(&catalog, day("2025-12-31"));
        let opus = system(&catalog, "opus");
        let err = prices
            .price(&opus, &sample(Some("msg_1"), None, tokens(1, 0, 1)))
            .unwrap_err();
        assert!(err.to_string().contains("claude-opus"), "{err}");
        assert!(err.to_string().contains("2025-12-31"), "{err}");
    }
}
