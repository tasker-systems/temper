use crate::events::{fire, SeedAction};
use crate::ids::{CogmapId, EntityId, EventId, LensId, RegionId};
use crate::{
    affinity::{affinity, Lens},
    cluster::{agglomerate, connected_components},
    drift,
    fingerprint::component_fingerprint,
    substrate::{self, Substrate},
};
use anyhow::{Context, Result};
use sqlx::{PgConnection, PgPool};
use std::collections::HashSet;
use uuid::Uuid;

pub struct MaterializeOutcome {
    pub regions: usize,
    pub membership_fingerprint: String,
}

/// One connected component's worth of work: its node set (sorted — the component identity), the
/// fingerprint of its membership-determining inputs, and the regions agglomeration produced for it.
struct ComponentWork {
    members: Vec<Uuid>,
    fingerprint: String,
    clusters: Vec<Vec<Uuid>>,
}

/// Decompose the substrate into nonzero-affinity components and agglomerate each (drift §3.2). Region
/// formation is component-local, so this is byte-identical to clustering the whole node set — but it
/// also yields the per-component fingerprints incremental materialization compares against.
fn cluster_components(s: &Substrate) -> Vec<ComponentWork> {
    let aff = |x: Uuid, y: Uuid| affinity(x, y, &s.edges, &s.facets, &s.lens);
    connected_components(&s.nodes, &aff)
        .into_iter()
        .map(|members| {
            let clusters = agglomerate(&members, &aff, s.lens.resolution);
            let fingerprint = component_fingerprint(&members, &s.edges, &s.facets, &s.lens);
            ComponentWork {
                members,
                fingerprint,
                clusters,
            }
        })
        .collect()
}

/// The membership fingerprint over a flat region set: each region's members sorted and joined, regions
/// sorted among themselves. UUID-based ⇒ comparable within one instantiation (the `reproducible` /
/// `fingerprint_differs` scenario checks); identical inputs ⇒ identical string. Full and incremental
/// materialize compute it the same way over the same current clustering, so they agree by construction.
fn membership_fingerprint(clusters: &[Vec<Uuid>]) -> String {
    let mut parts: Vec<String> = clusters
        .iter()
        .map(|members| {
            let mut ms: Vec<String> = members.iter().map(|m| m.to_string()).collect();
            ms.sort();
            ms.join("+")
        })
        .collect();
    parts.sort();
    parts.join("|")
}

/// A zero-vector(768) literal placeholder for the NOT-NULL centroid (overwritten by the centroid
/// UPDATE in [`assert_region`] before any readout reads it). An unconditional zero literal — NOT a
/// fragile `SELECT centroid … LIMIT 1`, which would be NULL on a clean run, violating NOT NULL.
fn zero_centroid() -> String {
    format!(
        "[{}]",
        vec!["0"; temper_ingest::embed::EMBEDDING_DIM].join(",")
    )
}

/// Job B (drift §3.2/§4, spec §6a): read substrate -> nonzero-affinity components -> deterministic
/// per-component clustering -> persist components + fold prior regions/components + assert new regions
/// and members + readouts, under ONE materialization event. A FULL pass: every prior region and
/// component for the lens is folded and recomputed. Cosine never enters formation; it enters only via
/// the readouts. The persisted per-component fingerprints are what [`incremental_materialize_cogmap`]
/// reuses on the next pass.
pub async fn materialize_cogmap(
    pool: &PgPool,
    cogmap: Uuid,
    lens_name: &str,
    emitter: Uuid,
) -> Result<MaterializeOutcome> {
    let s = substrate::load(pool, cogmap, lens_name).await?;
    let comps = cluster_components(&s);

    // fingerprint + region ids BEFORE the event (payload-first): the region_materialized payload
    // records the act's full identity — lens, watermark, membership fingerprint, region ids.
    let all_clusters: Vec<Vec<Uuid>> = comps.iter().flat_map(|c| c.clusters.clone()).collect();
    let fingerprint = membership_fingerprint(&all_clusters);
    // region ids grouped per component (aligned with each ComponentWork.clusters), plus a flat list
    // for the event payload.
    let region_ids: Vec<Vec<RegionId>> = comps
        .iter()
        .map(|c| {
            c.clusters
                .iter()
                .map(|_| RegionId::from(Uuid::now_v7()))
                .collect()
        })
        .collect();
    let flat_region_ids: Vec<RegionId> = region_ids.iter().flatten().copied().collect();

    let mut tx = pool.begin().await?;
    let watermark = current_watermark(&mut tx).await?;
    let ev = fire(
        &mut tx,
        SeedAction::Materialize {
            cogmap: CogmapId::from(cogmap),
            lens: LensId::from(s.lens_id),
            watermark: EventId::from(watermark),
            membership_fingerprint: &fingerprint,
            region_ids: &flat_region_ids,
            emitter: EntityId::from(emitter),
        },
    )
    .await?
    .materialize_event()?
    .uuid();

    // a full pass folds every prior live region AND component for this lens, then recreates them.
    fold_live_regions(&mut tx, cogmap, s.lens_id, ev).await?;
    fold_live_components(&mut tx, cogmap, s.lens_id, ev).await?;

    let zero = zero_centroid();
    for (comp, comp_region_ids) in comps.iter().zip(&region_ids) {
        let comp_id = create_component(&mut tx, cogmap, s.lens_id, comp, ev).await?;
        for (members, region_id) in comp.clusters.iter().zip(comp_region_ids) {
            assert_region(
                &mut tx,
                cogmap,
                s.lens_id,
                comp_id,
                members,
                region_id.uuid(),
                ev,
                &s.lens,
                &zero,
            )
            .await?;
        }
    }
    // (the materialization watermark on kb_cogmaps is set by _project_region_materialized — the
    // event's projection half — not here.)
    tx.commit().await?;

    Ok(MaterializeOutcome {
        regions: all_clusters.len(),
        membership_fingerprint: fingerprint,
    })
}

/// Incremental materialization (drift §4): re-cluster only the components whose inputs changed; reuse
/// every component whose (member set, fingerprint) still matches a live persisted component untouched.
/// Provably byte-identical to a full re-materialize at the same watermark — region formation is
/// component-local, so a reused component's regions equal what a full recompute would produce, and the
/// changed components are recomputed by the same `agglomerate`. The returned membership fingerprint is
/// over the FULL current clustering, so the `reproducible` / `fingerprint_differs` checks behave
/// exactly as under a full pass. Self-bootstraps to a full pass when no prior components exist.
pub async fn incremental_materialize_cogmap(
    pool: &PgPool,
    cogmap: Uuid,
    lens_name: &str,
    emitter: Uuid,
) -> Result<MaterializeOutcome> {
    let s = substrate::load(pool, cogmap, lens_name).await?;
    let comps = cluster_components(&s);

    let priors = drift::live_components(pool, cogmap, s.lens_id).await?;
    if priors.is_empty() {
        // nothing to diff against — the first materialize for this lens is a full pass.
        return materialize_cogmap(pool, cogmap, lens_name, emitter).await;
    }

    // the same fingerprint comparison drift detection uses: which components are reused untouched,
    // which member-sets must be re-clustered, which priors are stale. Member-sets are unique within
    // the partition, so map the changed member-sets back to their `ComponentWork` (for the clusters).
    let current_fps: Vec<(Vec<Uuid>, String)> = comps
        .iter()
        .map(|c| (c.members.clone(), c.fingerprint.clone()))
        .collect();
    let diff = drift::classify(&current_fps, &priors);
    let changed_keys: HashSet<&Vec<Uuid>> = diff.changed.iter().collect();
    let changed: Vec<&ComponentWork> = comps
        .iter()
        .filter(|c| changed_keys.contains(&c.members))
        .collect();
    let stale_prior: Vec<Uuid> = diff.stale.clone();

    // membership fingerprint + region count are over the FULL current clustering (reused + changed),
    // identical to what a full pass at this watermark computes.
    let all_clusters: Vec<Vec<Uuid>> = comps.iter().flat_map(|c| c.clusters.clone()).collect();
    let fingerprint = membership_fingerprint(&all_clusters);
    // only the CHANGED components mint new regions this act; reused regions keep their prior ids.
    let new_region_ids: Vec<Vec<RegionId>> = changed
        .iter()
        .map(|c| {
            c.clusters
                .iter()
                .map(|_| RegionId::from(Uuid::now_v7()))
                .collect()
        })
        .collect();
    let flat_new_region_ids: Vec<RegionId> = new_region_ids.iter().flatten().copied().collect();

    let mut tx = pool.begin().await?;
    let watermark = current_watermark(&mut tx).await?;
    // region_ids records the regions THIS act asserted (the changed components' new regions); the full
    // membership fingerprint records the complete resulting shape (reused regions included).
    let ev = fire(
        &mut tx,
        SeedAction::Materialize {
            cogmap: CogmapId::from(cogmap),
            lens: LensId::from(s.lens_id),
            watermark: EventId::from(watermark),
            membership_fingerprint: &fingerprint,
            region_ids: &flat_new_region_ids,
            emitter: EntityId::from(emitter),
        },
    )
    .await?
    .materialize_event()?
    .uuid();

    // fold the stale components and their regions; leave matched components + their regions live.
    fold_components(&mut tx, &stale_prior, ev).await?;

    let zero = zero_centroid();
    for (comp, comp_region_ids) in changed.iter().zip(&new_region_ids) {
        let comp_id = create_component(&mut tx, cogmap, s.lens_id, comp, ev).await?;
        for (members, region_id) in comp.clusters.iter().zip(comp_region_ids) {
            assert_region(
                &mut tx,
                cogmap,
                s.lens_id,
                comp_id,
                members,
                region_id.uuid(),
                ev,
                &s.lens,
                &zero,
            )
            .await?;
        }
    }

    // Readout-refresh (drift §1, slice 3b): reused components keep their membership AND their region
    // ids, but a content revision since the prior materialize moved a member's embedding — so their
    // stored readouts are stale. Re-run the readouts over the reused regions' fixed membership (no
    // re-cluster, no new region ids) so incremental matches a full recompute. Gated on a CONTENT touch
    // so a purely-structural pass does no redundant readout work on the reused side. `priors` is
    // non-empty here (the empty case returned early to a full pass), so a prior materialize exists.
    let prior_watermark = last_materialize_watermark(&mut tx, cogmap, s.lens_id).await?;
    let content_touched = match prior_watermark {
        Some(w) => crate::replay::content_touched_since(pool, cogmap, w).await?,
        None => false,
    };
    if content_touched {
        for prior_id in &diff.unchanged {
            let region_ids: Vec<Uuid> = sqlx::query_scalar(
                "SELECT id FROM kb_cogmap_regions WHERE component_id=$1 AND NOT is_folded",
            )
            .bind(*prior_id)
            .fetch_all(&mut *tx)
            .await?;
            for rid in region_ids {
                populate_readouts(&mut tx, rid, &s.lens, &zero).await?;
                sqlx::query("UPDATE kb_cogmap_regions SET last_event_id=$1 WHERE id=$2")
                    .bind(ev)
                    .bind(rid)
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }

    tx.commit().await?;

    Ok(MaterializeOutcome {
        regions: all_clusters.len(),
        membership_fingerprint: fingerprint,
    })
}

/// The substrate point-in-time this projection saw (uuidv7 — time-ordered; no max(uuid) in PG). The
/// emitter is passed explicitly by the caller — never derived from "latest event", which is NULL on an
/// empty log and arbitrary on occurred_at ties.
async fn current_watermark(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> Result<Uuid> {
    sqlx::query_scalar!("SELECT id FROM kb_events ORDER BY id DESC LIMIT 1")
        .fetch_optional(&mut **tx)
        .await?
        .context("materialize on an empty ledger (no events)")
}

/// The event id of the most recent region_materialized act for (cogmap, lens) BEFORE this transaction's
/// own act — the point-in-time the reused regions' readouts were last computed against. This pass's act
/// is already appended (the latest), so `OFFSET 1` skips it to the prior projection. `None` only if this
/// is the very first materialize (no prior), where incremental never reaches the readout-refresh path.
async fn last_materialize_watermark(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    cogmap: Uuid,
    lens_id: Uuid,
) -> Result<Option<Uuid>> {
    Ok(sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
         WHERE et.name='region_materialized' \
           AND (e.payload->>'cogmap_id')::uuid=$1 AND (e.payload->>'lens_id')::uuid=$2 \
         ORDER BY e.id DESC OFFSET 1 LIMIT 1",
    )
    .bind(cogmap)
    .bind(lens_id)
    .fetch_optional(&mut **tx)
    .await?)
}

async fn fold_live_regions(
    tx: &mut PgConnection,
    cogmap: Uuid,
    lens_id: Uuid,
    ev: Uuid,
) -> Result<()> {
    sqlx::query(
        "UPDATE kb_cogmap_regions SET is_folded=true, last_event_id=$1 \
         WHERE cogmap_id=$2 AND lens_id=$3 AND NOT is_folded",
    )
    .bind(ev)
    .bind(cogmap)
    .bind(lens_id)
    .execute(tx)
    .await?;
    Ok(())
}

async fn fold_live_components(
    tx: &mut PgConnection,
    cogmap: Uuid,
    lens_id: Uuid,
    ev: Uuid,
) -> Result<()> {
    sqlx::query(
        "UPDATE kb_cogmap_components SET is_folded=true, last_event_id=$1 \
         WHERE cogmap_id=$2 AND lens_id=$3 AND NOT is_folded",
    )
    .bind(ev)
    .bind(cogmap)
    .bind(lens_id)
    .execute(tx)
    .await?;
    Ok(())
}

/// Fold specific components by id AND their live regions (incremental's stale path).
async fn fold_components(tx: &mut PgConnection, component_ids: &[Uuid], ev: Uuid) -> Result<()> {
    if component_ids.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "UPDATE kb_cogmap_regions SET is_folded=true, last_event_id=$1 \
         WHERE component_id = ANY($2) AND NOT is_folded",
    )
    .bind(ev)
    .bind(component_ids)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE kb_cogmap_components SET is_folded=true, last_event_id=$1 WHERE id = ANY($2)",
    )
    .bind(ev)
    .bind(component_ids)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Persist one component row (its fingerprint + member set), returning its id for the regions to link.
async fn create_component(
    tx: &mut PgConnection,
    cogmap: Uuid,
    lens_id: Uuid,
    comp: &ComponentWork,
    ev: Uuid,
) -> Result<Uuid> {
    use sqlx::Row;
    let row = sqlx::query(
        "INSERT INTO kb_cogmap_components \
           (cogmap_id, lens_id, fingerprint, member_ids, asserted_by_event_id, last_event_id) \
         VALUES ($1,$2,$3,$4,$5,$5) RETURNING id",
    )
    .bind(cogmap)
    .bind(lens_id)
    .bind(&comp.fingerprint)
    .bind(&comp.members)
    .bind(ev)
    .fetch_one(&mut *tx)
    .await?;
    Ok(row.get::<Uuid, _>("id"))
}

/// Insert one region (linked to its component), its members, then populate the SQL readouts. The
/// region id is pre-generated (identity-as-input) and already recorded in the payload.
#[expect(clippy::too_many_arguments)]
async fn assert_region(
    tx: &mut PgConnection,
    cogmap: Uuid,
    lens_id: Uuid,
    component_id: Uuid,
    members: &[Uuid],
    region_id: Uuid,
    ev: Uuid,
    lens: &Lens,
    zero_centroid: &str,
) -> Result<()> {
    use sqlx::Row;
    // centroid computed in SQL after members are inserted; insert a placeholder then UPDATE.
    let region: Uuid = sqlx::query(
        "INSERT INTO kb_cogmap_regions \
           (id, cogmap_id, lens_id, component_id, centroid, salience, label, member_count, asserted_by_event_id, last_event_id) \
         VALUES ($6, $1,$2,$7, $5::vector, 0.0, NULL, $3, $4, $4) RETURNING id",
    )
    .bind(cogmap)
    .bind(lens_id)
    .bind(members.len() as i32)
    .bind(ev)
    .bind(zero_centroid)
    .bind(region_id)
    .bind(component_id)
    .fetch_one(&mut *tx)
    .await?
    .get::<Uuid, _>("id");
    for m in members {
        sqlx::query(
            "INSERT INTO kb_cogmap_region_members (region_id, member_table, member_id) \
             VALUES ($1,'kb_resources',$2)",
        )
        .bind(region)
        .bind(m)
        .execute(&mut *tx)
        .await?;
    }
    populate_readouts(tx, region, lens, zero_centroid).await
}

/// Re-derive a region's SQL readouts over its CURRENT members + embeddings: centroid (mean of
/// per-member pooled chunk vectors), then content_cohesion / telos_alignment / reference_standing /
/// centrality / internal_tension, then lens-weighted salience. Idempotent over fixed membership — the
/// readout-refresh tier (drift §1) calls this on reused components whose content moved; `assert_region`
/// calls it on a freshly-asserted region. Membership must already be inserted.
async fn populate_readouts(
    tx: &mut PgConnection,
    region: Uuid,
    lens: &Lens,
    zero_centroid: &str,
) -> Result<()> {
    // Centroid FIRST, in its own statement — Postgres evaluates all SET right-hand sides against the
    // OLD row, so the telos_alignment readout (which SELECTs the stored centroid) must run in a LATER
    // statement or it reads the zero placeholder → cosine-vs-zero = NaN → NaN salience. Pool per
    // concept (avg per member, then mean of members) to match cogmap_region_content_cohesion (OQ-1);
    // exclude folded blocks so the vector projection agrees with embed + body-text; coalesce a
    // memberless/unembedded region to the zero placeholder so centroid stays NOT NULL.
    sqlx::query(
        "UPDATE kb_cogmap_regions r SET centroid = coalesce(( \
           SELECT avg(mv) FROM ( \
             SELECT avg(ch.embedding) AS mv FROM kb_cogmap_region_members mm \
             JOIN kb_chunks ch ON ch.resource_id=mm.member_id AND ch.is_current \
             JOIN kb_content_blocks b ON b.id=ch.block_id AND NOT b.is_folded \
             WHERE mm.region_id=r.id GROUP BY mm.member_id) per_member \
         ), $2::vector) WHERE r.id=$1",
    )
    .bind(region)
    .bind(zero_centroid)
    .execute(&mut *tx)
    .await?;
    // Readouts now read the correct stored centroid. nullif guards the zero-centroid edge (a
    // memberless/unembedded region → cosine-vs-zero = NaN) so telos_alignment stores NULL, not NaN;
    // the salience UPDATE below already coalesces NULL telos_alignment to 0.
    sqlx::query(
        "UPDATE kb_cogmap_regions r SET \
           content_cohesion   = cogmap_region_content_cohesion(r.id), \
           telos_alignment    = nullif(cogmap_region_telos_alignment(r.id, r.cogmap_id), 'NaN'::double precision), \
           reference_standing = cogmap_region_reference_standing(r.id), \
           centrality         = cogmap_region_centrality(r.id), \
           internal_tension   = cogmap_region_internal_tension(r.id, ARRAY['contradicts']) \
         WHERE r.id=$1",
    )
    .bind(region)
    .execute(&mut *tx)
    .await?;
    // salience = lens-weighted blend of the three parts. telos_alignment is NULLABLE (NULL when the
    // telos has no embedded chunks) and salience is NOT NULL — so `$2*NULL = NULL` would violate the
    // constraint. coalesce to 0. (reference_standing/centrality coalesce to 0 inside their SQL fns.)
    sqlx::query(
        "UPDATE kb_cogmap_regions SET salience = \
           $2*coalesce(telos_alignment,0) + $3*reference_standing + $4*centrality WHERE id=$1",
    )
    .bind(region)
    .bind(lens.s_telos)
    .bind(lens.s_ref)
    .bind(lens.s_central)
    .execute(&mut *tx)
    .await?;
    Ok(())
}
