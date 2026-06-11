//! Access-world loader: persists an `AccessWorld` atomically in one transaction and returns the
//! `name → Uuid` maps the check-evaluator resolves through. Topology rows (teams, DAG, profiles,
//! entities, memberships, homes, grants, bare cogmaps) are direct inserts — the "tiny identity rows,
//! direct, not event-projected" convention the charter loader already uses. The only event-backed
//! writes are `cogmap_genesis` for a telos-bearing cogmap (so S4's charter has real blocks) and
//! `relationship_assert` for a homed edge (`kb_edges` carries NOT-NULL event FKs).
//!
//! Ordering is load-bearing: teams are inserted FIRST so the `sync_system_membership` trigger can
//! join enabled profiles to the `temper-system` root by slug.

use crate::content;
use crate::events::{fire, SeedAction};
use crate::ids::{CogmapId, EntityId, ProfileId, ResourceId};
use crate::scenario::access::model::*;
use anyhow::{Context, Result};
use sqlx::PgPool;
use std::collections::HashMap;
use uuid::Uuid;

/// Resolved identity maps for the check-evaluator (edges are resolved by label at eval time).
pub struct LoadedAccess {
    pub profiles: HashMap<String, Uuid>,  // handle -> id
    pub teams: HashMap<String, Uuid>,     // slug -> id
    pub cogmaps: HashMap<String, Uuid>,   // name -> id
    pub resources: HashMap<String, Uuid>, // key -> id
}

pub async fn load(pool: &PgPool, world: &AccessWorld) -> Result<LoadedAccess> {
    let mut tx = pool.begin().await?;

    // 1. Teams first — the sync_system_membership trigger joins enabled profiles to the
    //    temper-system root by slug, so the root must exist before any profile insert.
    let mut teams: HashMap<String, Uuid> = HashMap::new();
    for t in &world.teams {
        let id = sqlx::query_scalar!(
            "INSERT INTO kb_teams (slug, name) VALUES ($1,$2) RETURNING id",
            t.slug,
            t.name,
        )
        .fetch_one(&mut *tx)
        .await?;
        teams.insert(t.slug.clone(), id);
    }
    // 2. Teams DAG (child -> parents).
    for t in &world.teams {
        let child = teams.get(&t.slug).expect("team just inserted");
        for parent in &t.parents {
            let pid = teams
                .get(parent)
                .with_context(|| format!("team {} references unknown parent {}", t.slug, parent))?;
            sqlx::query!(
                "INSERT INTO kb_teams_parents (child_id, parent_id) VALUES ($1,$2)",
                child,
                pid,
            )
            .execute(&mut *tx)
            .await?;
        }
    }
    // 3. Profiles (trigger auto-joins the temper-system root for non-'none').
    let mut profiles: HashMap<String, Uuid> = HashMap::new();
    for p in &world.profiles {
        let id = sqlx::query_scalar!(
            "INSERT INTO kb_profiles (handle, display_name, system_access) \
             VALUES ($1,$2,$3::system_access) RETURNING id",
            p.handle,
            p.display_name,
            p.system_access as _,
        )
        .fetch_one(&mut *tx)
        .await?;
        profiles.insert(p.handle.clone(), id);
    }
    // 4. Entities (event emitters).
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
    // 5. Sub-team memberships (root joins already trigger-maintained).
    for m in &world.memberships {
        let tid = teams
            .get(&m.team)
            .with_context(|| format!("membership references unknown team {}", m.team))?;
        let pid = profiles
            .get(&m.profile)
            .with_context(|| format!("membership references unknown profile {}", m.profile))?;
        sqlx::query!(
            "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1,$2,$3::team_role)",
            tid,
            pid,
            m.role as _,
        )
        .execute(&mut *tx)
        .await?;
    }
    // 6. A single home-less placeholder telos resource for the bare producer maps
    //    (kb_cogmaps.telos_resource_id is NOT NULL; bare maps carry no charter — mirrors 03_seed's
    //    shared public telos). Genesis maps create their own telos.
    let placeholder_telos = sqlx::query_scalar!(
        "INSERT INTO kb_resources (title, origin_uri) \
         VALUES ('placeholder: bare-cogmap telos','temper://internal/placeholder-telos') RETURNING id",
    )
    .fetch_one(&mut *tx)
    .await?;

    // 7. Cogmaps. Bare maps: direct insert + team joins. Telos-bearing maps: cogmap_genesis.
    let mut cogmaps: HashMap<String, Uuid> = HashMap::new();
    for c in &world.cogmaps {
        let cid = match &c.telos {
            None => {
                sqlx::query_scalar!(
                    "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ($1,$2) RETURNING id",
                    c.name,
                    placeholder_telos,
                )
                .fetch_one(&mut *tx)
                .await?
            }
            Some(telos) => {
                let owner = ProfileId::from(
                    *profiles
                        .get(c.owner.as_deref().context("genesis cogmap needs owner")?)
                        .context("cogmap.owner not in world.profiles")?,
                );
                let emitter = EntityId::from(
                    *entities
                        .get(
                            c.emitter
                                .as_deref()
                                .context("genesis cogmap needs emitter")?,
                        )
                        .context("cogmap.emitter not in world.entities")?,
                );
                let specs = telos.block_specs();
                let refs: Vec<(Option<&str>, &str)> =
                    specs.iter().map(|(r, p)| (Some(*r), p.as_str())).collect();
                let blocks = content::prepare_blocks(&refs)?;
                let (cogmap, _telos) = fire(
                    &mut tx,
                    SeedAction::CogmapGenesis {
                        name: &c.name,
                        telos_title: &telos.title,
                        charter: &blocks,
                        owner,
                        emitter,
                    },
                )
                .await?
                .cogmap_genesis()?;
                cogmap.uuid()
            }
        };
        for team in &c.teams {
            let tid = teams
                .get(team)
                .with_context(|| format!("cogmap {} joins unknown team {}", c.name, team))?;
            sqlx::query!(
                "INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1,$2)",
                cid,
                tid,
            )
            .execute(&mut *tx)
            .await?;
        }
        cogmaps.insert(c.name.clone(), cid);
    }

    // 8. Resources: identity + home (context|cogmap) + explicit grants. Direct inserts (ports 03_seed).
    let mut resources: HashMap<String, Uuid> = HashMap::new();
    for r in &world.resources {
        let owner = *profiles.get(&r.owner).with_context(|| {
            format!("resource {} owner {} not in world.profiles", r.key, r.owner)
        })?;
        let rid = sqlx::query_scalar!(
            "INSERT INTO kb_resources (title, origin_uri) VALUES ($1,$2) RETURNING id",
            r.title,
            r.origin_uri,
        )
        .fetch_one(&mut *tx)
        .await?;
        let (anchor_table, anchor_id) = match &r.home {
            HomeDef::Cogmap { name } => (
                "kb_cogmaps",
                *cogmaps.get(name).with_context(|| {
                    format!("resource {} homes in unknown cogmap {}", r.key, name)
                })?,
            ),
            HomeDef::Context {} => ("kb_contexts", Uuid::now_v7()),
        };
        sqlx::query!(
            "INSERT INTO kb_resource_homes \
             (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
             VALUES ($1,$2,$3,$4,$4)",
            rid,
            anchor_table,
            anchor_id,
            owner,
        )
        .execute(&mut *tx)
        .await?;
        for g in &r.grants {
            let (ga_table, ga_id) = match &g.to {
                GrantAnchor::Team { slug } => (
                    "kb_teams",
                    *teams.get(slug).with_context(|| {
                        format!("grant on {} references unknown team {}", r.key, slug)
                    })?,
                ),
                GrantAnchor::Profile { handle } => (
                    "kb_profiles",
                    *profiles.get(handle).with_context(|| {
                        format!("grant on {} references unknown profile {}", r.key, handle)
                    })?,
                ),
            };
            sqlx::query!(
                "INSERT INTO kb_resource_access \
                 (resource_id, anchor_table, anchor_id, can_read, can_write, can_delete, can_grant, granted_by_profile_id) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
                rid,
                ga_table,
                ga_id,
                g.can_read,
                g.can_write,
                g.can_delete,
                g.can_grant,
                owner,
            )
            .execute(&mut *tx)
            .await?;
        }
        resources.insert(r.key.clone(), rid);
    }

    // 9. Edges: homed in a named cogmap, fired through relationship_assert.
    for e in &world.edges {
        let src = ResourceId::from(
            *resources
                .get(&e.from)
                .with_context(|| format!("edge from unknown key {}", e.from))?,
        );
        let tgt = ResourceId::from(
            *resources
                .get(&e.to)
                .with_context(|| format!("edge to unknown key {}", e.to))?,
        );
        let home = CogmapId::from(
            *cogmaps
                .get(&e.home)
                .with_context(|| format!("edge homes in unknown cogmap {}", e.home))?,
        );
        let emitter = EntityId::from(
            *entities
                .get(&e.emitter)
                .with_context(|| format!("edge emitter {} not in world.entities", e.emitter))?,
        );
        fire(
            &mut tx,
            SeedAction::RelationshipAssert {
                src,
                tgt,
                kind: e.kind,
                label: e.label.as_deref(),
                weight: e.weight,
                home,
                emitter,
            },
        )
        .await?;
    }

    tx.commit().await?;
    Ok(LoadedAccess {
        profiles,
        teams,
        cogmaps,
        resources,
    })
}
