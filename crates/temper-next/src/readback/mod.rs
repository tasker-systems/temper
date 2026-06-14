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
use serde_json::{Map, Value};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::synthesis::key_fate::is_managed_property_key;

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

/// One projected list row over `temper_next.*` — the readback counterpart of production's
/// `ResourceRow` for the fields the resource-list projection surfaces (`temper-api`'s
/// `resource_service::list_visible` via the `vault_resources_browse` view): the resource's
/// `origin_uri` + `title`, its `doc_type`, and the three workflow fields lifted from `managed_meta`
/// (`temper-stage`/`temper-mode`/`temper-effort`). Each workflow field is `None` when the resource
/// carries no such property (R1/R3/R5 in the prod-shape fixture), matching the view's
/// `managed_meta->>'…'` NULL for an absent key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRow {
    /// The verbatim-carried, UNIQUE `origin_uri` (the join key both schemas share).
    pub origin_uri: String,
    /// The resource title (`temper_next.kb_resources.title`).
    pub title: String,
    /// The authoritative doctype, from the `doc_type` property the resource pass stamps.
    pub doc_type: String,
    /// `temper-stage`, if present in the synthesized properties.
    pub stage: Option<String>,
    /// `temper-mode`, if present.
    pub mode: Option<String>,
    /// `temper-effort`, if present.
    pub effort: Option<String>,
}

/// Port of production's resource-list projection (`resource_service::list_visible` over the
/// `vault_resources_browse` view) onto `temper_next.*`: returns every synthesized resource (the §0
/// active set — synthesis never carries soft-deleted rows, so there is no `is_active` filter to apply
/// here) with the same projected fields production surfaces.
///
/// The doctype and the three workflow fields all live in `temper_next.kb_properties` (synthesis writes
/// them via `facet_set`, plus the direct `doc_type` property the resource pass stamps). `doc_type` is an
/// inner JOIN — every synthesized resource has one; the workflow keys are LEFT JOINs so a resource
/// without them comes back with `NULL` (not dropped). Property values are JSON scalars, extracted to
/// text with `#>> '{}'` (the same extraction `synthesis::run`'s property test uses).
///
/// Ordered by `origin_uri` (verbatim, UNIQUE) so the result is deterministic. It is deliberately NOT
/// ordered by `updated`: synthesis sources `kb_resources.created`/`updated` from the genesis event's
/// `occurred_at`, which is `now()` = transaction-start time and therefore identical across every row
/// written in the single synthesis transaction. Absolute recency ordering is not a migration-time
/// invariant (event-sourced backfill collapses timestamps to synthesis time); the row set + projected
/// fields are.
///
/// Runtime, schema-qualified `sqlx::query` (NEVER the `query!` macros) — see the module-level note.
pub async fn list(pool: &PgPool) -> Result<Vec<ListRow>> {
    let rows = sqlx::query(
        "SELECT r.origin_uri,
                r.title,
                dt.property_value #>> '{}' AS doc_type,
                st.property_value #>> '{}' AS stage,
                md.property_value #>> '{}' AS mode,
                ef.property_value #>> '{}' AS effort
           FROM temper_next.kb_resources r
           JOIN temper_next.kb_properties dt
             ON dt.owner_table = 'kb_resources' AND dt.owner_id = r.id
            AND dt.property_key = 'doc_type'
           LEFT JOIN temper_next.kb_properties st
             ON st.owner_table = 'kb_resources' AND st.owner_id = r.id
            AND st.property_key = 'temper-stage'
           LEFT JOIN temper_next.kb_properties md
             ON md.owner_table = 'kb_resources' AND md.owner_id = r.id
            AND md.property_key = 'temper-mode'
           LEFT JOIN temper_next.kb_properties ef
             ON ef.owner_table = 'kb_resources' AND ef.owner_id = r.id
            AND ef.property_key = 'temper-effort'
          ORDER BY r.origin_uri",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|row| ListRow {
            origin_uri: row.get("origin_uri"),
            title: row.get("title"),
            doc_type: row.get("doc_type"),
            stage: row.get("stage"),
            mode: row.get("mode"),
            effort: row.get("effort"),
        })
        .collect())
}

/// Reconstructed frontmatter for one synthesized resource, the inverse of the §7 fate table over
/// `temper_next.kb_properties`. Mirrors production `get_meta`'s managed/open split, EXCEPT the §7-died
/// keys (`temper-title`/`-slug`/`-id`/`-context`) never reappear (their state lives authoritatively in
/// the column / render-time decoration / row id / home row) and `temper-goal` lives as an edge, not
/// here. `temper-type` is reconciled to the `doc_type` column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconstructedMeta {
    /// Surviving managed (workflow + provenance) keys — those in
    /// [`crate::synthesis::key_fate::MANAGED_PROPERTY_KEYS`] — with values verbatim.
    pub managed: Map<String, Value>,
    /// Open (user-defined) keys, verbatim.
    pub open: Map<String, Value>,
    /// The authoritative doc type (the `doc_type` property; successor to production's `temper-type`).
    pub doc_type: String,
}

/// Port of production's `get_meta` (the meta tier behind `show`) onto `temper_next.*`: reconstruct the
/// managed/open frontmatter split for one synthesized resource from its `kb_properties` rows — the
/// inverse of the §7 fate table.
///
/// §7 dissolved the production manifest into columns, the home row, edges, the `doc_type` property, and
/// `kb_properties`. The properties carry both the surviving managed (workflow/provenance) keys and the
/// open (user-defined) keys; `is_managed_property_key` is the inverse fate that tells them apart (the
/// forward classifier can't — it carries unknown keys as `Property` too). The `doc_type` property is
/// the authoritative doctype the resource pass stamped (successor to production's `temper-type`). The
/// §7-died keys are absent by construction — they were never written as properties.
///
/// Read-only; no writes. Runtime, schema-qualified `sqlx::query` (NEVER the `query!` macros) — see the
/// module-level note.
pub async fn meta(pool: &PgPool, new_id: Uuid) -> Result<ReconstructedMeta> {
    let rows = sqlx::query(
        "SELECT property_key, property_value
           FROM temper_next.kb_properties
          WHERE owner_table = 'kb_resources' AND owner_id = $1",
    )
    .bind(new_id)
    .fetch_all(pool)
    .await?;

    let mut managed = Map::new();
    let mut open = Map::new();
    let mut doc_type: Option<String> = None;

    for row in &rows {
        let key: String = row.get("property_key");
        let value: Value = row.get("property_value");
        if key == "doc_type" {
            // The authoritative doctype is a JSON string scalar; surface it as the typed field.
            doc_type = Some(match value {
                Value::String(s) => s,
                other => other.to_string(),
            });
        } else if is_managed_property_key(&key) {
            managed.insert(key, value);
        } else {
            open.insert(key, value);
        }
    }

    let doc_type = doc_type.ok_or_else(|| {
        anyhow::anyhow!("synthesized resource {new_id} has no doc_type property (every resource pass stamps one)")
    })?;

    Ok(ReconstructedMeta {
        managed,
        open,
        doc_type,
    })
}
