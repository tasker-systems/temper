//! Managed (`temper-*`) frontmatter — re-exported from [`temper_core::types::managed_meta`],
//! plus the one shape in this family that cannot live there.
//!
//! [`ManagedMeta`] and friends moved to temper-core because `ManagedMeta` is a field of
//! `ResourceView`, which temper-substrate — the layer *below* this crate — reads back
//! directly. Every `temper_workflow::types::managed_meta::…` call site resolves unchanged.
//!
//! [`ResourceMetaListResponse`] stays here: its rows and facets are
//! [`ResourceDetail`](crate::types::resource::ResourceDetail) and
//! [`ResourceFacets`](crate::types::resource::ResourceFacets), which are temper-workflow
//! types. Moving it would drag the whole row family down a layer for no gain.

use serde::{Deserialize, Serialize};

pub use temper_core::types::managed_meta::{
    ManagedMeta, MetaUpdatePayload, ResourceManifestRow, ResourceMetaResponse,
    PROVENANCE_LLM_DISCOVERED, PROVENANCE_USER_CREATED,
};

/// Paginated response for the `?meta_only=true` list mode.
///
/// Mirror of [`crate::types::resource::ResourceListResponse`] with the row type
/// swapped to [`crate::types::resource::ResourceDetail`] — the full row (title,
/// doc_type, context, owner, the stage/seq/mode/effort projections) plus both
/// `managed_meta` and `open_meta` tiers. This is the list analogue of what
/// `--meta-only` gives on a single `show`: everything the default list row carries
/// **and** both meta tiers, per item — the whole view minus each resource's body.
/// (The default list row, [`crate::types::resource::ResourceRow`], carries neither
/// meta tier.) Returned by `GET /api/resources?meta_only=true`. Facets and total
/// are computed identically to the default list response — projection-independent.
///
/// No `ts_rs::TS` derive: `ResourceDetail` cannot codegen (its `#[serde(flatten)]`
/// row is unsupported by ts-rs), so this container cannot either. The SvelteKit UI
/// types the list endpoint as `ResourceListResponse` regardless; this shape is a
/// structural superset, so the extra keys are simply ignored there.
// No `schemars::JsonSchema` (mcp): `ResourceDetail` doesn't derive it, and this is
// not an MCP tool type — MCP serves orientation via `EnrichedResource`, never this.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ResourceMetaListResponse {
    pub rows: Vec<crate::types::resource::ResourceDetail>,
    pub total: i64,
    pub facets: crate::types::resource::ResourceFacets,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::resource::ResourceFacets;
    use std::collections::HashMap;

    #[test]
    fn resource_meta_list_response_roundtrip() {
        let response = ResourceMetaListResponse {
            rows: vec![],
            total: 0,
            facets: ResourceFacets {
                doc_type: HashMap::new(),
            },
        };
        let json = serde_json::to_value(&response).expect("serialize");
        let back: ResourceMetaListResponse =
            serde_json::from_value(json.clone()).expect("deserialize");
        assert_eq!(back.total, 0);
        assert!(back.rows.is_empty());
        assert_eq!(json["rows"], serde_json::json!([]));
        assert_eq!(json["total"], 0);
        assert!(json["facets"].is_object());
    }
}
