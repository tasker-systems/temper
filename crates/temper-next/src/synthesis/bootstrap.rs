//! Synthesis bootstrap (WS6 §1/§2): the administrative infrastructure the per-resource synthesis
//! sequence stands on — profiles, the migration + per-surface emitter entities, and the thin unowned
//! contexts. **Direct inserts, NOT event-sourced** (§1 open residue: *"Entity creation stays
//! administrative (no event)"*); profiles and contexts are likewise administrative.
//!
//! All writes are schema-qualified (`temper_next.*`) so this runs correctly on either pool shape: the
//! production `substrate::connect()` pool (`search_path = temper_next, public`) and the self-contained
//! `#[sqlx::test]` pool (default `search_path = public`, where `public` ALSO carries `kb_profiles` /
//! `kb_contexts` — an unqualified write would land in the wrong namespace). Reads of the source live
//! in [`super::source`] and are already `public.`-qualified.
//!
//! The remaps it returns ([`BootstrapMaps`]) are what the resource pass (next task) consumes to anchor
//! homes in the new context ids and bind originator/owner to the new profile ids.

use std::collections::{BTreeSet, HashMap};

use anyhow::{Context, Result};
use sqlx::PgPool;
use uuid::Uuid;

use super::source::{self, SourceResource};
use crate::ids::{ContextId, EntityId, ProfileId};

/// The three durable per-(profile, surface) emitter entities (§1b). Session/device identifiers move
/// into event `metadata`, never entity identity — so the device sprawl does not reproduce here.
#[derive(Debug, Clone, Copy)]
pub struct SurfaceEntities {
    pub cli: EntityId,
    pub mcp: EntityId,
    pub web: EntityId,
}

/// The old→new remaps + the synthesized emitter entities the resource/property/edge passes consume.
#[derive(Debug, Clone)]
pub struct BootstrapMaps {
    /// production `kb_contexts.id` → synthesized `temper_next.kb_contexts.id` (§2: by name).
    pub context_id_by_old: HashMap<Uuid, ContextId>,
    /// production `kb_profiles.id` → synthesized `temper_next.kb_profiles.id`.
    pub profile_id_by_old: HashMap<Uuid, ProfileId>,
    /// The `migration` entity (§1a): the emitter every synthesized genesis event attributes to.
    pub migration_entity: EntityId,
    /// The durable per-surface entities (§1b), bound to Pete's profile.
    pub surfaces: SurfaceEntities,
}

/// Seed profiles, entities, and contexts for the migration, returning the remaps for the resource
/// synthesis to consume. `resources` is the active production source (`source::active_resources`); the
/// distinct originator ∪ owner profiles and the distinct referenced contexts are derived from it.
pub async fn run(pool: &PgPool, resources: &[SourceResource]) -> Result<BootstrapMaps> {
    // --- Source reads (public.*, qualified) happen first, off the bare pool. -------------------------
    // Profiles: the distinct originator ∪ owner set across active resources (§2).
    let mut profile_ids: BTreeSet<Uuid> = BTreeSet::new();
    for r in resources {
        profile_ids.insert(r.originator_profile_id);
        profile_ids.insert(r.owner_profile_id);
    }
    let profile_ids: Vec<Uuid> = profile_ids.into_iter().collect();
    let src_profiles = source::profiles(pool, &profile_ids).await?;

    // Pete's profile: the owner of the resources (one human profile in production; the fixture has
    // one). Pick the owner of the most resources, tie-broken by smallest uuid, for determinism.
    let pete_old = pick_owner_profile(resources).context(
        "cannot bootstrap entities: no active resources to derive an owner profile from",
    )?;

    // Contexts: the distinct contexts referenced by active resources, looked up by name (§2).
    let referenced: BTreeSet<Uuid> = resources.iter().map(|r| r.kb_context_id).collect();
    let name_by_old: HashMap<Uuid, String> = source::contexts(pool)
        .await?
        .into_iter()
        .map(|c| (c.id, c.name))
        .collect();

    // --- All temper_next writes run in one transaction with `search_path = temper_next, public` so
    // the kb_profiles triggers (personal-team + root-membership maintenance) resolve their unqualified
    // table references into temper_next, not the bare pool's `public` (where they'd hit production's
    // differently-shaped tables). The explicit `temper_next.` prefixes below are belt-and-suspenders. -
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL search_path TO temper_next, public")
        .execute(&mut *tx)
        .await?;

    let mut profile_id_by_old: HashMap<Uuid, ProfileId> = HashMap::new();
    for p in &src_profiles {
        // handle is NOT NULL UNIQUE in the destination; production slug is the canonical source
        // (NOT NULL UNIQUE since `20260407000002`), with a sluggified display_name fallback (§1).
        let handle = p
            .slug
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| slugify(&p.display_name));
        let new_id = insert_profile(&mut tx, p.id, &handle, &p.display_name).await?;
        profile_id_by_old.insert(p.id, new_id);
    }

    let pete = *profile_id_by_old
        .get(&pete_old)
        .context("owner profile missing from profile remap (source::profiles gap)")?;

    // Entities: one migration emitter (§1a) + three durable per-surface emitters (§1b).
    let migration_meta = serde_json::to_value(MigrationMeta {
        intent: "migration",
        source: "temper-production",
    })?;
    let migration_entity = insert_entity(&mut tx, pete, "migration", &migration_meta).await?;
    let surfaces = SurfaceEntities {
        cli: insert_entity(&mut tx, pete, "pete@cli", &surface_meta("cli")?).await?,
        mcp: insert_entity(&mut tx, pete, "pete@mcp", &surface_meta("mcp")?).await?,
        web: insert_entity(&mut tx, pete, "pete@web", &surface_meta("web")?).await?,
    };

    let mut context_id_by_old: HashMap<Uuid, ContextId> = HashMap::new();
    for old in referenced {
        let name = name_by_old
            .get(&old)
            .with_context(|| format!("referenced context {old} absent from public.kb_contexts"))?;
        let new_id = insert_context(&mut tx, name).await?;
        context_id_by_old.insert(old, new_id);
    }

    tx.commit().await?;

    Ok(BootstrapMaps {
        context_id_by_old,
        profile_id_by_old,
        migration_entity,
        surfaces,
    })
}

/// Insert one `temper_next.kb_profiles` row, returning its new id. Tries the preferred `handle`; on a
/// uniqueness collision (two production profiles sluggifying to the same handle) disambiguates with a
/// short id suffix so the `NOT NULL UNIQUE` handle constraint always holds.
async fn insert_profile(
    conn: &mut sqlx::PgConnection,
    old_id: Uuid,
    handle: &str,
    display_name: &str,
) -> Result<ProfileId> {
    let inserted: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO temper_next.kb_profiles (handle, display_name) VALUES ($1, $2) \
         ON CONFLICT (handle) DO NOTHING RETURNING id",
    )
    .bind(handle)
    .bind(display_name)
    .fetch_optional(&mut *conn)
    .await?;
    let id = match inserted {
        Some(id) => id,
        None => {
            let disambiguated = format!("{handle}-{}", &old_id.simple().to_string()[..8]);
            sqlx::query_scalar(
                "INSERT INTO temper_next.kb_profiles (handle, display_name) VALUES ($1, $2) \
                 RETURNING id",
            )
            .bind(disambiguated)
            .bind(display_name)
            .fetch_one(&mut *conn)
            .await?
        }
    };
    Ok(ProfileId::from(id))
}

/// Insert one `temper_next.kb_entities` row (administrative — no event), returning its new id.
async fn insert_entity(
    conn: &mut sqlx::PgConnection,
    profile: ProfileId,
    name: &str,
    metadata: &serde_json::Value,
) -> Result<EntityId> {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO temper_next.kb_entities (profile_id, name, metadata) VALUES ($1, $2, $3) \
         RETURNING id",
    )
    .bind(profile.uuid())
    .bind(name)
    .bind(metadata)
    .fetch_one(&mut *conn)
    .await?;
    Ok(EntityId::from(id))
}

/// Insert one thin, unowned `temper_next.kb_contexts` row by name (§2), returning its new id.
async fn insert_context(conn: &mut sqlx::PgConnection, name: &str) -> Result<ContextId> {
    let id: Uuid =
        sqlx::query_scalar("INSERT INTO temper_next.kb_contexts (name) VALUES ($1) RETURNING id")
            .bind(name)
            .fetch_one(&mut *conn)
            .await?;
    Ok(ContextId::from(id))
}

/// Choose Pete's profile = the owner of the most active resources (tie-broken by smallest uuid for
/// determinism). Production has a single human profile; the fixture has one owner.
fn pick_owner_profile(resources: &[SourceResource]) -> Option<Uuid> {
    let mut counts: HashMap<Uuid, usize> = HashMap::new();
    for r in resources {
        *counts.entry(r.owner_profile_id).or_default() += 1;
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0)))
        .map(|(id, _)| id)
}

/// The `migration` entity's metadata (§1a): the established `intent=migration` pattern at the entity
/// tier. A wall-clock `migrated_at` is deliberately omitted (kept deterministic).
#[derive(serde::Serialize)]
struct MigrationMeta {
    intent: &'static str,
    source: &'static str,
}

/// A per-surface entity's metadata marker (§1b). Session/device specifics live in event metadata at
/// emit time, never here.
#[derive(serde::Serialize)]
struct SurfaceMeta<'a> {
    surface: &'a str,
}

/// Surface entities carry a small surface marker in metadata; session/device specifics live in event
/// metadata at emit time, never here (§1b).
fn surface_meta(surface: &str) -> Result<serde_json::Value> {
    Ok(serde_json::to_value(SurfaceMeta { surface })?)
}

/// Lowercase alphanumeric-or-dash slug (mirrors the surfaces' `slugify`); the fallback when a source
/// profile has no slug.
fn slugify(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|seg| !seg.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}
