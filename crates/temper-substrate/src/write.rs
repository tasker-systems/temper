use crate::events::{fire, SeedAction};
use crate::ids::{EntityId, EventId, LensId, RegionId, ResourceId};
use crate::{
    affinity::{affinity, Lens},
    cluster::{agglomerate, connected_components},
    drift,
    fingerprint::component_fingerprint,
    substrate::{self, Substrate},
};
use anyhow::{Context, Result};
use sqlx::{PgConnection, PgPool};
use std::collections::{HashMap, HashSet};
use temper_core::types::home::HomeAnchor;
use uuid::Uuid;

#[derive(Debug)]
pub struct MaterializeOutcome {
    pub regions: usize,
    pub membership_fingerprint: String,
    /// Of the regions this act (re-)asserted, how many kept their prior id because their member set was
    /// unchanged, vs. how many were minted fresh. A full pass re-asserts every region, so these sum to
    /// `regions`. An incremental pass only re-asserts the CHANGED components' regions, so they sum to
    /// fewer — a reused component's regions are not re-asserted at all, and trivially keep their ids.
    pub regions_reused: usize,
    pub regions_minted: usize,
}

/// The region id a cluster will be persisted under: the id of the live region whose member set is
/// exactly this cluster's (reused — nothing about the region changed, so neither should its identity),
/// or a freshly minted one.
#[derive(Clone, Copy, Debug)]
enum RegionAssignment {
    Reuse(RegionId),
    Mint(RegionId),
}

impl RegionAssignment {
    fn region_id(self) -> RegionId {
        match self {
            RegionAssignment::Reuse(id) | RegionAssignment::Mint(id) => id,
        }
    }
}

/// The ids of the reused regions — the fold's `keep` list. A reused region is never folded: `id` is the
/// primary key and the fold is SOFT (the row survives), so folding it and re-inserting the same id would
/// be a duplicate-key violation. Reuse is survive-and-refresh, not fold-and-recreate.
fn reused_ids(assignments: &[Vec<RegionAssignment>]) -> Vec<Uuid> {
    assignments
        .iter()
        .flatten()
        .filter_map(|a| match a {
            RegionAssignment::Reuse(id) => Some(id.uuid()),
            RegionAssignment::Mint(_) => None,
        })
        .collect()
}

/// (reused, minted) over every assignment — the churn measurement `MaterializeOutcome` reports.
fn count_assignments(assignments: &[Vec<RegionAssignment>]) -> (usize, usize) {
    assignments
        .iter()
        .flatten()
        .fold((0, 0), |(r, m), a| match a {
            RegionAssignment::Reuse(_) => (r + 1, m),
            RegionAssignment::Mint(_) => (r, m + 1),
        })
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
    // `affinity` is typed on `ResourceId`; the pure clustering algorithm (`cluster`) works over opaque
    // `Uuid` nodes. Bridge at this one boundary: feed cluster the bare node uuids, lift each pair back
    // to `ResourceId` for the affinity lookup.
    let aff = |x: Uuid, y: Uuid| affinity(x.into(), y.into(), &s.edges, &s.facets, &s.knn, &s.lens);
    let node_uuids: Vec<Uuid> = s.nodes.iter().map(|n| n.uuid()).collect();
    connected_components(&node_uuids, &aff)
        .into_iter()
        .map(|members| {
            let clusters = agglomerate(&members, &aff, s.lens.resolution);
            let fingerprint = component_fingerprint(&members, &s.edges, &s.facets, &s.knn, &s.lens);
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
/// component for the lens is folded and recomputed. The persisted per-component fingerprints are what
/// [`incremental_materialize`] reuses on the next pass.
///
/// `anchor` is a context OR a cognitive map (spec §3.6 M2). The regime is the LENS's business, not
/// this function's. With `w_cos = 0` (`telos-default` and every other cogmap lens) formation is
/// declared-graph-only. With `w_cos > 0` (`workflow-default`, the context lens) the sparse exact-kNN
/// cosine term joins the kernel and the embedding becomes the primary evidence of regionality — which
/// is what lets a context, carrying no facets and a near-monotone edge graph, form anything at all.
pub async fn materialize(
    pool: &PgPool,
    anchor: HomeAnchor,
    lens_name: &str,
    emitter: EntityId,
) -> Result<MaterializeOutcome> {
    let s = substrate::load(pool, anchor, lens_name).await?;
    let comps = cluster_components(&s);
    let comp_refs: Vec<&ComponentWork> = comps.iter().collect();

    // fingerprint + region ids BEFORE the event (payload-first): the region_materialized payload
    // records the act's full identity — lens, watermark, membership fingerprint, region ids. Region
    // ids are grouped per component (aligned with each ComponentWork.clusters), plus a flat list.
    //
    // The payload's `region_ids` carries BOTH kinds of assignment (reused and minted), which keeps its
    // meaning exactly what it has always been: for a full pass, the complete resulting partition. Only
    // the freshness of the ids changes — never the set. (Nothing reconstructs regions from this field;
    // `_project_region_materialized` reads only the anchor pair, to stamp the watermark.)
    let all_clusters: Vec<Vec<Uuid>> = comps.iter().flat_map(|c| c.clusters.clone()).collect();
    let fingerprint = membership_fingerprint(&all_clusters);
    let live = live_regions(pool, anchor, s.lens_id).await?;
    let assignments = resolve_region_ids(&comp_refs, &live);
    let flat_region_ids: Vec<RegionId> = assignments
        .iter()
        .flatten()
        .map(|a| a.region_id())
        .collect();
    let keep = reused_ids(&assignments);
    let (regions_reused, regions_minted) = count_assignments(&assignments);

    let mut tx = pool.begin().await?;
    let watermark = current_watermark(&mut tx).await?;
    let ev = fire(
        &mut tx,
        SeedAction::Materialize {
            anchor,
            lens: s.lens_id,
            watermark: EventId::from(watermark),
            membership_fingerprint: &fingerprint,
            region_ids: &flat_region_ids,
            emitter,
        },
    )
    .await?
    .materialize_event()?;

    // a full pass folds every prior live region AND component for this lens, then recreates them —
    // EXCEPT the regions whose member set is unchanged, which survive under their own ids and are
    // refreshed in place.
    fold_live_regions(&mut tx, anchor, s.lens_id, ev, &keep).await?;
    fold_live_components(&mut tx, anchor, s.lens_id, ev).await?;

    let zero = zero_centroid();
    let work: Vec<(&ComponentWork, &Vec<RegionAssignment>)> =
        comp_refs.iter().copied().zip(&assignments).collect();
    assert_component_regions(&mut tx, anchor, &s, &zero, ev, &work).await?;
    // (the materialization watermark on the anchor row is set by _project_region_materialized — the
    // event's projection half — not here.)
    tx.commit().await?;

    Ok(MaterializeOutcome {
        regions: all_clusters.len(),
        membership_fingerprint: fingerprint,
        regions_reused,
        regions_minted,
    })
}

/// Incremental materialization (drift §4): re-cluster only the components whose inputs changed; reuse
/// every component whose (member set, fingerprint) still matches a live persisted component untouched.
/// Provably byte-identical to a full re-materialize at the same watermark — region formation is
/// component-local, so a reused component's regions equal what a full recompute would produce, and the
/// changed components are recomputed by the same `agglomerate`. The returned membership fingerprint is
/// over the FULL current clustering, so the `reproducible` / `fingerprint_differs` checks behave
/// exactly as under a full pass. Self-bootstraps to a full pass when no prior components exist.
pub async fn incremental_materialize(
    pool: &PgPool,
    anchor: HomeAnchor,
    lens_name: &str,
    emitter: EntityId,
) -> Result<MaterializeOutcome> {
    let s = substrate::load(pool, anchor, lens_name).await?;
    let comps = cluster_components(&s);

    // `drift` keys on opaque component uuids (out of scope here) — bridge at the boundary.
    let priors = drift::live_components(pool, anchor, s.lens_id.uuid()).await?;
    if priors.is_empty() {
        // nothing to diff against — the first materialize for this lens is a full pass.
        return materialize(pool, anchor, lens_name, emitter).await;
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
    // identical to what a full pass at this watermark computes. Only the CHANGED components re-assert
    // regions this act; an unchanged component's regions are not touched, and trivially keep their ids.
    //
    // Within the changed components, a cluster whose member set matches a live region REUSES that
    // region's id — the case that matters in the context regime, where the whole anchor is ONE
    // component, so any content edit marks it changed and re-clusters it even when the partition comes
    // out identical. Matching against every live region of the anchor is safe: components partition the
    // node set, so a changed component's clusters can only collide with regions of that same component.
    let all_clusters: Vec<Vec<Uuid>> = comps.iter().flat_map(|c| c.clusters.clone()).collect();
    let fingerprint = membership_fingerprint(&all_clusters);
    let live = live_regions(pool, anchor, s.lens_id).await?;
    let assignments = resolve_region_ids(&changed, &live);
    let flat_region_ids: Vec<RegionId> = assignments
        .iter()
        .flatten()
        .map(|a| a.region_id())
        .collect();
    let keep = reused_ids(&assignments);
    let (regions_reused, regions_minted) = count_assignments(&assignments);

    let mut tx = pool.begin().await?;
    let watermark = current_watermark(&mut tx).await?;
    // region_ids records the regions THIS act (re-)asserted — the changed components' regions, reused
    // and minted alike, exactly the set it has always recorded. The full membership fingerprint records
    // the complete resulting shape (untouched components' regions included).
    let ev = fire(
        &mut tx,
        SeedAction::Materialize {
            anchor,
            lens: s.lens_id,
            watermark: EventId::from(watermark),
            membership_fingerprint: &fingerprint,
            region_ids: &flat_region_ids,
            emitter,
        },
    )
    .await?
    .materialize_event()?;

    // fold the stale components and their regions; leave matched components + their regions live, and
    // leave the reused regions of the STALE components live too — their member sets did not change.
    fold_components(&mut tx, &diff.stale, ev, &keep).await?;

    let zero = zero_centroid();
    let work: Vec<(&ComponentWork, &Vec<RegionAssignment>)> =
        changed.iter().copied().zip(&assignments).collect();
    assert_component_regions(&mut tx, anchor, &s, &zero, ev, &work).await?;

    refresh_moved_region_readouts(pool, &mut tx, anchor, &s, &zero, &diff.unchanged, ev).await?;

    tx.commit().await?;

    Ok(MaterializeOutcome {
        regions: all_clusters.len(),
        membership_fingerprint: fingerprint,
        regions_reused,
        regions_minted,
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

/// The live (non-folded) regions for (anchor, lens), keyed by their SORTED member set. The basis for
/// region-id reuse — `drift::live_components`, one grain down.
async fn live_regions(
    pool: &PgPool,
    anchor: HomeAnchor,
    lens_id: LensId,
) -> Result<HashMap<Vec<Uuid>, RegionId>> {
    let rows = sqlx::query!(
        "SELECT r.id, array_agg(m.member_id ORDER BY m.member_id) AS members \
         FROM kb_cogmap_regions r \
         JOIN kb_cogmap_region_members m \
           ON m.region_id = r.id AND m.member_table = 'kb_resources' \
         WHERE r.home_anchor_table=$1 AND r.home_anchor_id=$2 AND r.lens_id=$3 AND NOT r.is_folded \
         GROUP BY r.id",
        anchor.table(),
        anchor.uuid(),
        lens_id.uuid(),
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|r| r.members.map(|ms| (ms, RegionId::from(r.id))))
        .collect())
}

/// Resolve the region id for every cluster of every given component, grouped per component (aligned
/// with each `ComponentWork.clusters`): REUSE the live region whose member set is exactly this
/// cluster's, else MINT a fresh id (identity-as-input — the id enters the materialization payload
/// before the row is written).
///
/// A member set identifies at most one live region, because regions partition the anchor's nodes — the
/// same disjointness `drift::classify` relies on to match components. Sorting the cluster is not
/// decorative: `agglomerate` seeds from a sorted node list but merges by appending, so a cluster's Vec
/// is NOT ordered by construction, while `live_regions` keys on `array_agg(… ORDER BY member_id)`.
fn resolve_region_ids(
    comps: &[&ComponentWork],
    live: &HashMap<Vec<Uuid>, RegionId>,
) -> Vec<Vec<RegionAssignment>> {
    comps
        .iter()
        .map(|c| {
            c.clusters
                .iter()
                .map(|members| {
                    let mut key = members.clone();
                    key.sort();
                    match live.get(&key) {
                        Some(&id) => RegionAssignment::Reuse(id),
                        None => RegionAssignment::Mint(RegionId::from(Uuid::now_v7())),
                    }
                })
                .collect()
        })
        .collect()
}

/// Persist each component and assert its regions (members + readouts) in order, sharing the parent's
/// transaction. `work` pairs each component with its pre-minted per-cluster region ids.
///
/// Takes the whole `Substrate` rather than `(lens_id, lens)`: `assert_region` now needs the edges and
/// facets too, to score each member's affinity to its component peers.
async fn assert_component_regions(
    tx: &mut PgConnection,
    anchor: HomeAnchor,
    sub: &Substrate,
    zero_centroid: &str,
    ev: EventId,
    work: &[(&ComponentWork, &Vec<RegionAssignment>)],
) -> Result<()> {
    for &(comp, comp_assignments) in work {
        let comp_id = create_component(&mut *tx, anchor, sub.lens_id, comp, ev).await?;
        for (members, assignment) in comp.clusters.iter().zip(comp_assignments) {
            assert_region(
                &mut *tx,
                AssertRegionCtx {
                    anchor,
                    component_id: comp_id,
                    members,
                    assignment: *assignment,
                    ev,
                    sub,
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
    anchor: HomeAnchor,
    s: &Substrate,
    zero_centroid: &str,
    unchanged: &[Uuid],
    ev: EventId,
) -> Result<()> {
    let prior_watermark = last_materialize_watermark(tx, anchor, s.lens_id, ev).await?;
    let touched_resources = match prior_watermark {
        Some(w) => crate::replay::content_touched_resources_since(pool, anchor, w).await?,
        None => Vec::new(),
    };
    if !touched_resources.is_empty() {
        // one query for the reused regions that actually contain a moved member (not every reused
        // region, and not N per-component round-trips).
        let region_ids: Vec<Uuid> = sqlx::query_scalar!(
            "SELECT DISTINCT r.id FROM kb_cogmap_regions r \
             JOIN kb_cogmap_region_members m ON m.region_id = r.id \
             WHERE r.component_id = ANY($1) AND NOT r.is_folded \
               AND m.member_table = 'kb_resources' AND m.member_id = ANY($2)",
            unchanged,
            &touched_resources,
        )
        .fetch_all(&mut **tx)
        .await?;
        for rid in &region_ids {
            populate_readouts(tx, *rid, &s.lens, zero_centroid).await?;
        }
        // one batched last_event_id stamp for every refreshed region (same `ev` for all).
        if !region_ids.is_empty() {
            sqlx::query!(
                "UPDATE kb_cogmap_regions SET last_event_id=$1 WHERE id = ANY($2)",
                ev.uuid(),
                &region_ids,
            )
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

/// The event id of the most recent region_materialized act for (anchor, lens) BEFORE `current_ev`
/// (this pass's own act, already appended) — the point-in-time the reused regions' readouts were last
/// computed against. Excluding `current_ev` explicitly states the intent directly rather than relying
/// on "my own event is the single latest" (an `OFFSET 1` would silently return the wrong watermark
/// under a second act in the same txn or a concurrent materialize). `None` only if this is the very
/// first materialize, where incremental never reaches the readout-refresh path.
///
/// The dual-read of the payload anchor (new pair, else the pre-T3 `cogmap_id`) lives in
/// [`crate::replay::last_materialize_event`] — shared with drift's copy of this probe.
async fn last_materialize_watermark(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    anchor: HomeAnchor,
    lens_id: LensId,
    current_ev: EventId,
) -> Result<Option<Uuid>> {
    crate::replay::last_materialize_event(
        &mut **tx,
        anchor,
        lens_id.uuid(),
        Some(current_ev.uuid()),
    )
    .await
}

/// Fold every live region for (anchor, lens) EXCEPT those in `keep` — the regions being reused, whose
/// member set is unchanged. Keyed on the ANCHOR PAIR, not `cogmap_id` — which is why `20260712000040`
/// opens with a catch-up backfill: a region written in the T2→T3 window has a NULL anchor, would not
/// match this predicate, and would survive the fold as a duplicate live region.
///
/// `<> ALL('{}')` is TRUE, so an empty `keep` folds everything — the pre-reuse behavior exactly.
async fn fold_live_regions(
    tx: &mut PgConnection,
    anchor: HomeAnchor,
    lens_id: LensId,
    ev: EventId,
    keep: &[Uuid],
) -> Result<()> {
    sqlx::query!(
        "UPDATE kb_cogmap_regions SET is_folded=true, last_event_id=$1 \
         WHERE home_anchor_table=$2 AND home_anchor_id=$3 AND lens_id=$4 AND NOT is_folded \
           AND id <> ALL($5)",
        ev.uuid(),
        anchor.table(),
        anchor.uuid(),
        lens_id.uuid(),
        keep,
    )
    .execute(tx)
    .await?;
    Ok(())
}

/// See [`fold_live_regions`] — same anchor-keyed predicate, same backfill dependency.
async fn fold_live_components(
    tx: &mut PgConnection,
    anchor: HomeAnchor,
    lens_id: LensId,
    ev: EventId,
) -> Result<()> {
    sqlx::query!(
        "UPDATE kb_cogmap_components SET is_folded=true, last_event_id=$1 \
         WHERE home_anchor_table=$2 AND home_anchor_id=$3 AND lens_id=$4 AND NOT is_folded",
        ev.uuid(),
        anchor.table(),
        anchor.uuid(),
        lens_id.uuid(),
    )
    .execute(tx)
    .await?;
    Ok(())
}

/// Fold specific components by id AND their live regions (incremental's stale path), EXCEPT the regions
/// in `keep` — reused regions of a stale component survive the fold under their own ids. The component
/// row itself is always folded: a re-clustered component is a new component row, and `assert_region`
/// re-parents the survivors onto it.
async fn fold_components(
    tx: &mut PgConnection,
    component_ids: &[Uuid],
    ev: EventId,
    keep: &[Uuid],
) -> Result<()> {
    if component_ids.is_empty() {
        return Ok(());
    }
    sqlx::query!(
        "UPDATE kb_cogmap_regions SET is_folded=true, last_event_id=$1 \
         WHERE component_id = ANY($2) AND NOT is_folded AND id <> ALL($3)",
        ev.uuid(),
        component_ids,
        keep,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "UPDATE kb_cogmap_components SET is_folded=true, last_event_id=$1 WHERE id = ANY($2)",
        ev.uuid(),
        component_ids,
    )
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Persist one component row (its fingerprint + member set), returning its id for the regions to link.
///
/// `cogmap_id` is DUAL-WRITTEN through the expand window (spec §3.6 M1) so the previous commit's code
/// keeps reading these rows; it is NULL for a context component, which that code path never reads.
async fn create_component(
    tx: &mut PgConnection,
    anchor: HomeAnchor,
    lens_id: LensId,
    comp: &ComponentWork,
    ev: EventId,
) -> Result<Uuid> {
    let id = sqlx::query_scalar!(
        "INSERT INTO kb_cogmap_components \
           (cogmap_id, home_anchor_table, home_anchor_id, lens_id, fingerprint, member_ids, \
            asserted_by_event_id, last_event_id) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$7) RETURNING id",
        anchor.cogmap_id().map(|m| m.uuid()),
        anchor.table(),
        anchor.uuid(),
        lens_id.uuid(),
        &comp.fingerprint,
        &comp.members,
        ev.uuid(),
    )
    .fetch_one(&mut *tx)
    .await?;
    Ok(id)
}

/// Parameters for [`assert_region`]. The region id is pre-resolved (identity-as-input) and already
/// recorded in the materialization payload before this is called — reused or minted.
struct AssertRegionCtx<'a> {
    anchor: HomeAnchor,
    component_id: Uuid,
    members: &'a [Uuid],
    assignment: RegionAssignment,
    ev: EventId,
    /// The whole substrate, not just the lens: `member_affinity` needs the edges and facets to score
    /// each member against its peers.
    sub: &'a Substrate,
    zero_centroid: &'a str,
}

/// How CORE a member is to its region: its average-link affinity to the region's other members.
///
/// **This column was never written.** `kb_cogmap_region_members.affinity` has existed since the region
/// tier shipped, and four readers order by it — `graph_region_members`, `graph_region_territories`,
/// `graph_cogmap_territories`, `atlas_search`, all `ORDER BY m.affinity DESC NULLS LAST` — but nothing
/// ever populated it, so every "top member" and every derived region label in the product has been
/// arbitrary (whatever order the planner happened to return). Spec §3.9.1.
///
/// Average-link is the same linkage the clustering itself uses, so a member's score is coherent with
/// why it landed in this region. A singleton region yields 0.0 — there are no peers to be central to.
fn member_affinity(m: Uuid, members: &[Uuid], sub: &Substrate) -> f64 {
    let peers = members.iter().filter(|&&x| x != m).count();
    if peers == 0 {
        return 0.0;
    }
    let total: f64 = members
        .iter()
        .filter(|&&x| x != m)
        .map(|&p| {
            affinity(
                ResourceId::from(m),
                ResourceId::from(p),
                &sub.edges,
                &sub.facets,
                &sub.knn,
                &sub.lens,
            )
        })
        .sum();
    total / peers as f64
}

/// Bring one region to its current shape — its row, its members (each scored by [`member_affinity`]),
/// then its SQL readouts.
///
/// **Mint** inserts a new row. `cogmap_id` is dual-written through the expand window — NULL for a
/// context region, which the pre-M2 code path never reads.
///
/// **Reuse** UPDATEs the SURVIVING row of a region whose member set is unchanged. It is an UPDATE and
/// not a re-INSERT because `id` is the primary key and the fold is SOFT — the folded row still exists,
/// so re-inserting its id would be a duplicate-key violation. The region was therefore never folded at
/// all (`fold_live_regions`/`fold_components` skip it via `keep`); it is merely re-parented onto the
/// component row this pass created. `asserted_by_event_id` is deliberately NOT touched: it records when
/// the region came into being, which is precisely the identity reuse exists to preserve.
async fn assert_region(tx: &mut PgConnection, ctx: AssertRegionCtx<'_>) -> Result<()> {
    let AssertRegionCtx {
        anchor,
        component_id,
        members,
        assignment,
        ev,
        sub,
        zero_centroid,
    } = ctx;
    let region = assignment.region_id().uuid();
    match assignment {
        // centroid computed in SQL after members are written; insert a placeholder then UPDATE.
        RegionAssignment::Mint(_) => {
            sqlx::query(
                "INSERT INTO kb_cogmap_regions \
                   (id, cogmap_id, home_anchor_table, home_anchor_id, lens_id, component_id, centroid, \
                    salience, label, member_count, asserted_by_event_id, last_event_id) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7::vector, 0.0, NULL, $8, $9, $9)",
            )
            .bind(region)
            .bind(anchor.cogmap_id().map(|m| m.uuid()))
            .bind(anchor.table())
            .bind(anchor.uuid())
            .bind(sub.lens_id)
            .bind(component_id)
            .bind(zero_centroid)
            .bind(members.len() as i32)
            .bind(ev)
            .execute(&mut *tx)
            .await?;
        }
        RegionAssignment::Reuse(_) => {
            // `is_folded=false` is not redundant with the fold's `keep` list. `live_regions` reads
            // BEFORE this transaction opens, so a materialize racing us on the same anchor could have
            // folded this region in between — and an UPDATE that left `is_folded` alone would then
            // stamp a folded row and quietly drop a region out of the live partition. Re-asserting it
            // live states the act's intent: this region belongs to the partition I just computed.
            let touched = sqlx::query(
                "UPDATE kb_cogmap_regions \
                 SET component_id=$2, member_count=$3, last_event_id=$4, is_folded=false \
                 WHERE id=$1",
            )
            .bind(region)
            .bind(component_id)
            .bind(members.len() as i32)
            .bind(ev)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            // A reused id comes from a row we just read. If it is gone, something hard-deleted a live
            // region under us — fail loudly rather than return a partition that silently lost one.
            if touched != 1 {
                anyhow::bail!(
                    "region {region} was reused but no longer exists ({touched} rows updated) — a \
                     live region was deleted concurrently with this materialize"
                );
            }
        }
    }
    write_region_members(&mut *tx, region, members, sub).await?;
    populate_readouts(tx, region, &sub.lens, zero_centroid).await
}

/// Write a region's member rows, each scored by [`member_affinity`].
///
/// DELETE-then-INSERT rather than insert-only, because a REUSED region already carries member rows.
/// Reuse proves the member SET is unchanged — but `affinity` is a function of the substrate (edges,
/// facets, embeddings), and a region is only re-asserted because that substrate moved. Four readers
/// `ORDER BY m.affinity DESC`, so carrying the prior scores forward would trade one silent lie for
/// another. On a minted region the DELETE is a no-op.
async fn write_region_members(
    tx: &mut PgConnection,
    region: Uuid,
    members: &[Uuid],
    sub: &Substrate,
) -> Result<()> {
    sqlx::query!(
        "DELETE FROM kb_cogmap_region_members WHERE region_id=$1",
        region
    )
    .execute(&mut *tx)
    .await?;
    for m in members {
        sqlx::query(
            "INSERT INTO kb_cogmap_region_members (region_id, member_table, member_id, affinity) \
             VALUES ($1,'kb_resources',$2,$3)",
        )
        .bind(region)
        .bind(m)
        .bind(member_affinity(*m, members, sub))
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn work(clusters: Vec<Vec<Uuid>>) -> ComponentWork {
        let mut members: Vec<Uuid> = clusters.iter().flatten().copied().collect();
        members.sort();
        ComponentWork {
            members,
            fingerprint: "fp".into(),
            clusters,
        }
    }

    fn live(entries: &[(&[Uuid], Uuid)]) -> HashMap<Vec<Uuid>, RegionId> {
        entries
            .iter()
            .map(|(members, rid)| {
                let mut key = members.to_vec();
                key.sort();
                (key, RegionId::from(*rid))
            })
            .collect()
    }

    /// The whole fix, in one assertion: a cluster whose member set matches a live region takes that
    /// region's id back rather than minting a new one.
    #[test]
    fn an_unchanged_member_set_reuses_its_region_id() {
        let comp = work(vec![vec![id(1), id(2)], vec![id(3)]]);
        let live = live(&[(&[id(1), id(2)], id(900)), (&[id(3)], id(901))]);

        let got = resolve_region_ids(&[&comp], &live);

        assert!(matches!(got[0][0], RegionAssignment::Reuse(r) if r.uuid() == id(900)));
        assert!(matches!(got[0][1], RegionAssignment::Reuse(r) if r.uuid() == id(901)));
        assert_eq!(count_assignments(&got), (2, 0));
        assert_eq!(reused_ids(&got), vec![id(900), id(901)]);
    }

    /// Supersession stays honest. A member set that changed at all — even by one member, even when the
    /// region is "obviously the same region" to a human — is a NEW region and gets a new id. Anything
    /// looser would let a consumer holding that id silently follow a region it never asked for.
    #[test]
    fn a_changed_member_set_mints_a_new_id() {
        let comp = work(vec![vec![id(1), id(2), id(4)], vec![id(3)]]);
        let live = live(&[(&[id(1), id(2)], id(900)), (&[id(3)], id(901))]);

        let got = resolve_region_ids(&[&comp], &live);

        // {1,2,4} is not {1,2} — mint.
        assert!(matches!(got[0][0], RegionAssignment::Mint(_)));
        // {3} is untouched — its id survives the re-cluster of its component.
        assert!(matches!(got[0][1], RegionAssignment::Reuse(r) if r.uuid() == id(901)));
        assert_eq!(count_assignments(&got), (1, 1));
        assert_eq!(
            reused_ids(&got),
            vec![id(901)],
            "only the reused region enters the fold's keep-list; the superseded one must be folded"
        );
    }

    /// `agglomerate` seeds from a sorted node list but merges by APPENDING, so a cluster's Vec is not
    /// ordered by construction — while `live_regions` keys on `array_agg(… ORDER BY member_id)`. If the
    /// match key were not sorted on both sides, reuse would silently miss and every id would churn,
    /// which is indistinguishable from having never made this change at all.
    #[test]
    fn the_match_key_is_order_insensitive() {
        let comp = work(vec![vec![id(2), id(1)]]);
        let live = live(&[(&[id(1), id(2)], id(900))]);

        let got = resolve_region_ids(&[&comp], &live);

        assert!(matches!(got[0][0], RegionAssignment::Reuse(r) if r.uuid() == id(900)));
    }

    /// The first materialize of an anchor: nothing live to match, so everything mints — and `keep` is
    /// empty, which makes the fold's `id <> ALL('{}')` fold everything, exactly as before this change.
    #[test]
    fn no_live_regions_means_everything_mints() {
        let comp = work(vec![vec![id(1), id(2)], vec![id(3)]]);

        let got = resolve_region_ids(&[&comp], &HashMap::new());

        assert_eq!(count_assignments(&got), (0, 2));
        assert!(reused_ids(&got).is_empty());
    }
}
