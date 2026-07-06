//! SQL-level semantics for the R4 team-scoped Atlas neighborhood-slice functions
//! (`graph_traverse_scoped`, `graph_atlas_nodes`). Proves the new functions directly
//! against the migrated schema: depth clamp, edge-kind filter, no INNER-JOIN erasure
//! of doc-type-less nodes, home resolution, and team-scope exclusion — before the
//! HTTP endpoint is exercised (that is `graph_atlas_slice_e2e.rs`).
#![cfg(feature = "test-db")]

mod common;

use temper_core::types::graph::EdgeKind;
use temper_core::types::graph_atlas::SliceRequest;
use temper_core::types::ids::ProfileId;
use temper_services::services::graph_service::neighborhood_slice;
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

/// Team-anchored read grant (the mechanism `resources_in_team_scope` reads),
/// on the live `kb_access_grants` store (subject = resource, principal = team).
async fn grant_read_to_team(pool: &sqlx::PgPool, resource: Uuid, team: Uuid, granted_by: Uuid) {
    sqlx::query(
        "INSERT INTO kb_access_grants \
             (subject_table, subject_id, principal_table, principal_id, can_read, granted_by_profile_id) \
         VALUES ('kb_resources', $1, 'kb_teams', $2, true, $3)",
    )
    .bind(resource)
    .bind(team)
    .bind(granted_by)
    .execute(pool)
    .await
    .expect("grant read to team");
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

/// A context owned by a team — real (FK-backed by `owner_id`) so it passes
/// `anchor_readable_by_profile`'s "context OWNED by a team the principal is a
/// member of" branch, which `edges_visible_to` (hence node `degree`) depends on.
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

/// Seed a single current body chunk for a resource: a `kb_content_blocks` row,
/// a `kb_chunks` row over it, and its text in `kb_chunk_content`. This is the
/// minimal shape `graph_atlas_nodes`' `first_chunk` subquery reads (`ch.is_current
/// AND NOT b.is_folded`, ordered by `b.seq, ch.chunk_index`) — single block/chunk
/// at seq/index 0 is sufficient for a "first chunk" test; `content_hash` is a
/// placeholder since no query in this suite re-derives it.
async fn seed_first_chunk(pool: &sqlx::PgPool, resource: Uuid, content: &str, event: Uuid) {
    let block: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id) \
         VALUES ($1, 0, $2, $2) RETURNING id",
    )
    .bind(resource)
    .bind(event)
    .fetch_one(pool)
    .await
    .expect("insert content block");

    let chunk: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash) \
         VALUES ($1, $2, 0, 'test-hash') RETURNING id",
    )
    .bind(block)
    .bind(resource)
    .fetch_one(pool)
    .await
    .expect("insert chunk");

    sqlx::query("INSERT INTO kb_chunk_content (chunk_id, content) VALUES ($1, $2)")
        .bind(chunk)
        .bind(content)
        .execute(pool)
        .await
        .expect("insert chunk content");
}

/// `graph_traverse_scoped` walks only within the team scope, respects the depth
/// clamp, and the `p_edge_kinds` filter excludes an edge of an excluded kind
/// from being walked (both at the seed arm and the recursive arm).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn traverse_scoped_respects_depth_and_edge_kind_filter(pool: sqlx::PgPool) {
    let profile = mk_profile(&pool, "gas-walker").await;
    let team = create_team(&pool, "gas-team").await;
    add_member(&pool, team, profile).await;
    let event = any_event(&pool).await;

    // Edges are homed in a real team-owned context the member can read, so they
    // pass graph_traverse_scoped's anchor_readable_by_profile gate.
    let ctx = create_team_context(&pool, team, "gas-ctx").await;

    let r1 = create_resource(&pool, "seed", "temper://gas/r1").await;
    let r2 = create_resource(&pool, "hop-1", "temper://gas/r2").await;
    let r3 = create_resource(&pool, "hop-2", "temper://gas/r3").await;
    let r4 = create_resource(&pool, "excluded-kind-target", "temper://gas/r4").await;
    for r in [r1, r2, r3, r4] {
        grant_read_to_team(&pool, r, team, profile).await;
    }

    // r1 --contains--> r2 --leads_to--> r3   (both included kinds)
    // r1 --near--> r4                        (excluded kind for the filtered call)
    assert_edge(&pool, r1, r2, "contains", "kb_contexts", ctx, event).await;
    assert_edge(&pool, r2, r3, "leads_to", "kb_contexts", ctx, event).await;
    assert_edge(&pool, r1, r4, "near", "kb_contexts", ctx, event).await;

    // Depth 1, filtered to `contains` only: just r1->r2.
    let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT source_id, target_id FROM graph_traverse_scoped($1, $2, $3, $4, $5)",
    )
    .bind(profile)
    .bind(team)
    .bind(vec![r1])
    .bind(1_i32)
    .bind(vec![EdgeKind::Contains])
    .fetch_all(&pool)
    .await
    .expect("traverse depth 1, contains only");
    assert_eq!(rows, vec![(r1, r2)], "depth-1 contains-only walk = r1->r2");

    // Depth 2, filtered to contains+leads_to: r1->r2 and r2->r3, but NOT r1->r4 (excluded kind).
    let mut rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT source_id, target_id FROM graph_traverse_scoped($1, $2, $3, $4, $5)",
    )
    .bind(profile)
    .bind(team)
    .bind(vec![r1])
    .bind(2_i32)
    .bind(vec![EdgeKind::Contains, EdgeKind::LeadsTo])
    .fetch_all(&pool)
    .await
    .expect("traverse depth 2, contains+leads_to");
    rows.sort();
    let mut expected = vec![(r1, r2), (r2, r3)];
    expected.sort();
    assert_eq!(rows, expected, "excluded-kind edge r1->r4 is never walked");

    // Depth 1, no filter (empty array): both r1->r2 and r1->r4 are walked.
    let mut rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT source_id, target_id FROM graph_traverse_scoped($1, $2, $3, $4, $5)",
    )
    .bind(profile)
    .bind(team)
    .bind(vec![r1])
    .bind(1_i32)
    .bind(Vec::<EdgeKind>::new())
    .fetch_all(&pool)
    .await
    .expect("traverse depth 1, unfiltered");
    rows.sort();
    let mut expected = vec![(r1, r2), (r1, r4)];
    expected.sort();
    assert_eq!(rows, expected, "empty edge_kinds filter = all kinds");
}

/// `graph_atlas_nodes` projects a doc-type-less node with `doc_type IS NULL` (no
/// INNER-JOIN erasure), resolves `home` correctly (cogmap wins over context),
/// and excludes a resource outside the team's `resources_in_team_scope`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn atlas_nodes_preserves_null_doc_type_and_excludes_out_of_scope(pool: sqlx::PgPool) {
    let profile = mk_profile(&pool, "gas-nodes").await;
    let team = create_team(&pool, "gas-nodes-team").await;
    add_member(&pool, team, profile).await;
    let event = any_event(&pool).await;

    // A real team-owned context — used as the edge's home anchor so
    // `edges_visible_to` (hence `degree`) actually counts it. The cogmap anchor
    // for `typed`'s home row stays synthetic: graph_atlas_nodes' home LATERAL
    // only cares about anchor_table, not FK/readability of the anchor id.
    let ctx = create_team_context(&pool, team, "gas-nodes-ctx").await;
    let cogmap_anchor = Uuid::now_v7();

    let typed = create_resource(&pool, "typed node", "temper://gas/typed").await;
    let untyped = create_resource(&pool, "untyped node", "temper://gas/untyped").await;
    let outside = create_resource(&pool, "outside scope", "temper://gas/outside").await;

    grant_read_to_team(&pool, typed, team, profile).await;
    grant_read_to_team(&pool, untyped, team, profile).await;
    // `outside` is deliberately NOT granted to the team — stays out of scope.

    set_doc_type(&pool, typed, "concept", event).await;
    // `untyped` gets no kb_properties row at all.

    home_resource(&pool, typed, "kb_cogmaps", cogmap_anchor, profile).await;
    home_resource(&pool, untyped, "kb_contexts", ctx, profile).await;

    // One edge, homed in the real team context, so degree is nonzero and provable.
    assert_edge(&pool, typed, untyped, "near", "kb_contexts", ctx, event).await;

    let ids = vec![typed, untyped, outside];
    let rows: Vec<(Uuid, String, Option<String>, String, i32)> = sqlx::query_as(
        "SELECT id, title, doc_type, home, degree FROM graph_atlas_nodes($1, $2, $3)",
    )
    .bind(profile)
    .bind(team)
    .bind(ids)
    .fetch_all(&pool)
    .await
    .expect("graph_atlas_nodes");

    assert_eq!(
        rows.len(),
        2,
        "outside-scope resource is excluded, not just null-projected: {rows:?}"
    );
    let by_id = |id: Uuid| rows.iter().find(|r| r.0 == id).cloned();

    let typed_row = by_id(typed).expect("typed node present");
    assert_eq!(typed_row.2, Some("concept".to_string()));
    assert_eq!(typed_row.3, "cogmap", "cogmap-homed resolves to cogmap");
    assert_eq!(typed_row.4, 1, "typed participates in one edge");

    let untyped_row = by_id(untyped).expect("untyped node present");
    assert_eq!(
        untyped_row.2, None,
        "doc-type-less resource projects doc_type = NULL, not erased by an INNER JOIN"
    );
    assert_eq!(
        untyped_row.3, "context",
        "context-homed resolves to context"
    );
    assert_eq!(untyped_row.4, 1, "untyped participates in one edge");

    assert!(
        by_id(outside).is_none(),
        "a resource outside resources_in_team_scope is excluded entirely"
    );
}

/// I1 regression: with multiple seeds, the same edge can be reached at two
/// different depths (e.g. directly from one seed, and via a hop from another).
/// The recursive `walk` CTE's `UNION` dedups on the full row *including*
/// `depth`, so before the fix such an edge survived as two output rows once
/// `depth` was dropped. `graph_traverse_scoped` must return each distinct
/// `(source_id, target_id, edge_kind)` at most once.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn traverse_scoped_dedupes_edge_reached_at_multiple_depths(pool: sqlx::PgPool) {
    let profile = mk_profile(&pool, "gas-multi-seed").await;
    let team = create_team(&pool, "gas-multi-seed-team").await;
    add_member(&pool, team, profile).await;
    let event = any_event(&pool).await;

    let ctx = create_team_context(&pool, team, "gas-multi-ctx").await;

    let a = create_resource(&pool, "a", "temper://gas/a").await;
    let b = create_resource(&pool, "b", "temper://gas/b").await;
    let c = create_resource(&pool, "c", "temper://gas/c").await;
    for r in [a, b, c] {
        grant_read_to_team(&pool, r, team, profile).await;
    }

    // a --contains--> b   (reachable directly from seed a at depth 1)
    // c --contains--> a   (reachable directly from seed c at depth 1)
    // So a->b is ALSO reachable via c->a->b at depth 2 — the same edge at two depths.
    assert_edge(&pool, a, b, "contains", "kb_contexts", ctx, event).await;
    assert_edge(&pool, c, a, "contains", "kb_contexts", ctx, event).await;

    let rows: Vec<(Uuid, Uuid, EdgeKind)> = sqlx::query_as(
        "SELECT source_id, target_id, edge_kind FROM graph_traverse_scoped($1, $2, $3, $4, $5)",
    )
    .bind(profile)
    .bind(team)
    .bind(vec![a, c])
    .bind(2_i32)
    .bind(Vec::<EdgeKind>::new())
    .fetch_all(&pool)
    .await
    .expect("traverse with multiple seeds");

    let mut seen = std::collections::HashSet::new();
    for row in &rows {
        assert!(
            seen.insert(*row),
            "duplicate (source_id, target_id, edge_kind) row: {row:?} in {rows:?}"
        );
    }

    let expected: std::collections::HashSet<(Uuid, Uuid, EdgeKind)> =
        [(a, b, EdgeKind::Contains), (c, a, EdgeKind::Contains)]
            .into_iter()
            .collect();
    assert_eq!(
        seen, expected,
        "each edge appears exactly once, regardless of how many depths reach it"
    );
}

/// Security regression: `graph_traverse_scoped` must NOT walk an edge whose own
/// home anchor is unreadable to the profile, even when BOTH endpoints are in the
/// team scope. The full `edges_visible_to` gate requires
/// `anchor_readable_by_profile(home)` — not just endpoint visibility. This is the
/// "private edge between two public resources" leak; guard it directly.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn traverse_scoped_excludes_edge_with_unreadable_home(pool: sqlx::PgPool) {
    let profile = mk_profile(&pool, "gas-deny").await;
    let team = create_team(&pool, "gas-deny-team").await;
    add_member(&pool, team, profile).await;
    let event = any_event(&pool).await;

    let s = create_resource(&pool, "src", "temper://gas/deny-s").await;
    let t = create_resource(&pool, "tgt", "temper://gas/deny-t").await;
    // Both endpoints ARE in the team scope — so ONLY the home-anchor gate can exclude the edge.
    grant_read_to_team(&pool, s, team, profile).await;
    grant_read_to_team(&pool, t, team, profile).await;

    // Homed in a cogmap the profile cannot read (random id, joined to none of the
    // profile's teams) → anchor_readable_by_profile is false.
    let private_home = Uuid::now_v7();
    assert_edge(&pool, s, t, "contains", "kb_cogmaps", private_home, event).await;

    let rows: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT source_id, target_id FROM graph_traverse_scoped($1, $2, $3, $4, $5)",
    )
    .bind(profile)
    .bind(team)
    .bind(vec![s])
    .bind(2_i32)
    .bind(Vec::<EdgeKind>::new())
    .fetch_all(&pool)
    .await
    .expect("traverse with an unreadable-home edge");

    assert!(
        rows.is_empty(),
        "edge homed in an unreadable anchor is excluded despite visible endpoints: {rows:?}"
    );
}

/// Beat 2b N1: `neighborhood_slice`'s node projection carries a body excerpt
/// derived from the resource's first body chunk (via `compute_excerpt`), and
/// `None` for a resource with no body chunks at all. Exercises the full
/// service function (not just the raw SQL functions above) since the excerpt
/// derivation happens in the Rust mapping layer, not in `graph_atlas_nodes` itself.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn neighborhood_node_carries_body_excerpt(pool: sqlx::PgPool) {
    let profile = mk_profile(&pool, "gas-excerpt").await;
    let team = create_team(&pool, "gas-excerpt-team").await;
    add_member(&pool, team, profile).await;
    let event = any_event(&pool).await;

    let ctx = create_team_context(&pool, team, "gas-excerpt-ctx").await;

    let with_body = create_resource(&pool, "has body", "temper://gas/with-body").await;
    let no_body = create_resource(&pool, "no body", "temper://gas/no-body").await;
    for r in [with_body, no_body] {
        grant_read_to_team(&pool, r, team, profile).await;
    }
    // An edge between them so both land in the depth-1 induced subgraph from
    // the `with_body` seed.
    assert_edge(&pool, with_body, no_body, "near", "kb_contexts", ctx, event).await;

    let body = "First paragraph of the resource body, long enough to read as a \
                real excerpt once collapsed to a single line.\n\n\
                Second paragraph that must NOT appear in the excerpt.";
    seed_first_chunk(&pool, with_body, body, event).await;
    // `no_body` gets no kb_content_blocks/kb_chunks rows at all.

    let sub = neighborhood_slice(
        &pool,
        ProfileId::from(profile),
        team,
        SliceRequest {
            seeds: vec![with_body],
            depth: 1,
            edge_kinds: vec![],
        },
    )
    .await
    .expect("neighborhood_slice");

    let n_body = sub
        .nodes
        .iter()
        .find(|n| n.id == with_body)
        .expect("with_body node present in the slice");
    assert!(
        n_body
            .excerpt
            .as_deref()
            .expect("with_body node has an excerpt")
            .starts_with("First paragraph"),
        "excerpt should be derived from the first body chunk's first paragraph: {:?}",
        n_body.excerpt
    );

    let n_bare = sub
        .nodes
        .iter()
        .find(|n| n.id == no_body)
        .expect("no_body node present in the slice");
    assert_eq!(
        n_bare.excerpt, None,
        "a resource with no body chunks projects excerpt = None"
    );
}
