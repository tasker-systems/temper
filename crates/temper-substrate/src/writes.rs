//! Typed write composition over the substrate mutation functions (WS6 4c live write path).
//!
//! The `DbBackend` (temper-api) calls these. Identity is resolved by **natural key** (handle /
//! entity-name / context-slug) — the same keys synthesis writes by — so no old→new id-map table is
//! needed. Each op opens one transaction and fires through the single [`crate::events::fire`] surface;
//! the connection carries the schema search_path (`public`), so the SQL functions + triggers resolve
//! their unqualified references correctly.
//!
//! Resolver SQL is runtime `sqlx::query` (not the compile-time macro) so it needs no `.sqlx` cache
//! entry — the macro cache is reserved for the substrate read/mutation queries.

use anyhow::{Context, Result};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::affinity::EdgeKind;
use crate::content::{
    prepare_block, prepare_block_from_chunks, IncomingChunk, PreparedBlock, PreparedChunk,
};
use crate::events::{fire, fire_with, EdgeHome, EventContext, SeedAction};
use crate::ids::{
    BlockId, CogmapId, ContextId, EdgeId, EntityId, InvocationId, ProfileId, PropertyId, ResourceId,
};
use crate::payloads::{self, AnchorRef, EdgePolarity};
use crate::text::slugify;

// ── identity resolution (natural-key) ───────────────────────────────────────────

/// The caller's profile id resolved against the (single) schema. Post-collapse the caller's profile id
/// IS the substrate profile id — synthesis preserves profile ids verbatim (WS2), and the auth path
/// (`check_can_modify`) already binds it directly as the substrate principal — so this is an existence
/// check that returns the same id typed. Errors if no such profile exists.
pub async fn resolve_profile(pool: &PgPool, prod_profile: Uuid) -> Result<ProfileId> {
    let id: Uuid = sqlx::query("SELECT id FROM kb_profiles WHERE id = $1")
        .bind(prod_profile)
        .fetch_one(pool)
        .await
        .with_context(|| format!("profile {prod_profile} not found"))?
        .get("id");
    Ok(ProfileId::from(id))
}

/// The durable per-surface emitter entity `<handle>@<surface>` for a profile (§1b). `surface` is
/// the lowercase surface marker (`cli` / `mcp` / `web`); `<handle>` is the profile's
/// `kb_profiles.handle`. Resolves by joining through kb_profiles so the actor name is
/// handle-derived (no hardcoded literal) and needs no extra round-trip.
pub async fn resolve_emitter(pool: &PgPool, profile: ProfileId, surface: &str) -> Result<EntityId> {
    let id: Uuid = sqlx::query(
        "SELECT e.id FROM kb_entities e \
         JOIN kb_profiles p ON p.id = e.profile_id \
         WHERE e.profile_id = $1 AND e.name = p.handle || '@' || $2",
    )
    .bind(profile.uuid())
    .bind(surface)
    .fetch_one(pool)
    .await
    .with_context(|| format!("no emitter entity <handle>@{surface} for the resolved profile"))?
    .get("id");
    Ok(EntityId::from(id))
}

/// Home context by `(owner profile, slugify(name))` — the owner-scoped shape (§2 amendment).
///
/// **Retained for the substrate write-path integration test only** (`tests/write_path_mutations.rs`).
/// Production resolves contexts via `temper_services::services::context_service::resolve_context_ref`
/// (visibility-gated, UUID-primary). Do not introduce new callers of this function in production code.
pub async fn resolve_context(pool: &PgPool, owner: ProfileId, name: &str) -> Result<ContextId> {
    let slug = slugify(name);
    let id: Uuid = sqlx::query(
        "SELECT id FROM kb_contexts \
         WHERE owner_table = 'kb_profiles' AND owner_id = $1 AND slug = $2",
    )
    .bind(owner.uuid())
    .bind(&slug)
    .fetch_one(pool)
    .await
    .with_context(|| format!("no context {slug:?} owned by the resolved profile"))?
    .get("id");
    Ok(ContextId::from(id))
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Begin a write transaction. The connection carries the schema search_path (`public`), so the SQL
/// functions + triggers resolve their unqualified references correctly with no per-txn `SET LOCAL`.
async fn begin_scoped(pool: &PgPool) -> Result<sqlx::Transaction<'_, sqlx::Postgres>> {
    Ok(pool.begin().await?)
}

// ── resource writes ────────────────────────────────────────────────────────────

/// Create a resource: one body block (chunked + embedded inline) homed in `home`, then one property
/// per `(key, value)` pair. Returns the new resource id.
#[derive(Debug)]
pub struct CreateParams<'a> {
    pub title: &'a str,
    pub origin_uri: &'a str,
    pub body: &'a str,
    pub doc_type: &'a str,
    pub home: AnchorRef,
    pub owner: ProfileId,
    pub originator: ProfileId,
    pub emitter: EntityId,
    /// Managed (§7-Property-fated) + open property pairs, each fired as a `PropertyAssert`.
    pub properties: &'a [(String, serde_json::Value)],
    /// Caller-supplied, already-embedded chunks. When `Some`, the body block is built from these
    /// verbatim (no server-side embed — the client did extract→chunk→embed); when `None`, the server
    /// chunks + embeds `body` itself (the fallback path). Reverses PR#71's discard-client-chunks contract.
    pub chunks: Option<Vec<IncomingChunk>>,
}

pub async fn create_resource(pool: &PgPool, p: CreateParams<'_>) -> Result<ResourceId> {
    create_resource_with(pool, p, EventContext::default()).await
}

/// [`create_resource`] under an explicit [`EventContext`] — the authored `resource_created` act is
/// stamped with the caller's authorship (→ `kb_events.metadata`) and invocation correlator
/// (→ `kb_events.invocation_id`). The property acts fired at creation stay un-stamped (out of the
/// authored-act scope). Mirrors the [`crate::events::fire`]/`fire_with` split.
pub async fn create_resource_with(
    pool: &PgPool,
    p: CreateParams<'_>,
    ctx: EventContext,
) -> Result<ResourceId> {
    let block = match p.chunks {
        Some(chunks) => prepare_block_from_chunks(0, None, chunks),
        None => prepare_block(0, None, p.body)?,
    };
    let blocks = [block];
    let mut tx = begin_scoped(pool).await?;
    let new_id = fire_with(
        &mut tx,
        SeedAction::ResourceCreate {
            title: p.title,
            origin_uri: p.origin_uri,
            // A genuinely new resource created through the live write path — mint a fresh id.
            resource_id: None,
            home: p.home,
            owner: p.owner,
            originator: Some(p.originator),
            blocks: &blocks,
            doc_type: Some(p.doc_type),
            emitter: p.emitter,
        },
        ctx,
    )
    .await?
    .resource()?;
    for (key, value) in p.properties {
        fire(
            &mut tx,
            SeedAction::PropertySet {
                resource: new_id,
                key,
                value,
                weight: 1.0,
                emitter: p.emitter,
            },
        )
        .await?;
    }
    tx.commit().await?;
    Ok(new_id)
}

/// A partial resource update — only the fields present in the command are written.
#[derive(Debug)]
pub struct UpdateParams<'a> {
    pub resource: ResourceId,
    /// New body prose; revises the resource's single non-folded block (re-chunked + re-embedded).
    pub body: Option<&'a str>,
    pub title: Option<&'a str>,
    pub origin_uri: Option<&'a str>,
    /// Property pairs to (re)assert (stage/mode/effort/doc_type + meta keys).
    pub properties: &'a [(String, serde_json::Value)],
    /// Caller-supplied, already-embedded chunks for the body revise. When `Some` (and `body` is
    /// supplied), the new block is built from these verbatim (no server-side embed); when `None`, the
    /// server chunks + embeds `body` (the fallback path). Reverses PR#71's discard contract.
    pub chunks: Option<Vec<IncomingChunk>>,
    /// Destination context for a move (`move_to.context_to`).
    pub rehome_to: Option<ContextId>,
    pub emitter: EntityId,
}

pub async fn update_resource(pool: &PgPool, p: UpdateParams<'_>) -> Result<()> {
    update_resource_with(pool, p, EventContext::default()).await
}

/// [`update_resource`] under an explicit [`EventContext`] — every sub-event of the update fan-out
/// (`block_mutated` / `property_set` / `resource_updated` / `resource_rehomed`) is correlated to the
/// caller's invocation (→ `kb_events.invocation_id`) and stamped with its authorship
/// (→ `kb_events.metadata`). Mirrors the [`crate::events::fire`]/`fire_with` split.
pub async fn update_resource_with(
    pool: &PgPool,
    p: UpdateParams<'_>,
    ctx: EventContext,
) -> Result<()> {
    let mut tx = begin_scoped(pool).await?;
    update_resource_in_tx(&mut tx, p, ctx).await?;
    tx.commit().await?;
    Ok(())
}

/// In-transaction variant of [`update_resource`] — fires on a caller-supplied connection (no
/// begin/commit). The body-block lookup runs on `&mut *conn` so it shares the caller's transaction.
/// `ctx` correlates every sub-event the update fires (`EventContext::default()` for an un-attributed
/// update); it is cloned per sub-event since an update fans out to several.
pub async fn update_resource_in_tx(
    conn: &mut sqlx::PgConnection,
    p: UpdateParams<'_>,
    ctx: EventContext,
) -> Result<()> {
    if let Some(body) = p.body {
        // resolve the resource's single non-folded body block (CONFORM scenario runner revise).
        let block_ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM kb_content_blocks WHERE resource_id=$1 AND NOT is_folded ORDER BY seq",
        )
        .bind(p.resource.uuid())
        .fetch_all(&mut *conn)
        .await?;
        let block_id = match block_ids.as_slice() {
            [one] => *one,
            [] => anyhow::bail!(
                "update_resource: resource {} has no live block",
                p.resource.uuid()
            ),
            _ => anyhow::bail!(
                "update_resource: resource {} has >1 block (multi-block revise unsupported)",
                p.resource.uuid()
            ),
        };
        let prepared = match p.chunks {
            Some(chunks) => prepare_block_from_chunks(0, None, chunks),
            None => prepare_block(0, None, body)?,
        };
        if prepared.chunks.is_empty() {
            anyhow::bail!(
                "update_resource: empty/whitespace body — refusing to write a contentless block"
            );
        }
        fire_with(
            &mut *conn,
            SeedAction::BlockMutate {
                block: crate::ids::BlockId::from(block_id),
                chunks: &prepared.chunks,
                emitter: p.emitter,
            },
            ctx.clone(),
        )
        .await?;
    }

    for (key, value) in p.properties {
        fire_with(
            &mut *conn,
            SeedAction::PropertySet {
                resource: p.resource,
                key,
                value,
                weight: 1.0,
                emitter: p.emitter,
            },
            ctx.clone(),
        )
        .await?;
    }

    if p.title.is_some() || p.origin_uri.is_some() {
        fire_with(
            &mut *conn,
            SeedAction::ResourceUpdate {
                resource: p.resource,
                title: p.title,
                origin_uri: p.origin_uri,
                emitter: p.emitter,
            },
            ctx.clone(),
        )
        .await?;
    }

    if let Some(dest) = p.rehome_to {
        fire_with(
            &mut *conn,
            SeedAction::ResourceRehome {
                resource: p.resource,
                home: AnchorRef::context(dest),
                emitter: p.emitter,
            },
            ctx,
        )
        .await?;
    }

    Ok(())
}

/// Soft-delete a resource.
pub async fn delete_resource(pool: &PgPool, resource: ResourceId, emitter: EntityId) -> Result<()> {
    delete_resource_with(pool, resource, emitter, EventContext::default()).await
}

/// [`delete_resource`] under an explicit [`EventContext`] — the `resource_deleted` act is correlated
/// to the caller's invocation + stamped with its authorship. Mirrors `fire`/`fire_with`.
pub async fn delete_resource_with(
    pool: &PgPool,
    resource: ResourceId,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<()> {
    let mut tx = begin_scoped(pool).await?;
    delete_resource_in_tx(&mut tx, resource, emitter, ctx).await?;
    tx.commit().await?;
    Ok(())
}

/// In-transaction variant of [`delete_resource`] — fires on a caller-supplied connection (no
/// begin/commit). `ctx` correlates the `resource_deleted` act (`EventContext::default()` for an
/// un-attributed delete).
pub async fn delete_resource_in_tx(
    conn: &mut sqlx::PgConnection,
    resource: ResourceId,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<()> {
    fire_with(conn, SeedAction::ResourceDelete { resource, emitter }, ctx).await?;
    Ok(())
}

// ── cogmap-homed kernel writes (L0 reconcile) ──────────────────────────────────

/// Create a kernel resource homed to a **cogmap** (not a context) — the shape the L0 reconciler
/// uses. Mirrors [`create_resource`] but homes `AnchorRef::cogmap(p.cogmap)` and passes
/// `originator: None` (kernel content's originator COALESCEs to `owner` = system). The post-create
/// property loop of `create_resource` is intentionally omitted: kernel facets/provenance are stamped
/// by the caller via [`set_property`] / [`set_facet`].
#[derive(Debug)]
pub struct KernelCreateParams<'a> {
    pub cogmap: CogmapId,
    /// The STABLE landmark identity the resource is minted under (the reconcile diff key). Supplying it
    /// (rather than minting) makes a duplicate create a PRIMARY-KEY conflict — fail-loud, never a silent
    /// twin.
    pub resource_id: Uuid,
    pub title: &'a str,
    pub origin_uri: &'a str,
    pub doc_type: &'a str,
    pub body: &'a str,
    /// Caller-supplied, already-embedded chunks. When `Some`, the body block is built from these
    /// verbatim (the client embedded); when `None`, the server chunks + embeds `body` (fallback) —
    /// the same client-embed-or-server-fallback affordance as [`create_resource`].
    pub chunks: Option<Vec<IncomingChunk>>,
    pub owner: ProfileId,
    pub emitter: EntityId,
}

pub async fn create_kernel_resource(
    pool: &PgPool,
    p: KernelCreateParams<'_>,
) -> Result<ResourceId> {
    let mut tx = begin_scoped(pool).await?;
    let new_id = create_kernel_resource_in_tx(&mut tx, p, EventContext::default()).await?;
    tx.commit().await?;
    Ok(new_id)
}

/// In-transaction variant of [`create_kernel_resource`] — fires on a caller-supplied connection (no
/// begin/commit) so the L0 reconcile can run every mutation in ONE serializable transaction. `ctx`
/// correlates the `resource_created` act to the reconcile run (`EventContext::default()` for an
/// un-attributed create).
pub async fn create_kernel_resource_in_tx(
    conn: &mut sqlx::PgConnection,
    p: KernelCreateParams<'_>,
    ctx: EventContext,
) -> Result<ResourceId> {
    let block = match p.chunks {
        Some(chunks) => prepare_block_from_chunks(0, None, chunks),
        None => prepare_block(0, None, p.body)?,
    };
    let blocks = [block];
    let new_id = fire_with(
        conn,
        SeedAction::ResourceCreate {
            title: p.title,
            origin_uri: p.origin_uri,
            // Mint under the caller's STABLE landmark id (the diff key) — so a duplicate create is a
            // primary-key conflict, never a silent twin.
            resource_id: Some(ResourceId::from(p.resource_id)),
            home: AnchorRef::cogmap(p.cogmap),
            owner: p.owner,
            // Kernel content's originator COALESCEs to owner (= system).
            originator: None,
            blocks: &blocks,
            doc_type: Some(p.doc_type),
            emitter: p.emitter,
        },
        ctx,
    )
    .await?
    .resource()?;
    Ok(new_id)
}

/// Replace a cogmap's telos charter with `blocks` (role-tagged, pre-embedded), in a caller-supplied
/// transaction. Fires `SeedAction::CharterSet` → `cogmap_charter_set` (fold-then-reproject). Returns the
/// telos resource id. The L0 charter reconciler calls this when the desired charter's body merkle differs
/// from the telos's current `body_hash` (see [`crate::readback::telos_charter_state`]).
pub async fn set_charter_in_tx(
    conn: &mut sqlx::PgConnection,
    cogmap: CogmapId,
    blocks: &[PreparedBlock],
    emitter: EntityId,
    ctx: EventContext,
) -> Result<ResourceId> {
    fire_with(
        conn,
        SeedAction::CharterSet {
            cogmap,
            blocks,
            emitter,
        },
        ctx,
    )
    .await?
    .charter()
}

/// Set the **clustering** facet on a resource — one `kb_properties` row with `property_key='facet'`
/// holding the whole `values` object (e.g. `{layer: concept}`). This is what materialization/affinity
/// read. NOT interchangeable with [`set_property`] (Decision #6): `provenance` is per-key, not a
/// clustering facet.
pub async fn set_facet(
    pool: &PgPool,
    resource: ResourceId,
    values: &serde_json::Value,
    weight: f64,
    emitter: EntityId,
) -> Result<PropertyId> {
    set_facet_with(
        pool,
        resource,
        values,
        weight,
        emitter,
        EventContext::default(),
    )
    .await
}

/// [`set_facet`] under an explicit [`EventContext`] — the `facet_set` act is correlated to the
/// caller's invocation + stamped with its authorship. Mirrors `fire`/`fire_with`. Returns the
/// `kb_properties.id` the fire produced (surfaced from `Fired::Facet`).
pub async fn set_facet_with(
    pool: &PgPool,
    resource: ResourceId,
    values: &serde_json::Value,
    weight: f64,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<PropertyId> {
    let mut tx = begin_scoped(pool).await?;
    let property_id = set_facet_in_tx(&mut tx, resource, values, weight, emitter, ctx).await?;
    tx.commit().await?;
    Ok(property_id)
}

/// In-transaction variant of [`set_facet`] — fires on a caller-supplied connection (no begin/commit).
/// `ctx` correlates the `facet_set` act (`EventContext::default()` for an un-attributed facet). Returns
/// the `kb_properties.id` the fire produced.
pub async fn set_facet_in_tx(
    conn: &mut sqlx::PgConnection,
    resource: ResourceId,
    values: &serde_json::Value,
    weight: f64,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<PropertyId> {
    fire_with(
        conn,
        SeedAction::FacetSet {
            resource,
            values,
            weight,
            emitter,
        },
        ctx,
    )
    .await?
    .facet()
}

/// Set a single-valued **per-key** property — folds prior active `(owner, key)` rows then asserts the
/// new value, so the key holds one current value (`property_key=<key>`). This is the shape
/// `readback::kernel_slice` reads; the reconciler stamps `provenance: kernel` through it.
pub async fn set_property(
    pool: &PgPool,
    resource: ResourceId,
    key: &str,
    value: &serde_json::Value,
    emitter: EntityId,
) -> Result<()> {
    set_property_with(pool, resource, key, value, emitter, EventContext::default()).await
}

/// [`set_property`] under an explicit [`EventContext`] — the `property_set` act is correlated to the
/// caller's invocation + stamped with its authorship. Mirrors `fire`/`fire_with`.
pub async fn set_property_with(
    pool: &PgPool,
    resource: ResourceId,
    key: &str,
    value: &serde_json::Value,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<()> {
    let mut tx = begin_scoped(pool).await?;
    set_property_in_tx(&mut tx, resource, key, value, emitter, ctx).await?;
    tx.commit().await?;
    Ok(())
}

/// In-transaction variant of [`set_property`] — fires on a caller-supplied connection (no begin/commit).
/// `ctx` correlates the `property_set` act (`EventContext::default()` for an un-attributed property).
pub async fn set_property_in_tx(
    conn: &mut sqlx::PgConnection,
    resource: ResourceId,
    key: &str,
    value: &serde_json::Value,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<()> {
    fire_with(
        conn,
        SeedAction::PropertySet {
            resource,
            key,
            value,
            weight: 1.0,
            emitter,
        },
        ctx,
    )
    .await?;
    Ok(())
}

/// Re-block a resource's body block from already-prepared chunks (the update path — the caller
/// resolves the block id and prepares the new chunks). Mirrors the `Revise`/`BlockMutate` fire.
pub async fn mutate_block(
    pool: &PgPool,
    block: BlockId,
    chunks: &[PreparedChunk],
    emitter: EntityId,
) -> Result<()> {
    let mut tx = begin_scoped(pool).await?;
    fire(
        &mut tx,
        SeedAction::BlockMutate {
            block,
            chunks,
            emitter,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Assert (or idempotently re-assert) a **cogmap-homed** edge `src → tgt`, returning its id. Mirrors
/// [`assert_relationship`] but homes `EdgeHome::Cogmap(p.cogmap)` (kernel landmarks home to the map,
/// not a context).
#[derive(Debug)]
pub struct KernelEdgeParams<'a> {
    pub cogmap: CogmapId,
    pub src: ResourceId,
    pub tgt: ResourceId,
    pub kind: EdgeKind,
    pub polarity: EdgePolarity,
    pub label: Option<&'a str>,
    pub weight: f64,
    pub emitter: EntityId,
}

pub async fn assert_kernel_edge(pool: &PgPool, p: KernelEdgeParams<'_>) -> Result<EdgeId> {
    assert_kernel_edge_with(pool, p, EventContext::default()).await
}

/// [`assert_kernel_edge`] under an explicit [`EventContext`] — the authored `relationship_asserted`
/// act carries the caller's authorship + invocation correlator. This is the pool-level ctx variant
/// `DbBackend::assert_relationship` dispatches to when the source resource is **cogmap-homed** (a
/// steward's authored-4 node), homing the edge to the map rather than a context.
pub async fn assert_kernel_edge_with(
    pool: &PgPool,
    p: KernelEdgeParams<'_>,
    ctx: EventContext,
) -> Result<EdgeId> {
    let mut tx = begin_scoped(pool).await?;
    let edge = assert_kernel_edge_in_tx(&mut tx, p, ctx).await?;
    tx.commit().await?;
    Ok(edge)
}

/// In-transaction variant of [`assert_kernel_edge`] — fires on a caller-supplied connection (no
/// begin/commit). `ctx` correlates the `relationship_asserted` act to the reconcile run
/// (`EventContext::default()` for an un-attributed assert).
pub async fn assert_kernel_edge_in_tx(
    conn: &mut sqlx::PgConnection,
    p: KernelEdgeParams<'_>,
    ctx: EventContext,
) -> Result<EdgeId> {
    let edge = fire_with(
        conn,
        SeedAction::RelationshipAssert {
            src: p.src,
            tgt: p.tgt,
            kind: p.kind,
            polarity: p.polarity,
            label: p.label,
            weight: p.weight,
            home: EdgeHome::Cogmap(p.cogmap),
            emitter: p.emitter,
        },
        ctx,
    )
    .await?
    .relationship()?;
    Ok(edge)
}

// ── relationship writes ──────────────────────────────────────────────────────────

/// Assert (or idempotently re-assert) an edge `src → tgt`, returning its id.
#[derive(Debug)]
pub struct AssertParams<'a> {
    pub src: ResourceId,
    pub tgt: ResourceId,
    pub kind: EdgeKind,
    pub polarity: EdgePolarity,
    pub label: Option<&'a str>,
    pub weight: f64,
    pub home: ContextId,
    pub emitter: EntityId,
}

pub async fn assert_relationship(pool: &PgPool, p: AssertParams<'_>) -> Result<EdgeId> {
    assert_relationship_with(pool, p, EventContext::default()).await
}

/// [`assert_relationship`] under an explicit [`EventContext`] — the authored `relationship_asserted`
/// act carries the caller's authorship + invocation correlator. Mirrors `fire`/`fire_with`.
pub async fn assert_relationship_with(
    pool: &PgPool,
    p: AssertParams<'_>,
    ctx: EventContext,
) -> Result<EdgeId> {
    let mut tx = begin_scoped(pool).await?;
    let edge = fire_with(
        &mut tx,
        SeedAction::RelationshipAssert {
            src: p.src,
            tgt: p.tgt,
            kind: p.kind,
            polarity: p.polarity,
            label: p.label,
            weight: p.weight,
            home: EdgeHome::Context(p.home),
            emitter: p.emitter,
        },
        ctx,
    )
    .await?
    .relationship()?;
    tx.commit().await?;
    Ok(edge)
}

pub async fn retype_relationship(
    pool: &PgPool,
    edge: EdgeId,
    kind: EdgeKind,
    polarity: EdgePolarity,
    emitter: EntityId,
) -> Result<()> {
    retype_relationship_with(pool, edge, kind, polarity, emitter, EventContext::default()).await
}

/// [`retype_relationship`] under an explicit [`EventContext`] — the `relationship_retyped` act is
/// correlated to the caller's invocation + stamped with its authorship. Mirrors `fire`/`fire_with`.
pub async fn retype_relationship_with(
    pool: &PgPool,
    edge: EdgeId,
    kind: EdgeKind,
    polarity: EdgePolarity,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<()> {
    let mut tx = begin_scoped(pool).await?;
    fire_with(
        &mut tx,
        SeedAction::RelationshipRetype {
            edge,
            kind,
            polarity,
            emitter,
        },
        ctx,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn reweight_relationship(
    pool: &PgPool,
    edge: EdgeId,
    weight: f64,
    emitter: EntityId,
) -> Result<()> {
    reweight_relationship_with(pool, edge, weight, emitter, EventContext::default()).await
}

/// [`reweight_relationship`] under an explicit [`EventContext`] — the `relationship_reweighted` act is
/// correlated to the caller's invocation + stamped with its authorship. Mirrors `fire`/`fire_with`.
pub async fn reweight_relationship_with(
    pool: &PgPool,
    edge: EdgeId,
    weight: f64,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<()> {
    let mut tx = begin_scoped(pool).await?;
    fire_with(
        &mut tx,
        SeedAction::RelationshipReweight {
            edge,
            weight,
            emitter,
        },
        ctx,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn fold_relationship(
    pool: &PgPool,
    edge: EdgeId,
    reason: Option<&str>,
    emitter: EntityId,
) -> Result<()> {
    fold_relationship_with(pool, edge, reason, emitter, EventContext::default()).await
}

/// [`fold_relationship`] under an explicit [`EventContext`] — the authored `relationship_folded` act
/// carries the caller's authorship + invocation correlator. Mirrors `fire`/`fire_with`.
pub async fn fold_relationship_with(
    pool: &PgPool,
    edge: EdgeId,
    reason: Option<&str>,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<()> {
    let mut tx = begin_scoped(pool).await?;
    fold_relationship_in_tx(&mut tx, edge, reason, emitter, ctx).await?;
    tx.commit().await?;
    Ok(())
}

/// In-transaction variant of [`fold_relationship`] — fires on a caller-supplied connection (no
/// begin/commit). `ctx` stamps the authored `relationship_folded` act (`EventContext::default()`
/// for an un-attributed fold).
pub async fn fold_relationship_in_tx(
    conn: &mut sqlx::PgConnection,
    edge: EdgeId,
    reason: Option<&str>,
    emitter: EntityId,
    ctx: EventContext,
) -> Result<()> {
    fire_with(
        conn,
        SeedAction::RelationshipFold {
            edge,
            reason,
            emitter,
        },
        ctx,
    )
    .await?;
    Ok(())
}

// ── invocation envelope ──────────────────────────────────────────────────────────

/// Parameters for opening an invocation. The invocation id is minted here and
/// returned (server-mint v1; caller-supplied ids for byte-exact durable-resume
/// re-issue are a deferred runtime concern).
#[derive(Debug)]
pub struct OpenParams {
    pub trigger_kind: String,
    pub originating: CogmapId,
    pub parent: Option<CogmapId>,
    pub scoped_entity: EntityId,
    pub emitter: EntityId,
}

/// Open an invocation envelope, returning the minted invocation id.
pub async fn open_invocation(pool: &PgPool, p: OpenParams) -> Result<InvocationId> {
    let mut tx = begin_scoped(pool).await?;
    let opened = open_invocation_in_tx(&mut tx, p).await?;
    tx.commit().await?;
    Ok(opened)
}

/// In-transaction variant of [`open_invocation`] — fires on a caller-supplied connection (no
/// begin/commit) so the open + the reconcile body + the close share ONE serializable transaction.
pub async fn open_invocation_in_tx(
    conn: &mut sqlx::PgConnection,
    p: OpenParams,
) -> Result<InvocationId> {
    let invocation = InvocationId::from(Uuid::now_v7());
    let opened = fire(
        conn,
        SeedAction::InvocationOpen {
            invocation,
            trigger_kind: &p.trigger_kind,
            originating: p.originating,
            parent: p.parent,
            scoped_entity: p.scoped_entity,
            emitter: p.emitter,
        },
    )
    .await?
    .invocation()?;
    Ok(opened)
}

/// Close an invocation with a terminal disposition + opaque outcome. The
/// originating cogmap is supplied by the caller (it knows it from the open /
/// from an auth lookup) so the `SeedAction` is constructed truthfully; the
/// substrate ignores it on close but the typed action requires it.
pub async fn close_invocation(
    pool: &PgPool,
    invocation: InvocationId,
    originating: CogmapId,
    disposition: payloads::Disposition,
    outcome: serde_json::Value,
    emitter: EntityId,
) -> Result<()> {
    let mut tx = begin_scoped(pool).await?;
    close_invocation_in_tx(
        &mut tx,
        invocation,
        originating,
        disposition,
        outcome,
        emitter,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// In-transaction variant of [`close_invocation`] — fires on a caller-supplied connection (no
/// begin/commit).
pub async fn close_invocation_in_tx(
    conn: &mut sqlx::PgConnection,
    invocation: InvocationId,
    originating: CogmapId,
    disposition: payloads::Disposition,
    outcome: serde_json::Value,
    emitter: EntityId,
) -> Result<()> {
    fire(
        conn,
        SeedAction::InvocationClose {
            invocation,
            disposition,
            outcome,
            originating,
            emitter,
        },
    )
    .await?;
    Ok(())
}
