//! WS6 §9 chunk-3 read surface over `temper_next.*` — read-only parity tooling.
//!
//! This module ports the production read paths (list / meta / body / FTS / vector / neighbors) onto
//! the synthesized substrate so each can be asserted against the production read for the same logical
//! query (the parity-read harness in `tests/parity_reads.rs`).
//!
//! **Scope: this harness proves data/projection parity, NOT access parity.** The readback reads are
//! deliberately **visibility-unscoped** — they return every synthesized resource, where production's
//! oracles scope through `resources_visible_to(profile)` (list) or a `visible` CTE (FTS/vector). Access
//! over `temper_next` is WS2's concern (the producer-intersection model), not this §9 floor. The parity
//! tests hold because the prod-shape fixture makes every active resource visible to its owner P1, so the
//! production and readback result SETS coincide. Leaving readback unscoped is the SAFE direction: an
//! unscoped read is strictly more inclusive, so a synthesis bug producing an extra or wrong-owner row
//! makes the set comparison FAIL loudly — it can never mask a defect as a false pass.
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
            AND dt.property_key = 'doc_type' AND NOT dt.is_folded
           LEFT JOIN temper_next.kb_properties st
             ON st.owner_table = 'kb_resources' AND st.owner_id = r.id
            AND st.property_key = 'temper-stage' AND NOT st.is_folded
           LEFT JOIN temper_next.kb_properties md
             ON md.owner_table = 'kb_resources' AND md.owner_id = r.id
            AND md.property_key = 'temper-mode' AND NOT md.is_folded
           LEFT JOIN temper_next.kb_properties ef
             ON ef.owner_table = 'kb_resources' AND ef.owner_id = r.id
            AND ef.property_key = 'temper-effort' AND NOT ef.is_folded
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
/// Faithful only for production's closed managed-key universe (the 16 keys of G7): an *unrecognized*
/// managed key would synthesize as a `Property` (the forward classifier's conservative carry) yet land
/// in `open` here (`is_managed_property_key` returns false for it). That asymmetry cannot occur for the
/// fixed production key set; if the managed vocabulary ever grows, extend `MANAGED_PROPERTY_KEYS`.
///
/// Read-only; no writes. Runtime, schema-qualified `sqlx::query` (NEVER the `query!` macros) — see the
/// module-level note.
pub async fn meta(pool: &PgPool, new_id: Uuid) -> Result<ReconstructedMeta> {
    let rows = sqlx::query(
        "SELECT property_key, property_value
           FROM temper_next.kb_properties
          WHERE owner_table = 'kb_resources' AND owner_id = $1 AND NOT is_folded",
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

/// The migration-invariant subset of production's `ResourceRow`, reconstructed from `temper_next.*`
/// for the full-row reads (`show` / `by_uri`). Excludes the non-invariant fields by construction:
/// re-minted identity UUIDs (resource id / context id / profile ids), §7-dissolved
/// `slug`/`managed_hash`/`open_hash`, and the synthesis-collapsed `created`/`updated`. The caller
/// (`NextBackend::show_resource`) supplies those from elsewhere (re-minted ids verbatim, a transitional
/// `public.kb_doc_types` lookup for the doctype id, `None` for the dissolved fields, `Utc::now()` for
/// the timestamps). See the WS6 4b spec parity-floor amendment.
///
/// The re-minted ids (`re_minted_id` / `re_minted_context_id` / `owner_profile_id` /
/// `originator_profile_id`) are carried so the caller can populate `ResourceRow`'s non-optional UUID
/// fields with the synthesized values — they are NOT migration invariants and are never asserted in
/// parity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceRowParity {
    /// The synthesized resource id (re-minted; not a parity invariant).
    pub re_minted_id: Uuid,
    /// The synthesized home-anchor context id (re-minted; not a parity invariant).
    pub re_minted_context_id: Uuid,
    /// The synthesized owner profile id (re-minted; not a parity invariant).
    pub owner_profile_id: Uuid,
    /// The synthesized originator profile id (re-minted; not a parity invariant).
    pub originator_profile_id: Uuid,
    /// Verbatim-carried, UNIQUE origin_uri (invariant).
    pub origin_uri: String,
    /// Resource title (invariant).
    pub title: String,
    /// Active flag (invariant; synthesis carries only active resources, so always true here).
    pub is_active: bool,
    /// Home context display name (invariant).
    pub context_name: String,
    /// Authoritative doctype name (invariant) — the `doc_type` property.
    pub doc_type_name: String,
    /// Owner profile handle (invariant).
    pub owner_handle: String,
    /// `temper-stage`, if present (invariant).
    pub stage: Option<String>,
    /// `temper-mode`, if present (invariant).
    pub mode: Option<String>,
    /// `temper-effort`, if present (invariant).
    pub effort: Option<String>,
    /// `temper-seq` parsed to i64, if present (invariant).
    pub seq: Option<i64>,
    /// Denormalized body merkle hash (invariant) — `kb_resources.body_hash`.
    pub body_hash: Option<String>,
}

/// Port of production's full-row read (`resource_service::get_visible` / `resolve_by_uri`, behind
/// `show` / `by_uri`) onto `temper_next.*`, at the §9 INVARIANT-FIELD floor. Joins the home (which
/// anchors the context and owner profile), the `doc_type` property, and the workflow properties.
/// Deliberately does NOT select `created`/`updated` (temper-next's sqlx has no `chrono` feature, and
/// they are synthesis-collapsed non-invariants — the caller stamps read-time `now()`).
///
/// Read-only; no writes. Runtime, schema-qualified `sqlx::query` (NEVER the `query!` macros) — see the
/// module-level note.
pub async fn resource_row(pool: &PgPool, new_id: Uuid) -> Result<ResourceRowParity> {
    let row = sqlx::query(
        "SELECT r.id              AS re_minted_id,
                r.origin_uri,
                r.title,
                r.is_active,
                r.body_hash,
                c.id              AS re_minted_context_id,
                c.name            AS context_name,
                h.owner_profile_id,
                h.originator_profile_id,
                p.handle          AS owner_handle,
                dt.property_value #>> '{}' AS doc_type_name,
                st.property_value #>> '{}' AS stage,
                md.property_value #>> '{}' AS mode,
                ef.property_value #>> '{}' AS effort,
                sq.property_value #>> '{}' AS seq
           FROM temper_next.kb_resources r
           JOIN temper_next.kb_resource_homes h ON h.resource_id = r.id
           JOIN temper_next.kb_contexts c
             ON c.id = h.anchor_id AND h.anchor_table = 'kb_contexts'
           JOIN temper_next.kb_profiles p ON p.id = h.owner_profile_id
           JOIN temper_next.kb_properties dt
             ON dt.owner_table = 'kb_resources' AND dt.owner_id = r.id
            AND dt.property_key = 'doc_type' AND NOT dt.is_folded
           LEFT JOIN temper_next.kb_properties st
             ON st.owner_table = 'kb_resources' AND st.owner_id = r.id
            AND st.property_key = 'temper-stage' AND NOT st.is_folded
           LEFT JOIN temper_next.kb_properties md
             ON md.owner_table = 'kb_resources' AND md.owner_id = r.id
            AND md.property_key = 'temper-mode' AND NOT md.is_folded
           LEFT JOIN temper_next.kb_properties ef
             ON ef.owner_table = 'kb_resources' AND ef.owner_id = r.id
            AND ef.property_key = 'temper-effort' AND NOT ef.is_folded
           LEFT JOIN temper_next.kb_properties sq
             ON sq.owner_table = 'kb_resources' AND sq.owner_id = r.id
            AND sq.property_key = 'temper-seq' AND NOT sq.is_folded
          WHERE r.id = $1",
    )
    .bind(new_id)
    .fetch_one(pool)
    .await?;

    let seq_text: Option<String> = row.get("seq");
    let seq = match seq_text {
        Some(s) => Some(s.parse::<i64>().map_err(|e| {
            anyhow::anyhow!("temper-seq {s:?} is not an i64 for resource {new_id}: {e}")
        })?),
        None => None,
    };

    Ok(ResourceRowParity {
        re_minted_id: row.get("re_minted_id"),
        re_minted_context_id: row.get("re_minted_context_id"),
        owner_profile_id: row.get("owner_profile_id"),
        originator_profile_id: row.get("originator_profile_id"),
        origin_uri: row.get("origin_uri"),
        title: row.get("title"),
        is_active: row.get("is_active"),
        context_name: row.get("context_name"),
        doc_type_name: row.get("doc_type_name"),
        owner_handle: row.get("owner_handle"),
        stage: row.get("stage"),
        mode: row.get("mode"),
        effort: row.get("effort"),
        seq,
        body_hash: row.get("body_hash"),
    })
}

/// Resolve a synthesized resource id from its `origin_uri` (the verbatim-carried, UNIQUE key both
/// schemas share). `None` when no synthesized resource carries that uri. Used by read surfaces that
/// hold an `origin_uri` (e.g. search results) and need the row id for a follow-up reconstruction.
///
/// Read-only; no writes. Runtime, schema-qualified `sqlx::query` (NEVER the `query!` macros) — see the
/// module-level note.
pub async fn resource_id_by_origin_uri(pool: &PgPool, origin_uri: &str) -> Result<Option<Uuid>> {
    let row = sqlx::query("SELECT id FROM temper_next.kb_resources WHERE origin_uri = $1")
        .bind(origin_uri)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.get::<Uuid, _>("id")))
}

/// Reconstruct a synthesized resource's markdown body from `temper_next` chunks — the §9 body read
/// floor. Reuses [`crate::synthesis::parity::reconstruct_body`] (the production `get_content` assembly)
/// over the shared [`crate::synthesis::parity::new_substrate_chunks`] reader, so the read surface and
/// the §8 synthesis gate share one algorithm (CONFORM, no second body assembler).
///
/// Resolves `new_id` → `origin_uri` (the join key both schemas share, carried verbatim) so the shared
/// reader — keyed by `origin_uri` — can fetch the resource's current chunks.
///
/// Read-only; no writes. Runtime, schema-qualified `sqlx::query` (NEVER the `query!` macros) — see the
/// module-level note.
pub async fn body(pool: &PgPool, new_id: Uuid) -> Result<String> {
    let origin_uri: String =
        sqlx::query("SELECT origin_uri FROM temper_next.kb_resources WHERE id = $1")
            .bind(new_id)
            .fetch_one(pool)
            .await?
            .get("origin_uri");

    let chunks = crate::synthesis::parity::new_substrate_chunks(pool, &origin_uri).await?;
    Ok(crate::synthesis::parity::reconstruct_body(&chunks))
}

/// Port of production's FTS read (`search_service::search`, FTS-only) onto `temper_next.*` — the §9
/// search read floor. Builds, per resource, the §9-REBUILT weighted tsvector and returns the matching
/// `origin_uri`s ranked by `ts_rank DESC`.
///
/// The tsvector is `setweight(to_tsvector('english', title), 'A') || setweight(..body.., 'B')` —
/// title-only weight-A, body weight-B. This deliberately DIVERGES from production's
/// `rebuild_resource_search_vector` (migration 20260405000001), whose A-weight is `title || slug`: §7
/// dissolved slug, so §9 rebuilds FTS title-only. The body is the RAW current-chunk content
/// space-joined (`string_agg(content, ' ')`), exactly as production aggregates it — NOT the
/// heading-prefixed assembled markdown [`crate::synthesis::parity::reconstruct_body`] produces (that's
/// the `get_content` body, wrong for FTS). Config is `'english'` (production's default).
///
/// Because production ranks slug@A and readback structurally cannot, absolute `ts_rank` and the order
/// among equal-weight matches are NOT migration invariants — the parity floor the test asserts is the
/// matching SET, not the ordered list. `ORDER BY rank DESC` here stays faithful to production behavior.
/// Note slug@A can change *membership*, not just order: a query term living ONLY in a slug would match
/// production and not readback. Set parity is therefore guaranteed only for terms present in title/body
/// (where it holds exactly); a slug-only term legitimately diverges — that's §7 dissolving the slug, not
/// a defect. The fixture's queries are title/body terms by design.
///
/// Read-only; no writes. Runtime, schema-qualified `sqlx::query` (NEVER the `query!` macros) — see the
/// module-level note.
pub async fn fts_search(pool: &PgPool, query: &str) -> Result<Vec<String>> {
    let rows = sqlx::query(
        "WITH doc AS (
           SELECT r.id,
                  r.origin_uri,
                  setweight(to_tsvector('english', r.title), 'A') ||
                  setweight(to_tsvector('english', COALESCE(string_agg(cc.content, ' '), '')), 'B')
                    AS search_vector
             FROM temper_next.kb_resources r
             LEFT JOIN temper_next.kb_chunks c
               ON c.resource_id = r.id AND c.is_current
             LEFT JOIN temper_next.kb_chunk_content cc
               ON cc.chunk_id = c.id
            GROUP BY r.id, r.origin_uri, r.title
         )
         SELECT origin_uri
           FROM doc
          WHERE search_vector @@ plainto_tsquery('english', $1)
          ORDER BY ts_rank(search_vector, plainto_tsquery('english', $1)) DESC",
    )
    .bind(query)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|r| r.get::<String, _>("origin_uri"))
        .collect())
}

/// Format a `Vec<f32>` as a pgvector text literal (`[a,b,c]`) for binding into a `::vector` cast — the
/// inverse of [`crate::synthesis::source`]'s `parse_pgvector`. Inlined here (a tiny helper) rather than
/// reusing production's `temper_core::types::ingest::format_embedding`: temper-core is only a DEV-dep of
/// temper-next, not a lib dep, and pulling it into the lib just to format five floats would be
/// over-coupling. Uses `{}` (not `{:?}`) so each float renders without a debug wrapper.
fn format_pgvector(v: &[f32]) -> String {
    let mut out = String::with_capacity(v.len() * 8 + 2);
    out.push('[');
    for (i, x) in v.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&x.to_string());
    }
    out.push(']');
    out
}

/// Cosine vector search over `temper_next` chunks (§9 vector floor). Per resource, the best (min-cosine-
/// distance) current chunk decides rank; results ascend by that distance — exactly production's
/// `vec_hits` (MIN distance per resource, `ORDER BY MIN(embedding <=> query)`). Embeddings carry
/// verbatim from production (§8), so this ordered output matches production's vector search bit-for-bit
/// (contrast `fts_search`, where production's slug@A weight makes only the matching SET an invariant).
///
/// The query embedding is formatted to a pgvector text literal and bound into a `$1::vector` cast.
/// Runtime `sqlx::query` with the `::vector` cast is the ESTABLISHED pgvector-macro exception —
/// production's own `unified_search` uses runtime `query_as` for exactly this reason (the `query!`
/// macros don't support the `::vector` cast).
///
/// Read-only; no writes. Schema-qualified throughout — see the module-level note.
pub async fn vector_search(pool: &PgPool, query_embedding: &[f32]) -> Result<Vec<String>> {
    let embedding_text = format_pgvector(query_embedding);
    let rows = sqlx::query(
        "SELECT r.origin_uri
           FROM temper_next.kb_resources r
           JOIN temper_next.kb_chunks c
             ON c.resource_id = r.id AND c.is_current
          GROUP BY r.id, r.origin_uri
          ORDER BY MIN(c.embedding <=> $1::vector) ASC",
    )
    .bind(embedding_text)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|r| r.get::<String, _>("origin_uri"))
        .collect())
}

/// One 1-hop graph neighbor of a resource: the OTHER endpoint's origin_uri plus the connecting edge's
/// kind/polarity/label. The §9 graph-neighbors read floor over `temper_next.kb_edges` (folded edges
/// excluded, matching production's `NOT is_folded` gate).
///
/// `label` is `Option<String>`: an empty production label carries as `NULL` through synthesis, so an
/// edge with no label surfaces here as `None` (never `Some("")`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Neighbor {
    /// The neighbor (other endpoint) resource's verbatim-carried, UNIQUE `origin_uri`.
    pub origin_uri: String,
    /// The connecting edge's kind (`edge_kind::text`).
    pub edge_kind: String,
    /// The connecting edge's polarity (`polarity::text`), carried verbatim from production.
    pub polarity: String,
    /// The connecting edge's label, or `None` when absent (empty production label → `NULL`).
    pub label: Option<String>,
}

/// Port of production's 1-hop graph-neighbor read onto `temper_next.*` — the §9 graph-neighbors read
/// floor. Returns the resource↔resource neighbors of `new_id` over `temper_next.kb_edges`, in BOTH
/// directions (the seed as `source_id` → the `target` endpoint; the seed as `target_id` → the `source`
/// endpoint), with folded edges EXCLUDED (`NOT is_folded`, matching production's gate).
///
/// The production counterpart is a DIRECT symmetric edge read over `public.kb_resource_edges` (same
/// table + `NOT is_folded` gate + `edge_kind`/`polarity`/`label` projection) — NOT
/// `graph_service::aggregator_subgraph`, which is subgraph-over-a-node-set (it returns the edges among a
/// passed node set) and would be circular as a 1-hop neighbor oracle. The parity test writes that
/// production query directly.
///
/// 1-hop ONLY (the §9 neighbors floor) — there is deliberately NO `depth`/multi-hop traversal param:
/// the tested floor and the production neighbor read are both 1-hop; multi-hop is a kernel concern
/// beyond this parity task (SG-5, no speculative surface). Order is NOT contractual — the parity test
/// compares neighbor SETS.
///
/// Read-only; no writes. Runtime, schema-qualified `sqlx::query` (NEVER the `query!` macros) — see the
/// module-level note.
pub async fn neighbors(pool: &PgPool, new_id: Uuid) -> Result<Vec<Neighbor>> {
    let rows = sqlx::query(
        "SELECT t.origin_uri AS origin_uri, e.edge_kind::text AS edge_kind, \
                e.polarity::text AS polarity, e.label \
           FROM temper_next.kb_edges e \
           JOIN temper_next.kb_resources t ON t.id = e.target_id \
          WHERE e.source_id = $1 \
            AND e.source_table = 'kb_resources' AND e.target_table = 'kb_resources' \
            AND NOT e.is_folded \
         UNION ALL \
         SELECT s.origin_uri AS origin_uri, e.edge_kind::text AS edge_kind, \
                e.polarity::text AS polarity, e.label \
           FROM temper_next.kb_edges e \
           JOIN temper_next.kb_resources s ON s.id = e.source_id \
          WHERE e.target_id = $1 \
            AND e.source_table = 'kb_resources' AND e.target_table = 'kb_resources' \
            AND NOT e.is_folded",
    )
    .bind(new_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|row| Neighbor {
            origin_uri: row.get("origin_uri"),
            edge_kind: row.get("edge_kind"),
            polarity: row.get("polarity"),
            label: row.get("label"),
        })
        .collect())
}
