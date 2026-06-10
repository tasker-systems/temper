//! Scenario runner: resolves the scenario's seed (referenced or embedded), loads its substrate
//! through the same `loader::load_seed` path a standalone seed uses, then executes the ordered
//! `steps` runbook in-process (materialize / emit-event / assert). Materialize reuses
//! `embed_chunks` and `materialize_cogmap`; emit-event calls the reusable `relationship_assert`
//! function; assert evaluates each expectation against the materialized regions. A per-lens
//! fingerprint cache backs the `reproducible` / `fingerprint_differs` checks. Any failed
//! expectation aborts with a descriptive error.

use crate::events::{fire, SeedAction};
use crate::ids::{CogmapId, EntityId};
use crate::scenario::loader::{self, Loaded};
use crate::scenario::model::*;
use crate::{embed, write};
use anyhow::{bail, Context, Result};
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use uuid::Uuid;

/// `base_dir` anchors a `seed:` path reference — pass the scenario file's directory.
pub async fn run_scenario(pool: &PgPool, s: &Scenario, base_dir: &Path) -> Result<()> {
    let seed = s.resolve_seed(base_dir)?;
    let loaded = loader::load_seed(pool, &seed).await?;
    validate_lenses(pool, &loaded, &seed, &s.steps).await?;

    // per-lens fingerprints: `current` is the latest materialize; `previous` is the one before it.
    let mut current: HashMap<String, String> = HashMap::new();
    let mut previous: HashMap<String, String> = HashMap::new();

    for (i, step) in s.steps.iter().enumerate() {
        match step {
            Step::Materialize { lens } => {
                embed::embed_chunks(pool).await?;
                let out = write::materialize_cogmap(pool, loaded.cogmap, lens, loaded.emitter)
                    .await
                    .with_context(|| format!("step {i}: materialize {lens}"))?;
                if let Some(prev) = current.insert(lens.clone(), out.membership_fingerprint) {
                    previous.insert(lens.clone(), prev);
                }
            }
            Step::EmitEvent { event_type, edges } => {
                emit_event(pool, &loaded, edges)
                    .await
                    .with_context(|| format!("step {i}: emit_event {event_type}"))?;
            }
            Step::Assert { checks } => {
                for c in checks {
                    eval_expectation(pool, &loaded, c, &current, &previous)
                        .await
                        .with_context(|| format!("step {i}: assertion failed"))?;
                }
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
            Step::EmitEvent { .. } => {}
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
        | Expectation::Reproducible { lens } => vec![lens.as_str()],
        Expectation::FingerprintDiffers { lens_a, lens_b } => {
            vec![lens_a.as_str(), lens_b.as_str()]
        }
        Expectation::Stale { .. } => vec![],
    }
}

async fn emit_event(pool: &PgPool, loaded: &Loaded, edges: &[EdgeDef]) -> Result<()> {
    let mut tx = pool.begin().await?;
    for e in edges {
        let src = (*loaded
            .keys
            .get(&e.from)
            .with_context(|| format!("emit_event edge from unknown key {}", e.from))?)
        .into();
        let tgt = (*loaded
            .keys
            .get(&e.to)
            .with_context(|| format!("emit_event edge to unknown key {}", e.to))?)
        .into();
        fire(
            &mut tx,
            SeedAction::RelationshipAssert {
                src,
                tgt,
                kind: e.kind,
                label: e.label.as_deref(),
                weight: e.weight,
                home: CogmapId::from(loaded.cogmap),
                emitter: EntityId::from(loaded.emitter),
            },
        )
        .await?;
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
    }
    Ok(())
}
