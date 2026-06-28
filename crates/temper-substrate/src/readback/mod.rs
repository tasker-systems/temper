//! WS6 §9 chunk-3 read surface over the substrate tables — read-only parity tooling.
//!
//! This module ports the production read paths (list / meta / body / FTS / vector / neighbors) onto
//! the synthesized substrate so each can be asserted against the production read for the same logical
//! query (the parity-read harness in `tests/parity_reads.rs`).
//!
//! **Access scoping (WS2).** The single-resource and set reads (`list`/`resource_row`/`meta`/`body`/
//! `fts_search`/`vector_search`) take a `principal` and gate through `resources_visible_to`,
//! CONFORMing to production's `resources_visible_to(profile)` JOIN and its not-visible→404 deny. A
//! not-visible single-resource read errors (the read selector maps that to NotFound, never 403); the set
//! reads JOIN-filter to the visible set. `neighbors` is the lone exception — deliberately UNSCOPED, as it
//! has no surface caller yet (see its note); the graph-neighbor scoping lands with that surface.
//! The prod-shape parity tests pass `OWNER_PROFILE` (who owns all 4 active resources), so the scoped
//! and production result SETS still coincide; a separate test drives a non-owner principal to prove the
//! gate denies.
//!
//! Most reads are runtime `sqlx::query` (the pgvector `::vector` cast forces runtime; the rest follow
//! for consistency). The SQL is UNQUALIFIED (`kb_*` / `resources_visible_to`) — there is one schema
//! (`public`), and the connection's search_path resolves unqualified names and the visibility
//! function's own unqualified internals correctly with no per-txn `SET LOCAL`. The lone compile-time
//! macro ([`find_by_body_hash`]) is likewise unqualified and uses the workspace sqlx cache.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::ids::{EntityId, ProfileId};
use crate::keys::is_managed_property_key;

/// Why a single-resource readback (`resource_row`/`meta`/`body`, via `ensure_visible`) failed, typed so
/// the surface can map each mode to the right HTTP status. The alternative — string-matching one
/// `anyhow` message — is forbidden by the repo's no-stringly-typed-matching rule.
///
/// - [`ReadbackError::NotVisible`] is the leak-safe deny: the principal cannot see the resource. The
///   surface maps it to **404** — never 403 (403 would confirm the resource exists, an existence oracle),
///   never 500 (not a system failure).
/// - [`ReadbackError::Fault`] is a genuine failure (DB error, malformed synthesized state). The surface
///   maps it to **500**. Collapsing it into NotVisible — the pre-typing behavior — masked real faults
///   as 404.
///
/// The set reads (`list`/`fts_search`/`vector_search`/`neighbors`) JOIN-filter the visible set instead
/// of pre-checking one id, so they have no not-visible signal and stay `anyhow::Result` (any error
/// there is a genuine fault → 500).
#[derive(Debug)]
pub enum ReadbackError {
    /// The resource is not visible to the principal under `resources_visible_to` → surface 404.
    NotVisible {
        /// The (synthesized) resource id that was requested.
        resource_id: Uuid,
        /// The principal the visibility check ran against.
        principal: Uuid,
    },
    /// A genuine read-path fault (DB error, missing-by-construction state) → surface 500.
    Fault(anyhow::Error),
}

impl std::fmt::Display for ReadbackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotVisible {
                resource_id,
                principal,
            } => write!(f, "resource {resource_id} not visible to {principal}"),
            Self::Fault(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ReadbackError {}

impl From<sqlx::Error> for ReadbackError {
    fn from(e: sqlx::Error) -> Self {
        Self::Fault(e.into())
    }
}

impl From<anyhow::Error> for ReadbackError {
    fn from(e: anyhow::Error) -> Self {
        Self::Fault(e)
    }
}

/// One projected list row over the substrate tables — the readback counterpart of production's
/// `ResourceRow` for the fields the resource-list projection surfaces (`temper-api`'s
/// `resource_service::list_visible` via the `vault_resources_browse` view): the resource's
/// `origin_uri` + `title`, its `doc_type`, and the three workflow fields lifted from `managed_meta`
/// (`temper-stage`/`temper-mode`/`temper-effort`). Each workflow field is `None` when the resource
/// carries no such property (R1/R3/R5 in the prod-shape fixture), matching the view's
/// `managed_meta->>'…'` NULL for an absent key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRow {
    /// The verbatim-carried `origin_uri` (a provenance marker, NOT unique — empty for CLI/agent-created
    /// resources; the resource id is the stable identity).
    pub origin_uri: String,
    /// The resource title (`kb_resources.title`).
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
/// `vault_resources_browse` view) onto the substrate tables: returns every synthesized resource (the §0
/// active set — synthesis never carries soft-deleted rows, so there is no `is_active` filter to apply
/// here) with the same projected fields production surfaces.
///
/// The doctype and the three workflow fields all live in `kb_properties` (synthesis writes
/// them via `facet_set`, plus the direct `doc_type` property the resource pass stamps). `doc_type` is an
/// inner JOIN — every synthesized resource has one; the workflow keys are LEFT JOINs so a resource
/// without them comes back with `NULL` (not dropped). Property values are JSON scalars, extracted to
/// text with `#>> '{}'` (the doc-type-as-property extraction).
///
/// Ordered by `(origin_uri, id)` so the result is deterministic — `origin_uri` alone is NOT unique
/// (empty for CLI/agent-created resources), so the resource id is the tiebreaker. It is deliberately NOT
/// ordered by `updated`: synthesis sources `kb_resources.created`/`updated` from the genesis event's
/// `occurred_at`, which is `now()` = transaction-start time and therefore identical across every row
/// written in the single synthesis transaction. Absolute recency ordering is not a migration-time
/// invariant (event-sourced backfill collapses timestamps to synthesis time); the row set + projected
/// fields are.
///
/// Runtime, schema-qualified `sqlx::query` (NEVER the `query!` macros) — see the module-level note.
/// WS2 consumer-axis gate for single-resource reads: error unless `new_id` is visible to
/// `principal` under `resources_visible_to`. The set reads (`list`/`fts_search`/
/// `vector_search`/`neighbors`) instead JOIN the function directly (a set can't be pre-checked).
///
/// Returns a TYPED [`ReadbackError`]: [`ReadbackError::NotVisible`] when the principal can't see the
/// resource (the caller maps it to 404 — denying existence, never 403, no existence-leak oracle,
/// CONFORMing to production's NotFound-on-not-visible) versus [`ReadbackError::Fault`] for a genuine DB
/// error in the check itself (mapped to 500). Conflating the two — the pre-typing behavior — masked
/// real faults as 404.
async fn ensure_visible(
    pool: &PgPool,
    principal: Uuid,
    new_id: Uuid,
) -> std::result::Result<(), ReadbackError> {
    // `resources_visible_to` and its nested `profile_effective_teams`/`team_ancestors` resolve their
    // unqualified references against the connection search_path (`public` — the one schema), so no
    // per-txn `SET LOCAL`.
    let visible: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id = $2)",
    )
    .bind(principal)
    .bind(new_id)
    .fetch_one(pool)
    .await?;
    if visible {
        Ok(())
    } else {
        Err(ReadbackError::NotVisible {
            resource_id: new_id,
            principal,
        })
    }
}

pub async fn list(pool: &PgPool, principal: Uuid) -> Result<Vec<ListRow>> {
    let rows = sqlx::query(
        "SELECT r.origin_uri,
                r.title,
                dt.property_value #>> '{}' AS doc_type,
                st.property_value #>> '{}' AS stage,
                md.property_value #>> '{}' AS mode,
                ef.property_value #>> '{}' AS effort
           FROM kb_resources r
           JOIN resources_visible_to($1) v ON v.resource_id = r.id
           JOIN kb_properties dt
             ON dt.owner_table = 'kb_resources' AND dt.owner_id = r.id
            AND dt.property_key = 'doc_type' AND NOT dt.is_folded
           LEFT JOIN kb_properties st
             ON st.owner_table = 'kb_resources' AND st.owner_id = r.id
            AND st.property_key = 'temper-stage' AND NOT st.is_folded
           LEFT JOIN kb_properties md
             ON md.owner_table = 'kb_resources' AND md.owner_id = r.id
            AND md.property_key = 'temper-mode' AND NOT md.is_folded
           LEFT JOIN kb_properties ef
             ON ef.owner_table = 'kb_resources' AND ef.owner_id = r.id
            AND ef.property_key = 'temper-effort' AND NOT ef.is_folded
          ORDER BY r.origin_uri, r.id",
    )
    .bind(principal)
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
/// `kb_properties`. Mirrors production `get_meta`'s managed/open split, EXCEPT the §7-died
/// keys (`temper-title`/`-slug`/`-id`/`-context`) never reappear (their state lives authoritatively in
/// the column / render-time decoration / row id / home row) and `temper-goal` lives as an edge, not
/// here. `temper-type` is reconciled to the `doc_type` column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconstructedMeta {
    /// Surviving managed (workflow + provenance) keys — those in
    /// [`crate::keys::MANAGED_PROPERTY_KEYS`] — with values verbatim.
    pub managed: Map<String, Value>,
    /// Open (user-defined) keys, verbatim.
    pub open: Map<String, Value>,
    /// The authoritative doc type (the `doc_type` property; successor to production's `temper-type`).
    pub doc_type: String,
}

/// Port of production's `get_meta` (the meta tier behind `show`) onto the substrate tables: reconstruct the
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
pub async fn meta(
    pool: &PgPool,
    principal: Uuid,
    new_id: Uuid,
) -> std::result::Result<ReconstructedMeta, ReadbackError> {
    ensure_visible(pool, principal, new_id).await?;
    let rows = sqlx::query(
        "SELECT property_key, property_value
           FROM kb_properties
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

/// The migration-invariant subset of production's `ResourceRow`, reconstructed from the substrate tables
/// for the full-row reads (`show` / `by_uri`). The caller is `native_resource_row` in
/// `db_backend.rs`, which passes fields through verbatim. Excludes non-invariant fields by
/// construction: re-minted identity UUIDs (resource id / context id / profile ids) and
/// §7-dissolved `slug`/`managed_hash`/`open_hash`. `created`/`updated` are real `kb_resources`
/// columns — selected from the substrate and populated from `kb_events.occurred_at` at write time;
/// they are NOT synthesis-collapsed placeholders.
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
    /// Verbatim-carried origin_uri (invariant; a provenance marker, NOT unique).
    pub origin_uri: String,
    /// Resource title (invariant).
    pub title: String,
    /// Active flag (invariant; synthesis carries only active resources, so always true here).
    pub is_active: bool,
    /// Real genesis timestamp — `kb_resources.created` (event `occurred_at` at create).
    pub created: DateTime<Utc>,
    /// Real last-mutation timestamp — `kb_resources.updated` (event `occurred_at` at last write).
    pub updated: DateTime<Utc>,
    /// Home context display name (invariant).
    pub context_name: String,
    /// Authoritative doctype name (invariant) — the `doc_type` property.
    pub doc_type_name: String,
    /// Owner profile handle (invariant).
    pub owner_handle: String,
    /// Slug of the home context (the natural-key half of `@owner/slug`). Invariant.
    pub context_slug: String,
    /// Already-sigil'd owner of the home context (`@<handle>` or `+<team-slug>`). Invariant.
    pub context_owner_ref: String,
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
/// `show` / `by_uri`) onto the substrate tables, at the §9 INVARIANT-FIELD floor. Joins the home (which
/// anchors the context and owner profile), the `doc_type` property, and the workflow properties.
/// Selects `created`/`updated` from `kb_resources` — populated from `kb_events.occurred_at` at write
/// time (create sets both to genesis event's `occurred_at`; updates bump `updated`).
///
/// Read-only; no writes. Runtime, schema-qualified `sqlx::query` (NEVER the `query!` macros) — see the
/// module-level note.
pub async fn resource_row(
    pool: &PgPool,
    principal: Uuid,
    new_id: Uuid,
) -> std::result::Result<ResourceRowParity, ReadbackError> {
    ensure_visible(pool, principal, new_id).await?;
    let row = sqlx::query(
        "SELECT r.id              AS re_minted_id,
                r.origin_uri,
                r.title,
                r.is_active,
                r.created,
                r.updated,
                r.body_hash,
                c.id              AS re_minted_context_id,
                c.name            AS context_name,
                c.slug            AS context_slug,
                CASE c.owner_table
                  WHEN 'kb_teams' THEN '+' || (SELECT slug   FROM kb_teams    WHERE id = c.owner_id)
                  ELSE                   '@' || (SELECT handle FROM kb_profiles WHERE id = c.owner_id)
                END               AS context_owner_ref,
                h.owner_profile_id,
                h.originator_profile_id,
                p.handle          AS owner_handle,
                dt.property_value #>> '{}' AS doc_type_name,
                st.property_value #>> '{}' AS stage,
                md.property_value #>> '{}' AS mode,
                ef.property_value #>> '{}' AS effort,
                sq.property_value #>> '{}' AS seq
           FROM kb_resources r
           JOIN kb_resource_homes h ON h.resource_id = r.id
           JOIN kb_contexts c
             ON c.id = h.anchor_id AND h.anchor_table = 'kb_contexts'
           JOIN kb_profiles p ON p.id = h.owner_profile_id
           JOIN kb_properties dt
             ON dt.owner_table = 'kb_resources' AND dt.owner_id = r.id
            AND dt.property_key = 'doc_type' AND NOT dt.is_folded
           LEFT JOIN kb_properties st
             ON st.owner_table = 'kb_resources' AND st.owner_id = r.id
            AND st.property_key = 'temper-stage' AND NOT st.is_folded
           LEFT JOIN kb_properties md
             ON md.owner_table = 'kb_resources' AND md.owner_id = r.id
            AND md.property_key = 'temper-mode' AND NOT md.is_folded
           LEFT JOIN kb_properties ef
             ON ef.owner_table = 'kb_resources' AND ef.owner_id = r.id
            AND ef.property_key = 'temper-effort' AND NOT ef.is_folded
           LEFT JOIN kb_properties sq
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
        created: row.get("created"),
        updated: row.get("updated"),
        context_name: row.get("context_name"),
        context_slug: row.get("context_slug"),
        context_owner_ref: row.get("context_owner_ref"),
        doc_type_name: row.get("doc_type_name"),
        owner_handle: row.get("owner_handle"),
        stage: row.get("stage"),
        mode: row.get("mode"),
        effort: row.get("effort"),
        seq,
        body_hash: row.get("body_hash"),
    })
}

/// Reconstruct a resource's markdown body from its substrate chunks — the §9 body read floor.
/// Reuses [`crate::content::reconstruct_body`] (the production `get_content` assembly) — the one body
/// assembler (CONFORM, no second).
///
/// Reads the chunk tables UNQUALIFIED (like every sibling readback) so they resolve against the
/// connection's search_path / the single post-collapse `public` schema.
///
/// Keys by `new_id` directly — the resource id (preserved verbatim from production).
///
/// Read-only; no writes. Runtime `sqlx::query` (NEVER the `query!` macros) — see the module-level note.
pub async fn body(
    pool: &PgPool,
    principal: Uuid,
    new_id: Uuid,
) -> std::result::Result<String, ReadbackError> {
    use sqlx::Row;
    ensure_visible(pool, principal, new_id).await?;
    let rows = sqlx::query(
        "SELECT c.chunk_index, COALESCE(c.header_path, '') AS header_path, \
                COALESCE(c.heading_depth, 0::smallint) AS heading_depth, cc.content \
         FROM kb_chunks c \
         JOIN kb_content_blocks b ON b.id = c.block_id \
         JOIN kb_chunk_content cc ON cc.chunk_id = c.id \
         WHERE c.resource_id = $1 AND c.is_current \
         ORDER BY b.seq, c.chunk_index",
    )
    .bind(new_id)
    .fetch_all(pool)
    .await?;
    let chunks: Vec<crate::content::ReadChunk> = rows
        .iter()
        .map(|row| crate::content::ReadChunk {
            chunk_index: row.get("chunk_index"),
            header_path: row.get("header_path"),
            heading_depth: row.get("heading_depth"),
            content: row.get("content"),
        })
        .collect();
    Ok(crate::content::reconstruct_body(&chunks))
}

/// Substrate body-hash dedup for the create path (WS6 collapse Task F). Returns the id of an existing
/// active, visible resource whose `body_hash` matches `body_hash`, or `None`. Mirrors production's
/// `ingest_service::find_by_body_hash` but keys on the substrate `kb_resources.body_hash` (the
/// structural sha256 merkle over chunk content-hashes, `_recompute_resource_body_hash`) instead of the
/// dead `kb_resource_manifests`. Visibility-gated through `resources_visible_to`, like every other read.
///
/// Unlike the rest of this module (runtime, schema-qualified `sqlx::query`), this is a **compile-time
/// macro** — the one in `readback`. The SQL is UNQUALIFIED (`kb_resources` / `resources_visible_to`):
/// the workspace sqlx cache covers it, and at runtime the connection search_path (`public`) resolves
/// the unqualified table and `resources_visible_to`'s own unqualified internals correctly.
pub async fn find_by_body_hash(
    pool: &PgPool,
    principal: Uuid,
    body_hash: &str,
) -> Result<Option<Uuid>> {
    let dup = sqlx::query_scalar!(
        r#"SELECT r.id
             FROM kb_resources r
             JOIN resources_visible_to($1) v ON v.resource_id = r.id
            WHERE r.body_hash = $2 AND r.is_active
            LIMIT 1"#,
        principal,
        body_hash,
    )
    .fetch_optional(pool)
    .await?;
    Ok(dup)
}

/// One row of L0's reconcile diff source: a `provenance: kernel` resource homed to a cogmap, keyed
/// (by the caller) on `resource_id`, carrying its body merkle and merged facet object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelSliceRow {
    /// The diff key — the resource's STABLE id (UUIDv7). The reconcile applier keys its diff index on
    /// this and addresses blocks/edges by it. `origin_uri` is loose provenance, NEVER the key.
    pub resource_id: Uuid,
    /// Verbatim `kb_resources.origin_uri` — a provenance/display marker, NOT unique and NOT the diff
    /// key (that is `resource_id`). Empty for CLI/agent-created resources.
    pub origin_uri: String,
    /// The body merkle — `kb_resources.body_hash`, IDENTICAL to the expression `resource_row` reads
    /// (the bare column; the structural sha256 over chunk content-hashes), so the reconcile diff can
    /// compare `entry.content_hash` against it like-for-like.
    pub body_hash: Option<String>,
    /// The resource's merged non-folded property object (`jsonb_object_agg` over `kb_properties`):
    /// the facets (`provenance`/`layer`/…) plus the `doc_type` property, mirroring how [`meta`] reads
    /// the same rows. The reconcile diff re-asserts the incoming facet keys idempotently against this.
    pub facets: serde_json::Value,
}

/// All `provenance: kernel` resources homed to `cogmap_id`, keyed (by the caller) on `resource_id`,
/// each with its body merkle (`body_hash`) and merged facet object. The reconcile diff source: the
/// orchestration ([`crate`]'s temper-api caller) compares these against the incoming desired-state
/// entries to plan create/update/fold.
///
/// Scoped to the kernel slice by an inner join on the resource's non-folded `provenance` property
/// equal to `kernel`, so `promoted`/`operator` content homed to the same cogmap — and the cogmap's
/// own (provenance-less) telos — are excluded by construction. `body_hash` is the bare
/// `kb_resources.body_hash` column, byte-identical to [`resource_row`]'s `r.body_hash`.
///
/// Compile-time macro (like [`find_by_body_hash`], the only other macro here): the SQL resolves against
/// the single `public` schema, cached in the workspace `.sqlx` (post-`temper_next`-elimination, #178).
pub async fn kernel_slice(
    executor: impl sqlx::PgExecutor<'_>,
    cogmap_id: Uuid,
) -> Result<Vec<KernelSliceRow>> {
    let rows = sqlx::query!(
        r#"SELECT r.id              AS "resource_id!",
                  r.origin_uri      AS "origin_uri!",
                  r.body_hash       AS "body_hash?",
                  (SELECT jsonb_object_agg(p.property_key, p.property_value)
                     FROM kb_properties p
                    WHERE p.owner_table = 'kb_resources' AND p.owner_id = r.id
                      AND NOT p.is_folded) AS "facets?"
             FROM kb_resources r
             JOIN kb_resource_homes h
               ON h.resource_id = r.id
              AND h.anchor_table = 'kb_cogmaps' AND h.anchor_id = $1
             JOIN kb_properties prov
               ON prov.owner_table = 'kb_resources' AND prov.owner_id = r.id
              AND prov.property_key = 'provenance' AND NOT prov.is_folded
              AND prov.property_value #>> '{}' = 'kernel'
            WHERE r.is_active = true
            ORDER BY r.origin_uri"#,
        cogmap_id,
    )
    .fetch_all(executor)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| KernelSliceRow {
            resource_id: row.resource_id,
            origin_uri: row.origin_uri,
            body_hash: row.body_hash,
            // The inner join guarantees at least the `provenance` property, so the aggregate is never
            // SQL-NULL in practice; default to an empty object for total safety.
            facets: row.facets.unwrap_or_else(|| serde_json::json!({})),
        })
        .collect())
}

/// Resolve the **system actor** — the `(owner_profile, emitter_entity)` pair the L0 reconciler fires
/// every mutation under. The lookup is the L0 birth migration's exactly: the profile with
/// `handle = 'system'` joined to its `name = 'system'` entity. Returned typed so the reconcile
/// orchestration threads `ProfileId`/`EntityId` into the cogmap-homed writes without re-resolving.
///
/// Compile-time macro (resolves against `public`; workspace `.sqlx` cache).
pub async fn system_actor(pool: &PgPool) -> Result<(ProfileId, EntityId)> {
    let row = sqlx::query!(
        r#"SELECT p.id AS "owner!", e.id AS "emitter!"
             FROM kb_entities e
             JOIN kb_profiles p ON p.id = e.profile_id
            WHERE p.handle = 'system' AND e.name = 'system'"#,
    )
    .fetch_one(pool)
    .await?;
    Ok((ProfileId::from(row.owner), EntityId::from(row.emitter)))
}

/// Resolve a live (non-folded) resource→resource edge by `(source, target, kind[, polarity])` over
/// `kb_edges`, returning its id or `None`. The L0 reconcile uses this both for Phase-2 idempotent
/// edge dedup (polarity-aware — a forward and an inverse edge of the same kind to the same target are
/// distinct, so pass `Some(polarity)`) and for the Phase-3 edge-fold a `fold_edges` tombstone targets
/// (kind only — pass `None` to match any polarity). Substrate SQL lives in the substrate (CLAUDE.md
/// "Service layer owns SQL") — the reconciler in `db_backend.rs` calls this rather than inlining.
///
/// Casts the columns to text (`edge_kind::text = $3`, `polarity::text = $4`) and binds the SQL enum
/// labels, so the compile-time macro needs no custom enum Rust types. The `polarity` clause is a
/// NULL-passthrough — `$4::text IS NULL` matches any polarity (the fold path). Takes
/// `impl sqlx::PgExecutor` so the reconciler can run it on its serializable transaction connection.
///
/// Compile-time macro (resolves against `public`; workspace `.sqlx` cache).
pub async fn find_edge(
    executor: impl sqlx::PgExecutor<'_>,
    src: Uuid,
    tgt: Uuid,
    kind: &crate::affinity::EdgeKind,
    polarity: Option<&str>,
) -> Result<Option<Uuid>> {
    let id = sqlx::query_scalar!(
        r#"SELECT id FROM kb_edges
            WHERE source_id = $1 AND target_id = $2
              AND source_table = 'kb_resources' AND target_table = 'kb_resources'
              AND edge_kind::text = $3 AND NOT is_folded
              AND ($4::text IS NULL OR polarity::text = $4)"#,
        src,
        tgt,
        kind.as_sql(),
        polarity,
    )
    .fetch_optional(executor)
    .await?;
    Ok(id)
}

/// Port of production's FTS read (`search_service::search`, FTS-only) onto the substrate tables — the §9
/// search read floor. Reads the stored `kb_resource_search_index` tsvector and returns the matching
/// resource **ids** ranked by `ts_rank DESC`.
///
/// Returns the preserved resource id (not `origin_uri`): `origin_uri` is NOT unique (empty for
/// CLI/agent-created resources — 166/1214 in the production corpus), so an origin_uri-keyed result
/// collapses every empty-`origin_uri` match onto one indistinguishable handle and the caller cannot
/// recover which resource matched. Synthesis preserves the production id verbatim, so the id is the
/// stable identity for every resource (WS6 flip real-corpus rehearsal finding — same root cause the
/// body path fixed by keying `readback::body` on the id).
///
/// The tsvector is `setweight(to_tsvector('english', title), 'A') || setweight(..body.., 'B')` —
/// title-only weight-A, body weight-B. This deliberately DIVERGES from production's
/// `rebuild_resource_search_vector` (migration 20260405000001), whose A-weight is `title || slug`: §7
/// dissolved slug, so §9 rebuilds FTS title-only. The body is the RAW current-chunk content
/// space-joined (`string_agg(content, ' ')`), exactly as production aggregates it — NOT the
/// heading-prefixed assembled markdown [`crate::content::reconstruct_body`] produces (that's
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
pub async fn fts_search(pool: &PgPool, principal: Uuid, query: &str) -> Result<Vec<Uuid>> {
    // Beat-1 hardcode: all rows use 'english' tsquery/tsvector today (every resource is indexed with
    // the 'english' config in `rebuild_resource_search_vector`). A future multilingual rollout will
    // store a per-row `search_config` in `kb_resource_search_index` — when that lands, this query
    // MUST read the stored config per row instead of hardcoding 'english', or non-English rows will
    // silently fail to match (the stored tsvector and the query will use mismatched configurations).
    let rows = sqlx::query(
        "SELECT r.id
           FROM kb_resource_search_index si
           JOIN kb_resources r             ON r.id = si.resource_id
           JOIN resources_visible_to($1) v ON v.resource_id = r.id
          WHERE r.is_active
            AND si.search_vector @@ plainto_tsquery('english', $2)
          ORDER BY ts_rank(si.search_vector, plainto_tsquery('english', $2)) DESC",
    )
    .bind(principal)
    .bind(query)
    .fetch_all(pool)
    .await?;

    Ok(rows.iter().map(|r| r.get::<Uuid, _>("id")).collect())
}

/// Format a `Vec<f32>` as a pgvector text literal (`[a,b,c]`) for binding into a `::vector` cast.
/// Inlined here (a tiny helper) rather than
/// reusing production's `temper_core::types::ingest::format_embedding`: temper-core is only a DEV-dep of
/// temper-substrate, not a lib dep, and pulling it into the lib just to format five floats would be
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

/// Cosine vector search over substrate chunks (§9 vector floor). Per resource, the best (min-cosine-
/// distance) current chunk decides rank; results ascend by that distance — exactly production's
/// `vec_hits` (MIN distance per resource, `ORDER BY MIN(embedding <=> query)`). Embeddings carry
/// verbatim from production (§8), so this ordered output matches production's vector search bit-for-bit
/// (contrast `fts_search`, where production's slug@A weight makes only the matching SET an invariant).
///
/// Returns the preserved resource **id** (not `origin_uri`) for the same reason as [`fts_search`]:
/// `origin_uri` is non-unique (empty for CLI/agent-created resources), so an origin_uri-keyed result
/// cannot identify which resource matched. The id is preserved verbatim by synthesis.
///
/// The query embedding is formatted to a pgvector text literal and bound into a `$1::vector` cast.
/// Runtime `sqlx::query` with the `::vector` cast is the ESTABLISHED pgvector-macro exception —
/// production's own `unified_search` uses runtime `query_as` for exactly this reason (the `query!`
/// macros don't support the `::vector` cast).
///
/// Read-only; no writes. Schema-qualified throughout — see the module-level note.
pub async fn vector_search(
    pool: &PgPool,
    principal: Uuid,
    query_embedding: &[f32],
) -> Result<Vec<Uuid>> {
    let embedding_text = format_pgvector(query_embedding);
    let rows = sqlx::query(
        "SELECT r.id
           FROM kb_resources r
           JOIN resources_visible_to($1) v ON v.resource_id = r.id
           JOIN kb_chunks c
             ON c.resource_id = r.id AND c.is_current
          GROUP BY r.id
          ORDER BY MIN(c.embedding <=> $2::vector) ASC",
    )
    .bind(principal)
    .bind(embedding_text)
    .fetch_all(pool)
    .await?;

    Ok(rows.iter().map(|r| r.get::<Uuid, _>("id")).collect())
}

/// One surface-tier region of a cognitive map, as returned by `cogmap_shape`. Centroid-derived
/// readouts only — member identities are NEVER carried (the interior is dereferenced per-member
/// through `resources_visible_to` elsewhere). Substrate-local because `temper-substrate` cannot
/// depend on `temper-core`; the `temper-api` wrapper maps this to the `CogmapRegionRow` wire type.
#[derive(Debug, Clone, PartialEq)]
pub struct CogmapShapeRow {
    pub region_id: Uuid,
    pub lens_id: Uuid,
    pub salience: f64,
    pub content_cohesion: Option<f64>,
    pub label: Option<String>,
    pub member_count: i32,
}

/// The surface-tier read of a cognitive map's materialized regions (spec §A surfacing; SQL
/// `cogmap_shape`). The access gate is INSIDE the SQL: a principal who cannot read the map gets zero
/// rows (never an error). Folded regions are excluded by the function; `lens_id = None` returns all
/// lenses, `Some(l)` narrows to that lens.
///
/// Runtime `sqlx::query` (NOT the `query!` macros) — the SQL is unqualified and self-gating; see the
/// module-level note. Read-only.
pub async fn cogmap_shape(
    pool: &PgPool,
    cogmap_id: Uuid,
    principal: Uuid,
    lens_id: Option<Uuid>,
) -> Result<Vec<CogmapShapeRow>> {
    let rows = sqlx::query(
        "SELECT region_id, lens_id, salience, content_cohesion, label, member_count
           FROM cogmap_shape($1, 'profile', $2, $3)",
    )
    .bind(cogmap_id)
    .bind(principal)
    .bind(lens_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|r| CogmapShapeRow {
            region_id: r.get("region_id"),
            lens_id: r.get("lens_id"),
            salience: r.get("salience"),
            content_cohesion: r.get("content_cohesion"),
            label: r.get("label"),
            member_count: r.get("member_count"),
        })
        .collect())
}

/// One 1-hop graph neighbor of a resource: the OTHER endpoint's origin_uri plus the connecting edge's
/// kind/polarity/label. The §9 graph-neighbors read floor over `kb_edges` (folded edges
/// excluded, matching production's `NOT is_folded` gate).
///
/// `label` is `Option<String>`: an empty production label carries as `NULL` through synthesis, so an
/// edge with no label surfaces here as `None` (never `Some("")`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Neighbor {
    /// The neighbor (other endpoint) resource's verbatim-carried `origin_uri` (NOT unique).
    pub origin_uri: String,
    /// The connecting edge's kind (`edge_kind::text`).
    pub edge_kind: String,
    /// The connecting edge's polarity (`polarity::text`), carried verbatim from production.
    pub polarity: String,
    /// The connecting edge's label, or `None` when absent (empty production label → `NULL`).
    pub label: Option<String>,
}

/// Port of production's 1-hop graph-neighbor read onto the substrate tables — the §9 graph-neighbors read
/// floor. Returns the resource↔resource neighbors of `new_id` over `kb_edges`, in BOTH
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
///
/// `pub(crate)`, NOT `pub`: this is an UNSCOPED read (no principal — see the body note), so exposing it
/// across the crate boundary would let any external caller pull cross-profile graph structure. Keeping
/// it crate-private closes that leak surface until the graph-neighbor surface is wired with the
/// access-model amendment that makes scoping correct.
#[expect(
    dead_code,
    reason = "§9 graph-neighbors read floor; wired (and scoped) with the graph-neighbor surface — see the body note. No caller today."
)]
pub(crate) async fn neighbors(
    executor: impl sqlx::PgExecutor<'_>,
    new_id: Uuid,
) -> Result<Vec<Neighbor>> {
    // WS2 NOTE — deliberately UNSCOPED (no principal). `neighbors` has no surface caller yet (only
    // the §9 data-parity test reads it), so visibility-scoping it now protects nothing (SG-5: no
    // speculative surface). The leak-safe gate is `edges_visible_to(principal)` (edge-home + both
    // endpoints). The profile-owned-context edge-home gap that this note previously flagged —
    // `anchor_readable_by_profile` admitting a context-homed edge only by team share/ownership, so an
    // owner could not traverse edges homed in their own PROFILE-owned context — is now CLOSED:
    // migration `20260627000003` added the profile-owned clause (mirroring `context_visible_to`
    // clause 1), so `edges_visible_to` admits an owner to the edges in their own context.
    let rows = sqlx::query(
        "SELECT t.origin_uri AS origin_uri, e.edge_kind::text AS edge_kind, \
                e.polarity::text AS polarity, e.label \
           FROM kb_edges e \
           JOIN kb_resources t ON t.id = e.target_id \
          WHERE e.source_id = $1 \
            AND e.source_table = 'kb_resources' AND e.target_table = 'kb_resources' \
            AND NOT e.is_folded \
         UNION ALL \
         SELECT s.origin_uri AS origin_uri, e.edge_kind::text AS edge_kind, \
                e.polarity::text AS polarity, e.label \
           FROM kb_edges e \
           JOIN kb_resources s ON s.id = e.source_id \
          WHERE e.target_id = $1 \
            AND e.source_table = 'kb_resources' AND e.target_table = 'kb_resources' \
            AND NOT e.is_folded",
    )
    .bind(new_id)
    .fetch_all(executor)
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

/// One scored hit from Surface A unified search (Beat 2). The scores are the real blended sub-scores —
/// the either/or path's 0.0 placeholders are gone.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ScoredHit {
    pub resource_id: Uuid,
    pub fts_score: f32,
    pub vector_score: f32,
    pub graph_score: f32,
    pub combined_score: f32,
}

/// Request parameters for [`unified_search`] (params struct — 11 domain fields). Borrowed views; the
/// caller owns the underlying `SearchParams`. Empty `seed_ids`/`edge_types` ⇒ no explicit seeds / all
/// edge kinds. `None` `query`/`embedding` ⇒ that signal's term is zeroed in the blend.
#[derive(Debug, Clone)]
pub struct UnifiedSearchQuery<'a> {
    pub principal: Uuid,
    pub query: Option<&'a str>,
    pub embedding: Option<&'a [f32]>,
    pub seed_ids: &'a [Uuid],
    pub depth: i32,
    pub edge_types: &'a [String],
    pub context_id: Option<Uuid>,
    pub doc_type: Option<&'a str>,
    pub graph_expand: bool,
    pub limit: i64,
    pub offset: i64,
}

/// Surface A general search (Beat 2): one composed SQL statement (`unified_search`) blending FTS +
/// vector + graph into ranked, scored hits. Runtime `sqlx::query_as` — the `::vector` cast forbids the
/// compile-time macros (module note). All tuning constants live in the SQL function, not here.
pub async fn unified_search(pool: &PgPool, q: UnifiedSearchQuery<'_>) -> Result<Vec<ScoredHit>> {
    let emb_text = q.embedding.map(format_pgvector);
    let edge_types: Vec<String> = q.edge_types.to_vec();
    let hits = sqlx::query_as::<_, ScoredHit>(
        "SELECT resource_id, fts_score, vector_score, graph_score, combined_score
           FROM unified_search($1, $2, $3::vector, $4::uuid[], $5, $6::text[], $7, $8, $9, $10::int, $11::int)",
    )
    .bind(q.principal)
    .bind(q.query)
    .bind(emb_text)        // NULL when None → p_emb NULL → vector term zeroed
    .bind(q.seed_ids)
    .bind(q.depth)
    .bind(edge_types)
    .bind(q.context_id)
    .bind(q.doc_type)
    .bind(q.graph_expand)
    .bind(q.limit)
    .bind(q.offset)
    .fetch_all(pool)
    .await?;
    Ok(hits)
}
