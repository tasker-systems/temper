use crate::{affinity::affinity, cluster::cluster, substrate};
use anyhow::Result;
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
) -> Result<MaterializeOutcome> {
    let s = substrate::load(pool, cogmap, lens_name).await?;
    let aff = |x: Uuid, y: Uuid| affinity(x, y, &s.edges, &s.facets, &s.lens);
    let clusters = cluster(&s.nodes, &aff, s.lens.resolution);

    let mut tx = pool.begin().await?;
    // one materialization event (correlation root)
    let ev: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id) \
         SELECT (SELECT id FROM kb_event_types WHERE name='region_materialized'), \
                (SELECT emitter_entity_id FROM kb_events ORDER BY occurred_at DESC LIMIT 1), \
                'kb_cogmaps', $1 RETURNING id",
    )
    .bind(cogmap)
    .fetch_one(&mut *tx)
    .await?;
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
    let mut fingerprint_parts: Vec<String> = Vec::new();
    for members in &clusters {
        // centroid computed in SQL after members are inserted; insert a placeholder then UPDATE.
        let region: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_cogmap_regions \
               (cogmap_id, lens_id, centroid, salience, label, member_count, asserted_by_event_id, last_event_id) \
             VALUES ($1,$2, $5::vector, 0.0, NULL, $3, $4, $4) RETURNING id",
        )
        .bind(cogmap)
        .bind(s.lens_id)
        .bind(members.len() as i32)
        .bind(ev)
        .bind(&zero_centroid)
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
        // populate centroid + readouts via the Plan-1 SQL functions + a centroid recompute
        sqlx::query(
            "UPDATE kb_cogmap_regions r SET \
               centroid = (SELECT avg(ch.embedding) FROM kb_cogmap_region_members mm \
                           JOIN kb_chunks ch ON ch.resource_id=mm.member_id AND ch.is_current \
                           WHERE mm.region_id=r.id), \
               content_cohesion   = cogmap_region_content_cohesion(r.id), \
               telos_alignment    = cogmap_region_telos_alignment(r.id, r.cogmap_id), \
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
        let mut ms: Vec<String> = members.iter().map(|m| m.to_string()).collect();
        ms.sort();
        fingerprint_parts.push(ms.join("+"));
    }
    sqlx::query("UPDATE kb_cogmaps SET shape_materialized_event_id=$1 WHERE id=$2")
        .bind(ev)
        .bind(cogmap)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    fingerprint_parts.sort();
    Ok(MaterializeOutcome {
        regions: clusters.len(),
        membership_fingerprint: fingerprint_parts.join("|"),
    })
}
