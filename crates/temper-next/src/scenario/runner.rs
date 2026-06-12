//! Scenario runner: resolves the scenario's seed (referenced or embedded), loads its substrate
//! through the same `loader::load_seed` path a standalone seed uses, then executes the ordered
//! `steps` runbook in-process (materialize / mutation / assert). Materialize reuses
//! `embed_chunks` and `materialize_cogmap`; the mutation steps (create_resource / set_facet /
//! assert_edge / fold_edge) each fire their matching `SeedAction`; assert evaluates each
//! expectation against the materialized regions. A per-lens
//! fingerprint cache backs the `reproducible` / `fingerprint_differs` checks. Any failed
//! expectation aborts with a descriptive error.

use crate::events::{fire, SeedAction};
use crate::ids::{CogmapId, EntityId, ProfileId, ResourceId};
use crate::scenario::loader::{self, Loaded};
use crate::scenario::model::*;
use crate::{embed, write};
use anyhow::{bail, Context, Result};
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use uuid::Uuid;

/// Which materialize path the runner drives at each `materialize` step. `Full` recomputes every
/// region (the production default, and what the corpus asserts against); `Incremental` reuses
/// unchanged components and re-clusters only changed ones — exercised by the `incremental ≡ full`
/// equivalence proof, where running a growth runbook this way must still satisfy every per-step
/// assertion the full path does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MaterializeMode {
    Full,
    Incremental,
}

/// `base_dir` anchors a `seed:` path reference — pass the scenario file's directory.
pub async fn run_scenario(pool: &PgPool, s: &Scenario, base_dir: &Path) -> Result<()> {
    run_scenario_with(pool, s, base_dir, MaterializeMode::Full).await
}

/// As [`run_scenario`], with an explicit materialize mode (see [`MaterializeMode`]).
pub async fn run_scenario_with(
    pool: &PgPool,
    s: &Scenario,
    base_dir: &Path,
    mode: MaterializeMode,
) -> Result<()> {
    let seed = s.resolve_seed(base_dir)?;
    let mut loaded = loader::load_seed(pool, &seed).await?;
    validate_lenses(pool, &loaded, &seed, &s.steps).await?;

    // per-lens fingerprints: `current` is the latest materialize; `previous` is the one before it.
    let mut current: HashMap<String, String> = HashMap::new();
    let mut previous: HashMap<String, String> = HashMap::new();

    for (i, step) in s.steps.iter().enumerate() {
        match step {
            Step::Materialize { lens } => {
                embed::embed_chunks(pool).await?;
                let out = match mode {
                    MaterializeMode::Full => {
                        write::materialize_cogmap(pool, loaded.cogmap, lens, loaded.emitter).await
                    }
                    MaterializeMode::Incremental => {
                        write::incremental_materialize_cogmap(
                            pool,
                            loaded.cogmap,
                            lens,
                            loaded.emitter,
                        )
                        .await
                    }
                }
                .with_context(|| format!("step {i}: materialize {lens}"))?;
                if let Some(prev) = current.insert(lens.clone(), out.membership_fingerprint) {
                    previous.insert(lens.clone(), prev);
                }
            }
            Step::Assert { checks } => {
                for c in checks {
                    eval_expectation(pool, &loaded, c, &current, &previous)
                        .await
                        .with_context(|| format!("step {i}: assertion failed"))?;
                }
            }
            mutation => {
                apply_mutation(pool, &mut loaded, mutation)
                    .await
                    .with_context(|| format!("step {i}: mutation failed"))?;
            }
        }
    }
    Ok(())
}

/// Every lens named in the seed's `uses_lenses` / the scenario's `steps` must exist (global or homed
/// in this cogmap) up front — a mistyped lens fails with a friendly error instead of an opaque
/// RowNotFound mid-run.
async fn validate_lenses(
    pool: &PgPool,
    loaded: &Loaded,
    seed: &Seed,
    steps: &[Step],
) -> Result<()> {
    let mut names: HashSet<&str> = seed.uses_lenses.iter().map(String::as_str).collect();
    for step in steps {
        match step {
            Step::Materialize { lens } => {
                names.insert(lens);
            }
            Step::Assert { checks } => {
                for c in checks {
                    for l in expectation_lenses(c) {
                        names.insert(l);
                    }
                }
            }
            _ => {}
        }
    }
    for name in names {
        let exists = sqlx::query_scalar!(
            "SELECT id FROM kb_cogmap_lenses WHERE name=$1 AND (cogmap_id=$2 OR cogmap_id IS NULL) LIMIT 1",
            name,
            loaded.cogmap,
        )
        .fetch_optional(pool)
        .await?;
        if exists.is_none() {
            bail!("scenario references undeclared lens {name:?} (not global, not homed in this cogmap)");
        }
    }
    Ok(())
}

fn expectation_lenses(e: &Expectation) -> Vec<&str> {
    match e {
        Expectation::RegionCount { lens, .. }
        | Expectation::CoRegion { lens, .. }
        | Expectation::CohesionOrder { lens, .. }
        | Expectation::RegionSize { lens, .. }
        | Expectation::InternalTension { lens, .. }
        | Expectation::Reproducible { lens }
        | Expectation::DriftTier { lens, .. } => vec![lens.as_str()],
        Expectation::FingerprintDiffers { lens_a, lens_b } => {
            vec![lens_a.as_str(), lens_b.as_str()]
        }
        Expectation::Stale { .. } => vec![],
    }
}

/// Resolve a runbook key to its resource UUID. A free function (not a closure over `loaded`) so the
/// create_resource arm can take `&mut loaded.keys` to insert without a borrow conflict.
fn lookup(keys: &HashMap<String, Uuid>, k: &str) -> Result<Uuid> {
    keys.get(k)
        .copied()
        .with_context(|| format!("mutation references unknown key {k}"))
}

/// Apply one mutation step (create_resource / set_facet / assert_edge / fold_edge) by firing the
/// matching SeedAction in its own transaction. create_resource registers the new key in `loaded.keys`.
async fn apply_mutation(pool: &PgPool, loaded: &mut Loaded, step: &Step) -> Result<()> {
    let mut tx = pool.begin().await?;
    match step {
        Step::CreateResource {
            key: rkey,
            title,
            origin_uri,
            doc_type,
            body,
            facets,
        } => {
            let display = title.clone().unwrap_or_else(|| rkey.clone());
            let blocks = crate::content::prepare_blocks(&[(None, body.as_str())])?;
            let rid = fire(
                &mut tx,
                SeedAction::ResourceCreate {
                    title: &display,
                    origin_uri,
                    home: CogmapId::from(loaded.cogmap),
                    owner: ProfileId::from(loaded.owner),
                    blocks: &blocks,
                    doc_type: doc_type.as_deref(),
                    emitter: EntityId::from(loaded.emitter),
                },
            )
            .await?
            .resource()?;
            if let Some(f) = facets {
                let values = serde_json::Value::Object(f.values().clone());
                fire(
                    &mut tx,
                    SeedAction::FacetSet {
                        resource: rid,
                        values: &values,
                        weight: f.weight(),
                        emitter: EntityId::from(loaded.emitter),
                    },
                )
                .await?;
            }
            tx.commit().await?;
            loaded.keys.insert(rkey.clone(), rid.uuid());
            return Ok(());
        }
        Step::SetFacet {
            resource,
            values,
            weight,
        } => {
            let rid = ResourceId::from(lookup(&loaded.keys, resource)?);
            let v = serde_json::Value::Object(values.clone());
            fire(
                &mut tx,
                SeedAction::FacetSet {
                    resource: rid,
                    values: &v,
                    weight: *weight,
                    emitter: EntityId::from(loaded.emitter),
                },
            )
            .await?;
        }
        Step::AssertEdge {
            from,
            to,
            kind,
            label,
            weight,
        } => {
            fire(
                &mut tx,
                SeedAction::RelationshipAssert {
                    src: ResourceId::from(lookup(&loaded.keys, from)?),
                    tgt: ResourceId::from(lookup(&loaded.keys, to)?),
                    kind: *kind,
                    label: label.as_deref(),
                    weight: *weight,
                    home: CogmapId::from(loaded.cogmap),
                    emitter: EntityId::from(loaded.emitter),
                },
            )
            .await?;
        }
        Step::FoldEdge {
            from,
            to,
            kind,
            reason,
        } => {
            let src = lookup(&loaded.keys, from)?;
            let tgt = lookup(&loaded.keys, to)?;
            // Runtime query (not a !-macro): the live-edge resolution + ambiguity guard is dynamic
            // intent (the per-crate macro-cache exception). query_scalar returns the id column directly.
            let edge_ids: Vec<Uuid> = sqlx::query_scalar(
                "SELECT id FROM kb_edges \
                 WHERE source_table='kb_resources' AND source_id=$1 \
                   AND target_table='kb_resources' AND target_id=$2 \
                   AND edge_kind=$3::edge_kind \
                   AND home_anchor_table='kb_cogmaps' AND home_anchor_id=$4 \
                   AND NOT is_folded",
            )
            .bind(src)
            .bind(tgt)
            .bind(kind.as_sql())
            .bind(loaded.cogmap)
            .fetch_all(&mut *tx)
            .await?;
            let edge_id = match edge_ids.as_slice() {
                [one] => *one,
                [] => bail!("fold_edge: no live edge {from}-[{kind:?}]->{to}"),
                _ => bail!("fold_edge: ambiguous — >1 live edge {from}-[{kind:?}]->{to}"),
            };
            fire(
                &mut tx,
                SeedAction::RelationshipFold {
                    edge: crate::ids::EdgeId::from(edge_id),
                    reason: reason.as_deref(),
                    emitter: EntityId::from(loaded.emitter),
                },
            )
            .await?;
        }
        Step::Revise { resource, body } => {
            let rid = lookup(&loaded.keys, resource)?;
            // resolve the resource's single non-folded body block (concept resources have exactly one).
            let block_ids: Vec<Uuid> = sqlx::query_scalar(
                "SELECT id FROM kb_content_blocks WHERE resource_id=$1 AND NOT is_folded ORDER BY seq",
            )
            .bind(rid)
            .fetch_all(&mut *tx)
            .await?;
            let block_id = match block_ids.as_slice() {
                [one] => *one,
                [] => bail!("revise: resource {resource} has no live block"),
                _ => bail!(
                    "revise: resource {resource} has >1 block (multi-block revise unsupported)"
                ),
            };
            // re-chunk + re-embed the new body inline (payload-first, like create_resource).
            let prepared = crate::content::prepare_block(0, None, body)?;
            fire(
                &mut tx,
                SeedAction::BlockMutate {
                    block: crate::ids::BlockId::from(block_id),
                    chunks: &prepared.chunks,
                    emitter: EntityId::from(loaded.emitter),
                },
            )
            .await?;
        }
        Step::Materialize { .. } | Step::Assert { .. } => {
            unreachable!("materialize/assert handled in run_scenario")
        }
    }
    tx.commit().await?;
    Ok(())
}

/// region containing `member_id` under `lens` (None if the member is in no live region).
async fn region_of(pool: &PgPool, cogmap: Uuid, lens: &str, member: Uuid) -> Result<Option<Uuid>> {
    Ok(sqlx::query_scalar!(
        "SELECT m.region_id FROM kb_cogmap_region_members m \
         JOIN kb_cogmap_regions r ON r.id=m.region_id AND NOT r.is_folded \
         JOIN kb_cogmap_lenses  l ON l.id=r.lens_id AND l.name=$2 \
         WHERE r.cogmap_id=$1 AND m.member_id=$3",
        cogmap,
        lens,
        member,
    )
    .fetch_optional(pool)
    .await?)
}

async fn eval_expectation(
    pool: &PgPool,
    loaded: &Loaded,
    e: &Expectation,
    current: &HashMap<String, String>,
    previous: &HashMap<String, String>,
) -> Result<()> {
    let key = |k: &str| -> Result<Uuid> {
        loaded
            .keys
            .get(k)
            .copied()
            .with_context(|| format!("expectation references unknown member key {k}"))
    };
    match e {
        Expectation::RegionCount { lens, op, value } => {
            let n = sqlx::query_scalar!(
                "SELECT count(*) FROM kb_cogmap_regions r JOIN kb_cogmap_lenses l ON l.id=r.lens_id \
                 WHERE r.cogmap_id=$1 AND l.name=$2 AND NOT r.is_folded",
                loaded.cogmap,
                lens,
            )
            .fetch_one(pool)
            .await?
            .unwrap_or(0);
            if !op.cmp_f64(n as f64, *value as f64) {
                bail!("region_count {n} fails {op:?} {value} (lens {lens})");
            }
        }
        Expectation::CoRegion {
            lens,
            members,
            expect,
        } => {
            let mut regions = Vec::new();
            for m in members {
                regions.push(region_of(pool, loaded.cogmap, lens, key(m)?).await?);
            }
            let all_same = regions.windows(2).all(|w| w[0].is_some() && w[0] == w[1]);
            if all_same != *expect {
                bail!("co_region {members:?} expected {expect}, got regions {regions:?} (lens {lens})");
            }
        }
        Expectation::RegionSize {
            lens,
            member,
            value,
        } => {
            let region = region_of(pool, loaded.cogmap, lens, key(member)?)
                .await?
                .with_context(|| {
                    format!("region_size: {member} is in no live region (lens {lens})")
                })?;
            let n = sqlx::query_scalar!(
                "SELECT count(*) FROM kb_cogmap_region_members WHERE region_id=$1",
                region,
            )
            .fetch_one(pool)
            .await?
            .unwrap_or(0);
            if n != *value {
                bail!("region_size of {member} = {n}, expected {value} (lens {lens})");
            }
        }
        Expectation::CohesionOrder {
            lens,
            greater,
            lesser,
        } => {
            let rg = region_of(pool, loaded.cogmap, lens, key(greater)?)
                .await?
                .with_context(|| format!("cohesion_order: {greater} has no region"))?;
            let rl = region_of(pool, loaded.cogmap, lens, key(lesser)?)
                .await?
                .with_context(|| format!("cohesion_order: {lesser} has no region"))?;
            let cg = sqlx::query_scalar!(
                "SELECT content_cohesion FROM kb_cogmap_regions WHERE id=$1",
                rg
            )
            .fetch_one(pool)
            .await?
            .context("cohesion_order: greater region has null content_cohesion")?;
            let cl = sqlx::query_scalar!(
                "SELECT content_cohesion FROM kb_cogmap_regions WHERE id=$1",
                rl
            )
            .fetch_one(pool)
            .await?
            .context("cohesion_order: lesser region has null content_cohesion")?;
            if cg <= cl {
                bail!("cohesion_order: {greater}({cg}) not > {lesser}({cl}) (lens {lens})");
            }
        }
        Expectation::InternalTension {
            lens,
            member,
            op,
            value,
        } => {
            let region = region_of(pool, loaded.cogmap, lens, key(member)?)
                .await?
                .with_context(|| format!("internal_tension: {member} has no region"))?;
            let t = sqlx::query_scalar!(
                "SELECT internal_tension FROM kb_cogmap_regions WHERE id=$1",
                region
            )
            .fetch_one(pool)
            .await?
            .context("internal_tension: null")?;
            if !op.cmp_f64(t, *value) {
                bail!("internal_tension of {member} = {t} fails {op:?} {value} (lens {lens})");
            }
        }
        Expectation::Reproducible { lens } => {
            let now = current
                .get(lens)
                .context("reproducible: lens never materialized")?;
            let before = previous
                .get(lens)
                .context("reproducible: lens materialized only once (need two materializes)")?;
            if now != before {
                bail!("reproducible: lens {lens} fingerprints differ ({before} vs {now})");
            }
        }
        Expectation::FingerprintDiffers { lens_a, lens_b } => {
            let a = current
                .get(lens_a)
                .context("fingerprint_differs: lens_a not materialized")?;
            let b = current
                .get(lens_b)
                .context("fingerprint_differs: lens_b not materialized")?;
            if a == b {
                bail!("fingerprint_differs: {lens_a} and {lens_b} are identical");
            }
        }
        Expectation::Stale { expect } => {
            let is_stale =
                sqlx::query_scalar!("SELECT is_stale FROM cogmap_staleness($1)", loaded.cogmap)
                    .fetch_one(pool)
                    .await?
                    .context("cogmap_staleness returned null")?;
            if is_stale != *expect {
                bail!("stale = {is_stale}, expected {expect}");
            }
        }
        Expectation::DriftTier { lens, tier } => {
            let (got, _diff) = crate::drift::lens_drift(pool, loaded.cogmap, lens).await?;
            let want = match tier {
                DriftTierName::Fresh => crate::drift::DriftTier::Fresh,
                DriftTierName::Readout => crate::drift::DriftTier::Readout,
                DriftTierName::Structural => crate::drift::DriftTier::Structural,
            };
            if got != want {
                bail!("drift_tier: expected {want:?}, got {got:?} (lens {lens})");
            }
        }
    }
    Ok(())
}
