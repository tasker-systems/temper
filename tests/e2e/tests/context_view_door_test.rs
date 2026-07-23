//! HTTP e2e for the Beat E context door (GET /api/graph/contexts/{panorama,composition}).
//! A reader sees the context interior — goal-container territories with member counts and the
//! residual tray — and drills a container to its members; a stranger gets 404 (deny-as-absence,
//! never a 403 that would confirm the context exists).
#![cfg(feature = "test-db")]

mod common;

use uuid::Uuid;

// Helpers mirror graph_atlas_home_e2e.rs / graph_cogmap_panorama_e2e.rs (integration test
// binaries don't share code except via `common`, so these are copied rather than imported).
// The seed shape mirrors `seed_context_with_goal_and_tasks` in the temper-api integration
// `common` module, adapted to home the context under an *already-provisioned, JWT-reachable*
// profile so the same identity that owns the data also carries the token that reads it.

/// Resolve (auto-provisioning on first call) the profile id behind a bearer token.
async fn provision_profile(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("profile request failed");
    // D11: a fresh principal is born Denied. Approve so this actor clears the front door
    // and the ENDPOINT authz (ownership, admin-only, grants) is what the test exercises.
    let __pid: Uuid = resp.json::<serde_json::Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    common::approve(&app.pool, __pid).await;
    __pid
}

/// A profile-owned context plus the one genesis event its seeded resources/edges hang their
/// `asserted_by_event_id` / `last_event_id` FKs on. Returns `(context_id, event_id)`.
///
/// A profile-owned context makes every seeded resource visible to `owner` (via
/// `kb_resource_homes.owner_profile_id`) AND makes the context's edges readable (via
/// `anchor_readable_by_profile`'s personal-context clause), satisfying the full canonical
/// edge-visibility predicate the reads gate on.
async fn seed_context_owned_by(pool: &sqlx::PgPool, owner: Uuid) -> (Uuid, Uuid) {
    let ctx = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1, 'kb_profiles', $2, $3, $3)",
    )
    .bind(ctx)
    .bind(owner)
    .bind(format!("ctx-{ctx}"))
    .execute(pool)
    .await
    .expect("insert context");

    let entity = Uuid::now_v7();
    sqlx::query("INSERT INTO kb_entities (id, profile_id, name) VALUES ($1, $2, $3)")
        .bind(entity)
        .bind(owner)
        .bind(format!("owner-{entity}@web"))
        .execute(pool)
        .await
        .expect("insert entity");

    let event = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_events \
             (id, event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id) \
         SELECT $1, (SELECT id FROM kb_event_types WHERE name = 'relationship_asserted'), \
                $2, 'kb_contexts', $3",
    )
    .bind(event)
    .bind(entity)
    .bind(ctx)
    .execute(pool)
    .await
    .expect("insert genesis event");

    (ctx, event)
}

/// Insert one resource homed in `ctx` (owned + originated by `owner`) carrying a `doc_type`
/// property. Returns the resource id.
async fn seed_resource(
    pool: &sqlx::PgPool,
    ctx: Uuid,
    owner: Uuid,
    event: Uuid,
    title: &str,
    doc_type: &str,
) -> Uuid {
    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_resources (id, title, origin_uri, is_active) VALUES ($1, $2, $3, true)",
    )
    .bind(id)
    .bind(title)
    .bind(format!("test://{id}"))
    .execute(pool)
    .await
    .expect("insert resource");
    sqlx::query(
        "INSERT INTO kb_resource_homes \
             (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(id)
    .bind(ctx)
    .bind(owner)
    .execute(pool)
    .await
    .expect("insert home");
    sqlx::query(
        "INSERT INTO kb_properties \
             (owner_table, owner_id, property_key, property_value, asserted_by_event_id, last_event_id) \
         VALUES ('kb_resources', $1, 'doc_type', to_jsonb($2::text), $3, $3)",
    )
    .bind(id)
    .bind(doc_type)
    .bind(event)
    .execute(pool)
    .await
    .expect("insert doc_type property");
    id
}

/// `source --parent_of--> target`: the historical containment spine (edge_kind `contains`,
/// forward), homed in the context. Container walks filter on neither label nor direction, so a
/// count is invariant across the later `parent_of` -> `advances` conversion.
async fn seed_contains_edge(
    pool: &sqlx::PgPool,
    source: Uuid,
    target: Uuid,
    ctx: Uuid,
    event: Uuid,
) {
    sqlx::query(
        "INSERT INTO kb_edges \
             (source_table, source_id, target_table, target_id, edge_kind, polarity, label, \
              home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id) \
         VALUES ('kb_resources', $1, 'kb_resources', $2, 'contains', 'forward', 'parent_of', \
                 'kb_contexts', $3, $4, $4)",
    )
    .bind(source)
    .bind(target)
    .bind(ctx)
    .bind(event)
    .execute(pool)
    .await
    .expect("insert parent_of edge");
}

/// Seed a context owned by `owner` holding one `goal` and `n` `task` resources, each linked
/// `goal --parent_of--> task`. Returns `(context_id, goal_id)`.
async fn seed_goal_with_tasks(pool: &sqlx::PgPool, owner: Uuid, n: usize) -> (Uuid, Uuid) {
    let (ctx, event) = seed_context_owned_by(pool, owner).await;
    let goal = seed_resource(pool, ctx, owner, event, "Goal", "goal").await;
    for i in 0..n {
        let task = seed_resource(pool, ctx, owner, event, &format!("Task {i}"), "task").await;
        seed_contains_edge(pool, goal, task, ctx, event).await;
    }
    (ctx, goal)
}

/// Seed a context owned by `owner` holding a two-hop containment chain
/// `goal --parent_of--> task --parent_of--> session`. Returns `(context_id, session_id)`.
/// The session sits two hops from the goal, so it is *residual* at container-walk depth 1 and
/// *contained* at depth 2 — the difference that makes `container_depth` observable on the wire.
async fn seed_two_hop_chain(pool: &sqlx::PgPool, owner: Uuid) -> (Uuid, Uuid) {
    let (ctx, event) = seed_context_owned_by(pool, owner).await;
    let goal = seed_resource(pool, ctx, owner, event, "Goal", "goal").await;
    let task = seed_resource(pool, ctx, owner, event, "Task", "task").await;
    let session = seed_resource(pool, ctx, owner, event, "Session", "session").await;
    seed_contains_edge(pool, goal, task, ctx, event).await;
    seed_contains_edge(pool, task, session, ctx, event).await;
    (ctx, session)
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn panorama_returns_containers_and_residual_tray(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let owner = provision_profile(&app, &app.token).await;
    let (ctx, goal) = seed_goal_with_tasks(&pool, owner, 3).await;

    let body: temper_core::types::graph_context::ContextPanorama = app
        .reqwest_client
        .get(app.url(&format!(
            "/api/graph/contexts/panorama?context_ref={ctx}&group_by=doc_type"
        )))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .expect("panorama body decodes = renderer-compatible");

    assert_eq!(body.containers.len(), 1, "one goal container");
    assert_eq!(body.containers[0].id, goal);
    assert_eq!(
        body.containers[0].member_count, 3,
        "the goal container counts its three tasks"
    );

    // Residual tray shape: well-edged data (every task reaches the goal) leaves the tray empty —
    // buckets is an empty array, never null, so the tray shrinks to nothing without a special case.
    assert_eq!(body.residual.group_key, "doc_type");
    assert!(
        body.residual.buckets.is_empty(),
        "well-edged context has no residual buckets, got {:?}",
        body.residual.buckets
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn panorama_denies_stranger_as_absence(pool: sqlx::PgPool) {
    // Deny-as-absence: a fully-provisioned but unrelated profile asking for a context it cannot
    // see gets 404, NEVER a 403 that would confirm the context exists. This assertion is the
    // load-bearing one — it must fail if the handler ever leaks existence via 403.
    let app = common::setup(pool.clone()).await;
    let owner = provision_profile(&app, &app.token).await;
    let (ctx, _goal) = seed_goal_with_tasks(&pool, owner, 3).await;

    // A second identity, provisioned (via /api/profile) but with no relationship to the context.
    let stranger_token = common::generate_test_jwt("e2e-stranger", "stranger@test.example.com");
    let _stranger = provision_profile(&app, &stranger_token).await;

    let status = app
        .reqwest_client
        .get(app.url(&format!(
            "/api/graph/contexts/panorama?context_ref={ctx}&group_by=doc_type"
        )))
        .header("Authorization", format!("Bearer {stranger_token}"))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(
        status,
        reqwest::StatusCode::NOT_FOUND,
        "invisible context is absent (404), not forbidden (403)"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn container_drill_returns_container_and_members(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let owner = provision_profile(&app, &app.token).await;
    let (ctx, goal) = seed_goal_with_tasks(&pool, owner, 3).await;

    let sg: temper_core::types::graph_atlas::AtlasSubgraph = app
        .reqwest_client
        .get(app.url(&format!(
            "/api/graph/contexts/composition?context_ref={ctx}&container={goal}&depth=1"
        )))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .expect("composition body");

    assert_eq!(sg.nodes.len(), 4, "goal + 3 tasks");
    assert!(
        sg.nodes.iter().any(|n| n.id == goal),
        "the drilled container itself is present"
    );
    assert_eq!(sg.edges.len(), 3, "the three containment edges");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn bucket_drill_honors_container_depth_over_the_wire(pool: sqlx::PgPool) {
    // The bucket path carries the most wire surface: the `group=<key>:<value>` colon-split and
    // the separate `container_depth` that resolves which resources count as already-contained.
    // At container_depth=1 the two-hop session is residual, so drilling `doc_type:session` seeds
    // with it and the composition renders. Absent the parameter this would walk at the default
    // depth 2, find the session contained, and seed with nothing.
    let app = common::setup(pool.clone()).await;
    let owner = provision_profile(&app, &app.token).await;
    let (ctx, session) = seed_two_hop_chain(&pool, owner).await;

    let sg: temper_core::types::graph_atlas::AtlasSubgraph = app
        .reqwest_client
        .get(app.url(&format!(
            "/api/graph/contexts/composition?context_ref={ctx}&group=doc_type:session&container_depth=1"
        )))
        .header("Authorization", format!("Bearer {}", app.token))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .expect("composition body");

    assert!(
        sg.nodes.iter().any(|n| n.id == session),
        "the depth-1 residual session must seed the drill, got {:?}",
        sg.nodes.iter().map(|n| n.id).collect::<Vec<_>>()
    );
}
