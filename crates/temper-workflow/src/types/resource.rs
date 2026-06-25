//! Resource API types — shared between temper-api and temper-client.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use super::managed_meta::ManagedMeta;
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};

/// Row type for resource listings — includes joined display fields
/// and managed_meta projections from `vault_resources_browse` view.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceRow {
    pub id: ResourceId,
    pub kb_context_id: ContextId,
    pub origin_uri: String,
    pub title: String,
    pub originator_profile_id: ProfileId,
    pub owner_profile_id: ProfileId,
    pub is_active: bool,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    // Joined display fields
    pub context_name: String,
    pub doc_type_name: String,
    pub owner_handle: String,
    // Managed meta projections
    pub stage: Option<String>,
    #[cfg_attr(feature = "typescript", ts(type = "number | null"))]
    pub seq: Option<i64>,
    pub mode: Option<String>,
    pub effort: Option<String>,
    /// SHA-256 hash of the resource body content, from `kb_resource_manifests`.
    /// `None` when no manifest row exists (resource created via POST without a
    /// body trio, or the manifest join returned NULL).
    pub body_hash: Option<String>,
}

/// Sort field for resource listing.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ResourceSortField {
    #[default]
    Updated,
    Created,
    Title,
    Stage,
    Seq,
    ContextName,
    DocTypeName,
}

/// Sort direction.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    #[default]
    Desc,
    Asc,
}

/// Query parameters for listing visible resources.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceListParams {
    pub kb_context_id: Option<Uuid>,
    pub kb_doc_type_id: Option<Uuid>,
    pub context_name: Option<String>,
    pub doc_type_name: Option<String>,
    pub owner: Option<String>,
    pub q: Option<String>,
    pub stage: Option<String>,
    pub sort: Option<ResourceSortField>,
    pub order: Option<SortOrder>,
    #[cfg_attr(feature = "typescript", ts(type = "number | null"))]
    pub limit: Option<i64>,
    #[cfg_attr(feature = "typescript", ts(type = "number | null"))]
    pub offset: Option<i64>,
    /// When true, the list endpoint returns `ResourceMetaListResponse`
    /// (`Vec<ResourceMetaResponse>` rows) instead of `ResourceListResponse`
    /// (`Vec<ResourceRow>` rows). Default: false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta_only: Option<bool>,
}

/// Aggregated doc-type facet counts for the current filter set.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceFacets {
    pub doc_type: std::collections::HashMap<String, i64>,
}

/// Paginated response for resource list endpoints, with doc-type facets.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceListResponse {
    pub rows: Vec<ResourceRow>,
    pub total: i64,
    pub facets: ResourceFacets,
}

/// Request body for creating a resource.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceCreateRequest {
    pub kb_context_id: Uuid,
    /// Doc-type name (the substrate stores doc-type as a property name; the
    /// backend create path passes it straight through to `CreateResource`).
    pub doc_type: String,
    pub origin_uri: String,
    pub title: String,
    pub slug: Option<String>,
}

/// Request body for updating a resource.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceUpdateRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    /// Partial managed_meta — only fields with `Some` apply.
    /// Untouched fields preserve their stored value. There is no in-band
    /// signal for "clear this field"; field-clearing is reserved for a
    /// future PUT endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_meta: Option<ManagedMeta>,
    /// Partial open_meta — incoming keys win; absent keys preserved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_meta: Option<serde_json::Value>,
    /// New body markdown. Required iff `content_hash` and `chunks_packed`
    /// are also `Some` (all-or-nothing trio).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// SHA-256 hash of `content`. Required iff `content` is `Some`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// Pre-computed chunks (base64-encoded MessagePack). Required iff
    /// `content` is `Some`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunks_packed: Option<String>,
}

/// Chunk used to reconstitute markdown content.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ContentChunk {
    pub chunk_index: i32,
    pub header_path: String,
    pub heading_depth: i16,
    pub content: String,
}

/// Response body for resource content.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ContentResponse {
    pub resource_id: ResourceId,
    pub markdown: String,
    /// Typed server-side managed_meta from kb_resource_manifests. The
    /// typed fields name everything temper knows about; any extras the
    /// server stored round-trip through `ManagedMeta::extra`.
    /// Used by CLI sync pull to reconstruct complete frontmatter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_meta: Option<ManagedMeta>,
    /// Server-side open_meta from kb_resource_manifests. Intentionally
    /// untyped — open_meta is the free-form tier. Typed extraction of
    /// relationship fields lives in `ResourceRelationships` (see
    /// `types::graph`), which parses this value on demand and ignores
    /// anything it doesn't recognize.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_meta: Option<serde_json::Value>,
}

/// Response body for resource deletion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct DeleteResponse {
    pub deleted: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ContentResponse` with both managed_meta and open_meta populated
    /// must roundtrip through serde cleanly, preserving both tiers.
    #[test]
    fn content_response_roundtrips_with_both_meta_tiers() {
        let resource_id = ResourceId::from(Uuid::nil());
        let managed_meta = ManagedMeta {
            stage: Some("draft".to_string()),
            ..Default::default()
        };
        let open_meta = serde_json::json!({
            "tags": ["one", "two"],
            "related": ["res_999"],
        });
        let original = ContentResponse {
            resource_id,
            markdown: "# Hello".to_string(),
            managed_meta: Some(managed_meta.clone()),
            open_meta: Some(open_meta.clone()),
        };

        let json = serde_json::to_value(&original).expect("serialize");
        let roundtrip: ContentResponse = serde_json::from_value(json.clone()).expect("deserialize");

        assert_eq!(roundtrip.markdown, "# Hello");
        assert_eq!(roundtrip.managed_meta, Some(managed_meta));
        assert_eq!(roundtrip.open_meta, Some(open_meta));
    }

    /// `ContentResponse` with `open_meta: None` must omit the field
    /// entirely from the serialized JSON (not emit `"open_meta": null`),
    /// matching the `skip_serializing_if = "Option::is_none"` contract
    /// used by `managed_meta`. This preserves wire compatibility with
    /// older clients that don't know about `open_meta`.
    #[test]
    fn content_response_omits_open_meta_when_none() {
        let resource_id = ResourceId::from(Uuid::nil());
        let original = ContentResponse {
            resource_id,
            markdown: "body".to_string(),
            managed_meta: None,
            open_meta: None,
        };

        let json = serde_json::to_value(&original).expect("serialize");
        let obj = json.as_object().expect("object");

        assert!(
            !obj.contains_key("open_meta"),
            "open_meta should be omitted when None, got: {json}"
        );
        assert!(
            !obj.contains_key("managed_meta"),
            "managed_meta should be omitted when None, got: {json}"
        );
    }

    /// Old clients (no `open_meta` field in their request) must still
    /// deserialize a `ContentResponse` from servers that omit it.
    /// This is the `#[serde(default)]` contract.
    #[test]
    fn content_response_deserializes_without_open_meta() {
        let json = serde_json::json!({
            "resource_id": Uuid::nil(),
            "markdown": "hi",
        });

        let parsed: ContentResponse = serde_json::from_value(json).expect("deserialize");
        assert_eq!(parsed.markdown, "hi");
        assert!(parsed.managed_meta.is_none());
        assert!(parsed.open_meta.is_none());
    }

    #[test]
    fn resource_update_request_serde_round_trips_with_all_fields() {
        use serde_json::json;
        let req = ResourceUpdateRequest {
            title: Some("New Title".to_string()),
            slug: Some("new-slug".to_string()),
            managed_meta: Some(ManagedMeta {
                stage: Some("done".to_string()),
                ..Default::default()
            }),
            open_meta: Some(json!({"tags": ["rust"]})),
            content: Some("# Body\n".to_string()),
            content_hash: Some("sha256:abc".to_string()),
            chunks_packed: Some("base64-blob".to_string()),
        };
        let serialized = serde_json::to_string(&req).unwrap();
        let parsed: ResourceUpdateRequest = serde_json::from_str(&serialized).unwrap();
        assert_eq!(parsed.title.as_deref(), Some("New Title"));
        assert_eq!(parsed.slug.as_deref(), Some("new-slug"));
        assert_eq!(
            parsed
                .managed_meta
                .as_ref()
                .and_then(|m| m.stage.as_deref()),
            Some("done")
        );
        assert_eq!(parsed.open_meta, Some(json!({"tags": ["rust"]})));
        assert_eq!(parsed.content.as_deref(), Some("# Body\n"));
        assert_eq!(parsed.content_hash.as_deref(), Some("sha256:abc"));
        assert_eq!(parsed.chunks_packed.as_deref(), Some("base64-blob"));
    }

    #[test]
    fn resource_update_request_omits_none_fields_on_serialize() {
        let req = ResourceUpdateRequest {
            title: None,
            slug: None,
            managed_meta: Some(ManagedMeta {
                stage: Some("done".to_string()),
                ..Default::default()
            }),
            open_meta: None,
            content: None,
            content_hash: None,
            chunks_packed: None,
        };
        let serialized = serde_json::to_string(&req).unwrap();
        assert!(!serialized.contains("\"title\""));
        assert!(!serialized.contains("\"content\""));
        assert!(serialized.contains("\"managed_meta\""));
    }
}
