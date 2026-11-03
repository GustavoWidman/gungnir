//! gungnir runs vendor coding agents in sandboxes, grades and times what they write, and
//! exports tidy tables. See `docs/architecture.md` for the design contract.

pub mod catalog;
pub mod id;
pub mod store;

#[cfg(test)]
mod testing;
