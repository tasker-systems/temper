//! Beat D — SQL + service semantics for the region → resources COMPOSITION read
//! (`graph_region_composition_edges`, `graph_atlas_nodes_visible`,
//! `region_composition_slice`). The load-bearing property: the walk is seeded by
//! region members (facets) and follows *visible* edges out to context-homed
//! resources (the builder axis) — cross-home, NOT cogmap-fenced — while gating
//! every endpoint conjunct-for-conjunct through `resources_visible_to`. Proven
//! here before the HTTP endpoint / SvelteKit load are wired.
#![cfg(feature = "test-db")]

mod common;

use temper_core::types::graph::EdgeKind;
use temper_core::types::graph_atlas::NodeHome;
use temper_core::types::ids::ProfileId;
use temper_services::services::graph_service::region_composition_slice;
use uuid::Uuid;

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

async fn create_cogmap(pool: &sqlx::PgPool, name: &str, telos: Uuid) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(name)
    .bind(telos)
    .fetch_one(pool)
    .await
    .expect("create cogmap")
}

async fn join_cogmap(pool: &sqlx::PgPool, cogmap: Uuid, team: Uuid) {
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap)
        .bind(team)
        .execute(pool)
        .await
        .expect("join cogmap");
}

/// Home a resource in a cogmap (owner = `profile`). Visible to `profile` (owned
/// clause) AND to any member of a team the cogmap is joined to (cogmap-membership
/// clause of `resources_visible_to`) — the lever that makes a facet visible to
/// two profiles at once.
async fn home_in_cogmap(pool: &sqlx::PgPool, resource: Uuid, cogmap: Uuid, owner: Uuid) {
    sqlx::query(
        "INSERT INTO kb_resource_homes \
             (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_cogmaps', $2, $3, $3)",
    )
    .bind(resource)
    .bind(cogmap)
    .bind(owner)
    .execute(pool)
    .await
    .expect("home in cogmap");
}

async fn create_context(pool: &sqlx::PgPool, slug: &str, owner_profile: Uuid) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_profiles', $1, $2, $2) RETURNING id",
    )
    .bind(owner_profile)
    .bind(slug)
    .fetch_one(pool)
    .await
    .expect("create context")
}

/// Home a resource in a context, owned by `owner`. Visible to `owner` (owned home
/// clause) but — for a personal context not shared to any team — to nobody else.
async fn home_in_context(pool: &sqlx::PgPool, resource: Uuid, context: Uuid, owner: Uuid) {
    sqlx::query(
        "INSERT INTO kb_resource_homes \
             (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(resource)
    .bind(context)
    .bind(owner)
    .execute(pool)
    .await
    .expect("home in context");
}

fn zero_vec768() -> String {
    format!("[{}]", vec!["0"; 768].join(","))
}

async fn telos_default_lens(pool: &sqlx::PgPool) -> Uuid {
    sqlx::query_scalar(
        "SELECT id FROM kb_cogmap_lenses WHERE name = 'telos-default' AND cogmap_id IS NULL LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("telos-default lens")
}

async fn insert_region(
    pool: &sqlx::PgPool,
    cogmap: Uuid,
    lens: Uuid,
    label: &str,
    event: Uuid,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_cogmap_regions \
             (cogmap_id, lens_id, centroid, salience, label, member_count, asserted_by_event_id, last_event_id) \
         VALUES ($1, $2, $3::vector, 0.6, $4, 1, $5, $5) RETURNING id",
    )
    .bind(cogmap)
    .bind(lens)
    .bind(zero_vec768())
    .bind(label)
    .bind(event)
    .fetch_one(pool)
    .await
    .expect("insert region")
}

async fn add_region_member(pool: &sqlx::PgPool, region: Uuid, member: Uuid) {
    sqlx::query(
        "INSERT INTO kb_cogmap_region_members (region_id, member_table, member_id, affinity) \
         VALUES ($1, 'kb_resources', $2, 0.9)",
    )
    .bind(region)
    .bind(member)
    .execute(pool)
    .await
    .expect("add region member");
}

async fn assert_edge(
    pool: &sqlx::PgPool,
    source: Uuid,
    target: Uuid,
    anchor: Uuid,
    event: Uuid,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_edges \
             (source_table, source_id, target_table, target_id, edge_kind, \
              home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id) \
         VALUES ('kb_resources', $1, 'kb_resources', $2, $3, 'kb_cogmaps', $4, $5, $5) RETURNING id",
    )
    .bind(source)
    .bind(target)
    .bind(EdgeKind::Express)
    .bind(anchor)
    .bind(event)
    .fetch_one(pool)
    .await
    .expect("assert edge")
}

async fn any_event(pool: &sqlx::PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_events LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("an event row exists (L0 genesis)")
}

/// The shared build: team T with two members A and B, cogmap C joined to T (both
/// can read C), region R with facet F homed in C, and a context-homed resource X
/// (owned by A only) that F is `derived_from`. Returns (a, b, region, facet, ctx_doc).
async fn build_cross_home(pool: &sqlx::PgPool) -> (Uuid, Uuid, Uuid, Uuid, Uuid) {
    let a = mk_profile(pool, "grc-a").await;
    let b = mk_profile(pool, "grc-b").await;
    let event = any_event(pool).await;
    let lens = telos_default_lens(pool).await;

    let telos = create_resource(pool, "telos", "temper://grc/telos").await;
    let cogmap = create_cogmap(pool, "grc-cogmap", telos).await;
    let team = create_team(pool, "grc-team").await;
    add_member(pool, team, a).await;
    add_member(pool, team, b).await;
    join_cogmap(pool, cogmap, team).await;

    let region = insert_region(pool, cogmap, lens, "Region GRC", event).await;
    let facet = create_resource(pool, "facet F (idea)", "temper://grc/facet").await;
    home_in_cogmap(pool, facet, cogmap, a).await;
    add_region_member(pool, region, facet).await;

    // Context-homed work-product, visible to A only.
    let ctx = create_context(pool, "grc-personal-a", a).await;
    let ctx_doc = create_resource(pool, "session X (the work)", "temper://grc/x").await;
    home_in_context(pool, ctx_doc, ctx, a).await;

    // F --derived_from--> X, edge homed in the cogmap (readable by both A and B).
    assert_edge(pool, facet, ctx_doc, cogmap, event).await;

    (a, b, region, facet, ctx_doc)
}

/// The builder axis: A (who can see the context doc) gets the facet→context edge,
/// and the two nodes carry the right home (cogmap vs context = circle vs square).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn composition_surfaces_cross_home_builder_axis(pool: sqlx::PgPool) {
    let (a, _b, region, facet, ctx_doc) = build_cross_home(&pool).await;

    let edges: Vec<(Uuid, Uuid, Uuid)> = sqlx::query_as(
        "SELECT id, source_id, target_id FROM graph_region_composition_edges($1, $2, $3)",
    )
    .bind(a)
    .bind(vec![region])
    .bind(1_i32)
    .fetch_all(&pool)
    .await
    .expect("composition edges");
    assert!(
        edges.iter().any(|(_, s, t)| *s == facet && *t == ctx_doc),
        "the facet→context derived_from edge must surface for the owner: {edges:?}"
    );

    let nodes: Vec<(Uuid, String)> =
        sqlx::query_as("SELECT id, home FROM graph_atlas_nodes_visible($1, $2)")
            .bind(a)
            .bind(vec![facet, ctx_doc])
            .fetch_all(&pool)
            .await
            .expect("nodes");
    let home_of = |id: Uuid| {
        nodes
            .iter()
            .find(|(nid, _)| *nid == id)
            .map(|(_, h)| h.clone())
    };
    assert_eq!(
        home_of(facet).as_deref(),
        Some("cogmap"),
        "facet is an idea"
    );
    assert_eq!(
        home_of(ctx_doc).as_deref(),
        Some("context"),
        "the work is a document"
    );
}

/// Deny direction: B can read the cogmap and the facet, but NOT the context doc
/// (owned by A, in A's personal context). The composition must return the facet
/// and NOT the invisible neighbor, with no dangling edge to it.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn composition_denies_invisible_context_neighbor(pool: sqlx::PgPool) {
    let (_a, b, region, facet, ctx_doc) = build_cross_home(&pool).await;

    let edges: Vec<(Uuid, Uuid, Uuid)> = sqlx::query_as(
        "SELECT id, source_id, target_id FROM graph_region_composition_edges($1, $2, $3)",
    )
    .bind(b)
    .bind(vec![region])
    .bind(1_i32)
    .fetch_all(&pool)
    .await
    .expect("composition edges (denied caller)");
    assert!(
        !edges.iter().any(|(_, _, t)| *t == ctx_doc),
        "no edge to the invisible context doc may leak to B: {edges:?}"
    );

    let nodes: Vec<(Uuid,)> = sqlx::query_as("SELECT id FROM graph_atlas_nodes_visible($1, $2)")
        .bind(b)
        .bind(vec![facet, ctx_doc])
        .fetch_all(&pool)
        .await
        .expect("nodes (denied caller)");
    let ids: Vec<Uuid> = nodes.into_iter().map(|(id,)| id).collect();
    assert!(
        ids.contains(&facet),
        "B still sees the facet (readable cogmap)"
    );
    assert!(
        !ids.contains(&ctx_doc),
        "B must NOT see the context-owned doc: {ids:?}"
    );
}

/// Union: seeding from two regions returns both facets' builder edges.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn composition_unions_multiple_regions(pool: sqlx::PgPool) {
    let (a, _b, region1, facet1, ctx1) = build_cross_home(&pool).await;
    let event = any_event(&pool).await;
    let lens = telos_default_lens(&pool).await;

    // A second region in the SAME cogmap with its own facet→context edge.
    let cogmap: Uuid = sqlx::query_scalar(
        "SELECT anchor_id FROM kb_resource_homes WHERE resource_id = $1 AND anchor_table='kb_cogmaps'",
    )
    .bind(facet1)
    .fetch_one(&pool)
    .await
    .expect("facet1 cogmap");
    let region2 = insert_region(&pool, cogmap, lens, "Region GRC 2", event).await;
    let facet2 = create_resource(&pool, "facet F2", "temper://grc/facet2").await;
    home_in_cogmap(&pool, facet2, cogmap, a).await;
    add_region_member(&pool, region2, facet2).await;
    let ctx2ctx = create_context(&pool, "grc-personal-a2", a).await;
    let ctx2 = create_resource(&pool, "session X2", "temper://grc/x2").await;
    home_in_context(&pool, ctx2, ctx2ctx, a).await;
    assert_edge(&pool, facet2, ctx2, cogmap, event).await;

    let edges: Vec<(Uuid, Uuid, Uuid)> = sqlx::query_as(
        "SELECT id, source_id, target_id FROM graph_region_composition_edges($1, $2, $3)",
    )
    .bind(a)
    .bind(vec![region1, region2])
    .bind(1_i32)
    .fetch_all(&pool)
    .await
    .expect("union composition edges");
    assert!(
        edges.iter().any(|(_, s, t)| *s == facet1 && *t == ctx1),
        "region1 builder edge present"
    );
    assert!(
        edges.iter().any(|(_, s, t)| *s == facet2 && *t == ctx2),
        "region2 builder edge present"
    );
}

/// Service happy-path: the two-axis subgraph carries both nodes with their homes.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn service_returns_two_axis_subgraph(pool: sqlx::PgPool) {
    let (a, _b, region, facet, ctx_doc) = build_cross_home(&pool).await;

    let sub = region_composition_slice(&pool, ProfileId::from(a), &[region], 1)
        .await
        .expect("composition slice");
    assert!(sub
        .nodes
        .iter()
        .any(|n| n.id == facet && n.home == NodeHome::Cogmap));
    assert!(sub
        .nodes
        .iter()
        .any(|n| n.id == ctx_doc && n.home == NodeHome::Context));
    assert!(sub
        .edges
        .iter()
        .any(|e| e.source == facet && e.target == ctx_doc));
}

/// Service entry gate (deny-as-absence): a caller who cannot read the region's
/// cogmap gets NotFound, never a partial subgraph.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn service_entry_gate_denies_unreadable_region(pool: sqlx::PgPool) {
    let (_a, _b, region, _facet, _ctx) = build_cross_home(&pool).await;
    let stranger = mk_profile(&pool, "grc-stranger").await; // not a member of the team

    let err = region_composition_slice(&pool, ProfileId::from(stranger), &[region], 1)
        .await
        .expect_err("stranger must be denied");
    assert!(
        matches!(err, temper_services::error::ApiError::NotFound),
        "unreadable region → NotFound, got {err:?}"
    );
}
