#![cfg(feature = "test-db")]
//! HTTP-layer integration tests for `POST /api/resources/{id}/citation-audits`.
//!
//! CONFORM to `relationship_handler_test.rs`: the full stack (JWT auth → system-access gate →
//! handler → `citation_audit_service` → `DbBackend` → Postgres) via the `TestApp` harness, not a
//! service-direct call — unlike `evidence_handler_test.rs`, this endpoint's 401 and the
//! path/block transposition guard only exist at the HTTP layer, so a service-level test cannot
//! see them.
//!
//! Fixture shape mirrors `authz/audit_gate.rs`'s own test seed (a finding + one content block,
//! plus a read-only `kb_access_grants` row for a non-author reader) — the same three principals
//! that gate distinguishes: an author, a reader who did not author, and an outsider with no
//! grant at all.

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

// ─── Fixture helpers ─────────────────────────────────────────────────────────

/// Seed a finding (`kb_resources` + `kb_resource_homes`) owned/authored by `owner`, homed in
/// `ctx`, with one content block. Returns `(finding_id, block_id)`.
///
/// The block borrows a migration-seeded `kb_events` row for its genesis/last-event FKs — the
/// same fixture shortcut `authz/audit_gate.rs`'s own `seed` fn takes, since no event content is
/// read by anything under test here.
async fn seed_finding_with_block(
    pool: &PgPool,
    owner: Uuid,
    ctx: Uuid,
    title: &str,
) -> (Uuid, Uuid) {
    let finding = Uuid::now_v7();
    sqlx::query("INSERT INTO kb_resources (id, title, origin_uri) VALUES ($1, $2, $3)")
        .bind(finding)
        .bind(title)
        .bind(format!("test://{finding}"))
        .execute(pool)
        .await
        .expect("insert finding");
    sqlx::query(
        "INSERT INTO kb_resource_homes \
             (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(finding)
    .bind(ctx)
    .bind(owner)
    .execute(pool)
    .await
    .expect("home finding");

    let ev: Uuid = sqlx::query_scalar("SELECT id FROM kb_events LIMIT 1")
        .fetch_one(pool)
        .await
        .expect("seed event");
    let block = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_content_blocks (id, resource_id, seq, genesis_event_id, last_event_id) \
         VALUES ($1, $2, 0, $3, $3)",
    )
    .bind(block)
    .bind(finding)
    .bind(ev)
    .execute(pool)
    .await
    .expect("insert block");

    (finding, block)
}

/// A direct, profile-anchored read-without-write `kb_access_grants` row — the shape `AuditAuthority`
/// depends on to admit a reader who is not the author (`audit_gate.rs`'s own fixture, `:241-254`).
async fn grant_read_only(pool: &PgPool, finding: Uuid, reader: Uuid, granted_by: Uuid) {
    sqlx::query(
        "INSERT INTO kb_access_grants \
             (subject_table, subject_id, principal_table, principal_id, can_read, can_write, \
              granted_by_profile_id) \
         VALUES ('kb_resources', $1, 'kb_profiles', $2, true, false, $3)",
    )
    .bind(finding)
    .bind(reader)
    .bind(granted_by)
    .execute(pool)
    .await
    .expect("grant read-only access");
}

/// A well-formed request body: a resource-kind source (any id — the write path validates only
/// that the source KIND is `resource`, never that the id is an existing citation;
/// `migrations/20260723000010_citation_audits.sql:123-126`) and an in-range value.
fn valid_body(block: Uuid) -> Value {
    json!({
        "block_id": block,
        "source": { "kind": "resource", "value": Uuid::now_v7() },
        "value": 0.8,
        "reason": "independently verified",
    })
}

// ─── Test 1: a reader who is not the author gets 200 + the new audit id ─────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn posting_an_audit_returns_the_audit_id(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;

    let email_author = format!("cah-author-{}@example.com", Uuid::new_v4());
    let (author, ctx) =
        common::fixtures::create_test_profile_with_context(&pool, &email_author).await;
    let (finding, block) = seed_finding_with_block(&pool, author, ctx, "Finding").await;

    let email_reader = format!("cah-reader-{}@example.com", Uuid::new_v4());
    let (reader, _) =
        common::fixtures::create_test_profile_with_context(&pool, &email_reader).await;
    grant_read_only(&pool, finding, reader, author).await;
    let token = common::generate_test_jwt(&format!("test|{reader}"), &email_reader);

    let resp = app
        .client
        .post(app.url(&format!("/api/resources/{finding}/citation-audits")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&valid_body(block))
        .send()
        .await
        .expect("request failed");

    let status = resp.status().as_u16();
    let body_text = resp.text().await.unwrap_or_default();
    assert_eq!(status, 200, "expected 200; body: {body_text}");

    // The response body is a bare Uuid (temper-client's `record_citation_audit` deserializes it
    // as such, per Task 7's decision — `temper-client/src/resources.rs:213-224`).
    let audit_id: Uuid =
        serde_json::from_str(&body_text).expect("response body should be a bare JSON Uuid string");

    let row_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM kb_citation_audits WHERE id = $1 AND block_id = $2)",
    )
    .bind(audit_id)
    .bind(block)
    .fetch_one(&pool)
    .await
    .expect("check audit row");
    assert!(
        row_exists,
        "the returned id must name a real kb_citation_audits row on the audited block"
    );
}

// ─── Test 2: no Authorization header → 401 ──────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn posting_an_audit_without_auth_returns_401(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let resp = app
        .client
        .post(app.url(&format!(
            "/api/resources/{}/citation-audits",
            Uuid::new_v4()
        )))
        .json(&valid_body(Uuid::new_v4()))
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        401,
        "missing auth should return 401"
    );
}

// ─── Test 3: an outsider with no grant at all → 404 (Unreadable) ────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn posting_on_an_unreadable_finding_returns_404(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;

    let email_author = format!("cah-author-{}@example.com", Uuid::new_v4());
    let (author, ctx) =
        common::fixtures::create_test_profile_with_context(&pool, &email_author).await;
    let (finding, block) = seed_finding_with_block(&pool, author, ctx, "Finding").await;

    // No access grant, no home, no team — outright unreadable.
    let email_stranger = format!("cah-stranger-{}@example.com", Uuid::new_v4());
    let (stranger, _) =
        common::fixtures::create_test_profile_with_context(&pool, &email_stranger).await;
    let token = common::generate_test_jwt(&format!("test|{stranger}"), &email_stranger);

    let resp = app
        .client
        .post(app.url(&format!("/api/resources/{finding}/citation-audits")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&valid_body(block))
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        404,
        "an unreadable finding must refuse with 404, not 403 (no existence oracle)"
    );
}

// ─── Test 4: the author auditing its own finding → 404 (self-audit) ────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn posting_as_the_author_returns_404(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;

    let email_author = format!("cah-author-{}@example.com", Uuid::new_v4());
    let (author, ctx) =
        common::fixtures::create_test_profile_with_context(&pool, &email_author).await;
    let (finding, block) = seed_finding_with_block(&pool, author, ctx, "Finding").await;
    let token = common::generate_test_jwt(&format!("test|{author}"), &email_author);

    let resp = app
        .client
        .post(app.url(&format!("/api/resources/{finding}/citation-audits")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&valid_body(block))
        .send()
        .await
        .expect("request failed");

    // Without the backend's `AuditAuthority::Author` denial reaching the HTTP surface intact,
    // this would come back 200 (the author CAN read its own finding, so readability alone would
    // admit it) — this is the full-stack proof that the self-audit refusal survives the round
    // trip through the handler and service, not just the backend unit test that already covers it.
    assert_eq!(
        resp.status().as_u16(),
        404,
        "the author must not be able to audit its own finding"
    );
}

// ─── Test 5: block from a DIFFERENT finding than the path → 404 (transposition) ──

/// Both findings are readable by the caller and authored by neither — isolating the mismatch
/// itself as the only possible reason for a refusal. Without the path/block match check this
/// would return 200: the reader legitimately qualifies as `AuditAuthority::Auditor` on
/// `finding_b` (readable, not authored), so the backend alone would happily authorize and record
/// the write under the wrong route.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn posting_a_block_of_another_finding_returns_404(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;

    let email_a = format!("cah-owner-a-{}@example.com", Uuid::new_v4());
    let (owner_a, ctx_a) =
        common::fixtures::create_test_profile_with_context(&pool, &email_a).await;
    let (finding_a, _block_a) = seed_finding_with_block(&pool, owner_a, ctx_a, "Finding A").await;

    let email_b = format!("cah-owner-b-{}@example.com", Uuid::new_v4());
    let (owner_b, ctx_b) =
        common::fixtures::create_test_profile_with_context(&pool, &email_b).await;
    let (finding_b, block_b) = seed_finding_with_block(&pool, owner_b, ctx_b, "Finding B").await;

    let email_reader = format!("cah-reader-{}@example.com", Uuid::new_v4());
    let (reader, _) =
        common::fixtures::create_test_profile_with_context(&pool, &email_reader).await;
    // Readable on BOTH findings, authored on neither — so readability of finding_b cannot be
    // why this is refused.
    grant_read_only(&pool, finding_a, reader, owner_a).await;
    grant_read_only(&pool, finding_b, reader, owner_b).await;
    let token = common::generate_test_jwt(&format!("test|{reader}"), &email_reader);

    // Path names finding_a; the body's block belongs to finding_b.
    let resp = app
        .client
        .post(app.url(&format!("/api/resources/{finding_a}/citation-audits")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&valid_body(block_b))
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        404,
        "a block belonging to a different finding than the path must be refused, not silently \
         redirected onto the block's real finding"
    );

    // And no audit was recorded on either finding's block.
    let audited: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_citation_audits")
        .fetch_one(&pool)
        .await
        .expect("count audits");
    assert_eq!(audited, 0, "the mismatched write must not land");
}

// ─── Test 6: an out-of-range value → 400 ────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn posting_an_out_of_range_value_returns_400(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;

    let email_author = format!("cah-author-{}@example.com", Uuid::new_v4());
    let (author, ctx) =
        common::fixtures::create_test_profile_with_context(&pool, &email_author).await;
    let (finding, block) = seed_finding_with_block(&pool, author, ctx, "Finding").await;

    let email_reader = format!("cah-reader-{}@example.com", Uuid::new_v4());
    let (reader, _) =
        common::fixtures::create_test_profile_with_context(&pool, &email_reader).await;
    grant_read_only(&pool, finding, reader, author).await;
    let token = common::generate_test_jwt(&format!("test|{reader}"), &email_reader);

    let body = json!({
        "block_id": block,
        "source": { "kind": "resource", "value": Uuid::now_v7() },
        "value": 1.5,
        "reason": null,
    });

    let resp = app
        .client
        .post(app.url(&format!("/api/resources/{finding}/citation-audits")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status().as_u16(),
        400,
        "a verdict outside [-1.0, 1.0] must be refused as a caller fault (400), not surface as a \
         500 off the table's CHECK constraint"
    );
}
