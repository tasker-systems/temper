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
use crate::events::{fire, EdgeHome, SeedAction};
use crate::ids::{CogmapId, ContextId, EntityId, ProfileId, ResourceId};
use crate::scenario::access::model::*;
use anyhow::{Context, Result};
use sqlx::{PgConnection, PgPool};
use std::collections::HashMap;
use uuid::Uuid;

/// Resolved identity maps for the check-evaluator. Edges are keyed by their scenario `key` and carry
/// the `kb_edges.id` captured at fire time (NOT resolved by the non-unique `label` at eval time).
pub struct LoadedAccess {
    pub profiles: HashMap<String, Uuid>,  // handle -> id
    pub teams: HashMap<String, Uuid>,     // slug -> id (incl. trigger-created personal teams)
    pub contexts: HashMap<String, Uuid>,  // name -> id
    pub cogmaps: HashMap<String, Uuid>,   // name -> id
    pub resources: HashMap<String, Uuid>, // key -> id
    pub edges: HashMap<String, Uuid>,     // edge key -> kb_edges.id
}

pub async fn load(pool: &PgPool, world: &AccessWorld) -> Result<LoadedAccess> {
    load_scaled(pool, world, 1).await
}

/// [`load`] with a multiplier applied to every `populations:` `count`.
///
/// The multiplier is a LOAD-time argument and deliberately not a fixture field: one declaration
/// then describes one corpus *shape*, and a test can load it at the size a `#[sqlx::test]` can
/// afford while the seeder binary loads the same shape at measurement size. Putting scale in the
/// YAML would fork the shape into "the small one" and "the big one", which is exactly how the two
/// drift. Hand-declared `resources:` are never scaled — they are named referents, and duplicating
/// them would break the keys checks resolve through.
pub async fn load_scaled(pool: &PgPool, world: &AccessWorld, scale: u32) -> Result<LoadedAccess> {
    let mut tx = pool.begin().await?;

    let mut teams: HashMap<String, Uuid> = HashMap::new();
    insert_teams(&mut tx, world, &mut teams).await?;
    insert_team_dag(&mut tx, world, &teams).await?;

    let mut profiles: HashMap<String, Uuid> = HashMap::new();
    insert_profiles(&mut tx, world, &mut profiles).await?;
    let mut entities: HashMap<String, Uuid> = HashMap::new();
    insert_entities(&mut tx, world, &profiles, &mut entities).await?;
    insert_memberships(&mut tx, world, &teams, &profiles).await?;

    let mut contexts: HashMap<String, Uuid> = HashMap::new();
    insert_contexts(&mut tx, world, &profiles, &teams, &mut contexts).await?;
    refresh_team_map(&mut tx, &mut teams).await?;
    insert_context_shares(&mut tx, world, &contexts, &teams).await?;

    let placeholder_telos = insert_placeholder_telos(&mut tx).await?;
    let mut cogmaps: HashMap<String, Uuid> = HashMap::new();
    insert_cogmaps(
        &mut tx,
        world,
        &profiles,
        &entities,
        &teams,
        placeholder_telos,
        &mut cogmaps,
    )
    .await?;

    let mut resources: HashMap<String, Uuid> = HashMap::new();
    insert_resources(
        &mut tx,
        world,
        &profiles,
        &cogmaps,
        &contexts,
        &teams,
        &mut resources,
    )
    .await?;

    // Generated bulk, before edges so a future population-homed edge has endpoints to resolve.
    //
    // Running AFTER the hand-declared resources is what makes shadowing POSSIBLE, not what prevents
    // it — both write into one `resources` map, so a generated `<prefix>-0000` colliding with an
    // `AccessResourceDef.key` would overwrite the named referent and every later `check:` would
    // silently resolve to the generated row. `generate` refuses on collision rather than relying on
    // ordering.
    super::population::generate(
        &mut tx,
        world,
        scale,
        &profiles,
        &entities,
        &contexts,
        &cogmaps,
        &teams,
        &mut resources,
    )
    .await?;

    let mut edges: HashMap<String, Uuid> = HashMap::new();
    insert_edges(
        &mut tx, world, &resources, &cogmaps, &contexts, &entities, &mut edges,
    )
    .await?;

    tx.commit().await?;
    Ok(LoadedAccess {
        profiles,
        teams,
        contexts,
        cogmaps,
        resources,
        edges,
    })
}

/// 1. Teams first — the sync_system_membership trigger joins enabled profiles to the temper-system
///    root by slug, so the root must exist before any profile insert.
async fn insert_teams(
    tx: &mut PgConnection,
    world: &AccessWorld,
    teams: &mut HashMap<String, Uuid>,
) -> Result<()> {
    for t in &world.teams {
        // Reconcile on slug rather than insert blind. Every access fixture must declare
        // `temper-system` — the DAG parents reference it — but the L0 kernel migration
        // (`20260625000001`) already creates that team, so a blind INSERT can only work on a
        // schema that was reset first. That is true of every `#[sqlx::test]` here and false of any
        // real migrated database, which is what made this loader unusable outside the test harness.
        // ON CONFLICT resolves the declaration ONTO the incumbent row, so a fixture adopts the
        // migration's root team instead of colliding with it.
        let id = sqlx::query_scalar!(
            "INSERT INTO kb_teams (slug, name) VALUES ($1,$2) \
             ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name RETURNING id",
            t.slug,
            t.name,
        )
        .fetch_one(&mut *tx)
        .await?;
        teams.insert(t.slug.clone(), id);
    }
    Ok(())
}

/// 2. Teams DAG (child -> parents).
async fn insert_team_dag(
    tx: &mut PgConnection,
    world: &AccessWorld,
    teams: &HashMap<String, Uuid>,
) -> Result<()> {
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
    Ok(())
}

/// 3. Profiles (trigger auto-joins the temper-system root for non-'none').
async fn insert_profiles(
    tx: &mut PgConnection,
    world: &AccessWorld,
    profiles: &mut HashMap<String, Uuid>,
) -> Result<()> {
    for p in &world.profiles {
        let id = sqlx::query_scalar!(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ($1,$2) RETURNING id",
            p.handle,
            p.display_name,
        )
        .fetch_one(&mut *tx)
        .await?;
        profiles.insert(p.handle.clone(), id);

        // Mint the now-authoritative standing row from the declared tier (see loader.rs for the
        // fuller rationale; Phase 2 A4 dropped the `system_access` projection write). Emitter falls
        // back to the `system` actor that seed_system creates before any access scenario loads.
        let (standing, admin) = match p.system_access.as_sql() {
            "admin" => ("approved", true),
            "approved" => ("approved", false),
            _ => ("denied", false),
        };
        sqlx::query!(
            "SELECT principal_standing_apply($1,'provision',$2,NULL,'scenario load')",
            id,
            standing
        )
        .fetch_one(&mut *tx)
        .await?;
        if admin {
            sqlx::query!(
                "SELECT principal_governance_set($1,true,NULL,'scenario load')",
                id
            )
            .fetch_one(&mut *tx)
            .await?;
        }
    }
    Ok(())
}

/// 4. Entities (event emitters).
async fn insert_entities(
    tx: &mut PgConnection,
    world: &AccessWorld,
    profiles: &HashMap<String, Uuid>,
    entities: &mut HashMap<String, Uuid>,
) -> Result<()> {
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
    Ok(())
}

/// 5. Sub-team memberships (root joins already trigger-maintained).
async fn insert_memberships(
    tx: &mut PgConnection,
    world: &AccessWorld,
    teams: &HashMap<String, Uuid>,
    profiles: &HashMap<String, Uuid>,
) -> Result<()> {
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
            m.role.as_sql() as _,
        )
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

/// 5b. Contexts — real owner-scoped kb_contexts rows (WS6 §2 amendment), referents for named homes
///     and shares. Each ContextDef names exactly one owner (a world profile handle or team slug);
///     owner is namespace-scoping only — reachability is still via context_shares. slug = slugify(name).
async fn insert_contexts(
    tx: &mut PgConnection,
    world: &AccessWorld,
    profiles: &HashMap<String, Uuid>,
    teams: &HashMap<String, Uuid>,
    contexts: &mut HashMap<String, Uuid>,
) -> Result<()> {
    for c in &world.contexts {
        let (owner_table, owner_id) = match (&c.owner_profile, &c.owner_team) {
            (Some(handle), None) => (
                "kb_profiles",
                *profiles.get(handle).with_context(|| {
                    format!(
                        "context {} owner_profile {} not in world.profiles",
                        c.name, handle
                    )
                })?,
            ),
            (None, Some(slug)) => (
                "kb_teams",
                *teams.get(slug).with_context(|| {
                    format!("context {} owner_team {} not in world.teams", c.name, slug)
                })?,
            ),
            (Some(_), Some(_)) => {
                anyhow::bail!("context {} sets both owner_profile and owner_team", c.name)
            }
            (None, None) => anyhow::bail!(
                "context {} must set exactly one of owner_profile / owner_team",
                c.name
            ),
        };
        let slug = crate::text::slugify(&c.name);
        let id = sqlx::query_scalar!(
            "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) VALUES ($1,$2,$3,$4) RETURNING id",
            owner_table,
            owner_id,
            slug,
            c.name,
        )
        .fetch_one(&mut *tx)
        .await?;
        contexts.insert(c.name.clone(), id);
    }
    Ok(())
}

/// 5c. Refresh the team map from the DB — profile inserts trigger personal teams
///     (`personal-<handle>`) that world.teams never declares.
async fn refresh_team_map(tx: &mut PgConnection, teams: &mut HashMap<String, Uuid>) -> Result<()> {
    for row in sqlx::query!("SELECT slug, id FROM kb_teams")
        .fetch_all(&mut *tx)
        .await?
    {
        teams.entry(row.slug).or_insert(row.id);
    }
    Ok(())
}

/// 5d. Context shares (kb_team_contexts) — the team's vis-reach includes the context.
async fn insert_context_shares(
    tx: &mut PgConnection,
    world: &AccessWorld,
    contexts: &HashMap<String, Uuid>,
    teams: &HashMap<String, Uuid>,
) -> Result<()> {
    for s in &world.context_shares {
        let cid = contexts
            .get(&s.context)
            .with_context(|| format!("share references unknown context {}", s.context))?;
        let tid = teams
            .get(&s.team)
            .with_context(|| format!("share references unknown team {}", s.team))?;
        sqlx::query!(
            "INSERT INTO kb_team_contexts (context_id, team_id) VALUES ($1,$2)",
            cid,
            tid,
        )
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

/// 6. A single home-less placeholder telos resource for the bare producer maps
///    (kb_cogmaps.telos_resource_id is NOT NULL; bare maps carry no charter — mirrors 03_seed's
///    shared public telos). Genesis maps create their own telos.
async fn insert_placeholder_telos(tx: &mut PgConnection) -> Result<Uuid> {
    Ok(sqlx::query_scalar!(
        "INSERT INTO kb_resources (title, origin_uri) \
         VALUES ('placeholder: bare-cogmap telos','temper://internal/placeholder-telos') RETURNING id",
    )
    .fetch_one(&mut *tx)
    .await?)
}

/// 7. Cogmaps. Bare maps: direct insert + team joins. Telos-bearing maps: cogmap_genesis.
async fn insert_cogmaps(
    tx: &mut PgConnection,
    world: &AccessWorld,
    profiles: &HashMap<String, Uuid>,
    entities: &HashMap<String, Uuid>,
    teams: &HashMap<String, Uuid>,
    placeholder_telos: Uuid,
    cogmaps: &mut HashMap<String, Uuid>,
) -> Result<()> {
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
                    &mut *tx,
                    SeedAction::CogmapGenesis {
                        name: &c.name,
                        telos_title: &telos.title,
                        charter: &blocks,
                        cogmap_id: None,
                        telos_resource_id: None,
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
    Ok(())
}

/// 8. Resources: identity + home (context|cogmap) + explicit grants. Direct inserts (ports 03_seed).
async fn insert_resources(
    tx: &mut PgConnection,
    world: &AccessWorld,
    profiles: &HashMap<String, Uuid>,
    cogmaps: &HashMap<String, Uuid>,
    contexts: &HashMap<String, Uuid>,
    teams: &HashMap<String, Uuid>,
    resources: &mut HashMap<String, Uuid>,
) -> Result<()> {
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
            HomeDef::Context { name } => (
                "kb_contexts",
                match name {
                    Some(n) => *contexts.get(n).with_context(|| {
                        format!("resource {} homes in unknown context {}", r.key, n)
                    })?,
                    // anonymous unshared workspace anchor (pre-amendment form)
                    None => Uuid::now_v7(),
                },
            ),
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
        insert_resource_grants(&mut *tx, rid, owner, &r.grants, &r.key, teams, profiles).await?;
        resources.insert(r.key.clone(), rid);
    }
    Ok(())
}

/// The ONE `kb_access_grants` write in this crate.
///
/// Extracted rather than copied when the population generator needed the same insert. Two reasons,
/// and the second is the load-bearing one:
///
/// * A grant's shape (which caps, which granter) is exactly the kind of logic whose second copy
///   drifts silently from its first.
/// * `.github/scripts/audit-grant-sinks.sh` counts grant write-sites per file against a reviewed
///   baseline, so a duplicate would have added a NEW sink and required a baseline bump. That
///   script's own header records why that matters: absorbing a movement into the baseline "teaches
///   the next reader that the number moves for cosmetic reasons, which is how a tripwire stops
///   being read." Calling one helper keeps the count at one because there genuinely is one write.
///
/// AUTHORITY / ATTENUATION, the two questions the audit playbook asks: `granter` is the resource's
/// owner, recorded as `granted_by_profile_id`, and a fixture's owner holds every capability on a
/// resource it just created — so the conferred set is a subset by construction. This is fixture
/// loading, not a request path; no gate is bypassed because no principal is acting.
#[allow(clippy::too_many_arguments)]
pub(super) async fn insert_resource_grants(
    tx: &mut PgConnection,
    resource: Uuid,
    granter: Uuid,
    grants: &[GrantDef],
    subject_label: &str,
    teams: &HashMap<String, Uuid>,
    profiles: &HashMap<String, Uuid>,
) -> Result<()> {
    for g in grants {
        let (ga_table, ga_id) = match &g.to {
            GrantAnchor::Team { slug } => (
                "kb_teams",
                *teams.get(slug).with_context(|| {
                    format!("grant on {subject_label} references unknown team {slug}")
                })?,
            ),
            GrantAnchor::Profile { handle } => (
                "kb_profiles",
                *profiles.get(handle).with_context(|| {
                    format!("grant on {subject_label} references unknown profile {handle}")
                })?,
            ),
        };
        sqlx::query!(
            "INSERT INTO kb_access_grants \
             (subject_table, subject_id, principal_table, principal_id, can_read, can_write, can_delete, can_grant, granted_by_profile_id) \
             VALUES ('kb_resources',$1,$2,$3,$4,$5,$6,$7,$8)",
            resource,
            ga_table,
            ga_id,
            g.can_read,
            g.can_write,
            g.can_delete,
            g.can_grant,
            granter,
        )
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

/// 9. Edges: homed in a named cogmap, fired through relationship_assert. Capture each fired
///    `kb_edges.id` under the edge's `key` so checks resolve by stable id, not by non-unique label.
async fn insert_edges(
    tx: &mut PgConnection,
    world: &AccessWorld,
    resources: &HashMap<String, Uuid>,
    cogmaps: &HashMap<String, Uuid>,
    contexts: &HashMap<String, Uuid>,
    entities: &HashMap<String, Uuid>,
    edges: &mut HashMap<String, Uuid>,
) -> Result<()> {
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
        let home = match &e.home {
            EdgeHomeDef::Cogmap { name } => EdgeHome::Cogmap(CogmapId::from(
                *cogmaps
                    .get(name)
                    .with_context(|| format!("edge homes in unknown cogmap {}", name))?,
            )),
            EdgeHomeDef::Context { name } => EdgeHome::Context(ContextId::from(
                *contexts
                    .get(name)
                    .with_context(|| format!("edge homes in unknown context {}", name))?,
            )),
        };
        let emitter = EntityId::from(
            *entities
                .get(&e.emitter)
                .with_context(|| format!("edge emitter {} not in world.entities", e.emitter))?,
        );
        let edge_id = fire(
            &mut *tx,
            SeedAction::RelationshipAssert {
                src,
                tgt,
                kind: e.kind,
                polarity: crate::payloads::EdgePolarity::Forward,
                label: e.label.as_deref(),
                weight: e.weight,
                home,
                emitter,
            },
        )
        .await?
        .relationship()?;
        if edges.insert(e.key.clone(), Uuid::from(edge_id)).is_some() {
            anyhow::bail!("duplicate edge key {}", e.key);
        }
    }
    Ok(())
}

/// What a seeded corpus actually looks like, read back from the database.
///
/// **In the library rather than the `seed-corpus` binary, and that placement is the point.**
/// `cargo sqlx prepare --workspace` compiles lib targets only — measured: neither the plain
/// invocation nor `--all-targets` emits a cache entry for a `query!` in a bin, so the same macro
/// that verifies here fails an offline build there. The alternative was a per-crate cache for
/// temper-substrate, which emits its whole dependency closure: 131 files to cover four reads, the
/// trap `.claude/skills/sqlx-query-cache` records temper-api falling into at 255-against-11. So the
/// reads live where the ritual reaches, and the binary prints what they return.
///
/// They stayed macros rather than becoming declared exceptions because none of
/// `audit-sqlx-macro-exceptions.sh`'s four reasons fits a static `count(*)` — and that script's own
/// rule is that no fitting reason IS the answer: it converts.
#[derive(Debug, Clone)]
pub struct CorpusMeasurement {
    pub live_resources: i64,
    pub embedded_chunks: i64,
    /// `(handle, visible, owned)`, sorted by handle. `visible - owned` is what arrived through the
    /// team/grant arms — the arms that are EMPTY on the deployment whose numbers this corpus exists
    /// to replace, which is why the two are reported separately rather than as one total.
    pub per_principal: Vec<(String, i64, i64)>,
}

/// Read back the corpus a load produced: its size, and the gate's discriminating power over it.
pub async fn measure_corpus(pool: &PgPool, loaded: &LoadedAccess) -> Result<CorpusMeasurement> {
    // `AS "n!"` because `count(*)` is nullable to the macro (an aggregate over an outer join can be
    // NULL); it never is here, and the annotation says so rather than forcing an unwrap per site.
    let live_resources: i64 =
        sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM kb_resources WHERE is_active"#)
            .fetch_one(pool)
            .await?;
    let embedded_chunks: i64 = sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM kb_chunks WHERE is_current AND embedding IS NOT NULL"#
    )
    .fetch_one(pool)
    .await?;

    let mut handles: Vec<&String> = loaded.profiles.keys().collect();
    handles.sort();
    let mut per_principal = Vec::with_capacity(handles.len());
    for handle in handles {
        let id = loaded.profiles[handle];
        let seen: i64 = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM resources_visible_to($1)"#,
            id
        )
        .fetch_one(pool)
        .await?;
        let owned: i64 = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM kb_resource_homes WHERE owner_profile_id = $1"#,
            id
        )
        .fetch_one(pool)
        .await?;
        per_principal.push((handle.clone(), seen, owned));
    }

    Ok(CorpusMeasurement {
        live_resources,
        embedded_chunks,
        per_principal,
    })
}
