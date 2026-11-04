//! The run phase: each pending trial's agent, launched headless in a sandbox, until one attempt
//! is committed as the trial's sample.
mod attempt;
#[cfg(test)]
pub(crate) mod fixture;
mod gate;
mod records;
mod schedule;
mod workspace;

pub use attempt::{Attempted, Rig};
pub use records::{
    AgentExit, Cut, DiscardedAttempt, Outcome, Process, ProcessExit, Quota, RunInputs, RunRecord,
    Sample, Tokens,
};
pub use schedule::{Tally, schedule};

use crate::{
    catalog::Backend, experiment::Experiment, language::Language, scenario::Brief, trial::TrialKey,
};
use eyre::{Result, eyre};

/// One trial as the run phase sees it: the system, and what its agent is given. It holds the
/// scenario's brief and never the scenario, so nothing in it leads to a hidden case.
pub struct Task<'a> {
    pub trial: &'a TrialKey,
    pub language: &'a Language,
    pub brief: &'a Brief,
    pub backend: &'a Backend,
}

impl<'a> Task<'a> {
    pub fn of(experiment: &'a Experiment, trial: &'a TrialKey) -> Result<Self> {
        let language = experiment
            .languages()
            .iter()
            .find(|language| language.id() == &trial.language)
            .ok_or_else(|| eyre!("experiment has no language {}", trial.language))?;
        let scenario = experiment
            .scenarios()
            .iter()
            .find(|scenario| scenario.id() == &trial.scenario)
            .ok_or_else(|| eyre!("experiment has no scenario {}", trial.scenario))?;
        let backend = trial.system.backend();
        let backend = experiment
            .catalog()
            .backend(backend)
            .ok_or_else(|| eyre!("catalog has no backend {backend}"))?;
        Ok(Self {
            trial,
            language,
            brief: scenario.brief(),
            backend,
        })
    }
}
