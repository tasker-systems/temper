//! WS6 §9 chunk-3 read surface over the substrate tables — **the production read path**.
//!
//! It began as parity tooling: it ported the production reads (list / meta / body / FTS / vector /
//! neighbors) onto the synthesized substrate so each could be asserted against its production
//! counterpart for the same logical query. That harness — `crates/temper-next/tests/parity_reads.rs`
//! — was deleted with temper-next itself in the WS6 endgame collapse (`2fc0412e`, #166), and the
//! surfaces were wired straight here instead. So the comparison this module was built to enable no
//! longer exists, and every remaining caller is production: `temper-services`' `substrate_read.rs`
//! serves body / meta / unified search / wayfind / anchor shape / region metrics / cogmap analytics /
//! invocation show + list from here, `citation_audit_service` and `evidential_standing_service` the
//! evidential reads, and `authz/audit_gate` asks [`is_resource_visible`] for its readability half.
//! A change here changes what the API returns.
//!
//! **Access scoping (WS2).** The single-resource and set reads (`list`/`resource_row`/`meta`/`body`/
//! `fts_search`/`vector_search`) take a `principal` and gate through `resources_visible_to`,
//! CONFORMing to production's `resources_visible_to(profile)` JOIN and its not-visible→404 deny. A
//! not-visible single-resource read errors (the read selector maps that to NotFound, never 403); the set
//! reads JOIN-filter to the visible set. `neighbors` is the lone exception — deliberately UNSCOPED, as it
//! has no surface caller yet (see its note); the graph-neighbor scoping lands with that surface.
//! The `OWNER_PROFILE` prod-shape fixture this paragraph used to cite went with the parity harness;
//! what exercises the gate now is `temper-services`' `authz/audit_gate` suite, which asserts both
//! directions against a real database (e.g. `the_author_of_the_finding_is_refused` checks that
//! [`is_resource_visible`] returns `true` for the author — so that the refusal it then asserts is
//! provably the self-audit arm and not a readability failure).
//!
//! Reads are compile-time `query!`-family macros, so each leaves an entry in the workspace `.sqlx`
//! cache and is checked against the live schema at build time. A few are runtime `sqlx::query` /
//! `query_as` — [`vector_search`], [`search_wide`], [`wayfind_region_diagnostics`] — for one
//! allow-listed reason only: a `$n::vector` bind the macros cannot type.
//!
//! **That reason is the whole of the class.** A runtime read that cannot state it is drift, not an
//! exception, and belongs as a macro. Do not add one for tidiness, consistency, or because a
//! neighbour is runtime.
//!
//! **Count the members by reading the file, never by trusting a number written in prose** — here or
//! in any document describing this module. A restated count drifts silently as reads are added and
//! retired. Count them with `python3 scripts/classify-sqlx-calls.py`, which is the enumerator the
//! `audit-sqlx-macro-exceptions` CI gate itself runs, so the number you get is the number that
//! gates. A literal grep pattern for the call spelling is deliberately NOT written here: that
//! scanner matches the call head anywhere in the file and does not strip comments, so a pattern
//! quoted in this note counts itself and inflates this module's total by one.
//!
//! **The exemption must not spread.** Until 2026-07-30 sixteen further reads in this module were
//! runtime *"for consistency"* with the vector ones, which left them absent from the cache — and the
//! cache is the record a schema/binary change detector reads, so an exemption taken for tidiness was
//! subtracting from the coverage of a safety check. They were clawed back.
//!
//! A macro read states its own nullability, so several carry `!` / `?` overrides. Each one is a
//! claim about the SQL (a set-returning function's contract, a `COALESCE`, an INNER JOIN) and is
//! commented where it is not obvious. Getting one wrong is a real defect but not a catastrophic
//! one: `!` expands to `row.try_get_unchecked::<T, _>(i)?`
//! (`sqlx-macros-core-0.8.6/src/query/output.rs:147`), so a NULL arriving in a column declared
//! non-null is a decode **error** propagated through `?` — a 500, not a crash. That is strictly
//! better than the `row.get` it replaced, which panics on NULL (`try_get` is the fallible
//! spelling). The override is a claim to state carefully, not a loaded gun.
//!
//! The SQL is UNQUALIFIED (`kb_*` / `resources_visible_to`) — there is one schema
//! (`public`), and the connection's search_path resolves unqualified names and the visibility
//! function's own unqualified internals correctly with no per-txn `SET LOCAL`.

pub mod query_exec;
pub mod query_plan;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Map, Value};
use sqlx::{PgPool, Row};
use temper_core::types::home::HomeAnchor;
use uuid::Uuid;

use crate::ids::{
    BlockId, CogmapId, ContextId, EdgeId, EntityId, LensId, ProfileId, RegionId, ResourceId,
};
use crate::keys::{is_managed_property_key, MANAGED_PROPERTY_KEYS};
use temper_core::types::managed_meta::ManagedMeta;
use temper_core::types::resource::{BodyStorage, IngestState};
use temper_core::types::ResourceView;

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
        resource_id: ResourceId,
        /// The principal the visibility check ran against.
        principal: ProfileId,
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
/// resource-list projection (temper-services' `substrate_read::filtered_visible_page`) for the
/// fields that projection surfaces: the resource's `origin_uri` + `title`, its `doc_type`, and the
/// three workflow fields lifted from `managed_meta` (`temper-stage`/`temper-mode`/`temper-effort`).
/// Each workflow field is `None` when the resource carries no such property (R1/R3/R5 in the
/// prod-shape fixture), matching the `kb_resource_workflow_props` pivot's NULL for an absent key.
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

/// The one Rust-callable spelling of "can `principal` see `resource`?", under `resources_visible_to`.
///
/// This exists so `ensure_visible` (below — private, so deliberately not an intra-doc link from a
/// public item) and Task 6's authorization gate ask the identical
/// question instead of two independent restatements that could drift apart silently. It is also the
/// predicate the SQL side already treats as canonical for a profile principal:
/// `resources_readable_by('profile', p)` — the gate `resource_standing_shape` runs
/// (`20260724000120_standing_citation_components.sql:326`) — delegates to `resources_visible_to`
/// for the `'profile'` arm (`20260712000010_context_read_predicates.sql:419`), so calling
/// `resources_visible_to` directly here is equivalent for the profile principal, and is the
/// incumbent Rust-callable form.
///
pub async fn is_resource_visible(
    pool: &PgPool,
    principal: ProfileId,
    resource: ResourceId,
) -> Result<bool> {
    // `resources_visible_to` and its nested `profile_reachable_teams` → `profile_effective_teams`/
    // `team_ancestors` resolve their unqualified references against the connection search_path
    // (`public` — the one schema), so no per-txn `SET LOCAL`.
    //
    // `EXISTS` never yields SQL-NULL, but sqlx infers every expression column as nullable, so the
    // `!` override states what the operator guarantees rather than forcing an `unwrap_or` here.
    let visible = sqlx::query_scalar!(
        r#"SELECT EXISTS (
             SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id = $2
           ) AS "visible!""#,
        principal.uuid(),
        resource.uuid(),
    )
    .fetch_one(pool)
    .await?;
    Ok(visible)
}

/// WS2 consumer-axis gate for single-resource reads: error unless `new_id` is visible to
/// `principal` under `resources_visible_to` (delegates the check itself to
/// [`is_resource_visible`]). The set reads (`list`/`fts_search`/`vector_search`/`neighbors`) instead
/// JOIN the function directly (a set can't be pre-checked).
///
/// Returns a TYPED [`ReadbackError`]: [`ReadbackError::NotVisible`] when the principal can't see the
/// resource (the caller maps it to 404 — denying existence, never 403, no existence-leak oracle,
/// CONFORMing to production's NotFound-on-not-visible) versus [`ReadbackError::Fault`] for a genuine DB
/// error in the check itself (mapped to 500). Conflating the two — the pre-typing behavior — masked
/// real faults as 404.
async fn ensure_visible(
    pool: &PgPool,
    principal: ProfileId,
    new_id: ResourceId,
) -> std::result::Result<(), ReadbackError> {
    let visible = is_resource_visible(pool, principal, new_id).await?;
    if visible {
        Ok(())
    } else {
        Err(ReadbackError::NotVisible {
            resource_id: new_id,
            principal,
        })
    }
}

/// Port of production's resource-list projection (temper-services'
/// `substrate_read::filtered_visible_page`) onto the substrate tables: returns every synthesized
/// resource (the §0 active set — synthesis never carries soft-deleted rows, so there is no
/// `is_active` filter to apply here) with the same projected fields production surfaces.
///
/// This doc comment was stranded on [`is_resource_visible`] until the `ResourceView` convergence
/// moved it back onto the function it describes. It also cited a `vault_resources_browse` view as
/// production's source; that view was dropped by the WS6 endgame collapse (`2fc0412e`) when the
/// migration baseline was rebuilt, and production's list has read the same `kb_properties` +
/// `kb_resource_workflow_props` pair this function reads ever since.
///
/// The doctype and the three workflow fields all live in `kb_properties` (synthesis writes
/// them via `facet_set`, plus the direct `doc_type` property the resource pass stamps). `doc_type` is an
/// inner JOIN — every synthesized resource has one; the workflow keys come from a LEFT JOIN on the
/// `kb_resource_workflow_props` pivot view (migration `20260709000002` — the one statement of the
/// per-key pivot, shared with [`resource_row`] and temper-services' `filtered_visible_page`), so a
/// resource without them comes back with `NULL` (not dropped). Property values are JSON scalars,
/// extracted to text with `#>> '{}'` (the doc-type-as-property extraction).
///
/// Ordered by `(origin_uri, id)` so the result is deterministic — `origin_uri` alone is NOT unique
/// (empty for CLI/agent-created resources), so the resource id is the tiebreaker. It is deliberately NOT
/// ordered by `updated`: synthesis sources `kb_resources.created`/`updated` from the genesis event's
/// `occurred_at`, which is `now()` = transaction-start time and therefore identical across every row
/// written in the single synthesis transaction. Absolute recency ordering is not a migration-time
/// invariant (event-sourced backfill collapses timestamps to synthesis time); the row set + projected
/// fields are.
pub async fn list(pool: &PgPool, principal: ProfileId) -> Result<Vec<ListRow>> {
    // `doc_type!`: the JOIN is INNER, so the row exists; `#>> '{}'` is an expression, which sqlx
    // infers as nullable regardless. The override states the same thing the pre-macro
    // `row.get::<String, _>("doc_type")` asserted silently.
    let rows = sqlx::query!(
        r#"SELECT r.origin_uri,
                  r.title,
                  dt.property_value #>> '{}' AS "doc_type!",
                  wp.stage,
                  wp.mode,
                  wp.effort
             FROM kb_resources r
             JOIN resources_visible_to($1) v ON v.resource_id = r.id
             JOIN kb_properties dt
               ON dt.owner_table = 'kb_resources' AND dt.owner_id = r.id
              AND dt.property_key = 'doc_type' AND NOT dt.is_folded
             LEFT JOIN kb_resource_workflow_props wp ON wp.resource_id = r.id
            ORDER BY r.origin_uri, r.id"#,
        principal.uuid(),
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| ListRow {
            origin_uri: row.origin_uri,
            title: row.title,
            doc_type: row.doc_type,
            stage: row.stage,
            mode: row.mode,
            effort: row.effort,
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
/// Read-only; no writes.
pub async fn meta(
    pool: &PgPool,
    principal: ProfileId,
    new_id: ResourceId,
) -> std::result::Result<ReconstructedMeta, ReadbackError> {
    ensure_visible(pool, principal, new_id).await?;
    let rows = sqlx::query!(
        r#"SELECT property_key, property_value
             FROM kb_properties
            WHERE owner_table = 'kb_resources' AND owner_id = $1 AND NOT is_folded
            ORDER BY created, id"#,
        new_id.uuid(),
    )
    .fetch_all(pool)
    .await?;

    classify_properties(
        new_id,
        rows.into_iter()
            .map(|row| (row.property_key, row.property_value)),
    )
}

/// The batched [`meta`] — the managed/open/doc_type split for MANY resources in ONE statement.
///
/// Exists because the per-resource loop above it was the surviving N+1 on the list surface: the
/// `open-meta` section fill ran `meta` once per page row (and `meta` itself is two statements, its
/// own `ensure_visible` plus the read), so a 13-row page asking for the open tier cost **28
/// statements, measured** — the same defect [`hit_identities`] was written to end for search, one
/// section down. It is 3 now.
///
/// Visibility is gated by the `resources_visible_to` JOIN rather than by `ensure_visible`, matching
/// the set reads in this module: a resource the principal cannot see is simply ABSENT from the map,
/// never an error. That is the right convention here — every caller has already read the resource's
/// identity through a visibility-gated read, so a miss means the row disappeared between the two
/// statements, which is a dropped row rather than a fault.
///
/// The classification is `classify_properties`, shared verbatim with [`meta`]: the `facet` merge
/// and the newest-wins rule are subtle enough that a second copy would drift, and a drift here means
/// two doors disagreeing about what a resource's open tier IS.
pub async fn meta_batch(
    pool: &PgPool,
    principal: ProfileId,
    ids: &[ResourceId],
) -> std::result::Result<std::collections::HashMap<ResourceId, ReconstructedMeta>, ReadbackError> {
    if ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let raw: Vec<Uuid> = ids.iter().map(|id| id.uuid()).collect();
    // `ORDER BY owner_id` first so each resource's rows arrive contiguously; `created, id` within a
    // resource is `meta`'s ordering verbatim, which is what makes newest-wins mean the same thing.
    let rows = sqlx::query!(
        r#"SELECT pr.owner_id, pr.property_key, pr.property_value
             FROM kb_properties pr
             -- `AND v.resource_id = ANY($2)` is REDUNDANT AND LOAD-BEARING (see `hit_identities`,
             -- which carries the same pairing and the full argument). Do not delete it.
             JOIN resources_visible_to($1) v
               ON v.resource_id = pr.owner_id AND v.resource_id = ANY($2)
            WHERE pr.owner_table = 'kb_resources'
              AND pr.owner_id = ANY($2)
              AND NOT pr.is_folded
            ORDER BY pr.owner_id, pr.created, pr.id"#,
        principal.uuid(),
        &raw,
    )
    .fetch_all(pool)
    .await?;

    let mut grouped: Vec<(ResourceId, Vec<(String, Value)>)> = Vec::new();
    for row in rows {
        let id = ResourceId::from(row.owner_id);
        match grouped.last_mut() {
            Some((last, props)) if *last == id => {
                props.push((row.property_key, row.property_value));
            }
            _ => grouped.push((id, vec![(row.property_key, row.property_value)])),
        }
    }

    grouped
        .into_iter()
        .map(|(id, props)| Ok((id, classify_properties(id, props)?)))
        .collect()
}

/// The §7 fate split for one resource's property rows — the **one** implementation, shared by
/// [`meta`] (one statement per resource) and [`meta_batch`] (one statement for many).
///
/// `rows` must arrive in `created, id` order, which is what makes the last-write-wins inserts below
/// mean *newest wins*.
fn classify_properties(
    new_id: ResourceId,
    rows: impl IntoIterator<Item = (String, Value)>,
) -> std::result::Result<ReconstructedMeta, ReadbackError> {
    let mut managed = Map::new();
    let mut open = Map::new();
    let mut doc_type: Option<String> = None;

    // ORDER BY created, id + last-write-wins on the inserts below = newest wins.
    // Without the ordering the planner serves this from `uq_kb_properties_active`,
    // whose key order is (owner_table, owner_id, property_key, property_value) — so
    // rows arrive sorted by VALUE and the jsonb-largest wins, which is only
    // coincidentally the newest. `id` is a uuidv7 tiebreak for rows written in one
    // transaction.
    //
    // **`facet` MERGES rather than overwrites**, and it is the one key that does.
    // Since 20260730000010 a facet is stored one row per inner key, so the plain
    // last-row-wins below would disclose a single mark — `{"status": "resolved"}` —
    // where the whole set is `{status, node_label, as_of}`. That would be a *reads-as-
    // complete* answer, which is the defect the facet read
    // (`GET /api/resources/{id}/facets`) shipped to end, reappearing one layer down.
    // Merging is also strictly more disclosure than this read has ever given: before
    // the grain change it surfaced one ROW of several, discarding the siblings.
    //
    // Per-mark weight still does not appear here — `open_meta` is a key→value map with
    // nowhere to put it. The facet read remains the faithful door; this one is honest
    // about the marks, not about their strength.
    for (key, value) in rows {
        if key == "doc_type" {
            // The authoritative doctype is a JSON string scalar; surface it as the typed field.
            doc_type = Some(match value {
                Value::String(s) => s,
                other => other.to_string(),
            });
        } else if is_managed_property_key(&key) {
            managed.insert(key, value);
        } else if key == "facet" {
            // Newest-wins *per inner key*: rows arrive oldest-first, so a later mark for the
            // same key overwrites the earlier one while leaving unrelated marks in place.
            // A non-object mark (the pre-grain sentinel shape) has no inner key to merge on,
            // so it takes the whole slot, exactly as it did before.
            match (open.get_mut(&key), &value) {
                (Some(Value::Object(acc)), Value::Object(incoming)) => {
                    for (k, v) in incoming {
                        acc.insert(k.clone(), v.clone());
                    }
                }
                _ => {
                    open.insert(key, value);
                }
            }
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
    pub re_minted_id: ResourceId,
    /// The synthesized home-anchor context id (re-minted; not a parity invariant).
    /// `None` when the resource is homed in a cognitive map (no `kb_contexts` row).
    pub re_minted_context_id: Option<ContextId>,
    /// The synthesized owner profile id (re-minted; not a parity invariant).
    pub owner_profile_id: ProfileId,
    /// The synthesized originator profile id (re-minted; not a parity invariant).
    pub originator_profile_id: ProfileId,
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
    /// Are all the bytes here? `complete` | `in_progress` — see `ResourceRow::ingest_state`. NOT a
    /// parity invariant: it is born from the create payload's `segmented` flag and advanced by
    /// `resource_finalized`, both of which replay reproduces.
    pub ingest_state: String,
    /// Home context display name (invariant). `None` for a cogmap-homed resource.
    pub context_name: Option<String>,
    /// Authoritative doctype name (invariant) — the `doc_type` property.
    pub doc_type_name: String,
    /// Owner profile handle (invariant).
    pub owner_handle: String,
    /// Slug of the home context (the natural-key half of `@owner/slug`). Invariant.
    /// `None` for a cogmap-homed resource.
    pub context_slug: Option<String>,
    /// Already-sigil'd owner of the home context (`@<handle>` or `+<team-slug>`). Invariant.
    /// `None` for a cogmap-homed resource.
    pub context_owner_ref: Option<String>,
    /// Home cognitive map id — `Some` when the resource is cogmap-homed (Surface B).
    pub cogmap_id: Option<Uuid>,
    /// Home cognitive map display name — `Some` iff `cogmap_id` is `Some`.
    pub cogmap_name: Option<String>,
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
    /// What guarantee the body carries on read — `verbatim` | `derived` (see `ResourceRow::body_storage`).
    /// NOT a parity invariant: it is DERIVED from block content-coverage by the projectors, which replay
    /// reproduces (it recomputes over the same live block set as it re-projects the create/append acts).
    pub body_storage: String,
}

/// Port of production's full-row read (`resource_service::get_visible` / `resolve_by_uri`, behind
/// `show` / `by_uri`) onto the substrate tables, at the §9 INVARIANT-FIELD floor. Joins the home (which
/// anchors the context and owner profile), the `doc_type` property, and the workflow properties.
/// Selects `created`/`updated` from `kb_resources` — populated from `kb_events.occurred_at` at write
/// time (create sets both to genesis event's `occurred_at`; updates bump `updated`).
///
/// Read-only; no writes.
pub async fn resource_row(
    pool: &PgPool,
    principal: ProfileId,
    new_id: ResourceId,
) -> std::result::Result<ResourceRowParity, ReadbackError> {
    ensure_visible(pool, principal, new_id).await?;
    // The `?` overrides on the two LEFT-JOINed anchors are the load-bearing ones: a resource is
    // homed in EITHER a context or a cogmap, so exactly one side is NULL on every row. The `!` pair
    // covers the CASE/`#>>` expressions, which sqlx infers as nullable because they are expressions,
    // not because either can actually be absent (`context_owner_ref` is guarded by its own `?`).
    let row = sqlx::query!(
        r#"SELECT r.id              AS "re_minted_id!",
                  r.origin_uri,
                  r.title,
                  r.is_active,
                  r.created,
                  r.updated,
                  r.body_hash,
                  r.ingest_state,
                  r.body_storage,
                  c.id              AS "re_minted_context_id?",
                  c.name            AS "context_name?",
                  c.slug            AS "context_slug?",
                  CASE c.owner_table
                    WHEN 'kb_teams' THEN '+' || (SELECT slug   FROM kb_teams    WHERE id = c.owner_id)
                    ELSE                   '@' || (SELECT handle FROM kb_profiles WHERE id = c.owner_id)
                  END               AS "context_owner_ref?",
                  cm.id             AS "cogmap_id?",
                  cm.name           AS "cogmap_name?",
                  h.owner_profile_id,
                  h.originator_profile_id,
                  p.handle          AS owner_handle,
                  dt.property_value #>> '{}' AS "doc_type_name!",
                  wp.stage,
                  wp.mode,
                  wp.effort,
                  wp.seq
             FROM kb_resources r
             JOIN kb_resource_homes h ON h.resource_id = r.id
             LEFT JOIN kb_contexts c
               ON c.id = h.anchor_id AND h.anchor_table = 'kb_contexts'
             LEFT JOIN kb_cogmaps cm
               ON cm.id = h.anchor_id AND h.anchor_table = 'kb_cogmaps'
             JOIN kb_profiles p ON p.id = h.owner_profile_id
             JOIN kb_properties dt
               ON dt.owner_table = 'kb_resources' AND dt.owner_id = r.id
              AND dt.property_key = 'doc_type' AND NOT dt.is_folded
             LEFT JOIN kb_resource_workflow_props wp ON wp.resource_id = r.id
            WHERE r.id = $1"#,
        new_id.uuid(),
    )
    .fetch_one(pool)
    .await?;

    let seq = match row.seq {
        Some(s) => Some(s.parse::<i64>().map_err(|e| {
            anyhow::anyhow!("temper-seq {s:?} is not an i64 for resource {new_id}: {e}")
        })?),
        None => None,
    };

    Ok(ResourceRowParity {
        re_minted_id: ResourceId::from(row.re_minted_id),
        re_minted_context_id: row.re_minted_context_id.map(ContextId::from),
        owner_profile_id: ProfileId::from(row.owner_profile_id),
        originator_profile_id: ProfileId::from(row.originator_profile_id),
        origin_uri: row.origin_uri,
        title: row.title,
        is_active: row.is_active,
        created: row.created,
        updated: row.updated,
        context_name: row.context_name,
        context_slug: row.context_slug,
        context_owner_ref: row.context_owner_ref,
        cogmap_id: row.cogmap_id,
        cogmap_name: row.cogmap_name,
        doc_type_name: row.doc_type_name,
        owner_handle: row.owner_handle,
        stage: row.stage,
        mode: row.mode,
        effort: row.effort,
        seq,
        body_hash: row.body_hash,
        ingest_state: row.ingest_state,
        body_storage: row.body_storage,
    })
}

/// The heading breadcrumb a newly-appended segment resumes from: the `header_path` of the last
/// chunk of the highest-`seq` live block. A server-chunked segment seeds its chunker with this so
/// its chunks carry ancestor headings across the block boundary (see
/// [`crate::content::prepare_block_with_prefix`]).
///
/// Empty when the resource has no live blocks, or when its trailing chunk is an unheaded preamble
/// (NULL `header_path`) — in both cases the appended segment simply starts from no ancestors.
///
/// Not access-gated: the sole caller (`DbBackend::append_block`) has already passed
/// `can_modify_resource`, and this returns no prose.
pub async fn trailing_breadcrumb(pool: &PgPool, resource: ResourceId) -> Result<Vec<String>> {
    let path: Option<String> = sqlx::query_scalar!(
        r#"
        SELECT c.header_path
          FROM kb_content_blocks b
          JOIN kb_chunks c ON c.block_id = b.id AND c.is_current
         WHERE b.resource_id = $1 AND NOT b.is_folded
         ORDER BY b.seq DESC, c.chunk_index DESC
         LIMIT 1
        "#,
        resource.uuid(),
    )
    .fetch_optional(pool)
    .await?
    .flatten();

    Ok(path
        .map(|p| p.split(" > ").map(str::to_owned).collect())
        .unwrap_or_default())
}

/// Read a resource's markdown body — **verbatim when coverage is complete, derived otherwise** (the
/// §9 body read floor, and the sole body assembler behind `GET /api/resources/{id}/content`).
///
/// Two paths, tried in order:
///
/// 1. **Verbatim (W2 PR 3/4).** `string_agg` of the stored raw block bytes (`kb_block_content`,
///    per-revision), in `seq` order, with `''` as the separator — the bytes already carry their own
///    line terminators (PR 2's byte-exact segmenter), so blocks concat with nothing between them and
///    the result is `sha256`-identical to what was uploaded. The `HAVING count(*) =
///    count(bc.block_revision_id)` is the load-bearing guard: it makes a **short body impossible**. A
///    resource with *any* content-less live block — a legacy pre-PR-3 row, or one with only partial
///    verbatim coverage — returns **NO row** here (never a plausible-looking partial concat, which is
///    exactly how an INNER JOIN would silently truncate), so `None` correctly falls through. An
///    empty-block resource passes the `HAVING` (`0 = 0`) but `string_agg` over zero rows is SQL-NULL,
///    which `.flatten()` maps to `None` — also correctly derived.
///
/// 2. **Derived (legacy / partial coverage).** The unchanged chunk reconstruction via
///    [`crate::content::reconstruct_body`] — a LOSSY transform (CRLF→LF, trimmed, headings
///    re-synthesized), the one and only fallback assembler (CONFORM, no second). Survives solely to
///    serve `derived` rows; `verbatim` rows never reach it.
///
/// Reads the tables UNQUALIFIED (like every sibling readback) so they resolve against the connection's
/// search_path / the single post-collapse `public` schema. Keys by `new_id` directly — the resource id
/// (preserved verbatim from production). Read-only; no writes.
pub async fn body(
    pool: &PgPool,
    principal: ProfileId,
    new_id: ResourceId,
) -> std::result::Result<String, ReadbackError> {
    ensure_visible(pool, principal, new_id).await?;

    // Path 1: verbatim, coverage-verified. `.flatten()` collapses both "no row" (partial coverage,
    // HAVING excludes it) and "row with NULL agg" (zero live blocks) to `None` → the derived fallback.
    // The macro's own nullability is what makes that double-Option explicit rather than incidental.
    let verbatim = sqlx::query_scalar!(
        r#"SELECT string_agg(bc.content, '' ORDER BY b.seq)
             FROM kb_content_blocks b
             LEFT JOIN kb_block_content bc ON bc.block_revision_id = b.current_revision_id
            WHERE b.resource_id = $1 AND NOT b.is_folded
           HAVING count(*) = count(bc.block_revision_id)"#,
        new_id.uuid(),
    )
    .fetch_optional(pool)
    .await?
    .flatten();
    if let Some(body) = verbatim {
        return Ok(body);
    }

    // Path 2: derived reconstruction (legacy pre-PR-3 rows, or partial verbatim coverage).
    // The two `!` overrides are the COALESCE defaults: neither expression can yield NULL, but sqlx
    // infers every expression column as nullable.
    let rows = sqlx::query!(
        r#"SELECT c.chunk_index,
                  COALESCE(c.header_path, '')            AS "header_path!",
                  COALESCE(c.heading_depth, 0::smallint) AS "heading_depth!",
                  cc.content
             FROM kb_chunks c
             JOIN kb_content_blocks b ON b.id = c.block_id
             JOIN kb_chunk_content cc ON cc.chunk_id = c.id
            WHERE c.resource_id = $1 AND c.is_current
            ORDER BY b.seq, c.chunk_index"#,
        new_id.uuid(),
    )
    .fetch_all(pool)
    .await?;
    let chunks: Vec<crate::content::ReadChunk> = rows
        .into_iter()
        .map(|row| crate::content::ReadChunk {
            chunk_index: row.chunk_index,
            header_path: row.header_path,
            heading_depth: row.heading_depth,
            content: row.content,
        })
        .collect();
    Ok(crate::content::reconstruct_body(&chunks))
}

/// The telos resource id + its current body merkle for a cogmap — the charter reconcile diff source.
/// `body_hash` is `sha256_hex("")` for a fresh genesis telos (empty block set, the SQL's empty-aggregate
/// coalesce) — never SQL-NULL once the resource exists — or the structural merkle of the current charter.
#[derive(Debug, Clone)]
pub struct TelosCharterState {
    pub telos_resource_id: ResourceId,
    pub body_hash: Option<String>,
}

/// Resolve `cogmap` → telos resource, returning its current `body_hash` for the charter diff. Compile-time
/// macro (like [`kernel_slice`]; resolves against `public`, workspace `.sqlx` cache).
pub async fn telos_charter_state(
    conn: &mut sqlx::PgConnection,
    cogmap: CogmapId,
) -> Result<TelosCharterState> {
    let row = sqlx::query!(
        r#"SELECT t.id        AS "telos_resource_id!",
                  t.body_hash AS "body_hash?"
             FROM kb_cogmaps c
             JOIN kb_resources t ON t.id = c.telos_resource_id
            WHERE c.id = $1"#,
        cogmap.uuid(),
    )
    .fetch_one(&mut *conn)
    .await?;
    Ok(TelosCharterState {
        telos_resource_id: row.telos_resource_id.into(),
        body_hash: row.body_hash,
    })
}

/// One row of L0's reconcile diff source: a `provenance: kernel` resource homed to a cogmap, keyed
/// (by the caller) on `resource_id`, carrying its body merkle and merged facet object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelSliceRow {
    /// The diff key — the resource's STABLE id (UUIDv7). The reconcile applier keys its diff index on
    /// this and addresses blocks/edges by it. `origin_uri` is loose provenance, NEVER the key.
    pub resource_id: ResourceId,
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
/// Compile-time `query!` macro (like [`telos_charter_state`]; most reads in this module are runtime
/// `sqlx::query`): the SQL resolves against the single `public` schema, cached in the workspace
/// `.sqlx` (post-`temper_next`-elimination, #178).
pub async fn kernel_slice(
    executor: impl sqlx::PgExecutor<'_>,
    cogmap_id: CogmapId,
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
        cogmap_id.uuid(),
    )
    .fetch_all(executor)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| KernelSliceRow {
            resource_id: ResourceId::from(row.resource_id),
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
    src: ResourceId,
    tgt: ResourceId,
    kind: &crate::affinity::EdgeKind,
    polarity: Option<&str>,
) -> Result<Option<EdgeId>> {
    let id = sqlx::query_scalar!(
        r#"SELECT id FROM kb_edges
            WHERE source_id = $1 AND target_id = $2
              AND source_table = 'kb_resources' AND target_table = 'kb_resources'
              AND edge_kind::text = $3 AND NOT is_folded
              AND ($4::text IS NULL OR polarity::text = $4)"#,
        src.uuid(),
        tgt.uuid(),
        kind.as_sql(),
        polarity,
    )
    .fetch_optional(executor)
    .await?;
    Ok(id.map(EdgeId::from))
}

/// Port of production's FTS read (`search_service::search`, FTS-only) onto the substrate tables — the §9
/// search read floor. Reads the stored `kb_resource_search_index` tsvector and returns the matching
/// resource **ids** ranked by `ts_rank DESC`.
///
/// The query is parsed with `websearch_to_tsquery` (issue #356) — mirroring the `search_exact` SQL
/// function, which the exact arm of `/api/search` runs — so `"quoted phrases"`, `OR`, and `-negation` are
/// expressible. Plain unquoted input parses identically to `plainto_tsquery`, so this is fully
/// backward-compatible (the `fts_search_parity_with_inline_recipe` test keeps a `plainto_tsquery`
/// oracle precisely to pin that equivalence).
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
/// Read-only; no writes.
pub async fn fts_search(
    pool: &PgPool,
    principal: ProfileId,
    query: &str,
) -> Result<Vec<ResourceId>> {
    // Beat-1 hardcode: all rows use 'english' tsquery/tsvector today (every resource is indexed with
    // the 'english' config in `rebuild_resource_search_vector`). A future multilingual rollout will
    // store a per-row `search_config` in `kb_resource_search_index` — when that lands, this query
    // MUST read the stored config per row instead of hardcoding 'english', or non-English rows will
    // silently fail to match (the stored tsvector and the query will use mismatched configurations).
    let ids = sqlx::query_scalar!(
        r#"SELECT r.id
             FROM kb_resource_search_index si
             JOIN kb_resources r             ON r.id = si.resource_id
             JOIN resources_visible_to($1) v ON v.resource_id = r.id
            WHERE r.is_active
              AND si.search_vector @@ websearch_to_tsquery('english', $2)
            ORDER BY ts_rank(si.search_vector, websearch_to_tsquery('english', $2)) DESC"#,
        principal.uuid(),
        query,
    )
    .fetch_all(pool)
    .await?;

    Ok(ids.into_iter().map(ResourceId::from).collect())
}

/// Format a `Vec<f32>` as a pgvector text literal (`[a,b,c]`) for binding into a `::vector` cast.
/// Inlined here (a tiny helper) rather than
/// reusing production's `temper_core::types::ingest::format_embedding`: temper-substrate depends on
/// temper-core only for the shared id newtypes, and reaching into its ingest module just to format five
/// floats would be over-coupling. Uses `{}` (not `{:?}`) so each float renders without a debug wrapper.
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
/// production's own [`search_wide`] uses runtime `query_as` for exactly this reason (the `query!`
/// macros don't support the `::vector` cast).
///
/// Read-only; no writes. Schema-qualified throughout — see the module-level note.
pub async fn vector_search(
    pool: &PgPool,
    principal: ProfileId,
    query_embedding: &[f32],
) -> Result<Vec<ResourceId>> {
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

    Ok(rows.iter().map(|r| r.get::<ResourceId, _>("id")).collect())
}

/// One surface-tier region of an anchor, as returned by `anchor_shape`. Centroid-derived
/// readouts only — member identities are NEVER carried (the interior is dereferenced per-member
/// through `resources_visible_to` elsewhere). Substrate-local: the `temper-services` wrapper maps
/// this to the `CogmapRegionRow` wire type, which is anchor-neutral in payload despite its name
/// (the `cogmap_*` naming goes away at M3).
#[derive(Debug, Clone, PartialEq)]
pub struct CogmapShapeRow {
    pub region_id: RegionId,
    pub lens_id: LensId,
    pub salience: f64,
    pub content_cohesion: Option<f64>,
    pub label: Option<String>,
    /// Over the members **this caller can read** (spec §D5), not the stored count — the same gate the
    /// `label` already honors. Having declined to name a member the caller cannot see, we do not
    /// count it either.
    pub member_count: i32,
}

/// The surface-tier read of an anchor's materialized regions — a context OR a cogmap (spec §3.7,
/// T8; SQL `anchor_shape`). The access gate is INSIDE the SQL: a principal who cannot read the
/// anchor gets zero rows (never an error). Folded regions are excluded by the function;
/// `lens_id = None` returns all lenses, `Some(l)` narrows to that lens. Rows come back most-salient
/// first.
///
/// Anchor-generic because the region table is keyed on the anchor pair, not on the vestigial
/// `cogmap_id` — which is NULL for every context region and so made the old cogmap-keyed read
/// structurally blind to them.
///
/// The SQL is unqualified and self-gating; see the module-level note. Read-only.
///
/// Every column of a set-returning function reads as nullable to sqlx, so the `!` overrides carry
/// the function's own contract: `region_id`/`lens_id`/`salience`/`member_count` are always present
/// on a returned row, while `content_cohesion` and `label` are genuinely optional and stay `Option`.
pub async fn anchor_shape(
    pool: &PgPool,
    anchor: HomeAnchor,
    principal: ProfileId,
    lens_id: Option<LensId>,
) -> Result<Vec<CogmapShapeRow>> {
    let rows = sqlx::query!(
        r#"SELECT region_id     AS "region_id!",
                  lens_id       AS "lens_id!",
                  salience      AS "salience!",
                  content_cohesion,
                  label,
                  member_count  AS "member_count!"
             FROM anchor_shape($1, $2, 'profile', $3, $4)"#,
        anchor.table(),
        anchor.uuid(),
        principal.uuid(),
        lens_id.map(LensId::uuid),
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| CogmapShapeRow {
            region_id: RegionId::from(r.region_id),
            lens_id: LensId::from(r.lens_id),
            salience: r.salience,
            content_cohesion: r.content_cohesion,
            label: r.label,
            member_count: r.member_count,
        })
        .collect())
}

/// One finding's evidential-standing shape, as returned by `resource_standing_shape` (Set 5,
/// migration `20260724000120`, replacing Set 3's single `indep_breadth` scalar with three citation-
/// grain axes — spec §3.1). Substrate-local: the `temper-services` wrapper maps this to the
/// `StandingShape` wire type (Phase C, later PR). Standing is shape-primary (spec §1.1) — `band` is
/// a lossy read-time chip carried WITH the shape, never in place of it.
#[derive(Debug, Clone, PartialEq)]
pub struct StandingShapeRow {
    pub finding_id: ResourceId,
    /// Findability axis (spec §3.1): count of DISTINCT LIVE cited sources. Monotone — citing more
    /// evidence never lowers it. NOT `r_parent`, which counts every provenance row including
    /// duplicates of the same source.
    pub citation_magnitude: i32,
    /// Evaluated-ness axis (spec §3.1): count of those distinct sources carrying at least one
    /// citation audit. Monotone under the append-only audit trail — once a source is audited it
    /// stays covered.
    pub audit_coverage: i32,
    /// Quality axis (spec §3.1): the mean, over the AUDITED SUBSET ONLY, of each audited source's
    /// decay-weighted audit value, in `[-1.0, 1.0]`. `0.0` when `audit_coverage == 0` — an
    /// unaudited finding makes no quality claim; its low standing comes from the band gate, not a
    /// poisoned mean.
    pub citation_quality: f64,
    pub contradiction_balance: f64,
    pub freshness: f64,
    pub r_parent: f64,
    pub band: String,
}

/// Access-gated read of a finding's evidential-standing shape (SQL `resource_standing_shape`). The
/// gate is INSIDE the SQL (a `gated` CTE over `resources_readable_by`): a principal who cannot read
/// the finding gets ZERO rows, never an error — surfaced here as `None`. Components are recomputed
/// live (never read from the `kb_resource_standing` memo) so `freshness` reflects the current
/// moment; the memo/refresh exists for the write-path clock, not as this read's authority.
///
/// The SQL is unqualified and self-gating; see the module-level note. Read-only. The `!` overrides
/// carry the function's contract — a returned row has every axis; the *absence* of a row is the
/// deny/absent signal, and that is `Option` on the outside.
pub async fn resource_standing(
    pool: &PgPool,
    principal: ProfileId,
    finding: ResourceId,
) -> Result<Option<StandingShapeRow>> {
    let row = sqlx::query!(
        r#"SELECT finding_id            AS "finding_id!",
                  citation_magnitude    AS "citation_magnitude!",
                  audit_coverage        AS "audit_coverage!",
                  citation_quality      AS "citation_quality!",
                  contradiction_balance AS "contradiction_balance!",
                  freshness             AS "freshness!",
                  r_parent              AS "r_parent!",
                  band                  AS "band!"
             FROM resource_standing_shape($1, 'profile', $2)"#,
        finding.uuid(),
        principal.uuid(),
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| StandingShapeRow {
        finding_id: ResourceId::from(r.finding_id),
        citation_magnitude: r.citation_magnitude,
        audit_coverage: r.audit_coverage,
        citation_quality: r.citation_quality,
        contradiction_balance: r.contradiction_balance,
        freshness: r.freshness,
        r_parent: r.r_parent,
        band: r.band,
    }))
}

/// One citation audit of a finding, with its auditor's identity joined in — as returned by
/// `resource_citation_audit_trail` (Set 5 per-auditor collapse, migration `20260724000220`).
/// Substrate-local, the same split [`StandingShapeRow`] above uses: this struct is the DB read's
/// shape, and `CitationAuditRow` (`temper_core::types::citation_audit`) is the API contract — serde
/// field names, utoipa/ts-rs derives, the JSON a client parses. The `temper-services` wrapper maps
/// one to the other. NOT a dependency constraint: `temper-substrate` does depend on `temper-core`
/// (`crates/temper-substrate/Cargo.toml:23`) and re-exports its id newtypes (`crate::ids`). Keeping
/// the two apart is what lets the wire contract change without the readback changing with it.
///
/// This is the ATTRIBUTION that [`StandingShapeRow`]'s scalars collapse away: `audit_coverage` and
/// `citation_quality` are aggregates over exactly these rows, so two auditors disagreeing about one
/// citation reach the shape read as a single blended number and reach this read as two rows with two
/// different `auditor_profile_id`s.
#[derive(Debug, Clone, PartialEq)]
pub struct CitationAuditRecord {
    /// `kb_citation_audits.id`. A masked surrogate — the replay-stable identity of an audit is its
    /// `audited_by_event_id` (`20260724000110_citation_audits.sql:32-34`), so treat this as an
    /// addressing handle for this response only, never as a durable key.
    pub audit_id: Uuid,
    /// The audited citation's block. A citation is a `(block, source)` pair, so both halves are
    /// carried — a source cited by two of the finding's blocks is two auditable citations.
    pub block_id: BlockId,
    /// The audited citation's cited source. Always resource-kind: only resource-kind citations are
    /// auditable (`20260724000110_citation_audits.sql`, the `citation_audit` kind guard), which is
    /// why no `source_kind` is carried.
    pub source_id: ResourceId,
    /// The auditor's signed verdict in `[-1.0, 1.0]` (CHECK-constrained on the ledger column).
    pub value: f64,
    /// The auditor's optional free-text rationale.
    pub reason: Option<String>,
    /// When the audit was emitted — `kb_citation_audits.created`, which the projector stamps from
    /// the OWNING EVENT's `occurred_at`, never `now()`. It is the age the decay weight in
    /// `resource_citation_quality` is computed from, so it is the row's weight, not decoration.
    pub created: DateTime<Utc>,
    /// The auditor: `kb_citation_audits.audited_by_profile_id` (Beat 1). NOT the citer — the
    /// self-audit rule is precisely that the two must differ.
    pub auditor_profile_id: ProfileId,
    /// The auditor's `kb_profiles.handle` (unique, `NOT NULL`).
    pub auditor_handle: String,
    /// The auditor's `kb_profiles.display_name` (`NOT NULL`).
    pub auditor_display_name: String,
}

/// Access-gated read of a finding's citation-audit trail (SQL `resource_citation_audit_trail`). The
/// gate is INSIDE the SQL — the same `gated` CTE over `resources_readable_by` that
/// [`resource_standing`] runs, carried over byte-for-byte — so an unreadable finding yields ZERO
/// rows, never an error.
///
/// **An empty `Vec` here is three-way ambiguous by construction**: unreadable, absent, or readable
/// with no audits. That is deliberate at this layer (it is what makes the read leak-safe); a caller
/// that must tell those apart asks the readability question separately via
/// [`is_resource_visible`] — the Rust-callable spelling of the same predicate — rather than reading
/// a discriminator out of this function. `temper-services`' `citation_audit_service` does exactly
/// that.
///
/// Rows are the audits the standing axes actually summed (joined through `resource_live_citations`
/// on the full citation key), most recent first.
///
/// The SQL is unqualified and self-gating; see the module-level note. Read-only. `reason` is the one
/// genuinely optional column (a verdict need carry no rationale); the rest are always present on a
/// returned row, which is what the `!` overrides state.
pub async fn citation_audit_trail(
    pool: &PgPool,
    principal: ProfileId,
    finding: ResourceId,
) -> Result<Vec<CitationAuditRecord>> {
    let rows = sqlx::query!(
        r#"SELECT audit_id             AS "audit_id!",
                  block_id             AS "block_id!",
                  source_id            AS "source_id!",
                  value                AS "value!",
                  reason,
                  created              AS "created!",
                  auditor_profile_id   AS "auditor_profile_id!",
                  auditor_handle       AS "auditor_handle!",
                  auditor_display_name AS "auditor_display_name!"
             FROM resource_citation_audit_trail($1, 'profile', $2)"#,
        finding.uuid(),
        principal.uuid(),
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| CitationAuditRecord {
            audit_id: r.audit_id,
            block_id: BlockId::from(r.block_id),
            source_id: ResourceId::from(r.source_id),
            value: r.value,
            reason: r.reason,
            created: r.created,
            auditor_profile_id: ProfileId::from(r.auditor_profile_id),
            auditor_handle: r.auditor_handle,
            auditor_display_name: r.auditor_display_name,
        })
        .collect())
}

/// One act folded under an invocation envelope's show projection: a `kb_events` row stamped with this
/// invocation's id (`kb_events.invocation_id`). Substrate-local because `temper-substrate` cannot depend
/// on `temper-core`; the `temper-api` wrapper maps this to the `InvocationActRow` wire type.
#[derive(Debug, Clone, PartialEq)]
pub struct InvocationActRecord {
    pub event_id: Uuid,
    pub event_kind: String,
    pub emitter_entity_id: Uuid,
    pub occurred_at: DateTime<Utc>,
    /// The invocation this act is correlated under (`kb_events.invocation_id`). Always `Some` here
    /// (the acts query is keyed on it), exposed so the wire row is self-describing.
    pub invocation_id: Option<Uuid>,
    /// Raw `kb_events.metadata` — the per-act agent authorship (`{}` for an unauthored act). The
    /// `temper-api` wrapper decodes it into the typed `AgentAuthorship`.
    pub metadata: Value,
}

/// The full show projection of an invocation envelope: the `kb_invocations` row (minus the internal
/// ledger pointers `opened_by_event_id`/`closed_by_event_id`) plus its acts. Substrate-local; the
/// `temper-api` wrapper maps this to the `InvocationView` wire type.
#[derive(Debug, Clone, PartialEq)]
pub struct InvocationShowRow {
    pub id: Uuid,
    pub status: String,
    pub trigger_kind: String,
    pub originating_cogmap_id: Uuid,
    pub parent_cogmap_id: Option<Uuid>,
    pub scoped_entity_id: Uuid,
    pub telos_resource_id: Uuid,
    pub outcome: Option<Value>,
    pub opened_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub correlation_id: Option<Uuid>,
    pub acts: Vec<InvocationActRecord>,
}

/// The lighter list-row projection of an invocation envelope (no acts). Substrate-local; the
/// `temper-api` wrapper maps this to the `InvocationSummary` wire type.
#[derive(Debug, Clone, PartialEq)]
pub struct InvocationListRow {
    pub id: Uuid,
    pub status: String,
    pub trigger_kind: String,
    pub originating_cogmap_id: Uuid,
    pub opened_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
    pub correlation_id: Option<Uuid>,
}

/// The show read of one invocation envelope plus its acts. The access gate is INSIDE the SQL: the
/// envelope row is returned ONLY when the principal can read the originating cogmap
/// (`anchor_readable_by_profile($principal, 'kb_cogmaps', i.originating_cogmap_id)`). A denied or absent
/// invocation yields `Ok(None)` — never an error (leak-safe, like [`anchor_shape`]). Acts are fetched
/// ONLY after the envelope passes the gate, so an unreadable invocation never leaks its acts.
///
/// Unqualified, self-gating SQL; see the module-level note. Read-only.
pub async fn invocation_show(
    pool: &PgPool,
    invocation_id: Uuid,
    principal: ProfileId,
) -> Result<Option<InvocationShowRow>> {
    let Some(row) = sqlx::query!(
        r#"SELECT i.id, i.status, i.trigger_kind, i.originating_cogmap_id, i.parent_cogmap_id,
                  i.scoped_entity_id, i.telos_resource_id, i.outcome, i.opened_at, i.closed_at,
                  i.correlation_id
             FROM kb_invocations i
            WHERE i.id = $1
              AND anchor_readable_by_profile($2, 'kb_cogmaps', i.originating_cogmap_id)"#,
        invocation_id,
        principal.uuid(),
    )
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };

    let acts = sqlx::query!(
        r#"SELECT e.id, et.name, e.emitter_entity_id, e.occurred_at, e.invocation_id, e.metadata
             FROM kb_events e
             JOIN kb_event_types et ON et.id = e.event_type_id
            WHERE e.invocation_id = $1
            ORDER BY e.occurred_at, e.id"#,
        invocation_id,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| InvocationActRecord {
        event_id: r.id,
        event_kind: r.name,
        emitter_entity_id: r.emitter_entity_id,
        occurred_at: r.occurred_at,
        invocation_id: r.invocation_id,
        metadata: r.metadata,
    })
    .collect();

    Ok(Some(InvocationShowRow {
        id: row.id,
        status: row.status,
        trigger_kind: row.trigger_kind,
        originating_cogmap_id: row.originating_cogmap_id,
        parent_cogmap_id: row.parent_cogmap_id,
        scoped_entity_id: row.scoped_entity_id,
        telos_resource_id: row.telos_resource_id,
        outcome: row.outcome,
        opened_at: row.opened_at,
        closed_at: row.closed_at,
        correlation_id: row.correlation_id,
        acts,
    }))
}

/// The list read of invocation envelopes, every row gated by the same `anchor_readable_by_profile`
/// predicate as [`invocation_show`] (a principal who cannot read an invocation's originating cogmap
/// never sees its row — empty, never an error). Optionally narrowed by originating `cogmap` and/or
/// `status`; ordered newest-open-first.
///
/// Unqualified, self-gating SQL; see the module-level note. Read-only.
pub async fn invocation_list(
    pool: &PgPool,
    principal: ProfileId,
    cogmap: Option<Uuid>,
    status: Option<String>,
) -> Result<Vec<InvocationListRow>> {
    let rows = sqlx::query!(
        r#"SELECT i.id, i.status, i.trigger_kind, i.originating_cogmap_id, i.opened_at, i.closed_at,
                  i.correlation_id
             FROM kb_invocations i
            WHERE anchor_readable_by_profile($1, 'kb_cogmaps', i.originating_cogmap_id)
              AND ($2::uuid IS NULL OR i.originating_cogmap_id = $2)
              AND ($3::text IS NULL OR i.status = $3)
            ORDER BY i.opened_at DESC"#,
        principal.uuid(),
        cogmap,
        status,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| InvocationListRow {
            id: r.id,
            status: r.status,
            trigger_kind: r.trigger_kind,
            originating_cogmap_id: r.originating_cogmap_id,
            opened_at: r.opened_at,
            closed_at: r.closed_at,
            correlation_id: r.correlation_id,
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
/// table + `NOT is_folded` gate + `edge_kind`/`polarity`/`label` projection) — NOT a
/// subgraph-over-a-node-set read (one that returns the edges among a passed node set), which would be
/// circular as a 1-hop neighbor oracle. The parity test writes that production query directly.
///
/// 1-hop ONLY (the §9 neighbors floor) — there is deliberately NO `depth`/multi-hop traversal param:
/// the tested floor and the production neighbor read are both 1-hop; multi-hop is a kernel concern
/// beyond this parity task (SG-5, no speculative surface). Order is NOT contractual — the parity test
/// compares neighbor SETS.
///
/// Read-only; no writes.
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
    new_id: ResourceId,
) -> Result<Vec<Neighbor>> {
    // WS2 NOTE — deliberately UNSCOPED (no principal). `neighbors` has no surface caller yet (only
    // the §9 data-parity test reads it), so visibility-scoping it now protects nothing (SG-5: no
    // speculative surface). The leak-safe gate is `edges_visible_to(principal)` (edge-home + both
    // endpoints). The profile-owned-context edge-home gap that this note previously flagged —
    // `anchor_readable_by_profile` admitting a context-homed edge only by team share/ownership, so an
    // owner could not traverse edges homed in their own PROFILE-owned context — is now CLOSED:
    // migration `20260627000003` added the profile-owned clause (mirroring `context_visible_to`
    // clause 1), so `edges_visible_to` admits an owner to the edges in their own context.
    // A UNION takes its column names from the FIRST branch, so the overrides live there. The two
    // `::text` casts are expressions (nullable to sqlx) over `NOT NULL` enum columns; `origin_uri`
    // is `NOT NULL` on `kb_resources` and reads as nullable only because it arrives through a UNION.
    // `label` is the one genuinely absent-able column and stays `Option`.
    let rows = sqlx::query!(
        r#"SELECT t.origin_uri     AS "origin_uri!",
                  e.edge_kind::text AS "edge_kind!",
                  e.polarity::text  AS "polarity!",
                  e.label
             FROM kb_edges e
             JOIN kb_resources t ON t.id = e.target_id
            WHERE e.source_id = $1
              AND e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
              AND NOT e.is_folded
            UNION ALL
           SELECT s.origin_uri, e.edge_kind::text, e.polarity::text, e.label
             FROM kb_edges e
             JOIN kb_resources s ON s.id = e.source_id
            WHERE e.target_id = $1
              AND e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
              AND NOT e.is_folded"#,
        new_id.uuid(),
    )
    .fetch_all(executor)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| Neighbor {
            origin_uri: row.origin_uri,
            edge_kind: row.edge_kind,
            polarity: row.polarity,
            label: row.label,
        })
        .collect())
}

/// One hit from the **exact** arm (`search_exact`), carrying that arm's own quantity and no other.
///
/// There is deliberately no companion score here. The arm is ordered by `fts_norm` alone, and a
/// second number on this row would be something to rank it against.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ExactHit {
    pub resource_id: ResourceId,
    /// `ts_rank` with flag 33 — 32's `rank/(rank+1)` normalization plus flag 1's log-length
    /// division — so it lies in `[0,1)`.
    pub fts_norm: f32,
}

/// One hit from the **wide** arm (`search_wide`).
///
/// `vec_norm` rescales the pgvector cosine DISTANCE (which spans `[0,2]`) as `1 - d/2`, landing in
/// `[0,1]`. Note that `wayfind_region_scores.query_cos` rescales the SAME operator as `1 - d`,
/// spanning `[-1,1]` — the two are not one quantity and neither column name says so.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct WideHit {
    pub resource_id: ResourceId,
    pub vec_norm: f32,
}

/// What both arms take besides their own retrieval input.
///
/// A params struct because the alternative is seven positional arguments per arm, four of which are
/// `Option`s of the same shape — and because `limit`/`offset` mean the same thing to both arms and
/// should not be able to drift apart in two signatures.
#[derive(Debug, Clone, Copy)]
pub struct ArmQuery<'a> {
    pub principal: ProfileId,
    /// The anchor pair, `None` ⇒ unscoped.
    pub anchor: Option<HomeAnchor>,
    /// Filter on the resource's `doc_type` property. `None` ⇒ no filter.
    pub doc_type: Option<&'a str>,
    /// Rows to fetch. `None` ⇒ unbounded, which is `LIMIT NULL` in SQL.
    ///
    /// **The read path deliberately asks for one MORE row than it will return.** That extra row is
    /// how an arm tells "you have paged past the end" from "nothing matched" without paying for a
    /// second count — the two are indistinguishable from an empty page alone, and reporting
    /// `NoMatch` with "try rephrasing" to a caller who simply walked off the end is a wrong answer
    /// to the question they asked.
    pub limit: Option<i32>,
    pub offset: i32,
}

/// The exact arm. Scope is the anchor pair, taken as a [`HomeAnchor`] so the `(table, id)` literal is
/// derived in one place rather than at each call site — `None` ⇒ unscoped.
///
/// Ordering and paging are the SQL function's, not this caller's: `ORDER BY fts_norm DESC,
/// resource_id` then `LIMIT`/`OFFSET`. The tiebreak is part of the contract — without a total order
/// a tied row can appear on two pages or on none.
pub async fn search_exact(
    pool: &PgPool,
    query: Option<&str>,
    arm: ArmQuery<'_>,
) -> Result<Vec<ExactHit>> {
    // Compile-time `query!`, unlike its sibling [`search_wide`]: this arm binds no vector, so the
    // module note's `::vector` exemption never covered it and a runtime read here would sit outside
    // the `.sqlx` cache for no stated reason — which is the drift that note exists to prevent.
    //
    // Both `!` overrides restate what the function's own `RETURNS TABLE(resource_id uuid, fts_norm
    // real)` already guarantees: sqlx types every column of a set-returning function as nullable
    // regardless of the declaration, so without them the row fields would arrive as `Option`.
    let rows = sqlx::query!(
        r#"SELECT resource_id AS "resource_id!", fts_norm AS "fts_norm!"
             FROM search_exact($1, $2, $3, $4, $5, $6, $7)"#,
        arm.principal.uuid(),
        query,
        arm.anchor.map(|a| a.table()),
        arm.anchor.map(|a| a.uuid()),
        arm.doc_type,
        arm.limit,
        arm.offset,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| ExactHit {
            resource_id: ResourceId::from(row.resource_id),
            fts_norm: row.fts_norm,
        })
        .collect())
}

/// The wide arm. Runtime `sqlx::query_as` — the `::vector` cast forbids the compile-time macros
/// (module note).
///
/// `k` bounds the ANN draw in CHUNKS on the UNSCOPED branch only; the scoped branch is exhaustive
/// and ignores it. [`ArmQuery::limit`] is a different unit — it bounds the answer in RESOURCES,
/// after aggregation, on both branches. Collapsing the two would change what the shrinkage factor
/// is computed over.
///
/// `hnsw.ef_search` is pinned on the SQL function at or above the `k` used here — see
/// `search_wide_pins_ef_search_at_or_above_the_k_it_is_asked_for`. Raising `k` past the pin
/// re-introduces the silent truncation the pin exists to prevent.
pub async fn search_wide(
    pool: &PgPool,
    embedding: Option<&[f32]>,
    k: i32,
    arm: ArmQuery<'_>,
) -> Result<Vec<WideHit>> {
    let emb_text = embedding.map(format_pgvector);
    let hits = sqlx::query_as::<_, WideHit>(
        "SELECT resource_id, vec_norm FROM search_wide($1, $2::vector, $3, $4, $5, $6, $7, $8)",
    )
    .bind(arm.principal)
    .bind(emb_text) // NULL when None → p_emb NULL → the arm returns nothing
    .bind(k)
    .bind(arm.anchor.map(|a| a.table()))
    .bind(arm.anchor.map(|a| a.uuid()))
    .bind(arm.doc_type)
    .bind(arm.limit)
    .bind(arm.offset)
    .fetch_all(pool)
    .await?;
    Ok(hits)
}

/// Enrich a set of resource ids in ONE round-trip, visibility-gated.
///
/// This replaces a per-hit `resource_row` call — 50 results meant 51 queries. Both arms feed the
/// same call because a resource's identity does not depend on which arm found it.
///
/// A resource the principal cannot see is simply absent from the result, never an error: the arms
/// already gate visibility inside SQL, so a miss here would mean the row disappeared between the two
/// statements, which is a dropped hit rather than a fault.
///
/// Answers in [`ResourceView`] — the one shape a resource has — rather than in a search-local
/// projection, so an `ExactHit` and a list row describe the same resource identically. Two fields
/// are deliberately absent on every view this returns: `content` (the body is a section, and
/// reconstructing markdown for every hit is what this call exists NOT to do) and `open_meta` (the
/// open tier is a section too; `None` means *not requested*, never "empty").
///
/// The managed tier is not a section, so it is joined in here. It arrives as one `jsonb` object per
/// resource from a LATERAL aggregate — **still one statement**, which is the property this function
/// was written to hold. The key set the aggregate filters on is bound from
/// [`crate::keys::MANAGED_PROPERTY_KEYS`], not spelled into the SQL, so the managed/open split has
/// the same single source here as [`meta`]'s [`is_managed_property_key`] inverse.
///
/// # The id predicate is stated twice, and the second one is not redundant in practice
///
/// `$2` is bound to the *same* array in the `WHERE` and on the `resources_visible_to` join. The
/// second is implied by the first — `v.resource_id = r.id` and `r.id = ANY($2)` give
/// `v.resource_id = ANY($2)` — so it cannot change the admitted set. **It changes the plan by a
/// factor of 13, and Postgres will not derive it.**
///
/// The planner propagates a qual across an equijoin only through an *equivalence class*, and an
/// equivalence class is built from mergejoinable equality operators. `= ANY(array)` is a
/// `ScalarArrayOpExpr`, not an equality operator, so it never joins the class that
/// `v.resource_id = r.id` creates and nothing carries it to the other side. Given the predicate
/// directly, the planner pushes it into all six arms of the gate's `UNION` and the gate builds
/// only these rows; denied it, the gate materializes the principal's **entire visible set** and
/// hash-joins the answer out of it.
///
/// Measured on production (Neon PG 17, 3,559 resources, principal with 3,322 visible):
///
/// | shape | gate builds | execution |
/// |---|---|---|
/// | `r.id = ANY($2)` alone, 1 id | 3,477 rows | 5.31 ms |
/// | + the predicate on the gate, 1 id | 1 row | **0.40 ms** |
/// | `r.id = ANY($2)` alone, 20 ids | 3,477 rows | 7.72 ms |
/// | + the predicate on the gate, 20 ids | 20 rows | **2.32 ms** |
///
/// The same statement with a plain `r.id = $1` instead of `= ANY` costs 0.38 ms with **no** manual
/// duplication — the planner propagates it for free, which is the control that identifies the
/// operator class as the cause rather than the gate's shape.
///
/// So the cost arrived *with* this function: it uses `= ANY` because it is the set-shaped batch
/// read that ended search's per-hit `resource_row` N+1 ("50 results meant 51 queries"). Batching
/// the round-trips is what stopped the qual propagating.
///
/// **This property is not gated by a test, and cannot easily be.** It is invisible below corpus
/// scale — on a `#[sqlx::test]` database holding a handful of resources the two plans cost the
/// same, so a test asserting it would pass whether or not the predicate is there. What the tests
/// *do* hold is the half that can be held: that the admitted row set is unchanged
/// (`the_redundant_gate_predicate_does_not_change_who_can_see_what`). The performance half rests
/// on the comment in the SQL and on this note.
pub async fn hit_identities(
    pool: &PgPool,
    principal: ProfileId,
    ids: &[ResourceId],
) -> Result<Vec<ResourceView>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let raw: Vec<Uuid> = ids.iter().map(|r| r.uuid()).collect();
    let managed_keys: Vec<String> = MANAGED_PROPERTY_KEYS
        .iter()
        .map(|k| (*k).to_string())
        .collect();
    // The `?` overrides on the LEFT-JOINed anchors are the load-bearing ones (as in
    // `resource_row`): a resource is homed in EITHER a context or a cogmap, so exactly one side is
    // NULL on every row. `managed` is `?` because `jsonb_object_agg` over an empty set is NULL —
    // a resource with no managed property at all.
    let rows = sqlx::query!(
        r#"SELECT r.id AS "id!",
                  r.title,
                  r.origin_uri,
                  r.is_active,
                  r.created,
                  r.updated,
                  r.body_hash,
                  r.ingest_state,
                  r.body_storage,
                  h.owner_profile_id,
                  h.originator_profile_id,
                  p.handle                   AS owner_handle,
                  dt.property_value #>> '{}' AS "doc_type_name!",
                  c.id                       AS "kb_context_id?",
                  c.name                     AS "context_name?",
                  cm.id                      AS "cogmap_id?",
                  cm.name                    AS "cogmap_name?",
                  c.slug                     AS "context_slug?",
                  CASE c.owner_table
                    WHEN 'kb_teams' THEN '+' || (SELECT slug   FROM kb_teams    WHERE id = c.owner_id)
                    WHEN 'kb_profiles' THEN '@' || (SELECT handle FROM kb_profiles WHERE id = c.owner_id)
                    ELSE NULL
                  END                        AS "context_owner_ref?",
                  mm.managed                 AS "managed?"
             FROM kb_resources r
             JOIN kb_resource_homes h ON h.resource_id = r.id
             -- `AND v.resource_id = ANY($2)` is REDUNDANT AND LOAD-BEARING. Do not delete it.
             -- See the doc comment's "the predicate is stated twice" note: it is implied by
             -- `v.resource_id = r.id` + the WHERE below, and the planner cannot derive it,
             -- because `= ANY` forms no equivalence class. Without it the gate builds the
             -- principal's ENTIRE visible set to answer for these ids.
             JOIN resources_visible_to($1) v
               ON v.resource_id = r.id AND v.resource_id = ANY($2)
             LEFT JOIN kb_contexts c
               ON c.id = h.anchor_id AND h.anchor_table = 'kb_contexts'
             LEFT JOIN kb_cogmaps cm
               ON cm.id = h.anchor_id AND h.anchor_table = 'kb_cogmaps'
             JOIN kb_profiles p ON p.id = h.owner_profile_id
             JOIN kb_properties dt
               ON dt.owner_table = 'kb_resources' AND dt.owner_id = r.id
              AND dt.property_key = 'doc_type' AND NOT dt.is_folded
             LEFT JOIN LATERAL (
               SELECT jsonb_object_agg(newest.property_key, newest.property_value) AS managed
                 FROM (SELECT DISTINCT ON (mp.property_key)
                              mp.property_key, mp.property_value
                         FROM kb_properties mp
                        WHERE mp.owner_table = 'kb_resources' AND mp.owner_id = r.id
                          AND NOT mp.is_folded AND mp.property_key = ANY($3)
                        ORDER BY mp.property_key, mp.created DESC, mp.id DESC) newest
             ) mm ON TRUE
            WHERE r.id = ANY($2)"#,
        principal.uuid(),
        &raw,
        &managed_keys,
    )
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| {
            // Newest-wins per key is done by the `DISTINCT ON` above, matching `meta`'s
            // ORDER BY + last-write-wins. `deny_unknown_fields` on `ManagedMeta` is what makes the
            // bound key filter load-bearing rather than an optimization: an open key reaching here
            // is an error, not a silently-carried extra.
            let managed_meta: ManagedMeta = match row.managed {
                Some(v) => serde_json::from_value(v).map_err(|e| {
                    anyhow::anyhow!(
                        "managed meta for resource {} is not a ManagedMeta: {e}",
                        row.id
                    )
                })?,
                None => ManagedMeta::default(),
            };
            Ok(ResourceView {
                id: ResourceId::from(row.id),
                // Filled by `with_derived_refs` below — neither is a column.
                r#ref: String::new(),
                title: row.title,
                origin_uri: row.origin_uri,
                kb_context_id: row.kb_context_id.map(ContextId::from),
                context_name: row.context_name,
                context_slug: row.context_slug,
                context_owner_ref: row.context_owner_ref,
                context_ref: None,
                cogmap_id: row.cogmap_id,
                cogmap_name: row.cogmap_name,
                doc_type_name: row.doc_type_name,
                owner_handle: row.owner_handle,
                owner_profile_id: ProfileId::from(row.owner_profile_id),
                originator_profile_id: ProfileId::from(row.originator_profile_id),
                is_active: row.is_active,
                created: row.created,
                updated: row.updated,
                body_hash: row.body_hash,
                // Both columns are CHECK-constrained, so `from_wire` is total in practice; an
                // unparseable value is a schema violation, surfaced as None rather than coerced.
                ingest_state: IngestState::from_wire(&row.ingest_state),
                body_storage: BodyStorage::from_wire(&row.body_storage),
                managed_meta,
                // Both are sections this read does not serve — see the doc comment.
                open_meta: None,
                content: None,
            }
            .with_derived_refs())
        })
        .collect()
}

/// Request parameters for [`wayfind_region_diagnostics`] (params struct). Borrowed embedding view;
/// the caller owns it. `None` `lens_id` ⇒ each region's memoized salience; `Some` ⇒ recompute under
/// that lens's `s_*`. `None` `embedding` ⇒ the query-cosine term is zeroed (salience-only). `None`
/// `regions` ⇒ the SQL `k`-CTE default. `None` `anchor` ⇒ pool regions from **every** visible anchor
/// over both kinds; `Some` ⇒ "wayfind within this anchor" (spec §3.7). A `Some` anchor the principal
/// cannot read is not an error — it simply admits no rows, because the pool is still intersected with
/// `visible_region_anchors` inside the SQL.
#[derive(Debug, Clone)]
pub struct WayfindScopeQuery<'a> {
    pub principal: ProfileId,
    pub lens_id: Option<LensId>,
    pub embedding: Option<&'a [f32]>,
    pub regions: Option<i32>,
    pub anchor: Option<HomeAnchor>,
}

/// One candidate region's Stage-1 wayfind competition signal, as returned by
/// `wayfind_region_diagnostics` (issue #585). These are the per-region scores `wayfind_region_scores`
/// computes — surfaced here keyed by map so the cross-map competition is observable.
/// Substrate-local; read-only instrumentation, not a wire type.
///
/// There is ONE scoring home (`wayfind_region_scores`) and this is its only read surface, so
/// `in_top_n` is the flag that function itself computes rather than a mirror of it. Selection is
/// per-map round-robin over `region_score = α·sal_norm + β·query_cos + κ·prior`, so `in_top_n`'s
/// per-map distribution is the witness for "no single map monopolizes the top-N".
#[derive(Debug, Clone, PartialEq)]
pub struct WayfindRegionDiagnosticRow {
    /// The candidate region (`kb_cogmap_regions.id`).
    pub region_id: Uuid,
    /// The region's home anchor kind — `kb_cogmaps` or `kb_contexts`. Together with
    /// [`Self::home_anchor_id`], THE MAP the region belongs to: the key the monopoly is read against.
    pub home_anchor_table: String,
    /// The region's home anchor id (the cogmap/context UUID).
    pub home_anchor_id: Uuid,
    /// The home anchor's display name (cogmap/context `name`), resolved so a sweep reads by map rather
    /// than by UUID. `None` only if the anchor row was concurrently removed (defensive).
    pub home_anchor_name: Option<String>,
    /// Per-kind salience normalization (`percent_rank` within the anchor kind), in `[0,1]`. The axis
    /// the richest map is penalized on (#585): many small regions cluster at the bottom of the shared
    /// cogmap partition.
    pub sal_norm: f64,
    /// Cosine of the region centroid vs the query embedding, NaN-guarded to `0.0`. The dominant term
    /// (β=0.6) and the one a broad concept map wins against technical-vocabulary maps.
    pub query_cos: f64,
    /// The composite `α·sal_norm + β·query_cos + κ·prior` the top-N cut is applied to.
    pub region_score: f64,
    /// Whether this region cleared the Stage-1 top-N cut for this query. The per-map distribution of
    /// `true`s is what makes a monopoly visible.
    pub in_top_n: bool,
}

/// Read-only instrumentation of wayfind's Stage-1 region selection (SQL `wayfind_region_diagnostics`,
/// issue #585): one row per candidate region across the principal's visible anchors, carrying the
/// `sal_norm` / `query_cos` / `region_score` and top-N flag that `wayfind_region_scores` computes and
/// its own callers otherwise discard. Rows come back highest-`region_score` first.
///
/// **This is the ONLY read surface over those internals**, and the scope funnel that used to sit
/// beside it (`wayfind_scope_ids` / `wayfind_scope_reach`) went with the blended search mechanism on
/// 2026-08-06. So a change to `wayfind_region_scores` is observable from here or nowhere.
///
/// Gate is in the SQL (`visible_region_anchors`): a principal who can see no anchors gets zero rows,
/// never an error. Runtime `sqlx::query` — the `::vector` cast on the embedding forbids the compile-time
/// macros (the same established exception as [`vector_search`]).
pub async fn wayfind_region_diagnostics(
    pool: &PgPool,
    q: WayfindScopeQuery<'_>,
) -> Result<Vec<WayfindRegionDiagnosticRow>> {
    let emb_text = q.embedding.map(format_pgvector);
    let rows = sqlx::query(
        "SELECT region_id, home_anchor_table, home_anchor_id, home_anchor_name,
                sal_norm, query_cos, region_score, in_top_n
           FROM wayfind_region_diagnostics($1, $2, $3::vector, $4, $5, $6)",
    )
    .bind(q.principal)
    .bind(q.lens_id)
    .bind(emb_text) // NULL when None → p_emb NULL → query-cosine term zeroed
    .bind(q.regions)
    // NULL/NULL when None → unscoped: pool every visible anchor over both kinds.
    .bind(q.anchor.map(HomeAnchor::table))
    .bind(q.anchor.map(HomeAnchor::uuid))
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|r| WayfindRegionDiagnosticRow {
            region_id: r.get("region_id"),
            home_anchor_table: r.get("home_anchor_table"),
            home_anchor_id: r.get("home_anchor_id"),
            home_anchor_name: r.get("home_anchor_name"),
            sal_norm: r.get("sal_norm"),
            query_cos: r.get("query_cos"),
            region_score: r.get("region_score"),
            in_top_n: r.get("in_top_n"),
        })
        .collect())
}

/// One region's analytics-tier scalar metrics, as returned by `anchor_region_metrics`. The stored
/// readout columns of `kb_cogmap_regions` (computed once at materialization). Substrate-local; the
/// `temper-services` wrapper maps this to the `CogmapRegionMetricsRow` wire type.
///
/// Every metric is `Option` for an honest reason: a context materialized under `workflow-default`
/// carries no facets and no telos term, so `telos_alignment` is genuinely absent rather than zero.
#[derive(Debug, Clone, PartialEq)]
pub struct CogmapRegionMetricsRow {
    pub region_id: RegionId,
    pub lens_id: LensId,
    pub centrality: Option<f64>,
    pub content_cohesion: Option<f64>,
    pub internal_tension: Option<f64>,
    pub reference_standing: Option<f64>,
    pub telos_alignment: Option<f64>,
}

/// The per-region analytics tier read for either anchor kind (SQL `anchor_region_metrics`, T8). Gate
/// IS in the SQL (deny → empty); folded regions excluded; `lens_id = None` → all lenses, `Some(l)` →
/// that lens. Rows come back most-central first.
///
/// Only the two identity columns take a `!`: every metric is genuinely optional (see
/// [`CogmapRegionMetricsRow`] — a context materialized under `workflow-default` carries no telos
/// term at all), so forcing them non-null would assert exactly the thing the struct denies.
pub async fn anchor_region_metrics(
    pool: &PgPool,
    anchor: HomeAnchor,
    principal: ProfileId,
    lens_id: Option<LensId>,
) -> Result<Vec<CogmapRegionMetricsRow>> {
    let rows = sqlx::query!(
        r#"SELECT region_id AS "region_id!",
                  lens_id   AS "lens_id!",
                  centrality, content_cohesion, internal_tension,
                  reference_standing, telos_alignment
             FROM anchor_region_metrics($1, $2, 'profile', $3, $4)"#,
        anchor.table(),
        anchor.uuid(),
        principal.uuid(),
        lens_id.map(LensId::uuid),
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| CogmapRegionMetricsRow {
            region_id: RegionId::from(r.region_id),
            lens_id: LensId::from(r.lens_id),
            centrality: r.centrality,
            content_cohesion: r.content_cohesion,
            internal_tension: r.internal_tension,
            reference_standing: r.reference_standing,
            telos_alignment: r.telos_alignment,
        })
        .collect())
}

/// One regulation concept from `cogmap_analytics`'s `json_agg` column. `Deserialize` decodes it out of
/// the aggregated `jsonb`; field names match the `cogmap_regulation` return columns.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CogmapRegulationRow {
    pub resource_id: ResourceId,
    pub title: String,
    pub body_text: Option<String>,
    pub edge_label: String,
}

/// Map-level staleness, mirrored from `cogmap_staleness` columns. Substrate-local.
#[derive(Debug, Clone, PartialEq)]
pub struct CogmapStaleness {
    pub materialized_at: Option<DateTime<Utc>>,
    pub latest_touch: Option<DateTime<Utc>>,
    pub is_stale: bool,
}

/// The map-level analytics picture (`cogmap_analytics`): telos id + staleness + regulation set.
/// Substrate-local; the `temper-api` wrapper maps this to the `CogmapAnalyticsRow` wire type.
#[derive(Debug, Clone, PartialEq)]
pub struct CogmapAnalyticsRow {
    pub telos_resource_id: ResourceId,
    pub staleness: CogmapStaleness,
    pub regulation: Vec<CogmapRegulationRow>,
}

/// The map-level analytics read (`cogmap_analytics`). Gate IS in the SQL: a principal who cannot read
/// the map gets zero rows → `None` (never an error). `regulation` is decoded from the function's
/// `json_agg` `jsonb` column.
///
/// The `regulation!` override is the function's own guarantee, not an assumption: its body wraps the
/// aggregate in `COALESCE(…, '[]'::json)`
/// (`migrations/20260628000001_cogmap_analytics_read_functions.sql:33-36`), so the column is `[]`
/// for a map with no regulation and never SQL-NULL. The two staleness timestamps carry no such
/// coalesce and stay `Option`.
pub async fn cogmap_analytics(
    pool: &PgPool,
    cogmap_id: CogmapId,
    principal: ProfileId,
) -> Result<Option<CogmapAnalyticsRow>> {
    let row = sqlx::query!(
        r#"SELECT telos_resource_id AS "telos_resource_id!",
                  materialized_at,
                  latest_touch,
                  is_stale          AS "is_stale!",
                  regulation        AS "regulation!: sqlx::types::Json<Vec<CogmapRegulationRow>>"
             FROM cogmap_analytics($1, 'profile', $2)"#,
        cogmap_id.uuid(),
        principal.uuid(),
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| CogmapAnalyticsRow {
        telos_resource_id: ResourceId::from(r.telos_resource_id),
        staleness: CogmapStaleness {
            materialized_at: r.materialized_at,
            latest_touch: r.latest_touch,
            is_stale: r.is_stale,
        },
        regulation: r.regulation.0,
    }))
}
