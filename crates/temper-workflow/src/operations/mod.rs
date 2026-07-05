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
mod refs;
mod surface;

pub use actions::{
    apply_defaults, apply_defaults_value, assemble_frontmatter_document,
    ensure_managed_identity_keys, merge_managed_meta, merge_open_meta, strip_system_managed_fields,
    validate_create, validate_doctype, validate_managed_meta, validate_open_meta_keys,
    validate_slug, validate_update, ActionError, FrontmatterIdentity, ValidateManagedMetaParams,
};
pub use backend::{Backend, ResourceSummary, SearchHit};
pub use commands::{
    AdvanceStewardWatermark, AssertRelationship, CloseInvocation, CreateCognitiveMap,
    CreateResource, DeleteResource, FoldRelationship, ListResources, MaterializeOnThreshold,
    MoveSpec, OpenInvocation, ReconcileCognitiveMap, RetypeRelationship, ReweightRelationship,
    SearchResources, SetFacet, ShowResource, StewardDispatchTick, UpdateResource,
};
pub use events::{DomainEvent, PushDeferReason};
pub use inputs::{BodyUpdate, ListFilter, SearchQuery};
pub use output::CommandOutput;
pub use refs::{decorated_ref, parse_ref, resolve_provenance_source, sluggify};
pub use surface::Surface;

#[cfg(test)]
mod smoke {
    /// Smoke test: the module compiles and is reachable.
    #[test]
    fn module_exists() {
        // No-op; existence of this test passing means the module compiled.
    }
}
