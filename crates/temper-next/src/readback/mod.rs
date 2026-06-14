//! WS6 §9 chunk-3 read surface over `temper_next.*` — read-only parity tooling.
//!
//! This module ports the production read paths (list / meta / body / FTS / vector / neighbors) onto
//! the synthesized substrate so each can be asserted byte-for-byte against the production read for the
//! same logical query (the parity-read harness in `tests/parity_reads.rs`). This file currently carries
//! only the harness scaffold; the individual read ports land in later chunk-3 tasks.
//!
//! All reads are runtime, schema-qualified `sqlx::query` (NEVER the `query!`/`query_as!` macros), same
//! discipline as [`crate::synthesis::source`] and [`crate::synthesis::parity`]: the temper-next macro
//! cache resolves against the `temper_next` search_path, so a compile-time macro over `public.*` would
//! conflict. Qualifying every table keeps the reads correct regardless of the connection's search_path.

use std::collections::HashMap;

use anyhow::Result;
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// A bidirectional map between a production resource id (`public.kb_resources.id`, active only) and its
/// synthesized counterpart (`temper_next.kb_resources.id`), keyed by the shared `origin_uri` (carried
/// verbatim, UNIQUE in both schemas). Built once per parity read so a test can resolve a known fixture
/// resource across the two schemas.
#[derive(Debug, Clone, Default)]
pub struct ResolvedIds {
    /// production id → synthesized id.
    old_to_new: HashMap<Uuid, Uuid>,
    /// synthesized id → production id.
    new_to_old: HashMap<Uuid, Uuid>,
    /// synthesized id → its `origin_uri` (handy for later read ports).
    origin_uri_by_new: HashMap<Uuid, String>,
}

impl ResolvedIds {
    /// Load the bimap by reading `(id, origin_uri)` from `public.kb_resources WHERE is_active` and from
    /// `temper_next.kb_resources`, then joining in Rust on `origin_uri`. Only `origin_uri`s present in
    /// both schemas (the synthesized active set) become entries.
    pub async fn load(pool: &PgPool) -> Result<Self> {
        let old_by_uri: HashMap<String, Uuid> =
            sqlx::query("SELECT id, origin_uri FROM public.kb_resources WHERE is_active")
                .fetch_all(pool)
                .await?
                .iter()
                .map(|r| (r.get::<String, _>("origin_uri"), r.get::<Uuid, _>("id")))
                .collect();

        let new_rows = sqlx::query("SELECT id, origin_uri FROM temper_next.kb_resources")
            .fetch_all(pool)
            .await?;

        let mut resolved = ResolvedIds::default();
        for row in &new_rows {
            let origin_uri: String = row.get("origin_uri");
            let new_id: Uuid = row.get("id");
            let Some(&old_id) = old_by_uri.get(&origin_uri) else {
                continue;
            };
            resolved.old_to_new.insert(old_id, new_id);
            resolved.new_to_old.insert(new_id, old_id);
            resolved.origin_uri_by_new.insert(new_id, origin_uri);
        }
        Ok(resolved)
    }

    /// The synthesized id for a production id, if it was synthesized (active resources only).
    pub fn to_new(&self, public_id: Uuid) -> Option<Uuid> {
        self.old_to_new.get(&public_id).copied()
    }

    /// The production id for a synthesized id.
    pub fn to_old(&self, new_id: Uuid) -> Option<Uuid> {
        self.new_to_old.get(&new_id).copied()
    }

    /// The `origin_uri` for a synthesized id (the join key both schemas share).
    pub fn origin_uri_for_new(&self, new_id: Uuid) -> Option<&str> {
        self.origin_uri_by_new.get(&new_id).map(String::as_str)
    }

    /// The set of synthesized resource ids covered by the bimap.
    pub fn new_ids(&self) -> impl Iterator<Item = Uuid> + '_ {
        self.new_to_old.keys().copied()
    }

    /// The number of resolved (old↔new) resource pairs.
    pub fn len(&self) -> usize {
        self.new_to_old.len()
    }

    /// True iff no resources resolved.
    pub fn is_empty(&self) -> bool {
        self.new_to_old.is_empty()
    }
}
