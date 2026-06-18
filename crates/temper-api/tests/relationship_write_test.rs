#![cfg(feature = "test-db")]
//! DbBackend relationship-write methods: assert, retype, reweight, fold.

mod common;

use sqlx::PgPool;
use uuid::Uuid;

use temper_api::backend::DbBackend;
use temper_core::operations::{
    AssertRelationship, Backend, DomainEvent, FoldRelationship, RetypeRelationship,
    ReweightRelationship, Surface,
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
/// it is visible to and modifiable by the given profile.
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

/// Build an `AssertRelationship` command from pre-resolved source + target
/// resource ids (edge addressing is id-based on both endpoints).
fn assert_cmd_uuid(
    source_id: Uuid,
    target_id: Uuid,
    edge_kind: EdgeKind,
    polarity: Polarity,
    label: &str,
    weight: f64,
) -> AssertRelationship {
    AssertRelationship {
        source: source_id.into(),
        target: target_id.into(),
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
    // Create resources in the profile's own "temper" context so they are
    // visible to and modifiable by the profile.
    let a = create_resource_in_context(&pool, profile, context_id, "Doc A", "rw-assert-a").await;
    let b = create_resource_in_context(&pool, profile, context_id, "Doc B", "rw-assert-b").await;

    let cmd = assert_cmd_uuid(
        a,
        b,
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
/// The source id resolves fine, but `check_can_modify` must reject Q's write
/// attempt.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn assert_relationship_unauthorized_profile(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    let (profile_p, context_p) =
        common::fixtures::create_test_profile_with_context(&pool, "rw-authp@test.com").await;
    let (profile_q, _context_q) =
        common::fixtures::create_test_profile_with_context(&pool, "rw-authq@test.com").await;
    // A is owned by P.
    let a = create_resource_in_context(&pool, profile_p, context_p, "Doc A", "rw-auth-a").await;
    let b = create_resource_in_context(&pool, profile_p, context_p, "Doc B", "rw-auth-b").await;

    let before = count_relationship_events(&pool, a).await;

    // Q tries to assert from A (which P owns) using UUID ref so resolution succeeds.
    let cmd = assert_cmd_uuid(a, b, EdgeKind::Near, Polarity::Forward, "relates_to", 1.0);

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

// (Removed Test 3 "assert to nonexistent slug appends event but no edge row":
// the live `AssertRelationship` command no longer accepts an unresolved slug
// target. Both endpoints are pre-resolved ids now, so forward-reference-by-slug
// is not expressible on this surface. The forward-projection capability survives
// only in the frontmatter-declaration ingest path and historical event replay,
// covered by `relationship_projection_test.rs`.)

// ─── Test 4: retype changes edge_kind ────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn retype_changes_edge_kind(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    let (profile, context_id) =
        common::fixtures::create_test_profile_with_context(&pool, "rw-retype@test.com").await;
    let a = create_resource_in_context(&pool, profile, context_id, "Doc A", "rw-retype-a").await;
    let b = create_resource_in_context(&pool, profile, context_id, "Doc B", "rw-retype-b").await;

    let cmd = assert_cmd_uuid(
        a,
        b,
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
        edge_handle: correlation_id,
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
    let a = create_resource_in_context(&pool, profile, context_id, "Doc A", "rw-reweight-a").await;
    let b = create_resource_in_context(&pool, profile, context_id, "Doc B", "rw-reweight-b").await;

    let cmd = assert_cmd_uuid(
        a,
        b,
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
        edge_handle: correlation_id,
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
    let b = create_resource_in_context(&pool, profile, context_id, "Doc B", "rw-fold-b").await;

    let cmd = assert_cmd_uuid(
        a,
        b,
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
        edge_handle: correlation_id,
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
    let a = create_resource_in_context(&pool, profile_p, context_p, "Doc A", "rw-rauth-a").await;
    let b = create_resource_in_context(&pool, profile_p, context_p, "Doc B", "rw-rauth-b").await;

    // P asserts A→B.
    let cmd = assert_cmd_uuid(
        a,
        b,
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
        edge_handle: correlation_id,
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

// ─── Test 8: re-assert active edge converts to reweight ──────────────────────

/// Asserting the same edge twice (same source, target slug, kind, label,
/// polarity) when the edge is still active must divert to a `reweight` under
/// the FIRST assertion's correlation chain. No second `relationship_asserted`
/// event should appear in the ledger.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reassert_active_edge_converts_to_reweight(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    let (profile, context_id) =
        common::fixtures::create_test_profile_with_context(&pool, "rw-reassert@test.com").await;
    let a = create_resource_in_context(&pool, profile, context_id, "Doc A", "rw-reassert-a").await;
    let b = create_resource_in_context(&pool, profile, context_id, "Doc B", "rw-reassert-b").await;

    // First assertion (weight=1.0).
    let first_cmd = assert_cmd_uuid(
        a,
        b,
        EdgeKind::LeadsTo,
        Polarity::Forward,
        "depends_on",
        1.0,
    );
    let first_output = backend(pool.clone(), profile)
        .assert_relationship(first_cmd)
        .await
        .expect("first assert should succeed");

    let first_correlation_id = first_output.value;

    // Capture asserted_by_event_id so we can verify it stays unchanged.
    let original_asserted_by: Uuid = sqlx::query_scalar(
        "SELECT asserted_by_event_id FROM kb_resource_edges WHERE asserted_by_event_id = $1",
    )
    .bind(first_correlation_id)
    .fetch_one(&pool)
    .await
    .expect("original asserted_by_event_id");
    assert_eq!(original_asserted_by, first_correlation_id);

    // Second "assertion" with weight=3.0 — same key, edge is active.
    let second_cmd = assert_cmd_uuid(
        a,
        b,
        EdgeKind::LeadsTo,
        Polarity::Forward,
        "depends_on",
        3.0,
    );
    let second_output = backend(pool.clone(), profile)
        .assert_relationship(second_cmd)
        .await
        .expect("re-assert should succeed (divert to reweight)");

    // Response must be DbRelationshipReweighted, NOT DbRelationshipAsserted.
    assert!(
        second_output
            .events
            .iter()
            .any(|e| matches!(e, DomainEvent::DbRelationshipReweighted { .. })),
        "re-assert of active edge should produce DbRelationshipReweighted"
    );
    assert!(
        !second_output
            .events
            .iter()
            .any(|e| matches!(e, DomainEvent::DbRelationshipAsserted { .. })),
        "re-assert of active edge must NOT produce a second DbRelationshipAsserted"
    );

    // Returned correlation_id must equal the FIRST assertion's correlation_id.
    assert_eq!(
        second_output.value, first_correlation_id,
        "diverted reweight must return the original correlation_id"
    );

    // Ledger: exactly one `relationship_asserted` + one `relationship_reweighted`.
    let assert_count: i64 = sqlx::query_scalar(
        r#"SELECT count(*)
             FROM kb_events ev
             JOIN kb_event_types et ON et.id = ev.event_type_id
            WHERE et.name = 'relationship_asserted'
              AND (ev.payload->>'source_resource_id')::uuid = $1"#,
    )
    .bind(a)
    .fetch_one(&pool)
    .await
    .expect("assert count");
    assert_eq!(
        assert_count, 1,
        "exactly one relationship_asserted in ledger"
    );

    let reweight_count: i64 = sqlx::query_scalar(
        r#"SELECT count(*)
             FROM kb_events ev
             JOIN kb_event_types et ON et.id = ev.event_type_id
            WHERE et.name = 'relationship_reweighted'
              AND ev.correlation_id = $1"#,
    )
    .bind(first_correlation_id)
    .fetch_one(&pool)
    .await
    .expect("reweight count");
    assert_eq!(
        reweight_count, 1,
        "exactly one relationship_reweighted in ledger"
    );

    // Edge row: weight=3.0, asserted_by_event_id unchanged (pinned to original).
    let row = sqlx::query!(
        r#"SELECT weight                AS "weight!: f64",
                  asserted_by_event_id  AS "asserted_by_event_id!: Uuid",
                  last_event_id         AS "last_event_id!: Uuid"
             FROM kb_resource_edges
            WHERE asserted_by_event_id = $1"#,
        first_correlation_id,
    )
    .fetch_one(&pool)
    .await
    .expect("edge row after re-assert");

    assert!(
        (row.weight - 3.0).abs() < f64::EPSILON,
        "weight should be 3.0 after re-assert diverted to reweight, got {}",
        row.weight
    );
    assert_eq!(
        row.asserted_by_event_id, first_correlation_id,
        "asserted_by_event_id must remain pinned to original assertion"
    );
    assert_ne!(
        row.last_event_id, first_correlation_id,
        "last_event_id must be bumped by the reweight event"
    );
}

// ─── Test 9: re-assert folded edge starts new chain ──────────────────────────

/// Assert A→B, fold it, then re-assert. The re-assert must start a new
/// correlation chain (fresh `relationship_asserted`). The `asserted_by_event_id`
/// on the row must transfer to the new assertion's event id.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reassert_folded_edge_starts_new_chain(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;
    let (profile, context_id) =
        common::fixtures::create_test_profile_with_context(&pool, "rw-refold@test.com").await;
    let a = create_resource_in_context(&pool, profile, context_id, "Doc A", "rw-refold-a").await;
    let b = create_resource_in_context(&pool, profile, context_id, "Doc B", "rw-refold-b").await;

    // First assertion.
    let cmd1 = assert_cmd_uuid(
        a,
        b,
        EdgeKind::LeadsTo,
        Polarity::Forward,
        "depends_on",
        1.0,
    );
    let out1 = backend(pool.clone(), profile)
        .assert_relationship(cmd1)
        .await
        .expect("first assert");
    let first_correlation_id = out1.value;

    // Fold the edge.
    let fold = FoldRelationship {
        edge_handle: first_correlation_id,
        reason: Some("test fold before re-assert".to_string()),
        origin: Surface::ApiHttp,
    };
    backend(pool.clone(), profile)
        .fold_relationship(fold)
        .await
        .expect("fold");

    // Re-assert with weight=2.0 — edge is folded, so a new chain starts.
    let cmd2 = assert_cmd_uuid(
        a,
        b,
        EdgeKind::LeadsTo,
        Polarity::Forward,
        "depends_on",
        2.0,
    );
    let out2 = backend(pool.clone(), profile)
        .assert_relationship(cmd2)
        .await
        .expect("re-assert after fold");
    let new_correlation_id = out2.value;

    // Response must be DbRelationshipAsserted.
    assert!(
        out2.events
            .iter()
            .any(|e| matches!(e, DomainEvent::DbRelationshipAsserted { .. })),
        "re-assert of folded edge should produce DbRelationshipAsserted"
    );

    // New correlation_id must differ from the first.
    assert_ne!(
        new_correlation_id, first_correlation_id,
        "re-assert after fold must produce a new correlation chain"
    );

    // Ledger: two `relationship_asserted` events.
    let assert_count: i64 = sqlx::query_scalar(
        r#"SELECT count(*)
             FROM kb_events ev
             JOIN kb_event_types et ON et.id = ev.event_type_id
            WHERE et.name = 'relationship_asserted'
              AND (ev.payload->>'source_resource_id')::uuid = $1"#,
    )
    .bind(a)
    .fetch_one(&pool)
    .await
    .expect("assert event count");
    assert_eq!(
        assert_count, 2,
        "two relationship_asserted events in ledger"
    );

    // Edge row: is_folded=false, weight=2.0, asserted_by_event_id = new chain.
    let row = sqlx::query!(
        r#"SELECT weight                AS "weight!: f64",
                  is_folded             AS "is_folded!",
                  asserted_by_event_id  AS "asserted_by_event_id!: Uuid",
                  last_event_id         AS "last_event_id!: Uuid"
             FROM kb_resource_edges
            WHERE source_resource_id = $1"#,
        a,
    )
    .fetch_one(&pool)
    .await
    .expect("edge row after re-assert");

    assert!(!row.is_folded, "edge should be unfolded after re-assert");
    assert!(
        (row.weight - 2.0).abs() < f64::EPSILON,
        "weight should be 2.0 after re-assert, got {}",
        row.weight
    );
    assert_eq!(
        row.asserted_by_event_id, new_correlation_id,
        "asserted_by_event_id must be transferred to the new chain"
    );
    assert_eq!(
        row.last_event_id, new_correlation_id,
        "last_event_id must be the new assertion's event id"
    );

    // Verify the new chain is healthy: folding via new_correlation_id actually folds the row.
    let fold2 = FoldRelationship {
        edge_handle: new_correlation_id,
        reason: None,
        origin: Surface::ApiHttp,
    };
    backend(pool.clone(), profile)
        .fold_relationship(fold2)
        .await
        .expect("fold via new chain should succeed");

    let is_folded_now: bool =
        sqlx::query_scalar("SELECT is_folded FROM kb_resource_edges WHERE source_resource_id = $1")
            .bind(a)
            .fetch_one(&pool)
            .await
            .expect("is_folded after second fold");
    assert!(is_folded_now, "fold via new correlation chain must work");
}
