use crate::affinity::{Edge, EdgeKind, Facet, Lens};
use anyhow::{Context, Result};
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Debug)]
pub struct Substrate {
    pub nodes: Vec<Uuid>,
    pub edges: Vec<Edge>,
    pub facets: Vec<Facet>,
    pub lens: Lens,
    pub lens_id: Uuid,
}

pub async fn connect() -> Result<PgPool> {
    // The connection's search_path is the database default (`public`) in production, dev, and
    // tests. No per-connection `SET search_path` is needed. In tests, ephemeral databases are
    // provided by `#[sqlx::test]` with `temper_substrate::MIGRATOR` applied to `public`.
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://temper:temper@localhost:5437/temper_development".into());
    Ok(PgPool::connect(&url).await?)
}

pub async fn cogmap_by_name(pool: &PgPool, name: &str) -> Result<Uuid> {
    let row = sqlx::query("SELECT id FROM kb_cogmaps WHERE name = $1")
        .bind(name)
        .fetch_one(pool)
        .await?;
    Ok(row.get::<Uuid, _>("id"))
}

pub async fn load(pool: &PgPool, cogmap: Uuid, lens_name: &str) -> Result<Substrate> {
    // concept-resources homed in the cogmap
    let node_rows = sqlx::query(
        "SELECT resource_id FROM kb_resource_homes WHERE anchor_table='kb_cogmaps' AND anchor_id=$1",
    )
    .bind(cogmap)
    .fetch_all(pool)
    .await?;
    let nodes: Vec<Uuid> = node_rows
        .iter()
        .map(|r| r.get::<Uuid, _>("resource_id"))
        .collect();

    // declared edges homed in the cogmap, both endpoints resources
    let edge_rows = sqlx::query(
        "SELECT source_id, target_id, edge_kind::text AS kind, label, weight \
         FROM kb_edges WHERE home_anchor_table='kb_cogmaps' AND home_anchor_id=$1 \
           AND source_table='kb_resources' AND target_table='kb_resources' AND NOT is_folded",
    )
    .bind(cogmap)
    .fetch_all(pool)
    .await?;
    let edges = edge_rows
        .iter()
        .map(|r| -> Result<Edge> {
            let kind_str: String = r.get("kind");
            let kind = EdgeKind::from_sql(&kind_str)
                .with_context(|| format!("unknown edge_kind from DB: {kind_str:?}"))?;
            Ok(Edge {
                src: r.get("source_id"),
                tgt: r.get("target_id"),
                kind,
                weight: r.get("weight"),
                label: r.get("label"),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    // facets on those resources (property_key='facet', value jsonb {path:value})
    let facet_rows = sqlx::query(
        "SELECT owner_id, property_value, weight FROM kb_properties \
         WHERE owner_table='kb_resources' AND property_key='facet' AND NOT is_folded \
           AND owner_id = ANY($1)",
    )
    .bind(&nodes)
    .fetch_all(pool)
    .await?;
    let facets = facet_rows
        .iter()
        .flat_map(|r| {
            let v: serde_json::Value = r.get("property_value");
            let weight: f64 = r.get("weight");
            expand_facets(r.get("owner_id"), &v, weight)
        })
        .collect();

    // the named lens for this cogmap (or the global default). The name is bound (Plan 3 Step 0) so
    // the same producer materializes different lenses (e.g. telos-default vs telos-default-propheavy)
    // over one substrate — S6f plurality.
    let lr = sqlx::query(
        "SELECT id, w_express, w_contains, w_leads_to, w_near, w_prop, s_telos, s_ref, s_central, resolution \
         FROM kb_cogmap_lenses WHERE name=$2 AND (cogmap_id=$1 OR cogmap_id IS NULL) \
         ORDER BY cogmap_id NULLS LAST LIMIT 1",
    )
    .bind(cogmap)
    .bind(lens_name)
    .fetch_one(pool)
    .await?;
    let lens = Lens {
        w_express: lr.get("w_express"),
        w_contains: lr.get("w_contains"),
        w_leads_to: lr.get("w_leads_to"),
        w_near: lr.get("w_near"),
        w_prop: lr.get("w_prop"),
        s_telos: lr.get("s_telos"),
        s_ref: lr.get("s_ref"),
        s_central: lr.get("s_central"),
        resolution: lr.get("resolution"),
    };
    Ok(Substrate {
        nodes,
        edges,
        facets,
        lens,
        lens_id: lr.get("id"),
    })
}

/// Expand one `property_key='facet'` row's JSONB object into the `(path, value)` facet entries the
/// clustering needs. Multi-key objects yield one entry per key; an array value yields one entry per
/// element, all sharing the row weight. Non-string scalars are skipped (not part of M1's affinity model).
fn expand_facets(owner: Uuid, value: &serde_json::Value, weight: f64) -> Vec<Facet> {
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
        let o = Uuid::nil();
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
