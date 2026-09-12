//! Route witnesses for `POST /api/resources/adopt` — the corpus-adoption wire surface.
//!
//! Route-grain coverage only: each witness exercises the route's OWN contract — transport auth
//! (401), the backend seam's `Forbidden` for the deployment-wide arm surfacing through the route
//! (403), a well-formed survey receipt proving handler→backend dispatch end to end (200), and the
//! bounded-invocation refusal (400). Service-grain behaviour (per-row gating, the cursor, the
//! decline taxonomy, ledger effects) is covered by temper-services' `adoption_service_test` and
//! is not duplicated here.
//!
//! The one candidate is seeded through substrate `fire` directly with real chunker hashes and
//! stored verbatim bytes (no policy application at create), the same convention as the service
//! suite: a one-section body is already policy-conformed, so its survey row is a no-op.
#![cfg(feature = "test-db")]

mod common;

use sqlx::PgPool;
use temper_core::types::adoption::{AdoptRequest, AdoptScope};
use temper_substrate::content::{prepare_block_from_chunks, IncomingChunk};
use temper_substrate::events::{fire, SeedAction};
use temper_substrate::ids::{EntityId, ProfileId as SubstrateProfileId};
use temper_substrate::payloads::AnchorRef;
use uuid::Uuid;

/// The adopt request body, built through the typed request DTO — the same construction a typed
/// client performs, so the DTO's serde contract is what the route actually receives.
fn adopt_body(scope: AdoptScope, dry_run: bool, limit: Option<i64>) -> serde_json::Value {
    serde_json::to_value(&AdoptRequest {
        scope,
        dry_run,
        limit,
        after_id: None,
    })
    .expect("AdoptRequest serializes")
}

/// One-section body: already policy-conformed, so a survey row over it is a no-op.
const CONFORMED_BODY: &str = "# Alpha\n\nAlpha body paragraph.\n";

/// Real chunk hashes from the real chunker; the embedding is a placeholder the op never reads.
fn incoming_of(text: &str) -> Vec<IncomingChunk> {
    temper_ingest::chunk::chunk_markdown(text)
        .into_iter()
        .map(|c| IncomingChunk {
            chunk_index: c.chunk_index as i32,
            content_hash: c.content_hash,
            content: c.content,
            embedding: vec![0.0; 768],
            embedded_with: None,
            header_path: c.header_path.clone(),
            heading_depth: c.heading_depth as i16,
        })
        .collect()
}

/// One block with stored verbatim bytes and real chunk rows, fired directly (the create-path hook
/// partitions bodies at create, so this shape is reachable only by direct invocation, exactly as
/// the adoption tooling faces it). Returns the new resource id.
async fn fire_conformed_resource(
    pool: &PgPool,
    owner: Uuid,
    emitter: Uuid,
    context: Uuid,
    title: &str,
) -> Uuid {
    let mut block = prepare_block_from_chunks(0, None, incoming_of(CONFORMED_BODY));
    block.raw_text = Some(CONFORMED_BODY.to_string());
    let blocks = [block];
    let mut conn = pool.acquire().await.expect("acquire connection");
    fire(
        &mut conn,
        SeedAction::ResourceCreate {
            title,
            origin_uri: &format!("temper://adopt-route/{title}"),
            resource_id: None,
            home: AnchorRef::context(temper_substrate::ids::ContextId::from(context)),
            owner: SubstrateProfileId::from(owner),
            originator: Some(SubstrateProfileId::from(owner)),
            blocks: &blocks,
            doc_type: Some("concept"),
            emitter: EntityId::from(emitter),
            segmented: false,
        },
    )
    .await
    .expect("fire conformed resource")
    .resource()
    .expect("resource id")
    .uuid()
}

/// An unauthenticated POST to the adopt route is refused at the transport — the route lives on
/// the gated chain, so `require_auth` answers before any handler or backend code runs.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn unauthenticated_adopt_is_unauthorized(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let resp = app
        .client
        .post(app.url("/api/resources/adopt"))
        .json(&adopt_body(AdoptScope::All, true, None))
        .send()
        .await
        .expect("adopt request failed");

    assert_eq!(
        resp.status().as_u16(),
        401,
        "unauthenticated adopt must be 401; body: {}",
        resp.text().await.unwrap_or_default()
    );
}

/// `all` claims deployment-wide reach, so a non-admin caller is refused at the backend seam —
/// the route itself carries no admin gate, and the 403 the seam returns surfaces as-is.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn non_admin_all_scope_is_refused_through_the_route(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("adopt-nonadmin-{}@example.com", Uuid::new_v4());
    let (profile_id, _context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);

    let resp = app
        .client
        .post(app.url("/api/resources/adopt"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&adopt_body(AdoptScope::All, true, None))
        .send()
        .await
        .expect("adopt request failed");

    assert_eq!(
        resp.status().as_u16(),
        403,
        "non-admin all-scope adopt must be 403; body: {}",
        resp.text().await.unwrap_or_default()
    );
}

/// A dry-run survey over a context returns a well-formed receipt: the survey flag rides the
/// receipt, the batch correlation id is present, and the one conformed candidate is typed a
/// no-op — proving handler→backend dispatch end to end at route grain.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn dry_run_context_survey_returns_a_well_formed_receipt(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("adopt-owner-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);

    // The emitter entity the write path resolves for an ApiHttp caller (the fixture seeds
    // `<handle>@web|cli|mcp`; the @web entity doubles as the seeder's emitter).
    let emitter: Uuid = sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id = $1")
        .bind(profile_id)
        .fetch_one(&app.pool)
        .await
        .expect("seeded emitter entity");
    let resource =
        fire_conformed_resource(&app.pool, profile_id, emitter, context_id, "conformed").await;

    let resp = app
        .client
        .post(app.url("/api/resources/adopt"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&adopt_body(AdoptScope::Context(context_id), true, None))
        .send()
        .await
        .expect("adopt request failed");

    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.expect("expected JSON receipt");
    assert_eq!(status, 200, "survey adopt must be 200; body: {body}");

    assert_eq!(body["dry_run"], serde_json::json!(true));
    let correlation = body["correlation_id"]
        .as_str()
        .expect("correlation_id present");
    correlation
        .parse::<Uuid>()
        .expect("correlation_id is a UUID");

    let outcomes = body["outcomes"].as_array().expect("outcomes array");
    assert_eq!(outcomes.len(), 1, "one candidate, one row: {body}");
    assert_eq!(
        outcomes[0]["resource"],
        serde_json::json!(resource.to_string()),
        "the row names the candidate"
    );
    assert_eq!(
        outcomes[0]["outcome"], "NoOp",
        "the conformed candidate is typed a no-op: {body}"
    );

    assert_eq!(
        body["summary"]["no_op"],
        serde_json::json!(1),
        "body: {body}"
    );
    assert_eq!(
        body["after_id"],
        serde_json::json!(resource.to_string()),
        "the cursor names the last candidate considered"
    );
}

/// A non-positive limit is refused — every invocation is bounded, and the bound is checked at
/// the backend seam before any candidate work.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn non_positive_limit_is_a_bad_request(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    let email = format!("adopt-limit-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);

    let resp = app
        .client
        .post(app.url("/api/resources/adopt"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&adopt_body(AdoptScope::Context(context_id), true, Some(0)))
        .send()
        .await
        .expect("adopt request failed");

    assert_eq!(
        resp.status().as_u16(),
        400,
        "limit 0 must be a 400; body: {}",
        resp.text().await.unwrap_or_default()
    );
}
