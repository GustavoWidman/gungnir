//! gungnir runs vendor coding agents in sandboxes, grades and times what they write, and
//! exports tidy tables. See `docs/architecture.md` for the design contract.

pub mod catalog;
pub mod cli;
pub mod egress;
pub mod experiment;
pub mod id;
pub mod language;
pub mod provenance;
pub mod sandbox;
pub mod scenario;
pub mod secret;
pub mod stop;
pub mod store;
pub mod toolchains;
pub mod trial;

#[cfg(test)]
mod testing;
