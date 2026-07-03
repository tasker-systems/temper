//! SQL-level semantics for the R2 territory-overview functions
//! (`graph_region_territories`, `graph_context_territories`,
//! `graph_orphan_salient_nodes`, `graph_territory_bridges`). Proves the new
//! functions directly against the migrated schema: region projection under a
//! lens, context aggregation, sparsity-fallback orphan surfacing with NO
//! INNER-JOIN erasure of a doc-type-less resource, and cross-territory bridge
//! aggregation — before the HTTP endpoint is exercised (that is
//! `graph_territory_overview_e2e.rs`).
#![cfg(feature = "test-db")]

mod common;

use uuid::Uuid;

/// Insert a profile with the given handle, return its id.
async fn mk_profile(pool: &sqlx::PgPool, handle: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .expect("insert profile")
}

async fn create_team(pool: &sqlx::PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
        .bind(slug)
        .fetch_one(pool)
        .await
        .expect("create team")
}

async fn add_member(pool: &sqlx::PgPool, team: Uuid, profile: Uuid) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(team)
    .bind(profile)
    .execute(pool)
    .await
    .expect("add member");
}

async fn create_resource(pool: &sqlx::PgPool, title: &str, origin: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_resources (title, origin_uri) VALUES ($1, $2) RETURNING id")
        .bind(title)
        .bind(origin)
        .fetch_one(pool)
        .await
        .expect("insert resource")
}

async fn set_doc_type(pool: &sqlx::PgPool, resource: Uuid, doc_type: &str, event: Uuid) {
    sqlx::query(
        "INSERT INTO kb_properties \
             (owner_table, owner_id, property_key, property_value, asserted_by_event_id, last_event_id) \
         VALUES ('kb_resources', $1, 'doc_type', to_jsonb($2::text), $3, $3)",
    )
    .bind(resource)
    .bind(doc_type)
    .bind(event)
    .execute(pool)
    .await
    .expect("set doc_type");
}

/// A context owned by a team — passes `resources_in_team_scope`'s team-owned-context branch.
async fn create_team_context(pool: &sqlx::PgPool, team: Uuid, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_teams', $1, $2, $2) RETURNING id",
    )
    .bind(team)
    .bind(slug)
    .fetch_one(pool)
    .await
    .expect("create team context")
}

/// Home a resource, with `profile` as BOTH originator and owner — this puts the
/// resource in `resources_visible_to(profile)` via the owned/originated branch
/// regardless of the anchor, which is what `endpoint_readable_by_profile` needs
/// for edge endpoints (independent of the edge's own home-anchor readability).
async fn home_resource(
    pool: &sqlx::PgPool,
    resource: Uuid,
    anchor_table: &str,
    anchor_id: Uuid,
    profile: Uuid,
) {
    sqlx::query(
        "INSERT INTO kb_resource_homes \
             (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, $2, $3, $4, $4)",
    )
    .bind(resource)
    .bind(anchor_table)
    .bind(anchor_id)
    .bind(profile)
    .execute(pool)
    .await
    .expect("home resource");
}

async fn assert_edge(
    pool: &sqlx::PgPool,
    source: Uuid,
    target: Uuid,
    edge_kind: &str,
    home_anchor_table: &str,
    home_anchor_id: Uuid,
    event: Uuid,
) {
    sqlx::query(
        "INSERT INTO kb_edges \
             (source_table, source_id, target_table, target_id, edge_kind, \
              home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id) \
         VALUES ('kb_resources', $1, 'kb_resources', $2, $3::edge_kind, $4, $5, $6, $6)",
    )
    .bind(source)
    .bind(target)
    .bind(edge_kind)
    .bind(home_anchor_table)
    .bind(home_anchor_id)
    .bind(event)
    .execute(pool)
    .await
    .expect("assert edge");
}

/// Any pre-existing kb_events row (the L0 kernel cogmap genesis migration inserts
/// one) — sufficient FK target for asserted_by_event_id/last_event_id in these tests.
async fn any_event(pool: &sqlx::PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_events LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("at least one kb_events row exists (L0 genesis)")
}

/// The global `telos-default` lens (cogmap_id IS NULL) — seeded by canonical_seed.
async fn telos_default_lens(pool: &sqlx::PgPool) -> Uuid {
    sqlx::query_scalar(
        "SELECT id FROM kb_cogmap_lenses WHERE name = 'telos-default' AND cogmap_id IS NULL LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("telos-default lens seeded by canonical_seed")
}

async fn create_cogmap(pool: &sqlx::PgPool, name: &str, telos_resource: Uuid) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(name)
    .bind(telos_resource)
    .fetch_one(pool)
    .await
    .expect("create cogmap")
}

async fn join_cogmap_team(pool: &sqlx::PgPool, cogmap: Uuid, team: Uuid) {
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap)
        .bind(team)
        .execute(pool)
        .await
        .expect("join cogmap to team");
}

/// 768-dim zero pgvector text literal — determinism of the region row's
/// centroid does not matter for these functions (they don't cosine-rank).
fn zero_vec768() -> String {
    let v = vec!["0"; 768];
    format!("[{}]", v.join(","))
}

async fn insert_region(
    pool: &sqlx::PgPool,
    cogmap: Uuid,
    lens: Uuid,
    label: &str,
    member_count: i32,
    salience: f64,
    event: Uuid,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_cogmap_regions \
             (cogmap_id, lens_id, centroid, salience, label, member_count, asserted_by_event_id, last_event_id) \
         VALUES ($1, $2, $3::vector, $4, $5, $6, $7, $7) RETURNING id",
    )
    .bind(cogmap)
    .bind(lens)
    .bind(zero_vec768())
    .bind(salience)
    .bind(label)
    .bind(member_count)
    .bind(event)
    .fetch_one(pool)
    .await
    .expect("insert region")
}

/// One materialized region under a joined cogmap (A); a region-less joined
/// cogmap (B) whose high-degree, doc-type-less resource must surface as an
/// orphan; a context territory; and a cross-cogmap edge that must aggregate
/// into exactly one bridge row.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn territory_functions_project_regions_orphans_contexts_and_bridges(pool: sqlx::PgPool) {
    let profile = mk_profile(&pool, "gto-tester").await;
    let team = create_team(&pool, "gto-team").await;
    add_member(&pool, team, profile).await;
    let event = any_event(&pool).await;
    let lens = telos_default_lens(&pool).await;

    // Cogmap A — materialized region under the telos-default lens.
    let telos_a = create_resource(&pool, "telos a", "temper://gto/telos-a").await;
    let cogmap_a = create_cogmap(&pool, "cogmap-a", telos_a).await;
    join_cogmap_team(&pool, cogmap_a, team).await;
    let region_a = insert_region(&pool, cogmap_a, lens, "Region A", 3, 0.7, event).await;

    // A resource homed in cogmap A (for the bridge edge below). Its region
    // membership doesn't matter to these functions — only its home anchor does.
    let cross_target = create_resource(&pool, "cross target", "temper://gto/cross-target").await;
    home_resource(&pool, cross_target, "kb_cogmaps", cogmap_a, profile).await;

    // Cogmap B — joined, region-LESS. Its high-degree, doc-type-less resource
    // must surface as an orphan with doc_type IS NULL (no INNER-JOIN erasure).
    let telos_b = create_resource(&pool, "telos b", "temper://gto/telos-b").await;
    let cogmap_b = create_cogmap(&pool, "cogmap-b", telos_b).await;
    join_cogmap_team(&pool, cogmap_b, team).await;

    let orphan = create_resource(&pool, "orphan node", "temper://gto/orphan").await;
    home_resource(&pool, orphan, "kb_cogmaps", cogmap_b, profile).await;
    // Deliberately no kb_properties doc_type row on `orphan`.

    let neighbor1 = create_resource(&pool, "neighbor 1", "temper://gto/n1").await;
    home_resource(&pool, neighbor1, "kb_cogmaps", cogmap_b, profile).await;
    set_doc_type(&pool, neighbor1, "concept", event).await;

    let neighbor2 = create_resource(&pool, "neighbor 2", "temper://gto/n2").await;
    home_resource(&pool, neighbor2, "kb_cogmaps", cogmap_b, profile).await;
    set_doc_type(&pool, neighbor2, "concept", event).await;

    // orphan's degree = 3 (n1, n2, cross_target); n1/n2's degree = 1 each.
    assert_edge(
        &pool,
        orphan,
        neighbor1,
        "near",
        "kb_cogmaps",
        cogmap_b,
        event,
    )
    .await;
    assert_edge(
        &pool,
        orphan,
        neighbor2,
        "near",
        "kb_cogmaps",
        cogmap_b,
        event,
    )
    .await;
    // The cross-cogmap edge — homed in cogmap B, endpoints in A and B. This is
    // the one edge that must aggregate into a territory bridge.
    assert_edge(
        &pool,
        orphan,
        cross_target,
        "near",
        "kb_cogmaps",
        cogmap_b,
        event,
    )
    .await;

    // A context territory: one resource homed in a team-owned context.
    let ctx = create_team_context(&pool, team, "gto-ctx").await;
    let ctx_res = create_resource(&pool, "context resource", "temper://gto/ctx-res").await;
    home_resource(&pool, ctx_res, "kb_contexts", ctx, profile).await;

    // ── graph_region_territories ────────────────────────────────────────
    let regions: Vec<(Uuid, Uuid, Option<String>, i32, f64)> = sqlx::query_as(
        "SELECT region_id, cogmap_id, label, member_count, salience FROM graph_region_territories($1, $2, $3)",
    )
    .bind(profile)
    .bind(team)
    .bind(lens)
    .fetch_all(&pool)
    .await
    .expect("graph_region_territories");
    assert_eq!(
        regions.len(),
        1,
        "exactly the one materialized region: {regions:?}"
    );
    assert_eq!(
        regions[0],
        (region_a, cogmap_a, Some("Region A".to_string()), 3, 0.7)
    );

    // ── graph_context_territories ───────────────────────────────────────
    let contexts: Vec<(Uuid, String, i32)> = sqlx::query_as(
        "SELECT context_id, label, member_count FROM graph_context_territories($1, $2)",
    )
    .bind(profile)
    .bind(team)
    .fetch_all(&pool)
    .await
    .expect("graph_context_territories");
    assert_eq!(
        contexts.len(),
        1,
        "exactly the one context territory: {contexts:?}"
    );
    assert_eq!(contexts[0], (ctx, "gto-ctx".to_string(), 1));

    // ── graph_orphan_salient_nodes ──────────────────────────────────────
    let orphans: Vec<(Uuid, String, Option<String>, i32, Uuid)> = sqlx::query_as(
        "SELECT id, title, doc_type, degree, anchor_id FROM graph_orphan_salient_nodes($1, $2)",
    )
    .bind(profile)
    .bind(team)
    .fetch_all(&pool)
    .await
    .expect("graph_orphan_salient_nodes");
    // cross_target is homed in cogmap A, which HAS a region — must never appear here.
    assert!(
        orphans.iter().all(|(id, ..)| *id != cross_target),
        "a resource homed in a region-bearing cogmap is never an orphan: {orphans:?}"
    );
    let orphan_row = orphans
        .iter()
        .find(|(id, ..)| *id == orphan)
        .expect("orphan present in region-less cogmap's orphan set");
    assert_eq!(
        orphan_row.2, None,
        "doc-type-less orphan projects doc_type = NULL, not erased by an INNER JOIN: {orphan_row:?}"
    );
    assert_eq!(orphan_row.3, 3, "orphan degree = 3 (n1, n2, cross_target)");
    assert_eq!(orphan_row.4, cogmap_b, "anchor_id is orphan's home cogmap");
    assert_eq!(
        orphans[0].0, orphan,
        "highest-degree row (orphan, degree 3) sorts first: {orphans:?}"
    );

    // ── graph_territory_bridges ─────────────────────────────────────────
    let bridges: Vec<(Uuid, Uuid, i32)> = sqlx::query_as(
        "SELECT source_territory, target_territory, edge_count FROM graph_territory_bridges($1, $2)",
    )
    .bind(profile)
    .bind(team)
    .fetch_all(&pool)
    .await
    .expect("graph_territory_bridges");
    assert_eq!(
        bridges.len(),
        1,
        "exactly one cross-cogmap bridge: {bridges:?}"
    );
    let (lo, hi) = (cogmap_a.min(cogmap_b), cogmap_a.max(cogmap_b));
    assert_eq!(
        bridges[0],
        (lo, hi, 1),
        "the orphan-to-cross_target edge is the one cross-territory bridge, counted once"
    );
}
