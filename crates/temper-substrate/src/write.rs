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

#[derive(Debug)]
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
    let comp_refs: Vec<&ComponentWork> = comps.iter().collect();

    // fingerprint + region ids BEFORE the event (payload-first): the region_materialized payload
    // records the act's full identity — lens, watermark, membership fingerprint, region ids. Region
    // ids are grouped per component (aligned with each ComponentWork.clusters), plus a flat list.
    let all_clusters: Vec<Vec<Uuid>> = comps.iter().flat_map(|c| c.clusters.clone()).collect();
    let fingerprint = membership_fingerprint(&all_clusters);
    let region_ids = mint_region_ids(&comp_refs);
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
    let work: Vec<(&ComponentWork, &Vec<RegionId>)> =
        comp_refs.iter().copied().zip(&region_ids).collect();
    assert_component_regions(&mut tx, cogmap, s.lens_id, &s.lens, &zero, ev, &work).await?;
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
    let changed = changed_components(&comps, &diff);

    // membership fingerprint + region count are over the FULL current clustering (reused + changed),
    // identical to what a full pass at this watermark computes. Only the CHANGED components mint new
    // regions this act; reused regions keep their prior ids.
    let all_clusters: Vec<Vec<Uuid>> = comps.iter().flat_map(|c| c.clusters.clone()).collect();
    let fingerprint = membership_fingerprint(&all_clusters);
    let new_region_ids = mint_region_ids(&changed);
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
    fold_components(&mut tx, &diff.stale, ev).await?;

    let zero = zero_centroid();
    let work: Vec<(&ComponentWork, &Vec<RegionId>)> =
        changed.iter().copied().zip(&new_region_ids).collect();
    assert_component_regions(&mut tx, cogmap, s.lens_id, &s.lens, &zero, ev, &work).await?;

    refresh_moved_region_readouts(pool, &mut tx, cogmap, &s, &zero, &diff.unchanged, ev).await?;

    tx.commit().await?;

    Ok(MaterializeOutcome {
        regions: all_clusters.len(),
        membership_fingerprint: fingerprint,
    })
}

/// Map the diff's changed member-sets back to their `ComponentWork` (for the clusters). Member-sets
/// are unique within the partition, so the lookup is unambiguous.
fn changed_components<'a>(
    comps: &'a [ComponentWork],
    diff: &drift::ComponentDiff,
) -> Vec<&'a ComponentWork> {
    let changed_keys: HashSet<&Vec<Uuid>> = diff.changed.iter().collect();
    comps
        .iter()
        .filter(|c| changed_keys.contains(&c.members))
        .collect()
}

/// Pre-generate a fresh region id (identity-as-input) for every cluster of every given component,
/// grouped per component (aligned with each `ComponentWork.clusters`).
fn mint_region_ids(comps: &[&ComponentWork]) -> Vec<Vec<RegionId>> {
    comps
        .iter()
        .map(|c| {
            c.clusters
                .iter()
                .map(|_| RegionId::from(Uuid::now_v7()))
                .collect()
        })
        .collect()
}

/// Persist each component and assert its regions (members + readouts) in order, sharing the parent's
/// transaction. `work` pairs each component with its pre-minted per-cluster region ids.
async fn assert_component_regions(
    tx: &mut PgConnection,
    cogmap: Uuid,
    lens_id: Uuid,
    lens: &Lens,
    zero_centroid: &str,
    ev: Uuid,
    work: &[(&ComponentWork, &Vec<RegionId>)],
) -> Result<()> {
    for &(comp, comp_region_ids) in work {
        let comp_id = create_component(&mut *tx, cogmap, lens_id, comp, ev).await?;
        for (members, region_id) in comp.clusters.iter().zip(comp_region_ids) {
            assert_region(
                &mut *tx,
                AssertRegionCtx {
                    cogmap,
                    lens_id,
                    component_id: comp_id,
                    members,
                    region_id: region_id.uuid(),
                    ev,
                    lens,
                    zero_centroid,
                },
            )
            .await?;
        }
    }
    Ok(())
}

/// Readout-refresh (drift §1, slice 3b): reused components keep their membership AND their region
/// ids, but a content revision since the prior materialize moved a member's embedding — so a region
/// CONTAINING that member has stale readouts. Re-run the readouts over the moved region's fixed
/// membership (no re-cluster, no new region ids) so incremental matches a full recompute. Scoped to
/// the reused regions whose own members moved: a moved member shifts only its region's centroid, so
/// refreshing the others would re-introduce, one layer up, the over-trigger the per-component
/// decomposition removed — while still matching full (an untouched region's stored readouts already
/// equal a recompute). Only reached when `priors` is non-empty (the empty case is a full pass).
async fn refresh_moved_region_readouts(
    pool: &PgPool,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    cogmap: Uuid,
    s: &Substrate,
    zero_centroid: &str,
    unchanged: &[Uuid],
    ev: Uuid,
) -> Result<()> {
    let prior_watermark = last_materialize_watermark(tx, cogmap, s.lens_id, ev).await?;
    let touched_resources = match prior_watermark {
        Some(w) => crate::replay::content_touched_resources_since(pool, cogmap, w).await?,
        None => Vec::new(),
    };
    if !touched_resources.is_empty() {
        // one query for the reused regions that actually contain a moved member (not every reused
        // region, and not N per-component round-trips).
        let region_ids: Vec<Uuid> = sqlx::query_scalar(
            "SELECT DISTINCT r.id FROM kb_cogmap_regions r \
             JOIN kb_cogmap_region_members m ON m.region_id = r.id \
             WHERE r.component_id = ANY($1) AND NOT r.is_folded \
               AND m.member_table = 'kb_resources' AND m.member_id = ANY($2)",
        )
        .bind(unchanged)
        .bind(&touched_resources)
        .fetch_all(&mut **tx)
        .await?;
        for rid in &region_ids {
            populate_readouts(tx, *rid, &s.lens, zero_centroid).await?;
        }
        // one batched last_event_id stamp for every refreshed region (same `ev` for all).
        if !region_ids.is_empty() {
            sqlx::query("UPDATE kb_cogmap_regions SET last_event_id=$1 WHERE id = ANY($2)")
                .bind(ev)
                .bind(&region_ids)
                .execute(&mut **tx)
                .await?;
        }
    }
    Ok(())
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

/// The event id of the most recent region_materialized act for (cogmap, lens) BEFORE `current_ev`
/// (this pass's own act, already appended) — the point-in-time the reused regions' readouts were last
/// computed against. Excluding `current_ev` explicitly (`e.id < $3`) states the intent directly rather
/// than relying on "my own event is the single latest" (an `OFFSET 1` would silently return the wrong
/// watermark under a second act in the same txn or a concurrent materialize). `None` only if this is
/// the very first materialize, where incremental never reaches the readout-refresh path.
async fn last_materialize_watermark(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    cogmap: Uuid,
    lens_id: Uuid,
    current_ev: Uuid,
) -> Result<Option<Uuid>> {
    Ok(sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
         WHERE et.name='region_materialized' \
           AND (e.payload->>'cogmap_id')::uuid=$1 AND (e.payload->>'lens_id')::uuid=$2 \
           AND e.id < $3 \
         ORDER BY e.id DESC LIMIT 1",
    )
    .bind(cogmap)
    .bind(lens_id)
    .bind(current_ev)
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

/// Parameters for [`assert_region`]. The region id is pre-generated (identity-as-input) and already
/// recorded in the materialization payload before this is called.
struct AssertRegionCtx<'a> {
    cogmap: Uuid,
    lens_id: Uuid,
    component_id: Uuid,
    members: &'a [Uuid],
    region_id: Uuid,
    ev: Uuid,
    lens: &'a Lens,
    zero_centroid: &'a str,
}

/// Insert one region (linked to its component), its members, then populate the SQL readouts.
async fn assert_region(tx: &mut PgConnection, ctx: AssertRegionCtx<'_>) -> Result<()> {
    use sqlx::Row;
    let AssertRegionCtx {
        cogmap,
        lens_id,
        component_id,
        members,
        region_id,
        ev,
        lens,
        zero_centroid,
    } = ctx;
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
