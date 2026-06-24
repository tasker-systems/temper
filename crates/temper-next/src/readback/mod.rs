//! WS6 §9 chunk-3 read surface over `temper_next.*` — read-only parity tooling.
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
//! for consistency). The SQL is UNQUALIFIED (`kb_*` / `resources_visible_to`) — post-collapse there is
//! one schema, and the connection carries its search_path (dev: `temper_next,public`; live: `public`
//! after the rename), so unqualified names and the visibility function's own unqualified internals all
//! resolve correctly with no per-txn `SET LOCAL`. The lone compile-time macro ([`find_by_body_hash`])
//! is likewise unqualified — its `.sqlx` cache is prepared with `search_path=temper_next`.

use std::collections::HashMap;

use anyhow::Result;
use serde_json::{Map, Value};
use sqlx::{PgPool, Row};
use uuid::Uuid;

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

/// One projected list row over `temper_next.*` — the readback counterpart of production's
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
/// `vault_resources_browse` view) onto `temper_next.*`: returns every synthesized resource (the §0
/// active set — synthesis never carries soft-deleted rows, so there is no `is_active` filter to apply
/// here) with the same projected fields production surfaces.
///
/// The doctype and the three workflow fields all live in `kb_properties` (synthesis writes
/// them via `facet_set`, plus the direct `doc_type` property the resource pass stamps). `doc_type` is an
/// inner JOIN — every synthesized resource has one; the workflow keys are LEFT JOINs so a resource
/// without them comes back with `NULL` (not dropped). Property values are JSON scalars, extracted to
/// text with `#>> '{}'` (the same extraction `synthesis::run`'s property test uses).
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
    // unqualified references against the connection search_path — which post-collapse points at the one
    // schema (dev: `temper_next,public`; live: `public` after the rename), so no per-txn `SET LOCAL`.
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

/// One enriched list row over `temper_next.*` — the readback counterpart of MCP's `EnrichedResource`
/// list projection (Task 6 assembles `EnrichedResource` from this). Like [`ListRow`] it carries the
/// row's display fields + the three workflow keys as typed columns, PLUS the full reconstructed
/// managed/open frontmatter split (so the MCP enrichment needs no per-row meta round-trip — the §7
/// managed/open split, same as [`meta`], rides along here in batch). The re-minted `new_id` is carried
/// for `EnrichedResource.id`; it is a §-non-invariant (never asserted equal across legacy↔next).
#[derive(Debug, Clone)]
pub struct EnrichedListRow {
    /// The synthesized resource id (re-minted; non-invariant; carried for `EnrichedResource.id`).
    pub new_id: Uuid,
    /// Verbatim-carried `origin_uri` (a provenance marker, NOT unique — empty for CLI/agent-created
    /// resources; the resource id is the stable identity).
    pub origin_uri: String,
    /// The resource title (`kb_resources.title`).
    pub title: String,
    /// Active flag — always true here (synthesis carries only active resources).
    pub is_active: bool,
    /// Home context display name.
    pub context_name: String,
    /// The authoritative doctype (the `doc_type` property the resource pass stamps).
    pub doc_type: String,
    /// `temper-stage`, if present.
    pub stage: Option<String>,
    /// `temper-mode`, if present.
    pub mode: Option<String>,
    /// `temper-effort`, if present.
    pub effort: Option<String>,
    /// Surviving managed (workflow + provenance) keys, verbatim (same §7 split as [`meta`]).
    pub managed: Map<String, Value>,
    /// Open (user-defined) keys, verbatim.
    pub open: Map<String, Value>,
}

/// Batched, context/doctype-filtered list projection for MCP enrichment — two queries, no N+1:
/// (1) the visible set (`resources_visible_to($1)`) with row + display fields + the `doc_type`
/// property (an INNER JOIN that also serves the doctype filter) and the three workflow keys as typed
/// columns; (2) one `owner_id = ANY($ids)` property scan to reconstruct the managed/open split per row
/// (the §7 inverse fate, reusing [`is_managed_property_key`], exactly as [`meta`] does for a single
/// resource). `resources_visible_to`'s unqualified internals (`profile_effective_teams`/
/// `team_ancestors`) resolve against the connection search_path — the one schema, post-collapse.
///
/// `doc_type` is surfaced as the typed column ONLY, never duplicated into managed/open — parity with
/// [`meta`], which reconciles it to the authoritative doctype field. The workflow keys
/// (`temper-stage`/`-mode`/`-effort`) DO appear in both the typed columns and the managed map (they are
/// managed keys that survive §7) — consistent with production's `ResourceRow` columns + `ManagedMeta`.
///
/// Filters are applied in SQL: `context_name` matches the home context's name, `doc_type` matches the
/// `doc_type` property value (both NULL-passthrough — a `None` filter matches every row). Ordered by
/// `(origin_uri, id)` for determinism (origin_uri is NOT unique — id is the tiebreaker), matching [`list`].
///
/// Read-only; no writes. Runtime, schema-qualified `sqlx::query` (NEVER the `query!` macros) — see the
/// module-level note.
pub async fn enriched_list(
    pool: &PgPool,
    principal: Uuid,
    context_name: Option<&str>,
    doc_type: Option<&str>,
) -> Result<Vec<EnrichedListRow>> {
    // Query 1: the visible, filtered set with display fields + doc_type (INNER JOIN) + workflow keys.
    let set_rows = sqlx::query(
        "SELECT r.id AS new_id,
                r.origin_uri,
                r.title,
                r.is_active,
                c.name AS context_name,
                dt.property_value #>> '{}' AS doc_type,
                st.property_value #>> '{}' AS stage,
                md.property_value #>> '{}' AS mode,
                ef.property_value #>> '{}' AS effort
           FROM kb_resources r
           JOIN resources_visible_to($1) v ON v.resource_id = r.id
           JOIN kb_resource_homes h ON h.resource_id = r.id
           JOIN kb_contexts c
             ON c.id = h.anchor_id AND h.anchor_table = 'kb_contexts'
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
          WHERE ($2::text IS NULL OR c.name = $2)
            AND ($3::text IS NULL OR dt.property_value #>> '{}' = $3)
          ORDER BY r.origin_uri, r.id",
    )
    .bind(principal)
    .bind(context_name)
    .bind(doc_type)
    .fetch_all(pool)
    .await?;

    let ids: Vec<Uuid> = set_rows
        .iter()
        .map(|r| r.get::<Uuid, _>("new_id"))
        .collect();

    // Query 2: one batched property scan for the surviving ids → managed/open reconstruction.
    let prop_rows = sqlx::query(
        "SELECT owner_id, property_key, property_value
           FROM kb_properties
          WHERE owner_table = 'kb_resources'
            AND owner_id = ANY($1)
            AND NOT is_folded",
    )
    .bind(&ids)
    .fetch_all(pool)
    .await?;

    // Group properties by owner; reuse the §7 managed/open inverse fate (same split as `meta`).
    // (managed, open) maps per owner.
    type MetaSplit = (Map<String, Value>, Map<String, Value>);
    let mut by_owner: HashMap<Uuid, MetaSplit> = HashMap::new();
    for pr in &prop_rows {
        let owner: Uuid = pr.get("owner_id");
        let key: String = pr.get("property_key");
        let value: Value = pr.get("property_value");
        if key == "doc_type" {
            // Surfaced as the typed column, not in managed/open (parity with `meta`).
            continue;
        }
        let entry = by_owner.entry(owner).or_default();
        if is_managed_property_key(&key) {
            entry.0.insert(key, value);
        } else {
            entry.1.insert(key, value);
        }
    }

    Ok(set_rows
        .iter()
        .map(|r| {
            let new_id: Uuid = r.get("new_id");
            let (managed, open) = by_owner.remove(&new_id).unwrap_or_default();
            EnrichedListRow {
                new_id,
                origin_uri: r.get("origin_uri"),
                title: r.get("title"),
                is_active: r.get("is_active"),
                context_name: r.get("context_name"),
                doc_type: r.get("doc_type"),
                stage: r.get("stage"),
                mode: r.get("mode"),
                effort: r.get("effort"),
                managed,
                open,
            }
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

/// The migration-invariant subset of production's `ResourceRow`, reconstructed from `temper_next.*`
/// for the full-row reads (`show` / `by_uri`). Excludes the non-invariant fields by construction:
/// re-minted identity UUIDs (resource id / context id / profile ids), §7-dissolved
/// `slug`/`managed_hash`/`open_hash`, and the synthesis-collapsed `created`/`updated`. The caller
/// (`NextBackend::show_resource`) supplies those from elsewhere (re-minted ids verbatim, a re-minted
/// nil `kb_doc_type_id` since §7 dissolved the typed id and `doc_type_name` is authoritative, `None`
/// for the dissolved fields, `Utc::now()` for the timestamps). See the WS6 4b spec parity-floor
/// amendment.
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

/// Reconstruct a resource's markdown body from its substrate chunks — the §9 body read floor.
/// Reuses [`crate::parity::reconstruct_body`] (the production `get_content` assembly) so the read
/// surface and the §8 synthesis gate share one algorithm (CONFORM, no second body assembler).
///
/// Reads the chunk tables UNQUALIFIED (like every sibling readback) so they resolve against the
/// connection's search_path / the single post-collapse schema — the `parity::new_substrate_chunks`
/// reader this used is hard-qualified to `temper_next` (the dark-launch dual-schema comparator) and
/// would not resolve once the substrate is the lone `public` schema.
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
    let chunks: Vec<crate::parity::ReadChunk> = rows
        .iter()
        .map(|row| crate::parity::ReadChunk {
            chunk_index: row.get("chunk_index"),
            header_path: row.get("header_path"),
            heading_depth: row.get("heading_depth"),
            content: row.get("content"),
        })
        .collect();
    Ok(crate::parity::reconstruct_body(&chunks))
}

/// Substrate body-hash dedup for the create path (WS6 collapse Task F). Returns the id of an existing
/// active, visible resource whose `body_hash` matches `body_hash`, or `None`. Mirrors production's
/// `ingest_service::find_by_body_hash` but keys on the substrate `kb_resources.body_hash` (the
/// structural sha256 merkle over chunk content-hashes, `_recompute_resource_body_hash`) instead of the
/// dead `kb_resource_manifests`. Visibility-gated through `resources_visible_to`, like every other read.
///
/// Unlike the rest of this module (runtime, schema-qualified `sqlx::query`), this is a **compile-time
/// macro** — the one in `readback`. The SQL is UNQUALIFIED (`kb_resources` / `resources_visible_to`):
/// the per-crate `.sqlx` cache is prepared with `search_path=temper_next` (`cargo make prepare-next`),
/// and at runtime the connection search_path points at the one schema post-collapse — so the unqualified
/// table and `resources_visible_to`'s own unqualified internals all resolve correctly.
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

/// Port of production's FTS read (`search_service::search`, FTS-only) onto `temper_next.*` — the §9
/// search read floor. Builds, per resource, the §9-REBUILT weighted tsvector and returns the matching
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
/// heading-prefixed assembled markdown [`crate::parity::reconstruct_body`] produces (that's
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
    let rows = sqlx::query(
        "WITH doc AS (
           SELECT r.id,
                  setweight(to_tsvector('english', r.title), 'A') ||
                  setweight(to_tsvector('english', COALESCE(string_agg(cc.content, ' '), '')), 'B')
                    AS search_vector
             FROM kb_resources r
             JOIN resources_visible_to($1) v ON v.resource_id = r.id
             LEFT JOIN kb_chunks c
               ON c.resource_id = r.id AND c.is_current
             LEFT JOIN kb_chunk_content cc
               ON cc.chunk_id = c.id
            GROUP BY r.id, r.title
         )
         SELECT id
           FROM doc
          WHERE search_vector @@ plainto_tsquery('english', $2)
          ORDER BY ts_rank(search_vector, plainto_tsquery('english', $2)) DESC",
    )
    .bind(principal)
    .bind(query)
    .fetch_all(pool)
    .await?;

    Ok(rows.iter().map(|r| r.get::<Uuid, _>("id")).collect())
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

/// Port of production's 1-hop graph-neighbor read onto `temper_next.*` — the §9 graph-neighbors read
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
pub async fn neighbors(pool: &PgPool, new_id: Uuid) -> Result<Vec<Neighbor>> {
    // WS2 NOTE — deliberately UNSCOPED (no principal). `neighbors` has no surface caller yet (only
    // the §9 data-parity test reads it), so visibility-scoping it now protects nothing (SG-5: no
    // speculative surface). The leak-safe gate is `edges_visible_to(principal)` (edge-home + both
    // endpoints), but that surfaced a real access-model gap: `anchor_readable_by_profile` gates a
    // context-homed edge ONLY by a `kb_team_contexts` share, and synthesis auto-shares only
    // TEAM-owned contexts — so an owner cannot traverse edges homed in their own PROFILE-owned
    // context even though `resources_visible_to` returns the resources. Closing that (a personal-team
    // share for profile-owned contexts, or an ownership branch in `anchor_readable_by_profile`) is an
    // access-model amendment that lands with the graph-neighbor SURFACE wiring — tracked, not silent
    // (no surface reads this today).
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
