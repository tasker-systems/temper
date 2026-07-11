//! The home of a resource: exactly one of a context or a cognitive map.
//! Parse-don't-validate: surfaces resolve a ref into one variant before
//! building a `CreateResource` command — never a placeholder id plus a flag.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::ids::{CogmapId, ContextId};

/// `Copy`/`Eq`/`Hash`: this is two ids in a trench coat, and the region producer threads it through
/// hot loops and keys maps on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HomeAnchor {
    Context(ContextId),
    Cogmap(CogmapId),
}

impl HomeAnchor {
    /// The `anchor_table` / `home_anchor_table` SQL discriminant, spelled exactly as the DDL spells
    /// it. Deriving the literal here rather than at each call site is the whole reason this type
    /// exists.
    pub fn table(self) -> &'static str {
        match self {
            HomeAnchor::Context(_) => "kb_contexts",
            HomeAnchor::Cogmap(_) => "kb_cogmaps",
        }
    }

    /// The bare UUID, for binding alongside [`HomeAnchor::table`].
    pub fn uuid(self) -> Uuid {
        match self {
            HomeAnchor::Context(c) => c.uuid(),
            HomeAnchor::Cogmap(m) => m.uuid(),
        }
    }

    /// The cogmap id, or `None` for a context anchor. This is the *vestigial* `cogmap_id` column the
    /// region tables dual-write through the expand window (spec §3.6 M1) — not an accessor to reach
    /// for in new logic.
    pub fn cogmap_id(self) -> Option<CogmapId> {
        match self {
            HomeAnchor::Cogmap(m) => Some(m),
            HomeAnchor::Context(_) => None,
        }
    }

    /// Reconstruct from a `(table, id)` row pair. `None` on an unrecognized discriminant, so the call
    /// site escalates rather than silently defaulting to the wrong anchor kind.
    pub fn from_parts(table: &str, id: Uuid) -> Option<Self> {
        match table {
            "kb_contexts" => Some(HomeAnchor::Context(ContextId::from(id))),
            "kb_cogmaps" => Some(HomeAnchor::Cogmap(CogmapId::from(id))),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ids::{CogmapId, ContextId};

    #[test]
    fn table_and_uuid_round_trip_through_from_parts() {
        let c = HomeAnchor::Context(ContextId::new());
        assert_eq!(c.table(), "kb_contexts");
        assert_eq!(HomeAnchor::from_parts(c.table(), c.uuid()), Some(c));
        assert_eq!(c.cogmap_id(), None);

        let id = CogmapId::new();
        let m = HomeAnchor::Cogmap(id);
        assert_eq!(m.table(), "kb_cogmaps");
        assert_eq!(HomeAnchor::from_parts(m.table(), m.uuid()), Some(m));
        assert_eq!(m.cogmap_id(), Some(id));

        // An unrecognized discriminant escalates rather than defaulting to an anchor kind.
        assert_eq!(HomeAnchor::from_parts("kb_teams", Uuid::nil()), None);
    }

    #[test]
    fn home_anchor_serde_roundtrip() {
        let c = HomeAnchor::Context(ContextId::new());
        let j = serde_json::to_string(&c).unwrap();
        assert_eq!(c, serde_json::from_str(&j).unwrap());
        let m = HomeAnchor::Cogmap(CogmapId::new());
        let j = serde_json::to_string(&m).unwrap();
        assert_eq!(m, serde_json::from_str(&j).unwrap());
    }
}
