//! Declarative YAML seed/scenario DSL for the substrate.
//!
//! Two document kinds: a `Seed` (`tests/fixtures/seeds/`) is the substrate template a foundational
//! cogmap is born from; a `Scenario` (`tests/fixtures/scenarios/`) references (or embeds) a seed
//! and adds the ordered `steps` runbook. `model` holds the YAML structs; `bootseed` seeds the system
//! substrate (event-type registry + global lenses) separately from any seed; `loader` instantiates a
//! seed's substrate by calling the reusable mutation SQL functions; `runner` resolves a scenario's
//! seed, loads it through the same path, and drives the runbook in-process.
pub mod access;
pub mod bootseed;
pub mod loader;
pub mod model;
pub mod runner;
