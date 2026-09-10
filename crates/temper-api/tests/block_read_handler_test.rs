//! Witnesses for `GET /api/resources/{id}/blocks/{block_id}` (the `read_block` handler) — the
//! three-state resolution rendered over HTTP (the defined-dangling-state design, D-D1/D-D3):
//! `200` live / `410 Gone` folded (the same envelope body, never a redirect) / `404` absent,
//! and a not-visible home denying existence (404, never 403 — no existence oracle). The
//! blocking policy is exercised through the REAL create/update path: a two-heading content
//! PATCH becomes two blocks, and a filler-append re-PATCH folds the rewritten-away section.
//!
//! Harness cribbed from `resource_update_body_test.rs` (setup_test_app /
//! create_test_profile_with_context / generate_test_jwt / app.client / app.url).
#![cfg(feature = "test-db")]

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

const BODY_A_B: &str = "# Alpha\n\nAlpha body paragraph.\n## Beta\n\nBeta body paragraph.\n";

/// The filler-append rewrite of section A (whole_body_replace.rs:310-313's geometry): A's bytes
/// differ so its incumbent folds, while the beta section survives byte-identical and is kept.
fn body_a_with_filler() -> String {
    let filler = "Brand new prose. ".repeat(120);
    format!("# Alpha\n\nAlpha body paragraph.\n\n{filler}\n## Beta\n\nBeta body paragraph.\n")
}

/// Create a resource in the test profile's context (the create payload carries no body — the
/// body arrives on the content PATCH).
async fn create_resource(app: &common::TestApp, token: &str, context_id: Uuid) -> String {
    let created: Value = app
        .client
        .post(app.url("/api/resources"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "kb_context_id": context_id.to_string(),
            "doc_type": "research",
            "origin_uri": format!("test://block-read-{}", Uuid::new_v4()),
            "title": "Block Read Test",
            "slug": null
        }))
        .send()
        .await
        .expect("create failed")
        .json()
        .await
        .expect("create JSON");
    created["id"]
        .as_str()
        .expect("created resource id missing")
        .to_string()
}

/// Give the resource `content` via a content PATCH — the whole-body write whose partition the
/// blocking policy cuts along the heading structure.
async fn patch_content(app: &common::TestApp, token: &str, resource_id: &str, content: &str) {
    let resp = app
        .client
        .patch(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "content": content }))
        .send()
        .await
        .expect("content PATCH failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "fixture: content PATCH must land; got {}",
        resp.text().await.unwrap_or_default()
    );
}

/// (id, seq) of the resource's LIVE blocks, seq order — read straight off the pool so the
/// fixture's partition shape is verified against the database, not the API's word for it.
async fn live_block_ids(pool: &PgPool, resource_id: &str) -> Vec<Uuid> {
    let resource: Uuid = resource_id.parse().expect("resource id is a uuid");
    sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id=$1 AND NOT is_folded ORDER BY seq, id",
    )
    .bind(resource)
    .fetch_all(pool)
    .await
    .expect("live block query")
}

async fn get_block(
    app: &common::TestApp,
    token: &str,
    resource_id: &str,
    block_id: Uuid,
) -> (u16, Value) {
    let resp = app
        .client
        .get(app.url(&format!("/api/resources/{resource_id}/blocks/{block_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("block GET failed");
    let status = resp.status().as_u16();
    let body = resp.json::<Value>().await.expect("block GET JSON");
    (status, body)
}

/// CLAUSE: the route answers the resolved states BY NAME — 200 `live` for a surviving block,
/// 410 `folded` for a rewritten-away incumbent (the envelope body, not an error body), and
/// 404 `absent` for an address that names no row. Before this route a folded address had no
/// defined answer at all, and before the handler mapped the `Absent` arm it fell through as
/// a 200 with a serialized `"absent"` envelope — contradicting the route's own declaration.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_route_answers_live_and_folded_by_name(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let email = format!("block-read-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);

    let resource_id = create_resource(&app, &token, context_id).await;
    patch_content(&app, &token, &resource_id, BODY_A_B).await;
    let blocks = live_block_ids(&pool, &resource_id).await;
    assert_eq!(
        blocks.len(),
        2,
        "fixture: the blocking policy cut the two-heading body into two live blocks"
    );
    let folded_id = blocks[0];

    // The filler-append rewrite of section A: A folds, its section re-creates, B is kept.
    patch_content(&app, &token, &resource_id, &body_a_with_filler()).await;
    let after = live_block_ids(&pool, &resource_id).await;
    assert_eq!(
        after.len(),
        2,
        "A folds and its section re-creates; B is kept"
    );
    assert!(
        !after.contains(&folded_id),
        "fixture: the alpha incumbent folded"
    );

    let (live_status, live_body) = get_block(&app, &token, &resource_id, after[0]).await;
    assert_eq!(live_status, 200, "a live block answers 200");
    assert_eq!(live_body["state"], "live", "got {live_body}");

    let (folded_status, folded_body) = get_block(&app, &token, &resource_id, folded_id).await;
    assert_eq!(folded_status, 410, "a folded block answers 410 Gone");
    assert_eq!(
        folded_body["state"], "folded",
        "410 carries the envelope body, not an error body — got {folded_body}"
    );

    // An address that names no row under the resource: 404 — the ordinary not-found face,
    // never a 200-shaped `"absent"` envelope.
    let (absent_status, absent_body) = get_block(&app, &token, &resource_id, Uuid::new_v4()).await;
    assert_eq!(
        absent_status, 404,
        "a bogus address answers 404 — got {absent_body}"
    );
}

/// CLAUSE: a real block behind a home the caller cannot read renders 404 — the deny that
/// refuses to confirm the block exists, never 403. The address is REAL (read from the pool
/// after the owner's own write), so a broken visibility gate would answer 200, not 404.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_route_denies_existence_for_an_unreadable_home(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;

    let owner_email = format!("block-owner-{}@example.com", Uuid::new_v4());
    let (owner_id, owner_context) =
        common::fixtures::create_test_profile_with_context(&pool, &owner_email).await;
    let owner_token = common::generate_test_jwt(&format!("test|{owner_id}"), &owner_email);

    let other_email = format!("block-outsider-{}@example.com", Uuid::new_v4());
    let (other_id, _) =
        common::fixtures::create_test_profile_with_context(&pool, &other_email).await;
    let other_token = common::generate_test_jwt(&format!("test|{other_id}"), &other_email);

    // The owner's resource carries REAL blocks, addressed by the outsider below.
    let resource_id = create_resource(&app, &owner_token, owner_context).await;
    patch_content(&app, &owner_token, &resource_id, BODY_A_B).await;
    let block_id = live_block_ids(&pool, &resource_id).await[0];

    let (status, body) = get_block(&app, &other_token, &resource_id, block_id).await;
    assert_eq!(
        status, 404,
        "an unreadable home denies existence (404, never 403) — got {status} {body}"
    );
}

/// CLAUSE: the route never answers an address with a redirect (D-D3, pinned — the claim was
/// true but unpinned). All three resolved faces — live, folded, absent — must carry no 3xx
/// status and no `Location` header: successor-naming rides as gated data inside the response
/// body, never as a followable location the caller may not be authorized to follow.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_route_never_answers_an_address_with_a_redirect(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let email = format!("block-no-redirect-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);

    let resource_id = create_resource(&app, &token, context_id).await;
    patch_content(&app, &token, &resource_id, BODY_A_B).await;
    let blocks = live_block_ids(&pool, &resource_id).await;
    let folded_id = blocks[0];
    patch_content(&app, &token, &resource_id, &body_a_with_filler()).await;
    let after = live_block_ids(&pool, &resource_id).await;
    assert!(
        !after.contains(&folded_id),
        "fixture: the alpha incumbent folded"
    );

    for (face, block_id) in [
        ("live", after[0]),
        ("folded", folded_id),
        ("absent", Uuid::new_v4()),
    ] {
        let resp = app
            .client
            .get(app.url(&format!("/api/resources/{resource_id}/blocks/{block_id}")))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .expect("block GET failed");
        let status = resp.status();
        assert!(
            !status.is_redirection(),
            "the {face} face must never redirect, got {status}"
        );
        assert!(
            resp.headers().get("location").is_none(),
            "the {face} face must never carry a Location header"
        );
    }
}
