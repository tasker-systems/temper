use crate::affinity::{Edge, EdgeKind, Facet, Lens};
use crate::ids::{CogmapId, ContextId, LensId, ProfileId, ResourceId};
use anyhow::{Context, Result};
use sqlx::PgPool;
use temper_core::types::home::HomeAnchor;

#[derive(Debug)]
pub struct Substrate {
    pub nodes: Vec<ResourceId>,
    pub edges: Vec<Edge>,
    pub facets: Vec<Facet>,
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

    // resources homed in the anchor
    let nodes: Vec<ResourceId> = sqlx::query_scalar!(
        "SELECT resource_id FROM kb_resource_homes WHERE anchor_table=$1 AND anchor_id=$2",
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

    // the named lens for this anchor, else the global default (home_anchor_table IS NULL — how
    // telos-default and telos-default-propheavy are seeded). The name is bound (Plan 3 Step 0) so the
    // same producer materializes different lenses over one substrate — S6f plurality.
    //
    // `NULLS LAST` keeps an anchor-specific lens winning over the global default, exactly as
    // `ORDER BY cogmap_id NULLS LAST` did. (T4 extends this SELECT with w_cos / knn_k / cos_floor —
    // the columns exist as of T2 but nothing reads them until the kernel lands.)
    let lr = sqlx::query!(
        "SELECT id, w_express, w_contains, w_leads_to, w_near, w_prop, s_telos, s_ref, s_central, resolution \
         FROM kb_cogmap_lenses \
         WHERE name=$3 AND (home_anchor_table IS NULL OR (home_anchor_table=$1 AND home_anchor_id=$2)) \
         ORDER BY home_anchor_table NULLS LAST LIMIT 1",
        anchor_table,
        anchor_id,
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
        s_telos: lr.s_telos,
        s_ref: lr.s_ref,
        s_central: lr.s_central,
        resolution: lr.resolution,
    };
    Ok(Substrate {
        nodes,
        edges,
        facets,
        lens,
        lens_id: LensId::from(lr.id),
    })
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
