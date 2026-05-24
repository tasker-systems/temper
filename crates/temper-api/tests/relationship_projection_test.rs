#![cfg(feature = "test-db")]
//! apply_relationship_event projects edges; rebuild_edge_projection reproduces them.

mod common;

use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

use temper_api::services::relationship_service::{
    apply_relationship_event, rebuild_edge_projection, TOPIC_DECLARATION, TOPIC_DEFORMATION,
};
use temper_core::types::graph::{EdgeKind, Polarity};
use temper_core::types::relationship_events::{
    RelationshipAsserted, RelationshipFolded, RelationshipReweighted, TargetEndpoint,
};
use temper_events::ledger::append_event_tx;
use temper_events::types::event::{EventToWrite, EventType};

const PUBLIC_SCOPE_ID: &str = "019e3d6f-2300-7000-8000-000000000010";

fn declaration_topic_id() -> Uuid {
    Uuid::parse_str(TOPIC_DECLARATION).expect("TOPIC_DECLARATION parses")
}

fn deformation_topic_id() -> Uuid {
    Uuid::parse_str(TOPIC_DEFORMATION).expect("TOPIC_DEFORMATION parses")
}

fn public_scope_id() -> Uuid {
    Uuid::parse_str(PUBLIC_SCOPE_ID).expect("PUBLIC_SCOPE_ID parses")
}

/// Append a `relationship_asserted` root event and apply it as a projection.
/// Returns the event id (== correlation_id for root events).
async fn assert_and_project(
    pool: &PgPool,
    profile_id: Uuid,
    source_id: Uuid,
    target_id: Uuid,
    edge_kind: EdgeKind,
    polarity: Polarity,
    label: &str,
    weight: f64,
) -> Uuid {
    let payload = RelationshipAsserted {
        source_resource_id: source_id,
        target: TargetEndpoint::Resource(target_id),
        edge_kind,
        polarity,
        label: label.to_string(),
        weight,
    };
    let payload_value = serde_json::to_value(&payload).expect("serialize RelationshipAsserted");

    let mut write = EventToWrite::new_root(
        EventType::RelationshipAsserted,
        profile_id,
        declaration_topic_id(),
        public_scope_id(),
        payload_value,
        Utc::now(),
    );
    write.metadata = serde_json::json!({"intent": "fixture"});

    let mut tx = pool.begin().await.expect("begin tx");
    let event = append_event_tx(&mut tx, write)
        .await
        .expect("append_event_tx");
    let event_id = event.id;
    apply_relationship_event(&mut tx, &event, EventType::RelationshipAsserted)
        .await
        .expect("apply_relationship_event");
    tx.commit().await.expect("commit");
    event_id
}

// ─── Test 1: RelationshipAsserted projects an edge row ──────────────────────

/// Append a `relationship_asserted` event and verify one `kb_resource_edges`
/// row is created with the correct kind, source, target, and is_folded=false.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_asserted_projects_edge_row(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    let profile = common::fixtures::create_test_profile(&pool, "proj-assert@test.com").await;
    let a = common::fixtures::create_test_resource(&pool, profile, "Doc A", "proj-a").await;
    let b = common::fixtures::create_test_resource(&pool, profile, "Doc B", "proj-b").await;

    assert_and_project(
        &pool,
        profile,
        a,
        b,
        EdgeKind::LeadsTo,
        Polarity::Forward,
        "depends_on",
        1.0,
    )
    .await;

    let count: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM kb_resource_edges
            WHERE source_resource_id = $1
              AND target_resource_id = $2
              AND edge_kind = 'leads_to'
              AND NOT is_folded"#,
    )
    .bind(a)
    .bind(b)
    .fetch_one(&pool)
    .await
    .expect("count edge");

    assert_eq!(count, 1, "one active leads_to edge A→B expected");
}

// ─── Test 2: RelationshipReweighted updates weight ──────────────────────────

/// After asserting an edge, append a `relationship_reweighted` correlated
/// event and verify the weight and last_event_id are updated while
/// `asserted_by_event_id` is unchanged.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_reweighted_updates_weight(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    let profile = common::fixtures::create_test_profile(&pool, "proj-reweight@test.com").await;
    let a = common::fixtures::create_test_resource(&pool, profile, "Doc A", "rw-a").await;
    let b = common::fixtures::create_test_resource(&pool, profile, "Doc B", "rw-b").await;

    // Assert initial edge (weight 1.0); correlation_id == event_id for roots.
    let assertion_event_id = assert_and_project(
        &pool,
        profile,
        a,
        b,
        EdgeKind::LeadsTo,
        Polarity::Forward,
        "depends_on",
        1.0,
    )
    .await;

    // Capture the initial last_event_id.
    let initial_last: Uuid = sqlx::query_scalar(
        "SELECT last_event_id FROM kb_resource_edges WHERE asserted_by_event_id = $1",
    )
    .bind(assertion_event_id)
    .fetch_one(&pool)
    .await
    .expect("initial last_event_id");

    // Append reweighted event correlated to the assertion.
    let reweight_payload = RelationshipReweighted { weight: 2.5 };
    let reweight_payload_value =
        serde_json::to_value(&reweight_payload).expect("serialize RelationshipReweighted");

    let reweight_write = EventToWrite::new_correlated(
        EventType::RelationshipReweighted,
        profile,
        declaration_topic_id(),
        public_scope_id(),
        reweight_payload_value,
        assertion_event_id,
        Utc::now(),
    );

    let mut tx = pool.begin().await.expect("begin tx");
    let reweight_event = append_event_tx(&mut tx, reweight_write)
        .await
        .expect("append reweighted");
    apply_relationship_event(&mut tx, &reweight_event, EventType::RelationshipReweighted)
        .await
        .expect("apply reweighted");
    tx.commit().await.expect("commit");

    // Verify weight = 2.5, last_event_id bumped, asserted_by_event_id unchanged.
    let row = sqlx::query!(
        r#"
        SELECT weight                  AS "weight!: f64",
               last_event_id          AS "last_event_id!: Uuid",
               asserted_by_event_id   AS "asserted_by_event_id!: Uuid"
          FROM kb_resource_edges
         WHERE asserted_by_event_id = $1
        "#,
        assertion_event_id,
    )
    .fetch_one(&pool)
    .await
    .expect("edge row");

    assert!(
        (row.weight - 2.5).abs() < f64::EPSILON,
        "weight should be 2.5, got {}",
        row.weight
    );
    assert_ne!(
        row.last_event_id, initial_last,
        "last_event_id should be bumped"
    );
    assert_eq!(
        row.asserted_by_event_id, assertion_event_id,
        "asserted_by_event_id unchanged"
    );
}

// ─── Test 3: RelationshipFolded sets is_folded; graph_neighbors excludes it ─

/// After asserting an edge, fold it and verify is_folded=true. Also verify
/// the `graph_neighbors` SQL function no longer returns the folded peer.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_folded_sets_is_folded_and_excluded_from_neighbors(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    let profile = common::fixtures::create_test_profile(&pool, "proj-fold@test.com").await;
    let a = common::fixtures::create_test_resource(&pool, profile, "Doc A", "fold-a").await;
    let b = common::fixtures::create_test_resource(&pool, profile, "Doc B", "fold-b").await;

    let assertion_event_id = assert_and_project(
        &pool,
        profile,
        a,
        b,
        EdgeKind::LeadsTo,
        Polarity::Forward,
        "depends_on",
        1.0,
    )
    .await;

    // Fold the edge.
    let fold_payload = RelationshipFolded { reason: None };
    let fold_payload_value =
        serde_json::to_value(&fold_payload).expect("serialize RelationshipFolded");

    let fold_write = EventToWrite::new_correlated(
        EventType::RelationshipFolded,
        profile,
        deformation_topic_id(),
        public_scope_id(),
        fold_payload_value,
        assertion_event_id,
        Utc::now(),
    );

    let mut tx = pool.begin().await.expect("begin tx");
    let fold_event = append_event_tx(&mut tx, fold_write)
        .await
        .expect("append folded");
    apply_relationship_event(&mut tx, &fold_event, EventType::RelationshipFolded)
        .await
        .expect("apply folded");
    tx.commit().await.expect("commit");

    // Verify is_folded = true.
    let is_folded: bool = sqlx::query_scalar(
        "SELECT is_folded FROM kb_resource_edges WHERE asserted_by_event_id = $1",
    )
    .bind(assertion_event_id)
    .fetch_one(&pool)
    .await
    .expect("is_folded");

    assert!(is_folded, "edge should be folded");

    // Verify graph_neighbors excludes the folded edge.
    let neighbor_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM graph_neighbors($1, $2, 'outgoing', '{}')")
            .bind(profile)
            .bind(a)
            .fetch_one(&pool)
            .await
            .expect("graph_neighbors count");

    assert_eq!(
        neighbor_count, 0,
        "folded edge should not appear in graph_neighbors"
    );
}

// ─── Test 4: rebuild_edge_projection reproduces the same edge set ────────────

/// Build A→B (asserted), A→C (asserted + folded). Snapshot the edge set,
/// call `rebuild_edge_projection`, re-snapshot, and assert they are equivalent
/// (same rows modulo id/timestamps).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_rebuild_reproduces_projection(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    let profile = common::fixtures::create_test_profile(&pool, "proj-rebuild@test.com").await;
    let a = common::fixtures::create_test_resource(&pool, profile, "Doc A", "rb-a").await;
    let b = common::fixtures::create_test_resource(&pool, profile, "Doc B", "rb-b").await;
    let c = common::fixtures::create_test_resource(&pool, profile, "Doc C", "rb-c").await;

    // A→B active.
    assert_and_project(
        &pool,
        profile,
        a,
        b,
        EdgeKind::LeadsTo,
        Polarity::Forward,
        "depends_on",
        1.0,
    )
    .await;

    // A→C asserted then folded.
    let ac_assertion_id = assert_and_project(
        &pool,
        profile,
        a,
        c,
        EdgeKind::Near,
        Polarity::Forward,
        "relates_to",
        0.5,
    )
    .await;

    let fold_payload = RelationshipFolded { reason: None };
    let fold_payload_value = serde_json::to_value(&fold_payload).unwrap();
    let fold_write = EventToWrite::new_correlated(
        EventType::RelationshipFolded,
        profile,
        deformation_topic_id(),
        public_scope_id(),
        fold_payload_value,
        ac_assertion_id,
        Utc::now(),
    );
    let mut tx = pool.begin().await.expect("begin tx");
    let fold_event = append_event_tx(&mut tx, fold_write).await.unwrap();
    apply_relationship_event(&mut tx, &fold_event, EventType::RelationshipFolded)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // Snapshot before rebuild: (source, target, edge_kind, polarity, label, weight, is_folded).
    let snap_before = snapshot_edges(&pool).await;

    // Rebuild.
    let mut tx = pool.begin().await.expect("begin rebuild tx");
    rebuild_edge_projection(&mut tx).await.expect("rebuild");
    tx.commit().await.expect("commit rebuild");

    // Snapshot after rebuild.
    let snap_after = snapshot_edges(&pool).await;

    assert_eq!(
        snap_before.len(),
        snap_after.len(),
        "same number of edge rows after rebuild"
    );
    for (before, after) in snap_before.iter().zip(snap_after.iter()) {
        assert_eq!(before, after, "edge row mismatch after rebuild");
    }
}

/// Normalized edge snapshot: (source, target, kind, polarity, label, weight, is_folded).
/// Strips id and timestamps so the comparison is stable.
async fn snapshot_edges(pool: &PgPool) -> Vec<(Uuid, Uuid, String, String, String, i64, bool)> {
    let mut rows: Vec<(Uuid, Uuid, String, String, String, i64, bool)> = sqlx::query_as(
        r#"SELECT source_resource_id,
                  target_resource_id,
                  edge_kind::text,
                  polarity::text,
                  label,
                  (weight * 1000)::bigint,
                  is_folded
             FROM kb_resource_edges
            ORDER BY source_resource_id, target_resource_id, label"#,
    )
    .fetch_all(pool)
    .await
    .expect("snapshot_edges");
    rows.sort();
    rows
}

// ─── Test 5: Slug target that doesn't resolve projects no edge ───────────────

/// Append a `relationship_asserted` with a `Slug` target that doesn't exist.
/// `apply_relationship_event` should project NO edge row.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_unresolved_slug_projects_no_edge(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    let profile = common::fixtures::create_test_profile(&pool, "proj-slug@test.com").await;
    let a = common::fixtures::create_test_resource(&pool, profile, "Doc A", "slug-a").await;

    let payload = RelationshipAsserted {
        source_resource_id: a,
        target: TargetEndpoint::Slug("nonexistent-slug".to_string()),
        edge_kind: EdgeKind::LeadsTo,
        polarity: Polarity::Forward,
        label: "depends_on".to_string(),
        weight: 1.0,
    };
    let payload_value = serde_json::to_value(&payload).expect("serialize");

    let mut write = EventToWrite::new_root(
        EventType::RelationshipAsserted,
        profile,
        declaration_topic_id(),
        public_scope_id(),
        payload_value,
        Utc::now(),
    );
    write.metadata = serde_json::json!({"intent": "fixture"});

    let mut tx = pool.begin().await.expect("begin tx");
    let event = append_event_tx(&mut tx, write).await.expect("append event");
    apply_relationship_event(&mut tx, &event, EventType::RelationshipAsserted)
        .await
        .expect("apply — should not error on unresolved slug");
    tx.commit().await.expect("commit");

    // The event should be in the ledger.
    let event_count: i64 = sqlx::query_scalar(
        r#"SELECT count(*) FROM kb_events ev
            JOIN kb_event_types et ON et.id = ev.event_type_id
           WHERE et.name = 'relationship_asserted'
             AND (ev.payload->>'source_resource_id')::uuid = $1
             AND ev.payload->'target'->>'kind' = 'slug'
             AND ev.payload->'target'->>'value' = 'nonexistent-slug'"#,
    )
    .bind(a)
    .fetch_one(&pool)
    .await
    .expect("event count");
    assert_eq!(event_count, 1, "event appended to ledger");

    // No edge row should exist for this source.
    let edge_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_resource_edges WHERE source_resource_id = $1")
            .bind(a)
            .fetch_one(&pool)
            .await
            .expect("edge count");
    assert_eq!(edge_count, 0, "no edge projected for unresolved slug");
}

// ─── Test 6: slug resolution during rebuild is context-scoped ────────────────

/// Regression guard for Fix B: `apply_relationship_event` (the rebuild path)
/// must resolve slugs within the source resource's own context, not via
/// `resources_visible_to` (profile-visibility-scoped, broader).
///
/// Setup: Profile P owns two contexts CP1 and CP2. Source A is in CP1.
/// Target C lives in CP2 with slug "rbs-target". CP1 has NO resource with
/// slug "rbs-target".
///
/// We inject a `relationship_asserted` event with
/// `source = A, target = Slug("rbs-target")` (as the live-write path would
/// produce when the slug doesn't resolve in CP1).
///
/// Post-fix: rebuild must NOT project an edge A→C because "rbs-target" does
/// not exist in CP1 (the source's context). Pre-fix, the old
/// `resources_visible_to` path would have resolved the slug to C (in CP2)
/// and projected a spurious edge.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn rebuild_slug_resolution_is_context_scoped(pool: PgPool) {
    use temper_api::services::relationship_service::rebuild_edge_projection;

    common::fixtures::clean_and_seed(&pool).await;
    let profile = common::fixtures::create_test_profile(&pool, "proj-ctx-slug@test.com").await;

    // Create CP1 (source context).
    let cp1_id = uuid::Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO kb_contexts (id, name, kb_owner_table, kb_owner_id)
           VALUES ($1, 'cp1-rbs', 'kb_profiles', $2)"#,
    )
    .bind(cp1_id)
    .bind(profile)
    .execute(&pool)
    .await
    .expect("create CP1");

    // Create CP2 (target context).
    let cp2_id = uuid::Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO kb_contexts (id, name, kb_owner_table, kb_owner_id)
           VALUES ($1, 'cp2-rbs', 'kb_profiles', $2)"#,
    )
    .bind(cp2_id)
    .bind(profile)
    .execute(&pool)
    .await
    .expect("create CP2");

    // Resource A in CP1 (no slug "rbs-target" in CP1).
    let doc_type_id = uuid::Uuid::parse_str(common::fixtures::RESEARCH_DOC_TYPE_ID).unwrap();
    let a = uuid::Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO kb_resources
            (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
             originator_profile_id, owner_profile_id, is_active, created, updated)
           VALUES ($1, $2, $3, $4, 'Source A', 'rbs-src', $5, $5, true, now(), now())"#,
    )
    .bind(a)
    .bind(cp1_id)
    .bind(doc_type_id)
    .bind("test://rbs-src")
    .bind(profile)
    .execute(&pool)
    .await
    .expect("create A in CP1");

    // Resource C in CP2 with slug "rbs-target".
    let c = uuid::Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO kb_resources
            (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
             originator_profile_id, owner_profile_id, is_active, created, updated)
           VALUES ($1, $2, $3, $4, 'Target C', 'rbs-target', $5, $5, true, now(), now())"#,
    )
    .bind(c)
    .bind(cp2_id)
    .bind(doc_type_id)
    .bind("test://rbs-target")
    .bind(profile)
    .execute(&pool)
    .await
    .expect("create C in CP2");

    // Inject a `relationship_asserted` event with Slug target (as the live path
    // would produce when the slug didn't resolve in CP1).
    let payload = RelationshipAsserted {
        source_resource_id: a,
        target: TargetEndpoint::Slug("rbs-target".to_string()),
        edge_kind: EdgeKind::Near,
        polarity: Polarity::Forward,
        label: "relates_to".to_string(),
        weight: 1.0,
    };
    let payload_value = serde_json::to_value(&payload).expect("serialize");

    let mut write = EventToWrite::new_root(
        EventType::RelationshipAsserted,
        profile,
        declaration_topic_id(),
        public_scope_id(),
        payload_value,
        Utc::now(),
    );
    write.metadata = serde_json::json!({"intent": "fixture"});

    // Apply inline (live path) — no edge should be projected because CP1 has
    // no "rbs-target".
    let mut tx = pool.begin().await.expect("begin tx");
    let event = append_event_tx(&mut tx, write).await.expect("append");
    apply_relationship_event(&mut tx, &event, EventType::RelationshipAsserted)
        .await
        .expect("apply inline");
    tx.commit().await.expect("commit");

    // Verify no edge was projected inline.
    let inline_edge_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_resource_edges WHERE source_resource_id = $1")
            .bind(a)
            .fetch_one(&pool)
            .await
            .expect("inline edge count");
    assert_eq!(
        inline_edge_count, 0,
        "inline apply: no edge in CP1 for slug 'rbs-target'"
    );

    // Now rebuild — must also produce zero edges (slug "rbs-target" not in CP1).
    let mut tx = pool.begin().await.expect("begin rebuild tx");
    rebuild_edge_projection(&mut tx).await.expect("rebuild");
    tx.commit().await.expect("commit rebuild");

    let rebuilt_edge_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_resource_edges WHERE source_resource_id = $1")
            .bind(a)
            .fetch_one(&pool)
            .await
            .expect("rebuilt edge count");

    // Pre-fix: rebuild used resources_visible_to and would have found C in CP2,
    // producing count=1. Post-fix: context-scoped, count=0.
    assert_eq!(
        rebuilt_edge_count, 0,
        "rebuild must not project edge when slug exists in a different context (CP2) \
         than the source (CP1)"
    );

    // Verify C was NOT chosen as the target (belt+suspenders).
    let spurious_edge: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resource_edges WHERE source_resource_id = $1 AND target_resource_id = $2",
    )
    .bind(a)
    .bind(c)
    .fetch_one(&pool)
    .await
    .expect("spurious edge check");
    assert_eq!(
        spurious_edge, 0,
        "no spurious A→C edge via cross-context slug resolution"
    );
}
