//! gungnir runs vendor coding agents in sandboxes, grades and times what they write, and
//! exports tidy tables. See `docs/architecture.md` for the design contract.

pub mod catalog;
pub mod cli;
pub mod egress;
pub mod experiment;
pub mod grade;
pub mod harness;
pub mod id;
pub mod language;
pub mod lifecycle;
pub mod measure;
pub mod provenance;
pub mod quota;
pub mod report;
pub mod run;
pub mod sandbox;
pub mod scenario;
pub mod secret;
pub mod stop;
pub mod store;
pub mod toolchains;
pub mod trial;
pub mod vault;
pub mod workload;

#[cfg(test)]
mod testing;
