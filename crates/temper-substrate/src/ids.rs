//! Typed-UUID newtypes — the convergence-guidance foundation (scenario-DSL roadmap deliverable 6,
//! carried in-band). Bare `Uuid` mixes `resource`/`cogmap`/`event`/… identities at every call
//! boundary; these transparent newtypes make a mis-passed id a compile error. They encode/decode as a
//! plain `Uuid` (`#[sqlx(transparent)]`), so they cost nothing at the SQL-bind boundary — bind `id.0`.
//!
//! Introduced here as the substrate the Rust fire-event action (`SeedAction`/`fire`) threads through;
//! this crate's content path binds raw `Uuid` at the sqlx edge and uses these at typed surfaces.

use uuid::Uuid;

/// Define a transparent UUID newtype with the standard derive set + ergonomic conversions.
macro_rules! id_newtype {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord,
            serde::Serialize, serde::Deserialize, sqlx::Type,
        )]
        #[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
        #[sqlx(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            /// The underlying `Uuid` (for the sqlx-bind boundary).
            pub fn uuid(self) -> Uuid {
                self.0
            }
        }

        impl From<Uuid> for $name {
            fn from(u: Uuid) -> Self {
                $name(u)
            }
        }

        impl From<$name> for Uuid {
            fn from(id: $name) -> Uuid {
                id.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                std::fmt::Display::fmt(&self.0, f)
            }
        }
    };
}

id_newtype!(
    /// A `kb_resources` row (the named, edge-connected, findable unit).
    ResourceId
);
id_newtype!(
    /// A `kb_cogmaps` row.
    CogmapId
);
id_newtype!(
    /// A `kb_contexts` row (the Domain-A navigation/share anchor).
    ContextId
);
id_newtype!(
    /// A `kb_content_blocks` row (a resource's addressable interior unit).
    BlockId
);
id_newtype!(
    /// A `kb_profiles` row (the owner/actor principal).
    ProfileId
);
id_newtype!(
    /// A `kb_entities` row (the event emitter — agent instance / integration).
    EntityId
);
id_newtype!(
    /// A `kb_events` row (a ledger event).
    EventId
);
id_newtype!(
    /// A `kb_cogmap_lenses` row.
    LensId
);
id_newtype!(
    /// A `kb_chunks` row (one embedding window of a block's prose).
    ChunkId
);
id_newtype!(
    /// A `kb_edges` row (a declared relationship assertion).
    EdgeId
);
id_newtype!(
    /// A `kb_properties` row (a facet/doc_type/block_role assertion).
    PropertyId
);
id_newtype!(
    /// A `kb_cogmap_regions` row (one materialized region).
    RegionId
);
id_newtype!(
    /// A `kb_invocations` row (an agentic-workflow run, accountability grain).
    InvocationId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newtypes_roundtrip_through_uuid_and_serde() {
        let u = Uuid::now_v7();
        let r = ResourceId::from(u);
        // From<Uuid> / Into<Uuid> / uuid() all agree.
        assert_eq!(r.uuid(), u);
        assert_eq!(Uuid::from(r), u);
        assert_eq!(r.0, u);
        // Display matches the raw uuid.
        assert_eq!(r.to_string(), u.to_string());
        // serde round-trips as a bare uuid string (transparent on the wire too).
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, serde_json::to_string(&u).unwrap());
        assert_eq!(serde_json::from_str::<ResourceId>(&json).unwrap(), r);
    }

    #[test]
    fn distinct_newtypes_are_distinct_types() {
        // A compile-time guarantee in spirit; here we assert the values stay independent.
        let u = Uuid::now_v7();
        let cogmap = CogmapId::from(u);
        let resource = ResourceId::from(u);
        assert_eq!(cogmap.uuid(), resource.uuid());
        // (CogmapId and ResourceId cannot be compared with == — different types — which is the point.)
    }
}
