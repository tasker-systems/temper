//! Thin scenario loader: instantiates a scenario's substrate by calling the reusable mutation SQL
//! functions (`cogmap_genesis`, `resource_create`, `facet_set`, `relationship_assert`). The Rust side
//! never inserts substrate tables directly — it threads YAML inputs into the functions and records the
//! `key → Uuid` map (including the implicit `telos` key) the runner needs.

use crate::affinity::EdgeKind;
use crate::scenario::model::*;
use anyhow::{Context, Result};
use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

pub struct Loaded {
    pub cogmap: Uuid,
    pub emitter: Uuid,
    pub keys: HashMap<String, Uuid>,
}

pub(crate) fn edge_kind_sql(k: EdgeKind) -> &'static str {
    match k {
        EdgeKind::Express => "express",
        EdgeKind::Contains => "contains",
        EdgeKind::LeadsTo => "leads_to",
        EdgeKind::Near => "near",
    }
}

pub async fn load_scenario(pool: &PgPool, s: &Scenario) -> Result<Loaded> {
    // world identity rows (tiny — direct, not event-projected for M1)
    let mut profiles: HashMap<String, Uuid> = HashMap::new();
    for p in &s.world.profiles {
        let id = sqlx::query_scalar!(
            "INSERT INTO kb_profiles (handle, display_name, system_access) \
             VALUES ($1,$2,$3::system_access) RETURNING id",
            p.handle,
            p.display_name,
            p.system_access as _,
        )
        .fetch_one(pool)
        .await?;
        profiles.insert(p.handle.clone(), id);
    }
    let mut entities: HashMap<String, Uuid> = HashMap::new();
    for e in &s.world.entities {
        let pid = profiles.get(&e.profile).with_context(|| {
            format!("entity {} references unknown profile {}", e.name, e.profile)
        })?;
        let id = sqlx::query_scalar!(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1,$2,'{}'::jsonb) RETURNING id",
            pid,
            e.name,
        )
        .fetch_one(pool)
        .await?;
        entities.insert(e.name.clone(), id);
    }

    let owner = *profiles
        .get(&s.cogmap.owner)
        .context("cogmap.owner not in world.profiles")?;
    let emitter = *entities
        .get(&s.cogmap.emitter)
        .context("cogmap.emitter not in world.entities")?;

    // genesis (existing fn) → cogmap + telos charter resource
    let cogmap = sqlx::query_scalar!(
        "SELECT cogmap_genesis($1,$2,$3,$4,$5,$6)",
        s.name,
        s.cogmap.telos.title,
        s.cogmap.telos.statement,
        &s.cogmap.telos.questions,
        owner,
        emitter,
    )
    .fetch_one(pool)
    .await?
    .context("cogmap_genesis returned null")?;
    let telos = sqlx::query_scalar!(
        "SELECT telos_resource_id FROM kb_cogmaps WHERE id=$1",
        cogmap
    )
    .fetch_one(pool)
    .await?;

    let mut keys: HashMap<String, Uuid> = HashMap::new();
    keys.insert("telos".to_string(), telos);

    for r in &s.resources {
        let title = r.title.clone().unwrap_or_else(|| r.key.clone());
        // An ordinary resource's body is one content-block (content-block §"Write path"); it chunks +
        // embeds Rust-side (sha256 + bge-768), and resource_create persists the block→chunk nesting.
        // A multi-paragraph body that exceeds one 510-token window arrives as a multi-chunk block.
        let blocks = crate::content::prepare_blocks(&[r.body.as_str()])?;
        let blocks_json = serde_json::to_value(&blocks)?;
        let rid = sqlx::query_scalar!(
            "SELECT resource_create($1,$2,$3,$4,$5,$6,$7)",
            title,
            r.origin_uri,
            cogmap,
            owner,
            blocks_json,
            r.doc_type,
            emitter,
        )
        .fetch_one(pool)
        .await?
        .context("resource_create returned null")?;
        keys.insert(r.key.clone(), rid);

        if let Some(f) = &r.facets {
            let values = serde_json::Value::Object(f.values().clone());
            sqlx::query!(
                "SELECT facet_set($1,$2,$3,$4)",
                rid,
                values,
                f.weight(),
                emitter,
            )
            .fetch_one(pool)
            .await?;
        }
    }

    for e in &s.edges {
        let src = *keys
            .get(&e.from)
            .with_context(|| format!("edge from unknown key {}", e.from))?;
        let tgt = *keys
            .get(&e.to)
            .with_context(|| format!("edge to unknown key {}", e.to))?;
        sqlx::query!(
            "SELECT relationship_assert($1,$2,$3::edge_kind,$4,$5,$6,$7)",
            src,
            tgt,
            edge_kind_sql(e.kind) as _,
            e.label,
            e.weight,
            cogmap,
            emitter,
        )
        .fetch_one(pool)
        .await?;
    }

    Ok(Loaded {
        cogmap,
        emitter,
        keys,
    })
}
