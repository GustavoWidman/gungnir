//! A committed measurement and the inputs it was made from, so a later phase can tell a stale
//! measurement by comparing the stored inputs with the current ones.
use super::{Comparison, Machine, Outcome, Program, Settings};
use crate::{
    id::WorkloadId, language::Language, provenance::SessionId, scenario::Workload, store::Digest,
    toolchains::Toolchains,
};
use eyre::Result;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

/// Raised whenever measuring would time the same inputs differently, which makes every stored
/// measurement stale.
const MEASURE_VERSION: u32 = 1;

/// `measure/record.json`. The measure directory also holds the hyperfine exports that
/// [`super::compare`] writes as evidence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeasureRecord {
    session: SessionId,
    inputs: MeasureInputs,
    machine: Machine,
    workloads: BTreeMap<WorkloadId, Outcome>,
}

impl MeasureRecord {
    pub fn new(session: SessionId, inputs: MeasureInputs, comparison: Comparison) -> Self {
        Self {
            session,
            inputs,
            machine: comparison.machine,
            workloads: comparison.workloads,
        }
    }

    /// The invocation that measured.
    pub fn session(&self) -> &SessionId {
        &self.session
    }

    pub fn inputs(&self) -> &MeasureInputs {
        &self.inputs
    }

    pub fn machine(&self) -> &Machine {
        &self.machine
    }

    pub fn workloads(&self) -> &BTreeMap<WorkloadId, Outcome> {
        &self.workloads
    }
}

/// Everything a measurement depends on. Equal inputs time the same programs the same way, so
/// a stored measurement is stale exactly when its inputs differ from the current ones.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeasureInputs {
    measure: u32,
    /// Both workspaces without their build output.
    reference: Digest,
    candidate: Digest,
    workloads: BTreeMap<WorkloadId, Digest>,
    /// The store paths of the environments a timed run binds, each naming its whole closure.
    toolchains: Environments,
    build: Vec<Vec<String>>,
    run: Vec<String>,
    env: BTreeMap<String, String>,
    settings: Settings,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Environments {
    base: PathBuf,
    language: PathBuf,
    measure: PathBuf,
}

impl MeasureInputs {
    /// The inputs of timing `candidate` against `reference`, both `language` programs, on
    /// `workloads` under `settings` now.
    pub fn new(
        language: &Language,
        toolchains: &Toolchains,
        reference: &Program<'_>,
        candidate: &Program<'_>,
        workloads: &BTreeMap<WorkloadId, Workload>,
        settings: &Settings,
    ) -> Result<Self> {
        Ok(Self {
            measure: MEASURE_VERSION,
            reference: reference.digest(),
            candidate: candidate.digest(),
            workloads: workloads
                .iter()
                .map(|(id, workload)| (id.clone(), workload.digest))
                .collect(),
            toolchains: Environments {
                base: toolchains.base().root().to_owned(),
                language: toolchains
                    .language(language.id().as_str())?
                    .root()
                    .to_owned(),
                measure: toolchains.measure().root().to_owned(),
            },
            build: language
                .build()
                .iter()
                .map(|step| step.args().to_vec())
                .collect(),
            run: language.run().args().to_vec(),
            env: language.env().clone(),
            settings: *settings,
        })
    }
}
