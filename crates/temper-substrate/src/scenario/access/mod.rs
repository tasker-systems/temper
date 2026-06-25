//! Self-contained access-scenario kind: a world (profiles / entities / teams + DAG / cogmaps +
//! team-joins / resources + homes + grants / homed edges) plus inline access **checks** that assert
//! the kernel gate functions. Separate from the charter seed/scenario kinds — access proofs are
//! static (seed the topology, assert), with no materialize / lens / telos machinery. Ports the
//! legacy hand-SQL access fixtures (the seed topology + its scenario invariants) into the
//! declarative harness.
pub mod loader;
pub mod model;
pub mod runner;

pub use loader::{load, LoadedAccess};
pub use runner::run_access_scenario;
