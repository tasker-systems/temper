// crates/temper-core/src/types/ids.rs

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        #[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
        #[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
        #[cfg_attr(any(feature = "mcp", feature = "scenario-schema"), derive(schemars::JsonSchema))]
        // Inline the newtype in MCP tool schemas. As a named type schemars would otherwise emit
        // a `$ref` into `$defs`, and a `$ref` reaches the Anthropic tool-use layer with no type
        // signal and comes back as `null` (same bug fixed for the scalar enums in `types::graph`).
        // Inlining emits `{"type":"string","format":"uuid"}` directly at the field.
        #[cfg_attr(feature = "mcp", schemars(inline))]
        pub struct $name(pub Uuid);

        impl $name {
            /// Create a new time-sortable UUIDv7 ID.
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            /// Access the inner UUID by reference.
            pub fn as_uuid(&self) -> &Uuid {
                &self.0
            }

            /// The underlying `Uuid` by value (for the sqlx-bind boundary).
            pub fn uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl From<Uuid> for $name {
            fn from(uuid: Uuid) -> Self {
                Self(uuid)
            }
        }

        impl From<$name> for Uuid {
            fn from(id: $name) -> Uuid {
                id.0
            }
        }

        impl std::ops::Deref for $name {
            type Target = Uuid;
            fn deref(&self) -> &Uuid {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }

        impl sqlx::Type<sqlx::Postgres> for $name {
            fn type_info() -> sqlx::postgres::PgTypeInfo {
                <Uuid as sqlx::Type<sqlx::Postgres>>::type_info()
            }

            fn compatible(ty: &sqlx::postgres::PgTypeInfo) -> bool {
                <Uuid as sqlx::Type<sqlx::Postgres>>::compatible(ty)
            }
        }

        impl<'q> sqlx::Encode<'q, sqlx::Postgres> for $name {
            fn encode_by_ref(
                &self,
                buf: &mut sqlx::postgres::PgArgumentBuffer,
            ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
                self.0.encode_by_ref(buf)
            }
        }

        impl sqlx::Decode<'_, sqlx::Postgres> for $name {
            fn decode(
                value: sqlx::postgres::PgValueRef<'_>,
            ) -> Result<Self, sqlx::error::BoxDynError> {
                Ok(Self(<Uuid as sqlx::Decode<'_, sqlx::Postgres>>::decode(value)?))
            }
        }

        // Array support so `Vec<$name>`/`&[$name]` bind to `= ANY($1)` / `uuid[]` columns, exactly as a
        // bare `Uuid` does. (Substrate's prior `#[sqlx(transparent)]` derive supplied this; the
        // hand-written impls above must too, or typed id arrays would not bind.)
        impl sqlx::postgres::PgHasArrayType for $name {
            fn array_type_info() -> sqlx::postgres::PgTypeInfo {
                <Uuid as sqlx::postgres::PgHasArrayType>::array_type_info()
            }

            fn array_compatible(ty: &sqlx::postgres::PgTypeInfo) -> bool {
                <Uuid as sqlx::postgres::PgHasArrayType>::array_compatible(ty)
            }
        }
    };
}

define_id!(
    /// A `kb_contexts.id` value.
    ContextId
);

define_id!(
    /// A `kb_doc_types.id` value.
    DocTypeId
);

define_id!(
    /// A `kb_events.id` value. Always UUIDv7 (time-sortable).
    EventId
);

define_id!(
    /// A `kb_resources.id` value.
    ResourceId
);

define_id!(
    /// A `kb_profiles.id` value.
    ProfileId
);

define_id!(
    /// A `kb_cogmaps.id` value — a cognitive map.
    CogmapId
);

define_id!(
    /// A `kb_edges.id` value — a declared relationship assertion.
    ///
    /// Returned by `Backend::assert_relationship` and fed back into
    /// retype/reweight/fold. Post-WS6-flip there is a single substrate-backed
    /// backend, so this is always a real `kb_edges` row id (not a backend-opaque
    /// correlation handle).
    EdgeId
);

define_id!(
    /// A `kb_resource_audits.id` value.
    ResourceAuditId
);

define_id!(
    /// A `kb_resource_revisions.id` value. Always UUIDv7 (time-sortable).
    RevisionId
);

define_id!(
    /// A `kb_content_blocks.id` value — a resource's addressable interior unit.
    BlockId
);

define_id!(
    /// A `kb_entities.id` value — the event emitter (agent instance / integration).
    EntityId
);

define_id!(
    /// A `kb_cogmap_lenses.id` value.
    LensId
);

define_id!(
    /// A `kb_chunks.id` value — one embedding window of a block's prose.
    ChunkId
);

define_id!(
    /// A `kb_properties.id` value — a facet/doc_type/block_role assertion.
    PropertyId
);

define_id!(
    /// A `kb_cogmap_regions.id` value — one materialized region.
    RegionId
);

define_id!(
    /// A `kb_invocations.id` value — an agentic-workflow run (accountability grain).
    InvocationId
);

define_id!(
    /// A `kb_events.correlation_id` value — the **act** grain: the thread stitching the writes of
    /// one logical act, possibly spanning processes and credentials.
    ///
    /// Distinct from [`InvocationId`], which is *run*-grain and agent-shaped (trigger kind,
    /// originating cogmap, delegated launch) — it models an agent working-session envelope. A web
    /// request is not an agent run, but "publish this postmortem" spanning a request and the
    /// background job it enqueued is one act. This is a bare UUID minted by the caller, so it
    /// serializes into job arguments and outlives any credential.
    ///
    /// Correlation is a correlation aid, **never** authorization — nothing gates on it. Unlike the
    /// other ids here it names no row of its own: an event with no supplied correlation self-roots
    /// (`correlation_id` = its own event id), per `_event_append`'s root-event convention.
    CorrelationId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newtype_roundtrips_through_uuid_and_serde() {
        let u = Uuid::now_v7();
        let r = ResourceId::from(u);
        // From<Uuid> / Into<Uuid> / uuid() / as_uuid() / Deref all agree.
        assert_eq!(r.uuid(), u);
        assert_eq!(Uuid::from(r), u);
        assert_eq!(r.0, u);
        assert_eq!(*r.as_uuid(), u);
        assert_eq!(*r, u);
        // Display matches the raw uuid.
        assert_eq!(r.to_string(), u.to_string());
        // serde round-trips as a bare uuid string (transparent on the wire).
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, serde_json::to_string(&u).unwrap());
        assert_eq!(serde_json::from_str::<ResourceId>(&json).unwrap(), r);
    }

    #[test]
    fn distinct_newtypes_stay_independent_and_orderable() {
        let u = Uuid::now_v7();
        // CogmapId and ResourceId cannot be compared with `==` — different types — which is the point.
        assert_eq!(CogmapId::from(u).uuid(), ResourceId::from(u).uuid());
        // Ord is derived: sorting a vec of ids compiles and matches uuid order.
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        let mut v = vec![ResourceId::from(b), ResourceId::from(a)];
        v.sort();
        assert_eq!(
            v,
            vec![ResourceId::from(a.min(b)), ResourceId::from(a.max(b))]
        );
    }
}
