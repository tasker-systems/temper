//! How the reading principal reaches an object it reads: [`Reach`], carried by the single-object
//! reads (`GET /api/contexts/{id}`, `GET /api/resources/{id}`).

use serde::{Deserialize, Serialize};

/// The route by which the reader reaches the object.
///
/// A route, never a source: it names neither the team that opened the object nor anyone who did,
/// and it is answered only on a read the reader already passed, so it says nothing about an object
/// the reader cannot read.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "reach.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReachVia {
    /// Any reach that does not come through public_within: the reader owns the object, is a
    /// member of a team that reaches it (its own or a descendant's closure), or holds a share or a
    /// grant.
    Membership,
    /// The reader reads the object only because a team opened its owned contexts to the reader's
    /// part of the team tree. Read and reference only: no act on the object is admitted this way.
    PublicWithin,
}

impl ReachVia {
    /// The SQL spelling: what `context_reached_via` and `resource_reached_via` return.
    pub fn from_sql(s: &str) -> Option<Self> {
        match s {
            "membership" => Some(Self::Membership),
            "public_within" => Some(Self::PublicWithin),
            _ => None,
        }
    }
}

/// How the reader reaches the object. An object of its own, not a bare field, so a later answer
/// can grow beside `via` without moving it.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "reach.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reach {
    /// `public_within` when the reader reads the object and does not read it by membership;
    /// `membership` otherwise.
    pub via: ReachVia,
}

/// Decoded from the `text` the reach functions return, so a `query_as!` row can carry it.
impl sqlx::Type<sqlx::Postgres> for Reach {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }

    fn compatible(ty: &sqlx::postgres::PgTypeInfo) -> bool {
        <String as sqlx::Type<sqlx::Postgres>>::compatible(ty)
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for Reach {
    fn decode(value: sqlx::postgres::PgValueRef<'r>) -> Result<Self, sqlx::error::BoxDynError> {
        let s = <&str as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        let via = ReachVia::from_sql(s).ok_or_else(|| format!("unknown reach route {s:?}"))?;
        Ok(Self { via })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wire_and_sql_spellings_agree() {
        for via in [ReachVia::Membership, ReachVia::PublicWithin] {
            let wire = serde_json::to_value(via).expect("serialize");
            let wire = wire.as_str().expect("a string");
            assert_eq!(ReachVia::from_sql(wire), Some(via));
        }
        assert_eq!(
            serde_json::to_value(Reach {
                via: ReachVia::PublicWithin
            })
            .expect("serialize"),
            serde_json::json!({ "via": "public_within" })
        );
        assert_eq!(ReachVia::from_sql("granted_by_someone"), None);
    }
}
