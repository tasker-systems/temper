//! Typed write composition over the `temper_next` mutation functions (WS6 4c live write path).
//!
//! The `NextBackend` (temper-api) calls these. Identity is resolved by **natural key** (handle /
//! entity-name / context-slug) — the same keys synthesis writes by — so no old→new id-map table is
//! needed; a freshly-synthesized substrate is immediately writable. Every op opens one transaction with
//! `SET LOCAL search_path TO temper_next, public` (so the SQL functions + triggers resolve unqualified
//! references into `temper_next`) and fires through the single [`crate::events::fire`] surface.
//!
//! Resolver SQL is runtime `sqlx::query` with explicit `temper_next.`/`public.` qualification (the
//! [`crate::synthesis::source`] precedent): a compile-time macro over `public.*` would conflict with the
//! crate's `temper_next`-pinned `.sqlx` cache.

use anyhow::{Context, Result};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::affinity::EdgeKind;
use crate::content::prepare_block;
use crate::events::{fire, EdgeHome, SeedAction};
use crate::ids::{ContextId, EdgeId, EntityId, ProfileId, ResourceId};
use crate::payloads::{AnchorRef, EdgePolarity};
use crate::synthesis::bootstrap::slugify;

// ── identity resolution (natural-key) ───────────────────────────────────────────

/// Production profile id → synthesized `temper_next` profile id, by `handle` (= production
/// `kb_profiles.slug`, the key synthesis bootstraps profiles under). Errors if the substrate was not
/// synthesized for that profile.
pub async fn resolve_profile(pool: &PgPool, prod_profile: Uuid) -> Result<ProfileId> {
    let slug: String = sqlx::query("SELECT slug FROM public.kb_profiles WHERE id = $1")
        .bind(prod_profile)
        .fetch_one(pool)
        .await
        .with_context(|| format!("production profile {prod_profile} not found"))?
        .get("slug");
    let id: Uuid = sqlx::query("SELECT id FROM temper_next.kb_profiles WHERE handle = $1")
        .bind(&slug)
        .fetch_one(pool)
        .await
        .with_context(|| {
            format!("no temper_next profile for handle {slug:?} (substrate not synthesized?)")
        })?
        .get("id");
    Ok(ProfileId::from(id))
}

/// The durable per-surface emitter entity `pete@<surface>` for a profile (§1b). `surface` is the
/// lowercase surface marker (`cli` / `mcp` / `web`).
pub async fn resolve_emitter(pool: &PgPool, profile: ProfileId, surface: &str) -> Result<EntityId> {
    let name = format!("pete@{surface}");
    let id: Uuid =
        sqlx::query("SELECT id FROM temper_next.kb_entities WHERE profile_id = $1 AND name = $2")
            .bind(profile.uuid())
            .bind(&name)
            .fetch_one(pool)
            .await
            .with_context(|| format!("no emitter entity {name:?} for the resolved profile"))?
            .get("id");
    Ok(EntityId::from(id))
}

/// Home context by `(owner profile, slugify(name))` — the owner-scoped shape (§2 amendment).
pub async fn resolve_context(pool: &PgPool, owner: ProfileId, name: &str) -> Result<ContextId> {
    let slug = slugify(name);
    let id: Uuid = sqlx::query(
        "SELECT id FROM temper_next.kb_contexts \
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

/// Begin a `temper_next`-scoped transaction (the search_path discipline every write op shares).
async fn begin_scoped(pool: &PgPool) -> Result<sqlx::Transaction<'_, sqlx::Postgres>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL search_path TO temper_next, public")
        .execute(&mut *tx)
        .await?;
    Ok(tx)
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
