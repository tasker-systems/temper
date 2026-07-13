use crate::affinity::{Edge, EdgeKind, Facet, Lens};
use crate::ids::{CogmapId, ContextId, LensId, ProfileId, ResourceId};
use crate::knn::{self, KnnGraph};
use anyhow::{Context, Result};
use sqlx::PgPool;
use std::collections::HashMap;
use temper_core::types::home::HomeAnchor;

#[derive(Debug)]
pub struct Substrate {
    pub nodes: Vec<ResourceId>,
    pub edges: Vec<Edge>,
    pub facets: Vec<Facet>,
    /// The inferred half of the affinity kernel — the sparse exact-kNN graph over the members'
    /// pooled embeddings. **Empty in the declared-only (cogmap) regime**, where `load` never even
    /// runs the embedding query: a cogmap must not pay for a signal its lens ignores.
    pub knn: KnnGraph,
    pub lens: Lens,
    pub lens_id: LensId,
}

pub async fn connect() -> Result<PgPool> {
    // The connection's search_path is the database default (`public`) in production, dev, and
    // tests. No per-connection `SET search_path` is needed. In tests, ephemeral databases are
    // provided by `#[sqlx::test]` with `temper_substrate::MIGRATOR` applied to `public`.
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://temper:temper@localhost:5437/temper_development".into());
    Ok(PgPool::connect(&url).await?)
}

pub async fn cogmap_by_name(pool: &PgPool, name: &str) -> Result<CogmapId> {
    let id = sqlx::query_scalar!("SELECT id FROM kb_cogmaps WHERE name = $1", name)
        .fetch_one(pool)
        .await?;
    Ok(CogmapId::from(id))
}

/// A context is addressed by (owner, slug) — the slug is unique per owner, not globally, so this is
/// the peer of [`cogmap_by_name`] and not a drop-in shape.
pub async fn context_by_slug(pool: &PgPool, owner: ProfileId, slug: &str) -> Result<ContextId> {
    let id = sqlx::query_scalar!(
        "SELECT id FROM kb_contexts WHERE owner_table='kb_profiles' AND owner_id=$1 AND slug=$2",
        owner.uuid(),
        slug,
    )
    .fetch_one(pool)
    .await?;
    Ok(ContextId::from(id))
}

/// Load the substrate homed in `anchor` — a context OR a cognitive map. The anchor kind is a bound
/// value, not a hard-wired literal, so the same producer addresses both regimes; which regime it is
/// then *in* is decided entirely by the lens it resolves (`w_cos = 0` ⇒ the declared-graph-only
/// cogmap regime).
pub async fn load(pool: &PgPool, anchor: HomeAnchor, lens_name: &str) -> Result<Substrate> {
    let anchor_table = anchor.table();
    let anchor_id = anchor.uuid();

    // Resources homed in the anchor — LIVE ones only.
    //
    // A soft delete (`resource_delete` → `is_active = false`) deliberately leaves the `kb_resource_homes`
    // row in place: the row is preserved, not erased. So the home table alone is NOT the active set, and
    // joining `kb_resources` to filter tombstones is load-bearing rather than defensive. Without it a
    // deleted resource stayed in formation forever — clustered into a region, its vector still pulling the
    // centroid, still counted in `member_count` and `centrality`. Measured on prod: every dead-but-homed
    // resource was still a region member (40 for 40), and six regions had no live member at all.
    //
    // This is the ONE place the filter belongs. `nodes` is the root of every other input the producer
    // loads — facets, kNN/embeddings, edges, clusters, centroids — so filtering here fixes all of them at
    // once, and membership stays written solely by the materialize projection (a projection table is never
    // side-written; `resource_deleted` ticks the formation clock so the region re-forms promptly instead).
    let nodes: Vec<ResourceId> = sqlx::query_scalar!(
        "SELECT h.resource_id FROM kb_resource_homes h \
           JOIN kb_resources r ON r.id = h.resource_id \
          WHERE h.anchor_table=$1 AND h.anchor_id=$2 AND r.is_active",
        anchor_table,
        anchor_id,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(ResourceId::from)
    .collect();

    // declared edges homed in the anchor, both endpoints resources
    let edge_rows = sqlx::query!(
        "SELECT source_id, target_id, edge_kind::text AS \"kind!\", label, weight \
         FROM kb_edges WHERE home_anchor_table=$1 AND home_anchor_id=$2 \
           AND source_table='kb_resources' AND target_table='kb_resources' AND NOT is_folded",
        anchor_table,
        anchor_id,
    )
    .fetch_all(pool)
    .await?;
    let edges = edge_rows
        .into_iter()
        .map(|r| -> Result<Edge> {
            let kind = EdgeKind::from_sql(&r.kind)
                .with_context(|| format!("unknown edge_kind from DB: {:?}", r.kind))?;
            Ok(Edge {
                src: ResourceId::from(r.source_id),
                tgt: ResourceId::from(r.target_id),
                kind,
                weight: r.weight,
                label: r.label,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    // facets on those resources (property_key='facet', value jsonb {path:value})
    let facet_rows = sqlx::query!(
        "SELECT owner_id, property_value, weight FROM kb_properties \
         WHERE owner_table='kb_resources' AND property_key='facet' AND NOT is_folded \
           AND owner_id = ANY($1)",
        &nodes as &[ResourceId],
    )
    .fetch_all(pool)
    .await?;
    let facets = facet_rows
        .iter()
        .flat_map(|r| expand_facets(ResourceId::from(r.owner_id), &r.property_value, r.weight))
        .collect();

    let (lens, lens_id) = load_lens(pool, anchor, lens_name).await?;

    let knn = load_knn(pool, &nodes, &lens).await?;

    Ok(Substrate {
        nodes,
        edges,
        facets,
        knn,
        lens,
        lens_id,
    })
}

/// Resolve the named lens for an anchor — the lens row ALONE, with no substrate around it.
///
/// Split out of [`load`] for T6's cheap clock (spec §3.5): a salience-only refresh needs the lens's
/// blend weights and its id, and NOTHING else — no nodes, no edges, no facets, and above all no kNN
/// graph, whose exact-cosine build is O(n²·768) and is the single most expensive thing `load` does.
/// Resolving the lens through the same function the producer uses means the two cannot drift apart on
/// which lens they mean.
///
/// The named lens for this anchor, else the global default (`home_anchor_table IS NULL` — how
/// `telos-default`, `telos-default-propheavy` and `workflow-default` are seeded). The name is bound
/// (Plan 3 Step 0) so the same producer materializes different lenses over one substrate — S6f
/// plurality. `NULLS LAST` keeps an anchor-specific lens winning over the global default, exactly as
/// `ORDER BY cogmap_id NULLS LAST` did.
pub async fn load_lens(
    pool: &PgPool,
    anchor: HomeAnchor,
    lens_name: &str,
) -> Result<(Lens, LensId)> {
    let lr = sqlx::query!(
        "SELECT id, w_express, w_contains, w_leads_to, w_near, w_prop, w_cos, knn_k, cos_floor, \
                s_telos, s_ref, s_central, resolution \
         FROM kb_cogmap_lenses \
         WHERE name=$3 AND (home_anchor_table IS NULL OR (home_anchor_table=$1 AND home_anchor_id=$2)) \
         ORDER BY home_anchor_table NULLS LAST LIMIT 1",
        anchor.table(),
        anchor.uuid(),
        lens_name,
    )
    .fetch_one(pool)
    .await?;
    let lens = Lens {
        w_express: lr.w_express,
        w_contains: lr.w_contains,
        w_leads_to: lr.w_leads_to,
        w_near: lr.w_near,
        w_prop: lr.w_prop,
        w_cos: lr.w_cos,
        // SQL INT → Rust usize at the boundary. A negative k is meaningless; clamp rather than panic.
        knn_k: lr.knn_k.max(0) as usize,
        cos_floor: lr.cos_floor,
        s_telos: lr.s_telos,
        s_ref: lr.s_ref,
        s_central: lr.s_central,
        resolution: lr.resolution,
    };
    Ok((lens, LensId::from(lr.id)))
}

/// Pool each node's chunk embeddings into one per-resource vector and build the sparse kNN graph.
///
/// **Skipped entirely when `w_cos == 0.0`** — the declared-only regime multiplies the whole term by
/// zero, so a cogmap must not pay for the embedding scan, and its `Substrate` carries an empty graph.
/// That is the same predicate the fingerprint and the affinity kernel key on; all three agree that at
/// `w_cos = 0` the kNN is simply not an input.
///
/// The pooling — `avg(chunk.embedding)` over current, unfolded blocks — is deliberately the SAME pool
/// `populate_readouts` uses for region centroids (`write.rs`), so formation and readout agree on what
/// a resource's vector IS. Two different poolings would mean a region whose centroid is not the mean
/// of the thing that formed it.
async fn load_knn(pool: &PgPool, nodes: &[ResourceId], lens: &Lens) -> Result<KnnGraph> {
    if lens.w_cos == 0.0 {
        return Ok(KnnGraph::default());
    }

    // pgvector has no native sqlx decode here, and the codebase's established round-trip is a `::text`
    // cast parsed back on the Rust side (cf. `replay.rs`'s `c.embedding::text`). Do not introduce a
    // third convention.
    let rows = sqlx::query!(
        "SELECT ch.resource_id, avg(ch.embedding)::text AS \"vec?\" \
           FROM kb_chunks ch \
           JOIN kb_content_blocks b ON b.id = ch.block_id AND NOT b.is_folded \
          WHERE ch.is_current AND ch.embedding IS NOT NULL AND ch.resource_id = ANY($1) \
          GROUP BY ch.resource_id",
        &nodes.iter().map(|n| n.uuid()).collect::<Vec<_>>()[..],
    )
    .fetch_all(pool)
    .await?;

    let embeddings: HashMap<ResourceId, Vec<f32>> = rows
        .into_iter()
        .filter_map(|r| {
            let v = parse_pgvector(r.vec.as_deref()?)?;
            Some((ResourceId::from(r.resource_id), v))
        })
        .collect();

    Ok(knn::build(&embeddings, lens.knn_k, lens.cos_floor))
}

/// Parse pgvector's text form — `[0.1,-0.2,…]` — into floats. `None` for anything unparseable, which
/// drops that resource from the kNN graph rather than admitting a corrupt vector into formation.
fn parse_pgvector(s: &str) -> Option<Vec<f32>> {
    let body = s.trim().strip_prefix('[')?.strip_suffix(']')?;
    if body.is_empty() {
        return None;
    }
    body.split(',')
        .map(|x| x.trim().parse::<f32>().ok())
        .collect()
}

/// Expand one `property_key='facet'` row's JSONB object into the `(path, value)` facet entries the
/// clustering needs. Multi-key objects yield one entry per key; an array value yields one entry per
/// element, all sharing the row weight. Non-string scalars are skipped (not part of M1's affinity model).
fn expand_facets(owner: ResourceId, value: &serde_json::Value, weight: f64) -> Vec<Facet> {
    let Some(obj) = value.as_object() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (path, v) in obj {
        match v {
            serde_json::Value::String(s) => out.push(Facet {
                owner,
                path: path.clone(),
                value: s.clone(),
                weight,
            }),
            serde_json::Value::Array(items) => {
                for item in items {
                    if let Some(s) = item.as_str() {
                        out.push(Facet {
                            owner,
                            path: path.clone(),
                            value: s.to_string(),
                            weight,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expand_facets_handles_scalar_multikey_and_array() {
        let o = ResourceId::from(uuid::Uuid::nil());
        // single-key scalar (the onboarding seed shape) — unchanged behavior
        let f = expand_facets(o, &serde_json::json!({ "phase": "first-week" }), 1.0);
        assert_eq!(f.len(), 1);
        assert_eq!(
            (f[0].path.as_str(), f[0].value.as_str()),
            ("phase", "first-week")
        );
        // multi-key
        assert_eq!(
            expand_facets(
                o,
                &serde_json::json!({ "phase": "first-week", "topic": "deployment" }),
                1.0
            )
            .len(),
            2
        );
        // array value expands per element, sharing row weight
        let f = expand_facets(
            o,
            &serde_json::json!({ "topic": ["deployment", "release"] }),
            1.5,
        );
        assert_eq!(f.len(), 2);
        assert!(f.iter().all(|x| x.path == "topic" && x.weight == 1.5));
    }

    #[test]
    fn edge_kind_from_sql_is_exhaustive_and_rejects_unknown() {
        assert_eq!(EdgeKind::from_sql("near"), Some(EdgeKind::Near));
        assert_eq!(EdgeKind::from_sql("leads_to"), Some(EdgeKind::LeadsTo));
        assert_eq!(EdgeKind::from_sql("sideways"), None);
    }
}
