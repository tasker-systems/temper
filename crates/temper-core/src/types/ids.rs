// crates/temper-core/src/types/ids.rs

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        #[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
        #[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
        #[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
        pub struct $name(pub Uuid);

        impl $name {
            /// Create a new time-sortable UUIDv7 ID.
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            /// Access the inner UUID.
            pub fn as_uuid(&self) -> &Uuid {
                &self.0
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
    /// A `kb_resource_audits.id` value.
    ResourceAuditId
);

define_id!(
    /// A `kb_resource_revisions.id` value. Always UUIDv7 (time-sortable).
    RevisionId
);
