//! Equivalence guard for the set-based `edges_visible_to` rewrite (SQL function audit
//! 2026-07-08, chunk 6). The original body applied three per-row scalar gates —
//! `anchor_readable_by_profile` + `endpoint_readable_by_profile` on both endpoints — which
//! re-ran the full `resources_visible_to` subselect per edge (hidden N+1 feeding the
//! graph_atlas degree laterals). The rewrite materializes the visible/readable sets once and
//! semi-joins; the scalar helpers remain in the schema (invocation readback still gates
//! through `anchor_readable_by_profile`), so they double as the per-row ORACLE here: the
//! function must return exactly the edge set the scalar gates admit, for every principal.
//!
//! The fixture covers every branch class the audit called out: partial-visibility profiles;
//! cogmap anchors readable by team-join and by explicit grant; all four readable-context
//! anchor arms (personal-owned, shared-to-team, team-owned, explicit grant) plus an
//! unreadable one; resource AND cogmap (non-kb_resources) endpoints; folded edges; an
//! invisible endpoint; a soft-deleted endpoint (the chunk-2 is_active floor); and an
//! unknown endpoint table (the CASE ELSE false arm).
#![cfg(feature = "test-db")]

mod common;

use std::collections::HashSet;

use sqlx::PgPool;
use uuid::Uuid;

async fn any_event(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_events LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("bootstrap event for FK")
}

async fn mk_team(pool: &PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
        .bind(format!("{slug}-{}", Uuid::new_v4()))
        .fetch_one(pool)
        .await
        .expect("insert team")
}

async fn add_member(pool: &PgPool, team: Uuid, profile: Uuid) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(team)
    .bind(profile)
    .execute(pool)
    .await
    .expect("insert membership");
}

async fn mk_resource(pool: &PgPool, title: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_resources (title, origin_uri) VALUES ($1, $2) RETURNING id")
        .bind(title)
        .bind(format!("test://evt-eq/{}", Uuid::new_v4()))
        .fetch_one(pool)
        .await
        .expect("insert resource")
}

async fn mk_cogmap(pool: &PgPool, name: &str) -> Uuid {
    let telos = mk_resource(pool, &format!("{name}-telos")).await;
    sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(name)
    .bind(telos)
    .fetch_one(pool)
    .await
    .expect("insert cogmap")
}

async fn mk_context(pool: &PgPool, owner_table: &str, owner_id: Uuid, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) VALUES ($1, $2, $3, $3) RETURNING id",
    )
    .bind(owner_table)
    .bind(owner_id)
    .bind(format!("{slug}-{}", Uuid::new_v4()))
    .fetch_one(pool)
    .await
    .expect("insert context")
}

async fn home(pool: &PgPool, resource: Uuid, anchor_table: &str, anchor_id: Uuid, owner: Uuid) {
    sqlx::query(
        "INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id)
         VALUES ($1, $2, $3, $4, $4)",
    )
    .bind(resource)
    .bind(anchor_table)
    .bind(anchor_id)
    .bind(owner)
    .execute(pool)
    .await
    .expect("insert home");
}

async fn grant_read(
    pool: &PgPool,
    subject_table: &str,
    subject_id: Uuid,
    principal_table: &str,
    principal_id: Uuid,
    granted_by: Uuid,
) {
    sqlx::query(
        "INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id, granted_by_profile_id, can_read)
         VALUES ($1, $2, $3, $4, $5, true)",
    )
    .bind(subject_table)
    .bind(subject_id)
    .bind(principal_table)
    .bind(principal_id)
    .bind(granted_by)
    .execute(pool)
    .await
    .expect("insert read grant");
}

#[expect(
    clippy::too_many_arguments,
    reason = "test fixture — the params ARE the edge tuple under test"
)]
async fn mk_edge(
    pool: &PgPool,
    event: Uuid,
    src_t: &str,
    src: Uuid,
    tgt_t: &str,
    tgt: Uuid,
    home_t: &str,
    home_id: Uuid,
    folded: bool,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_edges
           (source_table, source_id, target_table, target_id, edge_kind,
            home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id, is_folded)
         VALUES ($1, $2, $3, $4, 'express', $5, $6, $7, $7, $8)
         RETURNING id",
    )
    .bind(src_t)
    .bind(src)
    .bind(tgt_t)
    .bind(tgt)
    .bind(home_t)
    .bind(home_id)
    .bind(event)
    .bind(folded)
    .fetch_one(pool)
    .await
    .expect("insert edge")
}

/// The per-row spec: the scalar gates the original function body applied, verbatim.
async fn oracle_edges(pool: &PgPool, profile: Uuid) -> HashSet<Uuid> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT e.id FROM kb_edges e
          WHERE NOT e.is_folded
            AND anchor_readable_by_profile($1, e.home_anchor_table, e.home_anchor_id)
            AND endpoint_readable_by_profile($1, e.source_table, e.source_id)
            AND endpoint_readable_by_profile($1, e.target_table, e.target_id)",
    )
    .bind(profile)
    .fetch_all(pool)
    .await
    .expect("oracle query")
    .into_iter()
    .collect()
}

async fn function_edges(pool: &PgPool, profile: Uuid) -> HashSet<Uuid> {
    sqlx::query_scalar::<_, Uuid>("SELECT edge_id FROM edges_visible_to($1)")
        .bind(profile)
        .fetch_all(pool)
        .await
        .expect("edges_visible_to query")
        .into_iter()
        .collect()
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn edges_visible_to_matches_per_row_oracle(pool: PgPool) {
    let viewer =
        common::fixtures::create_test_profile(&pool, &format!("viewer-{}@eq.test", Uuid::new_v4()))
            .await;
    let other =
        common::fixtures::create_test_profile(&pool, &format!("other-{}@eq.test", Uuid::new_v4()))
            .await;
    let event = any_event(&pool).await;

    // Teams: viewer ∈ team_a, other ∈ team_b.
    let team_a = mk_team(&pool, "eq-team-a").await;
    let team_b = mk_team(&pool, "eq-team-b").await;
    add_member(&pool, team_a, viewer).await;
    add_member(&pool, team_b, other).await;

    // Cogmap anchors: m1 readable by viewer (team join), m2 not (team_b only),
    // m3 readable by viewer via explicit cogmap read-grant.
    let m1 = mk_cogmap(&pool, "eq-m1").await;
    let m2 = mk_cogmap(&pool, "eq-m2").await;
    let m3 = mk_cogmap(&pool, "eq-m3").await;
    for (team, map) in [(team_a, m1), (team_b, m2)] {
        sqlx::query("INSERT INTO kb_team_cogmaps (team_id, cogmap_id) VALUES ($1, $2)")
            .bind(team)
            .bind(map)
            .execute(&pool)
            .await
            .expect("join cogmap to team");
    }
    grant_read(&pool, "kb_cogmaps", m3, "kb_profiles", viewer, other).await;

    // Context anchors: every readable arm of anchor_readable_by_profile's kb_contexts CASE,
    // plus one unreadable context.
    let c_personal = mk_context(&pool, "kb_profiles", viewer, "eq-personal").await;
    let c_shared = mk_context(&pool, "kb_profiles", other, "eq-shared").await;
    sqlx::query("INSERT INTO kb_team_contexts (team_id, context_id) VALUES ($1, $2)")
        .bind(team_a)
        .bind(c_shared)
        .execute(&pool)
        .await
        .expect("share context to team");
    let c_teamowned = mk_context(&pool, "kb_teams", team_a, "eq-teamowned").await;
    let c_granted = mk_context(&pool, "kb_profiles", other, "eq-granted").await;
    grant_read(
        &pool,
        "kb_contexts",
        c_granted,
        "kb_profiles",
        viewer,
        other,
    )
    .await;
    let c_hidden = mk_context(&pool, "kb_profiles", other, "eq-hidden").await;

    // Endpoint resources: visible (viewer-owned), invisible (other's, unshared), granted
    // (other's + resource read-grant), and soft-deleted (viewer-owned, is_active=false —
    // the chunk-2 READ floor must hide edges touching it).
    let r_vis = mk_resource(&pool, "eq-r-vis").await;
    home(&pool, r_vis, "kb_contexts", c_personal, viewer).await;
    let r_inv = mk_resource(&pool, "eq-r-inv").await;
    home(&pool, r_inv, "kb_contexts", c_hidden, other).await;
    let r_granted = mk_resource(&pool, "eq-r-granted").await;
    home(&pool, r_granted, "kb_contexts", c_hidden, other).await;
    grant_read(
        &pool,
        "kb_resources",
        r_granted,
        "kb_profiles",
        viewer,
        other,
    )
    .await;
    let r_del = mk_resource(&pool, "eq-r-del").await;
    home(&pool, r_del, "kb_contexts", c_personal, viewer).await;
    sqlx::query("UPDATE kb_resources SET is_active = false WHERE id = $1")
        .bind(r_del)
        .execute(&pool)
        .await
        .expect("soft-delete r_del");

    // Edges, one per branch class. Expected visibility (for viewer) in the comments.
    let e_ok_cogmap = mk_edge(
        &pool,
        event,
        "kb_resources",
        r_vis,
        "kb_resources",
        r_granted,
        "kb_cogmaps",
        m1,
        false,
    )
    .await; // visible
    let e_bad_target = mk_edge(
        &pool,
        event,
        "kb_resources",
        r_vis,
        "kb_resources",
        r_inv,
        "kb_cogmaps",
        m1,
        false,
    )
    .await; // target invisible
    let e_bad_anchor = mk_edge(
        &pool,
        event,
        "kb_resources",
        r_vis,
        "kb_resources",
        r_granted,
        "kb_cogmaps",
        m2,
        false,
    )
    .await; // anchor unreadable
    let e_ok_granted_map = mk_edge(
        &pool,
        event,
        "kb_resources",
        r_vis,
        "kb_resources",
        r_granted,
        "kb_cogmaps",
        m3,
        false,
    )
    .await; // visible (grant arm)
    let e_ok_personal = mk_edge(
        &pool,
        event,
        "kb_resources",
        r_vis,
        "kb_resources",
        r_granted,
        "kb_contexts",
        c_personal,
        false,
    )
    .await; // visible
    let e_ok_shared = mk_edge(
        &pool,
        event,
        "kb_resources",
        r_vis,
        "kb_resources",
        r_granted,
        "kb_contexts",
        c_shared,
        false,
    )
    .await; // visible
    let e_ok_teamowned = mk_edge(
        &pool,
        event,
        "kb_resources",
        r_vis,
        "kb_resources",
        r_granted,
        "kb_contexts",
        c_teamowned,
        false,
    )
    .await; // visible
    let e_ok_ctx_grant = mk_edge(
        &pool,
        event,
        "kb_resources",
        r_vis,
        "kb_resources",
        r_granted,
        "kb_contexts",
        c_granted,
        false,
    )
    .await; // visible (grant arm)
    let e_hidden_ctx = mk_edge(
        &pool,
        event,
        "kb_resources",
        r_vis,
        "kb_resources",
        r_granted,
        "kb_contexts",
        c_hidden,
        false,
    )
    .await; // anchor unreadable
    let e_ok_map_endpoint = mk_edge(
        &pool,
        event,
        "kb_resources",
        r_vis,
        "kb_cogmaps",
        m1,
        "kb_cogmaps",
        m1,
        false,
    )
    .await; // visible (cogmap endpoint)
    let e_bad_map_endpoint = mk_edge(
        &pool,
        event,
        "kb_resources",
        r_vis,
        "kb_cogmaps",
        m2,
        "kb_cogmaps",
        m1,
        false,
    )
    .await; // cogmap endpoint unreadable
    let e_folded = mk_edge(
        &pool,
        event,
        "kb_resources",
        r_vis,
        "kb_resources",
        r_granted,
        "kb_cogmaps",
        m1,
        true,
    )
    .await; // folded
    let e_deleted_endpoint = mk_edge(
        &pool,
        event,
        "kb_resources",
        r_vis,
        "kb_resources",
        r_del,
        "kb_cogmaps",
        m1,
        false,
    )
    .await; // soft-deleted endpoint
            // (No unknown-endpoint-table edge: kb_edges_source_table_check forbids anything but
            // kb_resources/kb_cogmaps at the data layer, so the CASE ELSE false arm is unreachable.)

    let fixture: HashSet<Uuid> = [
        e_ok_cogmap,
        e_bad_target,
        e_bad_anchor,
        e_ok_granted_map,
        e_ok_personal,
        e_ok_shared,
        e_ok_teamowned,
        e_ok_ctx_grant,
        e_hidden_ctx,
        e_ok_map_endpoint,
        e_bad_map_endpoint,
        e_folded,
        e_deleted_endpoint,
    ]
    .into_iter()
    .collect();
    let expected_viewer: HashSet<Uuid> = [
        e_ok_cogmap,
        e_ok_granted_map,
        e_ok_personal,
        e_ok_shared,
        e_ok_teamowned,
        e_ok_ctx_grant,
        e_ok_map_endpoint,
    ]
    .into_iter()
    .collect();

    // The guard proper: function == per-row oracle, over the WHOLE edge table (fixture +
    // any migration-born edges), for every principal class.
    for (who, profile) in [("viewer", viewer), ("other", other)] {
        let via_fn = function_edges(&pool, profile).await;
        let via_oracle = oracle_edges(&pool, profile).await;
        assert_eq!(
            via_fn, via_oracle,
            "{who}: edges_visible_to must equal the per-row scalar-gate oracle\n  fn-only: {:?}\n  oracle-only: {:?}",
            via_fn.difference(&via_oracle).collect::<Vec<_>>(),
            via_oracle.difference(&via_fn).collect::<Vec<_>>(),
        );
    }

    // Hand-computed expectations pin the semantics (not just old==new).
    let viewer_fixture: HashSet<Uuid> = function_edges(&pool, viewer)
        .await
        .intersection(&fixture)
        .copied()
        .collect();
    assert_eq!(
        viewer_fixture,
        expected_viewer,
        "viewer's visible fixture edges\n  unexpected: {:?}\n  missing: {:?}",
        viewer_fixture
            .difference(&expected_viewer)
            .collect::<Vec<_>>(),
        expected_viewer
            .difference(&viewer_fixture)
            .collect::<Vec<_>>(),
    );

    // `other` cannot see any fixture edge: every edge touches r_vis (invisible to other) or
    // is folded — including e_bad_anchor whose anchor (m2) other CAN read.
    let other_fixture: HashSet<Uuid> = function_edges(&pool, other)
        .await
        .intersection(&fixture)
        .copied()
        .collect();
    assert!(
        other_fixture.is_empty(),
        "other must see no fixture edge (r_vis is invisible to other); got {other_fixture:?}"
    );
}
