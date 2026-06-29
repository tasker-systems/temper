//! Thin seed loader: instantiates a seed's substrate by firing the reusable seeding mutations
//! through the single `events::fire` surface (`cogmap_genesis`/`resource_create`/`facet_set`/
//! `relationship_assert`). The Rust side never inserts substrate tables directly — it threads YAML inputs
//! into `SeedAction`s and records the `key → Uuid` map (including the implicit `telos` key) the runner
//! needs. The whole load runs in one transaction so a seed is instantiated atomically.
//!
//! The loader consumes a `Seed` — the template document a foundational cogmap is born from. A
//! scenario reaches here through `runner::run_scenario`, which resolves its seed reference first;
//! loading a seed standalone and loading it through a scenario are the same code path by
//! construction (the load-path equivalence proof pins this).

use crate::events::{fire, EdgeHome, SeedAction};
use crate::ids::{CogmapId, EntityId, ProfileId};
use crate::scenario::model::*;
use anyhow::{Context, Result};
use sqlx::{PgConnection, PgPool};
use std::collections::HashMap;
use uuid::Uuid;

pub struct Loaded {
    pub cogmap: Uuid,
    pub emitter: Uuid,
    pub owner: Uuid,
    pub keys: HashMap<String, Uuid>,
}

pub async fn load_seed(pool: &PgPool, s: &Seed) -> Result<Loaded> {
    let mut tx = pool.begin().await?;
    let (profiles, entities) = insert_world_identity(&mut tx, &s.world).await?;
    let (cogmap, owner, emitter, mut keys) =
        genesis_cogmap(&mut tx, s, &profiles, &entities).await?;
    load_resources(&mut tx, s, cogmap, owner, emitter, &mut keys).await?;
    load_edges(&mut tx, s, cogmap, emitter, &keys).await?;
    tx.commit().await?;

    Ok(Loaded {
        cogmap: cogmap.uuid(),
        emitter: emitter.uuid(),
        owner: owner.uuid(),
        keys,
    })
}

/// World identity rows (tiny — direct inserts, not event-projected for M1): profiles, then entities
/// keyed by their owning profile. Returns the `handle → id` and `name → id` maps the rest needs.
async fn insert_world_identity(
    tx: &mut PgConnection,
    world: &WorldDef,
) -> Result<(HashMap<String, Uuid>, HashMap<String, Uuid>)> {
    let mut profiles: HashMap<String, Uuid> = HashMap::new();
    for p in &world.profiles {
        let id = sqlx::query_scalar!(
            "INSERT INTO kb_profiles (handle, display_name, system_access) \
             VALUES ($1,$2,$3::system_access) RETURNING id",
            p.handle,
            p.display_name,
            p.system_access.as_sql() as _,
        )
        .fetch_one(&mut *tx)
        .await?;
        profiles.insert(p.handle.clone(), id);
    }
    let mut entities: HashMap<String, Uuid> = HashMap::new();
    for e in &world.entities {
        let pid = profiles.get(&e.profile).with_context(|| {
            format!("entity {} references unknown profile {}", e.name, e.profile)
        })?;
        let id = sqlx::query_scalar!(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1,$2,'{}'::jsonb) RETURNING id",
            pid,
            e.name,
        )
        .fetch_one(&mut *tx)
        .await?;
        entities.insert(e.name.clone(), id);
    }
    Ok((profiles, entities))
}

/// Genesis → cogmap + telos charter resource. The charter is real content-blocks (block-0 statement,
/// blocks 1..n questions-with-context, then framing), chunked + embedded Rust-side exactly like an
/// ordinary resource body; cogmap_genesis persists the block→chunk nesting and returns BOTH ids, so
/// the loader no longer re-fetches telos_resource_id. Returns the cogmap, its resolved owner/emitter,
/// and the `key → id` map seeded with the implicit `telos` key.
async fn genesis_cogmap(
    tx: &mut PgConnection,
    s: &Seed,
    profiles: &HashMap<String, Uuid>,
    entities: &HashMap<String, Uuid>,
) -> Result<(CogmapId, ProfileId, EntityId, HashMap<String, Uuid>)> {
    let owner = ProfileId::from(
        *profiles
            .get(&s.cogmap.owner)
            .context("cogmap.owner not in world.profiles")?,
    );
    let emitter = EntityId::from(
        *entities
            .get(&s.cogmap.emitter)
            .context("cogmap.emitter not in world.entities")?,
    );

    let charter_specs = s.cogmap.telos.block_specs();
    let charter_refs: Vec<(Option<&str>, &str)> = charter_specs
        .iter()
        .map(|(role, prose)| (Some(*role), prose.as_str()))
        .collect();
    let charter_blocks = crate::content::prepare_blocks(&charter_refs)?;
    let (cogmap, telos) = fire(
        &mut *tx,
        SeedAction::CogmapGenesis {
            name: &s.name,
            telos_title: &s.cogmap.telos.title,
            charter: &charter_blocks,
            cogmap_id: None,
            telos_resource_id: None,
            owner,
            emitter,
        },
    )
    .await?
    .cogmap_genesis()?;

    let mut keys: HashMap<String, Uuid> = HashMap::new();
    keys.insert("telos".to_string(), telos.uuid());
    Ok((cogmap, owner, emitter, keys))
}

/// Resource-create + optional facet-set, in seed order, recording each `key → id` (and guarding the
/// reserved/duplicate keys). Homes every resource in the just-born cogmap.
async fn load_resources(
    tx: &mut PgConnection,
    s: &Seed,
    cogmap: CogmapId,
    owner: ProfileId,
    emitter: EntityId,
    keys: &mut HashMap<String, Uuid>,
) -> Result<()> {
    for r in &s.resources {
        // Reserved/duplicate key guard: `keys` already holds the implicit charter key `telos`; a
        // resource keyed `telos` (or a duplicate of an earlier resource key) would silently shadow it
        // and corrupt the charter read. Fail fast with a clear message instead.
        if keys.contains_key(&r.key) {
            anyhow::bail!(
                "seed resource key {:?} collides with an existing key (the charter reserves `telos`); rename it",
                r.key
            );
        }
        let title = r.title.clone().unwrap_or_else(|| r.key.clone());
        // An ordinary resource's body is one content-block (content-block §"Write path"); it chunks +
        // embeds Rust-side (sha256 + bge-768), and resource_create persists the block→chunk nesting.
        // A multi-paragraph body that exceeds one 510-token window arrives as a multi-chunk block.
        let blocks = crate::content::prepare_blocks(&[(None, r.body.as_str())])?;
        let rid = fire(
            &mut *tx,
            SeedAction::ResourceCreate {
                title: &title,
                origin_uri: &r.origin_uri,
                resource_id: None,
                home: crate::payloads::AnchorRef::cogmap(cogmap),
                owner,
                originator: None,
                blocks: &blocks,
                doc_type: r.doc_type.as_deref(),
                emitter,
            },
        )
        .await?
        .resource()?;
        keys.insert(r.key.clone(), rid.uuid());

        if let Some(f) = &r.facets {
            let values = serde_json::Value::Object(f.values().clone());
            fire(
                &mut *tx,
                SeedAction::FacetSet {
                    resource: rid,
                    values: &values,
                    weight: f.weight(),
                    emitter,
                },
            )
            .await?;
        }
    }
    Ok(())
}

/// Edge-assertion loop: resolve each endpoint through the resource `key → id` map and fire a
/// forward-polarity relationship homed in the cogmap.
async fn load_edges(
    tx: &mut PgConnection,
    s: &Seed,
    cogmap: CogmapId,
    emitter: EntityId,
    keys: &HashMap<String, Uuid>,
) -> Result<()> {
    for e in &s.edges {
        let src = (*keys
            .get(&e.from)
            .with_context(|| format!("edge from unknown key {}", e.from))?)
        .into();
        let tgt = (*keys
            .get(&e.to)
            .with_context(|| format!("edge to unknown key {}", e.to))?)
        .into();
        fire(
            &mut *tx,
            SeedAction::RelationshipAssert {
                src,
                tgt,
                kind: e.kind,
                polarity: crate::payloads::EdgePolarity::Forward,
                label: e.label.as_deref(),
                weight: e.weight,
                home: EdgeHome::Cogmap(cogmap),
                emitter,
            },
        )
        .await?;
    }
    Ok(())
}
