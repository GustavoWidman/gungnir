//! Identifiers shared across the crate: slug names for catalog entries, languages, scenarios
//! and cases, repetition and attempt numbers, and reasoning effort levels.
use eyre::{Report, Result, bail, eyre};
use serde::{Deserialize, Serialize, de::IntoDeserializer};
use std::{fmt, num::NonZeroU32, str::FromStr};

macro_rules! slug {
    ($($(#[$doc:meta])* $name:ident => $what:literal;)+) => {$(
        $(#[$doc])*
        ///
        /// Always a slug matching `[a-z0-9][a-z0-9.-]*`, so it is safe as a path component.
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = Report;

            fn try_from(value: String) -> Result<Self> {
                check_slug(&value, $what)?;
                Ok(Self(value))
            }
        }

        impl FromStr for $name {
            type Err = Report;

            fn from_str(value: &str) -> Result<Self> {
                Self::try_from(value.to_owned())
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> Self {
                id.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    )+};
}

slug! {
    /// A toolchain profile, named by its directory under `languages/`.
    LanguageId => "language id";
    /// A task, named by its directory under `scenarios/`.
    ScenarioId => "scenario id";
    /// A model as the catalog names it, independent of the backend serving it.
    ModelId => "model id";
    /// One account behind one endpoint, a `[backends.<id>]` table in the catalog.
    BackendId => "backend id";
    /// A vendor CLI agent, a `[harnesses.<name>]` table in the catalog.
    HarnessName => "harness name";
    /// A pinned harness release, such as `2.1.316`.
    HarnessVersion => "harness version";
    /// A visible or hidden test case, the `NAME` of a `NAME.in` and `NAME.out` pair.
    CaseId => "case id";
    /// A timing input, the `NAME` of a `workloads/NAME.in` file.
    WorkloadId => "workload id";
}

fn check_slug(value: &str, what: &str) -> Result<()> {
    let mut chars = value.chars();
    let head = chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    let tail = chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-');
    if !(head && tail) {
        bail!("{what} {value:?} is not a slug matching [a-z0-9][a-z0-9.-]*");
    }
    Ok(())
}

/// The repetition of a trial cell, counted from one and displayed as `r1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Rep(NonZeroU32);

impl Rep {
    pub fn new(number: u32) -> Option<Self> {
        NonZeroU32::new(number).map(Self)
    }

    pub fn get(self) -> u32 {
        self.0.get()
    }

    /// `r1` through `r<count>`, in order.
    pub fn up_to(count: NonZeroU32) -> impl Iterator<Item = Self> {
        std::iter::successors(Some(Self(NonZeroU32::MIN)), |rep| {
            rep.0.checked_add(1).map(Self)
        })
        .take(count.get() as usize)
    }
}

impl fmt::Display for Rep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r{}", self.0)
    }
}

/// One launch of the agent for a trial, counted from one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AttemptNo(NonZeroU32);

impl AttemptNo {
    pub const FIRST: Self = Self(NonZeroU32::MIN);

    pub fn get(self) -> u32 {
        self.0.get()
    }

    pub fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

impl fmt::Display for AttemptNo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A reasoning effort level. The order is the rank, from `None` to `Max`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Effort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl Effort {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }

    /// The position in the order, from 0 for `None` to 6 for `Max`.
    pub fn rank(self) -> u8 {
        self as u8
    }
}

impl FromStr for Effort {
    type Err = Report;

    fn from_str(value: &str) -> Result<Self> {
        Self::deserialize(value.into_deserializer())
            .map_err(|err: serde::de::value::Error| eyre!("effort {value:?}: {err}"))
    }
}

impl fmt::Display for Effort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_accept_lowercase_digits_dots_and_dashes() {
        for value in ["python", "gpt-6.1-sol", "2.1.316", "0", "glm-5.3-flash"] {
            let id: ModelId = value.parse().unwrap();
            assert_eq!(id.as_str(), value);
            assert_eq!(id.to_string(), value);
        }
    }

    #[test]
    fn slugs_reject_anything_unsafe_as_a_path_component() {
        for value in ["", "Python", "-x", ".x", "..", "a/b", "a_b", "a b", "é"] {
            let err = value.parse::<LanguageId>().unwrap_err();
            assert!(err.to_string().contains("language id"), "{err}");
        }
    }

    #[test]
    fn slugs_validate_when_deserialized() {
        fn deserialize(value: &str) -> Result<ScenarioId, serde::de::value::Error> {
            ScenarioId::deserialize(value.into_deserializer())
        }
        assert_eq!(deserialize("word-freq").unwrap().as_str(), "word-freq");
        let err = deserialize("Word Freq").unwrap_err();
        assert!(err.to_string().contains("\"Word Freq\""), "{err}");
    }

    #[test]
    fn repetitions_count_from_r1() {
        let reps: Vec<String> = Rep::up_to(NonZeroU32::new(3).unwrap())
            .map(|rep| rep.to_string())
            .collect();
        assert_eq!(reps, ["r1", "r2", "r3"]);
        assert_eq!(Rep::new(3).map(Rep::get), Some(3));
        assert_eq!(Rep::new(0), None);
    }

    #[test]
    fn attempts_count_from_one() {
        assert_eq!(AttemptNo::FIRST.to_string(), "1");
        assert_eq!(AttemptNo::FIRST.next().to_string(), "2");
        assert_eq!(AttemptNo::FIRST.next().get(), 2);
    }

    #[test]
    fn efforts_are_ordered_by_rank_and_named_in_kebab_case() {
        let names = ["none", "minimal", "low", "medium", "high", "xhigh", "max"];
        let efforts: Vec<Effort> = names.iter().map(|name| name.parse().unwrap()).collect();
        assert!(efforts.is_sorted());
        for ((rank, effort), name) in efforts.iter().enumerate().zip(names) {
            assert_eq!(effort.to_string(), name);
            assert_eq!(usize::from(effort.rank()), rank);
        }
        assert!("extra-high".parse::<Effort>().is_err());
    }
}
