//! Command structs — declarative intent for a single operation.
//!
//! Commands are pure data. Surfaces translate user input (clap args, MCP
//! tool params, HTTP body) into a command; backends consume commands and
//! emit `CommandOutput`.
//!
//! Resource-action commands (`Show`, `Update`, `Delete`) carry a `ResourceRef`,
//! not a bare slug — slug uniqueness is scoped to (owner, context, doctype),
//! UUID is globally unique. `Create` carries the new resource's identity
//! directly. `List` and `Search` carry filter/query inputs.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::managed_meta::ManagedMeta;

use super::{
    inputs::{BodyUpdate, ListFilter, SearchQuery},
    resource_ref::ResourceRef,
    surface::Surface,
};

/// Create a new resource. The resource's identity is specified directly
/// (not via `ResourceRef`) since it doesn't exist yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateResource {
    pub slug: String,
    pub doctype: String,
    pub context: String,
    pub title: String,
    pub body: Option<BodyUpdate>,
    /// Caller-supplied managed_meta. Backends will apply doctype defaults
    /// for any required fields the caller omitted.
    pub managed_meta: ManagedMeta,
    /// Free-form user metadata (open_meta tier).
    pub open_meta: Option<Value>,
    /// Canonical resource URI for dedup and storage. Callers that derive a
    /// `kb://`-scheme URI from context/doctype/slug should set this; callers
    /// without a pre-computed URI may leave it `None` and the server will
    /// store an empty string (the existing behavior for the HTTP create path).
    pub origin_uri: Option<String>,
    /// Pre-computed chunks (base64-encoded MessagePack) supplied by the caller.
    /// When `Some`, passed through to `ingest_service::ingest` so the server
    /// does not need to run the embed pipeline. When `None`, the server
    /// recomputes via the pipeline (if enabled) or returns an error for
    /// non-empty bodies without the pipeline feature.
    pub chunks_packed: Option<String>,
    /// Caller-supplied content hash. Sync clients pre-compute this so the
    /// canonical body_hash round-trips verbatim into `kb_resource_audits`
    /// and the manifest. When `None`, the server recomputes from `body`.
    pub content_hash: Option<String>,
    pub origin: Surface,
}

/// Show a single resource.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShowResource {
    pub resource: ResourceRef,
    pub origin: Surface,
}

/// File-move spec for `UpdateResource`. Both fields are optional and
/// independent; supplying either (or both) triggers a filesystem move.
///
/// - `context_to`: move the file to a new context directory and update
///   `temper-context` in frontmatter. The DB backend ignores this field —
///   the new context is communicated via `managed_meta.context`.
/// - `type_to`: move the file to a new doc-type directory and update
///   `temper-type` in frontmatter. Likewise ignored by DbBackend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoveSpec {
    pub context_to: Option<String>,
    pub type_to: Option<String>,
}

/// Update a resource — partial; any combination of body, managed_meta,
/// open_meta may be supplied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateResource {
    pub resource: ResourceRef,
    pub body: Option<BodyUpdate>,
    pub managed_meta: Option<ManagedMeta>,
    pub open_meta: Option<Value>,
    /// File-move spec (VaultBackend only). `DbBackend` ignores `move_to` —
    /// the new context/type is conveyed via `managed_meta` which DbBackend
    /// already handles. This field carries no SQL and does not affect the
    /// `.sqlx/` query cache.
    pub move_to: Option<MoveSpec>,
    pub origin: Surface,
}

/// Delete a resource. In the cloud-first model this is a soft-delete on the
/// server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeleteResource {
    pub resource: ResourceRef,
    /// Bypass the local-file confirmation prompt (required for non-TTY).
    pub force: bool,
    pub origin: Surface,
}

/// List resources, filtered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListResources {
    pub filter: ListFilter,
    pub origin: Surface,
}

/// Semantic / hybrid search.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResources {
    pub query: SearchQuery,
    pub origin: Surface,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_resource_carries_resource_ref() {
        let cmd = ShowResource {
            resource: ResourceRef::scoped("@me", "temper", "task", "hello"),
            origin: Surface::CliCloud,
        };
        // Exercises the type compiles + the field is reachable.
        assert!(matches!(cmd.resource, ResourceRef::Scoped { .. }));
    }

    #[test]
    fn update_resource_all_optional_fields_default_none() {
        let cmd = UpdateResource {
            resource: ResourceRef::scoped("@me", "temper", "task", "x"),
            body: None,
            managed_meta: None,
            open_meta: None,
            move_to: None,
            origin: Surface::ApiHttp,
        };
        assert!(cmd.body.is_none());
        assert!(cmd.managed_meta.is_none());
        assert!(cmd.open_meta.is_none());
        assert!(cmd.move_to.is_none());
    }

    #[test]
    fn create_resource_does_not_use_resource_ref() {
        let cmd = CreateResource {
            slug: "new-task".to_string(),
            doctype: "task".to_string(),
            context: "temper".to_string(),
            title: "New task".to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            origin_uri: None,
            chunks_packed: None,
            content_hash: None,
            origin: Surface::CliCloud,
        };
        assert_eq!(cmd.slug, "new-task");
    }

    #[test]
    fn list_resources_default_filter_serializes() {
        let cmd = ListResources {
            filter: ListFilter::default(),
            origin: Surface::Mcp,
        };
        let s = serde_json::to_string(&cmd).unwrap();
        let back: ListResources = serde_json::from_str(&s).unwrap();
        assert_eq!(cmd, back);
    }
}
