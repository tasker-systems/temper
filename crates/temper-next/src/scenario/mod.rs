//! Declarative YAML seed/scenario DSL for the `temper_next` artifact.
//!
//! `model` holds the YAML structs; `bootseed` seeds the system substrate (event-type registry +
//! global lenses) separately from any scenario; `loader` instantiates a scenario's substrate by
//! calling the reusable mutation SQL functions; `runner` drives the ordered step runbook in-process.
pub mod bootseed;
pub mod loader;
pub mod model;
pub mod runner;
