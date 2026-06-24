//! Typed write composition over the `temper_next` mutation functions (WS6 4c live write path).
//!
//! The `DbBackend` (temper-api) calls these. Identity is resolved by **natural key** (handle /
//! entity-name / context-slug) — the same keys synthesis writes by — so no old→new id-map table is
//! needed. Each op opens one transaction and fires through the single [`crate::events::fire`] surface;
//! the connection carries the schema search_path (dev: `temper_next,public`; live: `public` after the
//! rename), so the SQL functions + triggers resolve their unqualified references correctly.
//!
//! Resolver SQL is runtime `sqlx::query` (not the compile-time macro) so it needs no `.sqlx` cache
//! entry — the macro cache is reserved for the substrate read/mutation queries.

use anyhow::{Context, Result};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::affinity::EdgeKind;
use crate::content::prepare_block;
use crate::events::{fire, EdgeHome, SeedAction};
use crate::ids::{CogmapId, ContextId, EdgeId, EntityId, InvocationId, ProfileId, ResourceId};
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

/// Begin a write transaction. Post-collapse the connection carries the schema search_path (dev:
/// `temper_next,public`; live: `public` after the rename), so the SQL functions + triggers resolve
/// their unqualified references correctly with no per-txn `SET LOCAL`.
async fn begin_scoped(pool: &PgPool) -> Result<sqlx::Transaction<'_, sqlx::Postgres>> {
    Ok(pool.begin().await?)
}

// ── resource writes ────────────────────────────────────────────────────────────

/// Create a resource: one body block (chunked + embedded inline) homed in `home`, then one property
/// per `(key, value)` pair. Returns the new resource id.
pub struct CreateParams<'a> {
    pub title: &'a str,
    pub origin_uri: &'a str,
    pub body: &'a str,
    pub doc_type: &'a str,
    pub home: ContextId,
    pub owner: ProfileId,
    pub originator: ProfileId,
    pub emitter: EntityId,
    /// Managed (§7-Property-fated) + open property pairs, each fired as a `PropertyAssert`.
    pub properties: &'a [(String, serde_json::Value)],
}

pub async fn create_resource(pool: &PgPool, p: CreateParams<'_>) -> Result<ResourceId> {
    let block = prepare_block(0, None, p.body)?;
    let blocks = [block];
    let mut tx = begin_scoped(pool).await?;
    let new_id = fire(
        &mut tx,
        SeedAction::ResourceCreate {
            title: p.title,
            origin_uri: p.origin_uri,
            // A genuinely new resource created through the live write path — mint a fresh id.
            resource_id: None,
            home: AnchorRef::context(p.home),
            owner: p.owner,
            originator: Some(p.originator),
            blocks: &blocks,
            doc_type: Some(p.doc_type),
            emitter: p.emitter,
        },
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
pub struct UpdateParams<'a> {
    pub resource: ResourceId,
    /// New body prose; revises the resource's single non-folded block (re-chunked + re-embedded).
    pub body: Option<&'a str>,
    pub title: Option<&'a str>,
    pub origin_uri: Option<&'a str>,
    /// Property pairs to (re)assert (stage/mode/effort/doc_type + meta keys).
    pub properties: &'a [(String, serde_json::Value)],
    /// Destination context for a move (`move_to.context_to`).
    pub rehome_to: Option<ContextId>,
    pub emitter: EntityId,
}

pub async fn update_resource(pool: &PgPool, p: UpdateParams<'_>) -> Result<()> {
    let mut tx = begin_scoped(pool).await?;

    if let Some(body) = p.body {
        // resolve the resource's single non-folded body block (CONFORM scenario runner revise).
        let block_ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM kb_content_blocks WHERE resource_id=$1 AND NOT is_folded ORDER BY seq",
        )
        .bind(p.resource.uuid())
        .fetch_all(&mut *tx)
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
        let prepared = prepare_block(0, None, body)?;
        if prepared.chunks.is_empty() {
            anyhow::bail!(
                "update_resource: empty/whitespace body — refusing to write a contentless block"
            );
        }
        fire(
            &mut tx,
            SeedAction::BlockMutate {
                block: crate::ids::BlockId::from(block_id),
                chunks: &prepared.chunks,
                emitter: p.emitter,
            },
        )
        .await?;
    }

    for (key, value) in p.properties {
        fire(
            &mut tx,
            SeedAction::PropertySet {
                resource: p.resource,
                key,
                value,
                weight: 1.0,
                emitter: p.emitter,
            },
        )
        .await?;
    }

    if p.title.is_some() || p.origin_uri.is_some() {
        fire(
            &mut tx,
            SeedAction::ResourceUpdate {
                resource: p.resource,
                title: p.title,
                origin_uri: p.origin_uri,
                emitter: p.emitter,
            },
        )
        .await?;
    }

    if let Some(dest) = p.rehome_to {
        fire(
            &mut tx,
            SeedAction::ResourceRehome {
                resource: p.resource,
                home: AnchorRef::context(dest),
                emitter: p.emitter,
            },
        )
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

/// Soft-delete a resource.
pub async fn delete_resource(pool: &PgPool, resource: ResourceId, emitter: EntityId) -> Result<()> {
    let mut tx = begin_scoped(pool).await?;
    fire(&mut tx, SeedAction::ResourceDelete { resource, emitter }).await?;
    tx.commit().await?;
    Ok(())
}

// ── relationship writes ──────────────────────────────────────────────────────────

/// Assert (or idempotently re-assert) an edge `src → tgt`, returning its id.
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
    let mut tx = begin_scoped(pool).await?;
    let edge = fire(
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
    let mut tx = begin_scoped(pool).await?;
    fire(
        &mut tx,
        SeedAction::RelationshipRetype {
            edge,
            kind,
            polarity,
            emitter,
        },
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
    let mut tx = begin_scoped(pool).await?;
    fire(
        &mut tx,
        SeedAction::RelationshipReweight {
            edge,
            weight,
            emitter,
        },
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
    let mut tx = begin_scoped(pool).await?;
    fire(
        &mut tx,
        SeedAction::RelationshipFold {
            edge,
            reason,
            emitter,
        },
    )
    .await?;
    tx.commit().await?;
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
    let invocation = InvocationId::from(Uuid::now_v7());
    let mut tx = begin_scoped(pool).await?;
    let opened = fire(
        &mut tx,
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
    tx.commit().await?;
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
    fire(
        &mut tx,
        SeedAction::InvocationClose {
            invocation,
            disposition,
            outcome,
            originating,
            emitter,
        },
    )
    .await?;
    tx.commit().await?;
    Ok(())
}
