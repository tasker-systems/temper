#![cfg(feature = "test-db")]
//! DbBackend relationship-write methods: assert, retype, reweight, fold.

mod common;

use sqlx::PgPool;
use uuid::Uuid;

use temper_api::backend::DbBackend;
use temper_core::operations::{
    AssertRelationship, FoldRelationship, ResourceRef, RetypeRelationship, ReweightRelationship,
    Surface,
};
use temper_core::types::graph::{EdgeKind, Polarity};
use temper_core::types::ids::ProfileId;

// ─── Helpers ────────────────────────────────────────────────────────────────

fn backend(pool: PgPool, profile: Uuid) -> DbBackend {
    DbBackend::new(
        pool,
        ProfileId::from(profile),
        "test-device".to_string(),
        Surface::ApiHttp,
    )
}

/// Create a resource in the given context (not the system context) so that
/// `resolve_by_uri` with `owner = "@me"` can find it for the given profile.
async fn create_resource_in_context(
    pool: &PgPool,
    owner_id: Uuid,
    context_id: Uuid,
    title: &str,
    slug: &str,
) -> Uuid {
    let id = Uuid::now_v7();
    let doc_type_id = uuid::Uuid::parse_str(common::fixtures::RESEARCH_DOC_TYPE_ID).unwrap();
    sqlx::query(
        r#"INSERT INTO kb_resources
            (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
             originator_profile_id, owner_profile_id, is_active, created, updated)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $7, true, now(), now())"#,
    )
    .bind(id)
    .bind(context_id)
    .bind(doc_type_id)
    .bind(format!("test://{slug}"))
    .bind(title)
    .bind(slug)
    .bind(owner_id)
    .execute(pool)
    .await
    .expect("create_resource_in_context");
    id
}

/// Build an `AssertRelationship` command using `ResourceRef::Scoped` for the
/// source. The source resource must live in a profile-owned "temper" context.
fn assert_cmd_scoped(
    context: &str,
    source_slug: &str,
    target_slug: &str,
    edge_kind: EdgeKind,
    polarity: Polarity,
    label: &str,
    weight: f64,
) -> AssertRelationship {
    AssertRelationship {
        source: ResourceRef::Scoped {
            owner: "@me".to_string(),
            context: context.to_string(),
            doctype: "research".to_string(),
            slug: source_slug.to_string(),
        },
        target_slug: target_slug.to_string(),
        edge_kind,
        polarity,
        label: label.to_string(),
        weight,
        origin: Surface::ApiHttp,
    }
}

/// Build an `AssertRelationship` command using `ResourceRef::Uuid` for the
/// source — bypasses ownership-based resolution, useful for auth tests where
/// a different profile tries to assert on a resource it doesn't own.
fn assert_cmd_uuid(
    source_id: Uuid,
    target_slug: &str,
    edge_kind: EdgeKind,
    polarity: Polarity,
    label: &str,
    weight: f64,
) -> AssertRelationship {
    AssertRelationship {
        source: ResourceRef::Uuid {
            id: source_id.into(),
        },
        target_slug: target_slug.to_string(),
        edge_kind,
        polarity,
        label: label.to_string(),
        weight,
        origin: Surface::ApiHttp,
    }
}

// Count relationship events in the ledger for a given source resource id.
async fn count_relationship_events(pool: &PgPool, source_resource_id: Uuid) -> i64 {
    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT count(*)
          FROM kb_events ev
          JOIN kb_event_types et ON et.id = ev.event_type_id
         WHERE et.name LIKE 'relationship_%'
           AND (ev.payload->>'source_resource_id')::uuid = $1
        "#,
    )
    .bind(source_resource_id)
    .fetch_one(pool)
    .await
    .expect("count_relationship_events")
}

// ─── Test 1: assert projects an edge row ────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn assert_relationship_projects_edge(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    let (profile, context_id) =
        common::fixtures::create_test_profile_with_context(&pool, "rw-assert@test.com").await;
    // Create resources in the profile's own "temper" context so
    // resolve_by_uri with owner="@me" can find them.
    let a = create_resource_in_context(&pool, profile, context_id, "Doc A", "rw-assert-a").await;
    let b = create_resource_in_context(&pool, profile, context_id, "Doc B", "rw-assert-b").await;

    let cmd = assert_cmd_scoped(
        "temper",
        "rw-assert-a",
        "rw-assert-b",
        EdgeKind::LeadsTo,
        Polarity::Forward,
        "depends_on",
        1.0,
    );

    let output = backend(pool.clone(), profile)
        .assert_relationship(cmd)
        .await
        .expect("assert_relationship should succeed");

    let correlation_id = output.value;

    // Events vec contains DbRelationshipAsserted.
    assert!(
        output.events.iter().any(|e| matches!(
            e,
            temper_core::operations::DomainEvent::DbRelationshipAsserted { .. }
        )),
        "output events should contain DbRelationshipAsserted"
    );

    // Edge row exists with correct shape.
    let edge_count: i64 = sqlx::query_scalar(
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
    assert_eq!(edge_count, 1, "one leads_to edge A→B expected");

    // Event has intent = 'explicit'.
    let intent: String = sqlx::query_scalar(
        r#"SELECT ev.metadata->>'intent'
             FROM kb_events ev
            WHERE ev.correlation_id = $1
            LIMIT 1"#,
    )
    .bind(correlation_id)
    .fetch_one(&pool)
    .await
    .expect("intent from event");
    assert_eq!(
        intent, "explicit",
        "event metadata.intent should be 'explicit'"
    );
}

// ─── Test 2: unauthorized profile cannot assert ──────────────────────────────

/// Profile Q tries to assert a relationship from resource A (owned by P).
/// `ResourceRef::Uuid` is used so resolve succeeds, but `check_can_modify` must
/// reject Q's write attempt.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn assert_relationship_unauthorized_profile(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    let (profile_p, context_p) =
        common::fixtures::create_test_profile_with_context(&pool, "rw-authp@test.com").await;
    let (profile_q, _context_q) =
        common::fixtures::create_test_profile_with_context(&pool, "rw-authq@test.com").await;
    // A is owned by P.
    let a = create_resource_in_context(&pool, profile_p, context_p, "Doc A", "rw-auth-a").await;
    let _b = create_resource_in_context(&pool, profile_p, context_p, "Doc B", "rw-auth-b").await;

    let before = count_relationship_events(&pool, a).await;

    // Q tries to assert from A (which P owns) using UUID ref so resolution succeeds.
    let cmd = assert_cmd_uuid(
        a,
        "rw-auth-b",
        EdgeKind::Near,
        Polarity::Forward,
        "relates_to",
        1.0,
    );

    let result = backend(pool.clone(), profile_q)
        .assert_relationship(cmd)
        .await;

    assert!(
        result.is_err(),
        "Q should not be able to assert on P's resource"
    );

    let after = count_relationship_events(&pool, a).await;
    assert_eq!(before, after, "no events should have been appended");
}

// ─── Test 3: assert to nonexistent slug appends event but no edge row ────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn assert_to_nonexistent_slug_appends_event_but_no_row(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    let (profile, context_id) =
        common::fixtures::create_test_profile_with_context(&pool, "rw-ghost@test.com").await;
    let a = create_resource_in_context(&pool, profile, context_id, "Doc A", "rw-ghost-a").await;

    let cmd = assert_cmd_scoped(
        "temper",
        "rw-ghost-a",
        "ghost-slug",
        EdgeKind::LeadsTo,
        Polarity::Forward,
        "depends_on",
        1.0,
    );

    let output = backend(pool.clone(), profile)
        .assert_relationship(cmd)
        .await
        .expect("assert to nonexistent slug should succeed");

    let _ = output.value;

    // Event exists in the ledger.
    let event_count: i64 = count_relationship_events(&pool, a).await;
    assert_eq!(event_count, 1, "event appended to ledger");

    // No edge row projected.
    let edge_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_resource_edges WHERE source_resource_id = $1")
            .bind(a)
            .fetch_one(&pool)
            .await
            .expect("edge count");
    assert_eq!(
        edge_count, 0,
        "no edge projected for unresolved slug target"
    );
}

// ─── Test 4: retype changes edge_kind ────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn retype_changes_edge_kind(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    let (profile, context_id) =
        common::fixtures::create_test_profile_with_context(&pool, "rw-retype@test.com").await;
    let _a = create_resource_in_context(&pool, profile, context_id, "Doc A", "rw-retype-a").await;
    let _b = create_resource_in_context(&pool, profile, context_id, "Doc B", "rw-retype-b").await;

    let cmd = assert_cmd_scoped(
        "temper",
        "rw-retype-a",
        "rw-retype-b",
        EdgeKind::LeadsTo,
        Polarity::Forward,
        "depends_on",
        1.0,
    );
    let output = backend(pool.clone(), profile)
        .assert_relationship(cmd)
        .await
        .expect("assert");
    let correlation_id = output.value;

    // Capture original last_event_id.
    let original_last: Uuid = sqlx::query_scalar(
        "SELECT last_event_id FROM kb_resource_edges WHERE asserted_by_event_id = $1",
    )
    .bind(correlation_id)
    .fetch_one(&pool)
    .await
    .expect("original last_event_id");

    // Retype to Contains.
    let retype = RetypeRelationship {
        correlation_id,
        edge_kind: EdgeKind::Contains,
        polarity: Polarity::Forward,
        origin: Surface::ApiHttp,
    };
    let retype_output = backend(pool.clone(), profile)
        .retype_relationship(retype)
        .await
        .expect("retype");

    assert!(
        retype_output.events.iter().any(|e| matches!(
            e,
            temper_core::operations::DomainEvent::DbRelationshipRetyped { .. }
        )),
        "output events should contain DbRelationshipRetyped"
    );

    // Verify edge row updated.
    let row = sqlx::query!(
        r#"SELECT edge_kind AS "edge_kind!: String",
                  last_event_id AS "last_event_id!: Uuid"
             FROM kb_resource_edges
            WHERE asserted_by_event_id = $1"#,
        correlation_id,
    )
    .fetch_one(&pool)
    .await
    .expect("edge row");

    assert_eq!(
        row.edge_kind, "contains",
        "edge_kind should be updated to contains"
    );
    assert_ne!(
        row.last_event_id, original_last,
        "last_event_id should be bumped"
    );
}

// ─── Test 5: reweight changes weight ─────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reweight_changes_weight(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    let (profile, context_id) =
        common::fixtures::create_test_profile_with_context(&pool, "rw-reweight@test.com").await;
    let _a = create_resource_in_context(&pool, profile, context_id, "Doc A", "rw-reweight-a").await;
    let _b = create_resource_in_context(&pool, profile, context_id, "Doc B", "rw-reweight-b").await;

    let cmd = assert_cmd_scoped(
        "temper",
        "rw-reweight-a",
        "rw-reweight-b",
        EdgeKind::LeadsTo,
        Polarity::Forward,
        "depends_on",
        1.0,
    );
    let output = backend(pool.clone(), profile)
        .assert_relationship(cmd)
        .await
        .expect("assert");
    let correlation_id = output.value;

    let reweight = ReweightRelationship {
        correlation_id,
        weight: 5.5,
        origin: Surface::ApiHttp,
    };
    let reweight_output = backend(pool.clone(), profile)
        .reweight_relationship(reweight)
        .await
        .expect("reweight");

    assert!(
        reweight_output.events.iter().any(|e| matches!(
            e,
            temper_core::operations::DomainEvent::DbRelationshipReweighted { .. }
        )),
        "output events should contain DbRelationshipReweighted"
    );

    let weight: f64 =
        sqlx::query_scalar("SELECT weight FROM kb_resource_edges WHERE asserted_by_event_id = $1")
            .bind(correlation_id)
            .fetch_one(&pool)
            .await
            .expect("weight");
    assert!(
        (weight - 5.5).abs() < f64::EPSILON,
        "weight should be 5.5, got {weight}"
    );
}

// ─── Test 6: fold marks row folded and excludes from neighbors ───────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn fold_marks_row_folded(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    let (profile, context_id) =
        common::fixtures::create_test_profile_with_context(&pool, "rw-fold@test.com").await;
    let a = create_resource_in_context(&pool, profile, context_id, "Doc A", "rw-fold-a").await;
    let _b = create_resource_in_context(&pool, profile, context_id, "Doc B", "rw-fold-b").await;

    let cmd = assert_cmd_scoped(
        "temper",
        "rw-fold-a",
        "rw-fold-b",
        EdgeKind::LeadsTo,
        Polarity::Forward,
        "depends_on",
        1.0,
    );
    let output = backend(pool.clone(), profile)
        .assert_relationship(cmd)
        .await
        .expect("assert");
    let correlation_id = output.value;

    let fold = FoldRelationship {
        correlation_id,
        reason: Some("test fold".to_string()),
        origin: Surface::ApiHttp,
    };
    let fold_output = backend(pool.clone(), profile)
        .fold_relationship(fold)
        .await
        .expect("fold");

    assert!(
        fold_output.events.iter().any(|e| matches!(
            e,
            temper_core::operations::DomainEvent::DbRelationshipFolded { .. }
        )),
        "output events should contain DbRelationshipFolded"
    );

    let is_folded: bool = sqlx::query_scalar(
        "SELECT is_folded FROM kb_resource_edges WHERE asserted_by_event_id = $1",
    )
    .bind(correlation_id)
    .fetch_one(&pool)
    .await
    .expect("is_folded");
    assert!(is_folded, "edge should be folded");

    // B should not appear in graph_neighbors for A.
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

// ─── Test 7: retype unauthorized ─────────────────────────────────────────────

/// Profile P asserts A→B. Profile Q tries to retype it — should fail.
/// Q uses `retype_relationship` directly with P's correlation_id; the
/// `edge_auth_row` lookup finds the edge, then `check_can_modify` rejects Q.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn retype_unauthorized(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    let (profile_p, context_p) =
        common::fixtures::create_test_profile_with_context(&pool, "rw-rauth-p@test.com").await;
    let (profile_q, _) =
        common::fixtures::create_test_profile_with_context(&pool, "rw-rauth-q@test.com").await;
    let _a = create_resource_in_context(&pool, profile_p, context_p, "Doc A", "rw-rauth-a").await;
    let _b = create_resource_in_context(&pool, profile_p, context_p, "Doc B", "rw-rauth-b").await;

    // P asserts A→B.
    let cmd = assert_cmd_scoped(
        "temper",
        "rw-rauth-a",
        "rw-rauth-b",
        EdgeKind::LeadsTo,
        Polarity::Forward,
        "depends_on",
        1.0,
    );
    let output = backend(pool.clone(), profile_p)
        .assert_relationship(cmd)
        .await
        .expect("assert by P");
    let correlation_id = output.value;

    let event_count_before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_events WHERE correlation_id = $1")
            .bind(correlation_id)
            .fetch_one(&pool)
            .await
            .expect("event count before");

    // Q tries to retype — should fail.
    let retype = RetypeRelationship {
        correlation_id,
        edge_kind: EdgeKind::Contains,
        polarity: Polarity::Forward,
        origin: Surface::ApiHttp,
    };
    let result = backend(pool.clone(), profile_q)
        .retype_relationship(retype)
        .await;
    assert!(result.is_err(), "Q should not be able to retype P's edge");

    let event_count_after: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_events WHERE correlation_id = $1")
            .bind(correlation_id)
            .fetch_one(&pool)
            .await
            .expect("event count after");

    assert_eq!(
        event_count_before, event_count_after,
        "no new event should have been appended after auth failure"
    );
}
