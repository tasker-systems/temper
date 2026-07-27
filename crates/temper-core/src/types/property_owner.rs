//! Who owns a property row.
//!
//! `kb_properties.owner_table` is CHECK-constrained to four tables, but only two of them have a
//! write path through `facet_set` / `property_set`: resources, and — since the edge-owned-properties
//! work — edges. (`kb_content_blocks` properties are written by `_project_blocks` directly;
//! `kb_cogmaps` has never had one.) This enum is exactly those two, so an owner kind the write path
//! cannot serve is a **compile error at the call site** rather than a runtime `RAISE` from
//! `_property_owner_anchor` — the "no stringly-typed matches over bounded sets" rule applied to the
//! owner axis.
//!
//! The two arms differ in more than their id type, which is why one `Uuid` with a table string
//! would be the wrong shape: they authorize differently. A resource-owned facet gates on
//! `can_modify_resource`; an edge-owned facet gates on the edge's own mutability clauses (the
//! source resource *and* container-write on the edge's home). Keeping the owner typed is what makes
//! the dispatch to the right gate exhaustive.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::ids::{EdgeId, ResourceId};

/// The owner of a facet/property write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub enum PropertyOwner {
    /// A `kb_resources` row — the incumbent owner, and every property in production before this.
    Resource { id: ResourceId },
    /// A `kb_edges` row — a facet qualifying a *relationship* rather than a thing. The canonical
    /// DDL has admitted this since the beginning ("§4a edges carry facets"); nothing wrote one
    /// until the write path was built.
    Edge { id: EdgeId },
}

impl PropertyOwner {
    /// The `kb_properties.owner_table` value, spelled exactly as the DDL spells it. This is the
    /// string `_property_owner_anchor` dispatches on, so the two must agree.
    pub fn owner_table(&self) -> &'static str {
        match self {
            PropertyOwner::Resource { .. } => "kb_resources",
            PropertyOwner::Edge { .. } => "kb_edges",
        }
    }

    /// The owner row's id, untyped — for binding into a query or a payload. Prefer matching on the
    /// enum where the *kind* matters; this is for the places that genuinely only need the uuid.
    pub fn uuid(&self) -> Uuid {
        match self {
            PropertyOwner::Resource { id } => id.uuid(),
            PropertyOwner::Edge { id } => id.uuid(),
        }
    }

    /// Convenience constructor for the common case.
    pub fn resource(id: ResourceId) -> Self {
        PropertyOwner::Resource { id }
    }

    /// Convenience constructor for an edge owner.
    pub fn edge(id: EdgeId) -> Self {
        PropertyOwner::Edge { id }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_table_matches_the_ddl_spelling() {
        // These strings are the dispatch keys `_property_owner_anchor` matches on. If either side
        // is renamed without the other, every write for that owner kind raises at runtime — so the
        // literals are pinned here rather than left to a reader to keep in step.
        assert_eq!(
            PropertyOwner::resource(ResourceId::new()).owner_table(),
            "kb_resources"
        );
        assert_eq!(PropertyOwner::edge(EdgeId::new()).owner_table(), "kb_edges");
    }

    #[test]
    fn uuid_survives_the_wrapper_in_both_arms() {
        let r = ResourceId::new();
        let e = EdgeId::new();
        assert_eq!(PropertyOwner::resource(r).uuid(), r.uuid());
        assert_eq!(PropertyOwner::edge(e).uuid(), e.uuid());
    }

    #[test]
    fn round_trips_tagged_so_the_kind_survives_the_wire() {
        // Untagged would make a Resource and an Edge owner indistinguishable once serialized —
        // both are `{"id": "<uuid>"}` — and the wrong gate would authorize the write.
        for owner in [
            PropertyOwner::resource(ResourceId::new()),
            PropertyOwner::edge(EdgeId::new()),
        ] {
            let json = serde_json::to_string(&owner).expect("serializes");
            assert_eq!(
                serde_json::from_str::<PropertyOwner>(&json).expect("round-trips"),
                owner
            );
        }
    }
}
