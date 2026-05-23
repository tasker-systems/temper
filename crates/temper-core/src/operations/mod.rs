//! Operations layer — commands, actions, events, and the Backend trait.
//!
//! This module is the canonical home for command/action/event vocabulary
//! shared across all surfaces (CLI-local-vault, CLI-cloud, MCP, API-HTTP)
//! and both backends (DbBackend in temper-api, CloudBackend in temper-cli).
//!
//! See `docs/superpowers/specs/2026-05-01-shared-core-execution-paths-design.md`.

mod actions;
mod backend;
mod commands;
mod events;
mod inputs;
mod output;
mod resource_ref;
mod surface;

pub use actions::{
    apply_defaults, apply_defaults_value, assemble_frontmatter_document,
    ensure_managed_identity_keys, merge_managed_meta, merge_open_meta, validate_create,
    validate_doctype, validate_open_meta_keys, validate_slug, validate_update, ActionError,
    FrontmatterIdentity,
};
pub use backend::{Backend, ResourceSummary, SearchHit};
pub use commands::{
    CreateResource, DeleteResource, ListResources, MoveSpec, SearchResources, ShowResource,
    UpdateResource,
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
