//! Operations layer — commands, actions, events, and the Backend trait.
//!
//! This module is the canonical home for command/action/event vocabulary
//! shared across all surfaces (CLI-local-vault, CLI-cloud, MCP, API-HTTP)
//! and both backends (DbBackend in temper-api, VaultBackend in temper-cli).
//!
//! See `docs/superpowers/specs/2026-05-01-shared-core-execution-paths-design.md`.

mod backend;
mod commands;
mod events;
mod inputs;
mod output;
mod resource_ref;
mod surface;

pub use backend::{Backend, ResourceSummary, SearchHit};
pub use commands::{
    CreateResource, DeleteResource, ListResources, SearchResources, ShowResource, UpdateResource,
};
pub use events::{DomainEvent, PushDeferReason};
pub use inputs::{BodyUpdate, ListFilter, SearchQuery};
pub use output::CommandOutput;
pub use resource_ref::ResourceRef;
pub use surface::Surface;

#[cfg(test)]
mod smoke {
    /// Smoke test: the module compiles and is reachable.
    #[test]
    fn module_exists() {
        // No-op; existence of this test passing means the module compiled.
    }
}
