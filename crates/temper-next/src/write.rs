use crate::events::{fire, SeedAction};
use crate::ids::{CogmapId, EntityId, EventId, LensId, RegionId};
use crate::{affinity::affinity, cluster::cluster, substrate};
use anyhow::{Context, Result};
use sqlx::PgPool;
use uuid::Uuid;

pub struct MaterializeOutcome {
    pub regions: usize,
    pub membership_fingerprint: String,
}

/// Job B (spec §6a): read substrate -> declared-only affinity -> deterministic clustering ->
/// fold prior regions + assert new ones + members under ONE materialization event -> populate the
/// SQL readouts (Plan 1 functions). Cosine never enters formation; it enters only via the readouts.
pub async fn materialize_cogmap(
    pool: &PgPool,
    cogmap: Uuid,
    lens_name: &str,
    emitter: Uuid,
) -> Result<MaterializeOutcome> {
    let s = substrate::load(pool, cogmap, lens_name).await?;
    let aff = |x: Uuid, y: Uuid| affinity(x, y, &s.edges, &s.facets, &s.lens);
    let clusters = cluster(&s.nodes, &aff, s.lens.resolution);

    // fingerprint + region ids BEFORE the event (payload-first): the region_materialized payload
    // records the act's full identity — lens, watermark, membership fingerprint, region ids.
    let mut fingerprint_parts: Vec<String> = clusters
        .iter()
        .map(|members| {
            let mut ms: Vec<String> = members.iter().map(|m| m.to_string()).collect();
            ms.sort();
            ms.join("+")
        })
        .collect();
    fingerprint_parts.sort();
    let fingerprint = fingerprint_parts.join("|");
    let region_ids: Vec<RegionId> = clusters
        .iter()
        .map(|_| RegionId::from(Uuid::now_v7()))
        .collect();

    let mut tx = pool.begin().await?;
    // the substrate point-in-time this projection saw (uuidv7 — time-ordered; no max(uuid) in PG)
    let watermark: Uuid = sqlx::query_scalar!("SELECT id FROM kb_events ORDER BY id DESC LIMIT 1")
        .fetch_optional(&mut *tx)
        .await?
        .context("materialize on an empty ledger (no events)")?;
    // one materialization event (correlation root), fired through the single events surface. The emitter
    // is passed explicitly (the actor on whose behalf materialization runs) — never derived from "latest
    // event", which is NULL on an empty log and arbitrary on occurred_at ties. The projection half
    // (_project_region_materialized) records the watermark on the cogmap.
    let ev: Uuid = fire(
        &mut tx,
        SeedAction::Materialize {
            cogmap: CogmapId::from(cogmap),
            lens: LensId::from(s.lens_id),
            watermark: EventId::from(watermark),
            membership_fingerprint: &fingerprint,
            region_ids: &region_ids,
            emitter: EntityId::from(emitter),
        },
    )
    .await?
    .materialize_event()?
    .uuid();
    // fold prior live regions for this lens
    sqlx::query(
        "UPDATE kb_cogmap_regions SET is_folded=true, last_event_id=$1 \
         WHERE cogmap_id=$2 AND lens_id=$3 AND NOT is_folded",
    )
    .bind(ev)
    .bind(cogmap)
    .bind(s.lens_id)
    .execute(&mut *tx)
    .await?;

    // A zero-vector(768) literal placeholder for the NOT-NULL centroid (overwritten by the UPDATE
    // below before any readout reads it). An unconditional zero literal — NOT a fragile
    // `SELECT centroid … LIMIT 1`, which would be NULL on a clean run once Plan 3 removes the
    // hand-seeded region, violating the NOT NULL constraint.
    let zero_centroid = format!(
        "[{}]",
        vec!["0"; temper_ingest::embed::EMBEDDING_DIM].join(",")
    );
    for (members, region_id) in clusters.iter().zip(&region_ids) {
        // centroid computed in SQL after members are inserted; insert a placeholder then UPDATE.
        // The region id is pre-generated (identity-as-input) — it is already recorded in the payload.
        let region: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_cogmap_regions \
               (id, cogmap_id, lens_id, centroid, salience, label, member_count, asserted_by_event_id, last_event_id) \
             VALUES ($6, $1,$2, $5::vector, 0.0, NULL, $3, $4, $4) RETURNING id",
        )
        .bind(cogmap)
        .bind(s.lens_id)
        .bind(members.len() as i32)
        .bind(ev)
        .bind(&zero_centroid)
        .bind(region_id.uuid())
        .fetch_one(&mut *tx)
        .await?;
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
        .bind(&zero_centroid)
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
        // salience = lens-weighted blend of the three parts.
        // telos_alignment is NULLABLE (NULL when the telos has no embedded chunks), and salience is
        // NOT NULL — so `$2*NULL = NULL` would violate the constraint. coalesce to 0.
        // (reference_standing/centrality are coalesce'd to 0 inside their SQL functions, so only
        // telos_alignment needs guarding here. No cogmap_region_salience fn shipped in Plan 1 ⇒ inline.)
        sqlx::query(
            "UPDATE kb_cogmap_regions SET salience = \
               $2*coalesce(telos_alignment,0) + $3*reference_standing + $4*centrality WHERE id=$1",
        )
        .bind(region)
        .bind(s.lens.s_telos)
        .bind(s.lens.s_ref)
        .bind(s.lens.s_central)
        .execute(&mut *tx)
        .await?;
    }
    // (the materialization watermark on kb_cogmaps is set by _project_region_materialized — the
    // event's projection half — not here.)
    tx.commit().await?;

    Ok(MaterializeOutcome {
        regions: clusters.len(),
        membership_fingerprint: fingerprint,
    })
}
