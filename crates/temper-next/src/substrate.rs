use crate::affinity::{Edge, EdgeKind, Facet, Lens};
use anyhow::Result;
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use uuid::Uuid;

pub struct Substrate {
    pub nodes: Vec<Uuid>,
    pub edges: Vec<Edge>,
    pub facets: Vec<Facet>,
    pub lens: Lens,
    pub lens_id: Uuid,
}

pub async fn connect() -> Result<PgPool> {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://temper:temper@localhost:5437/temper_development".into());
    let pool = PgPoolOptions::new()
        .after_connect(|c, _| {
            Box::pin(async move {
                sqlx::query("SET search_path = temper_next, public")
                    .execute(c)
                    .await
                    .map(|_| ())
            })
        })
        .connect(&url)
        .await?;
    Ok(pool)
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
        .map(|r| Edge {
            src: r.get("source_id"),
            tgt: r.get("target_id"),
            kind: parse_kind(r.get::<String, _>("kind")),
            weight: r.get("weight"),
            label: r.get("label"),
        })
        .collect();

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
        .filter_map(|r| {
            let v: serde_json::Value = r.get("property_value");
            let (path, value) = v.as_object()?.iter().next()?;
            Some(Facet {
                owner: r.get("owner_id"),
                path: path.clone(),
                value: value.as_str()?.to_string(),
                weight: r.get("weight"),
            })
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

fn parse_kind(s: String) -> EdgeKind {
    match s.as_str() {
        "express" => EdgeKind::Express,
        "contains" => EdgeKind::Contains,
        "leads_to" => EdgeKind::LeadsTo,
        _ => EdgeKind::Near,
    }
}
